//! インメモリ HiVolta(CAEN DT1415ET)応答器(`Read + Write`)。テスト専用
//! (`cfg(any(test, feature = "slowctl-fake"))`)。
//!
//! コーデックを二重実装しないよう、要求行 → 応答行の完全一致テーブルだけを持つ薄いスタブに
//! している(符号化・復号のロジックは `hivolta` モジュールにある — ここでは何も「賢い」ことを
//! しない、KISS)。
//!
//! **`$CMD:SET,...` で始まる行は、`PAR:ON` / `PAR:OFF` を除いて常に `#LOC:ERR` を返す** —
//! TODO/085 §0 の負性テスト(生文字列を直接流しても弾かれることを確認する)。
//!
//! TODO/124 Fable 9: `$CMD:SET,CH:X,PAR:ON|OFF`(X = 0..8)だけは受理して内部の STATUS の
//! bit 0 を書き換える(`set_local` 中は `#LOC:ERR`、X ≥ 8 は `#CH:ERR`)。STATUS の読み出しは
//! **登録した応答が優先**、無ければ内部の値で答える(既存テストのゴールデンを壊さない)。

use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};

/// 要求行 → 応答行の完全一致で応答するインメモリ応答器。
pub struct FakeHv {
    responses: HashMap<String, String>,
    /// ch ごとの STATUS(TODO/124)。`PAR:ON` / `PAR:OFF` が bit 0 を書き換える。
    status: [u16; 8],
    /// LOCAL モード(SET はすべて `#LOC:ERR`)。
    local: bool,
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
            status: [0; 8],
            local: false,
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

    /// ch の STATUS の初期値(TODO/124)。範囲外の ch は無視する(テストの書き間違いは
    /// 読み出しの食い違いで表に出る)。
    pub fn set_status(&mut self, ch: u8, status: u16) -> &mut Self {
        if let Some(slot) = self.status.get_mut(usize::from(ch)) {
            *slot = status;
        }
        self
    }

    /// LOCAL モードにする(以後の SET はすべて `#LOC:ERR`、マニュアル p.22)。
    pub fn set_local(&mut self) -> &mut Self {
        self.local = true;
        self
    }

    /// `$CMD:SET,...` への応答(TODO/124 Fable 9)。ON / OFF だけ受理する。
    fn answer_set(&mut self, line: &str) -> &'static str {
        if self.local {
            return "#LOC:ERR\r\n";
        }
        let Some(rest) = line.strip_prefix("$CMD:SET,CH:") else {
            return "#LOC:ERR\r\n";
        };
        let (ch, on) = if let Some(ch) = rest.strip_suffix(",PAR:ON\r\n") {
            (ch, true)
        } else if let Some(ch) = rest.strip_suffix(",PAR:OFF\r\n") {
            (ch, false)
        } else {
            return "#LOC:ERR\r\n";
        };
        let Some(slot) = ch
            .parse::<usize>()
            .ok()
            .and_then(|ch| self.status.get_mut(ch))
        else {
            return "#CH:ERR\r\n";
        };
        if on {
            *slot |= 1;
        } else {
            *slot &= !1;
        }
        "#CMD:OK\r\n"
    }

    /// 登録の無い STATUS 読み出しを内部の値で答える(単 ch = `CH:X`、全 ch = `CH:8`)。
    fn answer_status(&self, line: &str) -> Option<String> {
        let ch = line
            .strip_prefix("$CMD:MON,CH:")?
            .strip_suffix(",PAR:STATUS\r\n")?;
        let ch: usize = ch.parse().ok()?;
        if ch == self.status.len() {
            let all: Vec<String> = self.status.iter().map(|s| format!("{s:05}")).collect();
            return Some(format!("#CMD:OK,VAL:{}\r\n", all.join(";")));
        }
        let status = self.status.get(ch)?;
        Some(format!("#CMD:OK,VAL:{status:05}\r\n"))
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
                // TODO/085 §0 + TODO/124: ON / OFF 以外の SET は実機がロック/LOCAL モードで
                // 拒否するのと同じ扱い(生文字列を送りつける負性テストを受け止める側)。
                let reply = self.answer_set(&line);
                self.outgoing.extend(reply.as_bytes());
            } else if let Some(reply) = self.responses.get(&line) {
                self.outgoing.extend(reply.as_bytes());
            } else if let Some(reply) = self.answer_status(&line) {
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

    /// 1 行送って 1 行読む(テストの小道具)。
    fn ask(fake: &mut FakeHv, line: &str) -> String {
        fake.write_all(line.as_bytes()).unwrap();
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
        String::from_utf8(out).unwrap()
    }

    /// TODO/124 Fable 9: `PAR:ON` / `PAR:OFF` だけ受理し、内部の STATUS の bit 0 を書き換える。
    /// 登録の無い STATUS 読み出し(単 ch / 全 ch)は内部の値で答える(実機と同じ 5 桁)。
    #[test]
    fn par_on_and_off_rewrite_the_internal_status() {
        let mut fake = FakeHv::new();
        // ch5 は UNV(32)だけ立っている状態から。
        fake.set_status(5, 32);

        assert_eq!(ask(&mut fake, "$CMD:SET,CH:5,PAR:ON\r\n"), "#CMD:OK\r\n");
        // 32 + 1 = 33(ON + UNV)。ほかのビットは触らない。
        assert_eq!(
            ask(&mut fake, "$CMD:MON,CH:5,PAR:STATUS\r\n"),
            "#CMD:OK,VAL:00033\r\n"
        );
        assert_eq!(
            ask(&mut fake, "$CMD:MON,CH:8,PAR:STATUS\r\n"),
            "#CMD:OK,VAL:00000;00000;00000;00000;00000;00033;00000;00000\r\n"
        );

        assert_eq!(ask(&mut fake, "$CMD:SET,CH:5,PAR:OFF\r\n"), "#CMD:OK\r\n");
        assert_eq!(
            ask(&mut fake, "$CMD:MON,CH:5,PAR:STATUS\r\n"),
            "#CMD:OK,VAL:00032\r\n"
        );
    }

    /// ON / OFF 以外の SET は従来どおり `#LOC:ERR`(値は変わらない)。CH:8(全 ch)の ON は
    /// 受けない(`#CH:ERR`)—— slowctl が送ってはいけない行。
    #[test]
    fn other_set_commands_and_the_group_channel_are_still_refused() {
        let mut fake = FakeHv::new();
        assert_eq!(
            ask(&mut fake, "$CMD:SET,CH:2,PAR:VSET,VAL:500.00\r\n"),
            "#LOC:ERR\r\n"
        );
        assert_eq!(
            ask(&mut fake, "$CMD:SET,CH:2,PAR:RUP,VAL:5\r\n"),
            "#LOC:ERR\r\n"
        );
        assert_eq!(ask(&mut fake, "$CMD:SET,CH:8,PAR:ON\r\n"), "#CH:ERR\r\n");
        assert_eq!(
            ask(&mut fake, "$CMD:MON,CH:2,PAR:STATUS\r\n"),
            "#CMD:OK,VAL:00000\r\n"
        );
    }

    /// LOCAL モード(`set_local`)では ON / OFF も `#LOC:ERR`(マニュアル p.22)。値は変わらない。
    #[test]
    fn in_local_mode_on_and_off_are_refused_with_loc_err() {
        let mut fake = FakeHv::new();
        fake.set_local();
        assert_eq!(ask(&mut fake, "$CMD:SET,CH:1,PAR:ON\r\n"), "#LOC:ERR\r\n");
        assert_eq!(
            ask(&mut fake, "$CMD:MON,CH:1,PAR:STATUS\r\n"),
            "#CMD:OK,VAL:00000\r\n"
        );
    }

    /// 登録した応答は内部の値より優先(既存テストの STATUS ゴールデンを壊さない)。
    #[test]
    fn a_registered_status_reply_wins_over_the_internal_status() {
        let mut fake = FakeHv::new();
        fake.set_response("$CMD:MON,CH:3,PAR:STATUS\r\n", "#CMD:OK,VAL:04096\r\n");
        assert_eq!(ask(&mut fake, "$CMD:SET,CH:3,PAR:ON\r\n"), "#CMD:OK\r\n");
        assert_eq!(
            ask(&mut fake, "$CMD:MON,CH:3,PAR:STATUS\r\n"),
            "#CMD:OK,VAL:04096\r\n"
        );
    }
}
