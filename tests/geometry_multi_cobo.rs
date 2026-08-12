//! 複数 CoBo(架空 2-CoBo フィクスチャ)の結合テスト。
//! rust_reference の cobo 欠落キーの欠陥(SPEC §4.2)を再発させないことを確認する:
//! 同じ (asad, aget, raw_ch) でも cobo が違えば別チャンネルとして解決されること。
//! フィクスチャ: tests/fixtures/geometry_2cobo_fake.dat(合成、TODO/002)。
#![allow(clippy::unwrap_used)]

use tpcdaq::geometry::{self, ChannelRole, Plane};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/geometry_2cobo_fake.dat"
);

#[test]
fn cobo_count_reflects_both_cobos() {
    let g = geometry::load(FIXTURE).unwrap();
    assert_eq!(g.cobo_count(), 2);
}

#[test]
fn identical_asad_aget_raw_ch_resolve_independently_per_cobo() {
    let g = geometry::load(FIXTURE).unwrap();

    // cobo=0, asad=0, aget=0, raw_ch=0 -> strip 1(U)。
    assert_eq!(
        g.lookup(0, 0, 0, 0),
        ChannelRole::Strip {
            plane: Plane::U,
            section: 0,
            strip: 1
        }
    );
    // 同じ (asad=0,aget=0,raw_ch=0) でも cobo=1 なら別チャンネル(strip 101)。
    // rust_reference の (asad,aget,chan) 3 タプルキーだとここが cobo=0 の
    // strip1 と衝突してしまう(SPEC §4.2 が明示的に直した欠陥)。
    assert_eq!(
        g.lookup(1, 0, 0, 0),
        ChannelRole::Strip {
            plane: Plane::U,
            section: 0,
            strip: 101
        }
    );
    assert_eq!(
        g.lookup(1, 0, 0, 1),
        ChannelRole::Strip {
            plane: Plane::U,
            section: 0,
            strip: 102
        }
    );
}

#[test]
fn asad_count_can_differ_per_cobo() {
    let g = geometry::load(FIXTURE).unwrap();

    // cobo=1, asad=1 は本フィクスチャで使われている(V strip 201)。
    assert_eq!(
        g.lookup(1, 1, 0, 0),
        ChannelRole::Strip {
            plane: Plane::V,
            section: 0,
            strip: 201
        }
    );
    // cobo=0 は asad=0 しか使っていないので、asad=1 は範囲外 = Unmapped。
    assert_eq!(g.lookup(0, 1, 0, 0), ChannelRole::Unmapped);
}

#[test]
fn fpn_is_resolved_independently_for_each_cobo_asad_pair() {
    let g = geometry::load(FIXTURE).unwrap();
    assert_eq!(g.lookup(0, 0, 0, 11), ChannelRole::Fpn { index: 1 });
    assert_eq!(g.lookup(1, 0, 0, 11), ChannelRole::Fpn { index: 1 });
    assert_eq!(g.lookup(1, 1, 0, 11), ChannelRole::Fpn { index: 1 });
}

#[test]
fn fixture_has_no_warnings() {
    let g = geometry::load(FIXTURE).unwrap();
    assert!(g.duplicate_warnings().is_empty());
    assert!(g.malformed_lines().is_empty());
}
