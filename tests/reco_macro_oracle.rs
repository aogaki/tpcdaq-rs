//! TODO/127 — **本物の** macros/analyzeUVW.C + makeTracks.C と `tpcdaq::reco` の照合(env-gated)。
//! 125 改訂 2 の `ui/src/app/monitor/track-fit-oracle.spec.ts` の移植(2 モードと許容をそのまま写した)。
//!
//! 入力(どちらも実データ由来 — ローカルのみ、リポに入れない。無ければ skip):
//! - `TPCDAQ_MACRO_ORACLE`: `tools/macro_oracle/run_macro_oracle.sh` の出力 JSON
//!   (raw の adc・uvw の strip ヒット・tracks の全ブランチ・channel_map.csv)
//! - `TPCDAQ_MACRO_ORACLE_GEOMETRY`: 同じ検出器の stripGeometry JSON
//!   (`ws_proto_sample --geometry <.dat> --strip-geometry-out <json>`)
//!
//! オラクル JSON は 100 MB 級なので `cargo test --release --test reco_macro_oracle -- --nocapture` を推奨
//! (`tools/macro_oracle/README.md`)。読み込みと照合は 1 回だけ(`OnceLock`)で、7 本のテストが結果を共有する。
//!
//! adc から 0x02 と同じ格子を組み(channel_map の (aget, ch) → (view, strip)、section 合算 = 実機の
//! monitor と同じ)、本番の [`Reco`] を 2 通りに回す:
//! 1. **production**: s を stripGeometry から(本番どおり)。macros の s は channel_map.csv(小数 3 桁)
//!    なので、トラックの量は s の丸め(≤ 5e-4 mm)の分だけずれうる。
//! 2. **csv-s**: stripGeometry の線分を channel_map.csv の s に置き換えた複製で回す。入力の違いは
//!    macros の出力が float32 で書かれていることだけ → アルゴリズムの移植そのものを細かい許容で見る。

#![allow(clippy::unwrap_used)]

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tpcdaq::monitor::ws;
use tpcdaq::reco::{Reco, RecoGeometry, Track};
use tpcdaq::strip_xy::{PlaneSegments, Planes, Segment, StripGeometry};

/// `[aget, ch, view, strip, s_mm]`
type MapRow = (u32, u32, usize, u16, f64);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Oracle {
    channel_map: Vec<MapRow>,
    events: Vec<OracleEvent>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OracleEvent {
    event_id: u64,
    /// channel_map の行順 × 512 bucket。
    adc: Vec<Vec<u16>>,
    uvw: OracleUvw,
    track: OracleTrack,
}

#[derive(Deserialize)]
struct OracleUvw {
    view: Vec<usize>,
    z: Vec<f64>,
    s: Vec<f64>,
    q: Vec<f64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OracleTrack {
    ok: i32,
    x_start: f64,
    y_start: f64,
    z_start: f64,
    x_end: f64,
    y_end: f64,
    z_end: f64,
    dir_x: f64,
    dir_y: f64,
    dir_z: f64,
    length: f64,
    charge: f64,
    x_start_f: f64,
    y_start_f: f64,
    z_start_f: f64,
    x_end_f: f64,
    y_end_f: f64,
    z_end_f: f64,
    length_f: f64,
}

type Point3 = [f64; 3];

/// float32 の 1 ulp(相対)。macros の slZ / slQ は float で書かれる。
const FLOAT32_REL: f64 = 1.0 / 8_388_608.0; // 2⁻²³

/// 許容(TS の TOL と同じ値)。
/// - ヒットの z・q: float32 の丸め(相対 2⁻²³)。どちらの経路でも同じ(s に依らない)。
/// - トラックの量:
///   - csv-s: 入力の違いは float32 の丸めだけ(s・z ≲ 100 mm → 1 ulp ≈ 7.6e-6 mm)。位置・長さは
///     その数 ulp(2e-5 mm)、方向 1e-6、電荷(float の q の和)は相対 2⁻²³。
///   - production: さらに channel_map.csv の s の丸め(小数 3 桁 → strip ごとに ≤ 5e-4 mm)が入る。
///     位置・長さはその 2 倍(1e-3 mm)、方向は 5e-4 mm / 25 mm の腕 = 2e-5。
struct Tol {
    position: f64,
    direction: f64,
    length: f64,
    charge_rel: f64,
}

const TOL_CSV_S: Tol = Tol {
    position: 2e-5,
    direction: 1e-6,
    length: 2e-5,
    charge_rel: FLOAT32_REL,
};
const TOL_PRODUCTION: Tol = Tol {
    position: 1e-3,
    direction: 2e-5,
    length: 1e-3,
    charge_rel: FLOAT32_REL,
};

#[derive(Default, Debug)]
struct Stats {
    events: usize,
    ok_both: usize,
    not_ok_both: usize,
    /// 両方 ok でなかったイベントの、こちらの理由。
    not_ok_reasons: Vec<String>,
    ok_mismatch: Vec<String>,
    hit_set_mismatch: Vec<String>,
    hits_compared: usize,
    max_z: f64,
    max_q_rel: f64,
    max_position: f64,
    max_direction: f64,
    max_length: f64,
    max_charge_rel: f64,
    worst: String,
    /// `Reco::run` の所要(production モードだけ計る)。
    run_time: Duration,
    run_time_max: Duration,
    /// `Reco::run` + 0x04 のエンコード(monitor のフックと同じ仕事)。
    hook_time: Duration,
    track_bytes_max: usize,
}

struct Results {
    oracle_events: usize,
    max_s_diff: f64,
    production: Stats,
    csv_s: Stats,
}

fn dist(p: Point3, q: Point3) -> f64 {
    ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt()
}

/// 順序を問わない端点の組の差(良い方の対応の、悪い方の端点の距離)。
fn pair_distance(a: [Point3; 2], b: [Point3; 2]) -> f64 {
    let direct = dist(a[0], b[0]).max(dist(a[1], b[1]));
    let swapped = dist(a[0], b[1]).max(dist(a[1], b[0]));
    direct.min(swapped)
}

/// stripGeometry JSON(monitor が配るもの)→ [`StripGeometry`]。
fn parse_strip_geometry(path: &Path) -> StripGeometry {
    let text = std::fs::read_to_string(path).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["type"], "stripGeometry", "{}", path.display());
    assert!(
        v.get("unavailable").is_none(),
        "{} is not a usable stripGeometry: {}",
        path.display(),
        v["unavailable"]
    );
    let f = |x: &serde_json::Value| x.as_f64().unwrap();
    let pair = |x: &serde_json::Value| [f(&x[0]), f(&x[1])];
    let plane = |name: &str| {
        let p = &v["planes"][name];
        PlaneSegments {
            unit: pair(&p["unit"]),
            pitch_unit: pair(&p["pitchUnit"]),
            segments: p["segments"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| Segment {
                    strip: s[0].as_u64().unwrap() as u16,
                    section: s[1].as_u64().unwrap() as u8,
                    x0: f(&s[2]),
                    y0: f(&s[3]),
                    length: f(&s[4]),
                })
                .collect(),
        }
    };
    let range = &v["rangeXY"];
    StripGeometry {
        pad_pitch: f(&v["padPitch"]),
        strip_pitch: f(&v["stripPitch"]),
        range_xy: [f(&range[0]), f(&range[1]), f(&range[2]), f(&range[3])],
        planes: Planes {
            u: plane("U"),
            v: plane("V"),
            w: plane("W"),
        },
    }
}

fn planes_mut(geometry: &mut StripGeometry) -> [&mut PlaneSegments; 3] {
    let Planes { u, v, w } = &mut geometry.planes;
    [u, v, w]
}

fn planes(geometry: &StripGeometry) -> [&PlaneSegments; 3] {
    [&geometry.planes.u, &geometry.planes.v, &geometry.planes.w]
}

/// csv-s: 各 strip の線分を「pitch 座標が channel_map の s」の位置へ(pitch 方向に)置き直す。
fn csv_geometry(geometry: &StripGeometry, channel_map: &[MapRow]) -> StripGeometry {
    let s_of: HashMap<(usize, u16), f64> = channel_map
        .iter()
        .map(|&(_, _, view, strip, s)| ((view, strip), s))
        .collect();
    let mut csv = geometry.clone();
    for (view, plane) in planes_mut(&mut csv).into_iter().enumerate() {
        let [px, py] = plane.pitch_unit;
        for segment in &mut plane.segments {
            if let Some(&s) = s_of.get(&(view, segment.strip)) {
                let shift = s - (px * segment.x0 + py * segment.y0);
                segment.x0 += shift * px;
                segment.y0 += shift * py;
            }
        }
    }
    csv
}

/// 1 イベントの adc(channel_map の行順)→ U/V/W の格子(strip × 512、section 合算 — monitor と同じ
/// saturating add)。`n_strips` = 面ごとの stripGeometry の最大 strip 番号。
fn build_grids(
    event: &OracleEvent,
    channel_map: &[MapRow],
    n_strips: [usize; 3],
    grids: &mut [Vec<u16>; 3],
) -> usize {
    let n_buckets = event.adc[0].len();
    for (grid, n) in grids.iter_mut().zip(n_strips) {
        grid.clear();
        grid.resize(n * n_buckets, 0);
    }
    for (k, &(_, _, view, strip, _)) in channel_map.iter().enumerate() {
        let offset = (strip as usize - 1) * n_buckets;
        for (t, &adc) in event.adc[k].iter().enumerate() {
            let cell = &mut grids[view][offset + t];
            *cell = cell.saturating_add(adc);
        }
    }
    n_buckets
}

fn s_key(view: usize, s: f64) -> (usize, u64) {
    (view, (s + 0.0).to_bits()) // ±0 を同一視(TS の `${s}` と同じ)
}

fn compare(
    stats: &mut Stats,
    tol_name: &str,
    event: &OracleEvent,
    reco: &Reco,
    strip_of_s: &HashMap<(usize, u64), u16>,
) {
    stats.events += 1;
    let label = format!("event {}", event.event_id);

    // ヒット: (view, strip) の集合と、z・q
    let mut ours = HashMap::new();
    for hits in reco.selected() {
        for h in hits {
            ours.insert((h.view as usize, h.strip), *h);
        }
    }
    let mut theirs = HashMap::new();
    for k in 0..event.uvw.view.len() {
        let view = event.uvw.view[k];
        let strip = *strip_of_s
            .get(&s_key(view, event.uvw.s[k]))
            .unwrap_or_else(|| panic!("{label}: slS {} not in the map", event.uvw.s[k]));
        theirs.insert((view, strip), (event.uvw.z[k], event.uvw.q[k]));
    }
    let our_keys: HashSet<_> = ours.keys().copied().collect();
    let their_keys: HashSet<_> = theirs.keys().copied().collect();
    if our_keys != their_keys {
        let extra: Vec<_> = our_keys.difference(&their_keys).collect();
        let missing: Vec<_> = their_keys.difference(&our_keys).collect();
        stats.hit_set_mismatch.push(format!(
            "{label}: ours-only {extra:?} macro-only {missing:?}"
        ));
    }
    for (key, &(z, q)) in &theirs {
        let Some(h) = ours.get(key) else { continue };
        stats.hits_compared += 1;
        stats.max_z = stats.max_z.max((h.z - z).abs());
        stats.max_q_rel = stats.max_q_rel.max((h.q - q).abs() / q);
    }

    // トラック
    let fit = reco.event().fit;
    if (event.track.ok == 1) != fit.is_ok() {
        let ours = match &fit {
            Ok(_) => "ok".to_string(),
            Err(reason) => reason.to_string(),
        };
        stats.ok_mismatch.push(format!(
            "{label} [{tol_name}]: macro ok={}, ours {ours}",
            event.track.ok
        ));
        return;
    }
    let t: Track = match fit {
        Ok(t) => t,
        Err(reason) => {
            stats.not_ok_both += 1;
            stats.not_ok_reasons.push(format!("{label}: {reason}"));
            return;
        }
    };
    stats.ok_both += 1;
    let m = &event.track;
    let position = pair_distance(
        [t.start, t.end],
        [
            [m.x_start, m.y_start, m.z_start],
            [m.x_end, m.y_end, m.z_end],
        ],
    )
    .max(pair_distance(
        [t.start_f, t.end_f],
        [
            [m.x_start_f, m.y_start_f, m.z_start_f],
            [m.x_end_f, m.y_end_f, m.z_end_f],
        ],
    ));
    let dir = [m.dir_x, m.dir_y, m.dir_z];
    let direction = dist(t.dir, dir).min(dist(t.dir, [-dir[0], -dir[1], -dir[2]]));
    let length = (t.length - m.length)
        .abs()
        .max((t.length_f - m.length_f).abs());
    let charge_rel = (t.charge - m.charge).abs() / m.charge;
    if position > stats.max_position {
        stats.worst = format!("{label} (position {position:.2e} mm)");
    }
    stats.max_position = stats.max_position.max(position);
    stats.max_direction = stats.max_direction.max(direction);
    stats.max_length = stats.max_length.max(length);
    stats.max_charge_rel = stats.max_charge_rel.max(charge_rel);
}

fn report(mode: &str, s: &Stats) {
    eprintln!(
        "[TODO/127 macro oracle {mode}] events={} ok(both)={} notOk(both)={} okMismatch={} \
         hitSetMismatch={} hits={} max|dz|={:.2e}mm max|dq|/q={:.2e} max|dpos|={:.2e}mm \
         max|ddir|={:.2e} max|dlen|={:.2e}mm max|dQ|/Q={:.2e} worst: {} notOk: {:?}",
        s.events,
        s.ok_both,
        s.not_ok_both,
        s.ok_mismatch.len(),
        s.hit_set_mismatch.len(),
        s.hits_compared,
        s.max_z,
        s.max_q_rel,
        s.max_position,
        s.max_direction,
        s.max_length,
        s.max_charge_rel,
        s.worst,
        s.not_ok_reasons,
    );
}

fn run_oracle(oracle_path: &str, geometry_path: &str) -> Results {
    let started = Instant::now();
    let oracle: Oracle = serde_json::from_reader(std::io::BufReader::new(
        std::fs::File::open(oracle_path).unwrap(),
    ))
    .unwrap();
    let geometry = parse_strip_geometry(Path::new(geometry_path));
    eprintln!(
        "[TODO/127 macro oracle] loaded {} events in {:.1?}",
        oracle.events.len(),
        started.elapsed()
    );

    // channel_map.csv の s と stripGeometry の s(section 0 の線分の pitchUnit·(x0, y0))
    let mut max_s_diff: f64 = 0.0;
    for &(_, _, view, strip, s) in &oracle.channel_map {
        let plane = planes(&geometry)[view];
        let segment = plane
            .segments
            .iter()
            .find(|g| g.strip == strip && g.section == 0)
            .unwrap_or_else(|| panic!("view {view} strip {strip} has no section-0 segment"));
        let s_geometry = plane.pitch_unit[0] * segment.x0 + plane.pitch_unit[1] * segment.y0;
        max_s_diff = max_s_diff.max((s_geometry - s).abs());
    }
    eprintln!("[TODO/127 macro oracle] max|s_csv - s_geometry| = {max_s_diff:.2e} mm");

    let strip_of_s: HashMap<(usize, u64), u16> = oracle
        .channel_map
        .iter()
        .map(|&(_, _, view, strip, s)| (s_key(view, s), strip))
        .collect();
    let n_strips = planes(&geometry).map(|p| {
        p.segments
            .iter()
            .map(|s| s.strip as usize)
            .max()
            .unwrap_or(0)
    });

    let mut production = Reco::new(RecoGeometry::new(&geometry));
    let mut csv_s = Reco::new(RecoGeometry::new(&csv_geometry(
        &geometry,
        &oracle.channel_map,
    )));
    let mut stats_production = Stats::default();
    let mut stats_csv = Stats::default();
    let mut grids: [Vec<u16>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for event in &oracle.events {
        let n_buckets = build_grids(event, &oracle.channel_map, n_strips, &mut grids);
        let grid_refs = grids.each_ref().map(Vec::as_slice);

        let t0 = Instant::now();
        production.run(grid_refs, n_buckets);
        let ran = t0.elapsed();
        let header = ws::Header::event(ws::TYPE_TRACK, 70, event.event_id as u32, false);
        let bytes = ws::track(header, &production.event()).unwrap();
        let hooked = t0.elapsed();
        stats_production.run_time += ran;
        stats_production.run_time_max = stats_production.run_time_max.max(ran);
        stats_production.hook_time += hooked;
        stats_production.track_bytes_max = stats_production.track_bytes_max.max(bytes.len());
        compare(
            &mut stats_production,
            "production",
            event,
            &production,
            &strip_of_s,
        );

        csv_s.run(grid_refs, n_buckets);
        compare(&mut stats_csv, "csv-s", event, &csv_s, &strip_of_s);
    }
    report("production", &stats_production);
    report("csv-s", &stats_csv);
    let n = stats_production.events.max(1) as u32;
    eprintln!(
        "[TODO/127 macro oracle] cost per event (production, {} build): Reco::run mean {:.1?} \
         max {:.1?}; run + 0x04 encode mean {:.1?}; largest 0x04 {} B",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        stats_production.run_time / n,
        stats_production.run_time_max,
        stats_production.hook_time / n,
        stats_production.track_bytes_max,
    );
    Results {
        oracle_events: oracle.events.len(),
        max_s_diff,
        production: stats_production,
        csv_s: stats_csv,
    }
}

/// 両方の env があれば 1 回だけ読み込んで照合する(無ければ `None` = skip)。
fn results() -> Option<&'static Results> {
    static RESULTS: OnceLock<Option<Results>> = OnceLock::new();
    RESULTS
        .get_or_init(|| {
            let (Ok(oracle), Ok(geometry)) = (
                std::env::var("TPCDAQ_MACRO_ORACLE"),
                std::env::var("TPCDAQ_MACRO_ORACLE_GEOMETRY"),
            ) else {
                eprintln!(
                    "skip: TPCDAQ_MACRO_ORACLE and TPCDAQ_MACRO_ORACLE_GEOMETRY not both set \
                     (local-only regression)"
                );
                return None;
            };
            Some(run_oracle(&oracle, &geometry))
        })
        .as_ref()
}

fn check_agreement(results: &Results, s: &Stats) {
    assert_eq!(s.events, results.oracle_events);
    assert_eq!(s.ok_mismatch, Vec::<String>::new());
    assert_eq!(s.hit_set_mismatch, Vec::<String>::new());
    assert_eq!(s.ok_both + s.not_ok_both, s.events);
}

fn check_hits(s: &Stats) {
    assert!(s.hits_compared > 0);
    // z ≤ 512 cell × 0.1736 mm ≈ 89 mm → 2⁻²³ × 89 ≈ 1.1e-5 mm
    assert!(s.max_z <= FLOAT32_REL * 89.0, "max|dz| = {:e}", s.max_z);
    assert!(s.max_q_rel <= FLOAT32_REL, "max|dq|/q = {:e}", s.max_q_rel);
}

fn check_track(s: &Stats, tol: &Tol) {
    assert!(s.ok_both > 0);
    assert!(s.max_direction <= tol.direction, "{:e}", s.max_direction);
    assert!(
        s.max_position <= tol.position,
        "{:e} ({})",
        s.max_position,
        s.worst
    );
    assert!(s.max_length <= tol.length, "{:e}", s.max_length);
    assert!(s.max_charge_rel <= tol.charge_rel, "{:e}", s.max_charge_rel);
}

#[test]
fn channel_map_s_matches_strip_geometry_s_within_1e_3_mm() {
    let Some(results) = results() else { return };
    assert!(results.max_s_diff <= 1e-3, "{:e}", results.max_s_diff);
}

#[test]
fn production_ok_and_hit_sets_agree_on_every_event() {
    let Some(results) = results() else { return };
    check_agreement(results, &results.production);
}

#[test]
fn production_hit_z_and_q_within_float32_rounding() {
    let Some(results) = results() else { return };
    check_hits(&results.production);
}

#[test]
fn production_track_direction_ends_length_and_charge_within_tolerance() {
    let Some(results) = results() else { return };
    check_track(&results.production, &TOL_PRODUCTION);
}

#[test]
fn csv_s_ok_and_hit_sets_agree_on_every_event() {
    let Some(results) = results() else { return };
    check_agreement(results, &results.csv_s);
}

#[test]
fn csv_s_hit_z_and_q_within_float32_rounding() {
    let Some(results) = results() else { return };
    check_hits(&results.csv_s);
}

#[test]
fn csv_s_track_direction_ends_length_and_charge_within_tolerance() {
    let Some(results) = results() else { return };
    check_track(&results.csv_s, &TOL_CSV_S);
}
