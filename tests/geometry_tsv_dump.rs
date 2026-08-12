//! `dump_tsv`(SPEC §4.5、Rust/C++ 一致テスト用ダンプ。一致テスト自体は後続ユニット)
//! の形式・順序についての結合テスト。
//! フィクスチャ: tests/fixtures/geometry_mini_reduced.dat(合成・縮小版、TODO/002)。
#![allow(clippy::unwrap_used)]

use tpcdaq::geometry;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/geometry_mini_reduced.dat"
);

#[test]
fn dump_covers_every_slot_of_the_dense_array() {
    let g = geometry::load(FIXTURE).unwrap();
    let dump = geometry::dump_tsv(&g);
    // 手計算: cobo_count=1 * asad_count[0]=1 * AGET_CHIPS_PER_ASAD=4 * RAW_CH_PER_AGET=68 = 272。
    assert_eq!(dump.lines().count(), 272);
}

#[test]
fn dump_rows_are_tab_separated_with_eight_columns() {
    let g = geometry::load(FIXTURE).unwrap();
    let dump = geometry::dump_tsv(&g);
    for line in dump.lines() {
        assert_eq!(
            line.split('\t').count(),
            8,
            "expected 8 tab-separated columns in {line:?}"
        );
    }
}

#[test]
fn dump_rows_are_ordered_by_cobo_asad_aget_raw_ch_ascending() {
    let g = geometry::load(FIXTURE).unwrap();
    let dump = geometry::dump_tsv(&g);

    let mut prev: Option<(u32, u32, u32, u32)> = None;
    for line in dump.lines() {
        let cols: Vec<&str> = line.split('\t').collect();
        let key = (
            cols[0].parse::<u32>().unwrap(),
            cols[1].parse::<u32>().unwrap(),
            cols[2].parse::<u32>().unwrap(),
            cols[3].parse::<u32>().unwrap(),
        );
        if let Some(p) = prev {
            assert!(
                key > p,
                "expected strictly ascending order: {p:?} -> {key:?}"
            );
        }
        prev = Some(key);
    }
    assert!(prev.is_some());
}

#[test]
fn dump_first_row_is_the_first_strip_channel() {
    let g = geometry::load(FIXTURE).unwrap();
    let dump = geometry::dump_tsv(&g);
    let first = dump.lines().next().unwrap();
    // cobo=0 asad=0 aget=0 raw_ch=0 -> Strip U section=0 strip=1(フィクスチャの先頭行)。
    assert_eq!(first, "0\t0\t0\t0\tStrip\tU\t0\t1");
}

#[test]
fn dump_fpn_row_has_empty_plane_section_strip_columns() {
    let g = geometry::load(FIXTURE).unwrap();
    let dump = geometry::dump_tsv(&g);
    // aget=0, raw_ch=11 は FPN(index=1)だが、SPEC §4.5 のダンプ列は
    // (role, plane, section, strip) のみ — FPN の index 列は無い。
    assert!(dump.lines().any(|l| l == "0\t0\t0\t11\tFpn\t\t\t"));
}

#[test]
fn dump_aux_row_has_empty_plane_section_strip_columns() {
    let g = geometry::load(FIXTURE).unwrap();
    let dump = geometry::dump_tsv(&g);
    // aget=0, raw_ch=4 は TestAux0(SPEC §4.5 のダンプ列に AUX の name は無い)。
    assert!(dump.lines().any(|l| l == "0\t0\t0\t4\tAux\t\t\t"));
}

#[test]
fn dump_unmapped_row_has_empty_plane_section_strip_columns() {
    let g = geometry::load(FIXTURE).unwrap();
    let dump = geometry::dump_tsv(&g);
    // aget=2 はフィクスチャで一切使われていない(inactive)ので raw_ch=0 は Unmapped。
    assert!(dump.lines().any(|l| l == "0\t0\t2\t0\tUnmapped\t\t\t"));
}
