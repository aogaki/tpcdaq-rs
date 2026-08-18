//! GetController 経由 run の実 .graw に対する互換回帰(TODO/067)。
//!
//! - `TPCDAQ_REAL_GRAW_PULSER` = pulser run(**frameType 1**)。MFM オラクル照合(067-B)。
//! - `TPCDAQ_REAL_GRAW_PEDESTAL` = pedestal run(**frameType 2** compact)。
//!   topology + 単一ファイル AsAd インターリーブの実データ確認(067-A/C)。
//!
//! どちらも環境変数が未設定ならその場で return して green のまま終わる
//! (実 .graw はリポに入れない — CLAUDE.md)。
//!
//! # オラクル
//!
//! GET 純正 **MFM ライブラリ**(`reference/_spike/prefix/lib/libMultiFrame.dylib` +
//! `reference/config/CoboFormats.xcfg` → `CoboFormats-Rev-5.xcfg`)でダンプした値。
//! これは CoBoFrameViewer の `CoBoEvent::decodeSamples()`
//! (`reference/20190315_patched/CoBoFrameViewer/src/get/CoBoEvent.cpp:155`)と同じ
//! `mfm::Frame::read` → `itemAt(i).field("").bitField(...)` 経路である。
//! 対象 = `reference/exp_data/2026/pulser/CoBo_2026-08-17T08:09:11.852_0000.graw`
//! (2026-08-18 実測)。
//!
//! SPEC の「frameType 1 = 実データ照合なし・合成のみ」を格上げできる素材はここで確定する
//! (SPEC 本文の改訂は Fable の仕事 — 本ユニットは照合値の固定まで)。
#![allow(clippy::unwrap_used)]

use tpcdaq::decode::{parse_topology, Decoder};
use tpcdaq::framer::Framer;
use tpcdaq::msg::{items_from_bytes, unpack_item, Fragment};

// --- MFM オラクル(2026-08-18 実測、上記ダンプより) ---------------------------

/// topology 1 本 + データフレーム 304 本(= 76 event × 4 AsAd)。
const ORACLE_DATA_FRAMES: u64 = 304;
/// 全フレーム 557,312 B 固定 = ヘッダ 256 B + 139,264 item × 4 B。
const ORACLE_FRAME_BYTES: usize = 557_312;
/// 1 フレームの item 数 = 272 ch(4 AGET × 68)× 512 bucket。
const ORACLE_ITEMS_PER_FRAME: usize = 139_264;
/// FPN の raw チャンネル(SPEC / reuse/rust_reference)。
const FPN_CHANNELS: [usize; 4] = [11, 22, 45, 56];

/// データフレーム 0(asadIdx=0)の先頭 10 item。AGET ラウンドロビンの開始位相は aget=3。
const ORACLE_FRAME0_FIRST_10: [(u8, u8, u16, u16); 10] = [
    (3, 0, 0, 340),
    (0, 0, 0, 372),
    (1, 0, 0, 256),
    (2, 0, 0, 335),
    (3, 1, 0, 275),
    (0, 1, 0, 373),
    (1, 1, 0, 254),
    (2, 1, 0, 329),
    (3, 2, 0, 364),
    (0, 2, 0, 363),
];
/// データフレーム 0 の末尾 5 item。
const ORACLE_FRAME0_LAST_5: [(u8, u8, u16, u16); 5] = [
    (2, 66, 511, 404),
    (3, 67, 511, 391),
    (0, 67, 511, 388),
    (1, 67, 511, 345),
    (2, 67, 511, 336),
];
/// データフレーム 0 の (aget=0, chan=0) 先頭 8 bucket の sample。
const ORACLE_FRAME0_AGET0_CH0: [u16; 8] = [372, 381, 380, 382, 384, 383, 382, 385];

/// データフレーム 1(asadIdx=1)の先頭 10 item。開始位相は aget=2(フレーム毎に違う)。
const ORACLE_FRAME1_FIRST_10: [(u8, u8, u16, u16); 10] = [
    (2, 0, 0, 373),
    (3, 0, 0, 299),
    (0, 0, 0, 396),
    (1, 0, 0, 354),
    (2, 1, 0, 392),
    (3, 1, 0, 299),
    (0, 1, 0, 384),
    (1, 1, 0, 377),
    (2, 2, 0, 386),
    (3, 2, 0, 369),
];
/// データフレーム 1 の (aget=0, chan=0) 先頭 8 bucket の sample。
const ORACLE_FRAME1_AGET0_CH0: [u16; 8] = [396, 406, 409, 405, 405, 407, 409, 407];

/// Fragment の item を (aget, chan, bucket, adc) の並びへ展開する。
fn quads(fragment: &Fragment) -> Vec<(u8, u8, u16, u16)> {
    items_from_bytes(&fragment.items)
        .unwrap()
        .iter()
        .map(|&w| {
            let i = unpack_item(w);
            (i.aget, i.chan, i.bucket, i.adc)
        })
        .collect()
}

#[test]
fn real_pulser_frame_type1_matches_the_mfm_oracle() {
    let Ok(path) = std::env::var("TPCDAQ_REAL_GRAW_PULSER") else {
        eprintln!("skip: TPCDAQ_REAL_GRAW_PULSER not set (local-only regression)");
        return;
    };

    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"));

    let mut framer = Framer::new();
    let mut decoder = Decoder::new();
    let mut topologies = Vec::new();
    let mut fragments = Vec::new(); // 先頭 2 データフレームだけ保持(169 MB を全部持たない)
    let mut asad_sequence = Vec::new();
    let mut frame_sizes = std::collections::BTreeSet::new();
    // 先頭データフレームの hit pattern 生バイト(offset 31..40 = hitPat_0、xcfg より)。
    let mut hit_pattern_0: Option<[u8; 9]> = None;

    for chunk in bytes.chunks(1 << 20) {
        framer.push(chunk);
        while let Some(frame) = framer.next() {
            if let Some(topology) = parse_topology(frame) {
                topologies.push(topology);
                decoder.decode(frame); // カウンタも通す
                continue;
            }
            frame_sizes.insert(frame.len());
            if hit_pattern_0.is_none() {
                hit_pattern_0 = Some(frame[31..40].try_into().unwrap());
            }
            let Some(fragment) = decoder.decode(frame) else {
                panic!(
                    "real pulser frame failed to decode (frame_len={})",
                    frame.len()
                );
            };
            asad_sequence.push(fragment.asad);
            if fragments.len() < 2 {
                fragments.push(fragment);
            }
        }
    }

    eprintln!(
        "pulser oracle: data_frames={} items={} topology={} unsupported={} malformed={} \
         reset_count={} frame_sizes={frame_sizes:?}",
        decoder.frames(),
        decoder.items(),
        decoder.topology(),
        decoder.unsupported(),
        decoder.malformed(),
        framer.reset_count(),
    );

    // --- ストリーム全体 ---------------------------------------------------
    assert_eq!(decoder.malformed(), 0, "malformed オラクル");
    assert_eq!(framer.reset_count(), 0, "再同期は起きない");
    assert_eq!(decoder.frames(), ORACLE_DATA_FRAMES, "データフレーム数");
    assert_eq!(
        decoder.items(),
        ORACLE_DATA_FRAMES * ORACLE_ITEMS_PER_FRAME as u64,
        "item 総数 = 304 × 139,264"
    );
    assert_eq!(
        frame_sizes.iter().copied().collect::<Vec<_>>(),
        vec![ORACLE_FRAME_BYTES],
        "全データフレームが 557,312 B 固定"
    );

    // --- topology frame(TODO/067-A の実データ側) --------------------------
    assert_eq!(decoder.topology(), 1, "topology は _0000 の先頭 1 本だけ");
    assert_eq!(
        decoder.unsupported(),
        1,
        "topology 以外の制御フレームは無い"
    );
    let topology = topologies[0];
    assert_eq!(topology.cobo, 0);
    assert_eq!(topology.asad_mask, 0x0F, "AsAd 0–3 の 4 枚が有効");
    assert!(!topology.two_p_mode);

    // --- AsAd インターリーブ(eventIdx 毎に 0→1→2→3) -----------------------
    let expected_asads: Vec<u8> = (0..ORACLE_DATA_FRAMES).map(|i| (i % 4) as u8).collect();
    assert_eq!(
        asad_sequence, expected_asads,
        "単一ファイルの AsAd インターリーブ順 0,1,2,3 の繰り返し"
    );

    // --- データフレーム 0(asadIdx=0)------------------------------------
    let f0 = &fragments[0];
    assert_eq!(f0.frame_type, 1);
    assert_eq!(f0.revision, 5);
    assert_eq!(f0.cobo, 0);
    assert_eq!(f0.asad, 0);
    assert_eq!(f0.event_idx, 0);
    assert_eq!(f0.event_time, 103_261_370);
    assert_eq!(f0.read_offset, 0);
    assert_eq!(f0.status, 0);
    assert_eq!(f0.mult, [1, 2, 4, 8]);
    assert_eq!(f0.window_out, 0xFFFF_FFFF);
    assert_eq!(f0.last_cell, [690, 690, 690, 690]);

    let q0 = quads(f0);
    assert_eq!(q0.len(), ORACLE_ITEMS_PER_FRAME);
    assert_eq!(&q0[..10], &ORACLE_FRAME0_FIRST_10, "frame 0 先頭 10 item");
    assert_eq!(
        &q0[q0.len() - 5..],
        &ORACLE_FRAME0_LAST_5,
        "frame 0 末尾 5 item"
    );

    // (aget=0, chan=0) の bucket 0..7(item stride 272 = 4 AGET × 68 ch)。
    let aget0_ch0: Vec<u16> = q0
        .iter()
        .filter(|&&(aget, chan, _, _)| aget == 0 && chan == 0)
        .take(8)
        .map(|&(_, _, _, adc)| adc)
        .collect();
    assert_eq!(
        aget0_ch0, ORACLE_FRAME0_AGET0_CH0,
        "frame 0 の (aget0, ch0) 波形先頭 8 bucket"
    );

    // AGET 毎の item 数は均等(34,816 = 68 ch × 512 bucket)、範囲も MFM と一致。
    for aget in 0..4u8 {
        let n = q0.iter().filter(|&&(a, _, _, _)| a == aget).count();
        assert_eq!(n, 68 * 512, "aget {aget} の item 数");
    }
    assert_eq!(q0.iter().map(|q| q.1).min(), Some(0), "chan 最小");
    assert_eq!(q0.iter().map(|q| q.1).max(), Some(67), "chan 最大");
    assert_eq!(q0.iter().map(|q| q.2).min(), Some(0), "bucket 最小");
    assert_eq!(q0.iter().map(|q| q.2).max(), Some(511), "bucket 最大");
    assert_eq!(q0.iter().map(|q| q.3).min(), Some(232), "sample 最小");
    assert_eq!(
        q0.iter().map(|q| q.3).max(),
        Some(4095),
        "sample 最大(飽和)"
    );

    // --- データフレーム 1(asadIdx=1、ラウンドロビン位相が違う)--------------
    let f1 = &fragments[1];
    assert_eq!(f1.asad, 1);
    assert_eq!(f1.event_idx, 0, "同一 event の別 AsAd");
    assert_eq!(f1.event_time, 103_261_370);
    let q1 = quads(f1);
    assert_eq!(q1.len(), ORACLE_ITEMS_PER_FRAME);
    assert_eq!(&q1[..10], &ORACLE_FRAME1_FIRST_10, "frame 1 先頭 10 item");
    let f1_aget0_ch0: Vec<u16> = q1
        .iter()
        .filter(|&&(aget, chan, _, _)| aget == 0 && chan == 0)
        .take(8)
        .map(|&(_, _, _, adc)| adc)
        .collect();
    assert_eq!(f1_aget0_ch0, ORACLE_FRAME1_AGET0_CH0);
    assert_ne!(
        q0[0].0, q1[0].0,
        "ラウンドロビンの開始 AGET はフレーム毎に違う(順序仮定を持たないことの実証)"
    );

    // --- hit pattern の歯抜け = FPN(TODO/067-B で確定させる点)-------------
    //
    // hitPat_0 は offset 31..40 の 9 B big-endian(CoboFormats-Rev-5.xcfg)。
    // MFM の DynamicBitset は bit index 0 = 最終バイト(offset 39)の LSB なので、
    // chan の bit は「後ろから数えて chan/8 バイト目の (chan%8) ビット」。
    // 実測 raw = 1f fe ff df ff ff bf f7 ff。
    let hp = hit_pattern_0.unwrap();
    assert_eq!(
        hp,
        [0x1f, 0xfe, 0xff, 0xdf, 0xff, 0xff, 0xbf, 0xf7, 0xff],
        "hitPat_0 の生バイト"
    );
    let bit = |chan: usize| -> bool {
        let byte = hp[hp.len() - 1 - chan / 8];
        byte & (1 << (chan % 8)) != 0
    };
    let unset: Vec<usize> = (0..68).filter(|&c| !bit(c)).collect();
    assert_eq!(
        unset,
        FPN_CHANNELS.to_vec(),
        "hit pattern の歯抜けは FPN {{11,22,45,56}} と完全一致する"
    );

    // …にもかかわらず **item には FPN ch も入っている**(nItems = 272 × 512 = 全 ch 分)。
    // 「hit pattern は FPN を除く / データは除かない」——これが歯抜けの意味。
    for fpn in FPN_CHANNELS {
        let n = q0
            .iter()
            .filter(|&&(aget, chan, _, _)| aget == 0 && usize::from(chan) == fpn)
            .count();
        assert_eq!(
            n, 512,
            "FPN ch {fpn} も 512 bucket 分そのまま item に入っている(生 ADC 保持)"
        );
    }
}

/// pedestal run(frameType 2 compact = physics と同一形式)でも、単一ファイル先頭の
/// topology frame + AsAd インターリーブをそのまま読み切れること(TODO/067-A/C)。
///
/// 参照値(2026-08-18 実測、`CoBo_2026-08-16T17:37:09.555_0000.graw`):
/// topology 1 + frameType 2 が 1,868 frames(= 467 event × 4 AsAd)、
/// 全フレーム 278,784 B 固定(= ヘッダ 256 B + 139,264 item × 2 B)。
#[test]
fn real_pedestal_frame_type2_stream_is_read_without_loss() {
    let Ok(path) = std::env::var("TPCDAQ_REAL_GRAW_PEDESTAL") else {
        eprintln!("skip: TPCDAQ_REAL_GRAW_PEDESTAL not set (local-only regression)");
        return;
    };

    /// topology 1 + データ 1,868 frames(467 event × 4 AsAd)。
    const ORACLE_PEDESTAL_FRAMES: u64 = 1868;
    /// compact は 2 B/item: 256 + 139,264 × 2 = 278,784 B。
    const ORACLE_PEDESTAL_FRAME_BYTES: usize = 278_784;

    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"));

    let mut framer = Framer::new();
    let mut decoder = Decoder::new();
    let mut topologies = Vec::new();
    let mut arrivals: Vec<(u32, u8)> = Vec::new(); // (eventIdx, asadIdx) を到着順に
    let mut frame_sizes = std::collections::BTreeSet::new();

    for chunk in bytes.chunks(1 << 20) {
        framer.push(chunk);
        while let Some(frame) = framer.next() {
            if let Some(topology) = parse_topology(frame) {
                topologies.push(topology);
                decoder.decode(frame);
                continue;
            }
            frame_sizes.insert(frame.len());
            let Some(fragment) = decoder.decode(frame) else {
                panic!(
                    "real pedestal frame failed to decode (frame_len={})",
                    frame.len()
                );
            };
            arrivals.push((fragment.event_idx, fragment.asad));
        }
    }

    eprintln!(
        "pedestal oracle: data_frames={} items={} topology={} unsupported={} malformed={} \
         reset_count={} frame_sizes={frame_sizes:?}",
        decoder.frames(),
        decoder.items(),
        decoder.topology(),
        decoder.unsupported(),
        decoder.malformed(),
        framer.reset_count(),
    );

    assert_eq!(decoder.malformed(), 0);
    assert_eq!(framer.reset_count(), 0);
    assert_eq!(decoder.frames(), ORACLE_PEDESTAL_FRAMES, "データフレーム数");
    assert_eq!(
        decoder.items(),
        ORACLE_PEDESTAL_FRAMES * ORACLE_ITEMS_PER_FRAME as u64,
        "item 総数 = 1,868 × 139,264"
    );
    assert_eq!(
        frame_sizes.iter().copied().collect::<Vec<_>>(),
        vec![ORACLE_PEDESTAL_FRAME_BYTES],
        "全データフレームが 278,784 B 固定"
    );

    assert_eq!(decoder.topology(), 1, "topology は _0000 の先頭 1 本だけ");
    assert_eq!(decoder.unsupported(), 1);
    assert_eq!(topologies[0].asad_mask, 0x0F, "AsAd 0–3 の 4 枚が有効");

    // --- AsAd インターリーブの実態(TODO/067 の想定と違った点。2026-08-18 実測)------
    //
    // チケットは「eventIdx 毎に asadIdx 0→1→2→3 の順」と書いていたが、pedestal 実データは
    // **その順を常には守らない**:
    //   - event 毎の AsAd 集合は必ず {0,1,2,3} 揃い(欠落も重複も 0 件)。
    //   - ただし到着順が回転している event が 467 中 2 件(#105 = 2,3,0,1 / #345 = 3,0,1,2)。
    //   - eventIdx も単調増加ではない: 隣接 event が混ざる後退が 40 箇所、**後退幅は必ず 1**。
    //     1 event の 4 フレームが占める到着幅は 3(439 event)/ 4(2 event)/ 6(26 event)。
    // pulser 実データ(frameType 1)は逆に 76 event すべてが厳密に 0,1,2,3 だった。
    // → **イベントビルダは AsAd 順にも eventIdx の単調性にも依存してはならない**
    //   (eventIdx でグルーピングする現行実装はこの実データで安全。到着幅 6 は
    //    タイムアウトに対して十分小さい)。
    let mut by_event: std::collections::BTreeMap<u32, Vec<u8>> = std::collections::BTreeMap::new();
    for &(event_idx, asad) in &arrivals {
        by_event.entry(event_idx).or_default().push(asad);
    }
    assert_eq!(by_event.len(), 467, "event 数 = 1,868 / 4");
    for (event_idx, asads) in &by_event {
        let mut sorted = asads.clone();
        sorted.sort_unstable();
        assert_eq!(
            sorted,
            vec![0, 1, 2, 3],
            "event {event_idx} は AsAd 0–3 が 1 枚ずつ揃うこと(欠落も重複も無い)"
        );
    }

    // 到着順が 0,1,2,3 でない event はちょうど 2 件(回転しているだけで集合は同じ)。
    let rotated: Vec<(u32, Vec<u8>)> = by_event
        .iter()
        .filter(|(_, asads)| asads.as_slice() != [0, 1, 2, 3])
        .map(|(e, a)| (*e, a.clone()))
        .collect();
    assert_eq!(
        rotated,
        vec![(105, vec![2, 3, 0, 1]), (345, vec![3, 0, 1, 2])],
        "AsAd 到着順が回転している event(実測)"
    );

    // eventIdx の後退は隣接 event の混在のみ(後退幅は必ず 1)。
    let max_backward_step = arrivals
        .windows(2)
        .filter_map(|w| w[0].0.checked_sub(w[1].0))
        .max()
        .expect("at least one backward step in this file");
    assert_eq!(
        max_backward_step, 1,
        "eventIdx の後退は隣接 event 同士に限られる"
    );
}
