//! テスト共有の小道具(`#[cfg(test)]` 専用。各サブモジュールの `mod tests` から使う)。

use serde_bytes::ByteBuf;

use super::*;
use crate::msg::Fragment;

// ---- 投入データ(非対称に選ぶ)----

/// 1 サンプル = (aget, raw_ch, bucket, adc)。
pub(super) type Sample = (u8, u8, u16, u16);

pub(super) fn fragment(cobo: u8, asad: u8, samples: &[Sample]) -> Fragment {
    let words: Vec<u32> = samples
        .iter()
        .map(|(aget, chan, bucket, adc)| {
            crate::msg::pack_item(*aget, *chan, *bucket, *adc).unwrap()
        })
        .collect();
    Fragment {
        event_idx: 0,
        event_time: 0x0000_1234_5678_9abc,
        cobo,
        asad,
        frame_type: 2,
        revision: 5,
        read_offset: 136,
        status: 0,
        mult: [68, 0, 17, 3],
        window_out: 512,
        last_cell: [7, 9, 11, 13],
        items: ByteBuf::from(crate::msg::items_to_bytes(&words)),
    }
}

pub(super) fn built_event(
    run: u32,
    event_idx: u32,
    complete: bool,
    fragments: Vec<Fragment>,
) -> BuiltEventPayload {
    BuiltEventPayload {
        kind: "built_event".to_string(),
        run,
        event_idx,
        complete,
        fragments,
    }
}

/// 合成ジオメトリ(実 .dat は使わない — CLAUDE.md)。
///
/// `.dat` の `AGET_CH` は**信号番号 0–63**で、ジオメトリ側が FPN リオーダ
/// (SPEC §4.3)を掛けて raw 0–67 にする。ここでは信号 0–10 しか使わないので
/// **raw == 信号番号**(最初の FPN が raw 11)。
///
/// * U: strip1 = (aget0,raw0)、strip2 = (aget0,raw1) と (aget0,raw2)(**section 違いの合流**)
/// * V: strip1 = (aget1,raw0)
/// * W: strip3 = (aget1,raw5) だけ(strip1/2 は空 = 面の幅は 3)
/// * AUX = (aget0,raw4)、FPN = raw {11,22,45,56}
pub(super) const GEOMETRY_TEXT: &str = "\
ANGLES: 11.0 22.0 33.0
DIAMOND SIZE: 2.5
U\t0\t1\t0\t0\t0\t0\t0.0\t0.0\t10
U\t0\t2\t0\t0\t0\t1\t1.0\t0.1\t10
U\t1\t2\t0\t0\t0\t2\t2.0\t0.2\t10
V\t0\t1\t0\t0\t1\t0\t0.0\t0.0\t10
W\t0\t3\t0\t0\t1\t5\t0.0\t0.0\t10
TestAux0\t0\t0\t0\t4
";

pub(super) fn converter() -> DisplayConverter {
    let geometry = Arc::new(geometry::parse(GEOMETRY_TEXT));
    assert_eq!(geometry.max_strip, [2, 1, 3], "U2/V1/W3(面の幅)");
    DisplayConverter::new(geometry)
}

pub(super) fn hist(id: u8, name: &str, nx: u32, ny: u32, values: &[f64]) -> HistPayload {
    HistPayload {
        id,
        name: name.to_string(),
        nx,
        ny,
        bins: ByteBuf::from(f64_bytes(values)),
    }
}

/// SPEC §2.2 のエンベロープ(positional array(5))+ §5.3 の map 形式ペイロードを
/// 手で組む(root_sink の monitor_pub.hpp と同じバイト構成)。
pub(super) fn envelope(
    source_id: u32,
    run: u32,
    seq: u64,
    created_ns: u64,
    payload: &[u8],
) -> Vec<u8> {
    let mut out = vec![0x81]; // fixmap(1)
    out.extend_from_slice(&[0xa4, b'D', b'a', b't', b'a']); // "Data"
    out.push(0x95); // fixarray(5)
    for value in [u64::from(source_id), u64::from(run), seq, created_ns] {
        out.push(0xcf); // uint64
        out.extend_from_slice(&value.to_be_bytes());
    }
    out.extend_from_slice(payload);
    out
}

pub(super) fn status_payload() -> StatusPayload {
    let mut frames = std::collections::BTreeMap::new();
    frames.insert("0".to_string(), 108u64);
    let mut saturation = std::collections::BTreeMap::new();
    saturation.insert(
        "U".to_string(),
        SaturationPayload {
            saturated: 1,
            counted: 2,
        },
    );
    saturation.insert(
        "V".to_string(),
        SaturationPayload {
            saturated: 0,
            counted: 3,
        },
    );
    saturation.insert(
        "W".to_string(),
        SaturationPayload {
            saturated: 4,
            counted: 5,
        },
    );
    StatusPayload {
        kind: "status".to_string(),
        run: 7,
        state: "running".to_string(),
        events_built: 11,
        events_incomplete: 2,
        late_fragments: 3,
        pending_events: 4,
        frames_per_cobo: frames,
        bytes_written: 123_456,
        saturation,
        publish_drops: 9,
    }
}
