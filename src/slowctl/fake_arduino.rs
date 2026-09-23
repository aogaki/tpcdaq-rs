//! インメモリ Arduino ENV 応答器(`Read + Write`)。テスト専用
//! (`cfg(any(test, feature = "slowctl-fake"))`)。
//!
//! スケッチ(`tools/arduino_env/env_json.ino`)は**問い合わせ不要で一方的に**行を push する
//! (TODO/079 §2.3 / §2.7)ので、`FakeHmp`(要求行 → 応答行の対応表)とは形が違う —— こちらは
//! 「出力キューに積んだバイト列をそのまま読ませる」だけの単純な片方向バッファ。
//!
//! [`push_line`](FakeArduino::push_line) は 1 行(終端を自動で足す)、[`push_raw`](FakeArduino::push_raw)
//! は終端の有無を含めて呼び出し側が完全に制御する(壊れた行・尻切れ行のテスト用)。
//! キューが空になったら **`WouldBlock`**(タイムアウト相当、CLAUDE.md: never-stop /
//! silent failure を作らない)。`Write` は無視する(Arduino は入力を読まない)。

use std::collections::VecDeque;
use std::io::{self, Read, Write};

/// 出力バイト列をキューに積むだけのインメモリ応答器。
pub struct FakeArduino {
    outgoing: VecDeque<u8>,
}

impl Default for FakeArduino {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeArduino {
    pub fn new() -> Self {
        Self {
            outgoing: VecDeque::new(),
        }
    }

    /// `line`(終端無し)を積み、末尾に `\n` を足す。
    pub fn push_line(&mut self, line: &str) -> &mut Self {
        self.outgoing.extend(line.as_bytes());
        self.outgoing.push_back(b'\n');
        self
    }

    /// 生バイト列をそのまま積む(終端の有無はテストが決める —— 尻切れ行のテスト用)。
    pub fn push_raw(&mut self, bytes: &[u8]) -> &mut Self {
        self.outgoing.extend(bytes.iter().copied());
        self
    }
}

impl Write for FakeArduino {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // Arduino は入力を読まない —— 書き込みは黙って受理する(何もしない)。
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Read for FakeArduino {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.outgoing.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "FakeArduino: no more data queued",
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
// テスト(fake 自体の最小限の疎通確認。本命は arduino.rs 側の Driver テスト)。
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_pushed_line_is_read_back_byte_by_byte() {
        let mut fake = FakeArduino::new();
        fake.push_line(r#"{"dev":"mkr_env"}"#);

        let mut out = Vec::new();
        loop {
            let mut b = [0u8; 1];
            match fake.read(&mut b) {
                Ok(1) => out.push(b[0]),
                _ => break,
            }
        }
        assert_eq!(out, b"{\"dev\":\"mkr_env\"}\n");
    }

    #[test]
    fn an_empty_queue_yields_would_block() {
        let mut fake = FakeArduino::new();
        let mut b = [0u8; 1];
        let err = fake.read(&mut b).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
    }

    #[test]
    fn push_raw_does_not_add_a_terminator() {
        let mut fake = FakeArduino::new();
        fake.push_raw(b"no newline here");

        let mut out = Vec::new();
        loop {
            let mut b = [0u8; 1];
            match fake.read(&mut b) {
                Ok(1) => out.push(b[0]),
                _ => break,
            }
        }
        assert_eq!(out, b"no newline here");
    }
}
