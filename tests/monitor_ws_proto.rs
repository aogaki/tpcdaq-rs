//! `ws_proto_sample`(SPEC §10.4-1 の生成器、TODO/026)の回帰。
//!
//! * **決定的**であること(同一入力 → 同一バイト)—— TS 側(027)の検証器が
//!   毎回再生成したフィクスチャと突き合わせるので、ここが揺れると CI が揺れる。
//! * `u32 LE 長さ + ペイロード` の連結として分解でき、全メッセージ型が既知値で入っていること
//!   (SPEC §10.4-4 の Rust 側レイアウトテストを**生成物に対して**もう一度掛ける)。

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;
use std::process::Command;

use tpcdaq::msg;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tpcdaq_ws_proto_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(format!("ws_sample_{tag}.bin"))
}

fn generate(path: &PathBuf) -> Vec<u8> {
    let status = Command::new(env!("CARGO_BIN_EXE_ws_proto_sample"))
        .arg("--out")
        .arg(path)
        .status()
        .expect("run ws_proto_sample");
    assert!(status.success(), "ws_proto_sample failed: {status:?}");
    std::fs::read(path).expect("read the generated sample")
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn f32_at(bytes: &[u8], at: usize) -> f32 {
    f32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

#[test]
fn the_sample_stream_is_deterministic() {
    let first = generate(&scratch("a"));
    let second = generate(&scratch("b"));
    assert_eq!(first, second, "同一入力 → 同一バイト(SPEC §10.4-3)");
    assert!(!first.is_empty());
}

#[test]
fn the_sample_stream_decodes_to_the_known_values() {
    let path = scratch("decode");
    let bytes = generate(&path);
    let frames = msg::split_length_prefixed(&bytes).expect("u32 長さ前置で分解できる");
    assert_eq!(frames.len(), 4, "0x02 / 0x03 / 0x10 / 0x11 の 4 通");

    for frame in &frames {
        // SPEC §10.1 の共通ヘッダ
        assert_eq!(&frame[0..2], b"TP", "magic");
        assert_eq!(frame[3], 2, "version = 2");
        assert_eq!(u32_at(frame, 5), 7, "runNumber = 7");
    }

    // --- 0x02 Uvw: V 面(1)、3 strip × 4 bucket、値 = strip*10 + bucket ---
    let uvw = frames[0];
    assert_eq!(uvw[2], 0x02);
    assert_eq!(uvw[4], 0, "complete");
    assert_eq!(u32_at(uvw, 9), 42, "eventNumber");
    assert_eq!(uvw[13], 1, "plane = V");
    assert_eq!(u16_at(uvw, 14), 3, "nStrips");
    assert_eq!(u16_at(uvw, 16), 4, "nBuckets");
    assert_eq!(uvw.len(), 18 + 3 * 4 * 2);
    for strip in 1..=3u16 {
        for bucket in 0..4u16 {
            let idx = (strip as usize - 1) * 4 + bucket as usize;
            assert_eq!(
                u16_at(uvw, 18 + idx * 2),
                strip * 10 + bucket,
                "strip-major idx=(strip-1)*nBuckets+bucket"
            );
        }
    }

    // --- 0x03 Waveforms: cobo0/asad1、2 aget × 3 ch × 2 bucket、incomplete ---
    let wf = frames[1];
    assert_eq!(wf[2], 0x03);
    assert_eq!(wf[4], 0x01, "flags bit0 = incomplete");
    assert_eq!(u32_at(wf, 9), 43, "eventNumber");
    assert_eq!((wf[13], wf[14]), (0, 1), "cobo/asad");
    assert_eq!((wf[15], wf[16]), (2, 3), "nAget/nCh");
    assert_eq!(u16_at(wf, 17), 2, "nBuckets");
    assert_eq!(wf.len(), 19 + 2 * 3 * 2 * 2);
    for aget in 0..2usize {
        for ch in 0..3usize {
            for bucket in 0..2usize {
                let idx = (aget * 3 + ch) * 2 + bucket;
                assert_eq!(
                    u16_at(wf, 19 + idx * 2),
                    (aget * 100 + ch * 10 + bucket) as u16,
                    "aget-major・raw ch 順"
                );
            }
        }
    }

    // --- 0x10 Histo1d: id 4(ChargeU)、4 ビン、軸 [0,4096) ---
    let h1 = frames[2];
    assert_eq!(h1[2], 0x10);
    assert_eq!(u32_at(h1, 9), 0, "ヒストの eventNumber は 0");
    assert_eq!(u16_at(h1, 13), 4, "id");
    assert_eq!(u32_at(h1, 15), 4, "nbins");
    assert_eq!(f32_at(h1, 19), 0.0, "xmin");
    assert_eq!(f32_at(h1, 23), 4096.0, "xmax");
    let body: Vec<f32> = h1[27..].chunks_exact(4).map(|c| f32_at(c, 0)).collect();
    assert_eq!(body, vec![0.0, 1.5, 2.25, 3.75]);

    // --- 0x11 Histo2d: id 1(StripTimeU)、nx3 × ny2、iy 外側 row-major ---
    let h2 = frames[3];
    assert_eq!(h2[2], 0x11);
    assert_eq!(u16_at(h2, 13), 1, "id");
    assert_eq!(u16_at(h2, 15), 3, "nx");
    assert_eq!(u16_at(h2, 17), 2, "ny");
    assert_eq!(f32_at(h2, 19), 1.0, "xmin = 1(strip は 1 始まり)");
    assert_eq!(f32_at(h2, 23), 4.0, "xmax = nx + 1");
    assert_eq!(f32_at(h2, 27), 0.0, "ymin");
    assert_eq!(f32_at(h2, 31), 2.0, "ymax");
    let body: Vec<f32> = h2[35..].chunks_exact(4).map(|c| f32_at(c, 0)).collect();
    // 入力(PUB 順・ix 外側)= [11,12, 21,22, 31,32] → 転置して iy 外側。
    assert_eq!(body, vec![11.0, 21.0, 31.0, 12.0, 22.0, 32.0]);

    let _ = std::fs::remove_file(&path);
}
