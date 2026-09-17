//! 同期 ProPar リンク(Bronkhorst 9.17.027 §3.2 node scan / §3.7 chaining)。
//!
//! `T: Read + Write` はジェネリックにしてある(Phase 1 は IO を持たない — 呼び出し側が
//! シリアルポート等を渡す。Phase 2 で `serialport::TTYPort` がそのまま入る想定、TODO/079 §2.5)。

use std::fmt;
use std::io::{self, Read, Write};

use super::propar::{self, ProParError, Reply, Request, Value};

/// node scan で得る識別情報(`propar::ident_request` の 5 項目、9.17.027 §3.9 IDENTIFICATION
/// PARAMETERS)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ident {
    pub identification_number: u8,
    pub model: String,
    pub serial: String,
    pub firmware: String,
    pub user_tag: String,
}

/// `exchange`/`scan` の累積カウンタ。silent failure を作らない(CLAUDE.md)——
/// 呼び出し側はこれを `Display` で 1 行ログに出す。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LinkStats {
    pub ok: u64,
    pub timeout: u64,
    pub malformed: u64,
    pub node_mismatch: u64,
}

impl fmt::Display for LinkStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "propar link: ok={} timeout={} malformed={} node_mismatch={}",
            self.ok, self.timeout, self.malformed, self.node_mismatch
        )
    }
}

/// [`ProParLink::exchange`] の失敗要因(IO エラーとプロトコルエラーを吸収する)。
#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("propar error: {0}")]
    Protocol(#[from] ProParError),
}

/// 同期 ProPar リンク。1 要求につき 1 行送って 1 行読む(9.17.027 の ASCII プロトコルは
/// 「1 度に 1 つの master/slave 対話」— §3.1)。
pub struct ProParLink<T: Read + Write> {
    io: T,
    pub stats: LinkStats,
}

impl<T: Read + Write> ProParLink<T> {
    pub fn new(io: T) -> Self {
        Self {
            io,
            stats: LinkStats::default(),
        }
    }

    /// `request` を送って応答を 1 行読み、復号する。
    ///
    /// タイムアウトの実時間は呼び出し側が `T`(例: シリアルポートの read timeout)で設定する
    /// 前提(Phase 1 の範囲外)。ここでは IO が `WouldBlock`/`TimedOut` を返した場合、または
    /// 応答が復号できなかった場合を、それぞれ対応するカウンタに計上するだけ。
    pub fn exchange(&mut self, request: &Request) -> Result<Reply, LinkError> {
        let line = request.to_ascii_line();
        self.io.write_all(line.as_bytes())?;

        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            match self.io.read(&mut byte) {
                Ok(0) => {
                    self.stats.timeout += 1;
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "propar: EOF before CRLF",
                    )
                    .into());
                }
                Ok(_) => {
                    buf.push(byte[0]);
                    if buf.ends_with(b"\r\n") {
                        break;
                    }
                    // 9.17.027 §3.3.1: このホスト実装のデータ部は最大 64 byte。無限に
                    // 溜め込まない(壊れたリンクを malformed として打ち切る)。
                    if buf.len() > 256 {
                        self.stats.malformed += 1;
                        return Err(ProParError::Framing.into());
                    }
                }
                Err(e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        || e.kind() == io::ErrorKind::TimedOut =>
                {
                    self.stats.timeout += 1;
                    return Err(e.into());
                }
                Err(e) => return Err(e.into()),
            }
        }

        let line = String::from_utf8_lossy(&buf).into_owned();
        match propar::parse_reply(&line, request) {
            Ok(reply) => {
                self.stats.ok += 1;
                Ok(reply)
            }
            Err(e) => {
                match e {
                    ProParError::NodeMismatch { .. } => self.stats.node_mismatch += 1,
                    _ => self.stats.malformed += 1,
                }
                Err(e.into())
            }
        }
    }

    /// `nodes` を順に ping し、識別群を読めたノードだけ返す(9.17.027 §3.2)。
    ///
    /// 無応答ノードはタイムアウトとして `stats` に計上したうえでスキップする(silent 禁止 —
    /// CLAUDE.md)。戻り値には登場順(`nodes` の順序)で応答したノードだけが入る。
    pub fn scan(&mut self, nodes: impl Iterator<Item = u8>) -> Vec<(u8, Ident)> {
        let mut found = Vec::new();
        for node in nodes {
            let request = propar::ident_request(node);
            if let Ok(reply) = self.exchange(&request) {
                match ident_from_values(&reply.values) {
                    Some(ident) => found.push((node, ident)),
                    None => self.stats.malformed += 1,
                }
            }
            // Err の場合は exchange 内で既に ok/timeout/malformed/node_mismatch へ計上済み。
        }
        found
    }
}

/// `ident_request` の 5 値([U8, Str, Str, Str, Str])を [`Ident`] に組み立てる。
/// `parse_reply` は要求した `ParType` どおりに `Value` を作るので、`ident_request` で
/// 得た `Reply` に対しては形が狂うことは無い(念のため `None` 経路は残す — silent 禁止)。
fn ident_from_values(values: &[Value]) -> Option<Ident> {
    match values {
        [Value::U8(n), Value::Str(model), Value::Str(serial), Value::Str(firmware), Value::Str(user_tag)] => {
            Some(Ident {
                identification_number: *n,
                model: model.clone(),
                serial: serial.clone(),
                firmware: firmware.clone(),
                user_tag: user_tag.clone(),
            })
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------
// テスト
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::slowctl::fake::FakeProPar;
    use crate::slowctl::propar::{ParType, ReadItem};

    #[test]
    fn exchange_ok_reply_increments_ok_counter() {
        let req = Request {
            node: 0x80,
            items: vec![ReadItem {
                process: 1,
                param: 0,
                ty: ParType::U16,
            }],
        };
        let mut fake = FakeProPar::new();
        // measure: node128,process1,param0,type integer。9.17.027 p.24(propar.rs の golden と同じ実例)。
        fake.set_response(req.to_ascii_line(), ":06800201217D00\r\n");

        let mut link = ProParLink::new(fake);
        let reply = link.exchange(&req).unwrap();
        assert_eq!(reply.values, vec![Value::U16(32000)]);
        assert_eq!(
            link.stats,
            LinkStats {
                ok: 1,
                timeout: 0,
                malformed: 0,
                node_mismatch: 0
            }
        );
    }

    #[test]
    fn exchange_node_mismatch_increments_node_mismatch_counter() {
        let req = Request {
            node: 0x80,
            items: vec![ReadItem {
                process: 1,
                param: 0,
                ty: ParType::U16,
            }],
        };
        let mut fake = FakeProPar::new();
        // 応答の node を要求と違えて壊す(80→81)。
        fake.set_response(req.to_ascii_line(), ":06810201217D00\r\n");

        let mut link = ProParLink::new(fake);
        assert!(link.exchange(&req).is_err());
        assert_eq!(
            link.stats,
            LinkStats {
                ok: 0,
                timeout: 0,
                malformed: 0,
                node_mismatch: 1
            }
        );
    }

    #[test]
    fn exchange_malformed_reply_increments_malformed_counter() {
        let req = Request {
            node: 0x80,
            items: vec![ReadItem {
                process: 1,
                param: 0,
                ty: ParType::U16,
            }],
        };
        let mut fake = FakeProPar::new();
        // length フィールドを壊す(06→07、実バイト数は変わらない)。
        fake.set_response(req.to_ascii_line(), ":07800201217D00\r\n");

        let mut link = ProParLink::new(fake);
        assert!(link.exchange(&req).is_err());
        assert_eq!(
            link.stats,
            LinkStats {
                ok: 0,
                timeout: 0,
                malformed: 1,
                node_mismatch: 0
            }
        );
    }

    #[test]
    fn exchange_silent_node_increments_timeout_counter() {
        let req = propar::ident_request(6);
        let mut fake = FakeProPar::new();
        fake.set_silent(6);

        let mut link = ProParLink::new(fake);
        assert!(link.exchange(&req).is_err());
        assert_eq!(
            link.stats,
            LinkStats {
                ok: 0,
                timeout: 1,
                malformed: 0,
                node_mismatch: 0
            }
        );
    }

    // ---- ⑤fake 経由の node scan(3 ノード応答 + 2 ノード無応答、カウンタの値まで断定)----

    #[test]
    fn scan_finds_responding_nodes_and_counts_silent_ones() {
        let mut fake = FakeProPar::new();

        // 応答行は 9.17.027 §3.9.5 IDENTIFICATION PARAMETERS のビット配置を手で適用して
        // 構成した合成値(実装報告に導出根拠を記載)。ident_request(node) が生成する要求行を
        // そのままキーにするので、要求側の符号化は propar.rs のテストで別途検証済み。
        fake.set_response(
            propar::ident_request(3).to_ascii_line(),
            ":200302710007716007462D32303143567160024D3171600256317160044D464331\r\n",
        );
        fake.set_response(
            propar::ident_request(4).to_ascii_line(),
            ":1F0402710009716007502D37303243567160024D327160025632716003425043\r\n",
        );
        fake.set_response(
            propar::ident_request(5).to_ascii_line(),
            ":1D0502710009716005502D3530327160024D337160025631716003545043\r\n",
        );
        fake.set_silent(6);
        fake.set_silent(7);

        let mut link = ProParLink::new(fake);
        let found = link.scan([3u8, 4, 5, 6, 7].into_iter());

        assert_eq!(found.len(), 3);

        assert_eq!(found[0].0, 3);
        assert_eq!(found[0].1.identification_number, 7);
        assert_eq!(found[0].1.model, "F-201CV");
        assert_eq!(found[0].1.serial, "M1");
        assert_eq!(found[0].1.firmware, "V1");
        assert_eq!(found[0].1.user_tag, "MFC1");

        assert_eq!(found[1].0, 4);
        assert_eq!(found[1].1.identification_number, 9);
        assert_eq!(found[1].1.model, "P-702CV");
        assert_eq!(found[1].1.serial, "M2");
        assert_eq!(found[1].1.firmware, "V2");
        assert_eq!(found[1].1.user_tag, "BPC");

        assert_eq!(found[2].0, 5);
        assert_eq!(found[2].1.identification_number, 9);
        assert_eq!(found[2].1.model, "P-502");
        assert_eq!(found[2].1.serial, "M3");
        assert_eq!(found[2].1.firmware, "V1");
        assert_eq!(found[2].1.user_tag, "TPC");

        assert_eq!(
            link.stats,
            LinkStats {
                ok: 3,
                timeout: 2,
                malformed: 0,
                node_mismatch: 0
            }
        );
    }
}
