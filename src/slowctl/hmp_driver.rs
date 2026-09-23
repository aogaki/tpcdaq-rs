//! R&S/HAMEG HMP2020 ドライバ —— [`Driver`] 実装(TODO/089)。
//!
//! `hmp.rs`(TODO/085)の `ScpiLink` に USB CDC(`/dev/ttyACM*`)を差しただけのもの
//! (`bronkhorst.rs` / `arduino.rs` と同じ形)。**書き込みは無い**(085 と同じ安全境界)。
//!
//! # `INST:NSEL` の応答待ちについて(2026-09-23、報告事項)
//!
//! `ScpiLink::exchange`(085、変更禁止)は「1 行書いて 1 行読む」の対でしか提供されない。
//! `INST:NSEL n` は SCPI のクエリ(`?` 終端)ではないため、実機が何か応答を返す保証は無い
//! (むしろ SCPI の通例では非クエリコマンドは無応答)。`ScpiLink` に書き込み専用の経路が
//! 無く、`hmp.rs` を変更しない制約があるため、本実装は他に手段が無く `exchange` 経由で
//! `INST:NSEL` を送り、返ってきた行は意味を持たないものとして読み捨てる形にした
//! (fake には空行を登録する)。**実機が無応答だと毎 poll 周期 `READ_TIMEOUT_MS` 分
//! ブロックして `DriverError::Timeout` になる**——これは HMP2020 実機で検証していない
//! 部分なので、完了報告で明記する(発注書も実機受け入れは未実施と明記)。

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::time::Duration;

use tracing::info;

use super::hmp::{
    idn, meas_curr, meas_volt, nsel, outp_query, parse_bool01, parse_f64, parse_idn,
    parse_syst_err, syst_err, LinkError, ScpiLink, MAX_CH, MIN_CH,
};
use super::{now_ms, Driver, DriverError, Ident, Sample};

/// 読み出しタイムアウト(ミリ秒)。
pub const READ_TIMEOUT_MS: u64 = 2_000;
/// USB CDC のボーレート(発注書 §1: HO720 は仮想シリアルなので baud は名目)。
pub const BAUD_RATE: u32 = 9_600;

pub const PARAM_VOLT: &str = "volt";
pub const PARAM_CURR: &str = "curr";
pub const PARAM_OUTP: &str = "outp";

pub const UNIT_V: &str = "V";
pub const UNIT_A: &str = "A";
pub const UNIT_ON: &str = "on";

/// R&S/HAMEG HMP2020 ドライバ。`node` = チャンネル番号(1..=2、発注書 §1)。
pub struct HmpDriver<T: Read + Write> {
    device: String,
    link: ScpiLink<T>,
    /// 空でなければ `ident` で `*IDN?` の serial と照合する。
    expect_serial: String,
}

impl<T: Read + Write> HmpDriver<T> {
    pub fn new(device: impl Into<String>, io: T, expect_serial: String) -> Self {
        Self {
            device: device.into(),
            link: ScpiLink::new(io),
            expect_serial,
        }
    }
}

/// `LinkError` → `DriverError`(bronkhorst.rs / hivolta_driver.rs と同じ形)。
fn driver_error(error: LinkError) -> DriverError {
    match error {
        LinkError::Io(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock
                    | std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::UnexpectedEof
            ) =>
        {
            DriverError::Timeout
        }
        LinkError::Io(e) => DriverError::Io(e),
    }
}

impl<T: Read + Write> Driver for HmpDriver<T> {
    fn ident(&mut self) -> Result<Vec<Ident>, DriverError> {
        let reply = self.link.exchange(idn()).map_err(driver_error)?;
        let parsed = parse_idn(&reply).map_err(|e| DriverError::Protocol(e.to_string()))?;
        if !self.expect_serial.is_empty() && self.expect_serial != parsed.serial {
            return Err(DriverError::Identity {
                expected: self.expect_serial.clone(),
                got: parsed.serial,
            });
        }
        let mut fields = BTreeMap::new();
        fields.insert("vendor".to_string(), parsed.vendor.clone());
        fields.insert("model".to_string(), parsed.model.clone());
        fields.insert("serial".to_string(), parsed.serial.clone());
        fields.insert("fw".to_string(), parsed.fw.clone());

        info!(
            device = self.device,
            model = parsed.model,
            serial = parsed.serial,
            "slowctl: hmp2020 identified"
        );
        Ok(vec![Ident {
            device: self.device.clone(),
            node: None,
            fields,
        }])
    }

    fn poll(&mut self) -> Result<Vec<Sample>, DriverError> {
        let ts_ms = now_ms();
        let mut samples = Vec::with_capacity(6);

        for ch in MIN_CH..=MAX_CH {
            let nsel_cmd = nsel(ch).map_err(|e| DriverError::Protocol(e.to_string()))?;
            // 応答の中身は使わない(このファイル冒頭の doc 参照) —— 書けるのは exchange だけ。
            self.link.exchange(&nsel_cmd).map_err(driver_error)?;

            let volt_reply = self.link.exchange(meas_volt()).map_err(driver_error)?;
            let volt = parse_f64(&volt_reply).map_err(|e| DriverError::Protocol(e.to_string()))?;

            let curr_reply = self.link.exchange(meas_curr()).map_err(driver_error)?;
            let curr = parse_f64(&curr_reply).map_err(|e| DriverError::Protocol(e.to_string()))?;

            let outp_reply = self.link.exchange(outp_query()).map_err(driver_error)?;
            let outp =
                parse_bool01(&outp_reply).map_err(|e| DriverError::Protocol(e.to_string()))?;

            samples.push(Sample {
                device: self.device.clone(),
                node: Some(ch),
                param: PARAM_VOLT.to_string(),
                value: volt,
                unit: UNIT_V.to_string(),
                ts_ms,
                alarm: false,
            });
            samples.push(Sample {
                device: self.device.clone(),
                node: Some(ch),
                param: PARAM_CURR.to_string(),
                value: curr,
                unit: UNIT_A.to_string(),
                ts_ms,
                alarm: false,
            });
            samples.push(Sample {
                device: self.device.clone(),
                node: Some(ch),
                param: PARAM_OUTP.to_string(),
                value: if outp { 1.0 } else { 0.0 },
                unit: UNIT_ON.to_string(),
                ts_ms,
                alarm: false,
            });
        }

        let err_reply = self.link.exchange(syst_err()).map_err(driver_error)?;
        let (code, message) =
            parse_syst_err(&err_reply).map_err(|e| DriverError::Protocol(e.to_string()))?;
        if code != 0 {
            return Err(DriverError::Protocol(format!("SYST:ERR {code}: {message}")));
        }

        Ok(samples)
    }
}

// ---------------------------------------------------------------------
// 実シリアルポートの組み立て(唯一 IO に触る場所)
// ---------------------------------------------------------------------

/// `/dev/serial/by-id/…`(HO720 USB CDC)を開く(発注書 §1)。
pub fn open_serial(
    device: &str,
    port: &str,
    expect_serial: String,
) -> Result<HmpDriver<Box<dyn serialport::SerialPort>>, DriverError> {
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
        "slowctl: hmp2020 serial port open"
    );
    Ok(HmpDriver::new(device, handle, expect_serial))
}

// ---------------------------------------------------------------------
// テスト
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::slowctl::fake_hmp::FakeHmp;

    /// `INST:NSEL n` は固定応答(空行、このファイル冒頭の doc 参照)。
    /// `MEAS:VOLT?`/`MEAS:CURR?`/`OUTP?` は ch1 → ch2 の順で同じ要求文字列を再利用するので
    /// [`FakeHmp::push_response`](順番に消費するキュー)へ積む(ch を選ぶのは `INST:NSEL`
    /// であって要求文字列自体は変わらないため —— 実機の SCPI と同じ)。
    fn register_nsel_and_measurements(
        fake: &mut FakeHmp,
        ch: u8,
        volt: &str,
        curr: &str,
        outp: &str,
    ) {
        fake.set_response(format!("{}\n", nsel(ch).unwrap()), "\n");
        fake.push_response("MEAS:VOLT?\n", format!("{volt}\n"));
        fake.push_response("MEAS:CURR?\n", format!("{curr}\n"));
        fake.push_response("OUTP?\n", format!("{outp}\n"));
    }

    #[test]
    fn ident_reads_idn_and_keeps_all_four_fields() {
        let mut fake = FakeHmp::new();
        fake.set_response("*IDN?\n", "HAMEG,HMP2020,012345,HW1.0/SW2.0\n");
        let mut driver = HmpDriver::new("hv_hmp", fake, String::new());

        let idents = driver.ident().unwrap();
        assert_eq!(idents.len(), 1);
        assert_eq!(idents[0].device, "hv_hmp");
        assert_eq!(idents[0].node, None);
        assert_eq!(idents[0].fields["vendor"], "HAMEG");
        assert_eq!(idents[0].fields["model"], "HMP2020");
        assert_eq!(idents[0].fields["serial"], "012345");
        assert_eq!(idents[0].fields["fw"], "HW1.0/SW2.0");
    }

    /// `expect_serial` が空でなければ `*IDN?` の serial と照合する。不一致は `Identity`。
    #[test]
    fn ident_serial_mismatch_is_an_identity_error() {
        let mut fake = FakeHmp::new();
        fake.set_response("*IDN?\n", "HAMEG,HMP2020,012345,HW1.0/SW2.0\n");
        let mut driver = HmpDriver::new("hv_hmp", fake, "OTHER-SERIAL".to_string());

        assert!(matches!(
            driver.ident(),
            Err(DriverError::Identity { expected, got })
                if expected == "OTHER-SERIAL" && got == "012345"
        ));
    }

    /// ch1/ch2 それぞれ 3 サンプル(volt/curr/outp)、末尾に SYST:ERR = 0 を確認して通る
    /// (6 サンプル、発注書 §1/§2)。
    #[test]
    fn poll_of_both_channels_yields_six_samples_and_checks_syst_err_zero() {
        let mut fake = FakeHmp::new();
        register_nsel_and_measurements(&mut fake, 1, "5.000", "0.5000", "1");
        register_nsel_and_measurements(&mut fake, 2, "12.000", "1.2000", "0");
        fake.set_response("SYST:ERR?\n", "0, \"No error\"\n");

        let mut driver = HmpDriver::new("hv_hmp", fake, String::new());
        let samples = driver.poll().unwrap();

        assert_eq!(samples.len(), 6);
        assert_eq!(samples[0].node, Some(1));
        assert_eq!(samples[0].param, "volt");
        assert_eq!(samples[0].value, 5.0);
        assert_eq!(samples[0].unit, "V");
        assert_eq!(samples[1].param, "curr");
        assert_eq!(samples[1].value, 0.5);
        assert_eq!(samples[1].unit, "A");
        assert_eq!(samples[2].param, "outp");
        assert_eq!(samples[2].value, 1.0, "OUTP? 1 = ON");
        assert_eq!(samples[2].unit, "on");

        assert_eq!(samples[3].node, Some(2));
        assert_eq!(samples[3].value, 12.0);
        assert_eq!(samples[5].value, 0.0, "OUTP? 0 = OFF");
        assert!(
            samples.iter().all(|s| !s.alarm),
            "HMP に alarm は無い(発注書 §1)"
        );
    }

    /// `SYST:ERR?` が非 0 なら `Protocol`(発注書 §1)。
    #[test]
    fn a_nonzero_syst_err_is_a_protocol_error() {
        let mut fake = FakeHmp::new();
        register_nsel_and_measurements(&mut fake, 1, "5.000", "0.5000", "1");
        register_nsel_and_measurements(&mut fake, 2, "12.000", "1.2000", "0");
        fake.set_response("SYST:ERR?\n", "-102, \"Syntax error\"\n");

        let mut driver = HmpDriver::new("hv_hmp", fake, String::new());
        match driver.poll() {
            Err(DriverError::Protocol(message)) => {
                assert!(message.contains("-102"), "message = {message:?}");
                assert!(message.contains("Syntax error"), "message = {message:?}");
            }
            other => panic!("expected Protocol, got {other:?}"),
        }
    }

    /// 無応答は `Timeout`。
    #[test]
    fn a_silent_device_times_out() {
        let mut fake = FakeHmp::new();
        fake.set_silent();
        let mut driver = HmpDriver::new("hv_hmp", fake, String::new());
        assert!(matches!(driver.ident(), Err(DriverError::Timeout)));
    }

    /// HMP2020 は ch 1–2 のみ —— `nsel` の `Err` で自然に縛られる(生成段階、hmp.rs 側で
    /// 既にテスト済みだが、driver の `poll` が `MIN_CH..=MAX_CH` しか回さない根拠として明記)。
    #[test]
    fn only_channels_one_and_two_exist_for_the_hmp2020() {
        assert_eq!(MIN_CH, 1);
        assert_eq!(MAX_CH, 2);
        assert!(nsel(3).is_err());
        assert!(nsel(4).is_err());
    }
}
