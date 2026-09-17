//! 較正表マージ(TODO/081)。`pedestal.json` + `pulser.json` → per-channel の
//! baseline/noise/gain + 異常フラグ + 閾値初期値の 1 本化。
//!
//! レコード形は発注書どおり厳守: `{asad, aget, ch, fpn, baseline, noise, gain, flags, threshold_suggest}`。

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::pedestal::{PedestalChannel, PedestalTable};
use super::pulser::{PulserChannel, PulserTable};
use super::{mad, median, now_rfc3339, tool_version, CalibError, ChKey};

/// `--k` の既定値(発注書の CLI 例 `--k 5` と同一)。
pub const DEFAULT_K: f64 = 5.0;
/// `dead` 判定の絶対閾値(発注書どおり)。
pub const DEAD_RMS_THRESHOLD: f64 = 0.5;
/// `gain_out` 判定の MAD 倍数(発注書どおり: `|gain-1| > 2·MAD`)。
pub const GAIN_OUT_MAD_K: f64 = 2.0;
/// `threshold_suggest` のノイズ倍数(発注書どおり: `baseline + 5·noise`)。
pub const THRESHOLD_NOISE_SIGMA: f64 = 5.0;
/// `saturated` フラグの飽和率閾値。
///
/// **発注書に数値の指定がない**(`noisy`/`dead`/`gain_out` は式が明示されているが
/// `saturated` だけ条件式が書かれていない)。068 SUMMARY で FPN の飽和率が 98.7%、信号 ch が
/// 0% だったことを踏まえ、両者を安全に分ける「過半数」を暫定値として採用した。
/// **要レビュー**(最終報告に明記)。
pub const SATURATED_RATE_THRESHOLD: f64 = 0.5;

/// 1 ch 分のレコード。発注書の順序をそのまま守る。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CalibChannel {
    pub asad: u8,
    pub aget: u8,
    pub ch: u8,
    pub fpn: bool,
    pub baseline: f64,
    pub noise: f64,
    pub gain: Option<f64>,
    pub flags: Vec<String>,
    pub threshold_suggest: i64,
}

/// `calib.json` の `meta`(k と MAD の値を残す — 発注書「機械基準を残す」)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeMeta {
    pub pedestal_inputs: Vec<String>,
    pub pulser_inputs: Vec<String>,
    pub pedestal_events: u64,
    pub pulser_events: u64,
    pub generated_at: String,
    pub tool_version: String,
    pub k: f64,
    pub noise_signal_median: f64,
    pub noise_signal_mad: f64,
    pub noise_fpn_median: f64,
    pub noise_fpn_mad: f64,
    pub gain_median: f64,
    pub gain_mad: f64,
    pub saturated_rate_threshold: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibTable {
    pub meta: MergeMeta,
    pub channels: Vec<CalibChannel>,
}

/// `pedestal` と `pulser` を `(asad,aget,ch)` でジョインし、異常フラグと閾値初期値を付けた
/// `calib.json` 相当の表を作る。
///
/// 両方に存在するチャンネルだけを出力する(片方にしかないチャンネルは stderr に警告して
/// スキップ — silent にしない。実運用では pedestal/pulser とも同じ全チャンネル full-readout
/// を通すので通常は起こらない)。
pub fn merge(pedestal: &PedestalTable, pulser: &PulserTable, k: f64) -> CalibTable {
    let pedestal_by_key: BTreeMap<ChKey, &PedestalChannel> = pedestal
        .channels
        .iter()
        .map(|c| ((c.asad, c.aget, c.ch), c))
        .collect();
    let pulser_by_key: BTreeMap<ChKey, &PulserChannel> = pulser
        .channels
        .iter()
        .map(|c| ((c.asad, c.aget, c.ch), c))
        .collect();
    let pedestal_keys: BTreeSet<ChKey> = pedestal_by_key.keys().copied().collect();
    let pulser_keys: BTreeSet<ChKey> = pulser_by_key.keys().copied().collect();

    for key in pulser_keys.difference(&pedestal_keys) {
        eprintln!(
            "calib: merge: channel {key:?} is in pulser.json but not pedestal.json — skipped"
        );
    }
    for key in pedestal_keys.difference(&pulser_keys) {
        eprintln!(
            "calib: merge: channel {key:?} is in pedestal.json but not pulser.json — skipped"
        );
    }

    let joined: Vec<(ChKey, &PedestalChannel, &PulserChannel)> = pedestal_keys
        .intersection(&pulser_keys)
        .map(|key| (*key, pedestal_by_key[key], pulser_by_key[key]))
        .collect();

    let noise_signal: Vec<f64> = joined
        .iter()
        .filter(|(_, pc, _)| !pc.is_fpn)
        .map(|(_, pc, _)| pc.rms)
        .collect();
    let noise_fpn: Vec<f64> = joined
        .iter()
        .filter(|(_, pc, _)| pc.is_fpn)
        .map(|(_, pc, _)| pc.rms)
        .collect();
    let gains: Vec<f64> = joined
        .iter()
        .filter_map(|(_, _, pu)| pu.relative_gain)
        .collect();

    let noise_signal_median = median(&noise_signal);
    let noise_signal_mad = mad(&noise_signal, noise_signal_median);
    let noise_fpn_median = median(&noise_fpn);
    let noise_fpn_mad = mad(&noise_fpn, noise_fpn_median);
    let gain_median = median(&gains);
    let gain_mad = mad(&gains, gain_median);

    let mut channels: Vec<CalibChannel> = joined
        .into_iter()
        .map(|(key, pc, pu)| {
            let (asad, aget, ch) = key;
            let mut flags: Vec<String> = Vec::new();

            let (noise_med, noise_mad) = if pc.is_fpn {
                (noise_fpn_median, noise_fpn_mad)
            } else {
                (noise_signal_median, noise_signal_mad)
            };
            if pc.rms > noise_med + k * noise_mad {
                flags.push("noisy".to_string());
            }
            if pc.rms < DEAD_RMS_THRESHOLD {
                flags.push("dead".to_string());
            }
            if let Some(g) = pu.relative_gain {
                if (g - 1.0).abs() > GAIN_OUT_MAD_K * gain_mad {
                    flags.push("gain_out".to_string());
                }
            }
            if pu.saturated > SATURATED_RATE_THRESHOLD {
                flags.push("saturated".to_string());
            }

            CalibChannel {
                asad,
                aget,
                ch,
                fpn: pc.is_fpn,
                baseline: pc.mean,
                noise: pc.rms,
                gain: pu.relative_gain,
                flags,
                threshold_suggest: (pc.mean + THRESHOLD_NOISE_SIGMA * pc.rms).round() as i64,
            }
        })
        .collect();
    channels.sort_by_key(|c| (c.asad, c.aget, c.ch));

    CalibTable {
        meta: MergeMeta {
            pedestal_inputs: pedestal.meta.inputs.clone(),
            pulser_inputs: pulser.meta.inputs.clone(),
            pedestal_events: pedestal.meta.events,
            pulser_events: pulser.meta.events,
            generated_at: now_rfc3339(),
            tool_version: tool_version(),
            k,
            noise_signal_median,
            noise_signal_mad,
            noise_fpn_median,
            noise_fpn_mad,
            gain_median,
            gain_mad,
            saturated_rate_threshold: SATURATED_RATE_THRESHOLD,
        },
        channels,
    }
}

pub fn write(table: &CalibTable, out: &Path) -> Result<(), CalibError> {
    super::write_json(table, out)
}

pub fn read(path: &Path) -> Result<CalibTable, CalibError> {
    super::read_json(path)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::calib::Meta;

    fn meta(events: u64) -> Meta {
        Meta {
            inputs: vec!["x.graw".to_string()],
            events,
            generated_at: "2026-09-17T00:00:00Z".to_string(),
            tool_version: "0.1.0".to_string(),
        }
    }

    fn pedestal_ch(
        asad: u8,
        aget: u8,
        ch: u8,
        is_fpn: bool,
        mean: f64,
        rms: f64,
    ) -> PedestalChannel {
        PedestalChannel {
            asad,
            aget,
            ch,
            mean,
            rms,
            runs: 1,
            is_fpn,
        }
    }

    fn pulser_ch(
        asad: u8,
        aget: u8,
        ch: u8,
        is_fpn: bool,
        amplitude_median: f64,
        relative_gain: Option<f64>,
        saturated: f64,
    ) -> PulserChannel {
        PulserChannel {
            asad,
            aget,
            ch,
            is_fpn,
            amplitude_median,
            relative_gain,
            saturated,
            runs: 1,
        }
    }

    /// **6 ch で flags の全分岐を踏む**(発注書「表 5 ch 程度の小さな JSON で分岐を全部踏む」)。
    /// 信号 5 ch(ch0–ch4)+ FPN 1 ch(ch11)。
    ///
    /// 手計算(k=5、母集団は 5 個なので中央値・MAD とも中央 1 値そのもの):
    /// - noise 母集団(信号 5 本、pedestal.rms): [6.0, 6.2, 5.8, 40.0, 0.1]。
    ///   昇順 [0.1, 5.8, 6.0, 6.2, 40.0] → 中央値(奇数個の真ん中)= 6.0。
    ///   偏差絶対値 = [5.9, 0.2, 0, 0.2, 34.0] → 昇順 [0, 0.2, 0.2, 5.9, 34.0] → MAD = 0.2。
    ///   noisy 閾値 = 6.0 + 5*0.2 = 7.0。超えるのは ch3(40.0)だけ → `noisy`。
    /// - ch4 は rms=0.1 < 0.5(絶対閾値)→ `dead`(noisy 閾値 7.0 は超えないので noisy は付かない)。
    /// - gain 母集団(信号 5 本、pulser.relative_gain): [1.00, 1.02, 1.30, 0.98, 1.00]。
    ///   昇順 [0.98, 1.00, 1.00, 1.02, 1.30] → 中央値 = 1.00。
    ///   偏差絶対値 = [0.02, 0, 0, 0.02, 0.30] → 昇順 [0, 0, 0.02, 0.02, 0.30] → MAD = 0.02。
    ///   gain_out 閾値 = 2*0.02 = 0.04。超えるのは ch2(|1.30-1.00|=0.30)だけ → `gain_out`。
    /// - FPN ch11 は noise 母集団が単独の 1 値(2.0)なので中央値=2.0・MAD=0
    ///   → noisy 閾値 2.0 を rms=2.0 は超えない(非 `>`)。gain は `None` なので `gain_out` 判定対象外。
    ///   saturated=0.99 > 0.5 → `saturated` のみ。
    /// - ch0/ch1 はどの分岐にも触れない(flags 空)。
    #[test]
    fn merge_flags_six_channels_hit_every_branch() {
        let pedestal = PedestalTable {
            meta: meta(100),
            channels: vec![
                pedestal_ch(0, 0, 0, false, 300.0, 6.0),  // clean
                pedestal_ch(0, 0, 1, false, 300.0, 6.2),  // clean(noise 側)
                pedestal_ch(0, 0, 2, false, 300.0, 5.8),  // gain_out(noise 側は正常)
                pedestal_ch(0, 0, 3, false, 300.0, 40.0), // noisy
                pedestal_ch(0, 0, 4, false, 300.0, 0.1),  // dead
                pedestal_ch(0, 1, 11, true, 400.0, 2.0),  // FPN, saturated
            ],
        };
        let pulser = PulserTable {
            meta: meta(76),
            channels: vec![
                pulser_ch(0, 0, 0, false, 3200.0, Some(1.00), 0.0),
                pulser_ch(0, 0, 1, false, 3264.0, Some(1.02), 0.0),
                pulser_ch(0, 0, 2, false, 4160.0, Some(1.30), 0.0), // gain_out
                pulser_ch(0, 0, 3, false, 3136.0, Some(0.98), 0.0),
                pulser_ch(0, 0, 4, false, 3200.0, Some(1.00), 0.0),
                pulser_ch(0, 1, 11, true, 3700.0, None, 0.99), // FPN 飽和
            ],
        };

        let calib = merge(&pedestal, &pulser, 5.0);
        assert_eq!(calib.channels.len(), 6);

        let flags_of = |asad: u8, aget: u8, ch: u8| -> Vec<String> {
            calib
                .channels
                .iter()
                .find(|c| (c.asad, c.aget, c.ch) == (asad, aget, ch))
                .unwrap_or_else(|| panic!("channel ({asad},{aget},{ch}) missing"))
                .flags
                .clone()
        };

        assert_eq!(flags_of(0, 0, 0), Vec::<String>::new(), "ch0: clean");
        assert_eq!(flags_of(0, 0, 1), Vec::<String>::new(), "ch1: clean");
        assert_eq!(
            flags_of(0, 0, 2),
            vec!["gain_out".to_string()],
            "ch2: gain_out"
        );
        assert_eq!(flags_of(0, 0, 3), vec!["noisy".to_string()], "ch3: noisy");
        assert_eq!(flags_of(0, 0, 4), vec!["dead".to_string()], "ch4: dead");
        assert_eq!(
            flags_of(0, 1, 11),
            vec!["saturated".to_string()],
            "ch11(FPN): saturated"
        );

        // meta に k と MAD の値が残っていること(発注書「機械基準を残す」)。
        assert_eq!(calib.meta.k, 5.0);
        assert!((calib.meta.noise_signal_median - 6.0).abs() < 1e-9);
        assert!((calib.meta.noise_signal_mad - 0.2).abs() < 1e-9);
        assert!((calib.meta.gain_median - 1.00).abs() < 1e-9);
        assert!((calib.meta.gain_mad - 0.02).abs() < 1e-9);
    }

    #[test]
    fn threshold_suggest_matches_hand_calculation() {
        // 手計算: baseline=300, noise=6.0 → round(300 + 5*6.0) = round(330.0) = 330。
        let pedestal = PedestalTable {
            meta: meta(1),
            channels: vec![pedestal_ch(0, 0, 0, false, 300.0, 6.0)],
        };
        let pulser = PulserTable {
            meta: meta(1),
            channels: vec![pulser_ch(0, 0, 0, false, 3200.0, Some(1.0), 0.0)],
        };
        let calib = merge(&pedestal, &pulser, 5.0);
        assert_eq!(calib.channels[0].threshold_suggest, 330);
    }

    #[test]
    fn channels_missing_on_one_side_are_skipped_not_panicking() {
        let pedestal = PedestalTable {
            meta: meta(1),
            channels: vec![
                pedestal_ch(0, 0, 0, false, 300.0, 6.0),
                pedestal_ch(0, 0, 1, false, 300.0, 6.0), // pulser に無い
            ],
        };
        let pulser = PulserTable {
            meta: meta(1),
            channels: vec![pulser_ch(0, 0, 0, false, 3200.0, Some(1.0), 0.0)],
        };
        let calib = merge(&pedestal, &pulser, 5.0);
        assert_eq!(calib.channels.len(), 1, "unmatched pedestal ch は落ちる");
        assert_eq!(
            (
                calib.channels[0].asad,
                calib.channels[0].aget,
                calib.channels[0].ch
            ),
            (0, 0, 0)
        );
    }
}
