//! 飛跡の再構成(TODO/127 — TODO/125 改訂 2 の UI 実装 `track-fit.ts` の移植先)。純関数、IO なし。
//!
//! ユーザーの解析マクロ `macros/analyzeUVW.C`(0〜4)と `macros/makeTracks.C`(1〜5)の移植。
//! **1 イベント = 直線 1 本**。`track-fit.ts` から **f64・同じ演算順・同じ定数**で移した
//! (端点のための並べ替えも `l` → `q` の安定ソート)。照合は env-gated の
//! `tests/reco_macro_oracle.rs`(本物の macros の出力)。
//!
//! macros からの一般化(125 改訂 2 のとおり。それ以外は macros と同じ式・同じ順):
//! - strip 位置 s は channel_map.csv(mini 専用)ではなく [`RecoGeometry`] = `stripGeometry` から
//!   `s = pitchUnit·(x0, y0)`(その strip の section 番号が最小の線分 — TPCReco Strip2posUVW と同じ)。
//!   pitch ベクトルも macros の PITCH 定数ではなく `pitchUnit`。
//! - makeTracks 2 の閉じた式(mini の pitch 専用)は `var_v = p_vᵀ S p_v` の 3×3 連立([`xy_covariance`])。
//! - 入力は 0x02 Uvw と同じ格子(section 合算済みの strip × bucket の生 ADC)なので、1 strip =
//!   analyzeUVW の「1 ch」。
//! - z [mm] は macros の既定値(1200 V / 20 cm / 100 mbar、25 MHz)で、帯選択と z フィットの中だけで使う。
//!   表示用の点の時刻は time bucket([`TrackPoint::t`])。
//! - makeTracks 6(頭尻の向き付け)は移植しない — 頭尻は決めない(macros の WARNING と同じ)。
//!
//! 作業バッファ(ヒット・点)は [`Reco`] が持って使い回す(イベントごとの heap 確保をしない —
//! `DisplayConverter` と同じ流儀)。

use std::fmt;

use crate::strip_xy::StripGeometry;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;

// ---- analyzeUVW.C の定数 --------------------------------------------------------------------
pub const ADC_THRESHOLD: f64 = 35.0;
/// ペデスタル = cell `PED_LO..=PED_HI` の平均。
pub const PED_LO: usize = 1;
pub const PED_HI: usize = 20;
pub const PEAK_HALF_CELLS: usize = 5;
pub const BAND_MM: f64 = 10.0;
/// analyzeUVW.C の MIN_VIEW_HITS(帯フィットに要る strip ヒット数)。
pub const MIN_VIEW_HITS_BAND: usize = 4;
pub const STRIP_Q_MIN: f64 = 2000.0;
const DRIFT_HV_V: f64 = 1200.0;
const DRIFT_LEN_CM: f64 = 20.0;
const PRESSURE_MBAR: f64 = 100.0;
const SAMPLING_MHZ: f64 = 25.0;
/// analyzeUVW.C `driftVelocityCmUs` の CO2 移動度 [(cm/µs) per (V/cm/mbar)]。
const MOBILITY: f64 = 0.7233;
/// 1 time bucket の z [mm](macros の既定値、式も同じ順)= 0.173592。
pub const MM_PER_CELL: f64 =
    ((MOBILITY * (DRIFT_HV_V / DRIFT_LEN_CM / PRESSURE_MBAR)) / SAMPLING_MHZ) * 10.0;

// ---- makeTracks.C の定数 --------------------------------------------------------------------
pub const QUANTILE: f64 = 0.02;
/// makeTracks.C の MIN_VIEW_HITS(各面に要る帯内の strip ヒット数)。
pub const MIN_VIEW_HITS_TRACK: usize = 3;
pub const MIN_EIGEN_GAP_MM2: f64 = 1.0;
pub const MIN_PROJ: f64 = 0.3;

/// 3×3 連立の行列式がこれ未満なら pitch が退化しているとみなす。
const MIN_PITCH_DET: f64 = 1e-9;

const VIEW_NAMES: [&str; 3] = ["U", "V", "W"];

pub type Vec2 = [f64; 2];

/// strip 1 本のヒット(analyzeUVW の slView / slZ / slS / slQ の 1 組 + strip 番号と時刻)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StripHit {
    /// 0=U, 1=V, 2=W
    pub view: u8,
    /// 1 始まり
    pub strip: u16,
    /// strip 位置 [mm](pitch 座標)
    pub s: f64,
    /// 電荷加重のピーク時刻 [time bucket]
    pub t: f64,
    /// ドリフト z [mm] = t × MM_PER_CELL
    pub z: f64,
    /// ピーク窓の積分電荷 [ADC]
    pub q: f64,
}

/// 表示用の点: 直線上の (x, y) と、そのヒットの時刻・電荷・面。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackPoint {
    pub x: f64,
    pub y: f64,
    /// [time bucket]
    pub t: f64,
    pub q: f64,
    /// 0=U, 1=V, 2=W
    pub view: u8,
}

/// 直線 1 本(makeTracks の tracks の 1 行に当たる)。z は mm。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Track {
    /// makeTracks の phi [度](頭尻を決めないので 180° の不定性がある)。
    pub phi_deg: f64,
    /// 単位ベクトル (dx, dy, dz)。符号は決めない。
    pub dir: [f64; 3],
    /// 2 % / 98 % の電荷分位点の位置 (x, y, z[mm])。順序は決めない(start = l が小さい側)。
    pub start: [f64; 3],
    pub end: [f64; 3],
    /// 最も外側のヒット(makeTracks の *F)。
    pub start_f: [f64; 3],
    pub end_f: [f64; 3],
    /// 3D 長さ [mm](2–98 %)と最も外側どうし。
    pub length: f64,
    pub length_f: f64,
    /// xy の長さ [mm](2–98 %)。ドリフト速度に依らない。
    pub xy_length: f64,
    /// 3 面の帯内ヒットの電荷の和(makeTracks の charge)。
    pub charge: f64,
}

/// 1 面の strip ヒットの不足(どこで足りなくなったか)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortage {
    Enough,
    /// halo cut の後で MIN_VIEW_HITS_BAND 本未満(analyzeUVW が帯フィットをしない)。
    AfterHaloCut(usize),
    /// 帯の中で MIN_VIEW_HITS_TRACK 本未満(makeTracks 1)。
    InBand(usize),
}

/// 直線が引けない理由。文([`fmt::Display`])は 125 の UI と同じ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoTrack {
    /// U, V, W の順。
    TooFewHits([Shortage; 3]),
    DegeneratePitch,
    NoElongation,
    NoSpread,
    /// bucket 数がペデスタル窓(cell PED_LO..=PED_HI)に足りない。
    ShortWindow(usize),
}

impl fmt::Display for NoTrack {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NoTrack::TooFewHits(views) => {
                let mut first = true;
                for (view, shortage) in views.iter().enumerate() {
                    let (n, place, needs) = match *shortage {
                        Shortage::Enough => continue,
                        Shortage::AfterHaloCut(n) => (n, "after the halo cut", MIN_VIEW_HITS_BAND),
                        Shortage::InBand(n) => (n, "in the band", MIN_VIEW_HITS_TRACK),
                    };
                    if !first {
                        f.write_str("; ")?;
                    }
                    first = false;
                    let noun = if n == 1 { "hit" } else { "hits" };
                    write!(
                        f,
                        "{} has {n} strip {noun} {place} (needs {needs})",
                        VIEW_NAMES[view]
                    )?;
                }
                Ok(())
            }
            NoTrack::DegeneratePitch => {
                f.write_str("U/V/W pitch vectors are degenerate (cannot solve the xy covariance)")
            }
            NoTrack::NoElongation => f.write_str("no xy elongation (track along the drift)"),
            NoTrack::NoSpread => {
                f.write_str("no spread along the track (no hits with a usable path length)")
            }
            NoTrack::ShortWindow(n_buckets) => write!(
                f,
                "{n_buckets} time buckets do not cover the pedestal window (cells {PED_LO}–{PED_HI})"
            ),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// ジオメトリ(起動時に 1 回)
// ---------------------------------------------------------------------------------------------

/// 再構成に要るジオメトリ: 面ごとの strip 位置 s と pitch 単位ベクトル。
#[derive(Debug, Clone, PartialEq)]
pub struct RecoGeometry {
    /// 面ごと(U, V, W)、index = strip − 1 → s [mm]。線分の無い strip は `None`。
    pub positions: [Vec<Option<f64>>; 3],
    /// 面ごとの `pitchUnit`。
    pub pitch: [Vec2; 3],
}

impl RecoGeometry {
    /// `stripGeometry` から作る。s = `pitchUnit·(x0, y0)`、同じ strip の section が複数あれば番号の最小。
    /// strip 番号 0 の線分は使わない(0x02 の格子に strip 0 は無い)。
    pub fn new(geometry: &StripGeometry) -> Self {
        let planes = [&geometry.planes.u, &geometry.planes.v, &geometry.planes.w];
        let positions = planes.map(|plane| {
            let [px, py] = plane.pitch_unit;
            let mut sections: Vec<Option<u8>> = Vec::new();
            let mut positions: Vec<Option<f64>> = Vec::new();
            for segment in &plane.segments {
                let Some(index) = (segment.strip as usize).checked_sub(1) else {
                    continue;
                };
                if positions.len() <= index {
                    positions.resize(index + 1, None);
                    sections.resize(index + 1, None);
                }
                if matches!(sections[index], Some(seen) if seen <= segment.section) {
                    continue;
                }
                sections[index] = Some(segment.section);
                positions[index] = Some(px * segment.x0 + py * segment.y0);
            }
            positions
        });
        Self {
            positions,
            pitch: planes.map(|plane| plane.pitch_unit),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// analyzeUVW.C 0: 1 strip 1 ヒット
// ---------------------------------------------------------------------------------------------

/// analyzeUVW.C 0 を 1 面に: ペデスタル = cell PED_LO..=PED_HI の平均、ピーク = 最初の最大、
/// ピーク − ペデスタル > ADC_THRESHOLD なら ±PEAK_HALF_CELLS の閾値超えのセルで q と電荷加重時刻、
/// q ≥ STRIP_Q_MIN だけ残す(halo cut)。strip 順に `out` へ足す。
///
/// `adc` は strip-major(`(strip-1)*n_buckets + bucket`、0x02 と同じ)。s が無い strip
/// (`positions` に線分が無い)のヒットは外して、その数を返す(呼び手が可視化する)。
/// `n_buckets <= PED_HI` なら何もしない(呼び手 [`Reco::run`] が理由を出す)。
pub fn strip_hits(
    view: u8,
    adc: &[u16],
    n_buckets: usize,
    positions: &[Option<f64>],
    out: &mut Vec<StripHit>,
) -> usize {
    if n_buckets <= PED_HI {
        return 0;
    }
    let last = n_buckets - 1;
    let mut no_segment = 0;
    for (index, wave) in adc.chunks_exact(n_buckets).enumerate() {
        let mut ped = 0.0;
        for &value in &wave[PED_LO..=PED_HI] {
            ped += f64::from(value);
        }
        ped /= (PED_HI - PED_LO + 1) as f64;
        let mut t_peak = 0;
        for t in 1..n_buckets {
            if wave[t] > wave[t_peak] {
                t_peak = t;
            }
        }
        if f64::from(wave[t_peak]) - ped <= ADC_THRESHOLD {
            continue;
        }
        let t_lo = t_peak.saturating_sub(PEAK_HALF_CELLS);
        let t_hi = (t_peak + PEAK_HALF_CELLS).min(last);
        let mut q = 0.0;
        let mut qt = 0.0;
        for (t, &value) in wave.iter().enumerate().take(t_hi + 1).skip(t_lo) {
            let val = f64::from(value) - ped;
            if val <= ADC_THRESHOLD {
                continue;
            }
            q += val;
            qt += val * t as f64;
        }
        if q < STRIP_Q_MIN {
            continue; // halo strip
        }
        let Some(s) = positions.get(index).copied().flatten() else {
            no_segment += 1;
            continue;
        };
        let t = qt / q;
        out.push(StripHit {
            view,
            strip: (index + 1) as u16,
            s,
            t,
            z: t * MM_PER_CELL,
            q,
        });
    }
    no_segment
}

// ---------------------------------------------------------------------------------------------
// analyzeUVW.C 1–4: 面ごとの電荷加重 PCA の直線と帯選択
// ---------------------------------------------------------------------------------------------

/// analyzeUVW.C 1–4。`out` を空にしてから、直線から ±BAND_MM の帯に入るヒットを入れる。
/// ヒットが MIN_VIEW_HITS_BAND 本未満の面は空のまま(帯フィットをしない)。
pub fn band_select(hits: &[StripHit], out: &mut Vec<StripHit>) {
    out.clear();
    if hits.len() < MIN_VIEW_HITS_BAND {
        return;
    }

    // 1. 電荷加重の PCA 直線
    let mut sw = 0.0;
    let mut mz = 0.0;
    let mut ms = 0.0;
    for h in hits {
        sw += h.q;
        mz += h.q * h.z;
        ms += h.q * h.s;
    }
    mz /= sw; // sw > 0: 全ヒット q >= STRIP_Q_MIN
    ms /= sw;
    let mut czz = 0.0;
    let mut css = 0.0;
    let mut czs = 0.0;
    for h in hits {
        let dz = h.z - mz;
        let ds = h.s - ms;
        czz += h.q * dz * dz;
        css += h.q * ds * ds;
        czs += h.q * dz * ds;
    }
    // 2×2 共分散の主軸(最大固有値)
    let tr = czz + css;
    let det = czz * css - czs * czs;
    let lam = 0.5 * tr + (0.25 * tr * tr - det).max(0.0).sqrt();
    let mut dz0 = czs;
    let mut ds0 = lam - czz;
    let mut nn = dz0.hypot(ds0);
    if nn < 1e-9 {
        dz0 = 1.0;
        ds0 = 0.0;
        nn = 1.0;
    }
    dz0 /= nn;
    ds0 /= nn;

    // 2.+3.+4. 直線から ±BAND_MM の帯に入るヒットを残す
    for h in hits {
        let perp = -(h.z - mz) * ds0 + (h.s - ms) * dz0;
        if perp.abs() > BAND_MM {
            continue;
        }
        out.push(*h);
    }
}

// ---------------------------------------------------------------------------------------------
// makeTracks.C 1–5
// ---------------------------------------------------------------------------------------------

fn det3(m: &[[f64; 3]; 3]) -> f64 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}

/// makeTracks.C 2 の一般化: 3 面の分散 `var_v = p_vᵀ S p_v` から xy の 2×2 共分散 S を解く
/// (行 = [px², 2·px·py, py²]、未知数 = [Sxx, Sxy, Syy]、Cramer)。pitch が退化していれば `None`。
pub fn xy_covariance(variances: [f64; 3], pitch: [Vec2; 3]) -> Option<[f64; 3]> {
    let rows = pitch.map(|[px, py]| [px * px, 2.0 * px * py, py * py]);
    let det = det3(&rows);
    if det.abs().is_nan() || det.abs() < MIN_PITCH_DET {
        return None;
    }
    let solve = |col: usize| {
        let mut m = rows;
        for (row, &variance) in m.iter_mut().zip(variances.iter()) {
            row[col] = variance;
        }
        det3(&m) / det
    };
    Some([solve(0), solve(1), solve(2)])
}

/// makeTracks.C 1–5(帯選択を通ったヒット → 直線 1 本)。`kept` は U, V, W の順。
/// 頭尻は決めない(6 は移植しない): start = l が小さい側。
///
/// `lq` は作業バッファ(経路長 l と電荷 q)、`points` には表示用の点
/// (|pitch_v·d| ≥ MIN_PROJ の面のヒットを直線上に置いたもの、面 → ヒットの順)が入る。
/// どちらも最初に空にする。Err のとき `points` の中身は意味を持たない。
pub fn fit_track(
    kept: [&[StripHit]; 3],
    pitch: [Vec2; 3],
    lq: &mut Vec<(f64, f64)>,
    points: &mut Vec<TrackPoint>,
) -> Result<Track, NoTrack> {
    lq.clear();
    points.clear();

    // 1. 面ごとの電荷加重の平均と分散
    let mut nv = [0usize; 3];
    let mut qv = [0.0; 3];
    let mut ms = [0.0; 3];
    for v in 0..3 {
        for h in kept[v] {
            nv[v] += 1;
            qv[v] += h.q;
            ms[v] += h.q * h.s;
        }
    }
    if nv.iter().any(|&n| n < MIN_VIEW_HITS_TRACK) {
        return Err(NoTrack::TooFewHits(nv.map(|n| {
            if n < MIN_VIEW_HITS_TRACK {
                Shortage::InBand(n)
            } else {
                Shortage::Enough
            }
        })));
    }
    for v in 0..3 {
        ms[v] /= qv[v];
    }
    let mut variance = [0.0; 3];
    for v in 0..3 {
        for h in kept[v] {
            let ds = h.s - ms[v];
            variance[v] += h.q * ds * ds;
        }
        variance[v] /= qv[v];
    }

    // 2. 3 面の射影から xy の共分散 → 主軸 = d_xy
    let Some([sxx, sxy, syy]) = xy_covariance(variance, pitch) else {
        return Err(NoTrack::DegeneratePitch);
    };
    let gap = (sxx - syy).hypot(2.0 * sxy); // lam_max - lam_min
    if gap < MIN_EIGEN_GAP_MM2 {
        return Err(NoTrack::NoElongation);
    }
    let phi = 0.5 * (2.0 * sxy).atan2(sxx - syy);
    let ux = phi.cos();
    let uy = phi.sin();

    // 3. anchor (x0, y0) = pitch_v·(x0, y0) = ms_v の最小二乗。Q = 3 面の電荷
    let mut a00 = 0.0;
    let mut a01 = 0.0;
    let mut a11 = 0.0;
    let mut b0 = 0.0;
    let mut b1 = 0.0;
    let mut charge = 0.0;
    for v in 0..3 {
        let [px, py] = pitch[v];
        a00 += px * px;
        a01 += px * py;
        a11 += py * py;
        b0 += px * ms[v];
        b1 += py * ms[v];
        charge += qv[v];
    }
    let det = a00 * a11 - a01 * a01;
    let mx = (a11 * b0 - a01 * b1) / det;
    let my = (-a01 * b0 + a00 * b1) / det;

    // 4. 各ヒットの xy 経路長 l(|pitch_v·d| >= MIN_PROJ の面だけ)と z(l) の電荷加重直線フィット
    let proj = pitch.map(|[px, py]| px * ux + py * uy);
    let mut w = 0.0;
    let mut sl = 0.0;
    let mut sz = 0.0;
    let mut sll = 0.0;
    let mut slz = 0.0;
    for v in 0..3 {
        if proj[v].abs() < MIN_PROJ {
            continue;
        }
        for h in kept[v] {
            let l = (h.s - ms[v]) / proj[v];
            lq.push((l, h.q));
            points.push(TrackPoint {
                x: mx + l * ux,
                y: my + l * uy,
                t: h.t,
                q: h.q,
                view: v as u8,
            });
            w += h.q;
            sl += h.q * l;
            sz += h.q * h.z;
            sll += h.q * l * l;
            slz += h.q * l * h.z;
        }
    }
    let var_l = if w > 0.0 {
        sll / w - (sl / w) * (sl / w)
    } else {
        0.0
    };
    if var_l.is_nan() || var_l <= 0.0 {
        return Err(NoTrack::NoSpread);
    }
    let tan_th = (slz / w - (sl / w) * (sz / w)) / var_l;
    let z0 = sz / w - (tan_th * sl) / w; // l = 0 での切片
    let nrm = (1.0 + tan_th * tan_th).sqrt();

    // 5. 端点 = l の電荷分位点(QUANTILE .. 1 − QUANTILE)と最も外側のヒット。
    // 並べ替えは TS の `a.l - b.l || a.q - b.q`(安定ソート)と同じ順: 有限値では符号が一致し、
    // ±0 は `+ 0.0` で同一視する。total_cmp なので全順序(NaN が来ても panic しない)。
    lq.sort_by(|a, b| {
        (a.0 + 0.0)
            .total_cmp(&(b.0 + 0.0))
            .then((a.1 + 0.0).total_cmp(&(b.1 + 0.0)))
    });
    let (Some(&(lo_f, _)), Some(&(hi_f, _))) = (lq.first(), lq.last()) else {
        return Err(NoTrack::NoSpread); // var_l > 0 なら空ではない
    };
    let mut total = 0.0;
    for &(_, q) in lq.iter() {
        total += q;
    }
    let mut lo = lo_f;
    let mut hi = hi_f;
    let mut cum = 0.0;
    for &(l, q) in lq.iter() {
        cum += q;
        if cum / total <= QUANTILE {
            lo = l;
        }
        if cum / total <= 1.0 - QUANTILE {
            hi = l;
        }
    }
    let at = |l: f64| [mx + l * ux, my + l * uy, z0 + l * tan_th];

    Ok(Track {
        phi_deg: (phi * 180.0) / std::f64::consts::PI,
        dir: [ux / nrm, uy / nrm, tan_th / nrm],
        start: at(lo),
        end: at(hi),
        start_f: at(lo_f),
        end_f: at(hi_f),
        length: (hi - lo) * nrm,
        length_f: (hi_f - lo_f) * nrm,
        xy_length: hi - lo,
        charge,
    })
}

/// makeTracks 1 の「各面 MIN_VIEW_HITS_TRACK 本」を、どこで足りなくなったかにする。
/// halo cut の後で MIN_VIEW_HITS_BAND 本未満なら analyzeUVW が帯フィットをせず(その面は 0 本)、
/// そうでなければ帯の中の本数で判定する — macros と同じ判定(ok ⇔ 全面で帯内 ≥ 3 本)。
fn hit_shortage(halo_hits: [usize; 3], selected: [usize; 3]) -> Option<NoTrack> {
    let mut views = [Shortage::Enough; 3];
    let mut short = false;
    for v in 0..3 {
        if halo_hits[v] < MIN_VIEW_HITS_BAND {
            views[v] = Shortage::AfterHaloCut(halo_hits[v]);
            short = true;
        } else if selected[v] < MIN_VIEW_HITS_TRACK {
            views[v] = Shortage::InBand(selected[v]);
            short = true;
        }
    }
    short.then_some(NoTrack::TooFewHits(views))
}

// ---------------------------------------------------------------------------------------------
// 1 イベント(3 面の格子)→ 直線 1 本 + 表示用の点
// ---------------------------------------------------------------------------------------------

/// 1 イベントの結果(0x04 Track の中身)。[`Reco::event`] が作業バッファを借りて返す。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackEvent<'a> {
    /// 面ごとの帯内ヒット数(analyzeUVW の出力 = makeTracks の入力)。U, V, W の順。
    pub hits: [usize; 3],
    /// 面ごとの strip ヒット数(halo cut の後 = analyzeUVW 0 の出力)。
    pub halo_hits: [usize; 3],
    /// 直線上に置けなかった(|pitch_v·d| < MIN_PROJ の面の)帯内ヒット数。ok でなければ 0。
    pub unplaced: usize,
    /// ヒットがあったのに `stripGeometry` に線分の無い strip の数(使えないので外した)。
    pub no_segment: usize,
    pub fit: Result<Track, NoTrack>,
    /// 表示用の点(ok でなければ空)。
    pub points: &'a [TrackPoint],
}

/// 1 イベントの再構成器。ジオメトリと作業バッファを持ち、イベントごとに使い回す。
#[derive(Debug, Clone)]
pub struct Reco {
    geometry: RecoGeometry,
    /// 1 面ぶんの halo cut 後のヒット(面ごとに使い回す)。
    found: Vec<StripHit>,
    /// 面ごとの帯内ヒット。
    selected: [Vec<StripHit>; 3],
    lq: Vec<(f64, f64)>,
    points: Vec<TrackPoint>,
    halo_hits: [usize; 3],
    no_segment: usize,
    fit: Result<Track, NoTrack>,
}

impl Reco {
    pub fn new(geometry: RecoGeometry) -> Self {
        Self {
            geometry,
            found: Vec::new(),
            selected: [Vec::new(), Vec::new(), Vec::new()],
            lq: Vec::new(),
            points: Vec::new(),
            halo_hits: [0; 3],
            no_segment: 0,
            fit: Err(NoTrack::TooFewHits([Shortage::AfterHaloCut(0); 3])),
        }
    }

    pub fn geometry(&self) -> &RecoGeometry {
        &self.geometry
    }

    /// 1 イベントを再構成する(analyzeUVW 0–4 + makeTracks 1–5)。結果は [`Reco::event`]。
    ///
    /// `grids` は U, V, W の 0x02 と同じ格子(strip-major の `nStrips × n_buckets`、section 合算済み)。
    pub fn run(&mut self, grids: [&[u16]; 3], n_buckets: usize) {
        for selected in &mut self.selected {
            selected.clear();
        }
        self.points.clear();
        self.halo_hits = [0; 3];
        self.no_segment = 0;
        if n_buckets <= PED_HI {
            self.fit = Err(NoTrack::ShortWindow(n_buckets));
            return;
        }
        for (v, grid) in grids.iter().enumerate() {
            self.found.clear();
            self.no_segment += strip_hits(
                v as u8,
                grid,
                n_buckets,
                &self.geometry.positions[v],
                &mut self.found,
            );
            self.halo_hits[v] = self.found.len();
            band_select(&self.found, &mut self.selected[v]);
        }
        let selected = self.selected.each_ref().map(Vec::len);
        self.fit = match hit_shortage(self.halo_hits, selected) {
            Some(reason) => Err(reason),
            None => fit_track(
                self.selected.each_ref().map(Vec::as_slice),
                self.geometry.pitch,
                &mut self.lq,
                &mut self.points,
            ),
        };
        if self.fit.is_err() {
            self.points.clear();
        }
    }

    /// 直近の [`Reco::run`] の結果。
    pub fn event(&self) -> TrackEvent<'_> {
        let hits = self.selected.each_ref().map(Vec::len);
        let unplaced = if self.fit.is_ok() {
            hits.iter().sum::<usize>().saturating_sub(self.points.len())
        } else {
            0
        };
        TrackEvent {
            hits,
            halo_hits: self.halo_hits,
            unplaced,
            no_segment: self.no_segment,
            fit: self.fit,
            points: &self.points,
        }
    }

    /// 直近の [`Reco::run`] の帯内ヒット(U, V, W の順)。
    pub fn selected(&self) -> [&[StripHit]; 3] {
        self.selected.each_ref().map(Vec::as_slice)
    }
}
