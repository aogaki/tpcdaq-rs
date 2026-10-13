//! strip 線分の mm(TODO/125)— Event Display の XY / XZ / YZ 投影を UI が計算するための材料。
//!
//! [`Geometry`] の strip 行(`OFF_PAD OFF_STRIP LEN_PADS`)とヘッダ(`ANGLES` / `DIAMOND SIZE` /
//! `REFERENCE POINT`)から、各 strip の線分を TPCReco 規約で mm に直す**純関数**(IO なし)。
//! **mm を扱うのはこのモジュールだけ**(SPEC §4.4 の例外の範囲 — DAQ・保存・§5.2 ヒストの
//! hot path は引き続き strip/bucket のみ)。monitor が起動時に 1 回呼び、WS の `stripGeometry`
//! として接続時に配る(SPEC §10.3)。
//!
//! 移植元(TPCReco `TPCReco-HIGS2026_online`、読み取りのみ。照合は `tools/geometry_xy_oracle`):
//!
//! * `GeometryTPC::Load` — 角度は `[0, 360)` へ正規化し、平行・反平行の組は拒否する。
//!   pad_pitch = DIAMOND SIZE × √3、strip_pitch = DIAMOND SIZE × 1.5。
//!   unit_p = (cos θ_p, sin θ_p)、pitchUnit は U = −(W+V).Unit()、V = (U+W).Unit()、
//!   W = (V−U).Unit()(六角配置でなければ unit と直交しない)。
//!   offset = OFF_STRIP × strip_pitch × pitchUnit + OFF_PAD × pad_pitch × unit。
//!   `LEN_PADS < 1.0` の行は捨てる。同じ `(plane, section, strip)` は mapByStrip の後勝ち。
//! * `StripTPC` — 長さ = pad_pitch × (int)LEN_PADS(pad 数は `int` で持つ = 切り捨て)。
//! * `InitTH2Poly` + ROOT `TH2Poly::AddBin`(`SetFloat(true)`)— rangeXY = `GeometryTPC::rangeXY()`
//!   (Get2D のビン範囲)。TH2Poly の軸は全線分の始点・終点の外接矩形で作られ、strip ごとの
//!   菱形 pad の多角形(bin)を足すたびにその頂点まで広がる。頂点は
//!   `p0 + ipad·pad_pitch·unit + {0, d·R(+30°)unit, pad_pitch·unit, d·R(−30°)unit}`
//!   (`p0` = 線分の始点 `x0 − 0.5·pad_pitch·unit`、`d` = DIAMOND SIZE、`ipad` = 0..npads)。

use std::collections::BTreeMap;

use serde::{Serialize, Serializer};

use crate::geometry::{Geometry, Plane, StripRow};

/// `stripGeometry` の中身(SPEC §10.3 / TODO/125 の WS 契約。`type` は monitor が足す)。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StripGeometry {
    pub pad_pitch: f64,
    pub strip_pitch: f64,
    /// `[xmin, xmax, ymin, ymax]` [mm] = TPCReco `GeometryTPC::rangeXY()`(TH2Poly の軸)。
    #[serde(rename = "rangeXY")]
    pub range_xy: [f64; 4],
    pub planes: Planes,
}

/// 面ごとの線分。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Planes {
    #[serde(rename = "U")]
    pub u: PlaneSegments,
    #[serde(rename = "V")]
    pub v: PlaneSegments,
    #[serde(rename = "W")]
    pub w: PlaneSegments,
}

impl Planes {
    /// `Plane` で引く。
    pub fn get(&self, plane: Plane) -> &PlaneSegments {
        match plane {
            Plane::U => &self.u,
            Plane::V => &self.v,
            Plane::W => &self.w,
        }
    }
}

/// 1 面ぶん。`segments` は strip → section の昇順。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaneSegments {
    /// strip に沿う単位ベクトル(pad 番号が増える向き)。
    pub unit: [f64; 2],
    /// strip 番号が増える向きの単位ベクトル(TPCReco の pitch_unit_vec)。
    pub pitch_unit: [f64; 2],
    pub segments: Vec<Segment>,
}

/// 1 本の線分。JSON では `[strip, section, x0, y0, length]` の 5 要素配列。
///
/// `(x0, y0)` = reference point + offset [mm]。線分の有効範囲は `(x0, y0)` から `unit` 方向に
/// `[−0.5·padPitch, length]`(TPCReco `GetCrossPoint` の範囲、許容誤差はその外側)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    /// 1 始まり(0x02 Uvw の strip 番号と同じ)。
    pub strip: u16,
    pub section: u8,
    pub x0: f64,
    pub y0: f64,
    /// pad_pitch × (int)LEN_PADS [mm]。
    pub length: f64,
}

impl Serialize for Segment {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        (self.strip, self.section, self.x0, self.y0, self.length).serialize(serializer)
    }
}

/// 計算できたが線分にしなかった / 差し替えた行(monitor が warn で可視化する — silent 禁止)。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StripXyNotes {
    /// `LEN_PADS < 1.0` で線分にしなかった行の行番号(TPCReco も捨てる)。
    pub short_rows: Vec<usize>,
    /// 同じ `(plane, section, strip)` の後の行に差し替えられた(= 捨てた)行の行番号。
    pub replaced_rows: Vec<usize>,
}

/// `stripGeometry` を作れない理由(WS の `unavailable` にそのまま載る)。
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum StripXyError {
    #[error("the geometry header has no ANGLES")]
    MissingAngles,
    #[error("the geometry header has no DIAMOND SIZE")]
    MissingDiamondSize,
    #[error("the geometry header has no REFERENCE POINT")]
    MissingReferencePoint,
    #[error("the geometry has no strip rows (with LEN_PADS >= 1)")]
    NoStrips,
    /// TPCReco `Load` も拒否する(pitchUnit が定まらない)。
    #[error("ANGLES {0:?} has a parallel or anti-parallel pair")]
    ParallelAngles([f64; 3]),
    /// TPCReco `Load` も拒否する(pad_pitch が 0 以下になる)。
    #[error("DIAMOND SIZE {0} is not positive")]
    NonPositiveDiamondSize(f64),
}

/// [`Geometry`] → `stripGeometry`(と、線分にしなかった行の記録)。
pub fn strip_geometry(geometry: &Geometry) -> Result<(StripGeometry, StripXyNotes), StripXyError> {
    let header = &geometry.header;
    let angles = header.angles_deg.ok_or(StripXyError::MissingAngles)?;
    let diamond = header
        .diamond_size_mm
        .ok_or(StripXyError::MissingDiamondSize)?;
    let reference = header
        .reference_point_mm
        .ok_or(StripXyError::MissingReferencePoint)?;

    // GeometryTPC::Load: [0, 360) へ正規化してから平行・反平行を拒否する。
    let normalized = angles.map(|a| ((a % 360.0) + 360.0) % 360.0);
    let half_turn = normalized.map(|a| a % 180.0);
    if half_turn[0] == half_turn[1] || half_turn[1] == half_turn[2] || half_turn[2] == half_turn[0]
    {
        return Err(StripXyError::ParallelAngles(angles));
    }
    // TPCReco の `if (val > 0.0)` と同じく 0 以下と NaN を弾く。
    if diamond <= 0.0 || diamond.is_nan() {
        return Err(StripXyError::NonPositiveDiamondSize(diamond));
    }

    let pad_pitch = diamond * 3.0_f64.sqrt();
    let strip_pitch = diamond * 1.5;
    let unit = normalized.map(|deg| {
        let rad = deg * (std::f64::consts::PI / 180.0); // TMath::DegToRad()
        [rad.cos(), rad.sin()]
    });
    let [u, v, w] = unit;
    let pitch_u = scale(-1.0, unit_of(add(w, v)));
    let pitch_v = unit_of(add(u, w));
    let pitch_w = unit_of(sub(v, u));
    let pitch = [pitch_u, pitch_v, pitch_w];

    // (plane, strip, section) → 行。後勝ち(TPCReco mapByStrip)。BTreeMap の順 = 出力の並び。
    let mut notes = StripXyNotes::default();
    let mut chosen: BTreeMap<(usize, u16, u8), StripRow> = BTreeMap::new();
    for row in geometry.strip_rows() {
        // TPCReco の `length_in_pads >= 1.0` の否定(NaN も捨てる)。
        if row.len_pads < 1.0 || row.len_pads.is_nan() {
            notes.short_rows.push(row.line_number);
            continue;
        }
        if let Some(replaced) = chosen.insert((row.plane as usize, row.strip, row.section), *row) {
            notes.replaced_rows.push(replaced.line_number);
        }
    }
    if chosen.is_empty() {
        return Err(StripXyError::NoStrips);
    }

    let mut segments: [Vec<Segment>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let mut range = [
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ];
    for (&(plane, strip, section), row) in &chosen {
        let a = row.off_strip * strip_pitch;
        let b = row.off_pad * pad_pitch;
        let x0 = reference.0 + (a * pitch[plane][0] + b * unit[plane][0]);
        let y0 = reference.1 + (a * pitch[plane][1] + b * unit[plane][1]);
        let npads = row.len_pads.trunc(); // StripTPC: int npads
        let length = pad_pitch * npads;
        grow_by_strip_polygon(&mut range, [x0, y0], unit[plane], npads, pad_pitch, diamond);
        segments[plane].push(Segment {
            strip,
            section,
            x0,
            y0,
            length,
        });
    }

    let [segments_u, segments_v, segments_w] = segments;
    let geometry = StripGeometry {
        pad_pitch,
        strip_pitch,
        range_xy: range,
        planes: Planes {
            u: PlaneSegments {
                unit: u,
                pitch_unit: pitch_u,
                segments: segments_u,
            },
            v: PlaneSegments {
                unit: v,
                pitch_unit: pitch_v,
                segments: segments_v,
            },
            w: PlaneSegments {
                unit: w,
                pitch_unit: pitch_w,
                segments: segments_w,
            },
        },
    };
    Ok((geometry, notes))
}

/// TPCReco `InitTH2Poly` が 1 本の strip について TH2Poly の軸に足す点を `range` に足す。
///
/// 1. 軸の初期値 = 線分の始点 `point1 = x0 − unit·0.5·pad_pitch` と終点 `point1 + unit·length`
///    の外接矩形(TPCReco の else-if の癖は 2. の頂点が覆うので結果に効かない)。
/// 2. `SetFloat(true)` の `TH2Poly::AddBin` が軸を広げる先 = 菱形 pad の多角形の頂点:
///    `point0 + unit·ipad·pad_pitch + {0, unit.Rotate(π/6)·d, unit·pad_pitch, unit.Rotate(−π/6)·d}`。
///    演算の順も TPCReco(TVector2)に合わせる。
fn grow_by_strip_polygon(
    range: &mut [f64; 4],
    x0: [f64; 2],
    unit: [f64; 2],
    npads: f64,
    pad_pitch: f64,
    diamond: f64,
) {
    let mut include = |p: [f64; 2]| {
        range[0] = range[0].min(p[0]);
        range[1] = range[1].max(p[0]);
        range[2] = range[2].min(p[1]);
        range[3] = range[3].max(p[1]);
    };
    let point0 = [
        x0[0] - unit[0] * 0.5 * pad_pitch,
        x0[1] - unit[1] * 0.5 * pad_pitch,
    ];
    include(point0);
    include([
        point0[0] + unit[0] * (pad_pitch * npads),
        point0[1] + unit[1] * (pad_pitch * npads),
    ]);
    let rotate = |phi: f64| {
        // TVector2::Rotate(phi)
        [
            unit[0] * phi.cos() - unit[1] * phi.sin(),
            unit[0] * phi.sin() + unit[1] * phi.cos(),
        ]
    };
    let up = rotate(std::f64::consts::PI / 6.0);
    let down = rotate(-std::f64::consts::PI / 6.0);
    let corners = [
        [0.0, 0.0],
        [up[0] * diamond, up[1] * diamond],
        [unit[0] * pad_pitch, unit[1] * pad_pitch],
        [down[0] * diamond, down[1] * diamond],
    ];
    let mut ipad = 0.0;
    while ipad < npads {
        let base = [
            point0[0] + unit[0] * ipad * pad_pitch,
            point0[1] + unit[1] * ipad * pad_pitch,
        ];
        for corner in corners {
            include([base[0] + corner[0], base[1] + corner[1]]);
        }
        ipad += 1.0;
    }
}

fn add(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] + b[0], a[1] + b[1]]
}

fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn scale(s: f64, a: [f64; 2]) -> [f64; 2] {
    [s * a[0], s * a[1]]
}

/// TVector2::Unit()(呼び手が平行を拒否済みなので長さ 0 は来ない)。
fn unit_of(a: [f64; 2]) -> [f64; 2] {
    let norm = (a[0] * a[0] + a[1] * a[1]).sqrt();
    [a[0] / norm, a[1] / norm]
}

// =====================================================================
// テスト(値はすべて手計算 — 出典は各コメント)
// =====================================================================

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::geometry;

    const SQRT3: f64 = 1.732_050_807_568_877_2;
    const TOL: f64 = 1e-12;

    fn close(actual: f64, expected: f64, what: &str) {
        assert!(
            (actual - expected).abs() <= TOL,
            "{what}: {actual} != {expected}"
        );
    }

    fn close2(actual: [f64; 2], expected: [f64; 2], what: &str) {
        close(actual[0], expected[0], &format!("{what}.x"));
        close(actual[1], expected[1], &format!("{what}.y"));
    }

    fn segment(g: &StripGeometry, plane: Plane, strip: u16, section: u8) -> Segment {
        *g.planes
            .get(plane)
            .segments
            .iter()
            .find(|s| s.strip == strip && s.section == section)
            .unwrap_or_else(|| panic!("no segment {plane:?} {strip}/{section}"))
    }

    fn build(text: &str) -> (StripGeometry, StripXyNotes) {
        strip_geometry(&geometry::parse(text)).unwrap()
    }

    // --- 角度 90 / −30 / 30(mini と同じ配置)---------------------------------

    /// DIAMOND SIZE 2 → pad_pitch = 2√3、strip_pitch = 3。REFERENCE POINT (10, −4)。
    /// 単位ベクトル: U = (0, 1)、V = (√3/2, −1/2)、W = (√3/2, 1/2)。
    /// pitchUnit: U = −(W+V).Unit() = −(√3, 0)/√3 = (−1, 0)、
    ///            V = (U+W).Unit() = (√3/2, 3/2)/√3 = (1/2, √3/2)、
    ///            W = (V−U).Unit() = (√3/2, −3/2)/√3 = (1/2, −√3/2)。
    const HEX_90: &str = "\
ANGLES: 90.0 -30.0 30.0
DIAMOND SIZE: 2.0
REFERENCE POINT: 10.0 -4.0
U\t0\t3\t0\t0\t0\t0\t1.5\t2.0\t4.9
V\t0\t1\t0\t0\t0\t1\t-1.0\t1.0\t2
W\t0\t1\t0\t0\t0\t2\t0.0\t-1.0\t1
";

    #[test]
    fn pitch_units_follow_the_tpcreco_formula_for_angles_90_m30_30() {
        let (g, _) = build(HEX_90);
        close(g.pad_pitch, 2.0 * SQRT3, "padPitch");
        close(g.strip_pitch, 3.0, "stripPitch");
        close2(g.planes.u.unit, [0.0, 1.0], "U.unit");
        close2(g.planes.v.unit, [SQRT3 / 2.0, -0.5], "V.unit");
        close2(g.planes.w.unit, [SQRT3 / 2.0, 0.5], "W.unit");
        close2(g.planes.u.pitch_unit, [-1.0, 0.0], "U.pitchUnit");
        close2(g.planes.v.pitch_unit, [0.5, SQRT3 / 2.0], "V.pitchUnit");
        close2(g.planes.w.pitch_unit, [0.5, -SQRT3 / 2.0], "W.pitchUnit");
    }

    #[test]
    fn offsets_and_truncated_lengths_for_angles_90_m30_30() {
        let (g, _) = build(HEX_90);
        // U3: offset = 2·3·(−1, 0) + 1.5·2√3·(0, 1) = (−6, 3√3) → x0 = (4, −4 + 3√3)。
        //     LEN_PADS 4.9 → int 4 → length = 4·2√3 = 8√3。
        let u3 = segment(&g, Plane::U, 3, 0);
        close(u3.x0, 4.0, "U3.x0");
        close(u3.y0, -4.0 + 3.0 * SQRT3, "U3.y0");
        close(
            u3.length,
            8.0 * SQRT3,
            "U3.length (4.9 は 4 pad に切り捨て)",
        );
        // V1: offset = 1·3·(1/2, √3/2) + (−1)·2√3·(√3/2, −1/2) = (1.5, 1.5√3) + (−3, √3)
        //     = (−1.5, 2.5√3) → x0 = (8.5, −4 + 2.5√3)、length = 2·2√3。
        let v1 = segment(&g, Plane::V, 1, 0);
        close(v1.x0, 8.5, "V1.x0");
        close(v1.y0, -4.0 + 2.5 * SQRT3, "V1.y0");
        close(v1.length, 4.0 * SQRT3, "V1.length");
        // W1: offset = (−1)·3·(1/2, −√3/2) = (−1.5, 1.5√3) → x0 = (8.5, −4 + 1.5√3)、length = 2√3。
        let w1 = segment(&g, Plane::W, 1, 0);
        close(w1.x0, 8.5, "W1.x0");
        close(w1.y0, -4.0 + 1.5 * SQRT3, "W1.y0");
        close(w1.length, 2.0 * SQRT3, "W1.length");
    }

    #[test]
    fn range_xy_is_the_bounding_box_of_the_tpcreco_strip_polygons() {
        let (g, _) = build(HEX_90);
        // TPCReco InitTH2Poly は strip ごとに菱形 pad を並べた多角形を TH2Poly の bin にし、
        // SetFloat(true) の軸はその頂点まで広がる。頂点 = p0 + ipad·pad_pitch·unit +
        // {0, d·R(+30°)unit, pad_pitch·unit, d·R(−30°)unit}(d = DIAMOND SIZE = 2、
        // p0 = x0 − 0.5·pad_pitch·unit = 線分の始点)。
        //   U3(unit (0,1)、4 pad): p0 = (4, −4 + 2√3)。d·R(±30°)unit = (∓1, √3) → x は 4 ± 1、
        //       y は p0.y 〜 p0.y + 4·2√3 → [3, 5] × [−4 + 2√3, −4 + 10√3]
        //   V1(unit (√3/2, −1/2)、2 pad): p0 = (7, −4 + 3√3)。pad_pitch·unit = (3, −√3)、
        //       d·R(+30°)unit = (2, 0)、d·R(−30°)unit = (1, −√3)。頂点 (7, −4+3√3) (9, −4+3√3)
        //       (10, −4+2√3) (8, −4+2√3) (12, −4+2√3) (13, −4+√3) (11, −4+√3)
        //       → [7, 13] × [−4 + √3, −4 + 3√3]
        //   W1(unit (√3/2, 1/2)、1 pad): p0 = (7, −4 + √3)。頂点 (7, −4+√3) (8, −4+2√3)
        //       (10, −4+2√3) (9, −4+√3) → [7, 10] × [−4 + √3, −4 + 2√3]
        // → x ∈ [3, 13]、y ∈ [−4 + √3, −4 + 10√3]。線分の外接矩形なら xmin は 4(U3 の菱形が
        //   strip に直交する向きに d/2 = 1 はみ出す分だけ広い)。
        close(g.range_xy[0], 3.0, "xmin");
        close(g.range_xy[1], 13.0, "xmax");
        close(g.range_xy[2], -4.0 + SQRT3, "ymin");
        close(g.range_xy[3], -4.0 + 10.0 * SQRT3, "ymax");
    }

    #[test]
    fn range_xy_follows_the_diamond_corners_for_a_non_hexagonal_angle() {
        // U(11°)の 1 pad だけ。DIAMOND SIZE 2.5 → pad_pitch = 2.5√3。REFERENCE POINT (0, 0)、
        // OFF_PAD 0.5 なので p0 = 0.5·pad_pitch·unit − 0.5·pad_pitch·unit = (0, 0)。
        // 頂点 = (0,0)、2.5·unit(41°)、2.5√3·unit(11°)、2.5·unit(−19°):
        //   xmin = 0、xmax = 2.5√3·cos 11° = 4.25057…、ymin = −2.5·sin 19° = −0.81392…、
        //   ymax = 2.5·sin 41° = 1.64014…
        // (「線分の箱 ± pad_size/2」なら y は [−1.25, 0.826 + 1.25] になって合わない。)
        let text = "\
ANGLES: 11.0 22.0 33.0
DIAMOND SIZE: 2.5
REFERENCE POINT: 0.0 0.0
U\t0\t1\t0\t0\t0\t0\t0.5\t0.0\t1
";
        let (g, _) = build(text);
        close(g.range_xy[0], 0.0, "xmin");
        close(g.range_xy[1], 4.250_570_389_555_222, "xmax");
        close(g.range_xy[2], -0.813_920_386_142_891_7, "ymin");
        close(g.range_xy[3], 1.640_147_572_476_268_2, "ymax");
    }

    // --- 角度 0 / −120 / −60(ELITPC と同じ配置)------------------------------

    /// DIAMOND SIZE 1 → pad_pitch = √3、strip_pitch = 1.5。REFERENCE POINT (3, 2)。
    /// 正規化後 0 / 240 / 300 度: U = (1, 0)、V = (−1/2, −√3/2)、W = (1/2, −√3/2)。
    /// pitchUnit: U = −(W+V).Unit() = −(0, −√3)/√3 = (0, 1)、
    ///            V = (U+W).Unit() = (3/2, −√3/2)/√3 = (√3/2, −1/2)、
    ///            W = (V−U).Unit() = (−3/2, −√3/2)/√3 = (−√3/2, −1/2)。
    const HEX_0: &str = "\
ANGLES: 0.0 -120.0 -60.0
DIAMOND SIZE: 1.0
REFERENCE POINT: 3.0 2.0
U\t0\t1\t0\t0\t0\t0\t0.0\t0.0\t5
V\t2\t5\t0\t0\t0\t1\t2.0\t-3.0\t3
W\t1\t2\t0\t0\t0\t2\t-0.5\t1.0\t2
";

    #[test]
    fn pitch_units_and_offsets_for_angles_0_m120_m60() {
        let (g, _) = build(HEX_0);
        close2(g.planes.u.unit, [1.0, 0.0], "U.unit");
        close2(g.planes.v.unit, [-0.5, -SQRT3 / 2.0], "V.unit");
        close2(g.planes.w.unit, [0.5, -SQRT3 / 2.0], "W.unit");
        close2(g.planes.u.pitch_unit, [0.0, 1.0], "U.pitchUnit");
        close2(g.planes.v.pitch_unit, [SQRT3 / 2.0, -0.5], "V.pitchUnit");
        close2(g.planes.w.pitch_unit, [-SQRT3 / 2.0, -0.5], "W.pitchUnit");
        // V(section 2) 5: offset = −3·1.5·(√3/2, −1/2) + 2·√3·(−1/2, −√3/2)
        //   = (−2.25√3, 2.25) + (−√3, −3) = (−3.25√3, −0.75) → x0 = (3 − 3.25√3, 1.25)、length = 3√3。
        let v5 = segment(&g, Plane::V, 5, 2);
        close(v5.x0, 3.0 - 3.25 * SQRT3, "V5.x0");
        close(v5.y0, 1.25, "V5.y0");
        close(v5.length, 3.0 * SQRT3, "V5.length");
        // W(section 1) 2: offset = 1.5·(−√3/2, −1/2) + (−0.5)·√3·(1/2, −√3/2)
        //   = (−0.75√3, −0.75) + (−0.25√3, 0.75) = (−√3, 0) → x0 = (3 − √3, 2)、length = 2√3。
        let w2 = segment(&g, Plane::W, 2, 1);
        close(w2.x0, 3.0 - SQRT3, "W2.x0");
        close(w2.y0, 2.0, "W2.y0");
        close(w2.length, 2.0 * SQRT3, "W2.length");
    }

    // --- 角度 11 / 22 / 33(六角でない — geometry_mini_reduced.dat の角度)-----

    #[test]
    fn non_hexagonal_angles_11_22_33_keep_the_tpcreco_pitch_formula() {
        // 単位ベクトル 2 本の和 = 2cos((a−b)/2)·(cos((a+b)/2), sin((a+b)/2))、差も同様なので:
        //   U = −(W+V).Unit() = −unit(27.5°) = (−0.88701083…, −0.46174861…)
        //   V = (U+W).Unit()  = unit(22°)    = (0.92718385…, 0.37460659…) ← V の unit そのもの
        //   W = (V−U).Unit()  = unit(106.5°) = (−0.28401534…, 0.95881973…)
        // pitchUnit は unit と直交しない(UI が W 候補を pitch 座標で絞ってはいけない理由)。
        let text = "\
ANGLES: 11.0 22.0 33.0
DIAMOND SIZE: 2.5
REFERENCE POINT: -5.0 6.0
U\t0\t2\t0\t0\t0\t1\t1.0\t0.1\t10
V\t0\t1\t0\t0\t1\t0\t0.0\t0.0\t10
W\t0\t1\t0\t0\t1\t3\t0.0\t0.0\t10
";
        let (g, _) = build(text);
        close2(
            g.planes.u.pitch_unit,
            [-0.887_010_833_178_221_7, -0.461_748_613_235_033_9],
            "U.pitchUnit",
        );
        close2(
            g.planes.v.pitch_unit,
            [0.927_183_854_566_787_4, 0.374_606_593_415_912],
            "V.pitchUnit",
        );
        close2(
            g.planes.w.pitch_unit,
            [-0.284_015_344_703_922_65, 0.958_819_734_868_193],
            "W.pitchUnit",
        );
        let u = &g.planes.u;
        let dot = u.unit[0] * u.pitch_unit[0] + u.unit[1] * u.pitch_unit[1];
        close(
            dot,
            -0.958_819_734_868_193,
            "U.unit·U.pitchUnit = cos(196.5°)(直交しない)",
        );
        // U2: offset = 0.1·3.75·pitchU + 1.0·2.5√3·unit(11°)
        //   = (−0.33262906…, −0.17315573…) + (4.25056968…, 0.82622133…) → x0 = (−1.08205867…, 6.65307146…)
        let u2 = segment(&g, Plane::U, 2, 0);
        close(u2.x0, -1.082_058_672_886_611_7, "U2.x0");
        close(u2.y0, 6.653_071_456_370_238, "U2.y0");
        close(u2.length, 10.0 * 2.5 * SQRT3, "U2.length");
    }

    // --- 行の取捨(TPCReco Load / mapByStrip と同じ)---------------------------

    #[test]
    fn rows_shorter_than_one_pad_are_not_segments_and_are_recorded() {
        // 5 行目 V2: LEN_PADS 0.99 < 1.0 → TPCReco も捨てる。6 行目 W1: ちょうど 1.0 は残る。
        let text = "\
ANGLES: 0.0 -120.0 -60.0
DIAMOND SIZE: 1.0
REFERENCE POINT: 0.0 0.0
U\t0\t1\t0\t0\t0\t0\t0.0\t0.0\t3
V\t0\t2\t0\t0\t0\t1\t0.0\t0.0\t0.99
W\t0\t1\t0\t0\t0\t2\t0.0\t0.0\t1.0
";
        let (g, notes) = build(text);
        assert!(g.planes.v.segments.is_empty(), "V2 は線分にしない");
        assert_eq!(g.planes.w.segments.len(), 1);
        close(g.planes.w.segments[0].length, SQRT3, "W1 は 1 pad");
        assert_eq!(notes.short_rows, vec![5]);
        assert!(notes.replaced_rows.is_empty());
    }

    #[test]
    fn a_repeated_plane_section_strip_keeps_the_later_row() {
        // U(section 1)7 が 4 行目と 6 行目(別 ch)。TPCReco mapByStrip は後勝ち → 6 行目
        // (OFF_PAD 2、LEN_PADS 4)を採り、4 行目を「差し替え」として記録する。
        // 5 行目 U(section 2)7 は別 section なので別の線分。
        let text = "\
ANGLES: 0.0 -120.0 -60.0
DIAMOND SIZE: 1.0
REFERENCE POINT: 0.0 0.0
U\t1\t7\t0\t0\t0\t0\t0.0\t1.0\t9
U\t2\t7\t0\t0\t0\t1\t9.0\t1.0\t5
U\t1\t7\t0\t0\t0\t2\t2.0\t1.0\t4
";
        let (g, notes) = build(text);
        let u7 = segment(&g, Plane::U, 7, 1);
        // offset = 1·1.5·(0, 1) + 2·√3·(1, 0) = (2√3, 1.5)、length = 4√3。
        close(u7.x0, 2.0 * SQRT3, "U7/1.x0 は後の行");
        close(u7.y0, 1.5, "U7/1.y0");
        close(u7.length, 4.0 * SQRT3, "U7/1.length は後の行");
        assert_eq!(g.planes.u.segments.len(), 2);
        assert_eq!(notes.replaced_rows, vec![4]);
    }

    #[test]
    fn segments_are_sorted_by_strip_then_section() {
        // ファイル順はばらばら。出力は strip → section の昇順(契約の「plane → strip → section」)。
        let text = "\
ANGLES: 0.0 -120.0 -60.0
DIAMOND SIZE: 1.0
REFERENCE POINT: 0.0 0.0
V\t2\t3\t0\t0\t0\t0\t0.0\t0.0\t2
V\t0\t10\t0\t0\t0\t1\t0.0\t0.0\t2
V\t1\t3\t0\t0\t0\t2\t0.0\t0.0\t2
V\t0\t2\t0\t0\t0\t3\t0.0\t0.0\t2
";
        let (g, _) = build(text);
        let order: Vec<(u16, u8)> = g
            .planes
            .v
            .segments
            .iter()
            .map(|s| (s.strip, s.section))
            .collect();
        assert_eq!(order, vec![(2, 0), (3, 1), (3, 2), (10, 0)]);
    }

    // --- 作れないとき(WS の `unavailable` になる)-------------------------------

    const ROW: &str = "U\t0\t1\t0\t0\t0\t0\t0.0\t0.0\t3\n";

    #[test]
    fn a_missing_header_key_is_an_error_with_its_reason() {
        let cases = [
            (
                "DIAMOND SIZE: 1.0\nREFERENCE POINT: 0.0 0.0\n",
                StripXyError::MissingAngles,
            ),
            (
                "ANGLES: 0.0 -120.0 -60.0\nREFERENCE POINT: 0.0 0.0\n",
                StripXyError::MissingDiamondSize,
            ),
            (
                "ANGLES: 0.0 -120.0 -60.0\nDIAMOND SIZE: 1.0\n",
                StripXyError::MissingReferencePoint,
            ),
        ];
        for (header, expected) in cases {
            let geometry = geometry::parse(&format!("{header}{ROW}"));
            assert_eq!(strip_geometry(&geometry).unwrap_err(), expected);
        }
        assert_eq!(
            StripXyError::MissingDiamondSize.to_string(),
            "the geometry header has no DIAMOND SIZE"
        );
    }

    #[test]
    fn no_usable_strip_rows_is_an_error() {
        let header = "ANGLES: 0.0 -120.0 -60.0\nDIAMOND SIZE: 1.0\nREFERENCE POINT: 0.0 0.0\n";
        let empty = geometry::parse(header);
        assert_eq!(strip_geometry(&empty).unwrap_err(), StripXyError::NoStrips);
        // LEN_PADS < 1 しかない = 線分 0 本。
        let short = geometry::parse(&format!("{header}U\t0\t1\t0\t0\t0\t0\t0.0\t0.0\t0.5\n"));
        assert_eq!(strip_geometry(&short).unwrap_err(), StripXyError::NoStrips);
    }

    #[test]
    fn parallel_angles_and_a_non_positive_diamond_size_are_rejected_like_tpcreco() {
        // 0 と 180 は [0,360) の 180 で割った余りが同じ(反平行)。−120 と 240 は同じ向き。
        for angles in ["0.0 180.0 60.0", "-120.0 240.0 30.0"] {
            let text =
                format!("ANGLES: {angles}\nDIAMOND SIZE: 1.0\nREFERENCE POINT: 0.0 0.0\n{ROW}");
            assert!(matches!(
                strip_geometry(&geometry::parse(&text)).unwrap_err(),
                StripXyError::ParallelAngles(_)
            ));
        }
        let text =
            format!("ANGLES: 0.0 -120.0 -60.0\nDIAMOND SIZE: 0.0\nREFERENCE POINT: 0.0 0.0\n{ROW}");
        assert_eq!(
            strip_geometry(&geometry::parse(&text)).unwrap_err(),
            StripXyError::NonPositiveDiamondSize(0.0)
        );
    }

    // --- JSON の形(SPEC §10.3 / TODO/125 の WS 契約)----------------------------

    #[test]
    fn json_shape_matches_the_ws_contract() {
        let (g, _) = build(HEX_0);
        let value = serde_json::to_value(&g).unwrap();
        let keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, vec!["padPitch", "planes", "rangeXY", "stripPitch"]);
        assert_eq!(value["rangeXY"].as_array().unwrap().len(), 4);
        for plane in ["U", "V", "W"] {
            let p = &value["planes"][plane];
            assert_eq!(p["unit"].as_array().unwrap().len(), 2, "{plane}.unit");
            assert_eq!(
                p["pitchUnit"].as_array().unwrap().len(),
                2,
                "{plane}.pitchUnit"
            );
        }
        // [strip, section, x0, y0, length] —— V(section 2)5 は上の手計算の値。
        let v = &value["planes"]["V"]["segments"][0];
        assert_eq!(v[0], 5);
        assert_eq!(v[1], 2);
        close(v[2].as_f64().unwrap(), 3.0 - 3.25 * SQRT3, "V5.x0");
        close(v[3].as_f64().unwrap(), 1.25, "V5.y0");
        close(v[4].as_f64().unwrap(), 3.0 * SQRT3, "V5.length");
    }
}
