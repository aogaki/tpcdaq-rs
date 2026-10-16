use std::sync::Arc;

use tracing::warn;

use super::*;
use crate::calib::pedestal::PedestalTable;
use crate::geometry::{ChannelRole, Geometry, AGET_CHIPS_PER_ASAD, RAW_CH_PER_AGET};
use crate::msg::Fragment;
use crate::reco::{PED_HI, PED_LO};

// =====================================================================
// SPEC §5.4 — 表示変換(IO 非依存の純コア)
// =====================================================================

/// AsAd の AGET 数 / AGET の raw ch 数(固定長配列の大きさ用の usize 版)。
const AGETS: usize = AGET_CHIPS_PER_ASAD as usize;
const RAW_CH: usize = RAW_CH_PER_AGET as usize;

/// ZS 述語の ped 窓の幅 = 再構成の ped 窓 cells `PED_LO..=PED_HI`(1..20)の 20(TODO/152)。
const PED_WINDOW: u32 = (PED_HI - PED_LO + 1) as u32;

/// 1 (cobo, asad) 分の ZS 復元の前計算(TODO/152、[`DisplayConverter::with_pedestal_table`])。
struct AsadPedestal {
    cobo: u32,
    asad: u32,
    /// 表の `round(mean)`((asad, aget, raw ch) で引いたもの)。表に無い ch は `None`。
    round_mean: [[Option<i32>; RAW_CH]; AGETS],
    /// 面ごと、`strip − 1` → この AsAd の strip ch(表にあるもの)の `round(mean)` の和。
    base: [Vec<i32>; N_PLANES],
}

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
    /// ZS らしいフレームを pedestal 表で復元した数(TODO/152)。
    pub uvw_zs_fragments_restored: u64,
    /// ZS らしいのに表が無く、欠けたセルを 0 のまま積んだフラグメント数(TODO/152)。
    pub uvw_zs_fragments_no_table: u64,
    /// 復元中に表に無かった strip ch の数(フラグメント × ch。そのサンプルは積まない — TODO/152)。
    pub uvw_table_missing_channels: u64,
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
    /// ZS 復元の前計算(`[system] ped_table` — TODO/152)。`None` = 表なし。
    pedestals: Option<Vec<AsadPedestal>>,
    /// ZS 述語の作業領域(フラグメント毎に数え直す): ch 毎のサンプル数と、ped 窓内のサンプル数。
    zs_samples: [[u32; RAW_CH]; AGETS],
    zs_in_window: [[u32; RAW_CH]; AGETS],
    /// 復元中のフラグメントで「表に無い strip ch」に付けた印(フラグメント × ch で 1 回数える)。
    zs_missing: [[bool; RAW_CH]; AGETS],
    logged_zs_no_table: bool,
    logged_table_missing: bool,
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
            pedestals: None,
            zs_samples: [[0; RAW_CH]; AGETS],
            zs_in_window: [[0; RAW_CH]; AGETS],
            zs_missing: [[false; RAW_CH]; AGETS],
            logged_zs_no_table: false,
            logged_table_missing: false,
        }
    }

    /// `[system] ped_table`(`calib pedestal` の表)で 0x02 の ZS 復元を有効にする(TODO/152)。
    ///
    /// ジオメトリの (cobo, asad) ごとに `round(mean)[aget][raw ch]` と、strip ごとの `base` =
    /// その AsAd の strip ch の `round(mean)` の和を前計算する。表のキーは (asad, aget, ch) で
    /// CoBo 軸が無い(151 の root_sink と同じく 1 論理 CoBo 前提)ので、どの CoBo にも同じ
    /// asad の行を当てる。表に無い ch は base に入らない。
    pub fn with_pedestal_table(mut self, table: &PedestalTable) -> Self {
        let mut pedestals = Vec::new();
        for (cobo, asad) in self.geometry.asad_inventory() {
            let mut round_mean = [[None; RAW_CH]; AGETS];
            for channel in table.channels.iter().filter(|c| u32::from(c.asad) == asad) {
                let slot = round_mean
                    .get_mut(usize::from(channel.aget))
                    .and_then(|row| row.get_mut(usize::from(channel.ch)));
                if let Some(slot) = slot {
                    *slot = Some(channel.mean.round() as i32);
                }
            }
            let mut base = self.n_strips.map(|n| vec![0i32; usize::from(n)]);
            for (aget, row) in round_mean.iter().enumerate() {
                for (chan, mean) in row.iter().enumerate() {
                    let &Some(mean) = mean else {
                        continue;
                    };
                    // 起動時 1 回、表にある ch だけ引く。
                    let role = self
                        .geometry
                        .lookup_ref(cobo, asad, aget as u32, chan as u32);
                    let &ChannelRole::Strip { plane, strip, .. } = role else {
                        continue; // FPN / AUX / Unmapped は格子に無い
                    };
                    let slot = usize::from(strip)
                        .checked_sub(1)
                        .and_then(|index| base[plane as usize].get_mut(index));
                    if let Some(slot) = slot {
                        *slot += mean;
                    }
                }
            }
            pedestals.push(AsadPedestal {
                cobo,
                asad,
                round_mean,
                base,
            });
        }
        self.pedestals = Some(pedestals);
        // 上の lookup_ref が引いた Unmapped(表にあるが .dat に無い ch)は表示変換中のものでは
        // ないので、R-P2-13 のフック(`note_unmapped`)の既読値をここまで進める。
        self.unmapped_seen = self.geometry.unmapped_hit_count();
        self
    }

    /// 変換の副作用カウンタ。
    pub fn counts(&self) -> ConvertCounts {
        self.counts
    }

    /// 直近の [`DisplayConverter::uvw`] が作った面毎の格子(U, V, W。strip-major の
    /// `nStrips × N_BUCKETS`、section 合算済み)= 0x02 の中身そのもの。再構成(TODO/127)が同じ格子を読む。
    pub fn grids(&self) -> [&[u16]; N_PLANES] {
        self.grids.each_ref().map(Vec::as_slice)
    }

    /// built event → `0x02 Uvw` ×3(SPEC §10.2)。面毎 `nStrips×512` の u16 グリッド。
    ///
    /// - 同一ビンへの複数チャンネル(セクション合流)は **saturating add**(u16 天井で clamp。
    ///   表示専用で、正値はヒスト / monitor.root 側にある — SPEC §5.2)。
    /// - FPN / Aux / Unmapped は入れない(SPEC §5.2 のヒストと同じ扱い)。
    /// - ZS らしいフラグメントは、pedestal 表があれば欠けたセルを `round(mean)` で埋めたのと
    ///   同じ値にする(TODO/152 — [`Self::with_pedestal_table`])。非 ZS は従来とビット一致。
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

    /// 1 フラグメントを格子へ積む。**2 パス**(TODO/152): まず ZS らしいかを数え、
    /// ZS らしく表があれば、その AsAd の strip の全 bucket に `base` を足してから、在るサンプルを
    /// `adc − round(mean)` で積む(= 欠けたセルを round(mean) で埋めて合算したのと同じ。
    /// 全セル在る ch なら整数で恒等)。それ以外は従来どおり `adc` を積む。
    fn accumulate_uvw(&mut self, fragment: &Fragment) {
        let cobo = u32::from(fragment.cobo);
        let asad = u32::from(fragment.asad);
        let words = ItemWords::new(
            &fragment.items,
            &mut self.counts,
            &mut self.logged_misaligned,
        );
        let restore = if self.looks_zero_suppressed(words.clone()) {
            self.zs_restore_index(fragment)
        } else {
            None
        };
        let pedestal = match (restore, &self.pedestals) {
            (Some(index), Some(pedestals)) => pedestals.get(index),
            _ => None,
        };
        let restoring = pedestal.is_some();
        if let Some(pedestal) = pedestal {
            add_base(&mut self.grids, &pedestal.base);
            for row in &mut self.zs_missing {
                row.fill(false);
            }
        }
        for word in words {
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
            let delta = match pedestal {
                None => i32::from(item.adc),
                Some(pedestal) => {
                    let (aget, chan) = (usize::from(item.aget), usize::from(item.chan));
                    let round_mean = pedestal
                        .round_mean
                        .get(aget)
                        .and_then(|row| row.get(chan))
                        .copied()
                        .flatten();
                    match round_mean {
                        Some(round_mean) => i32::from(item.adc) - round_mean,
                        None => {
                            // 表に無い strip ch: base にも入っていない。積まずに印を付ける
                            // (数えるのはループの後 — フラグメント × ch で 1 回)。
                            let flag = self
                                .zs_missing
                                .get_mut(aget)
                                .and_then(|row| row.get_mut(chan));
                            if let Some(flag) = flag {
                                *flag = true;
                            }
                            continue;
                        }
                    }
                }
            };
            let idx = (strip as usize - 1) * N_BUCKETS as usize + item.bucket as usize;
            if let Some(cell) = self.grids[p].get_mut(idx) {
                // セクション合流は加算。u16 の天井(と 0)で clamp する(表示専用)。
                // 表なしの経路では `saturating_add(adc)` と同じ。
                *cell = clamp_u16(i32::from(*cell) + delta);
            }
        }
        if restoring {
            self.count_table_missing(cobo, asad);
        }
    }

    /// ZS 述語(TODO/152 — root_sink `frame_is_zero_suppressed` と同じ形): このフラグメントで
    /// サンプルのある ch(**FPN 込み**の 4 × 68)のうち、ped 窓 cells `PED_LO..=PED_HI` 内の
    /// サンプル数が窓幅に満たない ch が**過半**なら ZS らしい(半数ちょうどは非 ZS)。
    /// full / partial は読んだ ch が窓を全部持つので偽、ZS は pre-trigger のサンプルがほぼ
    /// 無いので真。ジオメトリは引かない(Unmapped の計数を汚さない)。
    fn looks_zero_suppressed(&mut self, words: ItemWords<'_>) -> bool {
        for row in &mut self.zs_samples {
            row.fill(0);
        }
        for row in &mut self.zs_in_window {
            row.fill(0);
        }
        for word in words {
            let item = crate::msg::unpack_item(word);
            let (aget, chan) = (usize::from(item.aget), usize::from(item.chan));
            let Some(samples) = self
                .zs_samples
                .get_mut(aget)
                .and_then(|row| row.get_mut(chan))
            else {
                continue; // AsAd に無い (aget, ch)(0x03 と同じく捨てる)
            };
            *samples += 1;
            if (PED_LO..=PED_HI).contains(&usize::from(item.bucket)) {
                self.zs_in_window[aget][chan] += 1;
            }
        }
        let mut with_samples = 0u32;
        let mut short_of_window = 0u32;
        let counts = self.zs_samples.iter().flatten();
        for (&samples, &in_window) in counts.zip(self.zs_in_window.iter().flatten()) {
            if samples == 0 {
                continue;
            }
            with_samples += 1;
            if in_window < PED_WINDOW {
                short_of_window += 1;
            }
        }
        2 * short_of_window > with_samples
    }

    /// ZS らしいフラグメントの行き先を決めて数える: 表があればその (cobo, asad) の前計算の
    /// 添字(`uvw_zs_fragments_restored`)、表が無ければ `None`(`uvw_zs_fragments_no_table`、
    /// WARN 1 回)。ジオメトリに無い (cobo, asad) も `None`(サンプルは全部 Unmapped で数える)。
    fn zs_restore_index(&mut self, fragment: &Fragment) -> Option<usize> {
        let Some(pedestals) = &self.pedestals else {
            self.counts.uvw_zs_fragments_no_table += 1;
            if !self.logged_zs_no_table {
                self.logged_zs_no_table = true;
                warn!(
                    cobo = fragment.cobo,
                    asad = fragment.asad,
                    "monitor: fragment looks zero-suppressed but no [system] ped_table is set — \
                     missing cells stay 0 in the 0x02 grid (the reco pedestal and the UI \
                     baseline come out ~0); counted as uvw_zs_fragments_no_table (warned once)"
                );
            }
            return None;
        };
        let (cobo, asad) = (u32::from(fragment.cobo), u32::from(fragment.asad));
        let index = pedestals
            .iter()
            .position(|p| p.cobo == cobo && p.asad == asad)?;
        self.counts.uvw_zs_fragments_restored += 1;
        Some(index)
    }

    /// 復元したフラグメントで印の付いた「表に無い strip ch」を数える(フラグメント × ch で 1 回、
    /// WARN は初回だけ)。
    fn count_table_missing(&mut self, cobo: u32, asad: u32) {
        for (aget, row) in self.zs_missing.iter().enumerate() {
            for (chan, _) in row.iter().enumerate().filter(|(_, &missing)| missing) {
                self.counts.uvw_table_missing_channels += 1;
                if !self.logged_table_missing {
                    self.logged_table_missing = true;
                    warn!(
                        cobo,
                        asad,
                        aget,
                        chan,
                        "monitor: the pedestal table has no entry for a zero-suppressed strip \
                         channel — its samples are left out of the 0x02 grid; counted as \
                         uvw_table_missing_channels (warned once)"
                    );
                }
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

/// ZS 復元の土台(TODO/152): その AsAd の strip に `round(mean)` の和を全 bucket ぶん足す。
fn add_base(grids: &mut [Vec<u16>; N_PLANES], base: &[Vec<i32>; N_PLANES]) {
    for (grid, base) in grids.iter_mut().zip(base) {
        for (row, &value) in grid.chunks_exact_mut(N_BUCKETS as usize).zip(base) {
            if value == 0 {
                continue;
            }
            for cell in row {
                *cell = clamp_u16(i32::from(*cell) + value);
            }
        }
    }
}

/// 表示格子の u16 に収める(0 と u16 の天井で clamp — 表示専用)。
fn clamp_u16(value: i32) -> u16 {
    value.clamp(0, i32::from(u16::MAX)) as u16
}

/// `Fragment.items`(u32 LE 連結)を**中間 Vec を作らずに**なめるイテレータ。
///
/// [`crate::msg::items_from_bytes`] は `Vec<u32>` を確保するのでホットパスでは使わない。
/// 4 の倍数でない端数は「壊れたフラグメント」としてカウント + 初回だけ warn する。
/// `Clone` は 2 パス(ZS 述語 → 積算、TODO/152)用 —— 端数の計数は `new` の 1 回だけ。
#[derive(Clone)]
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
    use crate::calib::pedestal::PedestalChannel;
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

    // -----------------------------------------------------------------
    // 4. ZS 復元(TODO/152 — `[system] ped_table`)
    // -----------------------------------------------------------------

    /// `calib pedestal` の JSON と同じ形の表(値は合成)。`(asad, aget, raw ch, mean)`。
    fn pedestal_table(entries: &[(u8, u8, u8, f64)]) -> PedestalTable {
        PedestalTable {
            meta: crate::calib::Meta {
                inputs: vec!["synthetic.graw".to_string()],
                events: 1,
                generated_at: "2026-10-16T00:00:00+00:00".to_string(),
                tool_version: "test".to_string(),
            },
            channels: entries
                .iter()
                .map(|&(asad, aget, ch, mean)| PedestalChannel {
                    asad,
                    aget,
                    ch,
                    mean,
                    rms: 2.0,
                    runs: 1,
                    is_fpn: crate::calib::is_fpn(ch),
                })
                .collect(),
        }
    }

    /// `GEOMETRY_TEXT` の strip ch の表(+ AUX・FPN —— strip でないのでグリッドには効かない)。
    /// round(mean): U1 = 100、U2 = 201(section 0)+ 50(section 1)、V1 = 300、
    /// W3 = 78(77.5 → 78: `f64::round` は 0.5 を 0 から遠い側へ)。
    fn small_table() -> PedestalTable {
        pedestal_table(&[
            (0, 0, 0, 100.4),  // U strip1
            (0, 0, 1, 200.6),  // U strip2 section 0
            (0, 0, 2, 49.7),   // U strip2 section 1
            (0, 1, 0, 299.8),  // V strip1
            (0, 1, 5, 77.5),   // W strip3
            (0, 0, 4, 10.0),   // AUX
            (0, 0, 11, 400.0), // FPN
        ])
    }

    /// ZS らしいフラグメント: サンプルのある 5 ch すべてが ped 窓(cells 1..20)に 20 サンプル
    /// 未満(W3 だけ窓内に 1 サンプル)。FPN は無い(zCoBo の ZS は FPN も抑制する)。
    fn zs_samples() -> Vec<Sample> {
        vec![
            (0, 0, 100, 160), // U strip1
            (0, 0, 101, 180), // U strip1
            (0, 1, 200, 260), // U strip2: section 0 だけ
            (0, 2, 300, 90),  // U strip2: section 1 だけ
            (0, 1, 400, 300), // U strip2: 両方(section 0)
            (0, 2, 400, 70),  // U strip2: 両方(section 1)
            (1, 5, 5, 1000),  // W strip3(窓内に 1 サンプル)
            (0, 4, 50, 900),  // AUX
        ]
    }

    /// 1 フラグメント (cobo 0, asad 0) のイベントを 0x02 に変換し、3 面の格子を返す。
    fn grids_of(converter: &mut DisplayConverter, samples: &[Sample]) -> [Vec<u16>; N_PLANES] {
        let event = built_event(1, 1, true, vec![fragment(0, 0, samples)]);
        let mut out = Vec::new();
        converter.uvw(&event, &mut out);
        converter.grids().map(<[u16]>::to_vec)
    }

    /// `(restored, no_table, missing_channels)`
    fn zs_counts(converter: &DisplayConverter) -> (u64, u64, u64) {
        let c = converter.counts();
        (
            c.uvw_zs_fragments_restored,
            c.uvw_zs_fragments_no_table,
            c.uvw_table_missing_channels,
        )
    }

    /// (a) ZS + 表: 無いセル = round(mean)、あるセル = adc(セクションは合算)。
    #[test]
    fn zs_fragment_with_a_table_restores_missing_cells_to_the_rounded_mean() {
        let mut converter = converter().with_pedestal_table(&small_table());
        let grids = grids_of(&mut converter, &zs_samples());

        // 復元オラクル: セル = Σ_section(そのセルにサンプルがあれば adc、無ければ round(mean))。
        let mut u = vec![100u16; 2 * 512]; // strip1 = 100
        u[512..].fill(201 + 50); // strip2 = 201 + 50
        u[100] = 160;
        u[101] = 180;
        u[512 + 200] = 260 + 50; // section 0 だけ在る → adc1 + round(mean2)
        u[512 + 300] = 201 + 90; // section 1 だけ在る → round(mean1) + adc2
        u[512 + 400] = 300 + 70; // 両方在る → adc1 + adc2
        let v = vec![300u16; 512]; // サンプル無し → 全セル round(mean)
        let mut w = vec![0u16; 3 * 512]; // W strip1/2 は ch が無い = 0
        w[2 * 512..].fill(78);
        w[2 * 512 + 5] = 1000;
        assert_eq!(grids, [u, v, w]);
        assert_eq!(zs_counts(&converter), (1, 0, 0));

        // 次のイベントで base が二重に積まれない(格子はイベント毎に 0 から)。
        assert_eq!(grids_of(&mut converter, &zs_samples()), grids);
        assert_eq!(zs_counts(&converter), (2, 0, 0));
    }

    /// (b) full(非 ZS)+ 表 → 表なしと同一(述語が発火しない)。
    #[test]
    fn full_fragment_with_a_table_is_unchanged() {
        // 全 4×68 ch × 512 bucket(FPN・AUX・未記載 ch も読む = full readout)。値は非対称。
        let mut samples: Vec<Sample> = Vec::new();
        for aget in 0..4u8 {
            for ch in 0..68u8 {
                for bucket in 0..512u16 {
                    let adc = 100 + 50 * u16::from(aget) + 3 * u16::from(ch) + (bucket * 13) % 29;
                    samples.push((aget, ch, bucket, adc));
                }
            }
        }
        let plain = grids_of(&mut converter(), &samples);
        let mut with_table = converter().with_pedestal_table(&small_table());

        assert_eq!(grids_of(&mut with_table, &samples), plain);
        assert_eq!(zs_counts(&with_table), (0, 0, 0));
    }

    /// 復元は全データなら恒等(整数の丸めで厳密): ZS らしいフラグメントでも 512 セル全部
    /// 在る ch の strip は生 ADC そのもの(round(mean) を足して引くだけ。adc < mean のセルも)。
    #[test]
    fn a_fully_present_channel_inside_a_zs_fragment_is_restored_exactly() {
        let raw_u1 = |bucket: u16| 90 + (bucket * 7) % 31; // 90..=120(round(mean) = 100 を跨ぐ)
        let mut samples: Vec<Sample> = (0..512u16).map(|b| (0, 0, b, raw_u1(b))).collect();
        // サンプルのある 3 ch のうち 2 ch が窓に満たない → 2·2 > 3 で ZS らしい。
        samples.push((0, 1, 300, 260)); // U strip2 section 0: 1 サンプル
        samples.push((1, 0, 300, 500)); // V strip1: 1 サンプル
        let mut converter = converter().with_pedestal_table(&small_table());
        let grids = grids_of(&mut converter, &samples);

        assert_eq!(zs_counts(&converter), (1, 0, 0));
        let expected_u1: Vec<u16> = (0..512u16).map(raw_u1).collect();
        assert_eq!(&grids[0][..512], &expected_u1[..], "U strip1 = 生 ADC");
        assert_eq!(grids[0][512 + 300], 260 + 50);
        assert_eq!(grids[1][300], 500);
        assert_eq!(grids[1][0], 300);
    }

    /// (c) ZS + 表なし → 従来どおり(欠けたセルは 0)+ `uvw_zs_fragments_no_table`。
    #[test]
    fn zs_fragment_without_a_table_is_unchanged_and_counted() {
        let mut converter = converter();
        let grids = grids_of(&mut converter, &zs_samples());

        let mut u = vec![0u16; 2 * 512];
        u[100] = 160;
        u[101] = 180;
        u[512 + 200] = 260;
        u[512 + 300] = 90;
        u[512 + 400] = 300 + 70;
        let mut w = vec![0u16; 3 * 512];
        w[2 * 512 + 5] = 1000;
        assert_eq!(grids, [u, vec![0u16; 512], w]);
        assert_eq!(zs_counts(&converter), (0, 1, 0));

        grids_of(&mut converter, &zs_samples());
        assert_eq!(zs_counts(&converter), (0, 2, 0), "フラグメント毎に数える");
    }

    /// 述語(root_sink `frame_is_zero_suppressed` と同じ形): サンプルのある ch(**FPN 込み**)の
    /// **過半**が ped 窓 cells 1..20(reco の PED_LO..=PED_HI、幅 20)に 20 サンプル未満なら ZS。
    /// 半数ちょうどは非 ZS。
    #[test]
    fn the_zs_predicate_needs_a_strict_majority_of_channels_short_of_the_window() {
        // 窓ちょうど(cells 1..=20)= 足りている / 1 つずれ(cells 0..=19 → 窓内 19)= 足りない。
        let full =
            |aget: u8, ch: u8| -> Vec<Sample> { (1..=20u16).map(|b| (aget, ch, b, 100)).collect() };
        let short =
            |aget: u8, ch: u8| -> Vec<Sample> { (0..=19u16).map(|b| (aget, ch, b, 200)).collect() };
        let restored_after = |samples: Vec<Sample>| -> u64 {
            let mut converter = converter().with_pedestal_table(&small_table());
            grids_of(&mut converter, &samples);
            converter.counts().uvw_zs_fragments_restored
        };

        // FPN(raw 11)が足りていて U2 が足りない: 1/2 = 半数ちょうど → 非 ZS(FPN も数に入る)
        assert_eq!(restored_after([full(0, 11), short(0, 1)].concat()), 0);
        // 足りない ch がもう 1 本: 2/3 → ZS
        assert_eq!(
            restored_after([full(0, 11), short(0, 1), short(1, 0)].concat()),
            1
        );
        // 窓の端: cells 1..=20 が揃えば足りている(0 と 21 は窓の外)。1/3 → 非 ZS
        assert_eq!(
            restored_after([full(0, 0), full(0, 1), short(1, 0)].concat()),
            0
        );
    }

    /// 表に無い strip ch: サンプルは積まない(base にも入らない)+ フラグメント × ch で数える。
    #[test]
    fn strip_channels_missing_from_the_table_are_skipped_and_counted_once_per_fragment() {
        // V strip1(aget1 raw0)が表に無い。
        let table = pedestal_table(&[
            (0, 0, 0, 100.4),
            (0, 0, 1, 200.6),
            (0, 0, 2, 49.7),
            (0, 1, 5, 77.5),
        ]);
        let mut converter = converter().with_pedestal_table(&table);
        let mut samples = zs_samples();
        samples.extend([(1, 0, 60, 500), (1, 0, 61, 520)]); // V strip1 に 2 サンプル

        let grids = grids_of(&mut converter, &samples);
        assert_eq!(grids[1], vec![0u16; 512], "表に無い ch は積まない");
        assert_eq!(grids[0][100], 160, "他の ch は復元する");
        assert_eq!(grids[0][0], 100);
        assert_eq!(
            zs_counts(&converter),
            (1, 0, 1),
            "2 サンプルでも ch は 1 本"
        );

        grids_of(&mut converter, &samples);
        assert_eq!(zs_counts(&converter), (2, 0, 2), "フラグメント毎");
    }

    /// 0x03(波形)は表があっても生のまま(欠けたセルは 0、減算なし — R13 / 152 §1.5)。
    #[test]
    fn waveforms_stay_raw_with_a_table() {
        let mut converter = converter().with_pedestal_table(&small_table());
        let event = built_event(1, 1, true, vec![fragment(0, 0, &zs_samples())]);
        let mut out = Vec::new();
        converter.waveforms(&event, &mut out);

        let cell = |aget: usize, ch: usize, bucket: usize| -> u16 {
            u16_at(&out[0], 19 + ((aget * 68 + ch) * 512 + bucket) * 2)
        };
        assert_eq!(cell(0, 0, 100), 160);
        assert_eq!(cell(0, 0, 0), 0, "欠けたセルは 0 のまま");
        assert_eq!(cell(1, 5, 5), 1000);
    }

    /// 起動時に表を格子へ割り付けるときに引いた Unmapped(表にあるが .dat に無い ch)は
    /// 「表示変換中の Unmapped」ではない —— R-P2-13 の可視化フックを鳴らさない。
    #[test]
    fn startup_table_lookups_are_not_unmapped_hits_of_the_conversion() {
        // raw 3 は `GEOMETRY_TEXT` に無い ch。
        let table = pedestal_table(&[(0, 0, 0, 100.4), (0, 0, 3, 123.0)]);
        let mut converter = converter().with_pedestal_table(&table);
        grids_of(&mut converter, &[(0, 0, 100, 160)]);

        assert_eq!(converter.counts().unmapped_hits, 0);
    }

    /// (d) 合成飛跡(`src/reco/tests.rs` と同じ流儀: ガウス σ 2 cell、mini の角度 90/−30/30)を
    /// full readout と ZS(パルス 20 ADC 未満のセルと FPN を落とす)で流す。表で復元した
    /// 格子からの `strip_hits` と直線は full と同じ(表なしの ZS 格子では違う)。
    ///
    /// 同じになる理由: ped(cells 1..20)は full でも復元でも基線そのもの(基線は整数、表の
    /// mean は基線 +0.3 / −0.4 → round で基線)。落としたセルはパルス < 20 ≤ ADC_THRESHOLD(35)
    /// なので、q にもピークにも効かない。
    #[test]
    fn reco_on_the_restored_grid_matches_the_full_readout() {
        use crate::reco::{strip_hits, Reco, RecoGeometry, StripHit};
        const N: usize = 40; // 面ごとの strip 本数
        const PITCH_MM: f64 = 1.5;
        const ZS_THRESHOLD: u16 = 20;
        const NB: usize = N_BUCKETS as usize;

        // .dat: U strip k = aget0 / V = aget1 / W = aget2 の信号 ch k−1(aget3 は空き)
        let mut text = String::new();
        for (plane, aget) in [("U", 0), ("V", 1), ("W", 2)] {
            for k in 1..=N {
                let signal = k - 1;
                writeln!(
                    text,
                    "{plane}\t0\t{k}\t0\t0\t{aget}\t{signal}\t0.0\t0.0\t10"
                )
                .unwrap();
            }
        }
        let geometry = Arc::new(geometry::parse(&text));
        assert_eq!(geometry.max_strip, [N as u16; 3]);

        // strip 位置 s_k [mm] と mini の pitch 単位ベクトル(reco/tests.rs の `mini_pitch`)。
        let s_of = |k: usize| (k as f64 - (N as f64 + 1.0) / 2.0) * PITCH_MM;
        let sqrt3_2 = 3.0_f64.sqrt() / 2.0;
        let pitch = [[-1.0, 0.0], [0.5, sqrt3_2], [0.5, -sqrt3_2]];
        let positions: Vec<Option<f64>> = (1..=N).map(|k| Some(s_of(k))).collect();
        let reco_geometry = RecoGeometry {
            positions: [positions.clone(), positions.clone(), positions],
            pitch,
        };

        // 直線: (−20, −5) mm @ bucket 150 → (20, 8) mm @ bucket 250。strip k には s_k で
        // 直線と交わる時刻に高さ 800 のガウス(±6σ)。交わる strip は U 26 / V 21 / W 6 本
        // (s_a, s_b = U: 20, −20 / V: −14.33, 16.93 / W: −5.67, 3.07)。
        let (a, b, t_a, t_b) = ([-20.0, -5.0], [20.0, 8.0], 150.0, 250.0);
        let mut pulse = vec![vec![0u16; N * NB]; 3];
        for (plane, [px, py]) in pitch.iter().enumerate() {
            let (s_a, s_b) = (px * a[0] + py * a[1], px * b[0] + py * b[1]);
            for k in 1..=N {
                let f = (s_of(k) - s_a) / (s_b - s_a);
                if !(0.0..=1.0).contains(&f) {
                    continue;
                }
                let center = t_a + f * (t_b - t_a);
                for t in 0..NB {
                    let d = (t as f64 - center) / 2.0;
                    if d.abs() <= 6.0 {
                        pulse[plane][(k - 1) * NB + t] =
                            (800.0 * (-0.5 * d * d).exp()).round() as u16;
                    }
                }
            }
        }
        let pulse_at = |aget: u8, raw: u8, t: usize| -> u16 {
            let signal = geometry::REORDER_FROM_GEOMETRY_TO_GRAW
                .iter()
                .position(|&r| r == u32::from(raw));
            match signal {
                Some(signal) if aget < 3 && signal < N => pulse[aget as usize][signal * NB + t],
                _ => 0, // FPN・空き ch
            }
        };

        // 全 4×68 ch: 基線(整数、ch 毎に違う)+ パルス。ZS は FPN を落とし、パルス < 20 を落とす。
        let mut full: Vec<Sample> = Vec::new();
        let mut zs: Vec<Sample> = Vec::new();
        let mut entries = Vec::new();
        for aget in 0..4u8 {
            for raw in 0..68u8 {
                let base = 200 + 9 * u16::from(aget) + u16::from(raw % 13);
                let frac = if raw % 2 == 0 { 0.3 } else { -0.4 };
                entries.push((0u8, aget, raw, f64::from(base) + frac));
                for t in 0..NB as u16 {
                    let p = pulse_at(aget, raw, t as usize);
                    full.push((aget, raw, t, base + p));
                    if !crate::calib::is_fpn(raw) && p >= ZS_THRESHOLD {
                        zs.push((aget, raw, t, base + p));
                    }
                }
            }
        }
        let table = pedestal_table(&entries);

        let full_grids = grids_of(&mut DisplayConverter::new(Arc::clone(&geometry)), &full);
        let mut restoring =
            DisplayConverter::new(Arc::clone(&geometry)).with_pedestal_table(&table);
        let restored_grids = grids_of(&mut restoring, &zs);
        assert_eq!(
            zs_counts(&restoring),
            (1, 0, 0),
            "ZS らしいと判定して復元した"
        );
        let raw_zs_grids = grids_of(&mut DisplayConverter::new(Arc::clone(&geometry)), &zs);

        let hits_of = |grids: &[Vec<u16>; N_PLANES]| -> Vec<StripHit> {
            let mut out = Vec::new();
            for (view, grid) in grids.iter().enumerate() {
                strip_hits(
                    view as u8,
                    grid,
                    NB,
                    &reco_geometry.positions[view],
                    &mut out,
                );
            }
            out
        };
        let full_hits = hits_of(&full_grids);
        assert_eq!(full_hits.len(), 26 + 21 + 6, "交わる strip が全部ヒット");
        assert_eq!(
            hits_of(&restored_grids),
            full_hits,
            "復元格子の strip_hits = full"
        );
        assert_ne!(
            hits_of(&raw_zs_grids),
            full_hits,
            "表なしの ZS 格子では違う(欠けセル 0 が ped を壊す)"
        );

        let run = |grids: &[Vec<u16>; N_PLANES]| -> Reco {
            let mut reco = Reco::new(reco_geometry.clone());
            reco.run(grids.each_ref().map(Vec::as_slice), NB);
            reco
        };
        let full_reco = run(&full_grids);
        assert!(full_reco.event().fit.is_ok(), "{:?}", full_reco.event().fit);
        assert_eq!(run(&restored_grids).event(), full_reco.event());
    }
}
