//! 実ジオメトリ回帰(TODO/002_geometry.md やること 6)。
//!
//! `TPCDAQ_REAL_GEOMETRY_MINI` / `TPCDAQ_REAL_GEOMETRY_ELITPC` 環境変数に実 `.dat`
//! へのパスを設定すると走る。未設定ならその場で return して green のまま終わる
//! (実 .dat はリポに入れない — CLAUDE.md)。
//!
//! オラクル値は TODO/002_geometry.md の発注書どおり:
//!   mini   = U72/V92/W92、ストリップ行 256
//!   ELITPC = ストリップ行 1018 + AUX 6、strip 最大 U132/V225/W226
#![allow(clippy::unwrap_used)]

use tpcdaq::geometry;

fn count_rows_with_role(dump: &str, role: &str) -> usize {
    dump.lines()
        .filter(|line| line.split('\t').nth(4) == Some(role))
        .count()
}

#[test]
fn mini_real_geometry_matches_oracle_values() {
    let Ok(path) = std::env::var("TPCDAQ_REAL_GEOMETRY_MINI") else {
        eprintln!("skip: TPCDAQ_REAL_GEOMETRY_MINI not set (local-only regression)");
        return;
    };

    let g = geometry::load(&path).unwrap();
    let dump = geometry::dump_tsv(&g);

    assert_eq!(g.max_strip, [72, 92, 92], "mini U/V/W max strip oracle");
    assert_eq!(
        count_rows_with_role(&dump, "Strip"),
        256,
        "mini total strip row count oracle"
    );
    assert!(
        g.duplicate_warnings().is_empty(),
        "unexpected duplicates: {:?}",
        g.duplicate_warnings()
    );
    assert!(
        g.malformed_lines().is_empty(),
        "unexpected malformed lines: {:?}",
        g.malformed_lines()
    );
}

#[test]
fn elitpc_real_geometry_matches_oracle_values() {
    let Ok(path) = std::env::var("TPCDAQ_REAL_GEOMETRY_ELITPC") else {
        eprintln!("skip: TPCDAQ_REAL_GEOMETRY_ELITPC not set (local-only regression)");
        return;
    };

    let g = geometry::load(&path).unwrap();
    let dump = geometry::dump_tsv(&g);

    assert_eq!(
        g.max_strip,
        [132, 225, 226],
        "ELITPC U/V/W max strip oracle"
    );
    assert_eq!(
        count_rows_with_role(&dump, "Strip"),
        1018,
        "ELITPC total strip row count oracle"
    );
    assert_eq!(
        count_rows_with_role(&dump, "Aux"),
        6,
        "ELITPC AUX row count oracle"
    );
    assert!(
        g.duplicate_warnings().is_empty(),
        "unexpected duplicates: {:?}",
        g.duplicate_warnings()
    );
    assert!(
        g.malformed_lines().is_empty(),
        "unexpected malformed lines: {:?}",
        g.malformed_lines()
    );
}
