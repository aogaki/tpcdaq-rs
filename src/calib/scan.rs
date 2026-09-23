//! パルサー振幅スキャン解析(TODO/091 §1)。ch 毎の応答直線性(振幅 vs ADC 波高)と
//! 平均波形を、振幅ごとに 1 run(.graw ファイル)ずつ処理して集計する。
//!
//! 振幅ラダーの値(peak − baseline)は `calib::pulser` と同じ定義を再利用する
//! (`pulser::BASELINE_END`)。1 パスの中で平均波形も一緒に計算する(ファイルを 2 回
//! 読まない — pedestal/pulser の docstring が明記する「定義は踏襲するが転記ではなく
//! 再実装」の方針をここでも踏襲する)。
//!
//! # 「振幅 ≤ −700 mV」の解釈(発注書に明記が無いので記録する — 要レビュー)
//!
//! 発注書(TODO/091 §1)は「直線性: 振幅 ≤ −700 mV(圧縮前)の点で最小二乗」と書くが、
//! 実測振幅はすべて負(−100〜−1000 mV)なので文字どおりの実数比較では
//! `{−700, −1000}`(圧縮側の 2 点)しか残らず、「圧縮前」という意図と逆になる
//! (069/091 の実測ノート: 比 −300/−100=2.91・−1000/−100=8.78 で圧縮は −1000 に
//! 集中している)。ここでは**振幅の絶対値が 700 mV 以下**(`amp.abs() <= 700.0`、
//! 実測では −100/−200/−300/−500/−700 の 5 点)を直線フィットに使い、−1000 は
//! 残差検証のみに回す解釈を採用した。
//!
//! 圧縮そのもののモデル化(飽和曲線フィットなど)はしない — 発注書が明示的に除外している
//! (「設計判断が残る分岐(例: 圧縮のモデル化)は実装せず報告」)。線形フィット + 残差報告
//! だけを行う。

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::pulser::BASELINE_END;
use super::{
    is_fpn, median, now_rfc3339, read_json, scan_files, tool_version, write_json, CalibError,
    ChKey, Meta, ScanStats, AGET_PER_ASAD, RAW_CH_PER_AGET,
};
use crate::msg::{items_from_bytes, unpack_item};

/// 波形の全バケット数。GET item の bucket は 9 bit = 0..511(512 点。`msg::ITEM_BUCKET_MAX`
/// と同じ幅)。
pub const WAVEFORM_LEN: usize = 512;
/// 平均波形のペデスタル窓(発注書: 「先頭 25 セル平均」)。振幅ラダーが使う
/// `pulser::BASELINE_END`(64)とは別の窓 — 発注書が明示的に別の値を指定しているため、
/// そのまま踏襲する(値を揃えない)。
pub const WAVEFORM_BASELINE_CELLS: usize = 25;
/// 直線フィットに使う振幅の絶対値の上限 [mV](モジュール doc の解釈を参照)。
pub const FIT_MAX_ABS_AMP_MV: f64 = 700.0;

/// 1 フレーム(= 1 AsAd 分)の per-(aget,ch) スクラッチ領域サイズ。
const PER_FRAME: usize = AGET_PER_ASAD * RAW_CH_PER_AGET;

/// ch 毎の(振幅中央値, 平均波形 512 点)。
type ChannelLadder = (f64, Vec<f64>);

/// 1 run(= 1 振幅点)の入力。
#[derive(Debug, Clone)]
pub struct ScanRun {
    pub amp_mv: f64,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScanFit {
    pub slope: f64,
    pub intercept: f64,
    pub max_resid_frac: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScanChannel {
    pub asad: u8,
    pub aget: u8,
    pub ch: u8,
    pub is_fpn: bool,
    /// `ScanTable::amps` と同じ順序・長さ。各要素は当該 run 内での振幅中央値
    /// (peak − baseline、生 ADC。`pulser::compute` と同じ定義)。
    pub amplitude: Vec<f64>,
    pub fit: ScanFit,
    /// `ScanTable::amps` と同じ順序。各要素は 512 点(ペデスタル差引後、イベント平均)。
    pub waveform: Vec<Vec<f64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanTable {
    pub meta: Meta,
    pub amps: Vec<f64>,
    pub channels: Vec<ScanChannel>,
}

// ---------------------------------------------------------------------
// 1 run 分の走査
// ---------------------------------------------------------------------

struct RunAccum {
    amps: Vec<f64>,
    wave_sum: Vec<f64>,
    wave_events: u32,
}

impl RunAccum {
    fn new() -> Self {
        Self {
            amps: Vec::new(),
            wave_sum: vec![0.0; WAVEFORM_LEN],
            wave_events: 0,
        }
    }
}

/// 1 run(1 振幅、1 .graw ファイル)を走査し、ch 毎の(振幅中央値, 平均波形)を返す。
fn compute_one_run(path: &Path) -> Result<(BTreeMap<ChKey, ChannelLadder>, ScanStats), CalibError> {
    let mut accum: BTreeMap<ChKey, RunAccum> = BTreeMap::new();

    // per-frame スクラッチ(ホットパスで per-frame 確保しない — CLAUDE.md)。
    let mut wave_frame = vec![0u16; PER_FRAME * WAVEFORM_LEN];
    let mut seen = vec![false; PER_FRAME];
    let mut n_baseline64 = vec![0u32; PER_FRAME];
    let mut n_baseline25 = vec![0u32; PER_FRAME];

    let stats = scan_files(&[path.to_path_buf()], |_file_idx, frag| {
        let asad = frag.asad;
        let items = match items_from_bytes(&frag.items) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("calib: scan: items_from_bytes failed for a decoded frame: {e}");
                return;
            }
        };

        wave_frame.iter_mut().for_each(|v| *v = 0);
        seen.iter_mut().for_each(|v| *v = false);
        n_baseline64.iter_mut().for_each(|v| *v = 0);
        n_baseline25.iter_mut().for_each(|v| *v = 0);

        for word in items {
            let it = unpack_item(word);
            let k = it.aget as usize * RAW_CH_PER_AGET + it.chan as usize;
            let Some(seen_slot) = seen.get_mut(k) else {
                // aget/chan がハードウェア範囲(0-3 / 0-67)を超える(理論上到達しないはずの
                // プロトコル不整合)。silent に握りつぶさず報告する(CLAUDE.md)。
                eprintln!(
                    "calib: scan: item aget={} chan={} out of the AGET/AsAd hardware range — skipped",
                    it.aget, it.chan
                );
                continue;
            };
            *seen_slot = true;
            // bucket は 9 bit(msg::ITEM_BUCKET_MAX=511)なので WAVEFORM_LEN(512) に必ず収まる。
            wave_frame[k * WAVEFORM_LEN + it.bucket as usize] = it.adc;
            if it.bucket < BASELINE_END {
                n_baseline64[k] += 1;
            }
            if (it.bucket as usize) < WAVEFORM_BASELINE_CELLS {
                n_baseline25[k] += 1;
            }
        }

        for k in 0..PER_FRAME {
            if !seen[k] || n_baseline64[k] == 0 {
                continue;
            }
            let wf = &wave_frame[k * WAVEFORM_LEN..(k + 1) * WAVEFORM_LEN];
            let baseline64: f64 = wf[..BASELINE_END as usize]
                .iter()
                .map(|&v| f64::from(v))
                .sum::<f64>()
                / f64::from(BASELINE_END);
            let peak = wf.iter().copied().max().unwrap_or(0);
            let amp = f64::from(peak) - baseline64;

            let aget = (k / RAW_CH_PER_AGET) as u8;
            let ch = (k % RAW_CH_PER_AGET) as u8;
            let key = (asad, aget, ch);
            let a = accum.entry(key).or_insert_with(RunAccum::new);
            a.amps.push(amp);

            if n_baseline25[k] == 0 {
                eprintln!(
                    "calib: scan: channel {key:?} has no bucket<{WAVEFORM_BASELINE_CELLS} sample this frame — waveform skipped for this event"
                );
                continue;
            }
            let pedestal25: f64 = wf[..WAVEFORM_BASELINE_CELLS]
                .iter()
                .map(|&v| f64::from(v))
                .sum::<f64>()
                / WAVEFORM_BASELINE_CELLS as f64;
            for (i, &v) in wf.iter().enumerate() {
                a.wave_sum[i] += f64::from(v) - pedestal25;
            }
            a.wave_events += 1;
        }
    })?;

    let out = accum
        .into_iter()
        .map(|(key, a)| {
            let amp_med = median(&a.amps);
            let wave_avg = if a.wave_events == 0 {
                vec![f64::NAN; WAVEFORM_LEN]
            } else {
                a.wave_sum
                    .iter()
                    .map(|s| s / f64::from(a.wave_events))
                    .collect()
            };
            (key, (amp_med, wave_avg))
        })
        .collect();
    Ok((out, stats))
}

// ---------------------------------------------------------------------
// 直線フィット(振幅ラダー → slope/intercept/残差)
// ---------------------------------------------------------------------

/// 最小二乗(1 次)。2 点未満または x が全て同じ値なら `(NaN, NaN)`(silent にせず警告)。
fn least_squares(xs: &[f64], ys: &[f64]) -> (f64, f64) {
    if xs.len() < 2 {
        eprintln!("calib: scan: fewer than 2 points within the linearity window — fit is NaN");
        return (f64::NAN, f64::NAN);
    }
    let n = xs.len() as f64;
    let sum_x: f64 = xs.iter().sum();
    let sum_y: f64 = ys.iter().sum();
    let sum_xy: f64 = xs.iter().zip(ys).map(|(x, y)| x * y).sum();
    let sum_x2: f64 = xs.iter().map(|x| x * x).sum();
    let denom = n * sum_x2 - sum_x * sum_x;
    if denom.abs() < 1e-12 {
        eprintln!("calib: scan: degenerate x values (all equal) — fit is NaN");
        return (f64::NAN, f64::NAN);
    }
    let slope = (n * sum_xy - sum_x * sum_y) / denom;
    let intercept = (sum_y - slope * sum_x) / n;
    (slope, intercept)
}

/// `amps`(振幅、mV)と `amplitude`(その ch の振幅中央値ラダー、同じ順序)から
/// [`ScanFit`] を作る。フィットは `|amp| <= FIT_MAX_ABS_AMP_MV` の点だけを使い、
/// 残差(fraction of fit)は全点で評価する(モジュール doc 参照)。
fn compute_fit(amps: &[f64], amplitude: &[f64]) -> ScanFit {
    let (fit_xs, fit_ys): (Vec<f64>, Vec<f64>) = amps
        .iter()
        .zip(amplitude)
        .filter(|(x, _)| x.abs() <= FIT_MAX_ABS_AMP_MV)
        .map(|(&x, &y)| (x, y))
        .unzip();
    let (slope, intercept) = least_squares(&fit_xs, &fit_ys);

    let max_resid_frac = amps
        .iter()
        .zip(amplitude)
        .filter_map(|(&x, &y)| {
            let pred = slope * x + intercept;
            if !pred.is_finite() || pred.abs() < 1e-9 {
                None
            } else {
                Some(((y - pred) / pred).abs())
            }
        })
        .fold(0.0f64, f64::max);

    ScanFit {
        slope,
        intercept,
        max_resid_frac,
    }
}

// ---------------------------------------------------------------------
// 複数 run の統合
// ---------------------------------------------------------------------

/// `runs`(振幅ごとに 1 run)を走査し、ch 毎の振幅ラダー・フィット・平均波形をまとめる。
pub fn compute(runs: &[ScanRun]) -> Result<ScanTable, CalibError> {
    if runs.is_empty() {
        return Err(CalibError::NoInputFiles);
    }
    let amps: Vec<f64> = runs.iter().map(|r| r.amp_mv).collect();
    let n_runs = runs.len();

    let mut per_channel: BTreeMap<ChKey, (Vec<f64>, Vec<Vec<f64>>)> = BTreeMap::new();
    let mut inputs = Vec::with_capacity(n_runs);
    let mut total_events = 0u64;

    for run in runs {
        let (by_ch, stats) = compute_one_run(&run.path)?;
        inputs.push(run.path.display().to_string());
        total_events += stats.events;
        if stats.malformed > 0 {
            eprintln!(
                "calib: scan: {}: {} malformed frame(s) skipped",
                run.path.display(),
                stats.malformed
            );
        }
        for (key, (amp_med, wave_avg)) in by_ch {
            let entry = per_channel.entry(key).or_default();
            entry.0.push(amp_med);
            entry.1.push(wave_avg);
        }
    }

    let mut channels = Vec::new();
    for (key, (amp_list, wave_list)) in per_channel {
        if amp_list.len() != n_runs {
            // full-readout 運用では全 run に全 ch が出るはず。silent に握りつぶさない。
            eprintln!(
                "calib: scan: channel {key:?} present in only {}/{n_runs} runs — skipped",
                amp_list.len()
            );
            continue;
        }
        let (asad, aget, ch) = key;
        let fit = compute_fit(&amps, &amp_list);
        channels.push(ScanChannel {
            asad,
            aget,
            ch,
            is_fpn: is_fpn(ch),
            amplitude: amp_list,
            fit,
            waveform: wave_list,
        });
    }

    Ok(ScanTable {
        meta: Meta {
            inputs,
            events: total_events,
            generated_at: now_rfc3339(),
            tool_version: tool_version(),
        },
        amps,
        channels,
    })
}

pub fn write(table: &ScanTable, out: &Path) -> Result<(), CalibError> {
    write_json(table, out)
}

pub fn read(path: &Path) -> Result<ScanTable, CalibError> {
    read_json(path)
}

// ---------------------------------------------------------------------
// CSV 出力(発注書「JSON は nlohmann を使わず calib scan --csv で CSV も出す方が KISS」)。
//
// ROOT マクロ(macros/pulserScan.C)が JSON パーサ無しで読めるよう、2 本の平坦な CSV に
// 分ける: channels(ch 毎 1 行、振幅ラダー + フィット結果)と waveform(long format、
// 512 点 × 振幅 × ch)。
// ---------------------------------------------------------------------

fn io_err(out: &Path, source: std::io::Error) -> CalibError {
    CalibError::Write {
        path: out.to_path_buf(),
        source,
    }
}

/// 1 行 = 1 ch。列: `asad,aget,ch,is_fpn,amp_<v0>,...,amp_<vN-1>,fit_slope,fit_intercept,fit_max_resid_frac`。
pub fn write_channels_csv(table: &ScanTable, out: &Path) -> Result<(), CalibError> {
    let file = std::fs::File::create(out).map_err(|e| io_err(out, e))?;
    let mut w = std::io::BufWriter::new(file);

    write!(w, "asad,aget,ch,is_fpn").map_err(|e| io_err(out, e))?;
    for amp in &table.amps {
        write!(w, ",amp_{amp}").map_err(|e| io_err(out, e))?;
    }
    writeln!(w, ",fit_slope,fit_intercept,fit_max_resid_frac").map_err(|e| io_err(out, e))?;

    for c in &table.channels {
        write!(w, "{},{},{},{}", c.asad, c.aget, c.ch, u8::from(c.is_fpn))
            .map_err(|e| io_err(out, e))?;
        for v in &c.amplitude {
            write!(w, ",{v}").map_err(|e| io_err(out, e))?;
        }
        writeln!(
            w,
            ",{},{},{}",
            c.fit.slope, c.fit.intercept, c.fit.max_resid_frac
        )
        .map_err(|e| io_err(out, e))?;
    }
    w.flush().map_err(|e| io_err(out, e))
}

/// long format: `asad,aget,ch,is_fpn,amp_mv,bucket,adc_avg`(1 行 = 1 (ch,振幅,bucket))。
pub fn write_waveform_csv(table: &ScanTable, out: &Path) -> Result<(), CalibError> {
    let file = std::fs::File::create(out).map_err(|e| io_err(out, e))?;
    let mut w = std::io::BufWriter::new(file);

    writeln!(w, "asad,aget,ch,is_fpn,amp_mv,bucket,adc_avg").map_err(|e| io_err(out, e))?;
    for c in &table.channels {
        for (amp, wave) in table.amps.iter().zip(&c.waveform) {
            for (bucket, v) in wave.iter().enumerate() {
                writeln!(
                    w,
                    "{},{},{},{},{},{},{}",
                    c.asad,
                    c.aget,
                    c.ch,
                    u8::from(c.is_fpn),
                    amp,
                    bucket,
                    v
                )
                .map_err(|e| io_err(out, e))?;
            }
        }
    }
    w.flush().map_err(|e| io_err(out, e))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn least_squares_matches_hand_calculation_for_two_points() {
        // 手計算: (x,y) = (-100,100),(-300,300)。slope=(300-100)/(-300+100)=-1.0、
        // intercept=100-(-1*-100)=0。
        let (slope, intercept) = least_squares(&[-100.0, -300.0], &[100.0, 300.0]);
        assert!((slope - (-1.0)).abs() < 1e-9, "slope={slope}");
        assert!((intercept - 0.0).abs() < 1e-9, "intercept={intercept}");
    }

    #[test]
    fn least_squares_is_nan_with_fewer_than_two_points() {
        let (slope, intercept) = least_squares(&[-100.0], &[100.0]);
        assert!(slope.is_nan());
        assert!(intercept.is_nan());
    }

    #[test]
    fn compute_fit_excludes_the_1000mv_point_and_reports_its_residual() {
        // 手計算(モジュール doc の例と同一): xs=[-100,-300,-1000], ys=[100,300,700]。
        // フィットは |x|<=700 の 2 点のみ → slope=-1, intercept=0。
        // -1000 での予測=1000、実測=700 → 残差=(700-1000)/1000=-0.3 → |resid|=0.3。
        let fit = compute_fit(&[-100.0, -300.0, -1000.0], &[100.0, 300.0, 700.0]);
        assert!((fit.slope - (-1.0)).abs() < 1e-9);
        assert!((fit.intercept - 0.0).abs() < 1e-9);
        assert!(
            (fit.max_resid_frac - 0.3).abs() < 1e-9,
            "{}",
            fit.max_resid_frac
        );
    }
}
