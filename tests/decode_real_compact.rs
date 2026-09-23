//! compact フレーム(frameType 2 / rev 5)の実機データ照合(TODO/069 #16)。
//!
//! 067-B は frameType 1(2018 形式)を GET 純正 **MFM ライブラリ**で照合したが、compact
//! (frameType 2、2025 実機標準)は未照合のままだった(SPEC §12-1 の穴)。本ユニットは
//! その穴を埋める。
//!
//! # オラクル
//!
//! `reference/_spike/mfm_compact/mfm_compact_dump.cpp`(使い捨てダンパ、gitignore、
//! リポにはコミットしない)を `reference/_spike/prefix/lib/libMultiFrame.dylib` へ直接リンク
//! してビルドし、`reference/config/CoboFormats.xcfg`(→ `CoboFormats-Rev-5-Compact.xcfg`)を
//! `mfm::FrameDictionary::addFormats` で読み込んで実 .graw を復号した(2026-09-23 実測)。
//! 経路は CoBoFrameViewer の `CoBoEvent::fromFrame()` / `decodeSamples()` の compact 分岐
//! (`reference/20190315_patched/CoBoFrameViewer/src/get/CoBoEvent.cpp:187-259`)と同一 ——
//! `mfm::Frame::read` → `frame.headerField(...)`(ヘッダ)/ `itemAt(i).field("").bitField(...)`
//! (item、per-AGET カーソルで ch/bucket を復元、numChannels=68)。
//!
//! ダンパは各フレームについて「ヘッダ固定順バイト列(frame_type u16 LE, revision u8,
//! cobo u8, asad u8, event_idx u32 LE, event_time u64 LE, read_offset u16 LE, status u8,
//! mult[4] u16 LE, window_out u32 LE, last_cell[4] u16 LE)+ item 列(`pack_item` と同一の
//! ビット詰め u32 LE 連結)」を SHA-256 に流し込み、run 全体(60 フレーム)で 1 本の
//! digest に畳んだ。本テストは `Decoder::decode` の出力(`Fragment`)から全く同じバイト列を
//! 組み立てて同じ digest を再現できることを確認する —— これが「我々の decoder は GET 純正
//! MFM ライブラリと全一致する」ことの証明になる。
//!
//! `TPCDAQ_EXP_DATA_2026` が未設定ならその場で return して green のまま終わる
//! (実 .graw はリポに入れない — CLAUDE.md)。
#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tpcdaq::decode::{parse_topology, Decoder};
use tpcdaq::framer::Framer;
use tpcdaq::msg::Fragment;

/// 1 フレームの item 数 = 272 raw ch(4 AGET × 68)× 512 bucket。frameType 1 の
/// `decoder_real_stream_compat.rs` と同一の定数(compact も full-readout なので同じ)。
const ORACLE_ITEMS_PER_FRAME: u64 = 139_264;
/// compact は 2 B/item: ヘッダ 256 B + 139,264 item × 2 B = 278,784 B。
const ORACLE_FRAME_BYTES: usize = 278_784;
/// d3_pulser_scan の各 run は 60 event(= 60 データフレーム、単一 AsAd0 ファイル、
/// topology frame なし — ファイルサイズ 16,727,040 B = 60 × 278,784 B ぴったりで確認済み)。
const ORACLE_FRAMES: u64 = 60;

fn exp_data_root() -> Option<PathBuf> {
    match std::env::var_os("TPCDAQ_EXP_DATA_2026") {
        Some(v) => Some(PathBuf::from(v)),
        None => {
            eprintln!(
                "skip: TPCDAQ_EXP_DATA_2026 not set (local-only regression, real .graw not in repo)"
            );
            None
        }
    }
}

/// `dir` 直下の唯一の `.graw` を返す(mini = 1 CoBo × 1 AsAd なので run あたり 1 ファイル)。
fn single_graw_in(dir: &Path) -> PathBuf {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("cannot read dir {}: {e}", dir.display()))
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "graw"))
        .collect();
    found.sort();
    assert_eq!(
        found.len(),
        1,
        "{} は .graw をちょうど 1 個含むこと(実測 {found:?})",
        dir.display()
    );
    found.remove(0)
}

/// [`Fragment`] のヘッダを、ダンパと全く同じ固定順バイト列(LE)へ展開する。
fn header_canonical_bytes(f: &Fragment) -> Vec<u8> {
    let mut out = Vec::with_capacity(40);
    out.extend_from_slice(&u16::from(f.frame_type).to_le_bytes());
    out.push(f.revision);
    out.push(f.cobo);
    out.push(f.asad);
    out.extend_from_slice(&f.event_idx.to_le_bytes());
    out.extend_from_slice(&f.event_time.to_le_bytes());
    out.extend_from_slice(&f.read_offset.to_le_bytes());
    out.push(f.status);
    for m in f.mult {
        out.extend_from_slice(&m.to_le_bytes());
    }
    out.extend_from_slice(&f.window_out.to_le_bytes());
    for c in f.last_cell {
        out.extend_from_slice(&c.to_le_bytes());
    }
    out
}

struct RunDigest {
    frames: u64,
    items: u64,
    malformed: u64,
    unsupported: u64,
    frame_sizes: std::collections::BTreeSet<usize>,
    sha256_hex: String,
    frame0: Fragment,
}

/// 1 run の .graw を `Framer` + `Decoder` で最後まで読み切り、ダンパと同一の正規化で
/// SHA-256(60 フレーム分を 1 本に畳んだもの)を計算する。
fn decode_run(path: &Path) -> RunDigest {
    let bytes =
        std::fs::read(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

    let mut framer = Framer::new();
    let mut decoder = Decoder::new();
    let mut frame_sizes = std::collections::BTreeSet::new();
    let mut hasher = Sha256::new();
    let mut frame0: Option<Fragment> = None;

    for chunk in bytes.chunks(1 << 20) {
        framer.push(chunk);
        while let Some(frame) = framer.next() {
            if parse_topology(frame).is_some() {
                decoder.decode(frame); // カウンタは通す(このデータには来ない想定)
                continue;
            }
            frame_sizes.insert(frame.len());
            let Some(fragment) = decoder.decode(frame) else {
                panic!(
                    "real compact frame failed to decode (frame_len={})",
                    frame.len()
                );
            };
            hasher.update(header_canonical_bytes(&fragment));
            hasher.update(&fragment.items);
            if frame0.is_none() {
                frame0 = Some(fragment);
            }
        }
    }

    RunDigest {
        frames: decoder.frames(),
        items: decoder.items(),
        malformed: decoder.malformed(),
        unsupported: decoder.unsupported(),
        frame_sizes,
        sha256_hex: format!("{:x}", hasher.finalize()),
        frame0: frame0.expect("at least one data frame"),
    }
}

/// run0011(`reference/exp_data/2026/d3_pulser_scan/run0011/*.graw`)。
///
/// MFM オラクル実測(2026-09-23、`mfm_compact_dump`):
/// frames=60 items_total=8,355,840 frame_sizes={278784}
/// frame0: frameType=2 revision=5 cobo=0 asad=0 eventIdx=0 eventTime=116416830
///         readOffset=0 status=0 mult=[1,2,4,8] windowOut=0xffffffff
///         lastCell=[690,690,690,690]
/// frame0_first10=(2,0,0,416)(2,1,0,402)(3,0,0,476)(3,1,0,449)(0,0,0,477)
///                (0,1,0,460)(1,0,0,362)(1,1,0,429)(3,2,0,403)(3,3,0,474)
/// frame0_last5 =(3,67,511,472)(0,66,511,560)(0,67,511,535)(1,66,511,498)(1,67,511,445)
/// sha256_all_frames=a19c01f925d29a64e1136e8684b4070f59671b2cea15f2435cf6b22218e5e34b
#[test]
fn real_compact_run0011_matches_the_mfm_oracle() {
    let Some(root) = exp_data_root() else {
        return;
    };
    let file = single_graw_in(&root.join("d3_pulser_scan/run0011"));
    let run = decode_run(&file);

    eprintln!(
        "run0011: frames={} items={} malformed={} unsupported={} frame_sizes={:?} sha256={}",
        run.frames, run.items, run.malformed, run.unsupported, run.frame_sizes, run.sha256_hex
    );

    assert_eq!(run.malformed, 0, "malformed オラクル");
    assert_eq!(
        run.unsupported, 0,
        "topology 等の制御フレームは無い(単一ファイル 60 frame ぴったり)"
    );
    assert_eq!(run.frames, ORACLE_FRAMES, "データフレーム数");
    assert_eq!(
        run.items,
        ORACLE_FRAMES * ORACLE_ITEMS_PER_FRAME,
        "item 総数 = 60 × 139,264"
    );
    assert_eq!(
        run.frame_sizes.iter().copied().collect::<Vec<_>>(),
        vec![ORACLE_FRAME_BYTES],
        "全データフレームが 278,784 B 固定"
    );

    let f0 = &run.frame0;
    assert_eq!(f0.frame_type, 2);
    assert_eq!(f0.revision, 5);
    assert_eq!(f0.cobo, 0);
    assert_eq!(f0.asad, 0);
    assert_eq!(f0.event_idx, 0);
    assert_eq!(f0.event_time, 116_416_830);
    assert_eq!(f0.read_offset, 0);
    assert_eq!(f0.status, 0);
    assert_eq!(f0.mult, [1, 2, 4, 8]);
    assert_eq!(f0.window_out, 0xFFFF_FFFF);
    assert_eq!(f0.last_cell, [690, 690, 690, 690]);

    assert_eq!(
        run.sha256_hex, "a19c01f925d29a64e1136e8684b4070f59671b2cea15f2435cf6b22218e5e34b",
        "60 フレーム全部(ヘッダ + item)の SHA-256 が MFM オラクルと一致すること"
    );
}

/// run0006(`reference/exp_data/2026/d3_pulser_scan/run0006/*.graw`)。同一手順の別 run。
///
/// MFM オラクル実測(2026-09-23、`mfm_compact_dump`):
/// frames=60 items_total=8,355,840 frame_sizes={278784}
/// frame0: frameType=2 revision=5 cobo=0 asad=0 eventIdx=0 eventTime=116437331
///         readOffset=0 status=0 mult=[1,2,4,8] windowOut=0xffffffff
///         lastCell=[690,690,690,690]
/// frame0_first10=(1,0,0,368)(1,1,0,432)(2,0,0,430)(2,1,0,404)(3,0,0,461)
///                (3,1,0,449)(0,0,0,486)(0,1,0,468)(2,2,0,402)(2,3,0,397)
/// frame0_last5 =(2,67,511,452)(3,66,511,429)(3,67,511,446)(0,66,511,541)(0,67,511,516)
/// sha256_all_frames=4ab94f859bfcd55e5a8070205168cc7cde84435d9ac07d3a0ca16d1e4491283e
#[test]
fn real_compact_run0006_matches_the_mfm_oracle() {
    let Some(root) = exp_data_root() else {
        return;
    };
    let file = single_graw_in(&root.join("d3_pulser_scan/run0006"));
    let run = decode_run(&file);

    eprintln!(
        "run0006: frames={} items={} malformed={} unsupported={} frame_sizes={:?} sha256={}",
        run.frames, run.items, run.malformed, run.unsupported, run.frame_sizes, run.sha256_hex
    );

    assert_eq!(run.malformed, 0, "malformed オラクル");
    assert_eq!(
        run.unsupported, 0,
        "topology 等の制御フレームは無い(単一ファイル 60 frame ぴったり)"
    );
    assert_eq!(run.frames, ORACLE_FRAMES, "データフレーム数");
    assert_eq!(
        run.items,
        ORACLE_FRAMES * ORACLE_ITEMS_PER_FRAME,
        "item 総数 = 60 × 139,264"
    );
    assert_eq!(
        run.frame_sizes.iter().copied().collect::<Vec<_>>(),
        vec![ORACLE_FRAME_BYTES],
        "全データフレームが 278,784 B 固定"
    );

    let f0 = &run.frame0;
    assert_eq!(f0.frame_type, 2);
    assert_eq!(f0.revision, 5);
    assert_eq!(f0.cobo, 0);
    assert_eq!(f0.asad, 0);
    assert_eq!(f0.event_idx, 0);
    assert_eq!(f0.event_time, 116_437_331);
    assert_eq!(f0.read_offset, 0);
    assert_eq!(f0.status, 0);
    assert_eq!(f0.mult, [1, 2, 4, 8]);
    assert_eq!(f0.window_out, 0xFFFF_FFFF);
    assert_eq!(f0.last_cell, [690, 690, 690, 690]);

    assert_eq!(
        run.sha256_hex, "4ab94f859bfcd55e5a8070205168cc7cde84435d9ac07d3a0ca16d1e4491283e",
        "60 フレーム全部(ヘッダ + item)の SHA-256 が MFM オラクルと一致すること"
    );
}
