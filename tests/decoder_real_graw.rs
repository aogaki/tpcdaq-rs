//! 実 .graw 回帰(TODO/004_framer_decoder.md やること 4)。
//!
//! `TPCDAQ_REAL_GRAW` 環境変数に実ファイルへのパスを設定すると走る。未設定ならその場で
//! return して green のまま終わる(実 .graw はリポに入れない — CLAUDE.md)。
//!
//! オラクル値は C++ 版 tpcdaq(`~/test/get/tpcdaq`)で確定済みの正解(TODO/INDEX_en.md /
//! TODO/004_framer_decoder.md 発注書どおり): events=108 / items=15,040,512 / malformed=0。
#![allow(clippy::unwrap_used)]

use tpcdaq::decode::Decoder;
use tpcdaq::framer::Framer;

#[test]
fn real_graw_decodes_to_the_fixed_regression_oracle() {
    let Ok(path) = std::env::var("TPCDAQ_REAL_GRAW") else {
        eprintln!("skip: TPCDAQ_REAL_GRAW not set (local-only regression)");
        return;
    };

    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
    assert!(bytes.len() > 88, "real .graw file suspiciously small");

    // ファイルをチャンク読みで push する(実受信を模す。フレームがチャンク境界を跨いでよいこと
    // を framer 自身が保証する — 8KiB は total 29MB に対して十分に細かい)。
    let mut framer = Framer::new();
    let mut decoder = Decoder::new();
    for chunk in bytes.chunks(8192) {
        framer.push(chunk);
        while let Some(frame) = framer.next() {
            decoder.decode(frame);
        }
    }

    eprintln!(
        "events={} items={} malformed={} unsupported={} reset_count={}",
        decoder.frames(),
        decoder.items(),
        decoder.malformed(),
        decoder.unsupported(),
        framer.reset_count()
    );

    assert_eq!(decoder.frames(), 108, "events oracle");
    assert_eq!(decoder.items(), 15_040_512, "items oracle");
    assert_eq!(decoder.malformed(), 0, "malformed oracle");
    assert_eq!(framer.reset_count(), 0, "no corrupt-header resync expected");
}
