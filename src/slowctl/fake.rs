//! インメモリ ProPar 応答器(`Read + Write`)。テスト専用(`cfg(any(test, feature =
//! "slowctl-fake"))`)。
//!
//! コーデックを二重実装しないよう、要求行 → 応答行の完全一致テーブルだけを持つ薄いスタブに
//! している(符号化・復号のロジックはすべて `propar`/呼び出し側テストコードにある —
//! ここでは何も「賢い」ことをしない、KISS)。

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{self, Read, Write};

/// ノードごとに完全一致する要求行 → 応答行を登録するインメモリ応答器。
pub struct FakeProPar {
    responses: HashMap<String, String>,
    silent_nodes: HashSet<u8>,
    incoming: Vec<u8>,
    outgoing: VecDeque<u8>,
}

impl Default for FakeProPar {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeProPar {
    pub fn new() -> Self {
        Self {
            responses: HashMap::new(),
            silent_nodes: HashSet::new(),
            incoming: Vec::new(),
            outgoing: VecDeque::new(),
        }
    }

    /// `request_line`(`Request::to_ascii_line()` の出力そのもの)に `reply_line`
    /// (末尾 "\r\n" を含む)で応答するよう登録する。
    pub fn set_response(
        &mut self,
        request_line: impl Into<String>,
        reply_line: impl Into<String>,
    ) -> &mut Self {
        self.responses
            .insert(request_line.into(), reply_line.into());
        self
    }

    /// `node` 宛ての要求に一切応答しない(9.17.027 §3.2 の無応答ノード)。
    /// **EOF ではなく `WouldBlock`(タイムアウト相当)を返す**(CLAUDE.md: never-stop /
    /// silent failure を作らない — 呼び出し側がタイムアウトとして数えられる形にする)。
    pub fn set_silent(&mut self, node: u8) -> &mut Self {
        self.silent_nodes.insert(node);
        self
    }
}

impl Write for FakeProPar {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.incoming.extend_from_slice(buf);
        if self.incoming.ends_with(b"\r\n") {
            let line = String::from_utf8_lossy(&self.incoming).into_owned();
            self.incoming.clear();

            match extract_node(&line) {
                Some(node) if self.silent_nodes.contains(&node) => {
                    // 何も outgoing に積まない → read() が WouldBlock を返す。
                }
                Some(_) => {
                    if let Some(reply) = self.responses.get(&line) {
                        self.outgoing.extend(reply.as_bytes());
                    } else {
                        // テスト設定漏れ。silent にせず可視化する(CLAUDE.md)。
                        eprintln!("FakeProPar: no response registered for {line:?}");
                    }
                }
                None => {
                    eprintln!("FakeProPar: could not parse node from request line {line:?}");
                }
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Read for FakeProPar {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.outgoing.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "FakeProPar: no reply queued (silent node or unmatched request)",
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

/// `:LLNN...` からノードバイト(length の直後 2 桁)を取り出す(9.17.027 §3.3.3)。
fn extract_node(line: &str) -> Option<u8> {
    let hex = line.strip_prefix(':')?;
    let node_hex = hex.get(2..4)?;
    u8::from_str_radix(node_hex, 16).ok()
}

// ---------------------------------------------------------------------
// テスト(fake 自体の最小限の疎通確認。node scan の本命テストは link.rs 側)。
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn registered_response_is_echoed_back() {
        let mut fake = FakeProPar::new();
        fake.set_response(":06800401210120\r\n", ":06800201217D00\r\n");

        fake.write_all(b":06800401210120\r\n").unwrap();

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
        assert_eq!(out, b":06800201217D00\r\n");
    }

    #[test]
    fn silent_node_yields_would_block() {
        let mut fake = FakeProPar::new();
        fake.set_silent(0x80);

        fake.write_all(b":06800401210120\r\n").unwrap();

        let mut b = [0u8; 1];
        let err = fake.read(&mut b).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
    }

    #[test]
    fn extract_node_reads_the_correct_byte() {
        assert_eq!(extract_node(":06800401210120\r\n"), Some(0x80));
        assert_eq!(extract_node(":0F090401200120\r\n"), Some(0x09));
        assert_eq!(extract_node("no-colon"), None);
    }
}
