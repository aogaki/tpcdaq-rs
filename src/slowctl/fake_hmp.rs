//! インメモリ HMP2020 応答器(`Read + Write`)。テスト専用
//! (`cfg(any(test, feature = "slowctl-fake"))`)。
//!
//! コーデックを二重実装しないよう、要求行 → 応答行の完全一致テーブルだけを持つ薄いスタブに
//! している(符号化・復号のロジックは `hmp` モジュールにある — ここでは何も「賢い」ことを
//! しない、KISS)。要求のキーは `ScpiLink::exchange` が送るバイト列そのもの(LF 終端)。
//!
//! この fake は登録されていない行(例えば `VOLT 5\n` のような設定系コマンド — `hmp` モジュールは
//! そもそもこれを生成する関数を持たない)には応答しない。生成できないコマンドが誤って流れても
//! 黙って成功したように見えないことを、fake 自体の負性テストで確認する。

use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};

/// 要求行 → 応答行の完全一致で応答するインメモリ応答器。
pub struct FakeHmp {
    responses: HashMap<String, String>,
    /// TODO/089: `INST:NSEL` でチャンネルを切り替えると、以後の同じ要求行
    /// (`MEAS:VOLT?` 等)への応答がチャンネルごとに変わる。固定の 1 対 1 map では
    /// 表現できないので、要求行ごとに **先入れ先出しのキュー**を別に持つ
    /// ([`FakeHmp::push_response`])。
    sequenced: HashMap<String, VecDeque<String>>,
    /// TODO/089 レビュー(2026-09-23): `INST:NSEL n`(非クエリ、応答なしで送るだけ —
    /// [`super::hmp::ScpiLink::send`])が選んだチャンネル。`INST:NSEL?`(クエリ)は
    /// この状態を読み戻す —— 実機の「選択は効いたが SET コマンド自体には応答が無い」
    /// 挙動を模す。
    selected_channel: Option<u8>,
    silent: bool,
    incoming: Vec<u8>,
    outgoing: VecDeque<u8>,
}

impl Default for FakeHmp {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeHmp {
    pub fn new() -> Self {
        Self {
            responses: HashMap::new(),
            sequenced: HashMap::new(),
            selected_channel: None,
            silent: false,
            incoming: Vec::new(),
            outgoing: VecDeque::new(),
        }
    }

    /// `request_line`(LF 終端込み、例 `"MEAS:VOLT?\n"`)に `reply_line`(LF または CRLF 終端込み)
    /// で応答するよう登録する。
    pub fn set_response(
        &mut self,
        request_line: impl Into<String>,
        reply_line: impl Into<String>,
    ) -> &mut Self {
        self.responses
            .insert(request_line.into(), reply_line.into());
        self
    }

    /// `request_line` への応答を**順番に消費するキュー**へ 1 個積む(呼ぶたびに末尾へ追加)。
    /// 同じ要求文字列でもチャンネル選択(`INST:NSEL`)によって答えが変わる場合に使う
    /// (TODO/089: `hmp_driver` の poll は ch1 → ch2 の順で同じ `MEAS:*?`/`OUTP?` を送るため)。
    /// キューが尽きれば [`FakeHmp::set_response`] の固定応答にフォールバックする。
    pub fn push_response(
        &mut self,
        request_line: impl Into<String>,
        reply_line: impl Into<String>,
    ) -> &mut Self {
        self.sequenced
            .entry(request_line.into())
            .or_default()
            .push_back(reply_line.into());
        self
    }

    /// 以後の要求に一切応答しない(USB CDC が固まった状態を模す)。**EOF ではなく
    /// `WouldBlock`(タイムアウト相当)を返す**(CLAUDE.md: never-stop / silent failure を
    /// 作らない)。
    pub fn set_silent(&mut self) -> &mut Self {
        self.silent = true;
        self
    }
}

impl Write for FakeHmp {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.incoming.extend_from_slice(buf);
        if self.incoming.ends_with(b"\n") {
            let line = String::from_utf8_lossy(&self.incoming).into_owned();
            self.incoming.clear();

            if self.silent {
                // outgoing に何も積まない → read() が WouldBlock を返す。
            } else if let Some(reply) = self
                .sequenced
                .get_mut(&line)
                .and_then(VecDeque::pop_front)
                .or_else(|| self.responses.get(&line).cloned())
            {
                // 明示登録があれば最優先(テストが `INST:NSEL?` の答えを強制上書きして
                // 「選択が効かなかった」実機挙動を模すのにも使う、TODO/089 レビュー)。
                self.outgoing.extend(reply.as_bytes());
            } else if line == "INST:NSEL?\n" {
                // クエリ: 直前の `INST:NSEL n` で選んだチャンネルを読み戻す(TODO/089)。
                // まだ選んでいなければ 0(どの ch でもない)——テストが必ず先に選ぶ前提。
                let ch = self.selected_channel.unwrap_or(0);
                self.outgoing.extend(format!("{ch}\n").into_bytes());
            } else if let Some(ch) = parse_nsel_set(&line) {
                // 非クエリ(`INST:NSEL n`)。実機はここに応答を返さない —— 積まずに状態だけ持つ。
                self.selected_channel = Some(ch);
            } else {
                // テスト設定漏れ、または本来生成できないはずのコマンド(VOLT/CURR/OUTP ON 等)。
                // silent にせず可視化する(CLAUDE.md)。
                eprintln!("FakeHmp: no response registered for {line:?}");
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Read for FakeHmp {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.outgoing.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "FakeHmp: no reply queued (silent device or unmatched request)",
            ));
        }
        let mut n = 0;
        for slot in buf.iter_mut() {
            match self.outgoing.pop_front() {
                Some(b) => {
                    *slot = b;
                    n += 1;
                }
                None => break,
            }
        }
        Ok(n)
    }
}

/// `"INST:NSEL {n}\n"`(非クエリ、末尾に `?` が無い)ならチャンネル番号を取り出す。
/// `"INST:NSEL?\n"`(クエリ)はここでは `None`(呼び出し側で先に完全一致を見ている)。
fn parse_nsel_set(line: &str) -> Option<u8> {
    let rest = line.strip_prefix("INST:NSEL ")?;
    let rest = rest.strip_suffix('\n')?;
    rest.trim().parse::<u8>().ok()
}

// ---------------------------------------------------------------------
// テスト(fake 自体の最小限の疎通確認。本命は hmp.rs 側の Link テスト)。
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn registered_response_is_echoed_back() {
        let mut fake = FakeHmp::new();
        fake.set_response("*IDN?\n", "HAMEG,HMP2020,012345,HW1.0/SW2.0\n");

        fake.write_all(b"*IDN?\n").unwrap();

        let mut out = Vec::new();
        loop {
            let mut b = [0u8; 1];
            match fake.read(&mut b) {
                Ok(1) => out.push(b[0]),
                _ => break,
            }
            if out.ends_with(b"\n") {
                break;
            }
        }
        assert_eq!(out, b"HAMEG,HMP2020,012345,HW1.0/SW2.0\n");
    }

    #[test]
    fn unregistered_setter_style_command_gets_no_reply() {
        // "VOLT 5" は hmp モジュールに生成関数が無いコマンド。fake に登録していなければ、
        // 送りつけても応答は積まれない(誤って成功したように見えないことの確認)。
        let mut fake = FakeHmp::new();
        fake.write_all(b"VOLT 5\n").unwrap();

        let mut b = [0u8; 1];
        let err = fake.read(&mut b).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
    }

    #[test]
    fn silent_device_yields_would_block() {
        let mut fake = FakeHmp::new();
        fake.set_silent();

        fake.write_all(b"MEAS:VOLT?\n").unwrap();

        let mut b = [0u8; 1];
        let err = fake.read(&mut b).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
    }

    fn read_line(fake: &mut FakeHmp) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let mut b = [0u8; 1];
            match fake.read(&mut b) {
                Ok(1) => out.push(b[0]),
                _ => break,
            }
            if out.ends_with(b"\n") {
                break;
            }
        }
        out
    }

    /// TODO/089: `INST:NSEL n`(非クエリ)には応答が積まれない —— 実機と同じ
    /// (呼んだだけでは `read` は WouldBlock のまま)。
    #[test]
    fn a_non_query_nsel_command_gets_no_reply() {
        let mut fake = FakeHmp::new();
        fake.write_all(b"INST:NSEL 1\n").unwrap();

        let mut b = [0u8; 1];
        let err = fake.read(&mut b).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
    }

    /// `INST:NSEL?` は直前の `INST:NSEL n` を読み戻す(fake が状態を持つ、TODO/089 レビュー)。
    #[test]
    fn nsel_query_echoes_back_the_last_selected_channel() {
        let mut fake = FakeHmp::new();
        fake.write_all(b"INST:NSEL 2\n").unwrap();
        fake.write_all(b"INST:NSEL?\n").unwrap();
        assert_eq!(read_line(&mut fake), b"2\n");

        // 選び直せば追従する。
        fake.write_all(b"INST:NSEL 1\n").unwrap();
        fake.write_all(b"INST:NSEL?\n").unwrap();
        assert_eq!(read_line(&mut fake), b"1\n");
    }
}
