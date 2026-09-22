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
            } else if let Some(reply) = self.responses.get(&line) {
                self.outgoing.extend(reply.as_bytes());
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
}
