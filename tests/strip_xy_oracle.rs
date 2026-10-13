//! strip 線分の mm を TPCReco GeometryTPC と照合する(TODO/125 レーン A-6)。
//!
//! オラクル = `tools/geometry_xy_oracle`(TPCReco HIGS2026_online の GeometryTPC を直コンパイル)が
//! 書いた `segments.json`。合成ジオメトリ `tests/fixtures/geometry_xy_synth.dat` の分はリポに
//! コミット済み(`make -C tools/geometry_xy_oracle fixtures` で再生成)。
//!
//! 実ジオメトリ(リポに入れない — CLAUDE.md)は env-gated:
//! `TPCDAQ_REAL_GEOMETRY_XY`(実 .dat)と `TPCDAQ_REAL_GEOMETRY_XY_ORACLE`(同じ .dat から
//! オラクルが書いた segments.json)が**両方**あるときだけ走る。

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;
use tpcdaq::geometry::{self, Plane};
use tpcdaq::strip_xy::{self, StripGeometry, StripXyNotes};

const SYNTH_DAT: &str = "tests/fixtures/geometry_xy_synth.dat";
const SYNTH_ORACLE: &str = "tests/fixtures/geometry_xy_oracle/segments.json";

/// 発注書の許容(1e-9 mm)。
const TOL: f64 = 1e-9;

const PLANES: [(Plane, &str); 3] = [(Plane::U, "U"), (Plane::V, "V"), (Plane::W, "W")];

fn build(dat: &Path) -> (StripGeometry, StripXyNotes) {
    strip_xy::strip_geometry(&geometry::load(dat).unwrap()).unwrap()
}

fn read_oracle(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn f(value: &Value) -> f64 {
    value.as_f64().unwrap()
}

/// 照合の結果(件数と最大差 — 結果節に記録する値)。
struct Compared {
    segments: usize,
    max_abs_diff: f64,
}

/// `ours` と TPCReco オラクルの全線分・pitch・rangeXY を突き合わせる(許容 1e-9)。
fn compare_with_oracle(ours: &StripGeometry, oracle: &Value) -> Compared {
    let mut max_abs_diff = 0.0_f64;
    let mut check = |what: &str, actual: f64, expected: f64| {
        let diff = (actual - expected).abs();
        assert!(diff <= TOL, "{what}: ours {actual} vs TPCReco {expected}");
        max_abs_diff = max_abs_diff.max(diff);
    };

    check("padPitch", ours.pad_pitch, f(&oracle["padPitch"]));
    check("stripPitch", ours.strip_pitch, f(&oracle["stripPitch"]));
    for (i, name) in ["xmin", "xmax", "ymin", "ymax"].iter().enumerate() {
        check(name, ours.range_xy[i], f(&oracle["rangeXY"][i]));
    }

    // オラクルの線分を (plane, section, strip) で引けるようにする。
    let mut expected: BTreeMap<(String, u64, u64), &Value> = BTreeMap::new();
    for segment in oracle["segments"].as_array().unwrap() {
        let key = (
            segment[0].as_str().unwrap().to_string(),
            segment[1].as_u64().unwrap(),
            segment[2].as_u64().unwrap(),
        );
        assert!(
            expected.insert(key.clone(), segment).is_none(),
            "oracle dup {key:?}"
        );
    }

    let mut compared = 0;
    for (plane, name) in PLANES {
        let ours_plane = ours.planes.get(plane);
        let oracle_plane = &oracle["planes"][name];
        for k in 0..2 {
            check(
                &format!("{name}.unit[{k}]"),
                ours_plane.unit[k],
                f(&oracle_plane["unit"][k]),
            );
            check(
                &format!("{name}.pitchUnit[{k}]"),
                ours_plane.pitch_unit[k],
                f(&oracle_plane["pitchUnit"][k]),
            );
        }
        for s in &ours_plane.segments {
            let key = (name.to_string(), u64::from(s.section), u64::from(s.strip));
            let Some(e) = expected.remove(&key) else {
                panic!("{key:?}: we made a segment TPCReco does not have");
            };
            // オラクルの列: [plane, section, strip, x0, y0, ux, uy, length]
            check(&format!("{key:?}.x0"), s.x0, f(&e[3]));
            check(&format!("{key:?}.y0"), s.y0, f(&e[4]));
            check(&format!("{key:?}.unit.x"), ours_plane.unit[0], f(&e[5]));
            check(&format!("{key:?}.unit.y"), ours_plane.unit[1], f(&e[6]));
            check(&format!("{key:?}.length"), s.length, f(&e[7]));
            compared += 1;
        }
    }
    assert!(
        expected.is_empty(),
        "TPCReco has segments we did not make: {:?}",
        expected.keys().collect::<Vec<_>>()
    );
    Compared {
        segments: compared,
        max_abs_diff,
    }
}

// =====================================================================
// 合成ジオメトリ(リポ内)
// =====================================================================

#[test]
fn synthetic_geometry_matches_tpcreco_segment_by_segment() {
    let (ours, _) = build(Path::new(SYNTH_DAT));
    let compared = compare_with_oracle(&ours, &read_oracle(Path::new(SYNTH_ORACLE)));
    // 57 本 = U 13 + V(6 + 7×2)+ W(6 + 7×2 + 探針 4)— make_synth_geometry.py の配置。
    assert_eq!(compared.segments, 57);
    eprintln!(
        "synthetic: {} segments, max |ours − TPCReco| = {:e} mm",
        compared.segments, compared.max_abs_diff
    );
}

/// 合成ジオメトリが発注書 A-5 の「確かめたい行」を本当に含んでいる(フィクスチャ自体の仕様)。
#[test]
fn synthetic_geometry_carries_the_rows_the_order_asks_for() {
    let text = std::fs::read_to_string(SYNTH_DAT).unwrap();
    let raw = geometry::parse(&text);
    let (ours, notes) = strip_xy::strip_geometry(&raw).unwrap();
    assert_eq!(
        raw.header.angles_deg,
        Some([0.0, -120.0, -60.0]),
        "ELITPC と同じ角度"
    );

    // V・W に 2 section を持つ strip(V7 / W7 は section 1 と 2)。
    for plane in [Plane::V, Plane::W] {
        let sections: Vec<u8> = ours
            .planes
            .get(plane)
            .segments
            .iter()
            .filter(|s| s.strip == 7)
            .map(|s| s.section)
            .collect();
        assert_eq!(sections, vec![1, 2], "{plane:?}7");
    }

    // LEN_PADS が整数でない行: U2 は 6.7 → 6 pad に切り捨て。
    let u2_row = raw
        .strip_rows()
        .iter()
        .find(|r| r.plane == Plane::U && r.strip == 2)
        .unwrap();
    assert_eq!(u2_row.len_pads, 6.7);
    let u2 = ours
        .planes
        .u
        .segments
        .iter()
        .find(|s| s.strip == 2)
        .unwrap();
    assert!((u2.length - 6.0 * ours.pad_pitch).abs() < 1e-12);

    // LEN_PADS < 1.0 の行(W14)は線分にならず、記録される。
    let w14_line = raw
        .strip_rows()
        .iter()
        .find(|r| r.plane == Plane::W && r.strip == 14)
        .unwrap()
        .line_number;
    assert_eq!(notes.short_rows, vec![w14_line]);
    assert!(!ours.planes.w.segments.iter().any(|s| s.strip == 14));

    // 同じ (plane, section, strip) の重複(U4)は後勝ち: 後の行は OFF_PAD −3.0 / LEN_PADS 7。
    let u4_rows: Vec<_> = raw
        .strip_rows()
        .iter()
        .filter(|r| r.plane == Plane::U && r.strip == 4)
        .collect();
    assert_eq!(u4_rows.len(), 2);
    assert_eq!(notes.replaced_rows, vec![u4_rows[0].line_number]);
    let u4 = ours
        .planes
        .u
        .segments
        .iter()
        .find(|s| s.strip == 4)
        .unwrap();
    assert!(
        (u4.length - 7.0 * ours.pad_pitch).abs() < 1e-12,
        "後の行の長さ"
    );
}

// =====================================================================
// 実ジオメトリ(env-gated、手元のみ)
// =====================================================================

#[test]
fn real_geometry_matches_tpcreco_when_paths_are_given() {
    let (Ok(dat), Ok(oracle)) = (
        std::env::var("TPCDAQ_REAL_GEOMETRY_XY"),
        std::env::var("TPCDAQ_REAL_GEOMETRY_XY_ORACLE"),
    ) else {
        eprintln!(
            "skip: TPCDAQ_REAL_GEOMETRY_XY and TPCDAQ_REAL_GEOMETRY_XY_ORACLE not both set \
             (local-only regression)"
        );
        return;
    };
    let (ours, notes) = build(Path::new(&dat));
    let compared = compare_with_oracle(&ours, &read_oracle(Path::new(&oracle)));
    let json = serde_json::to_string(&ours).unwrap();
    eprintln!(
        "{dat}: {} segments (U {} / V {} / W {}), max |ours − TPCReco| = {:e} mm, \
         stripGeometry JSON {} bytes, short rows {:?}, replaced rows {:?}",
        compared.segments,
        ours.planes.u.segments.len(),
        ours.planes.v.segments.len(),
        ours.planes.w.segments.len(),
        compared.max_abs_diff,
        json.len(),
        notes.short_rows,
        notes.replaced_rows,
    );
}
