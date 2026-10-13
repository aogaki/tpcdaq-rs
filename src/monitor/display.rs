use std::sync::Arc;

use tracing::warn;

use super::*;
use crate::geometry::{ChannelRole, Geometry, AGET_CHIPS_PER_ASAD, RAW_CH_PER_AGET};
use crate::msg::Fragment;

// =====================================================================
// SPEC §5.4 — 表示変換(IO 非依存の純コア)
// =====================================================================

/// live ストリームの種別(SPEC §10.3 の `subscribe`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Uvw,
    Waveforms,
    Histos,
}

/// 表示変換の副作用カウンタ(silent にしないための可視化フック)。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConvertCounts {
    /// `items` のバイト長が 4 の倍数でなかったフラグメント数。
    pub misaligned_items: u64,
    /// ジオメトリが返した strip 番号がグリッド外だったサンプル数。
    pub strip_out_of_range: u64,
    /// `bins` 長が `nx*ny*8` と食い違ったヒスト数。
    pub malformed_hists: u64,
    /// `unmapped` を新たに引いた回数(ジオメトリ側の累積 — R-P2-13 の可視化フック)。
    pub unmapped_hits: u64,
}

/// built event / hist snapshot を WS バイナリへ変換する(SPEC §5.4)。
///
/// 面グリッドと波形グリッドは**構築時に確保して使い回す**(ホットパスで per-event の
/// heap 確保をしない — CLAUDE.md)。IO は知らないので、そのまま単体テストできる。
pub struct DisplayConverter {
    geometry: Arc<Geometry>,
    /// 面毎の最大ストリップ番号(= グリッドの x 幅)。
    n_strips: [u16; N_PLANES],
    /// 面毎 `nStrips × N_BUCKETS` の再利用バッファ。
    grids: [Vec<u16>; N_PLANES],
    /// `nAget × nCh × N_BUCKETS` の再利用バッファ。
    waveform: Vec<u16>,
    counts: ConvertCounts,
    /// ジオメトリの `unmapped_hit_count` の既読値(増分検出用)。
    unmapped_seen: u64,
    /// unmapped の warn は**初回だけ**(ホットパスでログ整形しない — CLAUDE.md)。
    logged_unmapped: bool,
    logged_misaligned: bool,
    logged_strip_range: bool,
    logged_malformed_hist: bool,
}

impl DisplayConverter {
    pub fn new(geometry: Arc<Geometry>) -> Self {
        let n_strips = geometry.max_strip;
        let grids = [
            vec![0u16; n_strips[0] as usize * N_BUCKETS as usize],
            vec![0u16; n_strips[1] as usize * N_BUCKETS as usize],
            vec![0u16; n_strips[2] as usize * N_BUCKETS as usize],
        ];
        let waveform =
            vec![0u16; (AGET_CHIPS_PER_ASAD * RAW_CH_PER_AGET) as usize * N_BUCKETS as usize];
        let unmapped_seen = geometry.unmapped_hit_count();
        Self {
            geometry,
            n_strips,
            grids,
            waveform,
            counts: ConvertCounts::default(),
            unmapped_seen,
            logged_unmapped: false,
            logged_misaligned: false,
            logged_strip_range: false,
            logged_malformed_hist: false,
        }
    }

    /// 変換の副作用カウンタ。
    pub fn counts(&self) -> ConvertCounts {
        self.counts
    }

    /// built event → `0x02 Uvw` ×3(SPEC §10.2)。面毎 `nStrips×512` の u16 グリッド。
    ///
    /// - 同一ビンへの複数チャンネル(セクション合流)は **saturating add**(u16 天井で clamp。
    ///   表示専用で、正値はヒスト / monitor.root 側にある — SPEC §5.2)。
    /// - FPN / Aux / Unmapped は入れない(SPEC §5.2 のヒストと同じ扱い)。
    pub fn uvw(&mut self, event: &BuiltEventPayload, out: &mut Vec<Vec<u8>>) {
        for grid in &mut self.grids {
            grid.fill(0);
        }
        for fragment in &event.fragments {
            self.accumulate_uvw(fragment);
        }
        self.note_unmapped();

        let header = ws::Header::event(ws::TYPE_UVW, event.run, event.event_idx, !event.complete);
        for (plane, grid) in self.grids.iter().enumerate() {
            out.push(ws::uvw(
                header,
                plane as u8,
                self.n_strips[plane],
                N_BUCKETS,
                grid,
            ));
        }
    }

    fn accumulate_uvw(&mut self, fragment: &Fragment) {
        let cobo = u32::from(fragment.cobo);
        let asad = u32::from(fragment.asad);
        for word in ItemWords::new(
            &fragment.items,
            &mut self.counts,
            &mut self.logged_misaligned,
        ) {
            let item = crate::msg::unpack_item(word);
            // 参照で引く(`Aux { name: String }` の clone を per-sample で起こさない)。
            let role =
                self.geometry
                    .lookup_ref(cobo, asad, u32::from(item.aget), u32::from(item.chan));
            let &ChannelRole::Strip { plane, strip, .. } = role else {
                continue; // FPN / Aux / Unmapped は表示グリッドに入れない
            };
            let p = plane as usize;
            let n_strips = self.n_strips[p] as usize;
            if strip == 0 || strip as usize > n_strips {
                self.counts.strip_out_of_range += 1;
                if !self.logged_strip_range {
                    self.logged_strip_range = true;
                    warn!(
                        plane = plane.as_str(),
                        strip,
                        n_strips,
                        "monitor: strip number outside the geometry grid — skipped"
                    );
                }
                continue;
            }
            let idx = (strip as usize - 1) * N_BUCKETS as usize + item.bucket as usize;
            if let Some(cell) = self.grids[p].get_mut(idx) {
                // セクション合流は加算。u16 の天井で clamp する(表示専用)。
                *cell = cell.saturating_add(item.adc);
            }
        }
    }

    /// built event → `0x03 Waveforms`(SPEC §10.2)。(cobo,asad) 毎に
    /// `nAget×nCh×512` の dense グリッド、aget-major・raw ch 順・**FPN 込み・減算なし**(R13)。
    pub fn waveforms(&mut self, event: &BuiltEventPayload, out: &mut Vec<Vec<u8>>) {
        let header = ws::Header::event(
            ws::TYPE_WAVEFORMS,
            event.run,
            event.event_idx,
            !event.complete,
        );
        for fragment in &event.fragments {
            self.waveform.fill(0);
            for word in ItemWords::new(
                &fragment.items,
                &mut self.counts,
                &mut self.logged_misaligned,
            ) {
                let item = crate::msg::unpack_item(word);
                if u32::from(item.aget) >= AGET_CHIPS_PER_ASAD
                    || u32::from(item.chan) >= RAW_CH_PER_AGET
                {
                    continue; // §2.4 のビット幅は raw 0–127 を許すが、AsAd は 0–67 しか無い
                }
                let idx = (item.aget as usize * RAW_CH_PER_AGET as usize + item.chan as usize)
                    * N_BUCKETS as usize
                    + item.bucket as usize;
                if let Some(cell) = self.waveform.get_mut(idx) {
                    *cell = item.adc; // 生 ADC(減算なし)。1 (ch,bucket) は 1 サンプル
                }
            }
            out.push(ws::waveforms(
                header,
                fragment.cobo,
                fragment.asad,
                AGET_CHIPS_PER_ASAD as u8,
                RAW_CH_PER_AGET as u8,
                N_BUCKETS,
                &self.waveform,
            ));
        }
    }

    /// hist snapshot → `0x10 Histo1d` / `0x11 Histo2d`(SPEC §10.2)。
    ///
    /// 軸は SPEC §5.2 の定義そのもの: 2D は x=strip `[1, N+1)` / y=bucket `[0, 512)`、
    /// 1D は波高 `[0, 4096)`。ビン値は f64 → f32(表示専用)。
    pub fn histos(&mut self, snapshot: &HistSnapshotPayload, out: &mut Vec<Vec<u8>>) {
        for hist in &snapshot.hists {
            if !hist.is_consistent() {
                self.counts.malformed_hists += 1;
                if !self.logged_malformed_hist {
                    self.logged_malformed_hist = true;
                    warn!(
                        name = hist.name,
                        id = hist.id,
                        nx = hist.nx,
                        ny = hist.ny,
                        bins = hist.bins.len(),
                        "monitor: hist bins length does not match nx*ny*8 — skipped"
                    );
                }
                continue;
            }
            let id = u16::from(hist.id);
            if hist.ny > 1 {
                let header = ws::Header::hist(ws::TYPE_HISTO2D, snapshot.run);
                out.push(ws::histo2d(
                    header,
                    id,
                    hist.nx as u16,
                    hist.ny as u16,
                    ws::Axis {
                        min: 1.0,
                        max: hist.nx as f32 + 1.0,
                    },
                    ws::Axis {
                        min: 0.0,
                        max: hist.ny as f32,
                    },
                    &hist.bins,
                ));
            } else {
                let header = ws::Header::hist(ws::TYPE_HISTO1D, snapshot.run);
                out.push(ws::histo1d(
                    header,
                    id,
                    ws::Axis {
                        min: 0.0,
                        max: CHARGE_AXIS_MAX,
                    },
                    &hist.bins,
                ));
            }
        }
    }

    /// R-P2-13 の可視化フック: 変換で `Unmapped` を引いていたら**初回だけ** warn する
    /// (カウンタは常に進むので silent にはならない)。
    fn note_unmapped(&mut self) {
        let hits = self.geometry.unmapped_hit_count();
        if hits <= self.unmapped_seen {
            return;
        }
        self.counts.unmapped_hits = hits;
        self.unmapped_seen = hits;
        if !self.logged_unmapped {
            self.logged_unmapped = true;
            warn!(
                unmapped_hits = hits,
                "monitor: geometry lookup returned Unmapped during display conversion \
                 (channels absent from the .dat — check the geometry against the hardware)"
            );
        }
    }
}

/// 波高ヒスト(1D)の x レンジ上限(SPEC §5.2「[0,4096] 固定、オートレンジ禁止」)。
pub(super) const CHARGE_AXIS_MAX: f32 = 4096.0;

/// `Fragment.items`(u32 LE 連結)を**中間 Vec を作らずに**なめるイテレータ。
///
/// [`crate::msg::items_from_bytes`] は `Vec<u32>` を確保するのでホットパスでは使わない。
/// 4 の倍数でない端数は「壊れたフラグメント」としてカウント + 初回だけ warn する。
pub(super) struct ItemWords<'a> {
    chunks: std::slice::ChunksExact<'a, u8>,
}

impl<'a> ItemWords<'a> {
    pub(super) fn new(items: &'a [u8], counts: &mut ConvertCounts, logged: &mut bool) -> Self {
        let chunks = items.chunks_exact(4);
        if !chunks.remainder().is_empty() {
            counts.misaligned_items += 1;
            if !*logged {
                *logged = true;
                warn!(
                    len = items.len(),
                    "monitor: fragment items length is not a multiple of 4 — tail ignored"
                );
            }
        }
        Self { chunks }
    }
}

impl Iterator for ItemWords<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        let c = self.chunks.next()?;
        Some(u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::monitor::test_util::*;
    use std::fmt::Write as _;

    // ---- ワイヤ読み出しの小道具(バイトオフセットを明示的に扱う)----

    fn u16_at(bytes: &[u8], at: usize) -> u16 {
        u16::from_le_bytes([bytes[at], bytes[at + 1]])
    }

    fn u32_at(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
    }

    fn f32_at(bytes: &[u8], at: usize) -> f32 {
        f32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
    }

    // -----------------------------------------------------------------
    // 1. 13 B ヘッダのバイトオフセット(SPEC §10.1 / §10.4-4)
    // -----------------------------------------------------------------

    #[test]
    fn header_layout_matches_the_spec_byte_table() {
        // SPEC §10.1: 0-1 magic 'T''P' / 2 msgType / 3 version=2 / 4 flags(bit0=incomplete)
        //             / 5-8 u32 runNumber / 9-12 u32 eventNumber(LE)
        let mut out = Vec::new();
        ws::Header::event(ws::TYPE_UVW, 0x0A0B_0C0D, 0x0102_0304, true).write(&mut out);

        assert_eq!(out.len(), 13, "ヘッダは 13 バイト");
        assert_eq!(ws::HEADER_LEN, 13);
        assert_eq!(out[0], b'T', "off 0 = 'T'");
        assert_eq!(out[1], b'P', "off 1 = 'P'");
        assert_eq!(out[2], 0x02, "off 2 = msgType");
        assert_eq!(out[3], 2, "off 3 = version 2");
        assert_eq!(out[4], 0x01, "off 4 bit0 = incomplete");
        assert_eq!(u32_at(&out, 5), 0x0A0B_0C0D, "off 5..9 = runNumber(LE)");
        assert_eq!(u32_at(&out, 9), 0x0102_0304, "off 9..13 = eventNumber(LE)");
        // LE であることをバイトの並びでも押さえる(取り違えると UI 側が黙って壊れる)。
        assert_eq!(&out[5..9], &[0x0D, 0x0C, 0x0B, 0x0A]);
        assert_eq!(&out[9..13], &[0x04, 0x03, 0x02, 0x01]);

        // 完全なイベントは flags = 0
        let mut complete = Vec::new();
        ws::Header::event(ws::TYPE_WAVEFORMS, 1, 2, false).write(&mut complete);
        assert_eq!(complete[4], 0x00);
        assert_eq!(complete[2], 0x03);

        // ヒストは flags = 0・eventNumber = 0(SPEC §10.1)
        let mut hist = Vec::new();
        ws::Header::hist(ws::TYPE_HISTO2D, 9).write(&mut hist);
        assert_eq!(hist[2], 0x11);
        assert_eq!(hist[4], 0x00);
        assert_eq!(u32_at(&hist, 5), 9);
        assert_eq!(u32_at(&hist, 9), 0, "ヒストの eventNumber は 0");

        // msgType の値そのもの(SPEC §10.2 の表)
        assert_eq!(
            (
                ws::TYPE_UVW,
                ws::TYPE_WAVEFORMS,
                ws::TYPE_HISTO1D,
                ws::TYPE_HISTO2D
            ),
            (0x02, 0x03, 0x10, 0x11)
        );
    }

    // -----------------------------------------------------------------
    // 2. built_event → Uvw(SPEC §10.2 の 0x02)
    // -----------------------------------------------------------------

    #[test]
    fn uvw_bodies_are_strip_major_and_carry_the_plane_geometry() {
        let mut converter = converter();
        // U strip1 に (b5,100)・(b6,250)、V strip1 に (b0,40)、W strip3 に (b511,7)。
        let event = built_event(
            7,
            42,
            true,
            vec![fragment(
                0,
                0,
                &[
                    (0, 0, 5, 100),
                    (0, 0, 6, 250),
                    (1, 0, 0, 40),
                    (1, 5, 511, 7),
                ],
            )],
        );
        let mut out = Vec::new();
        converter.uvw(&event, &mut out);

        assert_eq!(out.len(), 3, "面 3 枚(U/V/W)");
        for (plane, message) in out.iter().enumerate() {
            let n_strips = [2u16, 1, 3][plane];
            // 本体 = u8 plane, u16 nStrips, u16 nBuckets, u16 × nStrips*nBuckets
            assert_eq!(message[13], plane as u8, "off 13 = plane(0=U,1=V,2=W)");
            assert_eq!(u16_at(message, 14), n_strips, "off 14 = nStrips");
            assert_eq!(u16_at(message, 16), 512, "off 16 = nBuckets");
            assert_eq!(
                message.len(),
                18 + n_strips as usize * 512 * 2,
                "本体長 = 18 + nStrips*nBuckets*2"
            );
            assert_eq!(message[2], 0x02, "msgType = 0x02");
            assert_eq!(u32_at(message, 5), 7, "runNumber");
            assert_eq!(u32_at(message, 9), 42, "eventNumber");
            assert_eq!(message[4], 0, "complete なので flags = 0");
        }

        // idx = (strip-1)*nBuckets + bucket(strip-major)
        let grid_at = |message: &[u8], strip: u16, bucket: u16| -> u16 {
            u16_at(
                message,
                18 + ((strip as usize - 1) * 512 + bucket as usize) * 2,
            )
        };
        assert_eq!(grid_at(&out[0], 1, 5), 100, "U strip1 bucket5");
        assert_eq!(grid_at(&out[0], 1, 6), 250, "U strip1 bucket6");
        assert_eq!(grid_at(&out[0], 1, 7), 0, "空きビンは 0");
        assert_eq!(grid_at(&out[0], 2, 5), 0, "U strip2 には何も入れていない");
        assert_eq!(grid_at(&out[1], 1, 0), 40, "V strip1 bucket0");
        assert_eq!(
            grid_at(&out[2], 3, 511),
            7,
            "W strip3 bucket511(最終バケット)"
        );
        assert_eq!(grid_at(&out[2], 1, 511), 0, "W strip1 は空");
    }

    #[test]
    fn uvw_incomplete_events_set_the_flag_bit() {
        let mut converter = converter();
        let event = built_event(3, 4, false, vec![fragment(0, 0, &[(0, 0, 1, 9)])]);
        let mut out = Vec::new();
        converter.uvw(&event, &mut out);
        for message in &out {
            assert_eq!(message[4], ws::FLAG_INCOMPLETE, "incomplete = flags bit0");
        }
    }

    /// セクション合流(同一 strip 番号の別セクション)は**同じビンに加算**する
    /// (SPEC §5.2 の「同一ストリップ番号の複数セクションは同一ビンに合算」と同じ扱い)。
    #[test]
    fn uvw_merges_sections_into_the_same_bin() {
        let mut converter = converter();
        // U strip2 = section0 の raw1 と section1 の raw2 の両方。100 + 250 = 350。
        let event = built_event(
            1,
            1,
            true,
            vec![fragment(0, 0, &[(0, 1, 3, 100), (0, 2, 3, 250)])],
        );
        let mut out = Vec::new();
        converter.uvw(&event, &mut out);
        // idx = (strip-1)*nBuckets + bucket = (2-1)*512 + 3
        let at = 18 + (512 + 3) * 2;
        assert_eq!(u16_at(&out[0], at), 350, "U strip2 bucket3 = 100 + 250");
    }

    /// 合流は **saturating add**(u16 天井で clamp — 表示専用)。
    ///
    /// 手計算: 同一 (strip,bucket) に 17 チャンネル × ADC 4095 = 69,615 > 65,535 → 65,535。
    /// (16 チャンネルだと 65,520 でまだ収まるので、17 本必要。)
    #[test]
    fn uvw_saturates_at_the_u16_ceiling() {
        // 信号 ch 0..16 → raw 0..10, 12..17(FPN raw11 を飛ばす — SPEC §4.3)。
        let mut text = String::from("ANGLES: 1.0 2.0 3.0\n");
        for section in 0..17u32 {
            writeln!(text, "U\t{section}\t1\t0\t0\t0\t{section}\t0.0\t0.0\t10").unwrap();
        }
        let geometry = Arc::new(geometry::parse(&text));
        assert_eq!(geometry.max_strip[0], 1, "全部 strip 1");
        let mut converter = DisplayConverter::new(geometry);

        let raw_channels: Vec<u8> = (0..17usize)
            .map(|signal| geometry::REORDER_FROM_GEOMETRY_TO_GRAW[signal] as u8)
            .collect();
        assert_eq!(raw_channels[10], 10);
        assert_eq!(raw_channels[11], 12, "raw 11 は FPN なので飛ぶ");
        let samples: Vec<Sample> = raw_channels
            .iter()
            .map(|ch| (0u8, *ch, 0u16, 4095u16))
            .collect();

        let event = built_event(1, 1, true, vec![fragment(0, 0, &samples)]);
        let mut out = Vec::new();
        converter.uvw(&event, &mut out);
        assert_eq!(
            u16_at(&out[0], 18),
            u16::MAX,
            "17 × 4095 = 69,615 → 65,535 で頭打ち"
        );
    }

    /// FPN / AUX / 未記載(Unmapped)は UVW グリッドに入れない。
    #[test]
    fn uvw_excludes_fpn_aux_and_unmapped_channels() {
        let mut converter = converter();
        let event = built_event(
            1,
            1,
            true,
            vec![fragment(
                0,
                0,
                &[
                    (0, 11, 0, 4095), // FPN(raw 11)
                    (0, 4, 1, 4095),  // AUX
                    (0, 60, 2, 4095), // .dat に無い ch = Unmapped
                    (0, 0, 3, 77),    // U strip1(これだけが入る)
                ],
            )],
        );
        let mut out = Vec::new();
        converter.uvw(&event, &mut out);

        let u_grid = &out[0][18..];
        let total: u32 = u_grid
            .chunks_exact(2)
            .map(|c| u32::from(u16::from_le_bytes([c[0], c[1]])))
            .sum();
        assert_eq!(total, 77, "U 面に入ったのは strip1 の 77 だけ");
        assert_eq!(u16_at(&out[0], 18 + 3 * 2), 77, "U strip1 bucket3");
        // R-P2-13: Unmapped を引いたことがカウンタに出ている(silent 禁止)。
        assert!(
            converter.counts().unmapped_hits >= 1,
            "Unmapped の可視化フックが動いていない: {:?}",
            converter.counts()
        );
    }

    // -----------------------------------------------------------------
    // 3. built_event → Waveforms(SPEC §10.2 の 0x03)
    // -----------------------------------------------------------------

    #[test]
    fn waveforms_are_aget_major_raw_channel_order_and_keep_fpn() {
        let mut converter = converter();
        // (cobo,asad) = (0,1) と (1,0) の 2 フラグメント。
        let event = built_event(
            5,
            6,
            true,
            vec![
                fragment(0, 1, &[(0, 0, 0, 11), (0, 11, 2, 4095), (3, 67, 511, 999)]),
                fragment(1, 0, &[(2, 33, 7, 512)]),
            ],
        );
        let mut out = Vec::new();
        converter.waveforms(&event, &mut out);

        assert_eq!(out.len(), 2, "(cobo,asad) 毎に 1 通");
        let first = &out[0];
        assert_eq!(first[2], 0x03, "msgType = 0x03");
        assert_eq!(u32_at(first, 5), 5, "runNumber");
        assert_eq!(u32_at(first, 9), 6, "eventNumber");
        // 本体 = u8 cobo, u8 asad, u8 nAget, u8 nCh, u16 nBuckets, u16 × nAget*nCh*nBuckets
        assert_eq!(first[13], 0, "off 13 = cobo");
        assert_eq!(first[14], 1, "off 14 = asad");
        assert_eq!(first[15], 4, "off 15 = nAget");
        assert_eq!(first[16], 68, "off 16 = nCh(FPN 込みの raw 68)");
        assert_eq!(u16_at(first, 17), 512, "off 17 = nBuckets");
        assert_eq!(first.len(), 19 + 4 * 68 * 512 * 2);

        // idx = ((aget*nCh) + ch)*nBuckets + bucket
        let cell = |message: &[u8], aget: usize, ch: usize, bucket: usize| -> u16 {
            u16_at(message, 19 + ((aget * 68 + ch) * 512 + bucket) * 2)
        };
        assert_eq!(cell(first, 0, 0, 0), 11);
        assert_eq!(
            cell(first, 0, 11, 2),
            4095,
            "FPN(raw 11)も**そのまま**載る(R13)"
        );
        assert_eq!(
            cell(first, 3, 67, 511),
            999,
            "最終 aget・最終 ch・最終 bucket"
        );
        assert_eq!(cell(first, 1, 0, 0), 0, "触っていない ch は 0");

        let second = &out[1];
        assert_eq!(
            (second[13], second[14]),
            (1, 0),
            "2 通目は (cobo,asad)=(1,0)"
        );
        assert_eq!(cell(second, 2, 33, 7), 512);
        assert_eq!(
            cell(second, 0, 0, 0),
            0,
            "フラグメント間でバッファが漏れていない"
        );
    }

    #[test]
    fn histo2d_transposes_pub_strip_major_order_into_iy_outer_rows() {
        let mut converter = converter();
        // nx=3(strip)× ny=2(bucket)。PUB 順は (strip-1)*ny + bucket = ix 外側。
        // 値は 10*strip + bucket(全ビン相異なる = 転置ミスが必ず見える)。
        let values = vec![10.0, 11.0, 20.0, 21.0, 30.0, 31.0];
        let snapshot = HistSnapshotPayload {
            kind: "hist_snapshot".to_string(),
            run: 12,
            hists: vec![hist(1, "StripTimeU", 3, 2, &values)],
        };
        let mut out = Vec::new();
        converter.histos(&snapshot, &mut out);
        assert_eq!(out.len(), 1);
        let message = &out[0];

        assert_eq!(message[2], 0x11, "msgType = 0x11");
        assert_eq!(u32_at(message, 5), 12, "runNumber");
        assert_eq!(u32_at(message, 9), 0, "ヒストの eventNumber は 0");
        assert_eq!(u16_at(message, 13), 1, "off 13 = id");
        assert_eq!(u16_at(message, 15), 3, "off 15 = nx");
        assert_eq!(u16_at(message, 17), 2, "off 17 = ny");
        // 軸: 2D は x=strip [1, N+1) / y=bucket [0, ny)(SPEC §5.2 / 発注書)
        assert_eq!(f32_at(message, 19), 1.0, "xmin");
        assert_eq!(f32_at(message, 23), 4.0, "xmax = nx + 1");
        assert_eq!(f32_at(message, 27), 0.0, "ymin");
        assert_eq!(f32_at(message, 31), 2.0, "ymax = ny");
        assert_eq!(message.len(), 35 + 3 * 2 * 4);

        // iy 外側 row-major: [ (iy0: ix0,ix1,ix2), (iy1: ix0,ix1,ix2) ]
        let body: Vec<f32> = message[35..]
            .chunks_exact(4)
            .map(|c| f32_at(c, 0))
            .collect();
        assert_eq!(
            body,
            vec![10.0, 20.0, 30.0, 11.0, 21.0, 31.0],
            "PUB の strip-major を iy 外側へ転置していない"
        );
    }

    #[test]
    fn histo1d_uses_the_fixed_charge_axis_and_drops_to_f32() {
        let mut converter = converter();
        // 4 ビン。f32 で表せる値(0.5 刻み)を選んで、変換の可逆性を機械照合する。
        let values = vec![0.0, 1.5, 2.25, 3.75];
        let snapshot = HistSnapshotPayload {
            kind: "hist_snapshot".to_string(),
            run: 3,
            hists: vec![hist(4, "ChargeU", 4, 1, &values)],
        };
        let mut out = Vec::new();
        converter.histos(&snapshot, &mut out);
        let message = &out[0];

        assert_eq!(message[2], 0x10, "msgType = 0x10");
        assert_eq!(u16_at(message, 13), 4, "off 13 = id");
        assert_eq!(u32_at(message, 15), 4, "off 15 = nbins(u32)");
        // 1D の軸は [0,4096) 固定(SPEC §5.2「オートレンジ禁止」)。
        assert_eq!(f32_at(message, 19), 0.0, "xmin");
        assert_eq!(f32_at(message, 23), 4096.0, "xmax");
        let body: Vec<f32> = message[27..]
            .chunks_exact(4)
            .map(|c| f32_at(c, 0))
            .collect();
        assert_eq!(body, vec![0.0, 1.5, 2.25, 3.75]);
        assert_eq!(message.len(), 27 + 4 * 4);
    }

    #[test]
    fn histos_convert_all_nine_ids_in_order() {
        let mut converter = converter();
        let mut hists = Vec::new();
        for id in 1..=9u8 {
            let (nx, ny) = if id <= 3 { (2u32, 4u32) } else { (3u32, 1u32) };
            let values: Vec<f64> = (0..(nx * ny))
                .map(|b| f64::from(b) + f64::from(id))
                .collect();
            hists.push(hist(id, &format!("h{id}"), nx, ny, &values));
        }
        let snapshot = HistSnapshotPayload {
            kind: "hist_snapshot".to_string(),
            run: 1,
            hists,
        };
        let mut out = Vec::new();
        converter.histos(&snapshot, &mut out);

        assert_eq!(out.len(), 9, "9 枚(SPEC §5.2)");
        for (i, message) in out.iter().enumerate() {
            let id = (i + 1) as u16;
            assert_eq!(u16_at(message, 13), id, "id は PUB の順のまま");
            let expected_type = if id <= 3 { 0x11 } else { 0x10 };
            assert_eq!(message[2], expected_type, "1–3 が 2D、4–9 が 1D");
        }
    }

    #[test]
    fn histos_skip_and_count_a_bins_length_mismatch() {
        let mut converter = converter();
        let broken = hist(2, "StripTimeV", 4, 4, &[1.0, 2.0]); // 宣言 16 ビン・実体 2 ビン
        let snapshot = HistSnapshotPayload {
            kind: "hist_snapshot".to_string(),
            run: 1,
            hists: vec![broken, hist(5, "ChargeV", 2, 1, &[1.0, 2.0])],
        };
        let mut out = Vec::new();
        converter.histos(&snapshot, &mut out);

        assert_eq!(out.len(), 1, "壊れた 1 枚は落とし、健全な 1 枚は通す");
        assert_eq!(u16_at(&out[0], 13), 5);
        assert_eq!(converter.counts().malformed_hists, 1, "落とした分は数える");
    }
}
