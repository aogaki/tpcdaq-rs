//! ELITPC 実データ回帰(TODO/019、SPEC §12-2 v1.7)。
//!
//! `TPCDAQ_REAL_GRAW_DIR` に実データディレクトリ(例: `reference/exp_data/2022`)が
//! 入っているときだけ走る任意テスト(実 .graw はリポに入れない — CLAUDE.md)。
//! 2022 / 2026 の両セットで同一オラクル(eventTime の実値以外は構造まで同一と実測済み)。
//!
//! オラクル(2026-08-13、両年実測):
//! - ディレクトリ = `CoBo0_AsAd{0..3}_{TS}_0000.graw` の 4 ファイル(1 論理 CoBo × 4 AsAd)
//! - 各ファイル: 3852 フレーム = 1,073,875,968 B、全フレーム固定 278,784 B、
//!   frameType 2・revision 5・cobo 0、eventIdx 0..=3851 連続、eventTime 単調、
//!   malformed=0・unsupported=0(制御フレームなし)・resync=0・残余 0 B
//! - ローテーション: 3852 フレーム目が 2^30 B を**超えてから**次ファイル
//!   (FrameStorage.cpp `write → tellp() > 1024 MiB → createNewFile` — 書き込み後判定)
//!
//! 実行(1 GiB ×4 を舐めるので release 推奨):
//! `TPCDAQ_REAL_GRAW_DIR=reference/exp_data/2022 cargo test --release --test elitpc_real_graw -- --nocapture`

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;

use serde_bytes::ByteBuf;
use tpcdaq::decode::Decoder;
use tpcdaq::framer::Framer;
use tpcdaq::graw_writer::RunWriter;

const FRAMES_PER_FILE: u64 = 3852;
const FRAME_BYTES: usize = 278_784; // 256 B ヘッダ + 139,264 item × 2 B(フル読み出し)
const FILE_BYTES: usize = FRAMES_PER_FILE as usize * FRAME_BYTES; // 1,073,875,968
const ITEMS_PER_FILE: u64 = FRAMES_PER_FILE * 139_264; // 4 AGET × 68 ch × 512 bucket

/// ディレクトリ内の実 .graw をソート済みで返す(ファイル名昇順 = AsAd 昇順)。
fn real_graw_files(dir: &str) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("cannot read {dir}: {e}"))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "graw"))
        .collect();
    paths.sort();
    paths
}

/// `CoBo{K}_AsAd{A}_{TS}_{idx:04}.graw` から (cobo, asad, idx) を取り出す(命名自体の検証)。
fn parse_datarouter_name(name: &str) -> (u32, u8, u32) {
    let parts: Vec<&str> = name.strip_suffix(".graw").unwrap().split('_').collect();
    assert_eq!(parts.len(), 4, "DataRouter 命名は 4 要素: {name}");
    let cobo = parts[0].strip_prefix("CoBo").unwrap().parse().unwrap();
    let asad = parts[1].strip_prefix("AsAd").unwrap().parse().unwrap();
    assert_eq!(parts[3].len(), 4, "idx は 4 桁ゼロ詰め: {name}");
    let idx = parts[3].parse().unwrap();
    (cobo, asad, idx)
}

/// 4 ファイル全部を Framer+Decoder に通し、構造オラクルを固定する(TODO/019 実測値)。
#[test]
fn elitpc_real_graw_decodes_to_the_fixed_structural_oracle() {
    let Ok(dir) = std::env::var("TPCDAQ_REAL_GRAW_DIR") else {
        eprintln!("SKIP: TPCDAQ_REAL_GRAW_DIR が未設定(実 .graw はローカルのみ)");
        return;
    };
    let paths = real_graw_files(&dir);
    assert_eq!(paths.len(), 4, "ELITPC は 1 CoBo × 4 AsAd = 4 ファイル");

    let mut seen_asads = Vec::new();
    for (i, path) in paths.iter().enumerate() {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap();
        let (cobo, asad, idx) = parse_datarouter_name(name);
        assert_eq!(cobo, 0, "ELITPC はワイヤ上 1 論理 CoBo(coboIdx=0): {name}");
        assert_eq!(asad as usize, i, "AsAd はファイル名昇順で 0..=3: {name}");
        assert_eq!(idx, 0, "このデータセットは各 AsAd の先頭ファイル: {name}");
        seen_asads.push(asad);

        let bytes = std::fs::read(path).unwrap();
        assert_eq!(bytes.len(), FILE_BYTES, "ファイルバイト数オラクル: {name}");

        let mut framer = Framer::new();
        let mut dec = Decoder::new();
        let mut prev_event: Option<u32> = None;
        let mut prev_time: Option<u64> = None;
        for chunk in bytes.chunks(1 << 20) {
            framer.push(chunk);
            while let Some(frame) = framer.next() {
                assert_eq!(frame.len(), FRAME_BYTES, "全フレーム固定長: {name}");
                let Some(frag) = dec.decode(frame) else {
                    continue; // malformed/unsupported はループ後のカウンタ照合で 0 を確認する
                };
                assert_eq!(frag.frame_type, 2, "両年とも compact: {name}");
                assert_eq!(frag.revision, 5, "{name}");
                assert_eq!(frag.cobo, 0, "{name}");
                assert_eq!(frag.asad, asad, "ファイル名の AsAd と中身が一致: {name}");
                if let Some(prev) = prev_event {
                    assert_eq!(frag.event_idx, prev + 1, "eventIdx は連続: {name}");
                }
                prev_event = Some(frag.event_idx);
                if let Some(prev) = prev_time {
                    assert!(frag.event_time >= prev, "eventTime は単調: {name}");
                }
                prev_time = Some(frag.event_time);
            }
        }

        assert_eq!(dec.frames(), FRAMES_PER_FILE, "フレーム数オラクル: {name}");
        assert_eq!(dec.items(), ITEMS_PER_FILE, "item 数オラクル: {name}");
        assert_eq!(dec.malformed(), 0, "{name}");
        assert_eq!(
            dec.unsupported(),
            0,
            "制御フレームなし(mini と違う): {name}"
        );
        assert_eq!(framer.reset_count(), 0, "{name}");
        assert_eq!(
            framer.buffered(),
            0,
            "残余なし = 末尾まで完全フレーム: {name}"
        );
        assert_eq!(prev_event, Some(3851), "eventIdx は 0..=3851: {name}");
        eprintln!("{name}: frames={} items={} ok", dec.frames(), dec.items());
    }
    assert_eq!(seen_asads, vec![0, 1, 2, 3]);
}

/// ローテーション実機一致(TODO/019 の核心): AsAd0 実ファイル(3852 フレーム =
/// 1,073,875,968 B > 2^30)を既定 max_file_bytes(2^30)の RunWriter に流すと、
/// 実機 FrameStorage と同じく**境界を跨いだ 3852 フレーム目まで _0000 に残り**、
/// _0000 が入力と完全バイト一致になる。ローテーションは書き込み後判定なので、
/// 直後に開かれた _0001 は空のまま閉じられる(createNewFile 即時オープンと同一挙動)。
#[test]
fn elitpc_real_file_rotation_matches_the_real_datarouter_boundary() {
    let Ok(dir) = std::env::var("TPCDAQ_REAL_GRAW_DIR") else {
        eprintln!("SKIP: TPCDAQ_REAL_GRAW_DIR が未設定(実 .graw はローカルのみ)");
        return;
    };
    let path = &real_graw_files(&dir)[0]; // AsAd0
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(bytes.len(), FILE_BYTES);
    assert!(
        bytes.len() as u64 > tpcdaq::config::DEFAULT_GRAW_WRITER_MAX_FILE_BYTES,
        "このファイルは既定 1 GiB を 1 フレーム分だけ超える(境界オラクルの前提)"
    );

    let root = std::env::temp_dir().join(format!("tpcdaq-elitpc-rot-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    const RUN: u32 = 1;
    let mut w = RunWriter::new(
        root.clone(),
        RUN,
        tpcdaq::config::DEFAULT_GRAW_WRITER_MAX_FILE_BYTES,
    );

    let mut framer = Framer::new();
    let mut batch: Vec<ByteBuf> = Vec::new();
    let mut seq = 0u64;
    for chunk in bytes.chunks(1 << 20) {
        framer.push(chunk);
        while let Some(frame) = framer.next() {
            batch.push(ByteBuf::from(frame.to_vec()));
            if batch.len() == 64 {
                w.handle_batch(0, RUN, seq, &batch).unwrap();
                seq += 1;
                batch.clear();
            }
        }
    }
    if !batch.is_empty() {
        w.handle_batch(0, RUN, seq, &batch).unwrap();
    }
    w.finalize().unwrap();
    assert!(!w.errored());

    let files = w.file_report();
    assert_eq!(
        files.len(),
        2,
        "_0000(全 3852 フレーム)+ ローテーション直後の空 _0001: {files:?}"
    );
    let idx0 = files.iter().find(|f| f.idx == 0).unwrap();
    let idx1 = files.iter().find(|f| f.idx == 1).unwrap();
    assert_eq!(
        idx0.frames, FRAMES_PER_FILE,
        "境界を跨いだフレームは現ファイルに残る(実機 FrameStorage の書き込み後判定)"
    );
    assert_eq!(idx0.bytes, FILE_BYTES as u64);
    assert_eq!(idx1.frames, 0, "次ファイルは即時オープンされ空のまま");
    assert_eq!(idx1.bytes, 0);

    let name0 = idx0.path.file_name().and_then(|n| n.to_str()).unwrap();
    let (cobo, asad, idx) = parse_datarouter_name(name0);
    assert_eq!(
        (cobo, asad, idx),
        (0, 0, 0),
        "実機 DataRouter 命名: {name0}"
    );

    let written = std::fs::read(&idx0.path).unwrap();
    assert!(
        written == bytes,
        "_0000 が入力の実ファイルと完全バイト一致(ローテーション境界まで含めて実機一致)"
    );

    let _ = std::fs::remove_dir_all(&root);
}
