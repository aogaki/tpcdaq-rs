//! Arduino MKR ENV Shield ドライバ —— [`Driver`] 実装(TODO/079 §2.3 / §2.7)。
//!
//! スケッチ(`tools/arduino_env/env_json.ino`)は 1 秒毎に JSON 1 行を**一方的に** push する
//! (問い合わせ不要)。よって [`Driver::poll`] は「要求して待つ」形ではなく「バッファに
//! 溜まった行を全部読み、**最後の完全な行**だけを使う」形になる(TODO/079 §2.7-2:
//! poll_hz が 1 Hz とずれても古い行に固定されない)。尻切れの末尾断片は捨てる
//! (次の poll で新しい行がまた来る —— モニタ系、CLAUDE.md の drop-with-count でよい)。
//!
//! **書き込みは無い**(Arduino は入力を読まない)。`T: Read + Write` は
//! `serialport::SerialPort` にそのまま差すための形式的な境界(bronkhorst.rs と同じ形)。

use std::collections::BTreeMap;
use std::io::{ErrorKind, Read, Write};
use std::time::Duration;

use serde::Deserialize;
use tracing::{info, warn};

use super::{now_ms, Driver, DriverError, Ident, Sample};

/// スケッチが名乗る `dev` 文字列(TODO/079 §2.7-1)。
pub const EXPECTED_DEV: &str = "mkr_env";

/// USB CDC のボーレート(TODO/079 §2.7-1: `Serial.begin(115200)`)。
pub const BAUD_RATE: u32 = 115_200;

/// 読み出しタイムアウト(ミリ秒)。1 行 ≈ 100 バイトは 115200 baud で 1 ms 未満なので、
/// スケッチの 1 秒周期に対して桁の余裕を持たせつつ、無応答を長く待ちすぎない値にする。
pub const READ_TIMEOUT_MS: u64 = 1_500;

/// `ident` が試す最大行数(TODO/079 §2.7-2「最大 3 行」)。ポートを開いた直後は
/// スケッチの出力の途中から読み始めることがあるので、壊れた最初の 1–2 行を
/// 読み飛ばす余地を持たせる(無応答そのものは即 `Timeout` —— リトライしない)。
const MAX_IDENT_LINES: u32 = 3;

/// 1 行の最大バイト数(終端 `\n` が来ない壊れた入力で無限に確保しないため)。
const MAX_LINE_BYTES: usize = 512;

pub const PARAM_T_C: &str = "t_c";
pub const PARAM_RH: &str = "rh";
pub const PARAM_P_HPA: &str = "p_hpa";
pub const PARAM_LUX: &str = "lux";
pub const PARAM_UVA: &str = "uva";
pub const PARAM_UVB: &str = "uvb";

pub const UNIT_DEGC: &str = "degC";
pub const UNIT_PERCENT: &str = "%";
pub const UNIT_HPA: &str = "hPa";
pub const UNIT_LUX: &str = "lux";
/// UV は較正単位が無いので生値をそのまま(TODO/079 §2.7-2 の表には無いが、
/// bronkhorst の valve output と同じ扱い —— 換算式が要るものは "raw" で通す)。
pub const UNIT_RAW: &str = "raw";

/// スケッチの JSON 行(TODO/079 §2.7-1)。数値項目は `error` 行に無いので Optional。
#[derive(Debug, Deserialize)]
struct EnvLine {
    dev: String,
    #[serde(default)]
    seq: Option<u64>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    t_c: Option<f64>,
    #[serde(default)]
    rh: Option<f64>,
    #[serde(default)]
    p_hpa: Option<f64>,
    #[serde(default)]
    lux: Option<f64>,
    #[serde(default)]
    uva: Option<f64>,
    #[serde(default)]
    uvb: Option<f64>,
}

/// Arduino MKR ENV Shield ドライバ。`node` は無い(SPEC §9.2 の `-`、[`super::NODELESS`])。
pub struct ArduinoEnvDriver<T: Read + Write> {
    device: String,
    io: T,
    last_seq: Option<u64>,
}

impl<T: Read + Write> ArduinoEnvDriver<T> {
    pub fn new(device: impl Into<String>, io: T) -> Self {
        Self {
            device: device.into(),
            io,
            last_seq: None,
        }
    }

    fn is_timeout(error: &std::io::Error) -> bool {
        matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
    }

    /// 1 行読む(`\n` まで、`\r` は剥がす)。読み切れなければ `Err`(silent 禁止)。
    /// 無応答(1 バイト目から詰まる)も、行の途中で詰まった場合も、区別せず `Timeout`
    /// にする —— どちらも「この呼び出しでは完全な行が取れなかった」という意味は同じ。
    fn read_one_line(&mut self) -> Result<String, DriverError> {
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            match self.io.read(&mut byte) {
                Ok(0) => return Err(DriverError::Timeout),
                Ok(_) => {
                    buf.push(byte[0]);
                    if buf.ends_with(b"\n") {
                        return Ok(decode_line(&buf));
                    }
                    if buf.len() > MAX_LINE_BYTES {
                        return Err(DriverError::Protocol(format!(
                            "line exceeds {MAX_LINE_BYTES} bytes without a terminator"
                        )));
                    }
                }
                Err(e) if Self::is_timeout(&e) => return Err(DriverError::Timeout),
                Err(e) => return Err(DriverError::Io(e)),
            }
        }
    }

    /// いま読める行を全部読み切る(以後は読めるものが無くなるまで —— タイムアウトで停止)。
    /// 終端の来ない末尾の断片は**捨てる**(次の呼び出しでまた 1 から読む。TODO/079 §2.7-2)。
    fn drain_lines(&mut self) -> Result<Vec<String>, DriverError> {
        let mut lines = Vec::new();
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            match self.io.read(&mut byte) {
                Ok(0) => break,
                Ok(_) => {
                    buf.push(byte[0]);
                    if buf.ends_with(b"\n") {
                        lines.push(decode_line(&buf));
                        buf.clear();
                    } else if buf.len() > MAX_LINE_BYTES {
                        warn!(
                            device = self.device,
                            "slowctl: arduino_env: dropping an over-long line without a terminator"
                        );
                        buf.clear();
                    }
                }
                Err(e) if Self::is_timeout(&e) => break,
                Err(e) => return Err(DriverError::Io(e)),
            }
        }
        Ok(lines)
    }
}

fn decode_line(buf: &[u8]) -> String {
    String::from_utf8_lossy(buf)
        .trim_end_matches(['\r', '\n'])
        .to_string()
}

impl<T: Read + Write> Driver for ArduinoEnvDriver<T> {
    fn ident(&mut self) -> Result<Vec<Ident>, DriverError> {
        let mut last_malformed: Option<String> = None;
        for _ in 0..MAX_IDENT_LINES {
            // 無応答はここで即座に `Timeout` として伝播する(リトライは壊れた内容にだけ効く)。
            let line = self.read_one_line()?;
            match serde_json::from_str::<EnvLine>(&line) {
                Ok(env) => {
                    if let Some(error) = env.error {
                        return Err(DriverError::Protocol(error));
                    }
                    if env.dev != EXPECTED_DEV {
                        return Err(DriverError::Identity {
                            expected: EXPECTED_DEV.to_string(),
                            got: env.dev,
                        });
                    }
                    let mut fields = BTreeMap::new();
                    fields.insert("dev".to_string(), env.dev.clone());
                    if let Some(seq) = env.seq {
                        fields.insert("seq".to_string(), seq.to_string());
                        self.last_seq = Some(seq);
                    }
                    info!(
                        device = self.device,
                        seq = env.seq,
                        "slowctl: arduino_env identified"
                    );
                    return Ok(vec![Ident {
                        device: self.device.clone(),
                        node: None,
                        fields,
                    }]);
                }
                Err(e) => {
                    warn!(
                        device = self.device,
                        line, error = %e,
                        "slowctl: arduino_env: malformed ident line — trying the next one"
                    );
                    last_malformed = Some(e.to_string());
                }
            }
        }
        Err(DriverError::Protocol(format!(
            "no valid ident line in {MAX_IDENT_LINES} attempts: {}",
            last_malformed.unwrap_or_else(|| "no data".to_string())
        )))
    }

    fn poll(&mut self) -> Result<Vec<Sample>, DriverError> {
        let lines = self.drain_lines()?;
        let Some(line) = lines.last() else {
            return Err(DriverError::Timeout);
        };
        let env: EnvLine = serde_json::from_str(line)
            .map_err(|e| DriverError::Protocol(format!("malformed line {line:?}: {e}")))?;
        if let Some(error) = env.error {
            return Err(DriverError::Protocol(error));
        }
        if let Some(seq) = env.seq {
            if let Some(last) = self.last_seq {
                let expected = last.wrapping_add(1);
                if seq != expected {
                    // 数えるだけ(エラーにしない)—— TODO/079 §2.7-2「seq の飛びは warn! で
                    // 可視化、数えるだけ、エラーにしない」。
                    warn!(
                        device = self.device,
                        last, seq, "slowctl: arduino_env: seq gap"
                    );
                }
            }
            self.last_seq = Some(seq);
        }

        let ts_ms = now_ms();
        let fields: [(&str, Option<f64>, &str); 6] = [
            (PARAM_T_C, env.t_c, UNIT_DEGC),
            (PARAM_RH, env.rh, UNIT_PERCENT),
            (PARAM_P_HPA, env.p_hpa, UNIT_HPA),
            (PARAM_LUX, env.lux, UNIT_LUX),
            (PARAM_UVA, env.uva, UNIT_RAW),
            (PARAM_UVB, env.uvb, UNIT_RAW),
        ];
        let mut samples = Vec::with_capacity(fields.len());
        for (param, value, unit) in fields {
            let Some(value) = value else {
                return Err(DriverError::Protocol(format!(
                    "missing field {param:?} in {line:?}"
                )));
            };
            samples.push(Sample {
                device: self.device.clone(),
                node: None,
                param: param.to_string(),
                value,
                unit: unit.to_string(),
                ts_ms,
                alarm: false,
            });
        }
        Ok(samples)
    }
}

// ---------------------------------------------------------------------
// 実シリアルポート版の組み立て(唯一 IO に触る場所)
// ---------------------------------------------------------------------

/// `/dev/serial/by-id/…` を開いて Arduino ENV ドライバを組む(TODO/079 §2.7-3)。
///
/// `expect_serial` は使わない —— 名乗り(JSON の `dev`)そのものが照合の手段なので
/// (`open_serial`(bronkhorst)の `expect_serial` に相当するものが要らない、TODO/079 §2.7-3)。
pub fn open_serial(
    device: &str,
    port: &str,
) -> Result<ArduinoEnvDriver<Box<dyn serialport::SerialPort>>, DriverError> {
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
        "slowctl: arduino_env serial port open"
    );
    Ok(ArduinoEnvDriver::new(device, handle))
}

// ---------------------------------------------------------------------
// テスト
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::slowctl::fake_arduino::FakeArduino;

    const NORMAL_LINE: &str = r#"{"dev":"mkr_env","seq":123,"t_c":24.31,"rh":41.2,"p_hpa":1003.7,"lux":120.5,"uva":0.12,"uvb":0.03}"#;

    // -------------------------------------------------------------
    // ident
    // -------------------------------------------------------------

    #[test]
    fn ident_succeeds_on_a_normal_first_line() {
        let mut fake = FakeArduino::new();
        fake.push_line(NORMAL_LINE);
        let mut driver = ArduinoEnvDriver::new("env", fake);

        let idents = driver.ident().unwrap();
        assert_eq!(idents.len(), 1);
        assert_eq!(idents[0].device, "env");
        assert_eq!(idents[0].node, None, "Arduino は node を持たない");
        assert_eq!(idents[0].fields["dev"], "mkr_env");
        assert_eq!(idents[0].fields["seq"], "123");
    }

    /// TODO/079 §2.7-2: `"error"` 行なら `DriverError::Protocol(<error 文言>)`。
    #[test]
    fn ident_fails_with_protocol_error_on_an_error_line() {
        let mut fake = FakeArduino::new();
        fake.push_line(r#"{"dev":"mkr_env","error":"ENV shield not found"}"#);
        let mut driver = ArduinoEnvDriver::new("env", fake);

        match driver.ident() {
            Err(DriverError::Protocol(message)) => {
                assert_eq!(message, "ENV shield not found");
            }
            other => panic!("expected Protocol, got {other:?}"),
        }
    }

    /// TODO/079 §2.7-2: `dev` が違えば `DriverError::Identity { expected, got }`。
    #[test]
    fn ident_fails_with_identity_mismatch_on_the_wrong_dev() {
        let mut fake = FakeArduino::new();
        fake.push_line(r#"{"dev":"some_other_device","seq":1}"#);
        let mut driver = ArduinoEnvDriver::new("env", fake);

        assert!(matches!(
            driver.ident(),
            Err(DriverError::Identity { expected, got })
                if expected == "mkr_env" && got == "some_other_device"
        ));
    }

    /// 開いた直後にスケッチの出力の途中から読み始めても、最大 3 行まで読み飛ばして
    /// 有効な行に辿り着く(TODO/079 §2.7-2「最大 3 行」)。
    #[test]
    fn ident_skips_malformed_lines_up_to_three_attempts_then_succeeds() {
        let mut fake = FakeArduino::new();
        fake.push_line("t\":24.31}"); // 途中から読んだ壊れた断片
        fake.push_line("not json at all");
        fake.push_line(NORMAL_LINE);
        let mut driver = ArduinoEnvDriver::new("env", fake);

        let idents = driver.ident().unwrap();
        assert_eq!(idents[0].fields["seq"], "123");
    }

    /// 3 行連続で壊れていれば `Protocol`(無限にリトライしない)。
    #[test]
    fn ident_fails_after_three_malformed_lines() {
        let mut fake = FakeArduino::new();
        fake.push_line("garbage one");
        fake.push_line("garbage two");
        fake.push_line("garbage three");
        let mut driver = ArduinoEnvDriver::new("env", fake);

        assert!(matches!(driver.ident(), Err(DriverError::Protocol(_))));
    }

    /// 何も来なければ `Timeout`(リトライしない —— 無応答は壊れた内容ではない)。
    #[test]
    fn ident_times_out_when_no_line_arrives() {
        let fake = FakeArduino::new();
        // 何も push しない。
        let mut driver = ArduinoEnvDriver::new("env", fake);
        assert!(matches!(driver.ident(), Err(DriverError::Timeout)));
    }

    // -------------------------------------------------------------
    // poll
    // -------------------------------------------------------------

    #[test]
    fn poll_of_a_normal_line_yields_six_samples_with_units() {
        let mut fake = FakeArduino::new();
        fake.push_line(NORMAL_LINE);
        let mut driver = ArduinoEnvDriver::new("env", fake);

        let samples = driver.poll().unwrap();

        assert_eq!(samples.len(), 6);
        let names: Vec<&str> = samples.iter().map(|s| s.param.as_str()).collect();
        assert_eq!(names, vec!["t_c", "rh", "p_hpa", "lux", "uva", "uvb"]);
        assert_eq!(samples[0].value, 24.31);
        assert_eq!(samples[0].unit, "degC");
        assert_eq!(samples[1].value, 41.2);
        assert_eq!(samples[1].unit, "%");
        assert_eq!(samples[2].value, 1003.7);
        assert_eq!(samples[2].unit, "hPa");
        assert_eq!(samples[3].value, 120.5);
        assert_eq!(samples[3].unit, "lux");
        assert_eq!(samples[4].value, 0.12);
        assert_eq!(samples[4].unit, "raw");
        assert_eq!(samples[5].value, 0.03);
        assert_eq!(samples[5].unit, "raw");
        assert!(samples.iter().all(|s| s.node.is_none()));
        assert!(samples.iter().all(|s| !s.alarm));
        assert_eq!(
            samples[0].key(),
            "env/-/t_c",
            "node 無しは \"-\"(SPEC §9.2)"
        );
    }

    /// poll_hz が 1 Hz とずれてバッファに複数行溜まっていても、**最後の行だけ**を使う
    /// (TODO/079 §2.7-2)。
    #[test]
    fn poll_uses_only_the_last_complete_line_when_several_are_buffered() {
        let mut fake = FakeArduino::new();
        fake.push_line(r#"{"dev":"mkr_env","seq":1,"t_c":10.0,"rh":10.0,"p_hpa":900.0,"lux":1.0,"uva":0.0,"uvb":0.0}"#);
        fake.push_line(r#"{"dev":"mkr_env","seq":2,"t_c":20.0,"rh":20.0,"p_hpa":901.0,"lux":2.0,"uva":0.1,"uvb":0.1}"#);
        fake.push_line(NORMAL_LINE); // seq 123、これが「最後の行」
        let mut driver = ArduinoEnvDriver::new("env", fake);

        let samples = driver.poll().unwrap();
        assert_eq!(samples[0].value, 24.31, "最後(3 行目)の t_c を使う");
    }

    /// 尻切れの末尾断片(終端 `\n` が来ていない)は使わない(次の poll でまた最初から)。
    #[test]
    fn a_trailing_partial_line_is_not_used() {
        let mut fake = FakeArduino::new();
        fake.push_line(NORMAL_LINE);
        fake.push_raw(br#"{"dev":"mkr_env","seq":124,"t_c":"#); // 終端 \n が無い断片
        let mut driver = ArduinoEnvDriver::new("env", fake);

        let samples = driver.poll().unwrap();
        assert_eq!(samples[0].value, 24.31, "完全な行(NORMAL_LINE)を使う");
    }

    #[test]
    fn malformed_json_is_a_protocol_error() {
        let mut fake = FakeArduino::new();
        fake.push_line("this is not json");
        let mut driver = ArduinoEnvDriver::new("env", fake);

        assert!(matches!(driver.poll(), Err(DriverError::Protocol(_))));
    }

    #[test]
    fn a_missing_field_is_a_protocol_error() {
        let mut fake = FakeArduino::new();
        // uvb が無い。
        fake.push_line(
            r#"{"dev":"mkr_env","seq":1,"t_c":24.31,"rh":41.2,"p_hpa":1003.7,"lux":120.5,"uva":0.12}"#,
        );
        let mut driver = ArduinoEnvDriver::new("env", fake);

        match driver.poll() {
            Err(DriverError::Protocol(message)) => {
                assert!(message.contains("uvb"), "message = {message:?}");
            }
            other => panic!("expected Protocol, got {other:?}"),
        }
    }

    /// poll 中に `error` 行が来ても(欠測フィールドと同じ扱いで)`Protocol` になる。
    #[test]
    fn an_error_line_during_poll_is_a_protocol_error() {
        let mut fake = FakeArduino::new();
        fake.push_line(r#"{"dev":"mkr_env","error":"ENV shield not found"}"#);
        let mut driver = ArduinoEnvDriver::new("env", fake);

        match driver.poll() {
            Err(DriverError::Protocol(message)) => {
                assert_eq!(message, "ENV shield not found");
            }
            other => panic!("expected Protocol, got {other:?}"),
        }
    }

    #[test]
    fn poll_times_out_when_no_line_arrives() {
        let fake = FakeArduino::new();
        let mut driver = ArduinoEnvDriver::new("env", fake);
        assert!(matches!(driver.poll(), Err(DriverError::Timeout)));
    }

    /// seq の飛びはエラーにしない(warn! で可視化するだけ —— ここでは「落ちない」ことを確認)。
    #[test]
    fn a_seq_gap_does_not_fail_the_poll() {
        let mut fake = FakeArduino::new();
        fake.push_line(r#"{"dev":"mkr_env","seq":1,"t_c":1.0,"rh":1.0,"p_hpa":1.0,"lux":1.0,"uva":1.0,"uvb":1.0}"#);
        let mut driver = ArduinoEnvDriver::new("env", fake);
        driver.poll().unwrap();

        // 次の行が seq=1 ではなく seq=9(飛んでいる)。
        driver.io.push_line(
            r#"{"dev":"mkr_env","seq":9,"t_c":2.0,"rh":2.0,"p_hpa":2.0,"lux":2.0,"uva":2.0,"uvb":2.0}"#,
        );
        let samples = driver.poll().unwrap();
        assert_eq!(samples[0].value, 2.0, "seq が飛んでも値は普通に読める");
    }
}
