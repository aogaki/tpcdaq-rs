//! CAEN DT1415ET(HiVolta)ドライバ —— [`Driver`] 実装(TODO/089)。
//!
//! `hivolta.rs`(TODO/085)のコーデックに実トランスポート(TCP :1470 / USB VCP 9600 8N1)を
//! 差しただけのもの(`bronkhorst.rs` / `arduino.rs` と同じ形)。**書き込みは無い**
//! (085 と同じ安全境界 —— コーデックに `$CMD:SET,...` を生成する関数が無い)。
//!
//! # 発注書 §1/§2 の食い違いについて(2026-09-23、報告事項)
//!
//! TODO/089 §1 は 1 ch あたり `VMON` / `IMON` / `STATUS` / `VSET` の **4 項目**だけを列挙して
//! いるが、同じ節の見積もり文「1 poll = (5 × nch + 2) 往復」および §2 のテスト文言
//! 「nch = 2 の fake で 12 サンプル」は 1 ch あたり **5 項目**を前提にした数(5×2+2=12、
//! 4×2+2=10 ではない)。発注書に 5 個目のパラメータ名が無いため、本実装は §1 の列挙を
//! そのまま実装した(4 項目 + ボード 2 項目 = `4×nch+2`)。5 項目目が何であるべきかは
//! 実装側で憶測すべきでないので、完了報告で発注元に差し戻す。

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use tracing::info;

use super::hivolta::{
    mon_bd, mon_ch, parse_alarm, parse_status, BdPar, ChPar, HvLink, HvLinkError,
};
use super::{now_ms, Driver, DriverError, Ident, Sample};

/// TCP 接続タイムアウト(発注書 §1: `connect_timeout` 3 s)。
pub const CONNECT_TIMEOUT_MS: u64 = 3_000;
/// TCP/シリアルの読み書きタイムアウト(発注書 §1: 2 s)。
pub const READ_TIMEOUT_MS: u64 = 2_000;
/// USB VCP のボーレート(発注書 §1: serial 9600 8N1)。
pub const BAUD_RATE: u32 = 9_600;

pub const PARAM_VMON: &str = "vmon";
pub const PARAM_IMON: &str = "imon";
pub const PARAM_STATUS: &str = "status";
pub const PARAM_VSET: &str = "vset";
pub const PARAM_BDALARM: &str = "bdalarm";
pub const PARAM_BDILK: &str = "bdilk";

pub const UNIT_V: &str = "V";
pub const UNIT_UA: &str = "µA";
pub const UNIT_BITMASK: &str = "bitmask";
pub const UNIT_FLAG: &str = "flag";

/// CAEN DT1415ET(HiVolta)ドライバ。`node` = チャンネル番号(0..nch)、ボード値は `node = None`。
pub struct HiVoltaDriver<T: Read + Write> {
    device: String,
    link: HvLink<T>,
    /// 空でなければ `ident` で `BDSNUM` と照合する(TODO/089 §1)。
    expect_serial: String,
    /// `ident` の `BDNCH` で確定するチャンネル数(**焼き込まない** —— TODO/089 §1)。
    nch: u8,
}

impl<T: Read + Write> HiVoltaDriver<T> {
    pub fn new(device: impl Into<String>, io: T, expect_serial: String) -> Self {
        Self {
            device: device.into(),
            link: HvLink::new(io),
            expect_serial,
            nch: 0,
        }
    }

    /// `ident` が確定したチャンネル数(テスト用)。
    pub fn nch(&self) -> u8 {
        self.nch
    }
}

/// `HvLinkError` → `DriverError`。**区別を潰さない**(bronkhorst.rs の `driver_error` と同じ形)。
fn driver_error(error: HvLinkError) -> DriverError {
    match error {
        HvLinkError::Io(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock
                    | std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::UnexpectedEof
            ) =>
        {
            DriverError::Timeout
        }
        HvLinkError::Io(e) => DriverError::Io(e),
        HvLinkError::Protocol(e) => DriverError::Protocol(e.to_string()),
    }
}

/// `parse_reply` の `Vec<String>` から単一値を取り出す(MON 応答は単 ch なら 1 要素のはず —
/// 個数が違えばプロトコル違反として可視化する。CLAUDE.md の silent failure 禁止)。
fn single_value(mut values: Vec<String>, what: &str) -> Result<String, DriverError> {
    if values.len() != 1 {
        return Err(DriverError::Protocol(format!(
            "{what}: expected a single value, got {values:?}"
        )));
    }
    values
        .pop()
        .ok_or_else(|| DriverError::Protocol(format!("{what}: empty value list")))
}

fn parse_f64_value(value: &str, what: &str) -> Result<f64, DriverError> {
    value
        .trim()
        .parse::<f64>()
        .map_err(|_| DriverError::Protocol(format!("{what}: not a number: {value:?}")))
}

/// `BDILK` の `YES`/`NO` を `1.0`/`0.0` にする(発注書 §1: `unit "flag"`)。
fn parse_bdilk(value: &str) -> Result<f64, DriverError> {
    match value.trim() {
        "YES" => Ok(1.0),
        "NO" => Ok(0.0),
        other => Err(DriverError::Protocol(format!(
            "BDILK: expected YES/NO, got {other:?}"
        ))),
    }
}

impl<T: Read + Write> Driver for HiVoltaDriver<T> {
    fn ident(&mut self) -> Result<Vec<Ident>, DriverError> {
        let name = single_value(
            self.link
                .exchange(&mon_bd(BdPar::Name))
                .map_err(driver_error)?,
            "BDNAME",
        )?;
        let nch_str = single_value(
            self.link
                .exchange(&mon_bd(BdPar::Nch))
                .map_err(driver_error)?,
            "BDNCH",
        )?;
        let nch: u8 = nch_str
            .trim()
            .parse()
            .map_err(|_| DriverError::Protocol(format!("BDNCH: not a number: {nch_str:?}")))?;
        let frel = single_value(
            self.link
                .exchange(&mon_bd(BdPar::Frel))
                .map_err(driver_error)?,
            "BDFREL",
        )?;
        let snum = single_value(
            self.link
                .exchange(&mon_bd(BdPar::Snum))
                .map_err(driver_error)?,
            "BDSNUM",
        )?;
        if !self.expect_serial.is_empty() && self.expect_serial != snum {
            return Err(DriverError::Identity {
                expected: self.expect_serial.clone(),
                got: snum,
            });
        }
        let ctr = single_value(
            self.link
                .exchange(&mon_bd(BdPar::Ctr))
                .map_err(driver_error)?,
            "BDCTR",
        )?;
        let ilkm = single_value(
            self.link
                .exchange(&mon_bd(BdPar::IlkMode))
                .map_err(driver_error)?,
            "BDILKM",
        )?;

        self.nch = nch;

        let mut fields = BTreeMap::new();
        fields.insert("name".to_string(), name.clone());
        fields.insert("nch".to_string(), nch.to_string());
        fields.insert("frel".to_string(), frel);
        fields.insert("snum".to_string(), snum.clone());
        fields.insert("ctr".to_string(), ctr);
        fields.insert("ilkm".to_string(), ilkm);

        info!(
            device = self.device,
            name, nch, snum, "slowctl: hivolta identified"
        );
        Ok(vec![Ident {
            device: self.device.clone(),
            node: None,
            fields,
        }])
    }

    fn poll(&mut self) -> Result<Vec<Sample>, DriverError> {
        let ts_ms = now_ms();
        let mut samples = Vec::with_capacity(usize::from(self.nch) * 4 + 2);

        for ch in 0..self.nch {
            let vmon_s = single_value(
                self.link
                    .exchange(&mon_ch(ch, ChPar::Vmon))
                    .map_err(driver_error)?,
                "VMON",
            )?;
            let vmon = parse_f64_value(&vmon_s, "VMON")?;

            let imon_s = single_value(
                self.link
                    .exchange(&mon_ch(ch, ChPar::Imon))
                    .map_err(driver_error)?,
                "IMON",
            )?;
            let imon = parse_f64_value(&imon_s, "IMON")?;

            let status_s = single_value(
                self.link
                    .exchange(&mon_ch(ch, ChPar::Status))
                    .map_err(driver_error)?,
                "STATUS",
            )?;
            let status =
                parse_status(&status_s).map_err(|e| DriverError::Protocol(e.to_string()))?;

            let vset_s = single_value(
                self.link
                    .exchange(&mon_ch(ch, ChPar::Vset))
                    .map_err(driver_error)?,
                "VSET",
            )?;
            let vset = parse_f64_value(&vset_s, "VSET")?;

            // 発注書 §1: 「STATUS ビット → alarm」= ovc|ovv|unv|trip|ovp|ovt|kill|intlk|fail。
            // bronkhorst.rs の「Alarm info ≠ 0 はそのノードの全サンプルに立てる」流儀を踏襲し、
            // この ch の 4 サンプル全部に同じ alarm を立てる。
            let alarm = status.ovc()
                || status.ovv()
                || status.unv()
                || status.trip()
                || status.ovp()
                || status.ovt()
                || status.kill()
                || status.intlk()
                || status.fail();

            samples.push(Sample {
                device: self.device.clone(),
                node: Some(ch),
                param: PARAM_VMON.to_string(),
                value: vmon,
                unit: UNIT_V.to_string(),
                ts_ms,
                alarm,
            });
            samples.push(Sample {
                device: self.device.clone(),
                node: Some(ch),
                param: PARAM_IMON.to_string(),
                value: imon,
                unit: UNIT_UA.to_string(),
                ts_ms,
                alarm,
            });
            samples.push(Sample {
                device: self.device.clone(),
                node: Some(ch),
                param: PARAM_STATUS.to_string(),
                value: f64::from(status.0),
                unit: UNIT_BITMASK.to_string(),
                ts_ms,
                alarm,
            });
            samples.push(Sample {
                device: self.device.clone(),
                node: Some(ch),
                param: PARAM_VSET.to_string(),
                value: vset,
                unit: UNIT_V.to_string(),
                ts_ms,
                alarm,
            });
        }

        let bdalarm_s = single_value(
            self.link
                .exchange(&mon_bd(BdPar::Alarm))
                .map_err(driver_error)?,
            "BDALARM",
        )?;
        let bdalarm = parse_alarm(&bdalarm_s).map_err(|e| DriverError::Protocol(e.to_string()))?;
        samples.push(Sample {
            device: self.device.clone(),
            node: None,
            param: PARAM_BDALARM.to_string(),
            value: f64::from(bdalarm.0),
            unit: UNIT_BITMASK.to_string(),
            ts_ms,
            alarm: bdalarm.0 != 0,
        });

        let bdilk_s = single_value(
            self.link
                .exchange(&mon_bd(BdPar::Ilk))
                .map_err(driver_error)?,
            "BDILK",
        )?;
        let bdilk = parse_bdilk(&bdilk_s)?;
        samples.push(Sample {
            device: self.device.clone(),
            node: None,
            param: PARAM_BDILK.to_string(),
            value: bdilk,
            unit: UNIT_FLAG.to_string(),
            ts_ms,
            alarm: false,
        });

        Ok(samples)
    }
}

// ---------------------------------------------------------------------
// 実トランスポートの組み立て(唯一 IO に触る場所)
// ---------------------------------------------------------------------

/// TCP :1470 へ接続する(発注書 §1: `addr` 優先)。
pub fn open_tcp(
    device: &str,
    addr: &str,
    expect_serial: String,
) -> Result<HiVoltaDriver<TcpStream>, DriverError> {
    let target = addr
        .to_socket_addrs()
        .map_err(|e| DriverError::Protocol(format!("cannot resolve hivolta addr {addr:?}: {e}")))?
        .next()
        .ok_or_else(|| {
            DriverError::Protocol(format!("hivolta addr {addr:?} resolved to no address"))
        })?;
    let stream = TcpStream::connect_timeout(&target, Duration::from_millis(CONNECT_TIMEOUT_MS))
        .map_err(|e| DriverError::Protocol(format!("cannot connect to hivolta at {addr}: {e}")))?;
    stream
        .set_read_timeout(Some(Duration::from_millis(READ_TIMEOUT_MS)))
        .map_err(|e| DriverError::Protocol(format!("set_read_timeout on {addr} failed: {e}")))?;
    stream
        .set_write_timeout(Some(Duration::from_millis(READ_TIMEOUT_MS)))
        .map_err(|e| DriverError::Protocol(format!("set_write_timeout on {addr} failed: {e}")))?;
    info!(device, addr, "slowctl: hivolta tcp connected");
    Ok(HiVoltaDriver::new(device, stream, expect_serial))
}

/// `/dev/serial/by-id/…` を開く(発注書 §1: `addr` が無い場合のフォールバック、9600 8N1)。
pub fn open_serial(
    device: &str,
    port: &str,
    expect_serial: String,
) -> Result<HiVoltaDriver<Box<dyn serialport::SerialPort>>, DriverError> {
    let handle = serialport::new(port, BAUD_RATE)
        .data_bits(serialport::DataBits::Eight)
        .parity(serialport::Parity::None)
        .stop_bits(serialport::StopBits::One)
        .timeout(Duration::from_millis(READ_TIMEOUT_MS))
        .open()
        .map_err(|e| DriverError::Protocol(format!("cannot open serial port {port}: {e}")))?;
    info!(
        device,
        port,
        baud = BAUD_RATE,
        "slowctl: hivolta serial port open"
    );
    Ok(HiVoltaDriver::new(device, handle, expect_serial))
}

// ---------------------------------------------------------------------
// テスト
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::slowctl::fake_hv::FakeHv;

    /// 6 個の BD 応答を登録する(name/nch/frel/snum/ctr/ilkm、発注書 §1 の順)。
    fn register_ident(fake: &mut FakeHv, nch: u8, snum: &str) {
        fake.set_response(mon_bd(BdPar::Name), "#CMD:OK,VAL:DT1415ET\r\n");
        fake.set_response(mon_bd(BdPar::Nch), format!("#CMD:OK,VAL:{nch}\r\n"));
        fake.set_response(mon_bd(BdPar::Frel), "#CMD:OK,VAL:1.5\r\n");
        fake.set_response(mon_bd(BdPar::Snum), format!("#CMD:OK,VAL:{snum}\r\n"));
        fake.set_response(mon_bd(BdPar::Ctr), "#CMD:OK,VAL:REMOTE\r\n");
        fake.set_response(mon_bd(BdPar::IlkMode), "#CMD:OK,VAL:DISABLE\r\n");
    }

    #[test]
    fn ident_reads_all_six_board_fields_and_keeps_nch() {
        let mut fake = FakeHv::new();
        register_ident(&mut fake, 2, "SN12345");
        let mut driver = HiVoltaDriver::new("hv", fake, String::new());

        let idents = driver.ident().unwrap();
        assert_eq!(idents.len(), 1);
        let ident = &idents[0];
        assert_eq!(ident.device, "hv");
        assert_eq!(ident.node, None);
        assert_eq!(ident.fields["name"], "DT1415ET");
        assert_eq!(ident.fields["nch"], "2");
        assert_eq!(ident.fields["frel"], "1.5");
        assert_eq!(ident.fields["snum"], "SN12345");
        assert_eq!(ident.fields["ctr"], "REMOTE");
        assert_eq!(ident.fields["ilkm"], "DISABLE");
        assert_eq!(driver.nch(), 2, "BDNCH を焼き込まずに保持する");
    }

    /// `expect_serial` が空でなければ `BDSNUM` と照合する(TODO/089 §1)。不一致は `Identity`。
    #[test]
    fn ident_serial_mismatch_is_an_identity_error() {
        let mut fake = FakeHv::new();
        register_ident(&mut fake, 2, "SN12345");
        let mut driver = HiVoltaDriver::new("hv", fake, "OTHER-SERIAL".to_string());

        assert!(matches!(
            driver.ident(),
            Err(DriverError::Identity { expected, got })
                if expected == "OTHER-SERIAL" && got == "SN12345"
        ));
    }

    /// 一致すれば通常どおり通る。
    #[test]
    fn ident_serial_match_succeeds() {
        let mut fake = FakeHv::new();
        register_ident(&mut fake, 1, "SN-OK");
        let mut driver = HiVoltaDriver::new("hv", fake, "SN-OK".to_string());
        assert!(driver.ident().is_ok());
    }

    /// nch = 2 の poll: 発注書 §1 が列挙した 4 項目 × 2 ch + ボード 2 項目 = 10 サンプル
    /// (発注書 §1/§2 の「5×nch+2」「12 サンプル」との食い違いはこのファイル冒頭の doc 参照 —
    /// 5 個目の ch パラメータ名が発注書に無いため、明記された 4 項目のみを実装した)。
    #[test]
    fn poll_of_two_channels_yields_four_samples_each_plus_two_board_samples() {
        let mut fake = FakeHv::new();
        register_ident(&mut fake, 2, "SN-UNUSED");
        // ch0: 正常(alarm 無し)。
        fake.set_response(mon_ch(0, ChPar::Vmon), "#CMD:OK,VAL:500.00\r\n");
        fake.set_response(mon_ch(0, ChPar::Imon), "#CMD:OK,VAL:12.50\r\n");
        fake.set_response(mon_ch(0, ChPar::Status), "#CMD:OK,VAL:1\r\n"); // bit0 = ON のみ
        fake.set_response(mon_ch(0, ChPar::Vset), "#CMD:OK,VAL:500.00\r\n");
        // ch1: TRIP(bit6 = 64)が立っている → alarm。
        fake.set_response(mon_ch(1, ChPar::Vmon), "#CMD:OK,VAL:0.00\r\n");
        fake.set_response(mon_ch(1, ChPar::Imon), "#CMD:OK,VAL:0.00\r\n");
        fake.set_response(mon_ch(1, ChPar::Status), "#CMD:OK,VAL:64\r\n");
        fake.set_response(mon_ch(1, ChPar::Vset), "#CMD:OK,VAL:800.00\r\n");
        // ボード: BDALARM 非 0(trip ビット = 64)、BDILK = YES。
        fake.set_response(mon_bd(BdPar::Alarm), "#CMD:OK,VAL:64\r\n");
        fake.set_response(mon_bd(BdPar::Ilk), "#CMD:OK,VAL:YES\r\n");

        let mut driver = HiVoltaDriver::new("hv", fake, String::new());
        driver.ident().unwrap();
        let samples = driver.poll().unwrap();

        assert_eq!(samples.len(), 10, "4 x 2ch + board 2 = 10");

        // ch0(正常)。
        assert_eq!(samples[0].param, "vmon");
        assert_eq!(samples[0].node, Some(0));
        assert_eq!(samples[0].value, 500.0);
        assert_eq!(samples[0].unit, "V");
        assert!(!samples[0].alarm, "bit0(ON)だけなら alarm 対象外");
        assert_eq!(samples[1].param, "imon");
        assert_eq!(samples[1].unit, "µA");
        assert_eq!(samples[2].param, "status");
        assert_eq!(samples[2].value, 1.0);
        assert_eq!(samples[2].unit, "bitmask");
        assert_eq!(samples[3].param, "vset");
        assert_eq!(samples[3].unit, "V");

        // ch1(TRIP)は 4 サンプルとも alarm。
        for sample in &samples[4..8] {
            assert_eq!(sample.node, Some(1));
            assert!(sample.alarm, "TRIP ビットは ch1 の全サンプルに立つ");
        }
        assert_eq!(samples[6].value, 64.0);

        // ボード。
        assert_eq!(samples[8].param, "bdalarm");
        assert_eq!(samples[8].node, None);
        assert_eq!(samples[8].value, 64.0);
        assert!(samples[8].alarm, "BDALARM 非 0 は alarm");
        assert_eq!(samples[9].param, "bdilk");
        assert_eq!(samples[9].value, 1.0, "YES = 1.0");
        assert_eq!(samples[9].unit, "flag");
        assert!(
            !samples[9].alarm,
            "BDILK 自体には alarm を付けない(発注書 §1 に規定無し)"
        );
    }

    /// `#LOC:ERR` 等のエラー応答は `Protocol`(parse_err に数えられる、発注書 §1)。
    #[test]
    fn a_loc_err_reply_is_a_protocol_error() {
        let mut fake = FakeHv::new();
        register_ident(&mut fake, 1, "SN-UNUSED");
        fake.set_response(mon_ch(0, ChPar::Vmon), "#LOC:ERR\r\n");

        let mut driver = HiVoltaDriver::new("hv", fake, String::new());
        driver.ident().unwrap();
        assert!(matches!(driver.poll(), Err(DriverError::Protocol(_))));
    }

    /// 無応答は `Timeout`(発注書 §1)。
    #[test]
    fn a_silent_device_times_out() {
        let mut fake = FakeHv::new();
        register_ident(&mut fake, 1, "SN-UNUSED");
        fake.set_silent();

        let mut driver = HiVoltaDriver::new("hv", fake, String::new());
        assert!(matches!(driver.ident(), Err(DriverError::Timeout)));
    }
}
