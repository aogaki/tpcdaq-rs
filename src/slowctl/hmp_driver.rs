//! R&S/HAMEG HMP2020 ドライバ —— [`Driver`] 実装(TODO/089)。
//!
//! `hmp.rs`(TODO/085)の `ScpiLink` に USB CDC(`/dev/ttyACM*`)を差しただけのもの
//! (`bronkhorst.rs` / `arduino.rs` と同じ形)。**書くのは出力の ON / OFF だけ**
//! (SPEC §11 v1.42、TODO/124 —— [`Driver::set_power`]、コマンドはコーデックの
//! [`outp_set`] だけ)。
//!
//! # power ON / OFF の手順(TODO/124 §1 + Fable 7)
//!
//! 両 ch の `OUTP?`(before)→ `INST:NSEL n` + `INST:NSEL?` で確認 → `OUTP ON|OFF` →
//! 両 ch の `OUTP?`(after)→ `SYST:ERR?` が 0。`OUTP ON` は選んだ ch と出力(マスタ)を同時に
//! ON にするので、ほかの ch が変わったことを隠さないよう**両 ch**を返す。各 ch の読み出しも
//! `INST:NSEL?` で選択を確かめてから行う。
//!
//! # `INST:NSEL` の送り方(2026-09-23、レビュー反映)
//!
//! `INST:NSEL n` は SCPI のクエリ(`?` 終端)ではないため、実機が応答を返す保証が無い。
//! 089 の初回実装では `ScpiLink::exchange`(書いて 1 行読む)しか使えず、応答を読み捨てる
//! 形にしていたが、レビューで `ScpiLink::send`(書くだけ、応答を待たない)が解禁されたため
//! 差し替えた: `send("INST:NSEL n")` の後に `exchange("INST:NSEL?")` で選択を確認し、
//! 返りが `n` と一致しなければ `DriverError::Protocol`(黙って別 ch の値を読まない)。
//! **`INST:NSEL?` が実機に存在するかは未確認**(`hmp.rs::nsel_query` の doc 参照)——
//! 実機受け入れは本チケットの範囲外(未実施と明記)。

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::time::Duration;

use serde_json::json;
use tracing::info;

use super::hmp::{
    curr_query, idn, meas_curr, meas_volt, nsel, nsel_query, outp_query, outp_set, parse_bool01,
    parse_f64, parse_idn, parse_syst_err, syst_err, volt_query, LinkError, ScpiLink, MAX_CH,
    MIN_CH,
};
use super::{now_ms, Driver, DriverError, Ident, PowerOutcome, Sample};

/// 読み出しタイムアウト(ミリ秒)。
pub const READ_TIMEOUT_MS: u64 = 2_000;
/// USB CDC のボーレート(発注書 §1: HO720 は仮想シリアルなので baud は名目)。
pub const BAUD_RATE: u32 = 9_600;

pub const PARAM_VOLT: &str = "volt";
pub const PARAM_CURR: &str = "curr";
pub const PARAM_OUTP: &str = "outp";
pub const PARAM_VSET: &str = "vset";
pub const PARAM_ISET: &str = "iset";

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

impl<T: Read + Write> HmpDriver<T> {
    /// `INST:NSEL ch` を送り、`INST:NSEL?` で選択が効いたことを確かめる ——
    /// 黙って別 ch を読んだり切り替えたりしない(CLAUDE.md)。
    fn select(&mut self, ch: u8) -> Result<(), DriverError> {
        let nsel_cmd = nsel(ch).map_err(|e| DriverError::Protocol(e.to_string()))?;
        // 非クエリなので送るだけ(応答は待たない、このファイル冒頭の doc 参照)。
        self.link.send(&nsel_cmd).map_err(driver_error)?;

        let selected_reply = self.link.exchange(nsel_query()).map_err(driver_error)?;
        let selected: u8 = selected_reply.trim().parse().map_err(|_| {
            DriverError::Protocol(format!(
                "INST:NSEL?: not a channel number: {selected_reply:?}"
            ))
        })?;
        if selected != ch {
            return Err(DriverError::Protocol(format!(
                "INST:NSEL {ch} did not take effect (INST:NSEL? reports {selected})"
            )));
        }
        Ok(())
    }

    /// 両 ch の `OUTP?` を `{"outp": {"1": 0|1, "2": 0|1}}` で返す(TODO/124 Fable 7)。
    fn read_outputs(&mut self) -> Result<serde_json::Value, DriverError> {
        let mut outputs = serde_json::Map::new();
        for ch in MIN_CH..=MAX_CH {
            self.select(ch)?;
            let reply = self.link.exchange(outp_query()).map_err(driver_error)?;
            let on = parse_bool01(&reply).map_err(|e| DriverError::Protocol(e.to_string()))?;
            outputs.insert(ch.to_string(), json!(u8::from(on)));
        }
        Ok(json!({ "outp": outputs }))
    }

    /// before を読んだ後の手順(選択 → `OUTP ON|OFF` → after → `SYST:ERR?`)。
    /// `Err` は応答の `error` そのもの。
    fn switch(&mut self, ch: u8, on: bool) -> Result<serde_json::Value, String> {
        self.select(ch)
            .map_err(|e| format!("cannot select ch {ch} ({e}) — nothing was switched"))?;
        self.link
            .send(outp_set(on))
            .map_err(|e| format!("sending {} failed ({e})", outp_set(on)))?;
        let after = self.read_outputs().map_err(|e| {
            format!(
                "{} was sent but reading OUTP? back failed ({e})",
                outp_set(on)
            )
        })?;
        let err_reply = self
            .link
            .exchange(syst_err())
            .map_err(|e| format!("SYST:ERR? failed ({})", driver_error(e)))?;
        let (code, message) = parse_syst_err(&err_reply).map_err(|e| e.to_string())?;
        if code != 0 {
            return Err(format!("SYST:ERR {code}: {message}"));
        }
        Ok(after)
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
        let mut samples = Vec::with_capacity(10);

        for ch in MIN_CH..=MAX_CH {
            // 選択が効いたかを確認してから読む —— 黙って別 ch の値を読まない(CLAUDE.md)。
            self.select(ch)?;

            let volt_reply = self.link.exchange(meas_volt()).map_err(driver_error)?;
            let volt = parse_f64(&volt_reply).map_err(|e| DriverError::Protocol(e.to_string()))?;

            let curr_reply = self.link.exchange(meas_curr()).map_err(driver_error)?;
            let curr = parse_f64(&curr_reply).map_err(|e| DriverError::Protocol(e.to_string()))?;

            let outp_reply = self.link.exchange(outp_query()).map_err(driver_error)?;
            let outp =
                parse_bool01(&outp_reply).map_err(|e| DriverError::Protocol(e.to_string()))?;

            // 設定値の読み出し(TODO/120、問い合わせのみ)。
            let vset_reply = self.link.exchange(volt_query()).map_err(driver_error)?;
            let vset = parse_f64(&vset_reply).map_err(|e| DriverError::Protocol(e.to_string()))?;
            let iset_reply = self.link.exchange(curr_query()).map_err(driver_error)?;
            let iset = parse_f64(&iset_reply).map_err(|e| DriverError::Protocol(e.to_string()))?;

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
            samples.push(Sample {
                device: self.device.clone(),
                node: Some(ch),
                param: PARAM_VSET.to_string(),
                value: vset,
                unit: UNIT_V.to_string(),
                ts_ms,
                alarm: false,
            });
            samples.push(Sample {
                device: self.device.clone(),
                node: Some(ch),
                param: PARAM_ISET.to_string(),
                value: iset,
                unit: UNIT_A.to_string(),
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

    /// TODO/124 §1 + Fable 7。before が読めなければ `Err`(応答の before は null)。
    fn set_power(&mut self, node: u8, on: bool) -> Result<PowerOutcome, DriverError> {
        if !(MIN_CH..=MAX_CH).contains(&node) {
            return Err(DriverError::Protocol(format!(
                "ch {node} is out of range (HMP2020: {MIN_CH}..={MAX_CH})"
            )));
        }
        let before = self.read_outputs()?;
        let result = self.switch(node, on);
        Ok(PowerOutcome { before, result })
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

    /// `INST:NSEL n` は fake が状態として持つので登録不要(`INST:NSEL?` はその状態を
    /// 読み戻す、TODO/089 レビュー)。`MEAS:VOLT?`/`MEAS:CURR?`/`OUTP?` は ch1 → ch2 の順で
    /// 同じ要求文字列を再利用するので [`FakeHmp::push_response`](順番に消費するキュー)へ積む。
    fn register_measurements(fake: &mut FakeHmp, volt: &str, curr: &str, outp: &str) {
        // 設定値(TODO/120)。測定値から導いた非対称な値(実機未採取、形式は MEAS と同じと仮定)。
        let vset = volt.parse::<f64>().unwrap() + 0.5;
        let iset = curr.parse::<f64>().unwrap() * 2.0;
        fake.push_response("VOLT?\n", format!("{vset}\n"));
        fake.push_response("CURR?\n", format!("{iset}\n"));
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

    /// ch1/ch2 それぞれ 5 サンプル(volt/curr/outp/vset/iset)、末尾に SYST:ERR = 0 を確認して通る
    /// (10 サンプル、発注書 §1/§2)。
    #[test]
    fn poll_of_both_channels_yields_ten_samples_and_checks_syst_err_zero() {
        let mut fake = FakeHmp::new();
        register_measurements(&mut fake, "5.000", "0.5000", "1");
        register_measurements(&mut fake, "12.000", "1.2000", "0");
        fake.set_response("SYST:ERR?\n", "0, \"No error\"\n");

        let mut driver = HmpDriver::new("hv_hmp", fake, String::new());
        let samples = driver.poll().unwrap();

        assert_eq!(samples.len(), 10);
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

        // ch1 の設定値(VOLT? = 5.0 + 0.5、CURR? = 0.5 * 2)。
        assert_eq!(samples[3].param, "vset");
        assert_eq!(samples[3].value, 5.5);
        assert_eq!(samples[3].unit, "V");
        assert_eq!(samples[4].param, "iset");
        assert_eq!(samples[4].value, 1.0);
        assert_eq!(samples[4].unit, "A");

        assert_eq!(samples[5].node, Some(2));
        assert_eq!(samples[5].value, 12.0);
        assert_eq!(samples[7].value, 0.0, "OUTP? 0 = OFF");
        // ch2 の設定値(12.0 + 0.5、1.2 * 2)。
        assert_eq!(samples[8].param, "vset");
        assert_eq!(samples[8].value, 12.5);
        assert_eq!(samples[9].param, "iset");
        assert_eq!(samples[9].value, 2.4);
        assert!(
            samples.iter().all(|s| !s.alarm),
            "HMP に alarm は無い(発注書 §1)"
        );
    }

    /// `INST:NSEL n` が効かなかった(実機が別 ch を報告する)場合は `Protocol` —— 黙って
    /// 別 ch の値を読まない(TODO/089 レビュー 2026-09-23)。
    #[test]
    fn a_selection_that_did_not_take_effect_is_a_protocol_error() {
        let mut fake = FakeHmp::new();
        // `INST:NSEL?` の答えを強制的に ch1 の選択と食い違わせる(実機が選択を無視した体)。
        fake.push_response("INST:NSEL?\n", "9\n");

        let mut driver = HmpDriver::new("hv_hmp", fake, String::new());
        match driver.poll() {
            Err(DriverError::Protocol(message)) => {
                assert!(message.contains("INST:NSEL 1"), "message = {message:?}");
                assert!(message.contains('9'), "message = {message:?}");
            }
            other => panic!("expected Protocol, got {other:?}"),
        }
    }

    /// `SYST:ERR?` が非 0 なら `Protocol`(発注書 §1)。
    #[test]
    fn a_nonzero_syst_err_is_a_protocol_error() {
        let mut fake = FakeHmp::new();
        register_measurements(&mut fake, "5.000", "0.5000", "1");
        register_measurements(&mut fake, "12.000", "1.2000", "0");
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

    // -----------------------------------------------------------------
    // TODO/124: 出力の ON / OFF(Fable 7 = 両 ch の before / after)
    // -----------------------------------------------------------------

    /// 送信行を記録する薄いラッパ(FakeHmp は Link に所有されるので外から見えない)。
    struct Tap {
        inner: FakeHmp,
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

    fn tapped(fake: FakeHmp) -> (HmpDriver<Tap>, std::rc::Rc<std::cell::RefCell<Vec<u8>>>) {
        let sent = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let io = Tap {
            inner: fake,
            sent: sent.clone(),
        };
        (HmpDriver::new("lv", io, String::new()), sent)
    }

    const NO_ERROR: &str = "0, \"No error\"\n";

    /// ch2 を ON: 両 ch を読む(before)→ ch2 を選んで `OUTP ON` → 両 ch を読む(after)→
    /// `SYST:ERR?` = 0。ch1(もともと ON)は変わらない。
    #[test]
    fn set_power_on_ch2_reads_both_channels_before_and_after() {
        let mut fake = FakeHmp::new();
        fake.set_output(1, true);
        fake.set_response("SYST:ERR?\n", NO_ERROR);
        let (mut driver, sent) = tapped(fake);

        let outcome = driver.set_power(2, true).unwrap();

        assert_eq!(
            String::from_utf8(sent.borrow().clone()).unwrap(),
            "INST:NSEL 1\nINST:NSEL?\nOUTP?\nINST:NSEL 2\nINST:NSEL?\nOUTP?\n\
             INST:NSEL 2\nINST:NSEL?\nOUTP ON\n\
             INST:NSEL 1\nINST:NSEL?\nOUTP?\nINST:NSEL 2\nINST:NSEL?\nOUTP?\n\
             SYST:ERR?\n"
        );
        assert_eq!(outcome.before, json!({"outp": {"1": 1, "2": 0}}));
        assert_eq!(outcome.result, Ok(json!({"outp": {"1": 1, "2": 1}})));
    }

    /// ch1 を OFF: ch2 は ON のまま(`OUTP OFF` は選んだ ch だけ)。
    #[test]
    fn set_power_off_ch1_leaves_ch2_as_it_was() {
        let mut fake = FakeHmp::new();
        fake.set_output(1, true);
        fake.set_output(2, true);
        fake.set_response("SYST:ERR?\n", NO_ERROR);
        let (mut driver, sent) = tapped(fake);

        let outcome = driver.set_power(1, false).unwrap();

        assert!(String::from_utf8(sent.borrow().clone())
            .unwrap()
            .contains("INST:NSEL 1\nINST:NSEL?\nOUTP OFF\n"));
        assert_eq!(outcome.before, json!({"outp": {"1": 1, "2": 1}}));
        assert_eq!(outcome.result, Ok(json!({"outp": {"1": 0, "2": 1}})));
    }

    /// 切り替え後の `SYST:ERR?` が非 0 ならエラー(before は残る)。
    #[test]
    fn a_nonzero_syst_err_after_the_switch_is_an_error_with_the_before() {
        let mut fake = FakeHmp::new();
        fake.set_response("SYST:ERR?\n", "-221, \"Settings conflict\"\n");
        let (mut driver, _sent) = tapped(fake);

        let outcome = driver.set_power(2, true).unwrap();

        assert_eq!(outcome.before, json!({"outp": {"1": 0, "2": 0}}));
        assert_eq!(
            outcome.result,
            Err("SYST:ERR -221: Settings conflict".to_string())
        );
    }

    /// ch は 1–2 だけ。範囲外は何も送らずに拒否。
    #[test]
    fn a_channel_outside_1_and_2_is_refused_before_anything_is_sent() {
        for ch in [0u8, 3] {
            let (mut driver, sent) = tapped(FakeHmp::new());
            assert!(matches!(
                driver.set_power(ch, true),
                Err(DriverError::Protocol(_))
            ));
            assert!(sent.borrow().is_empty(), "ch {ch}: 何も送っていない");
        }
    }

    /// before を読む途中で選択が効かなければ `Err`(応答の before は null)。`OUTP` は送らない。
    #[test]
    fn a_selection_failure_while_reading_the_before_is_a_driver_error() {
        let mut fake = FakeHmp::new();
        fake.push_response("INST:NSEL?\n", "9\n");
        let (mut driver, sent) = tapped(fake);

        assert!(matches!(
            driver.set_power(2, true),
            Err(DriverError::Protocol(_))
        ));
        assert!(!String::from_utf8(sent.borrow().clone())
            .unwrap()
            .contains("OUTP ON"));
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
