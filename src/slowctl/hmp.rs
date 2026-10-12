//! R&S/HAMEG HMP2020 読み出しコーデック(SCPI Programmers Manual EN、USB CDC `/dev/ttyACM*`)。
//! **純関数のみ(IO 無し)**: 監視コマンドの生成と応答の解析だけを持つ。
//!
//! # 監視のみ(SPEC §11 安全境界)
//!
//! `[SOURce:]VOLTage`/`[SOURce:]CURRent`/`OUTPut[:STATe] ON` 等の**設定系コマンドを生成する
//! 関数は無い**。`nsel()` は電圧/電流を書き換えず、後続の `MEAS:*?`/`OUTP?` が読む対象チャンネルを
//! 選ぶだけ(SCPI で「別の instrument を選ぶ」だけの操作 — 9.17.027 の ProPar と同様、読み出し
//! チェーンの一部として許容する)。
//!
//! # コマンド長短形(SCPI Programmers Manual p.9)
//!
//! 「短縮形は大文字、完全形は全体」— 短縮形か完全形のどちらかしか送れない(p.9 §2.1.1
//! "Long and short form")。本モジュールは短縮形を生成する: `*IDN?`、`INST:NSEL {ch}`
//! (p.22 の実例 `INST:NSEL 1` そのもの)、`MEAS:VOLT?` / `MEAS:CURR?`(p.9 の実例
//! "MEASure:CURRent? is equivalent to MEAS:CURR?"、p.31 のコマンド表)、`OUTP?`(p.26)、
//! `SYST:ERR?`(p.20)。
//!
//! # 終端(p.9 §1.1 NOTICE)
//!
//! 「The end character must be set to linefeed (LF) or carriage return + linefeed (CR-LF).」
//! — 要求側はどちらでもよい。本モジュールは LF のみを付けて送る(KISS)。応答側は LF/CRLF の
//! どちらでも受理する([`ScpiLink::exchange`] 参照)。
//!
//! # HMP2020 は ch 1–2 のみ(p.22 NOTICE)
//!
//! 「HMP2020: :NSELect {3} and :NSELect {4} are not available」。

use std::io::{self, Read, Write};

/// HMP2020 で有効なチャンネル番号の下限(SCPI manual p.22 NOTICE)。
pub const MIN_CH: u8 = 1;
/// HMP2020 で有効なチャンネル番号の上限(同上 — ch 3/4 は HMP2020 に存在しない)。
pub const MAX_CH: u8 = 2;

/// [`nsel`]/[`parse_idn`]/[`parse_f64`]/[`parse_bool01`]/[`parse_syst_err`] の失敗要因。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HmpError {
    /// `nsel(ch)` に HMP2020 に存在しないチャンネルを渡した(p.22 NOTICE)。
    #[error("hmp2020 channel {0} is out of range (valid: 1..=2)")]
    ChannelOutOfRange(u8),
    /// 応答が期待した形(`*IDN?` の 4 フィールド、数値、`0`/`1`、`<code>,"<msg>"`)と違う。
    #[error("hmp malformed response: {0}")]
    Malformed(String),
}

/// `*IDN?` 要求文字列(p.18)。
pub fn idn() -> &'static str {
    "*IDN?"
}

/// `INST:NSEL <ch>` 要求文字列を組み立てる(p.22 の実例 `INST:NSEL 1` と同じ書式)。
/// HMP2020 に存在しない ch(3, 4, または 1..=2 の範囲外)は `Err`。
pub fn nsel(ch: u8) -> Result<String, HmpError> {
    if !(MIN_CH..=MAX_CH).contains(&ch) {
        return Err(HmpError::ChannelOutOfRange(ch));
    }
    Ok(format!("INST:NSEL {ch}"))
}

/// `MEAS:VOLT?` 要求文字列(p.9 短縮形規則 + p.31 コマンド表)。
pub fn meas_volt() -> &'static str {
    "MEAS:VOLT?"
}

/// `MEAS:CURR?` 要求文字列(p.9 実例 "MEASure:CURRent? is equivalent to MEAS:CURR?"、p.31 コマンド表)。
pub fn meas_curr() -> &'static str {
    "MEAS:CURR?"
}

/// `VOLT?` 要求文字列 —— **設定値**の読み出し(p.23 `[SOURce:]VOLTage…?`、応答例 `10.000`)。
/// 問い合わせのみ。設定コマンド(`VOLT <値>`)を作る関数は作らない(SPEC §11、TODO/120)。
pub fn volt_query() -> &'static str {
    "VOLT?"
}

/// `CURR?` 要求文字列 —— **設定値**の読み出し(p.24、応答例 `5.0000`)。問い合わせのみ。
pub fn curr_query() -> &'static str {
    "CURR?"
}

/// `OUTP?` 要求文字列(p.9 短縮形規則 + p.26 コマンド表 `OUTPut[:STATe]?`)。
pub fn outp_query() -> &'static str {
    "OUTP?"
}

/// `SYST:ERR?` 要求文字列(p.9 短縮形規則 + p.20 コマンド表 `SYSTem:ERRor[:NEXT]?`)。
pub fn syst_err() -> &'static str {
    "SYST:ERR?"
}

/// `INST:NSEL?` 要求文字列 —— 現在選択中のチャンネルを読み戻すクエリ形(SCPI は多くの
/// `[SOURce]/INSTrument` 系設定コマンドに同名の `?` クエリを持つ —— 手元にページ番号の
/// 確証は無いが、実機の応答有無を確認する以外に選択が効いたことを知る手段が無いため
/// TODO/089 レビュー(2026-09-23)で導入した。**実機での存在確認は未実施**、`nsel()` は
/// 非クエリなので実機が応答する保証が無く、[`ScpiLink::send`] で送った後にこのクエリで
/// 選択を確認する設計にした。
pub fn nsel_query() -> &'static str {
    "INST:NSEL?"
}

/// `*IDN?` の応答(p.18: `HAMEG,‹device type›,‹serial number›,‹firmwareversion›`)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Idn {
    pub vendor: String,
    pub model: String,
    pub serial: String,
    pub fw: String,
}

/// `*IDN?` の応答行を復号する(カンマ区切り 4 フィールド固定)。
pub fn parse_idn(line: &str) -> Result<Idn, HmpError> {
    let trimmed = line.trim();
    let parts: Vec<&str> = trimmed.splitn(4, ',').collect();
    match parts.as_slice() {
        [vendor, model, serial, fw] => Ok(Idn {
            vendor: vendor.to_string(),
            model: model.to_string(),
            serial: serial.to_string(),
            fw: fw.to_string(),
        }),
        _ => Err(HmpError::Malformed(format!(
            "expected 4 comma-separated fields, got {line:?}"
        ))),
    }
}

/// `MEAS:VOLT?`/`MEAS:CURR?` の応答(p.31: `e.g. 1.0000` / `e.g. 1.000`)を復号する。
pub fn parse_f64(line: &str) -> Result<f64, HmpError> {
    line.trim()
        .parse::<f64>()
        .map_err(|_| HmpError::Malformed(format!("not a number: {line:?}")))
}

/// `OUTP?` の応答(p.26: `1` = ON / `0` = OFF)を復号する。
pub fn parse_bool01(line: &str) -> Result<bool, HmpError> {
    match line.trim() {
        "1" => Ok(true),
        "0" => Ok(false),
        other => Err(HmpError::Malformed(format!(
            "expected '0' or '1', got {other:?}"
        ))),
    }
}

/// `SYST:ERR?` の応答(p.20: `<code>, "<message>"`。例 `0, "No error"`)を復号する。
pub fn parse_syst_err(line: &str) -> Result<(i32, String), HmpError> {
    let trimmed = line.trim();
    let (code_str, msg_str) = trimmed
        .split_once(',')
        .ok_or_else(|| HmpError::Malformed(format!("missing ',': {line:?}")))?;
    let code = code_str
        .trim()
        .parse::<i32>()
        .map_err(|_| HmpError::Malformed(format!("bad error code: {code_str:?}")))?;
    let msg = msg_str.trim().trim_matches('"').to_string();
    Ok((code, msg))
}

/// [`ScpiLink::exchange`] の失敗要因(IO エラーを吸収する)。
#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
}

/// 同期 SCPI リンク。1 要求送って 1 応答(LF まで、CR は許容して剥がす)を読む。トランスポート
/// (USB CDC `/dev/ttyACM*`)は呼び出し側が `T` として渡す(079 §2.5 の `ProParLink` と同じ形。
/// component 配線は別チケット — 本票では触らない)。
pub struct ScpiLink<T: Read + Write> {
    io: T,
}

impl<T: Read + Write> ScpiLink<T> {
    pub fn new(io: T) -> Self {
        Self { io }
    }

    /// `cmd`(`idn()`/`nsel()`/`meas_volt()` 等の出力、終端無し)に LF を付けて送り、応答を
    /// 1 行(終端を剥がして)読む。応答の解釈は呼び出し側が `parse_idn`/`parse_f64`/…で行う。
    pub fn exchange(&mut self, cmd: &str) -> Result<String, LinkError> {
        let mut line = String::with_capacity(cmd.len() + 1);
        line.push_str(cmd);
        line.push('\n');
        self.io.write_all(line.as_bytes())?;

        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            match self.io.read(&mut byte) {
                Ok(0) => {
                    return Err(
                        io::Error::new(io::ErrorKind::UnexpectedEof, "hmp: EOF before LF").into(),
                    );
                }
                Ok(_) => {
                    buf.push(byte[0]);
                    if buf.ends_with(b"\n") {
                        break;
                    }
                    if buf.len() > 256 {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "hmp: reply too long",
                        )
                        .into());
                    }
                }
                Err(e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        || e.kind() == io::ErrorKind::TimedOut =>
                {
                    return Err(e.into());
                }
                Err(e) => return Err(e.into()),
            }
        }

        let raw = String::from_utf8_lossy(&buf).into_owned();
        // 終端 LF/CRLF どちらも剥がす(p.9 §1.1 NOTICE: LF or CR-LF)。
        let trimmed = raw.trim_end_matches(['\r', '\n']);
        Ok(trimmed.to_string())
    }

    /// `cmd` に LF を付けて送るだけで、応答は待たない。TODO/089 レビュー(2026-09-23)で解禁:
    /// `nsel()`(`INST:NSEL n`)のような非クエリ(`?` の無い)コマンドは実機が応答を返す
    /// 保証が無く、[`ScpiLink::exchange`] を使うと来ない応答をタイムアウトまで待ってしまう。
    /// 呼び出し側は必要なら別のクエリ(例 [`nsel_query`])で結果を確認する。
    pub fn send(&mut self, cmd: &str) -> Result<(), LinkError> {
        let mut line = String::with_capacity(cmd.len() + 1);
        line.push_str(cmd);
        line.push('\n');
        self.io.write_all(line.as_bytes())?;
        Ok(())
    }
}

// ---------------------------------------------------------------------
// テスト(仕様書 — 先に書く)。SCPI Programmers Manual EN の実例をゴールデンにする。
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    // ---- ①要求文字列の生成(p.9 短縮形規則、p.22 の実例)。 ----

    #[test]
    fn idn_matches_manual() {
        assert_eq!(idn(), "*IDN?");
    }

    #[test]
    fn nsel_matches_manual_example() {
        // p.22: "Example: INST:NSEL 1"。
        assert_eq!(nsel(1).unwrap(), "INST:NSEL 1");
        assert_eq!(nsel(2).unwrap(), "INST:NSEL 2");
    }

    #[test]
    fn nsel_channel_3_is_err_for_hmp2020() {
        // p.22 NOTICE: "HMP2020: :NSELect {3} and :NSELect {4} are not available"。
        assert_eq!(nsel(3).unwrap_err(), HmpError::ChannelOutOfRange(3));
        assert_eq!(nsel(4).unwrap_err(), HmpError::ChannelOutOfRange(4));
        assert_eq!(nsel(0).unwrap_err(), HmpError::ChannelOutOfRange(0));
    }

    #[test]
    fn meas_and_outp_and_syst_err_short_forms() {
        // p.9: "MEASure:CURRent? is equivalent to MEAS:CURR?"(実例)。
        assert_eq!(meas_curr(), "MEAS:CURR?");
        // p.31 コマンド表 "MEASure[:SCALar][:VOLTage] [:DC]?" の短縮形(p.9 規則適用)。
        assert_eq!(meas_volt(), "MEAS:VOLT?");
        // p.26 コマンド表 "OUTPut[:STATe]?" の短縮形。
        assert_eq!(outp_query(), "OUTP?");
        // p.20 コマンド表 "SYSTem:ERRor[:NEXT]?" の短縮形。
        assert_eq!(syst_err(), "SYST:ERR?");
    }

    /// TODO/120: 設定値の読み出し(問い合わせ形のみ)。
    #[test]
    fn set_value_queries_are_query_forms() {
        assert_eq!(volt_query(), "VOLT?");
        assert_eq!(curr_query(), "CURR?");
    }

    /// TODO/089 レビュー: `nsel()` は非クエリなので選択の確認に使うクエリ形。
    #[test]
    fn nsel_query_matches_the_nsel_setter_form() {
        assert_eq!(nsel_query(), "INST:NSEL?");
    }

    // ---- ②応答の解析。 ----

    #[test]
    fn parse_idn_matches_manual_example() {
        // p.18: "Example: HAMEG,HMP4040,055310003,HW50020001/SW2.41"(応答フォーマットの実例。
        // 型番は HMP4040 だがフィールド構造は HMP2020 と共通 — SCPI manual は HMP series 共通)。
        let idn = parse_idn("HAMEG,HMP4040,055310003,HW50020001/SW2.41").unwrap();
        assert_eq!(
            idn,
            Idn {
                vendor: "HAMEG".to_string(),
                model: "HMP4040".to_string(),
                serial: "055310003".to_string(),
                fw: "HW50020001/SW2.41".to_string(),
            }
        );
    }

    #[test]
    fn parse_idn_malformed_is_err() {
        assert!(parse_idn("HAMEG,HMP2020").is_err());
    }

    #[test]
    fn parse_f64_matches_manual_examples() {
        // p.31: "MEASure[:SCALar]:CURRent [:DC]? Return values: e.g. 1.0000"
        assert_eq!(parse_f64("1.0000").unwrap(), 1.0);
        // p.31: "MEASure[:SCALar][:VOLTage] [:DC]? Return values: e.g. 1.000"
        assert_eq!(parse_f64("1.000").unwrap(), 1.0);
    }

    #[test]
    fn parse_f64_malformed_is_err() {
        assert!(parse_f64("not-a-number").is_err());
    }

    #[test]
    fn parse_bool01_matches_manual() {
        // p.26: "OUTPut[:STATe]? Return values: 1 ON...; 0 OFF..."
        assert!(parse_bool01("1").unwrap());
        assert!(!parse_bool01("0").unwrap());
    }

    #[test]
    fn parse_bool01_malformed_is_err() {
        assert!(parse_bool01("YES").is_err());
    }

    #[test]
    fn parse_syst_err_matches_manual_examples() {
        // p.20: "0, "No error"" / "-100, "Command error"" / "-102, "Syntax error"" /
        // "-350, "Queue overflow""。
        assert_eq!(
            parse_syst_err("0, \"No error\"").unwrap(),
            (0, "No error".to_string())
        );
        assert_eq!(
            parse_syst_err("-100, \"Command error\"").unwrap(),
            (-100, "Command error".to_string())
        );
        assert_eq!(
            parse_syst_err("-102, \"Syntax error\"").unwrap(),
            (-102, "Syntax error".to_string())
        );
        assert_eq!(
            parse_syst_err("-350, \"Queue overflow\"").unwrap(),
            (-350, "Queue overflow".to_string())
        );
    }

    #[test]
    fn parse_syst_err_malformed_is_err() {
        assert!(parse_syst_err("no comma here").is_err());
    }

    // ---- ③Link: fake を挟んで 1 往復、空応答(タイムアウト)、CR LF / LF 両終端。 ----

    #[test]
    fn link_exchange_round_trips_with_lf_terminator() {
        use crate::slowctl::fake_hmp::FakeHmp;

        let mut fake = FakeHmp::new();
        fake.set_response("MEAS:VOLT?\n", "1.234\n");

        let mut link = ScpiLink::new(fake);
        let reply = link.exchange(meas_volt()).unwrap();
        assert_eq!(reply, "1.234");
    }

    #[test]
    fn link_exchange_round_trips_with_crlf_terminator() {
        use crate::slowctl::fake_hmp::FakeHmp;

        // 応答終端が CR LF でも受理する(p.9 §1.1 NOTICE: LF or CR-LF)。
        let mut fake = FakeHmp::new();
        fake.set_response("MEAS:CURR?\n", "0.5000\r\n");

        let mut link = ScpiLink::new(fake);
        let reply = link.exchange(meas_curr()).unwrap();
        assert_eq!(reply, "0.5000");
    }

    #[test]
    fn link_exchange_times_out_on_silent_device() {
        use crate::slowctl::fake_hmp::FakeHmp;

        let mut fake = FakeHmp::new();
        fake.set_silent();

        let mut link = ScpiLink::new(fake);
        let err = link.exchange(meas_volt()).unwrap_err();
        assert!(matches!(err, LinkError::Io(_)));
    }

    /// TODO/089 レビュー: `send` は書くだけで応答を待たない —— fake は非クエリの
    /// `INST:NSEL n` に応答を積まないので、`exchange` と違って `Ok(())` ですぐ返る。
    #[test]
    fn send_writes_without_waiting_for_a_reply() {
        use crate::slowctl::fake_hmp::FakeHmp;

        let fake = FakeHmp::new();
        let mut link = ScpiLink::new(fake);
        link.send(&nsel(1).unwrap()).unwrap();
    }

    /// `send` の後に `nsel_query` で選択を確認できる(fake が状態を持つ、TODO/089)。
    #[test]
    fn send_then_nsel_query_confirms_the_selected_channel() {
        use crate::slowctl::fake_hmp::FakeHmp;

        let fake = FakeHmp::new();
        let mut link = ScpiLink::new(fake);
        link.send(&nsel(2).unwrap()).unwrap();
        let reply = link.exchange(nsel_query()).unwrap();
        assert_eq!(reply, "2");
    }
}
