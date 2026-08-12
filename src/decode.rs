//! CoBo フレームバイト列 → [`msg::Fragment`] へのデコード(純コア、IO なし)。SPEC §2.4。
//!
//! [`framer::Framer`](crate::framer::Framer) が切り出した 1 フレーム分のバイト列を受け取り、
//! 共通ヘッダ + item 列を [`Fragment`] に正規化する。frameType 1(2018、4B item、
//! ch/bucket 明示)と frameType 2(2025 compact、2B item、ch/bucket は AGET 毎カーソルで
//! full-readout の順序から復元)の両方に対応する。
//!
//! フレームレイアウトの正 = C++ 版 tpcdaq(`src/decode/cobo_decoder.cpp`)。

use crate::msg::{items_to_bytes, pack_item, Fragment};
use serde_bytes::ByteBuf;

/// プライマリヘッダを読める最小バイト数(metaType~revision)。
const MIN_FRAME_BYTES: usize = 8;

/// CoBo 共通ヘッダの最小バイト数(フィールドは offset 79+2*4=87 まで使用)。
pub const HEADER_MIN_BYTES: usize = 88;

/// AGET 1 チップあたりの raw チャンネル数(0–67、FPN 込み)。
/// frameType 2 の per-AGET カーソルが一周する周期。
const RAW_CH_PER_AGET: u16 = 68;

/// CoBo フレームバイト列を [`Fragment`] へデコードする(純コア)。
///
/// malformed / unsupported フレームは `decode` が `None` を返し、それぞれのカウンタに
/// 計上する(silent にしない — CLAUDE.md)。frameType ∉ {1,2} は topology 等の制御フレーム
/// として `unsupported`(malformed とは区別 — 生 graw には残る正常系)。
#[derive(Debug, Default)]
pub struct Decoder {
    frames: u64,
    items: u64,
    malformed: u64,
    unsupported: u64,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// 1 フレーム分のバイト列をデコードする。成功時 `Some(Fragment)`、
    /// malformed/unsupported 時 `None`(カウンタは必ず進む)。
    pub fn decode(&mut self, frame: &[u8]) -> Option<Fragment> {
        if frame.len() < MIN_FRAME_BYTES {
            self.malformed += 1;
            return None;
        }

        let meta_type = frame[0];
        let little = meta_type & 0x80 != 0;
        let blk_size: usize = 1usize << (meta_type & 0x0F);

        let frame_type = read_uint(&frame[5..7], little) as u16;
        if frame_type != 1 && frame_type != 2 {
            self.unsupported += 1;
            return None;
        }
        if frame.len() < HEADER_MIN_BYTES {
            self.malformed += 1;
            return None;
        }

        let revision = frame[7];
        let header_size_blk = read_uint(&frame[8..10], little) as usize;
        let item_size = read_uint(&frame[10..12], little) as usize;
        let item_count = read_uint(&frame[12..16], little) as usize;
        let item_start = header_size_blk * blk_size;
        let expected_item_size: usize = if frame_type == 1 { 4 } else { 2 };

        let items_end = item_count
            .checked_mul(item_size)
            .and_then(|len| item_start.checked_add(len));
        let Some(items_end) = items_end else {
            self.malformed += 1;
            return None;
        };

        if item_size != expected_item_size
            || item_start < HEADER_MIN_BYTES
            || items_end > frame.len()
        {
            self.malformed += 1;
            return None;
        }

        let event_time = read_uint(&frame[16..22], little);
        let event_idx = read_uint(&frame[22..26], little) as u32;
        let cobo = frame[26];
        let asad = frame[27];
        let read_offset = read_uint(&frame[28..30], little) as u16;
        let status = frame[30];

        let mut mult = [0u16; 4];
        for (a, m) in mult.iter_mut().enumerate() {
            let off = 67 + a * 2;
            *m = read_uint(&frame[off..off + 2], little) as u16;
        }
        let window_out = read_uint(&frame[75..79], little) as u32;
        let mut last_cell = [0u16; 4];
        for (a, c) in last_cell.iter_mut().enumerate() {
            let off = 79 + a * 2;
            *c = read_uint(&frame[off..off + 2], little) as u16;
        }

        let Some(words) = decode_items(frame_type, &frame[item_start..items_end], little) else {
            self.malformed += 1;
            return None;
        };

        self.frames += 1;
        self.items += words.len() as u64;

        Some(Fragment {
            event_idx,
            event_time,
            cobo,
            asad,
            frame_type: frame_type as u8,
            revision,
            read_offset,
            status,
            mult,
            window_out,
            last_cell,
            items: ByteBuf::from(items_to_bytes(&words)),
        })
    }

    /// 成功裡にデコードしたフレーム数(= 実 graw オラクルの "events")。
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// 成功裡にデコードした item(サンプル)の総数。
    pub fn items(&self) -> u64 {
        self.items
    }

    /// ヘッダ/サイズ不整合でスキップしたフレーム数。
    pub fn malformed(&self) -> u64 {
        self.malformed
    }

    /// frameType ∉ {1,2}(topology 等の制御フレーム)でスキップした数。
    pub fn unsupported(&self) -> u64 {
        self.unsupported
    }
}

/// item バイト列(`item_start..items_end` 済み切り出し)を [`pack_item`] 済み u32 列へ展開する。
/// パック時の範囲エラー(理論上到達しない — bit マスクで既に幅を保証済み)も `None` で拾い、
/// 呼び出し側が malformed として計上できるようにする(panic しない)。
fn decode_items(frame_type: u16, item_bytes: &[u8], little: bool) -> Option<Vec<u32>> {
    let item_count = if frame_type == 1 {
        item_bytes.len() / 4
    } else {
        item_bytes.len() / 2
    };
    let mut words = Vec::with_capacity(item_count);

    if frame_type == 1 {
        // partial(2018): 4B item に aget/chan/bucket/ADC が明示。
        for chunk in item_bytes.chunks_exact(4) {
            let w = read_uint(chunk, little) as u32;
            let aget = ((w >> 30) & 0x3) as u8;
            let chan = ((w >> 23) & 0x7F) as u8;
            let bucket = ((w >> 14) & 0x1FF) as u16;
            let adc = (w & 0xFFF) as u16;
            words.push(pack_item(aget, chan, bucket, adc).ok()?);
        }
    } else {
        // compact(2025): 2B item = aget(bit14,2) + ADC(bit0,12)。ch/bucket は AGET 毎
        // カーソルで full-readout の順序(ch 0..67 → 次 bucket)から復元する。
        // AGET 間インターリーブに耐える(カーソルは aget ごとに独立)。
        let mut chan_cur = [0u16; 4];
        let mut buck_cur = [0u16; 4];
        for chunk in item_bytes.chunks_exact(2) {
            let w = read_uint(chunk, little) as u16;
            let aget = usize::from((w >> 14) & 0x3);
            if chan_cur[aget] >= RAW_CH_PER_AGET {
                chan_cur[aget] = 0;
                buck_cur[aget] += 1;
            }
            let chan = chan_cur[aget] as u8;
            let bucket = buck_cur[aget];
            let adc = w & 0xFFF;
            words.push(pack_item(aget as u8, chan, bucket, adc).ok()?);
            chan_cur[aget] += 1;
        }
    }
    Some(words)
}

/// `bytes` をエンディアン `little` に従って符号なし整数として読む(1–8 バイト)。
fn read_uint(bytes: &[u8], little: bool) -> u64 {
    let mut v: u64 = 0;
    if little {
        for &b in bytes.iter().rev() {
            v = (v << 8) | u64::from(b);
        }
    } else {
        for &b in bytes.iter() {
            v = (v << 8) | u64::from(b);
        }
    }
    v
}

// ---------------------------------------------------------------------
// テスト(仕様書 — 先に書く)。C++ 版 test/test_cobo_decoder.cpp の移植 + big/little 両対応。
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::msg::unpack_item;

    const HEADER_BYTES: usize = 88;

    /// `bytes` へ `value` を `n` バイトの符号なし整数として `little` に従い書き込む
    /// (decode 側 `read_uint` の逆演算。テスト用フィクスチャ組み立て専用)。
    fn write_uint(buf: &mut [u8], offset: usize, value: u64, n: usize, little: bool) {
        let target = &mut buf[offset..offset + n];
        if little {
            for (i, b) in target.iter_mut().enumerate() {
                *b = ((value >> (8 * i)) & 0xFF) as u8;
            }
        } else {
            for (i, b) in target.iter_mut().enumerate() {
                *b = ((value >> (8 * (n - 1 - i))) & 0xFF) as u8;
            }
        }
    }

    #[derive(Default, Clone, Copy)]
    struct HeaderFields {
        revision: u8,
        event_time: u64,
        event_idx: u32,
        cobo: u8,
        asad: u8,
        read_offset: u16,
        status: u8,
        mult: [u16; 4],
        window_out: u32,
        last_cell: [u16; 4],
    }

    /// frameType 1(4B item)の CoBo フレームを手組みする(blkSize=1)。
    fn make_cobo_frame(
        frame_type: u16,
        little: bool,
        h: &HeaderFields,
        items: &[(u8, u8, u16, u16)], // (aget, chan, bucket, adc)
    ) -> Vec<u8> {
        let total = HEADER_BYTES + items.len() * 4;
        let mut b = vec![0u8; total];
        b[0] = if little { 0x80 } else { 0x00 }; // blkSize=1
        write_uint(&mut b, 1, total as u64, 3, little);
        write_uint(&mut b, 5, u64::from(frame_type), 2, little);
        b[7] = h.revision;
        write_uint(&mut b, 8, HEADER_BYTES as u64, 2, little); // headerSize(blk, blkSize=1)
        write_uint(&mut b, 10, 4, 2, little); // itemSize
        write_uint(&mut b, 12, items.len() as u64, 4, little);
        write_uint(&mut b, 16, h.event_time, 6, little);
        write_uint(&mut b, 22, u64::from(h.event_idx), 4, little);
        b[26] = h.cobo;
        b[27] = h.asad;
        write_uint(&mut b, 28, u64::from(h.read_offset), 2, little);
        b[30] = h.status;
        for (a, m) in h.mult.iter().enumerate() {
            write_uint(&mut b, 67 + a * 2, u64::from(*m), 2, little);
        }
        write_uint(&mut b, 75, u64::from(h.window_out), 4, little);
        for (a, c) in h.last_cell.iter().enumerate() {
            write_uint(&mut b, 79 + a * 2, u64::from(*c), 2, little);
        }
        for (i, &(aget, chan, bucket, adc)) in items.iter().enumerate() {
            let w = (u32::from(aget & 0x3) << 30)
                | (u32::from(chan & 0x7F) << 23)
                | (u32::from(bucket & 0x1FF) << 14)
                | u32::from(adc & 0xFFF);
            write_uint(&mut b, HEADER_BYTES + i * 4, u64::from(w), 4, little);
        }
        b
    }

    /// frameType 2(compact、2B item)のフレームを手組みする(blkSize=1)。
    /// `items` = (aget, adc) の生順序(ch/bucket は decode 側が復元する)。
    fn make_compact_frame(little: bool, items: &[(u8, u16)]) -> Vec<u8> {
        let total = HEADER_BYTES + items.len() * 2;
        let mut b = vec![0u8; total];
        b[0] = if little { 0x80 } else { 0x00 };
        write_uint(&mut b, 1, total as u64, 3, little);
        write_uint(&mut b, 5, 2, 2, little); // frameType 2
        write_uint(&mut b, 8, HEADER_BYTES as u64, 2, little);
        write_uint(&mut b, 10, 2, 2, little); // itemSize
        write_uint(&mut b, 12, items.len() as u64, 4, little);
        for (i, &(aget, adc)) in items.iter().enumerate() {
            let w = (u16::from(aget & 0x3) << 14) | (adc & 0xFFF);
            write_uint(&mut b, HEADER_BYTES + i * 2, u64::from(w), 2, little);
        }
        b
    }

    /// 実データと同じ blkSize=256・big-endian のフレームを手組みする
    /// (headerSize=1 block=256B、items は offset 256 から)。
    fn make_blk256_frame(frame_type: u16, item_size_bytes: usize, item_words: &[u64]) -> Vec<u8> {
        const BLK: usize = 256;
        let item_start = BLK;
        let body = item_start + item_words.len() * item_size_bytes;
        let blocks = body.div_ceil(BLK);
        let mut b = vec![0u8; blocks * BLK];
        b[0] = 0x08; // metaType: blkSize=2^8=256, bit7=0=big-endian
        write_uint(&mut b, 1, blocks as u64, 3, false);
        write_uint(&mut b, 5, u64::from(frame_type), 2, false);
        write_uint(&mut b, 8, 1, 2, false); // headerSize=1 block → itemStart=256
        write_uint(&mut b, 10, item_size_bytes as u64, 2, false);
        write_uint(&mut b, 12, item_words.len() as u64, 4, false);
        write_uint(&mut b, 27, 3, 1, false); // asadIdx=3(ヘッダ読みも検証)
        for (i, &w) in item_words.iter().enumerate() {
            write_uint(
                &mut b,
                item_start + i * item_size_bytes,
                w,
                item_size_bytes,
                false,
            );
        }
        b
    }

    // -----------------------------------------------------------------
    // frameType 1 — 実データ符号化(blkSize=256・big-endian)
    // -----------------------------------------------------------------

    /// 手計算の出典: item0 = aget2 chan5 bucket10 adc385、item1 = aget0 chan67 bucket511 adc4095
    /// (C++ 版 test_cobo_decoder.cpp「blkSize=256・big-endian の frameType1」の移植)。
    #[test]
    fn frame_type1_blk256_big_endian_real_encoding_decodes() {
        let w0 = (2u64 << 30) | (5u64 << 23) | (10u64 << 14) | 385u64;
        let w1 = (67u64 << 23) | (511u64 << 14) | 4095u64; // aget=0
        let frame = make_blk256_frame(1, 4, &[w0, w1]);

        let mut dec = Decoder::new();
        let frag = dec.decode(&frame).unwrap();

        assert_eq!(frag.frame_type, 1);
        assert_eq!(frag.asad, 3);
        assert_eq!(dec.frames(), 1);
        assert_eq!(dec.items(), 2);
        assert_eq!(dec.malformed(), 0);

        let words = crate::msg::items_from_bytes(&frag.items).unwrap();
        assert_eq!(words.len(), 2);
        let i0 = unpack_item(words[0]);
        assert_eq!((i0.aget, i0.chan, i0.bucket, i0.adc), (2, 5, 10, 385));
        let i1 = unpack_item(words[1]);
        assert_eq!((i1.aget, i1.chan, i1.bucket, i1.adc), (0, 67, 511, 4095));
    }

    /// 手計算の出典: aget0 の 68ch(bucket0)を敷き詰めた後、69 個目は bucket1 ch0 に一周する。
    /// (C++ 版「blkSize=256・big-endian の frameType2 compact」の移植)。
    #[test]
    fn frame_type2_blk256_big_endian_real_encoding_decodes() {
        let mut words: Vec<u64> = (0..68u64).map(|c| c + 1000).collect(); // aget0 bucket0 ch0..67
        words.push(2000); // aget0 bucket1 ch0
        let frame = make_blk256_frame(2, 2, &words);

        let mut dec = Decoder::new();
        let frag = dec.decode(&frame).unwrap();
        assert_eq!(frag.frame_type, 2);

        let packed = crate::msg::items_from_bytes(&frag.items).unwrap();
        assert_eq!(packed.len(), 69);
        let i0 = unpack_item(packed[0]);
        assert_eq!((i0.bucket, i0.chan, i0.adc), (0, 0, 1000));
        let i67 = unpack_item(packed[67]);
        assert_eq!((i67.bucket, i67.chan, i67.adc), (0, 67, 1067));
        let i68 = unpack_item(packed[68]);
        assert_eq!((i68.bucket, i68.chan, i68.adc), (1, 0, 2000));
    }

    // -----------------------------------------------------------------
    // frameType 2 — per-AGET カーソル、AGET 間インターリーブ耐性(blkSize=1、両エンディアン)
    // -----------------------------------------------------------------

    /// 手計算の出典: aget0 が 68ch(bucket0)を消費した後、bucket1 ch0 に一周。その直後に
    /// aget1 が 1 item 挟まっても aget0 のカーソル(bucket1 ch1 待ち)は独立に継続する
    /// (C++ 版「frameType2 compact: ch/buck を順序から復元(per-AGET カーソル)」の移植)。
    #[test]
    fn frame_type2_aget_cursors_survive_interleaving() {
        for little in [true, false] {
            let mut items: Vec<(u8, u16)> = (0..68u16).map(|c| (0, 1000 + c)).collect(); // aget0 bucket0 ch0..67
            items.push((0, 2000)); // aget0 bucket1 ch0
            items.push((1, 3000)); // aget1 挟み込み(独立カーソル)
            items.push((0, 2001)); // aget0 続き → bucket1 ch1

            let frame = make_compact_frame(little, &items);
            let mut dec = Decoder::new();
            let frag = dec.decode(&frame).unwrap();
            let packed = crate::msg::items_from_bytes(&frag.items).unwrap();
            assert_eq!(packed.len(), 71);

            let i0 = unpack_item(packed[0]);
            assert_eq!((i0.aget, i0.chan, i0.bucket, i0.adc), (0, 0, 0, 1000));
            let i67 = unpack_item(packed[67]);
            assert_eq!((i67.aget, i67.chan, i67.bucket, i67.adc), (0, 67, 0, 1067));
            let i68 = unpack_item(packed[68]);
            assert_eq!((i68.aget, i68.chan, i68.bucket, i68.adc), (0, 0, 1, 2000));
            let i69 = unpack_item(packed[69]);
            assert_eq!((i69.aget, i69.chan, i69.bucket, i69.adc), (1, 0, 0, 3000));
            let i70 = unpack_item(packed[70]);
            assert_eq!((i70.aget, i70.chan, i70.bucket, i70.adc), (0, 1, 1, 2001));
        }
    }

    // -----------------------------------------------------------------
    // frameType 1 — ヘッダのロスレス展開(blkSize=1、両エンディアン)
    // -----------------------------------------------------------------

    /// 手計算の出典: C++ 版「frameType1: ヘッダとアイテムをロスレスに展開」の値をそのまま移植。
    /// FPN raw ch(11)を含む item も落とさず運ぶことを確認する。
    #[test]
    fn frame_type1_header_and_items_roundtrip_losslessly() {
        for little in [true, false] {
            let h = HeaderFields {
                revision: 5,
                event_time: 353_019_082,
                event_idx: 42,
                cobo: 0,
                asad: 3,
                read_offset: 7,
                status: 1,
                mult: [10, 20, 30, 40],
                window_out: 12345,
                last_cell: [100, 200, 300, 400],
            };
            let items = [
                (2u8, 0u8, 0u16, 385u16),
                (3, 11, 5, 446),
                (0, 67, 511, 4095),
            ];
            let frame = make_cobo_frame(1, little, &h, &items);

            let mut dec = Decoder::new();
            let frag = dec.decode(&frame).unwrap();

            assert_eq!(frag.frame_type, 1);
            assert_eq!(frag.revision, 5);
            assert_eq!(frag.event_time, 353_019_082);
            assert_eq!(frag.event_idx, 42);
            assert_eq!(frag.cobo, 0);
            assert_eq!(frag.asad, 3);
            assert_eq!(frag.read_offset, 7);
            assert_eq!(frag.status, 1);
            assert_eq!(frag.mult, [10, 20, 30, 40]);
            assert_eq!(frag.window_out, 12345);
            assert_eq!(frag.last_cell, [100, 200, 300, 400]);

            let packed = crate::msg::items_from_bytes(&frag.items).unwrap();
            assert_eq!(packed.len(), 3);
            let i0 = unpack_item(packed[0]);
            assert_eq!((i0.aget, i0.chan, i0.bucket, i0.adc), (2, 0, 0, 385));
            let i1 = unpack_item(packed[1]);
            assert_eq!((i1.aget, i1.chan, i1.bucket, i1.adc), (3, 11, 5, 446)); // FPN(chan=11)保持
            let i2 = unpack_item(packed[2]);
            assert_eq!((i2.aget, i2.chan, i2.bucket, i2.adc), (0, 67, 511, 4095));
        }
    }

    // -----------------------------------------------------------------
    // unsupported / malformed 系
    // -----------------------------------------------------------------

    #[test]
    fn unknown_frame_type_is_unsupported_not_malformed() {
        let h = HeaderFields::default();
        let frame = make_cobo_frame(5, false, &h, &[]);
        let mut dec = Decoder::new();
        assert!(dec.decode(&frame).is_none());
        assert_eq!(dec.unsupported(), 1);
        assert_eq!(dec.malformed(), 0);
    }

    /// topology 等の非 CoBo 制御フレーム(frameType 7)もスキップされる(malformed とは区別)。
    #[test]
    fn non_cobo_control_frame_is_skipped() {
        let h = HeaderFields::default();
        let frame = make_cobo_frame(7, false, &h, &[]);
        let mut dec = Decoder::new();
        assert!(dec.decode(&frame).is_none());
        assert_eq!(dec.unsupported(), 1);
    }

    /// frameType 2 なのに itemSize が frameType1 用(4)のまま = 不整合 → malformed。
    #[test]
    fn item_size_mismatch_for_frame_type_is_malformed() {
        let h = HeaderFields::default();
        // frameType=2 だが 4B item のフレームを組む(itemSize フィールドは 4 のまま残る)。
        let frame = make_cobo_frame(2, false, &h, &[]);
        let mut dec = Decoder::new();
        assert!(dec.decode(&frame).is_none());
        assert_eq!(dec.malformed(), 1);
        assert_eq!(dec.unsupported(), 0);
    }

    /// item 本体がフレーム長を超える(末尾が削られた)場合は malformed。例外は投げない。
    #[test]
    fn truncated_item_body_is_malformed_without_panicking() {
        let h = HeaderFields::default();
        let mut frame = make_cobo_frame(1, false, &h, &[(1, 1, 1, 1)]);
        let new_len = frame.len() - 2; // 末尾を削ってアイテムを不完全にする
        frame.truncate(new_len);
        let mut dec = Decoder::new();
        assert!(dec.decode(&frame).is_none());
        assert_eq!(dec.malformed(), 1);
    }

    /// headerSize_blk × blkSize が共通ヘッダ最小長(88)未満を示す場合は malformed。
    #[test]
    fn item_start_below_header_minimum_is_malformed() {
        let h = HeaderFields::default();
        let mut frame = make_cobo_frame(1, false, &h, &[(0, 0, 0, 0)]);
        write_uint(&mut frame, 8, 10, 2, false); // headerSize_blk=10(blkSize=1)→ itemStart=10 < 88
        let mut dec = Decoder::new();
        assert!(dec.decode(&frame).is_none());
        assert_eq!(dec.malformed(), 1);
    }

    /// フレーム全体が 8 バイト未満(プライマリヘッダも読めない)は malformed。panic しない。
    #[test]
    fn frame_shorter_than_primary_header_is_malformed() {
        let frame = vec![0u8; 4];
        let mut dec = Decoder::new();
        assert!(dec.decode(&frame).is_none());
        assert_eq!(dec.malformed(), 1);
    }

    /// 共通ヘッダ最小長(88)未満(だが frameType は既知)は malformed。
    #[test]
    fn frame_shorter_than_common_header_is_malformed() {
        let mut frame = vec![0u8; 40];
        write_uint(&mut frame, 5, 1, 2, false); // frameType=1(既知)
        let mut dec = Decoder::new();
        assert!(dec.decode(&frame).is_none());
        assert_eq!(dec.malformed(), 1);
        assert_eq!(dec.unsupported(), 0);
    }

    /// カウンタは複数回の decode 呼び出しをまたいで累積する(構造化ゲッタ、002 と同じ流儀)。
    #[test]
    fn counters_accumulate_across_calls() {
        let h = HeaderFields::default();
        let good = make_cobo_frame(1, false, &h, &[(0, 0, 0, 1), (0, 0, 0, 2)]);
        let bad = make_cobo_frame(5, false, &h, &[]); // unsupported

        let mut dec = Decoder::new();
        assert!(dec.decode(&good).is_some());
        assert!(dec.decode(&bad).is_none());
        assert!(dec.decode(&good).is_some());

        assert_eq!(dec.frames(), 2);
        assert_eq!(dec.items(), 4);
        assert_eq!(dec.unsupported(), 1);
        assert_eq!(dec.malformed(), 0);
    }
}
