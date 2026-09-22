//! インメモリ HiVolta(CAEN DT1415ET)応答器(`Read + Write`)。テスト専用
//! (`cfg(any(test, feature = "slowctl-fake"))`)。
//!
//! コーデックを二重実装しないよう、要求行 → 応答行の完全一致テーブルだけを持つ薄いスタブに
//! している(符号化・復号のロジックは `hivolta` モジュールにある — ここでは何も「賢い」ことを
//! しない、KISS)。
//!
//! **`$CMD:SET,...` で始まる行は登録の有無に関わらず常に `#LOC:ERR` を返す** — TODO/085 §0:
//! 「設定系コマンドを生成する関数は無い」ことの負性テスト(生文字列を直接流しても弾かれることを
//! 確認する)。

use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};

/// 要求行 → 応答行の完全一致で応答するインメモリ応答器。
pub struct FakeHv {
    responses: HashMap<String, String>,
    silent: bool,
    incoming: Vec<u8>,
    outgoing: VecDeque<u8>,
}

impl Default for FakeHv {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeHv {
    pub fn new() -> Self {
        Self {
            responses: HashMap::new(),
            silent: false,
            incoming: Vec::new(),
            outgoing: VecDeque::new(),
        }
    }

    /// `request_line`(`mon_ch`/`mon_bd` の出力そのもの、`<CR,LF>` 込み)に `reply_line`
    /// (`<CR,LF>` 込み)で応答するよう登録する。
    pub fn set_response(
        &mut self,
        request_line: impl Into<String>,
        reply_line: impl Into<String>,
    ) -> &mut Self {
        self.responses
            .insert(request_line.into(), reply_line.into());
        self
    }

    /// 以後の要求に一切応答しない(TCP/USB が固まった状態を模す)。**EOF ではなく
    /// `WouldBlock`(タイムアウト相当)を返す**(CLAUDE.md: never-stop / silent failure を
    /// 作らない — 呼び出し側がタイムアウトとして扱える形にする)。
    pub fn set_silent(&mut self) -> &mut Self {
        self.silent = true;
        self
    }
}

impl Write for FakeHv {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.incoming.extend_from_slice(buf);
        if self.incoming.ends_with(b"\r\n") {
            let line = String::from_utf8_lossy(&self.incoming).into_owned();
            self.incoming.clear();

            if self.silent {
                // outgoing に何も積まない → read() が WouldBlock を返す。
            } else if line.starts_with("$CMD:SET") {
                // TODO/085 §0: 設定系コマンドの型が無い分、生文字列を送りつける負性テストを
                // 受け止める側 — 実機がロック/LOCAL モードで拒否するのと同じ扱いにする。
                self.outgoing.extend(b"#LOC:ERR\r\n");
            } else if let Some(reply) = self.responses.get(&line) {
                self.outgoing.extend(reply.as_bytes());
            } else {
                // テスト設定漏れ。silent にせず可視化する(CLAUDE.md)。
                eprintln!("FakeHv: no response registered for {line:?}");
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Read for FakeHv {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.outgoing.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "FakeHv: no reply queued (silent device or unmatched request)",
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

// ---------------------------------------------------------------------
// テスト(fake 自体の最小限の疎通確認。本命は hivolta.rs 側の Link テスト)。
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn registered_response_is_echoed_back() {
        let mut fake = FakeHv::new();
        fake.set_response("$CMD:MON,CH:0,PAR:VMON\r\n", "#CMD:OK,VAL:503.20\r\n");

        fake.write_all(b"$CMD:MON,CH:0,PAR:VMON\r\n").unwrap();

        let mut out = Vec::new();
        loop {
            let mut b = [0u8; 1];
            match fake.read(&mut b) {
                Ok(1) => out.push(b[0]),
                _ => break,
            }
            if out.ends_with(b"\r\n") {
                break;
            }
        }
        assert_eq!(out, b"#CMD:OK,VAL:503.20\r\n");
    }

    #[test]
    fn raw_set_command_is_rejected_with_loc_err_even_when_unregistered() {
        let mut fake = FakeHv::new();
        // 何も登録していない生の SET 文字列 — それでも #LOC:ERR が返る。
        fake.write_all(b"$CMD:SET,CH:0,PAR:VSET,VAL:500.00\r\n")
            .unwrap();

        let mut out = Vec::new();
        loop {
            let mut b = [0u8; 1];
            match fake.read(&mut b) {
                Ok(1) => out.push(b[0]),
                _ => break,
            }
            if out.ends_with(b"\r\n") {
                break;
            }
        }
        assert_eq!(out, b"#LOC:ERR\r\n");
    }

    #[test]
    fn silent_device_yields_would_block() {
        let mut fake = FakeHv::new();
        fake.set_silent();

        fake.write_all(b"$CMD:MON,CH:0,PAR:VMON\r\n").unwrap();

        let mut b = [0u8; 1];
        let err = fake.read(&mut b).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
    }
}
