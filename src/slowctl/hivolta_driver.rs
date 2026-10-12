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
    mon_bd, mon_ch, parse_alarm, parse_status, BdPar, ChPar, HvLink, HvLinkError, ALL_CH,
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
pub const PARAM_ISET: &str = "iset";
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

impl<T: Read + Write> HiVoltaDriver<T> {
    /// 全 ch 一括(`CH:8`)で 1 パラメータ読む。値の数が `nch` と違えば Protocol エラー
    /// (黙って切り詰めない・埋めない)。
    fn bulk(&mut self, par: ChPar, what: &str) -> Result<Vec<String>, DriverError> {
        let values = self
            .link
            .exchange(&mon_ch(ALL_CH, par))
            .map_err(driver_error)?;
        if values.len() != usize::from(self.nch) {
            return Err(DriverError::Protocol(format!(
                "{what}: expected {} values, got {}: {values:?}",
                self.nch,
                values.len()
            )));
        }
        Ok(values)
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
        // TODO/119: poll は全 ch 一括(CH:8)で読むので ALL_CH 以外の機種は扱わない(可視化)。
        if nch != ALL_CH {
            return Err(DriverError::Protocol(format!(
                "BDNCH: expected {ALL_CH} channels, got {nch}"
            )));
        }
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
        let mut samples = Vec::with_capacity(usize::from(self.nch) * 5 + 2);

        // TODO/119: 通信 1 往復ごとに偽トリガが入る(TODO/118)ので、全 ch 一括(CH:8)で
        // 5 往復 + ボード 2 往復 = 7 往復/周期にする(TODO/120: ISET を追加)。
        let vmon_all = self.bulk(ChPar::Vmon, "VMON")?;
        let imon_all = self.bulk(ChPar::Imon, "IMON")?;
        let status_all = self.bulk(ChPar::Status, "STATUS")?;
        let vset_all = self.bulk(ChPar::Vset, "VSET")?;
        let iset_all = self.bulk(ChPar::Iset, "ISET")?;

        for ch in 0..self.nch {
            let i = usize::from(ch);
            let vmon = parse_f64_value(&vmon_all[i], "VMON")?;
            let imon = parse_f64_value(&imon_all[i], "IMON")?;
            let status =
                parse_status(&status_all[i]).map_err(|e| DriverError::Protocol(e.to_string()))?;
            let vset = parse_f64_value(&vset_all[i], "VSET")?;
            let iset = parse_f64_value(&iset_all[i], "ISET")?;

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
            samples.push(Sample {
                device: self.device.clone(),
                node: Some(ch),
                param: PARAM_ISET.to_string(),
                value: iset,
                unit: UNIT_UA.to_string(),
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

type SerialBox = Box<dyn serialport::SerialPort>;

/// `/dev/serial/by-id/…` 用のドライバ(発注書 §1: `addr` が無い場合のフォールバック、9600 8N1)。
/// ポートはここでは開かない(`ident` / `poll` のたびに開閉する —— [`OnDemandHiVolta`])。
pub fn open_serial(
    device: &str,
    port: &str,
    expect_serial: String,
) -> OnDemandHiVolta<SerialBox, impl FnMut() -> Result<SerialBox, DriverError> + Send> {
    let port = port.to_string();
    let open = move || {
        serialport::new(&port, BAUD_RATE)
            .data_bits(serialport::DataBits::Eight)
            .parity(serialport::Parity::None)
            .stop_bits(serialport::StopBits::One)
            .timeout(Duration::from_millis(READ_TIMEOUT_MS))
            .open()
            .map_err(|e| DriverError::Protocol(format!("cannot open serial port {port}: {e}")))
    };
    OnDemandHiVolta::new(device, open, expect_serial)
}

// ---------------------------------------------------------------------
// シリアル(USB)経路: 読むときだけポートを開く(TODO/119 追補 §5)
// ---------------------------------------------------------------------

/// `ident` / `poll` のたびに `open` でポートを開き、スコープの終わりで閉じる [`Driver`]。
///
/// DT1415ET は USB のシリアルポートを**開いているだけ**で検出器に偽トリガを作る(実機確認済み)
/// ので、呼び出しの間はポートを保持しない。`nch` は最初の `ident` で確定したものを保持し、
/// `poll` では BDSNUM を読み直さない(往復が増える)。
pub struct OnDemandHiVolta<T: Read + Write, F: FnMut() -> Result<T, DriverError>> {
    device: String,
    open: F,
    expect_serial: String,
    nch: u8,
}

impl<T: Read + Write, F: FnMut() -> Result<T, DriverError>> OnDemandHiVolta<T, F> {
    pub fn new(device: impl Into<String>, open: F, expect_serial: String) -> Self {
        Self {
            device: device.into(),
            open,
            expect_serial,
            nch: 0,
        }
    }
}

impl<T: Read + Write, F: FnMut() -> Result<T, DriverError>> Driver for OnDemandHiVolta<T, F> {
    fn ident(&mut self) -> Result<Vec<Ident>, DriverError> {
        let io = (self.open)()?;
        let mut driver = HiVoltaDriver::new(self.device.clone(), io, self.expect_serial.clone());
        let idents = driver.ident()?;
        self.nch = driver.nch();
        Ok(idents)
    }

    fn poll(&mut self) -> Result<Vec<Sample>, DriverError> {
        let io = (self.open)()?;
        let mut driver = HiVoltaDriver::new(self.device.clone(), io, self.expect_serial.clone());
        driver.nch = self.nch;
        driver.poll()
    }
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
        register_ident(&mut fake, 8, "SN12345");
        let mut driver = HiVoltaDriver::new("hv", fake, String::new());

        let idents = driver.ident().unwrap();
        assert_eq!(idents.len(), 1);
        let ident = &idents[0];
        assert_eq!(ident.device, "hv");
        assert_eq!(ident.node, None);
        assert_eq!(ident.fields["name"], "DT1415ET");
        assert_eq!(ident.fields["nch"], "8");
        assert_eq!(ident.fields["frel"], "1.5");
        assert_eq!(ident.fields["snum"], "SN12345");
        assert_eq!(ident.fields["ctr"], "REMOTE");
        assert_eq!(ident.fields["ilkm"], "DISABLE");
        assert_eq!(driver.nch(), 8, "BDNCH を焼き込まずに保持する");
    }

    /// `expect_serial` が空でなければ `BDSNUM` と照合する(TODO/089 §1)。不一致は `Identity`。
    #[test]
    fn ident_serial_mismatch_is_an_identity_error() {
        let mut fake = FakeHv::new();
        register_ident(&mut fake, 8, "SN12345");
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
        register_ident(&mut fake, 8, "SN-OK");
        let mut driver = HiVoltaDriver::new("hv", fake, "SN-OK".to_string());
        assert!(driver.ident().is_ok());
    }

    // 実機(ELITPC、2026-10-12)の全 ch 一括応答。一字も変えない(TODO/119)。
    const VSET_REPLY: &str =
        "#CMD:OK,VAL:0350.00;0250.00;0350.00;0250.00;0350.00;0250.00;0600.00;0600.00\r\n";
    const VMON_REPLY: &str =
        "#CMD:OK,VAL:0341.06;0250.24;0349.88;0250.22;0349.92;0250.16;0595.96;0599.96\r\n";
    const IMON_REPLY: &str = "#CMD:OK,VAL:+0009.593;+0008.868;+0009.084;+0009.067;+0009.042;+0008.905;+0003.753;+0005.930\r\n";
    // 実機応答(ELITPC、2026-10-12)。一字も変えない。
    const ISET_REPLY: &str =
        "#CMD:OK,VAL:0015.00;0015.00;0015.00;0015.00;0015.00;0015.00;0015.00;0015.00\r\n";
    const STATUS_REPLY: &str = "#CMD:OK,VAL:00033;00001;00001;00001;00001;00001;00001;00001\r\n";

    /// 送信行を記録する薄いラッパ(FakeHv は Link に所有されるので外から見えない)。
    struct Tap {
        inner: FakeHv,
        sent: std::rc::Rc<std::cell::RefCell<Vec<u8>>>,
    }
    impl Read for Tap {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.inner.read(buf)
        }
    }
    impl Write for Tap {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.sent.borrow_mut().extend_from_slice(buf);
            self.inner.write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.inner.flush()
        }
    }

    fn register_golden(fake: &mut FakeHv) {
        fake.set_response(mon_ch(ALL_CH, ChPar::Vset), VSET_REPLY);
        fake.set_response(mon_ch(ALL_CH, ChPar::Vmon), VMON_REPLY);
        fake.set_response(mon_ch(ALL_CH, ChPar::Imon), IMON_REPLY);
        fake.set_response(mon_ch(ALL_CH, ChPar::Status), STATUS_REPLY);
        fake.set_response(mon_ch(ALL_CH, ChPar::Iset), ISET_REPLY);
        // ボード: BDALARM 非 0(trip ビット = 64)、BDILK = YES。
        fake.set_response(mon_bd(BdPar::Alarm), "#CMD:OK,VAL:64\r\n");
        fake.set_response(mon_bd(BdPar::Ilk), "#CMD:OK,VAL:YES\r\n");
    }

    /// poll は要求ちょうど 7 本(全 ch 一括 5 + ボード 2)で、Sample 列は ch 毎 5 サンプル
    /// (vmon/imon/status/vset/iset)× 8 + ボード 2 = 42(旧実装と同一の並び)。
    #[test]
    fn poll_sends_exactly_seven_requests_and_yields_the_per_channel_samples() {
        let mut inner = FakeHv::new();
        register_ident(&mut inner, 8, "SN-UNUSED");
        register_golden(&mut inner);
        let sent = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let io = Tap {
            inner,
            sent: sent.clone(),
        };
        let mut driver = HiVoltaDriver::new("hv", io, String::new());
        driver.ident().unwrap();
        sent.borrow_mut().clear();
        let samples = driver.poll().unwrap();

        let text = String::from_utf8(sent.borrow().clone()).unwrap();
        assert_eq!(
            text,
            "$CMD:MON,CH:8,PAR:VMON\r\n$CMD:MON,CH:8,PAR:IMON\r\n\
             $CMD:MON,CH:8,PAR:STATUS\r\n$CMD:MON,CH:8,PAR:VSET\r\n\
             $CMD:MON,CH:8,PAR:ISET\r\n$CMD:MON,PAR:BDALARM\r\n$CMD:MON,PAR:BDILK\r\n"
        );

        assert_eq!(samples.len(), 42, "5 x 8ch + board 2");

        // ch0(STATUS 33 = ON + UNV(bit5)→ alarm)。値は応答の 1 番目。
        assert_eq!(samples[0].param, "vmon");
        assert_eq!(samples[0].node, Some(0));
        assert_eq!(samples[0].value, 341.06);
        assert_eq!(samples[0].unit, "V");
        assert_eq!(samples[1].param, "imon");
        assert_eq!(samples[1].value, 9.593);
        assert_eq!(samples[1].unit, "µA");
        assert_eq!(samples[2].param, "status");
        assert_eq!(samples[2].value, 33.0);
        assert_eq!(samples[2].unit, "bitmask");
        assert_eq!(samples[3].param, "vset");
        assert_eq!(samples[3].value, 350.0);
        assert_eq!(samples[4].param, "iset");
        assert_eq!(samples[4].node, Some(0));
        assert_eq!(samples[4].value, 15.0);
        assert_eq!(samples[4].unit, "µA");
        for sample in &samples[0..5] {
            assert!(sample.alarm, "STATUS 33 の UNV は ch0 の全サンプルに alarm");
        }

        // ch1..7 は STATUS 1(ON のみ)→ alarm なし。ch6 = 末尾から 2 番目の ch。
        for (ch, chunk) in samples[5..40].chunks(5).enumerate() {
            let ch = ch + 1;
            assert!(chunk.iter().all(|s| s.node == Some(ch as u8) && !s.alarm));
        }
        assert_eq!(samples[30].value, 595.96, "ch6 vmon");
        assert_eq!(samples[31].value, 3.753, "ch6 imon");
        assert_eq!(samples[33].value, 600.0, "ch6 vset");
        assert_eq!(samples[35].value, 599.96, "ch7 vmon");
        assert_eq!(samples[36].value, 5.930, "ch7 imon");
        assert_eq!(samples[39].param, "iset");
        assert_eq!(samples[39].node, Some(7));
        assert_eq!(samples[39].value, 15.0, "ch7 iset");

        // ボード。
        assert_eq!(samples[40].param, "bdalarm");
        assert_eq!(samples[40].node, None);
        assert_eq!(samples[40].value, 64.0);
        assert!(samples[40].alarm, "BDALARM 非 0 は alarm");
        assert_eq!(samples[41].param, "bdilk");
        assert_eq!(samples[41].value, 1.0, "YES = 1.0");
        assert_eq!(samples[41].unit, "flag");
        assert!(!samples[41].alarm);
    }

    /// 応答の値が 7 個や 9 個なら Protocol(切り詰め・埋めをしない)。
    #[test]
    fn a_bulk_reply_with_the_wrong_value_count_is_a_protocol_error() {
        for reply in [
            "#CMD:OK,VAL:0341.06;0250.24;0349.88;0250.22;0349.92;0250.16;0595.96\r\n",
            "#CMD:OK,VAL:0341.06;0250.24;0349.88;0250.22;0349.92;0250.16;0595.96;0599.96;0001.00\r\n",
        ] {
            let mut fake = FakeHv::new();
            register_ident(&mut fake, 8, "SN-UNUSED");
            register_golden(&mut fake);
            fake.set_response(mon_ch(ALL_CH, ChPar::Vmon), reply);
            let mut driver = HiVoltaDriver::new("hv", fake, String::new());
            driver.ident().unwrap();
            assert!(matches!(driver.poll(), Err(DriverError::Protocol(_))));
        }
    }

    /// nch ≠ 8 の機種は ident で Protocol エラー(一括読みの前提が崩れる)。
    #[test]
    fn ident_rejects_a_board_that_is_not_eight_channels() {
        let mut fake = FakeHv::new();
        register_ident(&mut fake, 4, "SN-UNUSED");
        let mut driver = HiVoltaDriver::new("hv", fake, String::new());
        assert!(matches!(driver.ident(), Err(DriverError::Protocol(_))));
    }

    /// `#LOC:ERR` 等のエラー応答は `Protocol`(parse_err に数えられる、発注書 §1)。
    #[test]
    fn a_loc_err_reply_is_a_protocol_error() {
        let mut fake = FakeHv::new();
        register_ident(&mut fake, 8, "SN-UNUSED");
        fake.set_response(mon_ch(ALL_CH, ChPar::Vmon), "#LOC:ERR\r\n");

        let mut driver = HiVoltaDriver::new("hv", fake, String::new());
        driver.ident().unwrap();
        assert!(matches!(driver.poll(), Err(DriverError::Protocol(_))));
    }

    /// 無応答は `Timeout`(発注書 §1)。
    #[test]
    fn a_silent_device_times_out() {
        let mut fake = FakeHv::new();
        register_ident(&mut fake, 8, "SN-UNUSED");
        fake.set_silent();

        let mut driver = HiVoltaDriver::new("hv", fake, String::new());
        assert!(matches!(driver.ident(), Err(DriverError::Timeout)));
    }

    /// open / close / 送信行を数える。close は Drop で数える(スコープ終わりで確実に閉じる証拠)。
    struct Counters {
        opens: std::cell::Cell<u32>,
        closes: std::cell::Cell<u32>,
        sent: std::cell::RefCell<Vec<u8>>,
    }
    struct Counted {
        inner: FakeHv,
        c: std::rc::Rc<Counters>,
    }
    impl Read for Counted {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.inner.read(buf)
        }
    }
    impl Write for Counted {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.c.sent.borrow_mut().extend_from_slice(buf);
            self.inner.write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.inner.flush()
        }
    }
    impl Drop for Counted {
        fn drop(&mut self) {
            self.c.closes.set(self.c.closes.get() + 1);
        }
    }

    fn counted_driver(
        expect_serial: &str,
    ) -> (
        OnDemandHiVolta<Counted, impl FnMut() -> Result<Counted, DriverError>>,
        std::rc::Rc<Counters>,
    ) {
        let c = std::rc::Rc::new(Counters {
            opens: std::cell::Cell::new(0),
            closes: std::cell::Cell::new(0),
            sent: std::cell::RefCell::new(Vec::new()),
        });
        let c2 = c.clone();
        let open = move || {
            c2.opens.set(c2.opens.get() + 1);
            let mut inner = FakeHv::new();
            register_ident(&mut inner, 8, "SN-1");
            register_golden(&mut inner);
            Ok(Counted {
                inner,
                c: c2.clone(),
            })
        };
        (
            OnDemandHiVolta::new("hv", open, expect_serial.to_string()),
            c,
        )
    }

    /// poll 1 回 = open 1 回 + 要求 7 本 + close 1 回(TODO/119 §5/§6)。
    #[test]
    fn on_demand_poll_is_one_open_seven_requests_one_close() {
        let (mut driver, c) = counted_driver("");
        driver.ident().unwrap();
        c.sent.borrow_mut().clear();
        let (opens0, closes0) = (c.opens.get(), c.closes.get());

        let samples = driver.poll().unwrap();

        assert_eq!(samples.len(), 42);
        assert_eq!(c.opens.get() - opens0, 1);
        assert_eq!(c.closes.get() - closes0, 1, "poll の終わりで閉じている");
        let text = String::from_utf8(c.sent.borrow().clone()).unwrap();
        assert_eq!(text.matches("\r\n").count(), 7);
        assert!(text.starts_with("$CMD:MON,CH:8,PAR:VMON"));
    }

    /// open の回数 = ident と poll の回数の合計(間は保持しない)。ident は poll で繰り返さない。
    #[test]
    fn on_demand_opens_equal_the_number_of_ident_and_poll_calls() {
        let (mut driver, c) = counted_driver("SN-1");
        driver.ident().unwrap();
        assert_eq!((c.opens.get(), c.closes.get()), (1, 1));
        for _ in 0..3 {
            driver.poll().unwrap();
        }
        assert_eq!((c.opens.get(), c.closes.get()), (4, 4));
    }

    /// 開けなければ DriverError(silent にしない)。
    #[test]
    fn on_demand_open_failure_is_a_driver_error() {
        let open = || -> Result<Counted, DriverError> {
            Err(DriverError::Protocol("cannot open serial port x".into()))
        };
        let mut driver = OnDemandHiVolta::new("hv", open, String::new());
        assert!(matches!(driver.ident(), Err(DriverError::Protocol(_))));
        assert!(matches!(driver.poll(), Err(DriverError::Protocol(_))));
    }
}
