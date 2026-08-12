//! golden fixture(既知値メッセージの `u32 LE 長さ + ペイロード` 連結ストリーム)の
//! 生成と読み戻し。SPEC §10.4 の方式(生成器 = 本番エンコーダ / 検証器 = 別実装)を
//! ZMQ メッセージにも適用するための材料で、将来 root-sink(C++)の照合に使う。
//!
//! 生成物はコミットしない(毎回再生成 — 陳腐化が構造的に起きない)。

#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::PathBuf;

use tpcdaq::msg::{self, Fragments, Message, RawFrames};

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tpcdaq_{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn golden_stream_round_trips_through_a_file_with_the_known_values() {
    let dir = scratch_dir("golden_stream");
    let path = dir.join("msg_golden.bin");

    msg::write_golden_stream(&path).unwrap();
    let bytes = fs::read(&path).unwrap();
    let frames = msg::split_length_prefixed(&bytes).unwrap();
    assert_eq!(
        frames.len(),
        4,
        "golden = Data(raw)/Data(frag)/Heartbeat/EOS"
    );

    // 枠組み: 先頭 4 バイトは 1 通目の長さ(u32 LE)
    assert_eq!(
        u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize,
        frames[0].len()
    );

    // 0: receiver → graw-writer(RawFrames)
    let m0: Message<RawFrames> = Message::from_msgpack(frames[0]).unwrap();
    match m0 {
        Message::Data(batch) => {
            assert_eq!(batch.source_id, 0); // receiver = cobo_id(SPEC §3.2)
            assert_eq!(batch.run_number, 7);
            assert_eq!(batch.sequence_number, 0);
            assert_eq!(batch.created_ns, 1_755_000_000_123_456_789);
            assert_eq!(batch.payload.len(), 2);
            assert_eq!(
                batch.payload[0].as_ref(),
                &[0x08, 0x05, 0x00, 0x01, 0x02, 0x03]
            );
            assert_eq!(batch.payload[1].as_ref(), &[0xff, 0x7f]);
        }
        other => panic!("frame 0 must be Data, got {other:?}"),
    }

    // 1: decoder → root-sink(Fragments)
    let m1: Message<Fragments> = Message::from_msgpack(frames[1]).unwrap();
    match m1 {
        Message::Data(batch) => {
            assert_eq!(batch.source_id, 100); // decoder(SPEC §3.2)
            assert_eq!(batch.run_number, 7);
            assert_eq!(batch.sequence_number, 1);
            assert_eq!(batch.payload.len(), 1);
            let f = &batch.payload[0];
            assert_eq!(f.event_idx, 42);
            assert_eq!(f.event_time, 0x0000_1234_5678_9abc);
            assert_eq!(f.cobo, 0);
            assert_eq!(f.asad, 1);
            assert_eq!(f.frame_type, 2);
            assert_eq!(f.revision, 5);
            assert_eq!(f.read_offset, 136);
            assert_eq!(f.status, 0);
            assert_eq!(f.mult, [68, 0, 17, 3]);
            assert_eq!(f.window_out, 512);
            assert_eq!(f.last_cell, [7, 9, 11, 13]);
            // items = pack(0,0,0,0) / pack(2,45,300,1234) / pack(3,127,511,4095)
            assert_eq!(
                msg::items_from_bytes(f.items.as_ref()).unwrap(),
                vec![0x0000_0000, 0x96cb_04d2, 0xffff_cfff]
            );
        }
        other => panic!("frame 1 must be Data, got {other:?}"),
    }

    // 2/3: Heartbeat と EndOfStream(payload 型に依存しない)
    let m2: Message<RawFrames> = Message::from_msgpack(frames[2]).unwrap();
    assert_eq!(
        m2,
        Message::Heartbeat {
            source_id: 0,
            run_number: 7,
            counter: 3
        }
    );
    let m3: Message<RawFrames> = Message::from_msgpack(frames[3]).unwrap();
    assert_eq!(
        m3,
        Message::EndOfStream {
            source_id: 0,
            run_number: 7
        }
    );
    assert!(!m3.is_droppable());

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn golden_stream_is_deterministic() {
    let dir = scratch_dir("golden_determinism");
    let a = dir.join("a.bin");
    let b = dir.join("b.bin");
    msg::write_golden_stream(&a).unwrap();
    msg::write_golden_stream(&b).unwrap();
    assert_eq!(fs::read(&a).unwrap(), fs::read(&b).unwrap());
    fs::remove_dir_all(&dir).unwrap();
}
