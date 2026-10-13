use std::path::Path;

use super::*;
use crate::reco::{Track, TrackEvent, TrackPoint};

// =====================================================================
// SPEC §10.4-1 — 適合性テスト用のサンプルストリーム(既知値)
// =====================================================================

/// 全 WS メッセージ型を**既知値**でエンコードしたもの(SPEC §10.4-1)。
///
/// 値は非対称に選んである(取り違えが目に見えるように)。TS 側(027)の本番デコーダが
/// これを分解して同じ値を復元できることが適合性の定義。**同一入力 → 同一バイト**。
///
/// 並びは 0x02 / 0x03 / 0x10 / 0x11 / 0x04(0x04 は TODO/127 で末尾に足した)。
/// Err は 0x04 の JSON 化の失敗だけ(`Vec` への書き込みなので実際には起きない)。
pub fn ws_sample_messages() -> Result<Vec<Vec<u8>>, serde_json::Error> {
    // Uvw: 3 ストリップ × 4 バケット。値は strip*10 + bucket(全ビン相異なる)。
    let n_strips: u16 = 3;
    let n_buckets: u16 = 4;
    let mut grid = Vec::with_capacity((n_strips * n_buckets) as usize);
    for strip in 1..=n_strips {
        for bucket in 0..n_buckets {
            grid.push(strip * 10 + bucket);
        }
    }
    let uvw = ws::uvw(
        ws::Header::event(ws::TYPE_UVW, 7, 42, false),
        1, // V 面
        n_strips,
        n_buckets,
        &grid,
    );

    // Waveforms: 2 AGET × 3 ch × 2 bucket(実機の 4×68×512 の縮小版)。
    let (n_aget, n_ch, wf_buckets) = (2u8, 3u8, 2u16);
    let mut wf = Vec::with_capacity(n_aget as usize * n_ch as usize * wf_buckets as usize);
    for aget in 0..n_aget as u16 {
        for ch in 0..n_ch as u16 {
            for bucket in 0..wf_buckets {
                wf.push(aget * 100 + ch * 10 + bucket);
            }
        }
    }
    let waveforms = ws::waveforms(
        ws::Header::event(ws::TYPE_WAVEFORMS, 7, 43, true), // incomplete = flags bit0
        0,
        1,
        n_aget,
        n_ch,
        wf_buckets,
        &wf,
    );

    // Histo1d: 4 ビン(0.5 刻みで f32 に落ちても値が変わらない数)。
    let h1: Vec<f64> = vec![0.0, 1.5, 2.25, 3.75];
    let histo1d = ws::histo1d(
        ws::Header::hist(ws::TYPE_HISTO1D, 7),
        4, // ChargeU(SPEC §5.2)
        ws::Axis {
            min: 0.0,
            max: CHARGE_AXIS_MAX,
        },
        &f64_bytes(&h1),
    );

    // Histo2d: nx=3(strip)× ny=2(bucket)。入力は PUB 順(ix 外側)。
    let h2: Vec<f64> = vec![11.0, 12.0, 21.0, 22.0, 31.0, 32.0];
    let histo2d = ws::histo2d(
        ws::Header::hist(ws::TYPE_HISTO2D, 7),
        1, // StripTimeU
        3,
        2,
        ws::Axis { min: 1.0, max: 4.0 },
        ws::Axis { min: 0.0, max: 2.0 },
        &f64_bytes(&h2),
    );

    // Track: run=7 / event=44 / complete。本文は TODO/127 の発注書の JSON 例そのもの
    // (UI レーンとの契約 — 値を変えるときは UI の適合 spec と同時に)。
    let points = [
        TrackPoint {
            x: -52.9,
            y: -7.9,
            t: 152.0,
            q: 5480.0,
            view: 0,
        },
        TrackPoint {
            x: -51.4,
            y: -7.8,
            t: 151.6,
            q: 5512.0,
            view: 0,
        },
    ];
    let track = ws::track(
        ws::Header::event(ws::TYPE_TRACK, 7, 44, false),
        &TrackEvent {
            hits: [72, 40, 34],
            halo_hits: [72, 40, 34],
            unplaced: 0,
            no_segment: 0,
            fit: Ok(Track {
                phi_deg: 2.7,
                dir: [0.998, 0.047, -0.031],
                start: [-53.1, -7.9, 26.4],
                end: [53.0, -3.0, 23.1],
                start_f: [-53.5, -8.0, 26.4],
                end_f: [53.6, -3.0, 23.0],
                length: 106.2,
                length_f: 107.2,
                xy_length: 106.1,
                charge: 412345.0,
            }),
            points: &points,
        },
    )?;

    Ok(vec![uvw, waveforms, histo1d, histo2d, track])
}

/// f64 列を SPEC §5.3 の生バイト列(f64 LE)にする。
pub(super) fn f64_bytes(values: &[f64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 8);
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// [`ws_sample_messages`] を `u32 LE 長さ + ペイロード` の連結ストリームとして書き出す
/// (SPEC §10.4-1。生成物はコミットしない — 毎回再生成)。
pub fn write_ws_sample_stream(path: &Path) -> Result<(), std::io::Error> {
    use std::io::Write as _;
    let file = std::fs::File::create(path)?;
    let mut out = std::io::BufWriter::new(file);
    for frame in ws_sample_messages()? {
        let len = u32::try_from(frame.len()).unwrap_or(u32::MAX);
        out.write_all(&len.to_le_bytes())?;
        out.write_all(&frame)?;
    }
    out.flush()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn ws_sample_messages_are_deterministic_and_cover_every_type() {
        let first = ws_sample_messages().unwrap();
        let second = ws_sample_messages().unwrap();
        assert_eq!(first, second, "同一入力 → 同一バイト");

        let types: Vec<u8> = first.iter().map(|m| m[2]).collect();
        assert_eq!(
            types,
            vec![
                ws::TYPE_UVW,
                ws::TYPE_WAVEFORMS,
                ws::TYPE_HISTO1D,
                ws::TYPE_HISTO2D,
                ws::TYPE_TRACK
            ],
            "全メッセージ型が 1 通ずつ"
        );
        for message in &first {
            assert_eq!(&message[0..2], b"TP", "magic");
            assert_eq!(message[3], ws::VERSION);
        }
        // incomplete の例が入っていること(flags bit0 の検証材料)。
        assert_eq!(first[1][4], ws::FLAG_INCOMPLETE);
    }
}
