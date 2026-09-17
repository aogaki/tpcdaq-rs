//! 較正表ツール `calib`(TODO/081)。pedestal / pulser run(実機 .graw、AsAd 毎ファイル)から
//! per-channel の baseline / noise / 相対ゲイン / 異常フラグを JSON にまとめる。
//!
//! フレーム切り出しは本体 [`crate::framer::Framer`]、item 展開は本体
//! [`crate::decode::Decoder`] をそのまま使う(asset_survey(`reference/_spike/asset_survey`)の
//! `fast.rs` 高速経路は持ち込まない — KISS。068 実測: pedestal 全走査 74 s。Decoder 直経由は
//! それより遅くなりうるが、遅すぎても最適化はしない — 発注書どおり実測を報告するだけに留める)。
//!
//! 数値の定義(mean/rms は生 ADC・減算なし、pulser の baseline 窓・飽和判定など)は
//! 068 SUMMARY §0 / `reference/_spike/asset_survey/src/{pedestal,pulser}.rs` を読んで踏襲する
//! (転記ではなく再実装。定義は変えない)。
//!
//! # 「run 数」の解釈(発注書に明記が無いので、ここに解釈を記録する)
//!
//! `calib pedestal` / `calib pulser` は **CLI に渡された `.graw` ファイルの列**を処理する
//! (ディレクトリ走査もランのグルーピングもしない)。したがってチャンネル毎の `runs` は
//! 「そのチャンネルのサンプルが実際に含まれていた入力ファイルの数」と定義する
//! (asset_survey のようなファイル名からのラン推定はしない — KISS。1 run 分の AsAd 毎ファイルを
//! まとめて渡す通常運用では、1 チャンネルは 1 ファイルにしか現れないので `runs` は自然に
//! 「そのチャンネルが載っていたファイル数」になる)。同様に pulser の「中央値」は **全入力
//! ファイルの全イベントをプールした上でのチャンネル毎中央値**(ファイル/ラン単位の中間集約はしない)。

pub mod pedestal;
pub mod pulser;
pub mod table;

use std::collections::BTreeSet;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::decode::Decoder;
use crate::framer::Framer;
use crate::geometry::REORDER_FROM_GEOMETRY_TO_GRAW;
use crate::msg::Fragment;

/// AGET 1 チップの raw チャンネル数(信号 64 + FPN 4、AGET ハードウェア定数。
/// `geometry::RAW_CH_PER_AGET` と同じ値 — ここでは usize 版として再エクスポートする)。
pub const RAW_CH_PER_AGET: usize = crate::geometry::RAW_CH_PER_AGET as usize;
/// AsAd 1 枚あたりの AGET チップ数(GET ハードウェア定数)。
pub const AGET_PER_ASAD: usize = crate::geometry::AGET_CHIPS_PER_ASAD as usize;

/// `(asad, aget, ch)` の昇順キー。
pub type ChKey = (u8, u8, u8);

/// raw ch(0–67)が FPN(非信号)かどうか。
///
/// `geometry::REORDER_FROM_GEOMETRY_TO_GRAW`(信号 ch 0–63 → raw ch の並び替え定数表)に
/// 現れない raw ch = FPN。CLAUDE.md の FPN raw ch = {11,22,45,56} をハードコードし直すのではなく
/// 既存の正本テーブルを参照することで定義を一箇所に保つ。
pub fn is_fpn(raw_ch: u8) -> bool {
    !REORDER_FROM_GEOMETRY_TO_GRAW.contains(&u32::from(raw_ch))
}

/// 較正ツール共通のエラー(silent failure を作らない — CLAUDE.md)。
#[derive(Debug, thiserror::Error)]
pub enum CalibError {
    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("JSON encode error for {path}: {source}")]
    JsonEncode {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("JSON decode error for {path}: {source}")]
    JsonDecode {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("no input .graw files given")]
    NoInputFiles,
}

/// 出力 JSON 共通のメタ情報(発注書: 「入力ファイル一覧・イベント数・日付・ツール版」)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Meta {
    pub inputs: Vec<String>,
    pub events: u64,
    pub generated_at: String,
    pub tool_version: String,
}

/// 現在時刻を RFC3339 で(`meta.generated_at`)。
pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// `env!("CARGO_PKG_VERSION")`(発注書どおり)。
pub fn tool_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// 値を pretty-print JSON でファイルへ書く。
pub fn write_json<T: Serialize>(value: &T, path: &Path) -> Result<(), CalibError> {
    let json = serde_json::to_string_pretty(value).map_err(|source| CalibError::JsonEncode {
        path: path.to_path_buf(),
        source,
    })?;
    std::fs::write(path, json).map_err(|source| CalibError::Write {
        path: path.to_path_buf(),
        source,
    })
}

/// JSON ファイルを読んで構造体へ復元する。
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, CalibError> {
    let text = std::fs::read_to_string(path).map_err(|source| CalibError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&text).map_err(|source| CalibError::JsonDecode {
        path: path.to_path_buf(),
        source,
    })
}

/// フレーム走査の集計(silent にせず呼び出し側 CLI が表示できるように返す)。
#[derive(Debug, Default, Clone, Copy)]
pub struct ScanStats {
    /// デコード成功フレーム数(= 実 graw オラクルの "events" に相当する frame 数)。
    pub frames: u64,
    pub malformed: u64,
    pub unsupported: u64,
    /// distinct `event_idx` の総数。
    pub events: u64,
}

/// `paths` の各ファイルを先頭から順にフレーム走査し、デコードできたフレーム毎に
/// `on_frag(file_index, &Fragment)` を呼ぶ。
///
/// フレーマは**ファイル毎に新規**(MFM フレームはファイル境界を跨がない)。デコーダは
/// 全ファイルを通して 1 個(malformed/unsupported カウンタを累積するため)。
/// malformed/unsupported/reset を検出したファイルは stderr に 1 行warning を出す
/// (CLAUDE.md「silent failure を作らない」— cache miss/範囲外値/プロトコル不一致の可視化)。
pub fn scan_files(
    paths: &[PathBuf],
    mut on_frag: impl FnMut(usize, &Fragment),
) -> Result<ScanStats, CalibError> {
    if paths.is_empty() {
        return Err(CalibError::NoInputFiles);
    }
    const CHUNK: usize = 8 * 1024 * 1024;
    let mut seen_events: BTreeSet<u32> = BTreeSet::new();
    let mut decoder = Decoder::new();

    for (file_idx, path) in paths.iter().enumerate() {
        let mut file = File::open(path).map_err(|source| CalibError::Read {
            path: path.clone(),
            source,
        })?;
        let mut framer = Framer::new();
        let mut buf = vec![0u8; CHUNK];
        let malformed_before = decoder.malformed();
        let unsupported_before = decoder.unsupported();

        loop {
            let n = file.read(&mut buf).map_err(|source| CalibError::Read {
                path: path.clone(),
                source,
            })?;
            if n == 0 {
                break;
            }
            framer.push(&buf[..n]);
            while let Some(frame) = framer.next() {
                if let Some(frag) = decoder.decode(frame) {
                    seen_events.insert(frag.event_idx);
                    on_frag(file_idx, &frag);
                }
            }
        }

        let new_malformed = decoder.malformed() - malformed_before;
        let new_unsupported = decoder.unsupported() - unsupported_before;
        if new_malformed > 0 {
            eprintln!(
                "calib: {}: {new_malformed} malformed frame(s) skipped",
                path.display()
            );
        }
        if new_unsupported > 0 {
            eprintln!(
                "calib: {}: {new_unsupported} unsupported/control frame(s) skipped",
                path.display()
            );
        }
        if framer.reset_count() > 0 {
            eprintln!(
                "calib: {}: framer resynced {} time(s) (corrupt header)",
                path.display(),
                framer.reset_count()
            );
        }
    }

    Ok(ScanStats {
        frames: decoder.frames(),
        malformed: decoder.malformed(),
        unsupported: decoder.unsupported(),
        events: seen_events.len() as u64,
    })
}

/// 中央値(コピーしてソート。非有限値は除く)。空/非有限値のみなら `NaN`。
///
/// 定義は `reference/_spike/asset_survey/src/lib.rs` の `median` と同一(再実装)。
pub fn median(values: &[f64]) -> f64 {
    let mut v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// MAD(median absolute deviation)。正規分布 σ 換算はしない(生の MAD を返す)。
///
/// 定義は `reference/_spike/asset_survey/src/lib.rs` の `mad` と同一(再実装)。
pub fn mad(values: &[f64], med: f64) -> f64 {
    let dev: Vec<f64> = values
        .iter()
        .copied()
        .filter(|x| x.is_finite())
        .map(|x| (x - med).abs())
        .collect();
    median(&dev)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn is_fpn_matches_the_known_fpn_raw_channels() {
        // 手計算の出典: CLAUDE.md / geometry.rs FPN_RAW_CHANNELS = {11,22,45,56}。
        for &raw in &[11u8, 22, 45, 56] {
            assert!(is_fpn(raw), "raw {raw} should be FPN");
        }
        for &raw in &[0u8, 1, 10, 12, 63, 67] {
            assert!(!is_fpn(raw), "raw {raw} should be signal");
        }
    }

    #[test]
    fn median_matches_hand_calculation_for_odd_and_even_counts() {
        // 手計算: [1,3,5] は奇数個なので中央値 = 3。
        assert_eq!(median(&[1.0, 3.0, 5.0]), 3.0);
        // 手計算: [1,2,3,4] は偶数個なので中央値 = (2+3)/2 = 2.5。
        assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), 2.5);
        assert!(median(&[]).is_nan());
    }

    #[test]
    fn mad_matches_hand_calculation() {
        // 手計算: 値 [1,2,3,4,5]、中央値 3。偏差の絶対値 = [2,1,0,1,2] → 中央値 = 1。
        let med = median(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(mad(&[1.0, 2.0, 3.0, 4.0, 5.0], med), 1.0);
    }
}
