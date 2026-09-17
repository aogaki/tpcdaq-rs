//! pulser 較正表(TODO/081)。ch 毎のパルス波高中央値(baseline 差引後、生 ADC)と相対ゲイン。
//!
//! 定義は 068 SUMMARY §0 / `reference/_spike/asset_survey/src/pulser.rs` を踏襲する
//! (転記ではなく再実装): `baseline` = bucket `0..BASELINE_END` の平均、`波高` = 全 bucket の
//! 最大 ADC − baseline。FPN は 12 bit フルスケール(4095)へ飽和するため `relative_gain` は
//! `null` にし、`saturated`(飽和率)だけを出す。
//!
//! `amplitude_median` / `relative_gain` は**全入力ファイルの全イベントをプールした上での
//! チャンネル毎中央値**(mod.rs 冒頭の「run 数」解釈を参照 — ファイル/ラン単位の中間集約はしない)。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{
    is_fpn, median, now_rfc3339, read_json, scan_files, tool_version, write_json, CalibError,
    ChKey, Meta, AGET_PER_ASAD, RAW_CH_PER_AGET,
};
use crate::msg::{items_from_bytes, unpack_item};

/// baseline を取る bucket 範囲の終端(排他)。068 実測(信号 ch 立ち上がり ~85 / FPN 立ち上がり
/// ~72)より手前の安全側の値と同一。
pub const BASELINE_END: u16 = 64;
/// 12 bit ADC のフルスケール。これに達したイベントは飽和として数える。
pub const ADC_FULL_SCALE: u16 = 4095;

/// 1 ch 分のレコード。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PulserChannel {
    pub asad: u8,
    pub aget: u8,
    pub ch: u8,
    pub is_fpn: bool,
    /// チャンネル毎パルス波高の中央値(baseline 差引後、生 ADC)。
    pub amplitude_median: f64,
    /// `amplitude_median` / 全信号 ch の `amplitude_median` 中央値。FPN は `null`。
    pub relative_gain: Option<f64>,
    /// ピークが [`ADC_FULL_SCALE`] に達したイベントの割合(0.0–1.0)。
    pub saturated: f64,
    /// このチャンネルのサンプルが実際に含まれていた入力ファイルの数。
    pub runs: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PulserTable {
    pub meta: Meta,
    pub channels: Vec<PulserChannel>,
}

#[derive(Default)]
struct Accum {
    amps: Vec<f64>,
    saturated_events: u64,
    files: std::collections::BTreeSet<usize>,
}

/// 1 フレーム(= 1 AsAd 分)の per-(aget,ch) スクラッチ領域サイズ。AGET/AsAd のハードウェア
/// 定数のみに依存する(ASAD 枚数には依存しない — 「ch 数を焼き込まない」不変条件)。
const PER_FRAME: usize = AGET_PER_ASAD * RAW_CH_PER_AGET;

/// `paths` の全フレームを走査し、ch 毎の波高中央値/相対ゲイン/飽和率を計算する。
pub fn compute(paths: &[PathBuf]) -> Result<PulserTable, CalibError> {
    let mut accum: BTreeMap<ChKey, Accum> = BTreeMap::new();

    // per-frame スクラッチ(ホットパスで per-frame 確保しないよう外に出す — CLAUDE.md)。
    let mut base_sum = vec![0f64; PER_FRAME];
    let mut base_n = vec![0u32; PER_FRAME];
    let mut peak = vec![0u16; PER_FRAME];
    let mut seen = vec![false; PER_FRAME];

    let stats = scan_files(paths, |file_idx, frag| {
        let asad = frag.asad;
        let items = match items_from_bytes(&frag.items) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("calib: pulser: items_from_bytes failed for a decoded frame: {e}");
                return;
            }
        };

        base_sum.iter_mut().for_each(|v| *v = 0.0);
        base_n.iter_mut().for_each(|v| *v = 0);
        peak.iter_mut().for_each(|v| *v = 0);
        seen.iter_mut().for_each(|v| *v = false);

        for word in items {
            let it = unpack_item(word);
            let k = it.aget as usize * RAW_CH_PER_AGET + it.chan as usize;
            let Some(slot) = seen.get_mut(k) else {
                // aget/chan がハードウェア範囲(0-3 / 0-67)を超える(理論上到達しないはずの
                // プロトコル不整合)。silent に握りつぶさず報告する(CLAUDE.md)。
                eprintln!(
                    "calib: pulser: item aget={} chan={} out of the AGET/AsAd hardware range — skipped",
                    it.aget, it.chan
                );
                continue;
            };
            *slot = true;
            if it.bucket < BASELINE_END {
                base_sum[k] += f64::from(it.adc);
                base_n[k] += 1;
            }
            if it.adc > peak[k] {
                peak[k] = it.adc;
            }
        }

        for k in 0..PER_FRAME {
            if !seen[k] || base_n[k] == 0 {
                continue;
            }
            let aget = (k / RAW_CH_PER_AGET) as u8;
            let ch = (k % RAW_CH_PER_AGET) as u8;
            let baseline = base_sum[k] / f64::from(base_n[k]);
            let amp = f64::from(peak[k]) - baseline;
            let key = (asad, aget, ch);
            let a = accum.entry(key).or_default();
            a.amps.push(amp);
            if peak[k] >= ADC_FULL_SCALE {
                a.saturated_events += 1;
            }
            a.files.insert(file_idx);
        }
    })?;

    // ch 毎の中央値を先に確定し、次に全信号 ch の中央値(相対ゲインの基準点)を出す。
    let per_ch_median: BTreeMap<ChKey, f64> = accum
        .iter()
        .map(|(key, a)| (*key, median(&a.amps)))
        .collect();
    let signal_medians: Vec<f64> = per_ch_median
        .iter()
        .filter(|((_, _, ch), _)| !is_fpn(*ch))
        .map(|(_, m)| *m)
        .collect();
    let global_median = median(&signal_medians);

    let channels: Vec<PulserChannel> = accum
        .into_iter()
        .map(|(key, a)| {
            let (asad, aget, ch) = key;
            let fpn = is_fpn(ch);
            let amp_med = *per_ch_median.get(&key).unwrap_or(&f64::NAN);
            let n = a.amps.len() as u64;
            let saturated = if n == 0 {
                0.0
            } else {
                a.saturated_events as f64 / n as f64
            };
            PulserChannel {
                asad,
                aget,
                ch,
                is_fpn: fpn,
                amplitude_median: amp_med,
                relative_gain: if fpn {
                    None
                } else {
                    Some(amp_med / global_median)
                },
                saturated,
                runs: a.files.len() as u32,
            }
        })
        .collect();

    Ok(PulserTable {
        meta: Meta {
            inputs: paths.iter().map(|p| p.display().to_string()).collect(),
            events: stats.events,
            generated_at: now_rfc3339(),
            tool_version: tool_version(),
        },
        channels,
    })
}

pub fn write(table: &PulserTable, out: &Path) -> Result<(), CalibError> {
    write_json(table, out)
}

pub fn read(path: &Path) -> Result<PulserTable, CalibError> {
    read_json(path)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn amplitude_is_peak_minus_baseline_hand_calculation() {
        // baseline bucket 0..1(BASELINE_END を 1 に見立てず、既定 64 のまま): bucket0=100,
        // ピークは bucket100=500。baseline=100(bucket0 のみ寄与)。amp=500-100=400。
        let mut base_sum = 0.0f64;
        let mut base_n = 0u32;
        let mut peak = 0u16;
        for (bucket, adc) in [(0u16, 100u16), (100, 500)] {
            if bucket < BASELINE_END {
                base_sum += f64::from(adc);
                base_n += 1;
            }
            if adc > peak {
                peak = adc;
            }
        }
        let baseline = base_sum / f64::from(base_n);
        let amp = f64::from(peak) - baseline;
        assert_eq!(baseline, 100.0);
        assert_eq!(amp, 400.0);
    }
}
