//! NEW(10 欄)フォーマットの結合テスト。
//! フィクスチャ: tests/fixtures/geometry_mini_reduced.dat(合成・縮小版、TODO/002)。
#![allow(clippy::unwrap_used)]

use tpcdaq::geometry::{self, ChannelRole, Plane};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/geometry_mini_reduced.dat"
);

#[test]
fn header_scalars_are_parsed_verbatim() {
    let g = geometry::load(FIXTURE).unwrap();

    // フィクスチャファイルの値をそのまま手で書き写しただけ(mm 計算はしない — SPEC §4.4)。
    assert_eq!(g.header.angles_deg, Some([11.0, 22.0, 33.0]));
    assert_eq!(g.header.diamond_size_mm, Some(2.5));
    assert_eq!(g.header.reference_point_mm, Some((-5.0, 6.0)));
    assert_eq!(g.header.drift_velocity_cm_per_us, Some(0.6));
    assert_eq!(g.header.sampling_rate_mhz, Some(12.5));
    assert_eq!(g.header.trigger_delay_us, Some(4.5));
    assert_eq!(g.header.drift_cage_acceptance_mm, Some((-40.0, 40.0)));
}

#[test]
fn strip_channels_resolve_to_expected_plane_section_strip() {
    let g = geometry::load(FIXTURE).unwrap();

    // signal_ch 0..4(< 11)は REORDER_FROM_GEOMETRY_TO_GRAW で恒等写像
    // (raw_ch == signal_ch)なので、フィクスチャの AGET_CH をそのまま raw_ch として使える。
    assert_eq!(
        g.lookup(0, 0, 0, 0),
        ChannelRole::Strip {
            plane: Plane::U,
            section: 0,
            strip: 1
        }
    );
    assert_eq!(
        g.lookup(0, 0, 0, 3),
        ChannelRole::Strip {
            plane: Plane::U,
            section: 0,
            strip: 4
        }
    );
    assert_eq!(
        g.lookup(0, 0, 1, 0),
        ChannelRole::Strip {
            plane: Plane::V,
            section: 0,
            strip: 1
        }
    );
    assert_eq!(
        g.lookup(0, 0, 1, 3),
        ChannelRole::Strip {
            plane: Plane::W,
            section: 0,
            strip: 1
        }
    );
    assert_eq!(
        g.lookup(0, 0, 1, 6),
        ChannelRole::Strip {
            plane: Plane::W,
            section: 0,
            strip: 4
        }
    );
}

#[test]
fn aux_channels_resolve_with_their_name() {
    let g = geometry::load(FIXTURE).unwrap();

    assert_eq!(
        g.lookup(0, 0, 0, 4),
        ChannelRole::Aux {
            name: "TestAux0".to_string()
        }
    );
    assert_eq!(
        g.lookup(0, 0, 1, 7),
        ChannelRole::Aux {
            name: "TestAux1".to_string()
        }
    );
}

#[test]
fn fpn_channels_resolve_for_every_active_aget_only() {
    let g = geometry::load(FIXTURE).unwrap();

    // aget=0 と aget=1 はどちらもストリップ/AUX を持つ「active」な AGET なので
    // raw {11,22,45,56} が FPN(index 1..4)になる。
    for aget in 0..2u32 {
        assert_eq!(g.lookup(0, 0, aget, 11), ChannelRole::Fpn { index: 1 });
        assert_eq!(g.lookup(0, 0, aget, 22), ChannelRole::Fpn { index: 2 });
        assert_eq!(g.lookup(0, 0, aget, 45), ChannelRole::Fpn { index: 3 });
        assert_eq!(g.lookup(0, 0, aget, 56), ChannelRole::Fpn { index: 4 });
    }

    // aget=2/3 はフィクスチャに一切登場しないので、FPN スロットも含めて Unmapped。
    assert_eq!(g.lookup(0, 0, 2, 11), ChannelRole::Unmapped);
    assert_eq!(g.lookup(0, 0, 3, 56), ChannelRole::Unmapped);
}

#[test]
fn unmapped_channels_are_unmapped_within_active_aget_too() {
    let g = geometry::load(FIXTURE).unwrap();

    // aget=0 は active だが signal_ch=63(raw63)はどのストリップにも割り当てられていない。
    assert_eq!(g.lookup(0, 0, 0, 63), ChannelRole::Unmapped);
}

#[test]
fn max_strip_matches_fixture_hand_count() {
    let g = geometry::load(FIXTURE).unwrap();
    // 手計算: U は strip 1..4(4 本)、V は 1..3(3 本)、W は 1..4(4 本)。
    assert_eq!(g.max_strip, [4, 3, 4]);
}

#[test]
fn cobo_count_is_one_for_single_cobo_fixture() {
    let g = geometry::load(FIXTURE).unwrap();
    assert_eq!(g.cobo_count(), 1);
}

#[test]
fn clean_fixture_has_no_duplicate_or_malformed_warnings() {
    let g = geometry::load(FIXTURE).unwrap();
    assert!(g.duplicate_warnings().is_empty());
    assert!(g.malformed_lines().is_empty());
}

#[test]
fn load_missing_file_returns_io_error() {
    let missing = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/does-not-exist.dat"
    );
    let err = geometry::load(missing).unwrap_err();
    assert!(matches!(err, geometry::GeometryError::Io { .. }));
}
