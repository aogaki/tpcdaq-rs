//! MFM フレーマ(バイトストリーム → フレーム境界)。純コア、IO なし。SPEC §2.4。
//!
//! MFM プライマリヘッダ(先頭 8 バイト)から 1 フレームの総バイト数を得て切り出す:
//! - byte0 = metaType。bit7 = エンディアン(set=little / clear=big)、bits[3:0] = log2(blkSize)。
//! - bytes1–3 = 24bit frameSize_blk(エンディアンは bit7 に従う)。
//! - 総フレーム長 = frameSize_blk × blkSize。
//!
//! 壊れたヘッダ(フレーム長 0 / ヘッダ長未満 / `max_frame_bytes` 超)は **再同期を試みず**
//! バッファ全体を破棄する([`Framer::reset_count`] に計上)。ホットパスで panic しない。

/// MFM プライマリヘッダのバイト数(byte0–7)。
pub const PRIMARY_HEADER_SIZE: usize = 8;

/// フレーム長上限の既定値(64 MiB)。これを超えるフレーム長を示すヘッダは壊れているとみなす。
pub const DEFAULT_MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

/// 連続バイト列から完成した MFM フレームを境界正しく切り出す。
///
/// `push` でチャンクを追記し、`next` で完成フレームを 1 つずつ取り出す逐次 API。
/// フレームはチャンク境界を跨いでよい(本体が揃うまで内部バッファに保持する)。
///
/// `next` の返り値は内部バッファを指す借用であり、次に `push`/`next`/`reset` を
/// 呼ぶまでの間だけ有効(呼べば borrow checker が古い借用の使用を拒否する)。
#[derive(Debug)]
pub struct Framer {
    buf: Vec<u8>,
    /// 消費済みオフセット。`push` の度に前置部を切り詰める(per-frame 確保を避ける)。
    head: usize,
    max_frame_bytes: usize,
    reset_count: u64,
}

impl Default for Framer {
    fn default() -> Self {
        Self::new()
    }
}

impl Framer {
    /// `max_frame_bytes` は [`DEFAULT_MAX_FRAME_BYTES`]。
    pub fn new() -> Self {
        Self::with_max_frame_bytes(DEFAULT_MAX_FRAME_BYTES)
    }

    pub fn with_max_frame_bytes(max_frame_bytes: usize) -> Self {
        Self {
            buf: Vec::new(),
            head: 0,
            max_frame_bytes,
            reset_count: 0,
        }
    }

    /// 受信したバイトを内部バッファに追記する。
    pub fn push(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        self.compact();
        self.buf.extend_from_slice(data);
    }

    /// 完成フレームが 1 つあれば返す。無ければ `None`(本体未完 = 続きの `push` を待つ)。
    ///
    /// 壊れたヘッダを検出した場合もバッファを破棄したうえで `None` を返す(panic しない)。
    ///
    /// 返り値は内部バッファを指す借用のため `Iterator` は実装できない(streaming/lending
    /// 形。次に `push`/`next`/`reset` を呼ぶまでの間だけ有効)。名前は発注書どおり `next`。
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<&[u8]> {
        let avail = self.buf.len() - self.head;
        if avail < PRIMARY_HEADER_SIZE {
            return None; // プライマリヘッダ未到達
        }

        let header = &self.buf[self.head..self.head + PRIMARY_HEADER_SIZE];
        let frame_bytes = match parse_frame_size(header) {
            Some(n) if n <= self.max_frame_bytes => n,
            _ => {
                // 壊れたヘッダ: 再同期せず破棄する。例外(panic)は投げない。
                self.reset_count += 1;
                self.reset();
                return None;
            }
        };

        if avail < frame_bytes {
            return None; // 本体未完。次の push を待つ
        }

        let start = self.head;
        self.head += frame_bytes;
        Some(&self.buf[start..start + frame_bytes])
    }

    /// バッファを破棄して再同期する(壊れたヘッダ検出時に内部でも呼ばれる)。
    pub fn reset(&mut self) {
        self.buf.clear();
        self.head = 0;
    }

    /// 未消費バイト数。
    pub fn buffered(&self) -> usize {
        self.buf.len() - self.head
    }

    /// 壊れたヘッダを検出してバッファを破棄した累計回数。
    pub fn reset_count(&self) -> u64 {
        self.reset_count
    }

    /// 消費済み前置部を切り詰めてバッファを再利用する。
    fn compact(&mut self) {
        if self.head == 0 {
            return;
        }
        if self.head == self.buf.len() {
            // 全消費: 容量を保ったまま空にする(安価)。
            self.buf.clear();
        } else {
            self.buf.drain(0..self.head);
        }
        self.head = 0;
    }
}

/// プライマリヘッダ(8 バイト以上)からフレーム総バイト数を読む。
/// フレーム長 0 やヘッダ長未満を示す場合は `None`(呼び出し側は壊れ扱いにする)。
fn parse_frame_size(header: &[u8]) -> Option<usize> {
    let meta_type = header[0];
    let little = meta_type & 0x80 != 0;
    let blk_log2 = meta_type & 0x0F;
    let blk_size: usize = 1usize << blk_log2;

    let frame_blk: u32 = if little {
        u32::from(header[1]) | (u32::from(header[2]) << 8) | (u32::from(header[3]) << 16)
    } else {
        (u32::from(header[1]) << 16) | (u32::from(header[2]) << 8) | u32::from(header[3])
    };

    let frame_bytes = frame_blk as usize * blk_size;
    if frame_bytes < PRIMARY_HEADER_SIZE {
        None
    } else {
        Some(frame_bytes)
    }
}

// ---------------------------------------------------------------------
// テスト(仕様書 — 先に書く)。C++ 版 test/test_mfm_framer.cpp の移植。
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// `total_bytes` の MFM フレームを作る。blkSize=1(metaType 下位ニブル=0)、
    /// frameSize_blk = total_bytes を 24bit で格納。残りは識別用パターンで埋める。
    fn make_frame(total_bytes: usize, little: bool, fill: u8) -> Vec<u8> {
        assert!(total_bytes >= PRIMARY_HEADER_SIZE);
        assert!(total_bytes <= 0xFF_FFFF); // 24bit に収まる(blkSize=1)
        let mut f = vec![fill; total_bytes];
        f[0] = if little { 0x80 } else { 0x00 }; // bit7=endian, 下位ニブル0 → blkSize=1
        let blk = total_bytes as u32;
        if little {
            f[1] = (blk & 0xFF) as u8;
            f[2] = ((blk >> 8) & 0xFF) as u8;
            f[3] = ((blk >> 16) & 0xFF) as u8;
        } else {
            f[1] = ((blk >> 16) & 0xFF) as u8;
            f[2] = ((blk >> 8) & 0xFF) as u8;
            f[3] = (blk & 0xFF) as u8;
        }
        f
    }

    #[test]
    fn single_frame_exact_returns_one_then_empty() {
        let mut fr = Framer::new();
        let f = make_frame(32, true, 0xAB);
        fr.push(&f);

        let s = fr.next().unwrap();
        assert_eq!(s.len(), 32);
        assert_eq!(s, f.as_slice()); // バイト完全一致(データ忠実性)

        assert!(fr.next().is_none());
        assert_eq!(fr.buffered(), 0);
    }

    #[test]
    fn split_push_still_assembles() {
        let mut fr = Framer::new();
        let f = make_frame(40, true, 0xAB);

        // 3 + 5 + 残り に分けて push
        fr.push(&f[0..3]);
        assert!(fr.next().is_none()); // ヘッダ未満
        fr.push(&f[3..8]);
        assert!(fr.next().is_none()); // ヘッダは揃ったが本体未完
        fr.push(&f[8..]);

        let s = fr.next().unwrap().to_vec();
        assert_eq!(s, f);
        assert!(fr.next().is_none());
    }

    #[test]
    fn two_concatenated_frames_in_one_push_yield_two_frames() {
        let mut fr = Framer::new();
        let f1 = make_frame(24, true, 0x11);
        let f2 = make_frame(48, true, 0x22);
        let mut both = f1.clone();
        both.extend_from_slice(&f2);
        fr.push(&both);

        let s1 = fr.next().unwrap().to_vec();
        assert_eq!(s1, f1);
        let s2 = fr.next().unwrap().to_vec();
        assert_eq!(s2, f2);
        assert!(fr.next().is_none());
    }

    #[test]
    fn complete_frame_plus_partial_retains_the_remainder() {
        let mut fr = Framer::new();
        let f1 = make_frame(32, true, 0x33);
        let f2 = make_frame(64, true, 0x44);
        let mut chunk = f1.clone();
        chunk.extend_from_slice(&f2[0..20]); // f2 の途中まで
        fr.push(&chunk);

        let s1 = fr.next().unwrap().to_vec();
        assert_eq!(s1, f1);
        assert!(fr.next().is_none()); // f2 未完
        assert_eq!(fr.buffered(), 20);

        fr.push(&f2[20..]); // 残りを供給
        let s2 = fr.next().unwrap().to_vec();
        assert_eq!(s2, f2);
    }

    #[test]
    fn interprets_both_little_and_big_endian() {
        for little in [true, false] {
            let mut fr = Framer::new();
            let f = make_frame(50, little, 0xAB);
            fr.push(&f);
            let s = fr.next().unwrap();
            assert_eq!(s.len(), 50);
        }
    }

    #[test]
    fn corrupt_header_size_zero_resets_without_panicking() {
        let mut fr = Framer::new();
        let bad = vec![0x00u8; 16]; // metaType=0, blk=0 → frameSize=0
        fr.push(&bad);

        assert!(fr.next().is_none()); // panic しない
        assert_eq!(fr.reset_count(), 1);
        assert_eq!(fr.buffered(), 0); // 破棄された
    }

    #[test]
    fn unrealistically_large_size_is_treated_as_corrupt_and_resets() {
        let mut fr = Framer::with_max_frame_bytes(1024);
        let f = make_frame(4096, true, 0xAB); // max=1024 を超える
        fr.push(&f);
        assert!(fr.next().is_none());
        assert_eq!(fr.reset_count(), 1);
    }

    #[test]
    fn accepts_new_frames_after_reset() {
        let mut fr = Framer::new();
        let bad = vec![0x00u8; 16];
        fr.push(&bad);
        fr.next(); // reset される
        assert_eq!(fr.reset_count(), 1);

        let good = make_frame(24, true, 0x55);
        fr.push(&good);
        let s = fr.next().unwrap().to_vec();
        assert_eq!(s, good);
    }

    /// フレーム長がプライマリヘッダ長(8)未満を示す場合も壊れ扱い(reset)。
    #[test]
    fn frame_size_below_header_size_is_treated_as_corrupt() {
        let mut fr = Framer::new();
        // metaType=0(big-endian, blkSize=1)、frameSize_blk=4 → frame_bytes=4 < 8
        let mut bad = vec![0u8; 16];
        bad[0] = 0x00;
        bad[1] = 0x00;
        bad[2] = 0x00;
        bad[3] = 0x04;
        fr.push(&bad);
        assert!(fr.next().is_none());
        assert_eq!(fr.reset_count(), 1);
    }

    /// 実機 CoBo が FDT リンク開設直後に送る topology frame(frameType 7・12 B)を、
    /// 後続のデータフレームを 1 バイトも巻き込まずに切り出せること(TODO/067-A)。
    ///
    /// metaType 0x40 は bit6 が立っているが、フレーム長の解釈に効くのは bit7(endian)と
    /// bits[3:0](log2 blkSize)だけなので **big-endian・blkSize 1** と読める。
    /// 出典 = reference/20190315_patched/GetBench/src/get/daq/MemRead.cpp:362。
    #[test]
    fn real_topology_frame_is_cut_at_12_bytes_without_eating_the_next_frame() {
        let topology: [u8; 12] = [
            0x40, 0x00, 0x00, 0x0c, 0x00, 0x00, 0x07, 0x00, 0x00, 0x0f, 0x00, 0x00,
        ];
        let data = make_frame(96, false, 0xD1);

        let mut fr = Framer::new();
        let mut stream = topology.to_vec();
        stream.extend_from_slice(&data);
        fr.push(&stream);

        let first = fr.next().unwrap().to_vec();
        assert_eq!(first, topology, "topology frame はちょうど 12 B");
        let second = fr.next().unwrap().to_vec();
        assert_eq!(second, data, "後続のデータフレームは無傷");
        assert!(fr.next().is_none());
        assert_eq!(fr.reset_count(), 0, "再同期は起きない");
    }

    /// 実 2025 データの符号化(metaType 0x08 = blkSize 256、big-endian)でも切り出せる。
    #[test]
    fn blk_size_256_big_endian_header_is_parsed() {
        let mut fr = Framer::new();
        let blocks = 2u32; // 512 バイト
        let mut f = vec![0u8; 512];
        f[0] = 0x08; // bit7=0(big), 下位ニブル=8 → blkSize=256
        f[1] = ((blocks >> 16) & 0xFF) as u8;
        f[2] = ((blocks >> 8) & 0xFF) as u8;
        f[3] = (blocks & 0xFF) as u8;
        fr.push(&f);
        let s = fr.next().unwrap();
        assert_eq!(s.len(), 512);
    }
}
