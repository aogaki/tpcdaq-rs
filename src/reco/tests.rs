//! TODO/127 — `ui/src/app/monitor/track-fit.spec.ts`(125 改訂 2 レーン D、39 本)の Rust 移植。
//!
//! 値は手計算(各ケースのコメント)。合成ジオメトリ・合成格子は下の「合成フィクスチャ」
//! (`track-test-fixtures.ts` の移植)。実データの照合は env-gated の `tests/reco_macro_oracle.rs`。
//!
//! TS の `toBeCloseTo(x, d)` は |差| < 10^-d / 2 なので、ここでは `close(a, b, d)` で同じ判定をする。

use super::*;
use crate::strip_xy::{PlaneSegments, Planes, Segment, StripGeometry};

use std::f64::consts::PI;

/// TS の `toBeCloseTo(expected, digits)` と同じ判定。
fn close(actual: f64, expected: f64, digits: i32) -> bool {
    (actual - expected).abs() < 10f64.powi(-digits) / 2.0
}

#[track_caller]
fn assert_close(actual: f64, expected: f64, digits: i32) {
    assert!(
        close(actual, expected, digits),
        "{actual} is not within 10^-{digits}/2 of {expected}"
    );
}

/// mini の pitch(makeTracks.C の PITCH を √3/2 の厳密値で)。
fn mini_pitch() -> [Vec2; 3] {
    let sqrt3_2 = 3.0_f64.sqrt() / 2.0;
    [[-1.0, 0.0], [0.5, sqrt3_2], [0.5, -sqrt3_2]]
}

/// fit_track の入力を手で作る(t は z から逆算)。
fn hit(view: u8, s: f64, z: f64, q: f64) -> StripHit {
    StripHit {
        view,
        strip: 0,
        s,
        t: z / MM_PER_CELL,
        z,
        q,
    }
}

// =============================================================================================
// 合成フィクスチャ(`track-test-fixtures.ts` の移植 — テスト専用)
//
// ジオメトリ: 面 p の strip 方向 = (cos θ_p, sin θ_p)、pitch 方向は TPCReco `GeometryTPC::Load` の式
// (pitch_U = -(W+V).Unit()、pitch_V = (U+W).Unit()、pitch_W = (V-U).Unit())。strip k の中心線は
// pitch 座標 s_k = (k - (n+1)/2) * pitch を通る長さ `length` の線分(中央が s_k·pitchUnit)。
// 六角配置(mini 90/-30/30、ELITPC 0/-120/-60)では pitch ⟂ strip なので `pitchUnit·(x0,y0) = s_k`。
//
// 格子: 全セル BASE の上に、直線トラックが横切る strip ごとにガウス形のパルスを 1 つ足す。
// パルスの中心 bucket はトラックの xy 位置から線形に決まる(z の傾きを持つ)。
// =============================================================================================

/// mini eTPC の strip 角度(U/V/W、度)。
const MINI_ANGLES: [f64; 3] = [90.0, -30.0, 30.0];
/// ELITPC の strip 角度(U/V/W、度)。
const ELITPC_ANGLES: [f64; 3] = [0.0, -120.0, -60.0];

/// 全セルの値(ベースライン)。
const BASE: u16 = 250;
const N_BUCKETS: usize = 512;

struct Layout {
    angles: [f64; 3],
    /// 面ごとの strip 本数(奇数 → 中央の strip が s = 0)。
    n: usize,
    /// strip の間隔 [mm]。
    pitch: f64,
    /// 線分の長さ [mm]。
    length: f64,
}

/// ±100 mm を覆う 135 本 × 1.5 mm。
fn layout_of(angles: [f64; 3]) -> Layout {
    Layout {
        angles,
        n: 135,
        pitch: 1.5,
        length: 300.0,
    }
}

fn unit_of(deg: f64) -> Vec2 {
    let a = (deg * PI) / 180.0;
    [a.cos(), a.sin()]
}

fn normalized(x: f64, y: f64) -> Vec2 {
    let norm = x.hypot(y);
    [x / norm, y / norm]
}

/// TPCReco `GeometryTPC::Load`(GeometryTPC.cpp:271-283)と同じ式の pitch 単位ベクトル。
fn pitch_units(angles: [f64; 3]) -> [Vec2; 3] {
    let [u, v, w] = angles.map(unit_of);
    let pu = normalized(w[0] + v[0], w[1] + v[1]);
    [
        [-pu[0], -pu[1]],
        normalized(u[0] + w[0], u[1] + w[1]),
        normalized(v[0] - u[0], v[1] - u[1]),
    ]
}

/// strip k(1 始まり)の pitch 座標 [mm]。
fn strip_s(layout: &Layout, strip: usize) -> f64 {
    (strip as f64 - (layout.n as f64 + 1.0) / 2.0) * layout.pitch
}

fn synthetic_geometry(layout: &Layout) -> StripGeometry {
    let pitch = pitch_units(layout.angles);
    let plane = |p: usize| {
        let unit = unit_of(layout.angles[p]);
        let segments = (1..=layout.n)
            .map(|strip| {
                let s = strip_s(layout, strip);
                Segment {
                    strip: strip as u16,
                    section: 0,
                    x0: s * pitch[p][0] - (unit[0] * layout.length) / 2.0,
                    y0: s * pitch[p][1] - (unit[1] * layout.length) / 2.0,
                    length: layout.length,
                }
            })
            .collect();
        PlaneSegments {
            unit,
            pitch_unit: pitch[p],
            segments,
        }
    };
    let extent = (layout.n as f64 * layout.pitch) / 2.0;
    StripGeometry {
        pad_pitch: layout.pitch * (2.0 / 3.0_f64.sqrt()),
        strip_pitch: layout.pitch,
        range_xy: [-extent, extent, -extent, extent],
        planes: Planes {
            u: plane(0),
            v: plane(1),
            w: plane(2),
        },
    }
}

type Grids = [Vec<u16>; 3];

/// 全セル BASE の 3 面(U/V/W)。
fn empty_event(layout: &Layout, n_buckets: usize) -> Grids {
    std::array::from_fn(|_| vec![BASE; layout.n * n_buckets])
}

fn refs(grids: &Grids) -> [&[u16]; 3] {
    grids.each_ref().map(Vec::as_slice)
}

/// strip 1 本にガウス形のパルスを足す(中心 `center` [bucket]、σ = 2 cell、高さ `amplitude` [ADC])。
/// ±6σ の外は足さない。値は 4095 で止める。
fn add_pulse(grid: &mut [u16], n_buckets: usize, strip: usize, center: f64, amplitude: f64) {
    let sigma = 2.0;
    let n_strips = grid.len() / n_buckets;
    if strip < 1 || strip > n_strips {
        return;
    }
    let offset = (strip - 1) * n_buckets;
    let lo = (center - 6.0 * sigma).ceil().max(0.0);
    let hi = (center + 6.0 * sigma).floor().min((n_buckets - 1) as f64);
    if hi < lo {
        return;
    }
    for t in lo as usize..=hi as usize {
        let d = (t as f64 - center) / sigma;
        let add = (amplitude * (-0.5 * d * d).exp()).round();
        let cell = &mut grid[offset + t];
        *cell = (f64::from(*cell) + add).min(4095.0) as u16;
    }
}

/// 直線トラック: (x, y) [mm] と bucket が始点 a から終点 b まで線形に変わる。
struct Line {
    a: [f64; 3],
    b: [f64; 3],
}

/// 一様な dE/dx の直線トラック: strip k は自分の帯 [s_k − pitch/2, s_k + pitch/2] に入るトラックの部分の
/// 電荷を集める(端の strip は一部だけ)。パルスの高さ = `amplitude` × (その部分の xy 長さ / pitch)、
/// 中心 bucket = その部分の中点の時刻。xy 長さ = pitch 座標の重なり / |pitch·d_xy|。
/// 帯を丸ごと横切る |pitch·d_xy| = 1 の strip: 高さ 1100 ADC・σ 2 cell → 積分電荷 ≈ 5500 ADC
/// (macros/analyzeUVW.C の注記「track strips peak at ~5500」)。
fn add_track(grids: &mut Grids, layout: &Layout, line: &Line, amplitude: f64) {
    let pitch = pitch_units(layout.angles);
    let dx = line.b[0] - line.a[0];
    let dy = line.b[1] - line.a[1];
    let xy_length = dx.hypot(dy);
    for (plane, grid) in grids.iter_mut().enumerate() {
        let [px, py] = pitch[plane];
        let s_a = px * line.a[0] + py * line.a[1];
        let s_b = px * line.b[0] + py * line.b[1];
        let proj = (s_b - s_a).abs() / xy_length; // |pitch·d_xy|
        for strip in 1..=layout.n {
            let s = strip_s(layout, strip);
            let lo = s_a.min(s_b).max(s - layout.pitch / 2.0);
            let hi = s_a.max(s_b).min(s + layout.pitch / 2.0);
            if hi <= lo {
                continue;
            }
            let f = ((lo + hi) / 2.0 - s_a) / (s_b - s_a);
            let height = (amplitude * (hi - lo)) / proj / layout.pitch;
            let center = line.a[2] + f * (line.b[2] - line.a[2]);
            add_pulse(grid, N_BUCKETS, strip, center, height);
        }
    }
}

/// 全 strip に小さなパルス(halo — 積分電荷 ≈ 100·2·√(2π) ≈ 500 < 2000)を足す。
fn add_halo(grids: &mut Grids, n_buckets: usize, center: f64) {
    for grid in grids.iter_mut() {
        for strip in 1..=grid.len() / n_buckets {
            add_pulse(grid, n_buckets, strip, center, 100.0);
        }
    }
}

fn reco_for(geometry: &StripGeometry) -> Reco {
    Reco::new(RecoGeometry::new(geometry))
}

fn reason(fit: &Result<Track, NoTrack>) -> String {
    match fit {
        Ok(_) => panic!("expected no track, got {fit:?}"),
        Err(reason) => reason.to_string(),
    }
}

fn track_of(fit: &Result<Track, NoTrack>) -> Track {
    match fit {
        Ok(track) => *track,
        Err(reason) => panic!("expected a track, got: {reason}"),
    }
}

// =============================================================================================
// 定数は macros と同じ値
// =============================================================================================

#[test]
fn constants_match_the_macros() {
    assert_eq!(ADC_THRESHOLD, 35.0);
    assert_eq!((PED_LO, PED_HI), (1, 20));
    assert_eq!(PEAK_HALF_CELLS, 5);
    assert_eq!(STRIP_Q_MIN, 2000.0);
    assert_eq!(BAND_MM, 10.0);
    assert_eq!(MIN_VIEW_HITS_BAND, 4);
    assert_eq!(MIN_VIEW_HITS_TRACK, 3);
    assert_eq!(QUANTILE, 0.02);
    assert_eq!(MIN_EIGEN_GAP_MM2, 1.0);
    assert_eq!(MIN_PROJ, 0.3);
}

#[test]
fn mm_per_cell_is_the_macros_default_drift() {
    // 0.7233 × (1200 V / 20 cm / 100 mbar) / 25 MHz × 10 = 0.173592 mm/cell
    assert_close(MM_PER_CELL, 0.173592, 12);
}

// =============================================================================================
// strip_hits — analyzeUVW.C 0(1 strip 1 ヒット、ピーク ±5 cell、halo cut)
// =============================================================================================

/// 6 strip × 512 cell、ベースライン 250。s は任意の非対称な値。
const POSITIONS: [Option<f64>; 6] = [
    Some(-7.25),
    Some(-2.5),
    Some(0.75),
    Some(4.0),
    Some(9.5),
    Some(13.0),
];

fn set_cell(grid: &mut [u16], strip: usize, bucket: usize, adc: u16) {
    grid[(strip - 1) * N_BUCKETS + bucket] = adc;
}

fn strip_hits_plane() -> Vec<u16> {
    let mut grid = vec![250u16; 6 * N_BUCKETS];
    // strip 1: halo — ピーク 300 + 両隣 150 → q = 600 < 2000 で落ちる
    set_cell(&mut grid, 1, 29, 400);
    set_cell(&mut grid, 1, 30, 550);
    set_cell(&mut grid, 1, 31, 400);
    // strip 2: ペデスタル窓 1..20 = 245×10 + 255×10 → 250。cell 0 は窓の外(0 でも効かない)。
    //   cell 40: +30(≤ 35 で除外)、41: +1000、42: +1500(ピーク)、43: +1000、44: +36(> 35 で採用)、
    //   47: +20(窓内だが除外)、48: +500(ピーク+6 = 窓の外)。
    //   q = 1000 + 1500 + 1000 + 36 = 3536、Σ q·t = 41000 + 63000 + 43000 + 1584 = 148584
    set_cell(&mut grid, 2, 0, 0);
    for t in 1..=10 {
        set_cell(&mut grid, 2, t, 245);
    }
    for t in 11..=20 {
        set_cell(&mut grid, 2, t, 255);
    }
    set_cell(&mut grid, 2, 40, 280);
    set_cell(&mut grid, 2, 41, 1250);
    set_cell(&mut grid, 2, 42, 1750);
    set_cell(&mut grid, 2, 43, 1250);
    set_cell(&mut grid, 2, 44, 286);
    set_cell(&mut grid, 2, 47, 270);
    set_cell(&mut grid, 2, 48, 750);
    // strip 3: ピークがちょうどペデスタル + 35 → `<= ADC_THRESHOLD` で落ちる
    set_cell(&mut grid, 3, 60, 285);
    // strip 4: 同じ高さの山が 2 つ(cell 100 と 300)→ 最初の最大(100)を採る。q = 3500、t = 100
    for c in [100, 300] {
        set_cell(&mut grid, 4, c - 1, 1250);
        set_cell(&mut grid, 4, c, 1750);
        set_cell(&mut grid, 4, c + 1, 1250);
    }
    // strip 5: 末尾(cell 509..511)。窓は 505..511 で止まる(strip 6 の cell 0.. を読まない)。
    //   q = 3500、Σ q·t = 509000 + 765000 + 511000 → t = 510
    set_cell(&mut grid, 5, 509, 1250);
    set_cell(&mut grid, 5, 510, 1750);
    set_cell(&mut grid, 5, 511, 1250);
    // strip 6: cell 0..3 = 2250 → ペデスタル = (3×2250 + 17×250)/20 = 550、ピーク = cell 0(最初の最大)、
    //   窓 0..5: 1700 × 4(cell 4,5 は −300 で除外)→ q = 6800、t = 1700×(0+1+2+3)/6800 = 1.5
    for t in 0..=3 {
        set_cell(&mut grid, 6, t, 2250);
    }
    grid
}

fn strip_hits_of(positions: &[Option<f64>]) -> (Vec<StripHit>, usize) {
    let mut hits = Vec::new();
    let no_segment = strip_hits(1, &strip_hits_plane(), N_BUCKETS, positions, &mut hits);
    (hits, no_segment)
}

#[test]
fn strip_hits_drop_halo_and_threshold_strips_one_hit_per_strip_in_order() {
    let (hits, no_segment) = strip_hits_of(&POSITIONS);
    let strips: Vec<u16> = hits.iter().map(|h| h.strip).collect();
    assert_eq!(strips, vec![2, 4, 5, 6]);
    assert_eq!(no_segment, 0);
    assert!(hits.iter().all(|h| h.view == 1));
}

#[test]
fn strip_hits_charge_and_time_skip_cells_at_or_below_threshold_and_outside_the_window() {
    let (hits, _) = strip_hits_of(&POSITIONS);
    let h = hits[0];
    assert_eq!(h.q, 3536.0);
    assert_close(h.t, 148584.0 / 3536.0, 12);
    assert_close(h.z, (148584.0 / 3536.0) * MM_PER_CELL, 12);
    assert_eq!(h.s, -2.5);
}

#[test]
fn strip_hits_take_the_first_maximum_on_a_tie() {
    let (hits, _) = strip_hits_of(&POSITIONS);
    assert_eq!(hits[1].q, 3500.0);
    assert_eq!(hits[1].t, 100.0);
}

#[test]
fn strip_hits_window_stops_at_the_last_cell_and_pedestal_is_cells_1_to_20() {
    let (hits, _) = strip_hits_of(&POSITIONS);
    assert_eq!(hits[2].q, 3500.0);
    assert_eq!(hits[2].t, 510.0);
    assert_eq!(hits[3].q, 6800.0);
    assert_eq!(hits[3].t, 1.5);
}

/// stripGeometry に線分の無い strip のヒットは数えて外す(silent にしない)。
#[test]
fn strip_hits_count_and_skip_strips_without_a_segment() {
    let mut partial = POSITIONS;
    partial[3] = None; // strip 4
    let (hits, no_segment) = strip_hits_of(&partial);
    let strips: Vec<u16> = hits.iter().map(|h| h.strip).collect();
    assert_eq!(strips, vec![2, 5, 6]);
    assert_eq!(no_segment, 1);
}

// =============================================================================================
// RecoGeometry — s = pitchUnit·(x0, y0)(section 番号が最小の線分)
// =============================================================================================

#[test]
fn reco_geometry_takes_the_lowest_section_whatever_the_order() {
    let segment = |strip, section, x0, y0| Segment {
        strip,
        section,
        x0,
        y0,
        length: 5.0,
    };
    let geometry = StripGeometry {
        pad_pitch: 1.7320508,
        strip_pitch: 1.5,
        range_xy: [-10.0, 10.0, -10.0, 10.0],
        planes: Planes {
            // U: pitch (0, 1) → s = y0。strip 3 は section 1 が先に来ても section 0(y0 = 4.5)を採る。
            u: PlaneSegments {
                unit: [1.0, 0.0],
                pitch_unit: [0.0, 1.0],
                segments: vec![
                    segment(3, 1, 0.0, 9.9),
                    segment(3, 0, -2.0, 4.5),
                    segment(7, 0, 1.0, -3.0),
                ],
            },
            // V: pitch (0.6, 0.8) → s = 0.6·x0 + 0.8·y0 = 0.6·5 + 0.8·(−2.5) = 1
            v: PlaneSegments {
                unit: [0.8, -0.6],
                pitch_unit: [0.6, 0.8],
                segments: vec![segment(2, 0, 5.0, -2.5)],
            },
            w: PlaneSegments {
                unit: [1.0, 0.0],
                pitch_unit: [0.0, 1.0],
                segments: vec![],
            },
        },
    };
    let reco = RecoGeometry::new(&geometry);
    assert_eq!(
        reco.positions[0],
        vec![None, None, Some(4.5), None, None, None, Some(-3.0)]
    );
    assert_eq!(reco.positions[1].len(), 2);
    assert_eq!(reco.positions[1][0], None);
    assert_close(reco.positions[1][1].unwrap(), 1.0, 12);
    assert!(reco.positions[2].is_empty());
    assert_eq!(reco.pitch, [[0.0, 1.0], [0.6, 0.8], [0.0, 1.0]]);
}

// =============================================================================================
// band_select — analyzeUVW.C 1–4(電荷加重 PCA の直線から ±10 mm)
// =============================================================================================

/// 平らなトラック: z = 50 mm、s = −15..15(1.5 mm 刻み、21 本、q = 5000)。
fn flat_line() -> Vec<StripHit> {
    (-10..=10)
        .map(|k| hit(0, 1.5 * f64::from(k), 50.0, 5000.0))
        .collect()
}

#[test]
fn band_select_drops_hits_beyond_10_mm_and_keeps_those_within() {
    // 外れ値: s = 0.75 で z = 50 ± 12(帯の外)と 50 ± 9.5(帯の中)、q = 3000。
    // 対称なので mz = 50、czs = 0、css ≫ czz → 主軸 = s 軸、perp = −(z − 50)。
    let mut hits = flat_line();
    hits.extend([hit(0, 0.75, 62.0, 3000.0), hit(0, 0.75, 38.0, 3000.0)]);
    hits.extend([hit(0, 0.75, 59.5, 3000.0), hit(0, 0.75, 40.5, 3000.0)]);
    let mut kept = Vec::new();
    band_select(&hits, &mut kept);
    assert_eq!(kept.len(), 23);
    assert!(!kept.iter().any(|h| h.z == 62.0 || h.z == 38.0));
    assert_eq!(
        kept.iter().filter(|h| h.z == 59.5 || h.z == 40.5).count(),
        2
    );
}

#[test]
fn band_select_needs_four_hits() {
    let line = flat_line();
    let mut kept = vec![hit(0, 99.0, 99.0, 1.0)]; // 前の中身は消える
    band_select(&line[..3], &mut kept);
    assert!(kept.is_empty(), "4 本未満の面は帯フィットをしない(空)");
    band_select(&line[..4], &mut kept);
    assert_eq!(kept.len(), 4);
}

// =============================================================================================
// xy_covariance — makeTracks.C 2 の一般化(3×3 連立)
// =============================================================================================

#[test]
fn xy_covariance_matches_the_macros_closed_form_on_the_mini_pitch() {
    let variances = [2.0, 7.5, 3.25];
    // macros: Sxx = var_U、Sxy = (var_V − var_W)/√3、Syy = (var_V + var_W − 0.5·var_U)/1.5
    let closed = [
        2.0,
        (7.5 - 3.25) / 3.0_f64.sqrt(),
        (7.5 + 3.25 - 0.5 * 2.0) / 1.5,
    ];
    for pitch in [mini_pitch(), pitch_units(MINI_ANGLES)] {
        let s = xy_covariance(variances, pitch).unwrap();
        for i in 0..3 {
            assert_close(s[i], closed[i], 12);
        }
    }
}

#[test]
fn xy_covariance_round_trips_on_the_elitpc_pitch() {
    // var_v = p_vᵀ S p_v の往復
    let [sxx, sxy, syy] = [2.0, -0.7, 5.5];
    let pitch = pitch_units(ELITPC_ANGLES);
    let variances = pitch.map(|[px, py]| px * px * sxx + 2.0 * px * py * sxy + py * py * syy);
    let s = xy_covariance(variances, pitch).unwrap();
    assert_close(s[0], sxx, 12);
    assert_close(s[1], sxy, 12);
    assert_close(s[2], syy, 12);
}

#[test]
fn xy_covariance_is_none_for_parallel_pitches() {
    assert_eq!(
        xy_covariance([1.0, 2.0, 3.0], [[1.0, 0.0], [-1.0, 0.0], [0.0, 1.0]]),
        None
    );
}

// =============================================================================================
// fit_track — makeTracks.C 1–5(手計算)
// =============================================================================================

/// y = 2 の x 方向のトラック、x = 4, 7, 10, 13, 16、z = 10 + 0.5·(x − 10)、q = 1000(全 15 ヒット)。
///   U: s = −x、V: s = 0.5x + √3、W: s = 0.5x − √3(mini の pitch)
///   var_U = 18、var_V = var_W = 4.5 → Sxx = 18、Sxy = 0、Syy = 0 → φ = 0、d = (1, 0)
///   ms = (−10, 5 + √3, 5 − √3) → anchor = (10, 2)。l = x − 10 ∈ {−6, −3, 0, 3, 6}
///   z = 10 + 0.5·l → tanθ = 0.5、z0 = 10。2 % の位置: 1 本目で 1/15 > 0.02 → lo = −6、
///   98 %: 14/15 ≤ 0.98 → hi = 6。start = (4, 2, 7)、end = (16, 2, 13)、
///   length = 12·√1.25、dir = (2, 0, 1)/√5、charge = 15000
fn hand_line() -> [Vec<StripHit>; 3] {
    let xs = [4.0, 7.0, 10.0, 13.0, 16.0];
    let s3 = 3.0_f64.sqrt();
    let z = |x: f64| 10.0 + 0.5 * (x - 10.0);
    [
        xs.iter().map(|&x| hit(0, -x, z(x), 1000.0)).collect(),
        xs.iter()
            .map(|&x| hit(1, 0.5 * x + s3, z(x), 1000.0))
            .collect(),
        xs.iter()
            .map(|&x| hit(2, 0.5 * x - s3, z(x), 1000.0))
            .collect(),
    ]
}

fn fit(kept: [&[StripHit]; 3], pitch: [Vec2; 3]) -> (Result<Track, NoTrack>, Vec<TrackPoint>) {
    let mut lq = Vec::new();
    let mut points = Vec::new();
    let result = fit_track(kept, pitch, &mut lq, &mut points);
    (result, points)
}

#[test]
fn fit_track_hand_computed_line() {
    let kept = hand_line();
    let (result, _) = fit(kept.each_ref().map(Vec::as_slice), mini_pitch());
    let t = track_of(&result);
    assert_close(t.phi_deg, 0.0, 9);
    assert_close(t.dir[0], 2.0 / 5.0_f64.sqrt(), 9);
    assert_close(t.dir[1], 0.0, 9);
    assert_close(t.dir[2], 1.0 / 5.0_f64.sqrt(), 9);
    for (got, want) in [
        (t.start, [4.0, 2.0, 7.0]),
        (t.end, [16.0, 2.0, 13.0]),
        (t.start_f, [4.0, 2.0, 7.0]),
        (t.end_f, [16.0, 2.0, 13.0]),
    ] {
        for i in 0..3 {
            assert_close(got[i], want[i], 9);
        }
    }
    assert_close(t.length, 12.0 * 1.25_f64.sqrt(), 9);
    assert_close(t.length_f, 12.0 * 1.25_f64.sqrt(), 9);
    assert_close(t.xy_length, 12.0, 9);
    assert_eq!(t.charge, 15000.0);
}

/// 表示用の点: 各ヒットを anchor + l·d に置き、bucket はそのヒットの時刻。
#[test]
fn fit_track_points_lie_on_the_line_with_the_hit_time() {
    let kept = hand_line();
    let (result, points) = fit(kept.each_ref().map(Vec::as_slice), mini_pitch());
    assert!(result.is_ok());
    assert_eq!(points.len(), 15);
    for p in &points {
        assert_close(p.y, 2.0, 9);
        // z = 10 + 0.5·(x − 10) → x = 2z − 10
        assert_close(p.x, 2.0 * p.t * MM_PER_CELL - 10.0, 9);
        assert_eq!(p.q, 1000.0);
    }
    // 面 → ヒットの順(U 5 本、V 5 本、W 5 本)
    let views: Vec<u8> = points.iter().map(|p| p.view).collect();
    assert_eq!(views, [[0u8; 5], [1; 5], [2; 5]].concat());
}

/// 面のヒットが 3 本未満 → ok でない(理由に面と本数)。
#[test]
fn fit_track_needs_three_hits_per_view() {
    let kept = hand_line();
    let (result, _) = fit([&kept[0], &kept[1][..2], &kept[2]], mini_pitch());
    assert_eq!(
        result,
        Err(NoTrack::TooFewHits([
            Shortage::Enough,
            Shortage::InBand(2),
            Shortage::Enough
        ]))
    );
    assert_eq!(reason(&result), "V has 2 strip hits in the band (needs 3)");
}

#[test]
fn fit_track_rejects_degenerate_pitches() {
    let kept = hand_line();
    let (result, _) = fit(
        kept.each_ref().map(Vec::as_slice),
        [[1.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
    );
    assert_eq!(result, Err(NoTrack::DegeneratePitch));
    assert!(reason(&result).contains("degenerate"));
}

/// xy に伸びていない(全面で同じ分散 → S = c·I)→ no xy elongation。
#[test]
fn fit_track_rejects_no_xy_elongation() {
    // 各面 s = −3, −1.5, 0, 1.5, 3(q 同じ)→ var = 1.5²·(4+1+0+1+4)/5 = 4.5 → S = 4.5·I、gap = 0
    let blob: [Vec<StripHit>; 3] = std::array::from_fn(|view| {
        (-2..=2)
            .map(|k| {
                let k = f64::from(k);
                hit(view as u8, 1.5 * k, 30.0 + 4.0 * k, 3000.0)
            })
            .collect()
    });
    let (result, _) = fit(blob.each_ref().map(Vec::as_slice), mini_pitch());
    assert_eq!(result, Err(NoTrack::NoElongation));
    assert_eq!(reason(&result), "no xy elongation (track along the drift)");
}

/// どの面も |pitch·d| < 0.3 → 経路長の分散が無い → ok でない。
#[test]
fn fit_track_rejects_no_usable_path_length() {
    // pitch U = (1,0)、V/W = (cos 10°, ±sin 10°)。S = diag(0.1, 100) になる分散を与える
    // → d = (0, 1)、|pitch·d| = 0, 0.174, 0.174 < 0.3 → l を持つヒットが無い → varL = 0
    let c = (10.0 * PI / 180.0).cos();
    let sn = (10.0 * PI / 180.0).sin();
    let pitch = [[1.0, 0.0], [c, sn], [c, -sn]];
    let variances = pitch.map(|[px, py]| px * px * 0.1 + py * py * 100.0);
    // 3 本(−a, 0, a、q 同じ)の分散 = 2a²/3 → a = √(1.5·var)
    let hits: [Vec<StripHit>; 3] = std::array::from_fn(|view| {
        let a = (1.5 * variances[view]).sqrt();
        [-a, 0.0, a]
            .iter()
            .map(|&s| hit(view as u8, s, 40.0, 3000.0))
            .collect()
    });
    let (result, _) = fit(hits.each_ref().map(Vec::as_slice), pitch);
    assert_eq!(result, Err(NoTrack::NoSpread));
    assert!(reason(&result).contains("no spread along the track"));
}

// =============================================================================================
// 合成ジオメトリ上の既知の直線(mini と ELITPC の角度)
// =============================================================================================

/// 原点を通らない非対称な直線: 第 1 象限の (10, 4) mm → (90, 26) mm、bucket 150 → 250(xy 長さ 83 mm)。
/// 傾き dy/dx = 22/80 = 0.275(転置なら 3.6、y 鏡像なら負)。φ = atan2(22, 80) = 15.38°。
/// どの strip 方向(mini 90/−30/30、ELITPC 0/−120/−60)とも 14.6° 以上離れる → 最も寝る面でも 15 本の
/// strip を横切る。
const TRACK: Line = Line {
    a: [10.0, 4.0, 150.0],
    b: [90.0, 26.0, 250.0],
};

/// 真の直線上の、始点から長さの割合 f の点 (x, y, z[mm])。
fn along_track(f: f64) -> [f64; 3] {
    let at = |i: usize| TRACK.a[i] + f * (TRACK.b[i] - TRACK.a[i]);
    [at(0), at(1), at(2) * MM_PER_CELL]
}

fn distance3(p: [f64; 3], q: [f64; 3]) -> f64 {
    ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt()
}

/// 電荷加重の傾き d(縦)/d(横)。
fn slope(points: &[(f64, f64, f64)]) -> f64 {
    let w: f64 = points.iter().map(|p| p.2).sum();
    let mx = points.iter().map(|p| p.2 * p.0).sum::<f64>() / w;
    let my = points.iter().map(|p| p.2 * p.1).sum::<f64>() / w;
    let sxy: f64 = points.iter().map(|p| p.2 * (p.0 - mx) * (p.1 - my)).sum();
    let sxx: f64 = points.iter().map(|p| p.2 * (p.0 - mx).powi(2)).sum();
    sxy / sxx
}

/// 既知の直線のイベントを再構成した結果(ジオメトリも返す)。
fn known_line(angles: [f64; 3]) -> (Reco, StripGeometry) {
    let layout = layout_of(angles);
    let geometry = synthetic_geometry(&layout);
    let mut grids = empty_event(&layout, N_BUCKETS);
    add_track(&mut grids, &layout, &TRACK, 1100.0);
    let mut reco = reco_for(&geometry);
    reco.run(refs(&grids), N_BUCKETS);
    (reco, geometry)
}

/// ok、φ は真の方向 atan2(22, 80) = 15.38° から 0.5° 以内(頭尻は決めない → 180° の不定性)。
fn check_phi(angles: [f64; 3]) {
    let (reco, _) = known_line(angles);
    let track = track_of(&reco.event().fit);
    let true_phi = 22.0_f64.atan2(80.0) * 180.0 / PI;
    let diff = (((track.phi_deg - true_phi + 270.0) % 180.0) - 90.0).abs();
    assert!(diff < 0.5, "φ = {} vs {true_phi}", track.phi_deg);
}

/// 2–98 % の端点が真の直線の 2 % / 98 % の点から 2 mm 以内(順序は問わない)。
/// 電荷は一様(dE/dx 一定)→ 2 % / 98 % の電荷分位点の真の位置は長さの 2 % / 98 %。
fn check_quantile_ends(angles: [f64; 3]) {
    let (reco, _) = known_line(angles);
    let track = track_of(&reco.event().fit);
    let (true2, true98) = (along_track(0.02), along_track(0.98));
    let direct = distance3(track.start, true2).max(distance3(track.end, true98));
    let swapped = distance3(track.start, true98).max(distance3(track.end, true2));
    assert!(direct.min(swapped) < 2.0, "{direct} / {swapped}");
}

/// z の傾きの向き: xy で終点側へ進むと z(bucket)が増える。
fn check_z_direction(angles: [f64; 3]) {
    let (reco, _) = known_line(angles);
    let track = track_of(&reco.event().fit);
    let along = track.dir[0] * (TRACK.b[0] - TRACK.a[0]) + track.dir[1] * (TRACK.b[1] - TRACK.a[1]);
    assert_ne!(track.dir[2], 0.0);
    assert_eq!(track.dir[2].signum(), along.signum());
}

/// 鏡像・回転を落とす: 点はすべて第 1 象限のトラックの外接矩形の中、傾きは真の 0.275 の近く
/// (転置なら 3.6、鏡像なら負)。UI 側はビンの位置(xy-projection.spec.ts)。
fn check_quadrant(angles: [f64; 3]) {
    let (reco, _) = known_line(angles);
    let points = reco.event().points;
    assert!(points.len() > 20, "{} points", points.len());
    for p in points {
        assert!(p.x > 7.0 && p.x < 93.0, "x = {}", p.x);
        assert!(p.y > 1.0 && p.y < 29.0, "y = {}", p.y);
    }
    let xy: Vec<(f64, f64, f64)> = points.iter().map(|p| (p.x, p.y, p.q)).collect();
    let s = slope(&xy);
    assert!(s > 0.22 && s < 0.33, "dy/dx = {s}");
}

/// XZ / YZ: 点の時刻は 150..250 の中で、x・y が増えると増える。
fn check_time_slopes(angles: [f64; 3]) {
    let (reco, _) = known_line(angles);
    let points = reco.event().points;
    for p in points {
        assert!(p.t > 145.0 && p.t < 255.0, "t = {}", p.t);
    }
    // bucket/x = 100/80 = 1.25、bucket/y = 100/22 = 4.5
    let xt: Vec<(f64, f64, f64)> = points.iter().map(|p| (p.x, p.t, p.q)).collect();
    let yt: Vec<(f64, f64, f64)> = points.iter().map(|p| (p.y, p.t, p.q)).collect();
    let (sx, sy) = (slope(&xt), slope(&yt));
    assert!(sx > 1.1 && sx < 1.4, "dt/dx = {sx}");
    assert!(sy > 4.0 && sy < 5.0, "dt/dy = {sy}");
}

/// 点 = |pitch_v·d| ≥ MIN_PROJ の面の帯内ヒット(電荷もそのまま)、残りは `unplaced`。
/// 点はすべて rangeXY と bucket の範囲の中(TS の「3 枚の総和 = 表示点の電荷の和、範囲外は 0」)。
fn check_placed_charge(angles: [f64; 3]) {
    let (reco, geometry) = known_line(angles);
    let event = reco.event();
    let track = track_of(&event.fit);
    let pitch = reco.geometry().pitch;
    let norm_xy = track.dir[0].hypot(track.dir[1]);
    let mut placed_q = 0.0;
    let mut placed_n = 0;
    let mut unplaced = 0;
    for (v, hits) in reco.selected().iter().enumerate() {
        let proj = (pitch[v][0] * track.dir[0] + pitch[v][1] * track.dir[1]) / norm_xy;
        if proj.abs() < MIN_PROJ {
            unplaced += hits.len();
        } else {
            placed_n += hits.len();
            placed_q += hits.iter().map(|h| h.q).sum::<f64>();
        }
    }
    // この直線(φ = 15.4°)では mini の W・ELITPC の U が |pitch·d| ≈ 0.26 < 0.3 で置けない。
    assert!(unplaced > 0, "1 面は直線上に置けないはず");
    assert_eq!(event.unplaced, unplaced);
    assert_eq!(event.points.len(), placed_n);
    assert_eq!(
        event.points.len() + event.unplaced,
        event.hits.iter().sum::<usize>()
    );
    let total: f64 = event.points.iter().map(|p| p.q).sum();
    assert_close(total, placed_q, 6);
    let [xmin, xmax, ymin, ymax] = geometry.range_xy;
    for p in event.points {
        assert!(p.x >= xmin && p.x < xmax && p.y >= ymin && p.y < ymax);
        assert!(p.t >= -0.5 && p.t < N_BUCKETS as f64 - 0.5);
    }
}

#[test]
fn known_line_mini_phi_within_half_a_degree() {
    check_phi(MINI_ANGLES);
}

#[test]
fn known_line_mini_quantile_ends_within_2_mm() {
    check_quantile_ends(MINI_ANGLES);
}

#[test]
fn known_line_mini_z_grows_towards_the_end() {
    check_z_direction(MINI_ANGLES);
}

#[test]
fn known_line_mini_points_stay_in_the_first_quadrant_with_the_true_slope() {
    check_quadrant(MINI_ANGLES);
}

#[test]
fn known_line_mini_point_times_rise_with_x_and_y() {
    check_time_slopes(MINI_ANGLES);
}

#[test]
fn known_line_mini_points_carry_the_placed_charge_inside_the_range() {
    check_placed_charge(MINI_ANGLES);
}

#[test]
fn known_line_elitpc_phi_within_half_a_degree() {
    check_phi(ELITPC_ANGLES);
}

#[test]
fn known_line_elitpc_quantile_ends_within_2_mm() {
    check_quantile_ends(ELITPC_ANGLES);
}

#[test]
fn known_line_elitpc_z_grows_towards_the_end() {
    check_z_direction(ELITPC_ANGLES);
}

#[test]
fn known_line_elitpc_points_stay_in_the_first_quadrant_with_the_true_slope() {
    check_quadrant(ELITPC_ANGLES);
}

#[test]
fn known_line_elitpc_point_times_rise_with_x_and_y() {
    check_time_slopes(ELITPC_ANGLES);
}

#[test]
fn known_line_elitpc_points_carry_the_placed_charge_inside_the_range() {
    check_placed_charge(ELITPC_ANGLES);
}

// =============================================================================================
// 1 イベント — halo・帯・ok でない理由・計算しない状態
// =============================================================================================

fn mini() -> (Layout, StripGeometry) {
    let layout = layout_of(MINI_ANGLES);
    let geometry = synthetic_geometry(&layout);
    (layout, geometry)
}

/// halo(q < 2000)の strip は落ちる: 全 strip に halo を足しても結果は同じ。
#[test]
fn halo_strips_are_dropped_so_adding_halo_changes_nothing() {
    let (layout, geometry) = mini();
    let mut clean = empty_event(&layout, N_BUCKETS);
    add_track(&mut clean, &layout, &TRACK, 1100.0);
    let mut haloed = empty_event(&layout, N_BUCKETS);
    add_track(&mut haloed, &layout, &TRACK, 1100.0);
    add_halo(&mut haloed, N_BUCKETS, 400.0); // トラック(150..250)から離れた時刻
    let mut a = reco_for(&geometry);
    a.run(refs(&clean), N_BUCKETS);
    let mut b = reco_for(&geometry);
    b.run(refs(&haloed), N_BUCKETS);
    assert_eq!(b.event().halo_hits, a.event().halo_hits);
    let strips = |reco: &Reco| -> Vec<Vec<u16>> {
        reco.selected()
            .iter()
            .map(|v| v.iter().map(|h| h.strip).collect())
            .collect()
    };
    assert_eq!(strips(&b), strips(&a));
    assert_close(
        track_of(&b.event().fit).phi_deg,
        track_of(&a.event().fit).phi_deg,
        9,
    );
}

/// 帯から 10 mm を超えて離れた strip は落ちる(U のトラックの端の隣に時刻の離れたパルス 1 本)。
#[test]
fn a_pulse_beyond_the_band_is_dropped() {
    let (layout, geometry) = mini();
    let mut grids = empty_event(&layout, N_BUCKETS);
    add_track(&mut grids, &layout, &TRACK, 1100.0);
    // U は s = −x。トラックは s ∈ [−90, −10]、z = 26.0..43.4 mm(dz/ds = −0.217)。
    // strip 7 = s −91.5 mm(トラックの外の隣)に bucket 330(z = 57.3 mm)のパルス:
    // 直線の延長(z ≈ 43.7)から Δz ≈ 13.6 → 直線との距離 ≈ 13.6·cos(12.2°) ≈ 13.3 mm > 10。
    // 遠すぎる外れ値は PCA の直線そのものを引っ張る(macros と同じ — 頑健ではない)ので、近くに置く。
    add_pulse(&mut grids[0], N_BUCKETS, 7, 330.0, 450.0);
    let mut reco = reco_for(&geometry);
    reco.run(refs(&grids), N_BUCKETS);
    let event = reco.event();
    assert_eq!(event.halo_hits[0], reco.selected()[0].len() + 1);
    assert!(!reco.selected()[0].iter().any(|h| h.strip == 7));
    assert!(event.fit.is_ok());
}

/// halo だけ → no track: 各面のヒット数と必要数を文で。
#[test]
fn halo_only_gives_the_per_view_shortage_reason() {
    let (layout, geometry) = mini();
    let mut grids = empty_event(&layout, N_BUCKETS);
    add_halo(&mut grids, N_BUCKETS, 200.0);
    let mut reco = reco_for(&geometry);
    reco.run(refs(&grids), N_BUCKETS);
    let event = reco.event();
    assert_eq!(
        reason(&event.fit),
        "U has 0 strip hits after the halo cut (needs 4); \
         V has 0 strip hits after the halo cut (needs 4); \
         W has 0 strip hits after the halo cut (needs 4)"
    );
    assert!(event.points.is_empty());
    assert_eq!((event.hits, event.unplaced), ([0, 0, 0], 0));
}

/// 1 面だけ 3 本 → その面だけ理由に出る(単数・複数も正しく)。
#[test]
fn one_short_view_is_named_alone_with_singular_and_plural() {
    let (layout, geometry) = mini();
    let mut grids = empty_event(&layout, N_BUCKETS);
    add_track(&mut grids, &layout, &TRACK, 1100.0);
    // W のトラックの strip を 3 本だけ残す: W の全セルを BASE に戻してから 3 本だけ足す
    grids[2].fill(BASE);
    for strip in [80, 81, 82] {
        add_pulse(&mut grids[2], N_BUCKETS, strip, 200.0, 600.0);
    }
    let mut reco = reco_for(&geometry);
    reco.run(refs(&grids), N_BUCKETS);
    assert_eq!(
        reason(&reco.event().fit),
        "W has 3 strip hits after the halo cut (needs 4)"
    );
    grids[2].fill(BASE);
    add_pulse(&mut grids[2], N_BUCKETS, 80, 200.0, 600.0);
    reco.run(refs(&grids), N_BUCKETS);
    assert_eq!(
        reason(&reco.event().fit),
        "W has 1 strip hit after the halo cut (needs 4)"
    );
}

/// ドリフト方向のトラック(xy に伸びない)→ no xy elongation。
#[test]
fn a_track_along_the_drift_has_no_xy_elongation() {
    // (0,0) の真上: 各面で s = −3..3 の 5 本(中央 strip ±2)に、時刻をずらしたパルス
    let (layout, geometry) = mini();
    let mut grids = empty_event(&layout, N_BUCKETS);
    let center = layout.n.div_ceil(2); // = (n + 1) / 2 = 68(中央の strip)
    for grid in grids.iter_mut() {
        for k in -2i32..=2 {
            let strip = (center as i32 + k) as usize;
            add_pulse(grid, N_BUCKETS, strip, 200.0 + 30.0 * f64::from(k), 600.0);
        }
    }
    let mut reco = reco_for(&geometry);
    reco.run(refs(&grids), N_BUCKETS);
    assert_eq!(
        reason(&reco.event().fit),
        "no xy elongation (track along the drift)"
    );
}

/// bucket がペデスタル窓(cell 1..20)に足りない → 計算せず、理由を出す(TS は null = 計算しない。
/// Rust は 0x04 に理由として載せる — silent にしない)。3 面が揃わない状態は Rust には無い
/// (3 面とも同じ built event の同じ格子 — monitor のフックの試験を参照)。
#[test]
fn a_short_bucket_window_is_reported_not_computed() {
    let (layout, geometry) = mini();
    let mut reco = reco_for(&geometry);
    reco.run(refs(&empty_event(&layout, 20)), 20);
    assert_eq!(reco.event().fit, Err(NoTrack::ShortWindow(20)));
    assert_eq!(
        reason(&reco.event().fit),
        "20 time buckets do not cover the pedestal window (cells 1–20)"
    );
    reco.run(refs(&empty_event(&layout, 21)), 21);
    assert!(matches!(reco.event().fit, Err(NoTrack::TooFewHits(_))));
}

/// stripGeometry に無い strip のヒットは数えて外す(`no_segment` — 0x04 の `unplaced` とは別)。
#[test]
fn hits_on_strips_without_a_segment_are_counted() {
    let (layout, geometry) = mini();
    let mut grids = empty_event(&layout, N_BUCKETS);
    add_track(&mut grids, &layout, &TRACK, 1100.0);
    let mut trimmed = geometry.clone();
    trimmed.planes.v.segments.retain(|s| s.strip != 90);

    let mut full = reco_for(&geometry);
    full.run(refs(&grids), N_BUCKETS);
    assert!(full.selected()[1].iter().any(|h| h.strip == 90));
    assert_eq!(full.event().no_segment, 0);

    let mut reco = reco_for(&trimmed);
    reco.run(refs(&grids), N_BUCKETS);
    assert_eq!(reco.event().no_segment, 1);
    assert_eq!(reco.event().halo_hits[1], full.event().halo_hits[1] - 1);
}
