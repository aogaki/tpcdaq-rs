//! LEGACY(7 欄)フォーマットの結合テスト。
//! フィクスチャ: tests/fixtures/geometry_legacy.dat(合成、TODO/002)。
#![allow(clippy::unwrap_used)]

use tpcdaq::geometry::{self, ChannelRole, Plane};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/geometry_legacy.dat"
);

#[test]
fn legacy_lines_default_cobo_asad_section_to_zero() {
    let g = geometry::load(FIXTURE).unwrap();

    // LEGACY: DIR STRIP AGET AGET_CH OFF_PAD OFF_STRIP LEN_PADS(section/cobo/asad は 0)。
    // U 1 0 0 ... -> aget=0, signal_ch=0 -> raw_ch=0(< 11 なので恒等写像)。
    assert_eq!(
        g.lookup(0, 0, 0, 0),
        ChannelRole::Strip {
            plane: Plane::U,
            section: 0,
            strip: 1
        }
    );
    // U 2 0 1 ... -> aget=0, signal_ch=1 -> raw_ch=1。
    assert_eq!(
        g.lookup(0, 0, 0, 1),
        ChannelRole::Strip {
            plane: Plane::U,
            section: 0,
            strip: 2
        }
    );
    // V 1 1 0 ... -> aget=1, signal_ch=0 -> raw_ch=0。
    assert_eq!(
        g.lookup(0, 0, 1, 0),
        ChannelRole::Strip {
            plane: Plane::V,
            section: 0,
            strip: 1
        }
    );
    // W 1 1 1 ... -> aget=1, signal_ch=1 -> raw_ch=1。
    assert_eq!(
        g.lookup(0, 0, 1, 1),
        ChannelRole::Strip {
            plane: Plane::W,
            section: 0,
            strip: 1
        }
    );
}

#[test]
fn legacy_fpn_is_still_resolved_for_active_agets() {
    let g = geometry::load(FIXTURE).unwrap();
    assert_eq!(g.lookup(0, 0, 0, 11), ChannelRole::Fpn { index: 1 });
    assert_eq!(g.lookup(0, 0, 1, 56), ChannelRole::Fpn { index: 4 });
}

#[test]
fn legacy_max_strip_matches_hand_count() {
    let g = geometry::load(FIXTURE).unwrap();
    // 手計算: U は strip 1,2(最大 2)。V は strip 1(最大 1)。W は strip 1(最大 1)。
    assert_eq!(g.max_strip, [2, 1, 1]);
}

#[test]
fn legacy_fixture_has_no_warnings() {
    let g = geometry::load(FIXTURE).unwrap();
    assert!(g.duplicate_warnings().is_empty());
    assert!(g.malformed_lines().is_empty());
}
