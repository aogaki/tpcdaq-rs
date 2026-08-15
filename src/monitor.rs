//! monitor — root-sink のモニタ PUB を購読し、表示用に変換して WS で配るコンポーネント。
//! SPEC §5.3(PUB ワイヤ — 受け側)/ §5.4(責務)/ §10(WS プロトコル)/ §3.2(WS 9000、PUB 47004)。
//!
//! # 構成
//!
//! ```text
//!   root-sink ──PUB(47004)──▶ [SUB 専用 OS スレッド]
//!                                 │  decode(§5.3)→ ギャップ計数 → 表示変換(§5.4)
//!                                 ▼
//!                              [Hub]  live = broadcast(有界・drop-oldest)
//!                                 │   JSON = クライアント毎 mpsc(reliable)
//!                                 ▼
//!                              [axum WS] ──▶ クライアント(UI)
//! ```
//!
//! # モニタ系の掟(CLAUDE.md / SPEC §1.4-4)
//!
//! **落としてよいが silent にしない**。落ちる場所は 2 つしかなく、両方数える:
//!
//! 1. root-sink → monitor(ZMQ PUB/SUB の間引き)= エンベロープ `sequence_number` の
//!    飛び → `monitor_gaps`。
//! 2. monitor → クライアント(遅いクライアント)= live キューの drop-oldest → `ws_dropped`。
//!
//! どちらも 1 Hz の `status` JSON に載って UI から見える(`{monitorGaps, clients, wsDropped}`)。
//! monitor は保存系に一切触れない(PUB を受けるだけ。REP も持たない — 純コンシューマ)。
//!
//! # モジュールの切り方(Clean Architecture)
//!
//! 前半(§5.3 パーサ / [`GapTracker`] / [`ws`] エンコーダ / [`DisplayConverter`] / JSON 組み立て)は
//! **IO を知らない純コア**で、そのまま単体テストできる。後半([`Hub`] / [`run_monitor`])だけが
//! ZMQ と axum を知る。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use axum::body::Bytes;
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_bytes::ByteBuf;
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::{error, info, warn};

use crate::config::Config;
use crate::geometry::{self, ChannelRole, Geometry, AGET_CHIPS_PER_ASAD, RAW_CH_PER_AGET};
use crate::msg::Fragment;

/// 1 チャンネルの time bucket 数(GET ハードウェア定数。SPEC §2.4 の bucket 0–511)。
pub const N_BUCKETS: u16 = 512;

/// 平面の数(U/V/W)。
pub const N_PLANES: usize = 3;

// =====================================================================
// SPEC §5.3 — モニタ PUB のペイロード(named struct パーサ)
// =====================================================================
//
// ワイヤは map 形式 msgpack(`to_vec_named` 相当の自己記述)。エンベロープだけは
// §2.2 と同形の positional array(5) なので、[`crate::msg::Message`] / [`crate::msg::Batch`]
// をそのまま被せて読む。

/// 面毎の飽和カウンタ(SPEC §5.3 の `saturation`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaturationPayload {
    pub saturated: u64,
    pub counted: u64,
}

/// `kind = "status"`(SPEC §5.3、v1.10 で `pending_events` を含む 11 キー)。
///
/// WS の `status` JSON は**このフィールドをそのまま**出し、`{monitorGaps, clients, wsDropped}`
/// を足す(SPEC §10.3)ので `Serialize` も導出する。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusPayload {
    pub kind: String,
    pub run: u32,
    pub state: String,
    pub events_built: u64,
    pub events_incomplete: u64,
    pub late_fragments: u64,
    /// ビルダ組み上げ中の瞬間値(SPEC §5.3 v1.10、R-P2-5)。`late_fragments` の次のキー。
    pub pending_events: u64,
    pub frames_per_cobo: std::collections::BTreeMap<String, u64>,
    pub bytes_written: u64,
    pub saturation: std::collections::BTreeMap<String, SaturationPayload>,
    pub publish_drops: u64,
}

/// ヒスト 1 枚(SPEC §5.3 の `hists[]`)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistPayload {
    pub id: u8,
    pub name: String,
    pub nx: u32,
    /// 1D は 1。
    pub ny: u32,
    /// f64 LE の生バイト列(長さ `nx*ny*8`)。2D の添字は `(strip-1)*512 + bucket`
    /// (**strip が遅い軸** — SPEC §5.3)。
    pub bins: ByteBuf,
}

impl HistPayload {
    /// 宣言どおりのビン数(`nx*ny`)。
    pub fn bin_count(&self) -> usize {
        (self.nx as usize).saturating_mul(self.ny as usize)
    }

    /// `bins` の長さが `nx*ny*8` と辻褄が合っているか。
    pub fn is_consistent(&self) -> bool {
        self.bins.len() == self.bin_count() * 8
    }

    /// ビン値を f64 として取り出す(照合用の便宜メソッド。表示変換は
    /// [`ws::histo1d`] / [`ws::histo2d`] が生バイトのまま読むので中間 Vec を作らない)。
    pub fn values(&self) -> Vec<f64> {
        self.bins.chunks_exact(8).map(f64_le).collect()
    }
}

/// `kind = "hist_snapshot"`(SPEC §5.3)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistSnapshotPayload {
    pub kind: String,
    pub run: u32,
    pub hists: Vec<HistPayload>,
}

/// `kind = "built_event"`(SPEC §5.3。`fragments` は §2.4 の positional array)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuiltEventPayload {
    pub kind: String,
    pub run: u32,
    pub event_idx: u32,
    pub complete: bool,
    pub fragments: Vec<Fragment>,
}

/// `kind` だけを先に読む前哨(3 種の判別 + 未知 kind の前方互換)。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct KindProbe {
    kind: String,
}

/// 判別済みペイロード。**未知 kind は捨てずに [`MonitorPayload::Unknown`] として返す**
/// (呼び手が数えて可視化する — 前方互換、silent 禁止)。
#[derive(Debug, Clone, PartialEq)]
pub enum MonitorPayload {
    Status(Box<StatusPayload>),
    HistSnapshot(Box<HistSnapshotPayload>),
    BuiltEvent(Box<BuiltEventPayload>),
    Unknown(String),
}

/// モニタ PUB の 1 通(エンベロープ = SPEC §2.2 + 判別済みペイロード)。
#[derive(Debug, Clone, PartialEq)]
pub struct MonitorMessage {
    pub source_id: u32,
    pub run_number: u32,
    /// **run リセットなしの単調増加**(SPEC §5.3 v1.10)。飛び = ドロップ数。
    pub sequence_number: u64,
    pub created_ns: u64,
    pub payload: MonitorPayload,
}

impl MonitorMessage {
    /// `status` ならその中身(照合・テスト用)。
    pub fn status(&self) -> Option<&StatusPayload> {
        match &self.payload {
            MonitorPayload::Status(s) => Some(s),
            _ => None,
        }
    }

    /// `hist_snapshot` ならその中身。
    pub fn hist_snapshot(&self) -> Option<&HistSnapshotPayload> {
        match &self.payload {
            MonitorPayload::HistSnapshot(s) => Some(s),
            _ => None,
        }
    }

    /// `built_event` ならその中身。
    pub fn built_event(&self) -> Option<&BuiltEventPayload> {
        match &self.payload {
            MonitorPayload::BuiltEvent(e) => Some(e),
            _ => None,
        }
    }
}

/// モニタ PUB の 1 通が読めなかった理由。
#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("monitor PUB message ({len} B) is not a SPEC §2.2 Data envelope with a `kind` payload: {source}")]
    Envelope {
        len: usize,
        #[source]
        source: rmp_serde::decode::Error,
    },

    #[error("monitor PUB message is not Data (EndOfStream/Heartbeat is not part of SPEC §5.3)")]
    NotData,

    #[error("monitor PUB payload kind={kind} does not parse: {source}")]
    Payload {
        kind: String,
        #[source]
        source: rmp_serde::decode::Error,
    },
}

/// モニタ PUB の 1 通を復号する(SPEC §5.3)。
///
/// `kind` を先に読んでから本体を型付きで読み直す 2 段構え(map 形式の自己記述ワイヤに
/// 対する素直な実装)。未知 kind はエラーにせず [`MonitorPayload::Unknown`] で返す。
pub fn decode_message(raw: &[u8]) -> Result<MonitorMessage, WireError> {
    let probe: crate::msg::Message<KindProbe> =
        rmp_serde::from_slice(raw).map_err(|source| WireError::Envelope {
            len: raw.len(),
            source,
        })?;
    let crate::msg::Message::Data(batch) = probe else {
        return Err(WireError::NotData);
    };
    let kind = batch.payload.kind;
    let payload = match kind.as_str() {
        "status" => MonitorPayload::Status(Box::new(payload_of(raw, &kind)?)),
        "hist_snapshot" => MonitorPayload::HistSnapshot(Box::new(payload_of(raw, &kind)?)),
        "built_event" => MonitorPayload::BuiltEvent(Box::new(payload_of(raw, &kind)?)),
        _ => MonitorPayload::Unknown(kind),
    };
    Ok(MonitorMessage {
        source_id: batch.source_id,
        run_number: batch.run_number,
        sequence_number: batch.sequence_number,
        created_ns: batch.created_ns,
        payload,
    })
}

fn payload_of<T: DeserializeOwned>(raw: &[u8], kind: &str) -> Result<T, WireError> {
    let message: crate::msg::Message<T> =
        rmp_serde::from_slice(raw).map_err(|source| WireError::Payload {
            kind: kind.to_string(),
            source,
        })?;
    match message {
        crate::msg::Message::Data(batch) => Ok(batch.payload),
        _ => Err(WireError::NotData),
    }
}

// =====================================================================
// ギャップ計数(SPEC §5.4「モニタ取りこぼし数」)
// =====================================================================

/// モニタ PUB の `sequence_number` の飛びを数える。
///
/// PUB リンクの seq は **run リセット無しの単調増加**(SPEC §5.3 v1.10)なので、
/// 「前回 + 1」以外は取りこぼし。送り手(root-sink)が再起動して seq が巻き戻った場合は
/// **ギャップではなく再起動**として数え直す(でっちあげの巨大ギャップを出さない)。
#[derive(Debug, Default)]
pub struct GapTracker {
    last: Option<u64>,
    gaps: u64,
    restarts: u64,
}

impl GapTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// 1 通観測する。返り値 = **この 1 通で新たに判明した取りこぼし数**。
    pub fn observe(&mut self, sequence_number: u64) -> u64 {
        let missed = match self.last {
            None => 0,
            Some(prev) if sequence_number > prev => sequence_number - prev - 1,
            Some(_) => {
                // 巻き戻り = 送り手の再起動(seq は単調のはずなので)。
                self.restarts += 1;
                0
            }
        };
        self.last = Some(sequence_number);
        self.gaps += missed;
        missed
    }

    /// 累積の取りこぼし数(= `monitor_gaps`)。
    pub fn gaps(&self) -> u64 {
        self.gaps
    }

    /// 送り手の seq 巻き戻り(再起動)を観測した回数。
    pub fn restarts(&self) -> u64 {
        self.restarts
    }
}

// =====================================================================
// SPEC §10.1 / §10.2 — WS バイナリワイヤ(IO 非依存の純エンコーダ)
// =====================================================================

/// WS バイナリメッセージのエンコーダ(SPEC §10.1/§10.2)。
///
/// バイトオフセットは SPEC の表そのもの。**ここを動かすと UI(027)の
/// デコーダが黙って壊れる**ので、レイアウトはオフセット assert 付きの単体テスト
/// (SPEC §10.4-4)で固定してある。
pub mod ws {
    /// 13 バイトヘッダ(SPEC §10.1)。
    pub const HEADER_LEN: usize = 13;
    /// マジック `'T' 'P'`(off 0–1)。
    pub const MAGIC: [u8; 2] = *b"TP";
    /// off 3。型再定義のため 2(SPEC §10.1)。
    pub const VERSION: u8 = 2;
    /// off 4 の bit0 = incomplete event。
    pub const FLAG_INCOMPLETE: u8 = 0x01;

    /// off 2 の msgType(SPEC §10.2)。
    pub const TYPE_UVW: u8 = 0x02;
    pub const TYPE_WAVEFORMS: u8 = 0x03;
    pub const TYPE_HISTO1D: u8 = 0x10;
    pub const TYPE_HISTO2D: u8 = 0x11;

    /// 13 バイトヘッダの中身。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Header {
        pub msg_type: u8,
        /// bit0(イベント系のみ。ヒストは常に false)。
        pub incomplete: bool,
        pub run_number: u32,
        /// ヒスト・status 系は 0(SPEC §10.1)。
        pub event_number: u32,
    }

    impl Header {
        /// イベント系(0x02/0x03)のヘッダ。
        pub fn event(msg_type: u8, run_number: u32, event_number: u32, incomplete: bool) -> Header {
            Header {
                msg_type,
                incomplete,
                run_number,
                event_number,
            }
        }

        /// ヒスト系(0x10/0x11)のヘッダ。`eventNumber` は 0、flags は 0(SPEC §10.1)。
        pub fn hist(msg_type: u8, run_number: u32) -> Header {
            Header {
                msg_type,
                incomplete: false,
                run_number,
                event_number: 0,
            }
        }

        /// ヘッダを書き出す(全フィールド LE)。
        pub fn write(&self, out: &mut Vec<u8>) {
            out.extend_from_slice(&MAGIC);
            out.push(self.msg_type);
            out.push(VERSION);
            out.push(if self.incomplete { FLAG_INCOMPLETE } else { 0 });
            out.extend_from_slice(&self.run_number.to_le_bytes());
            out.extend_from_slice(&self.event_number.to_le_bytes());
        }
    }

    /// ヒストの軸レンジ(f32 で運ぶ — 表示専用)。
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct Axis {
        pub min: f32,
        pub max: f32,
    }

    /// `0x02 Uvw` — `u8 plane`, `u16 nStrips`, `u16 nBuckets`, `u16 ADC × nStrips×nBuckets`
    /// (strip-major: `idx=(strip-1)*nBuckets+bucket`)。
    pub fn uvw(header: Header, plane: u8, n_strips: u16, n_buckets: u16, grid: &[u16]) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + 5 + grid.len() * 2);
        header.write(&mut out);
        out.push(plane);
        out.extend_from_slice(&n_strips.to_le_bytes());
        out.extend_from_slice(&n_buckets.to_le_bytes());
        for v in grid {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }

    /// `0x03 Waveforms` — `u8 cobo`, `u8 asad`, `u8 nAget`, `u8 nCh`, `u16 nBuckets`,
    /// `u16 ADC × nAget×nCh×nBuckets`(aget-major、raw ch 順、FPN 込み・減算なし)。
    pub fn waveforms(
        header: Header,
        cobo: u8,
        asad: u8,
        n_aget: u8,
        n_ch: u8,
        n_buckets: u16,
        grid: &[u16],
    ) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + 6 + grid.len() * 2);
        header.write(&mut out);
        out.push(cobo);
        out.push(asad);
        out.push(n_aget);
        out.push(n_ch);
        out.extend_from_slice(&n_buckets.to_le_bytes());
        for v in grid {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }

    /// `0x10 Histo1d` — `u16 id`, `u32 nbins`, `f32 xmin`, `f32 xmax`, `f32 × nbins`。
    ///
    /// `bins_le` は SPEC §5.3 の生バイト列(f64 LE)。中間 Vec を作らずに f32 へ落とす。
    pub fn histo1d(header: Header, id: u16, x: Axis, bins_le: &[u8]) -> Vec<u8> {
        let nbins = bins_le.len() / 8;
        let mut out = Vec::with_capacity(HEADER_LEN + 14 + nbins * 4);
        header.write(&mut out);
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&(nbins as u32).to_le_bytes());
        out.extend_from_slice(&x.min.to_le_bytes());
        out.extend_from_slice(&x.max.to_le_bytes());
        for chunk in bins_le.chunks_exact(8) {
            out.extend_from_slice(&(super::f64_le(chunk) as f32).to_le_bytes());
        }
        out
    }

    /// `0x11 Histo2d` — `u16 id`, `u16 nx`, `u16 ny`, `f32 xmin,xmax,ymin,ymax`,
    /// `f32 × nx×ny`(**iy 外側 row-major**)。
    ///
    /// 入力 `bins_le` は SPEC §5.3 の PUB 順(`(strip-1)*512 + bucket` = **ix が外側**)なので、
    /// ここで**転置**する。この 1 行が §5.3 と §10.2 の並び順の違いを吸収している。
    pub fn histo2d(
        header: Header,
        id: u16,
        nx: u16,
        ny: u16,
        x: Axis,
        y: Axis,
        bins_le: &[u8],
    ) -> Vec<u8> {
        let (nxu, nyu) = (nx as usize, ny as usize);
        let mut out = Vec::with_capacity(HEADER_LEN + 22 + nxu * nyu * 4);
        header.write(&mut out);
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&nx.to_le_bytes());
        out.extend_from_slice(&ny.to_le_bytes());
        for v in [x.min, x.max, y.min, y.max] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for iy in 0..nyu {
            for ix in 0..nxu {
                let at = (ix * nyu + iy) * 8;
                let value = match bins_le.get(at..at + 8) {
                    Some(chunk) => super::f64_le(chunk) as f32,
                    None => 0.0,
                };
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        out
    }
}

/// f64 LE の 8 バイトを読む(長さが足りなければ 0.0)。
fn f64_le(chunk: &[u8]) -> f64 {
    match chunk.try_into() {
        Ok(bytes) => f64::from_le_bytes(bytes),
        Err(_) => 0.0,
    }
}

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
const CHARGE_AXIS_MAX: f32 = 4096.0;

/// `Fragment.items`(u32 LE 連結)を**中間 Vec を作らずに**なめるイテレータ。
///
/// [`crate::msg::items_from_bytes`] は `Vec<u32>` を確保するのでホットパスでは使わない。
/// 4 の倍数でない端数は「壊れたフラグメント」としてカウント + 初回だけ warn する。
struct ItemWords<'a> {
    chunks: std::slice::ChunksExact<'a, u8>,
}

impl<'a> ItemWords<'a> {
    fn new(items: &'a [u8], counts: &mut ConvertCounts, logged: &mut bool) -> Self {
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

// =====================================================================
// SPEC §10.3 — JSON テキストメッセージ(純)
// =====================================================================

/// `meta`(S→C、接続時・run 変化時)の材料。
#[derive(Debug, Clone, PartialEq)]
pub struct Meta {
    pub n_buckets: u16,
    /// 面毎の最大ストリップ番号(= `Geometry::max_strip`)。
    pub planes: [u16; N_PLANES],
    /// ジオメトリの設定パス名。
    pub geometry: String,
    /// `HeaderScalars::angles_deg`(無ければ `null`)。
    pub angles_deg: Option<[f64; 3]>,
    /// 検出器名(`[system] experiment`)。
    pub detector: String,
    pub cobos: Vec<u32>,
    pub run: u32,
}

/// `meta` を JSON 文字列にする(SPEC §10.3)。
pub fn meta_json(meta: &Meta) -> Result<String, serde_json::Error> {
    let value = serde_json::json!({
        "type": "meta",
        "nBuckets": meta.n_buckets,
        "planes": {
            "U": meta.planes[0],
            "V": meta.planes[1],
            "W": meta.planes[2],
        },
        "geometry": meta.geometry,
        "anglesDeg": meta.angles_deg,
        "detector": meta.detector,
        "cobos": meta.cobos,
        "run": meta.run,
    });
    serde_json::to_string(&value)
}

/// monitor 自身の可視化値(SPEC §10.3 の `status` に足す 3 つ)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitorStats {
    pub monitor_gaps: u64,
    pub clients: u64,
    pub ws_dropped: u64,
}

/// `status` を JSON 文字列にする(SPEC §10.3: §5.3 の status **そのまま** +
/// `{monitorGaps, clients, wsDropped}`)。
pub fn status_json(
    status: &StatusPayload,
    stats: MonitorStats,
) -> Result<String, serde_json::Error> {
    let mut value = serde_json::to_value(status)?;
    if let Some(map) = value.as_object_mut() {
        map.insert("type".to_string(), serde_json::json!("status"));
        map.insert(
            "monitorGaps".to_string(),
            serde_json::json!(stats.monitor_gaps),
        );
        map.insert("clients".to_string(), serde_json::json!(stats.clients));
        map.insert("wsDropped".to_string(), serde_json::json!(stats.ws_dropped));
    }
    serde_json::to_string(&value)
}

/// `run`(S→C、state 遷移時)を JSON 文字列にする(SPEC §10.3)。
pub fn run_json(state: &str, run: u32, ts: &str) -> Result<String, serde_json::Error> {
    let value = serde_json::json!({ "type": "run", "state": state, "run": run, "ts": ts });
    serde_json::to_string(&value)
}

/// クライアント毎の購読集合(SPEC §10.3)。**既定は waveforms 以外 ON**。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamSet {
    pub uvw: bool,
    pub waveforms: bool,
    pub histos: bool,
    pub status: bool,
}

impl Default for StreamSet {
    fn default() -> Self {
        Self {
            uvw: true,
            waveforms: false, // 帯域制御(波形ビューを開いたクライアントだけ)
            histos: true,
            status: true,
        }
    }
}

impl StreamSet {
    /// この live ストリームを受けるか。
    pub fn wants(&self, stream: Stream) -> bool {
        match stream {
            Stream::Uvw => self.uvw,
            Stream::Waveforms => self.waveforms,
            Stream::Histos => self.histos,
        }
    }
}

/// C→S の `subscribe` の解釈結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscribe {
    pub streams: StreamSet,
    /// 知らないストリーム名(数えて可視化する — silent にしない)。
    pub unknown: Vec<String>,
}

/// `{"streams": [...]}`(SPEC §10.3)を解釈する。`streams` 配列が無ければ `None`。
///
/// **配列に無いものは OFF**(既定値との OR ではない)。UI が「波形を閉じた」ことを
/// 表現できる必要があるため。
pub fn parse_subscribe(text: &str) -> Option<Subscribe> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let list = value.get("streams")?.as_array()?;
    let mut streams = StreamSet {
        uvw: false,
        waveforms: false,
        histos: false,
        status: false,
    };
    let mut unknown = Vec::new();
    for item in list {
        match item.as_str() {
            Some("uvw") => streams.uvw = true,
            Some("waveforms") => streams.waveforms = true,
            Some("histos") => streams.histos = true,
            Some("status") => streams.status = true,
            Some(other) => unknown.push(other.to_string()),
            None => unknown.push(item.to_string()),
        }
    }
    Some(Subscribe { streams, unknown })
}

// =====================================================================
// SPEC §10.4-1 — 適合性テスト用のサンプルストリーム(既知値)
// =====================================================================

/// 全 WS メッセージ型を**既知値**でエンコードしたもの(SPEC §10.4-1)。
///
/// 値は非対称に選んである(取り違えが目に見えるように)。TS 側(027)の本番デコーダが
/// これを分解して同じ値を復元できることが適合性の定義。**同一入力 → 同一バイト**。
pub fn ws_sample_messages() -> Vec<Vec<u8>> {
    // Uvw: 3 ストリップ × 4 バケット。値は strip*10 + bucket(全ビン相異なる)。
    let n_strips: u16 = 3;
    let n_buckets: u16 = 4;
    let mut grid = Vec::with_capacity((n_strips * n_buckets) as usize);
    for strip in 1..=n_strips {
        for bucket in 0..n_buckets {
            grid.push(strip * 10 + bucket);
        }
    }
    let uvw = ws::uvw(
        ws::Header::event(ws::TYPE_UVW, 7, 42, false),
        1, // V 面
        n_strips,
        n_buckets,
        &grid,
    );

    // Waveforms: 2 AGET × 3 ch × 2 bucket(実機の 4×68×512 の縮小版)。
    let (n_aget, n_ch, wf_buckets) = (2u8, 3u8, 2u16);
    let mut wf = Vec::with_capacity(n_aget as usize * n_ch as usize * wf_buckets as usize);
    for aget in 0..n_aget as u16 {
        for ch in 0..n_ch as u16 {
            for bucket in 0..wf_buckets {
                wf.push(aget * 100 + ch * 10 + bucket);
            }
        }
    }
    let waveforms = ws::waveforms(
        ws::Header::event(ws::TYPE_WAVEFORMS, 7, 43, true), // incomplete = flags bit0
        0,
        1,
        n_aget,
        n_ch,
        wf_buckets,
        &wf,
    );

    // Histo1d: 4 ビン(0.5 刻みで f32 に落ちても値が変わらない数)。
    let h1: Vec<f64> = vec![0.0, 1.5, 2.25, 3.75];
    let histo1d = ws::histo1d(
        ws::Header::hist(ws::TYPE_HISTO1D, 7),
        4, // ChargeU(SPEC §5.2)
        ws::Axis {
            min: 0.0,
            max: CHARGE_AXIS_MAX,
        },
        &f64_bytes(&h1),
    );

    // Histo2d: nx=3(strip)× ny=2(bucket)。入力は PUB 順(ix 外側)。
    let h2: Vec<f64> = vec![11.0, 12.0, 21.0, 22.0, 31.0, 32.0];
    let histo2d = ws::histo2d(
        ws::Header::hist(ws::TYPE_HISTO2D, 7),
        1, // StripTimeU
        3,
        2,
        ws::Axis { min: 1.0, max: 4.0 },
        ws::Axis { min: 0.0, max: 2.0 },
        &f64_bytes(&h2),
    );

    vec![uvw, waveforms, histo1d, histo2d]
}

/// f64 列を SPEC §5.3 の生バイト列(f64 LE)にする。
fn f64_bytes(values: &[f64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 8);
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// [`ws_sample_messages`] を `u32 LE 長さ + ペイロード` の連結ストリームとして書き出す
/// (SPEC §10.4-1。生成物はコミットしない — 毎回再生成)。
pub fn write_ws_sample_stream(path: &Path) -> Result<(), std::io::Error> {
    use std::io::Write as _;
    let file = std::fs::File::create(path)?;
    let mut out = std::io::BufWriter::new(file);
    for frame in ws_sample_messages() {
        let len = u32::try_from(frame.len()).unwrap_or(u32::MAX);
        out.write_all(&len.to_le_bytes())?;
        out.write_all(&frame)?;
    }
    out.flush()
}

// =====================================================================
// ここから下が IO 層(ZMQ SUB + axum WS)
// =====================================================================

/// monitor 1 プロセス分の起動パラメタ(= 設定の解決結果)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorParams {
    /// root-sink のモニタ PUB への SUB 接続先(SPEC §3.2)。
    pub sub_endpoint: String,
    /// WS の listen アドレス(SPEC §3.2)。テストはポート 0 を使う。
    pub ws_listen: String,
    /// 表示変換に使うジオメトリ。
    pub geometry: PathBuf,
    /// live 送信キューの段数(drop-oldest の深さ)。
    pub live_queue: usize,
    /// `meta.detector`(`[system] experiment`)。
    pub detector: String,
    /// `meta.cobos`(設定の `[[cobo]]` id)。
    pub cobos: Vec<u32>,
}

impl MonitorParams {
    pub fn from_config(config: &Config) -> Self {
        Self {
            sub_endpoint: config.monitor.sub_endpoint.clone(),
            ws_listen: config.monitor.ws_listen.clone(),
            geometry: config.monitor.geometry.clone(),
            live_queue: config.monitor.live_queue,
            detector: config.system.experiment.clone(),
            cobos: config.cobo.iter().map(|c| c.id).collect(),
        }
    }
}

/// monitor の起動失敗。
#[derive(Debug, thiserror::Error)]
pub enum MonitorError {
    #[error("cannot load geometry {path}: {source}")]
    Geometry {
        path: PathBuf,
        #[source]
        source: geometry::GeometryError,
    },

    #[error("cannot bind the WS listener {listen}: {source}")]
    Bind {
        listen: String,
        #[source]
        source: std::io::Error,
    },

    #[error("cannot start the monitor SUB thread: {0}")]
    Spawn(#[source] std::io::Error),

    #[error("WS server failed: {0}")]
    Serve(#[source] std::io::Error),
}

/// monitor のカウンタ(全部 `status` JSON か終了ログで見える — silent にしない)。
#[derive(Debug, Default)]
pub struct MonitorCounters {
    pub messages_in: AtomicU64,
    /// PUB の seq の飛び = root-sink → monitor の取りこぼし(SPEC §5.4)。
    pub monitor_gaps: AtomicU64,
    /// 送り手の seq 巻き戻り(root-sink 再起動)。
    pub source_restarts: AtomicU64,
    pub status_in: AtomicU64,
    pub snapshots_in: AtomicU64,
    pub events_in: AtomicU64,
    /// 未知 kind(前方互換 — 数えて無視する)。
    pub unknown_kind: AtomicU64,
    pub decode_errors: AtomicU64,
    /// live キューの drop-oldest で落ちた通数(SPEC §10.3)。
    pub ws_dropped: AtomicU64,
    /// 現在の接続クライアント数。
    pub clients: AtomicU64,
    /// 累積の接続数。
    pub clients_total: AtomicU64,
    /// JSON(reliable)が詰まって切ったクライアント数。
    pub clients_dropped_slow: AtomicU64,
    /// 解釈できなかった C→S メッセージ数。
    pub bad_client_messages: AtomicU64,
}

impl MonitorCounters {
    fn bump(counter: &AtomicU64, by: u64) {
        counter.fetch_add(by, Ordering::Relaxed);
    }
}

/// live 1 通(全クライアント共有 — [`Bytes`] なのでクライアント毎のコピーは参照数だけ)。
#[derive(Debug)]
struct LiveFrame {
    stream: Stream,
    bytes: Bytes,
}

/// クライアント毎の購読フラグ(WS 受信タスクが書き、送信タスクが読む)。
#[derive(Debug)]
struct ClientStreams {
    uvw: AtomicBool,
    waveforms: AtomicBool,
    histos: AtomicBool,
    status: AtomicBool,
}

impl ClientStreams {
    fn new(set: StreamSet) -> Self {
        Self {
            uvw: AtomicBool::new(set.uvw),
            waveforms: AtomicBool::new(set.waveforms),
            histos: AtomicBool::new(set.histos),
            status: AtomicBool::new(set.status),
        }
    }

    fn store(&self, set: StreamSet) {
        self.uvw.store(set.uvw, Ordering::Relaxed);
        self.waveforms.store(set.waveforms, Ordering::Relaxed);
        self.histos.store(set.histos, Ordering::Relaxed);
        self.status.store(set.status, Ordering::Relaxed);
    }

    fn wants(&self, stream: Stream) -> bool {
        match stream {
            Stream::Uvw => self.uvw.load(Ordering::Relaxed),
            Stream::Waveforms => self.waveforms.load(Ordering::Relaxed),
            Stream::Histos => self.histos.load(Ordering::Relaxed),
        }
    }
}

/// JSON(reliable)チャンネルの深さ。制御メッセージは 1 Hz 級なので、ここが詰まる
/// = そのクライアントが数十秒読んでいない、ということ。**黙って溜め続けない**
/// (SPEC §10.3「JSON 制御 = reliable」)。
const JSON_QUEUE: usize = 32;

struct Client {
    json: mpsc::Sender<String>,
    streams: Arc<ClientStreams>,
}

/// 接続クライアントの集合 + live の配り口(SPEC §10.3)。
///
/// live は **1 本の broadcast**(容量 = `live_queue`)。遅いクライアントは
/// `RecvError::Lagged(n)` で「n 通落ちた」ことを知り、`ws_dropped` に積む
/// (= drop-oldest + 計数。速いクライアントは影響を受けない)。
pub struct Hub {
    live: broadcast::Sender<Arc<LiveFrame>>,
    clients: Mutex<HashMap<u64, Client>>,
    next_id: AtomicU64,
    counters: Arc<MonitorCounters>,
    /// waveforms を購読しているクライアント数(0 なら 0x03 を**作らない** — 帯域と CPU)。
    waveform_demand: AtomicUsize,
    meta: Mutex<Meta>,
    /// `meta` の run(直近の status 由来)。
    meta_run: AtomicU32,
    /// 直近に見た `(run, state)`(遷移検知用)。
    last_run_state: Mutex<Option<(u32, String)>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // 毒された Mutex でも monitor を止めない(中身は monitor 自身が作ったものだけ)。
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Hub {
    fn new(meta: Meta, live_queue: usize, counters: Arc<MonitorCounters>) -> Arc<Hub> {
        let (live, _) = broadcast::channel(live_queue.max(1));
        Arc::new(Hub {
            live,
            clients: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(0),
            counters,
            waveform_demand: AtomicUsize::new(0),
            meta: Mutex::new(meta),
            meta_run: AtomicU32::new(0),
            last_run_state: Mutex::new(None),
        })
    }

    fn stats(&self) -> MonitorStats {
        MonitorStats {
            monitor_gaps: self.counters.monitor_gaps.load(Ordering::Relaxed),
            clients: self.counters.clients.load(Ordering::Relaxed),
            ws_dropped: self.counters.ws_dropped.load(Ordering::Relaxed),
        }
    }

    /// 現在の `meta` JSON(接続時 / run 変化時に送るもの)。
    fn meta_json(&self) -> Option<String> {
        let mut meta = lock(&self.meta).clone();
        meta.run = self.meta_run.load(Ordering::Relaxed);
        match meta_json(&meta) {
            Ok(text) => Some(text),
            Err(e) => {
                error!(error = %e, "monitor: cannot encode the meta message");
                None
            }
        }
    }

    /// 波形(0x03)を作る必要があるか。
    fn waveforms_wanted(&self) -> bool {
        self.waveform_demand.load(Ordering::Relaxed) > 0
    }

    fn refresh_waveform_demand(&self) {
        let clients = lock(&self.clients);
        let n = clients
            .values()
            .filter(|c| c.streams.waveforms.load(Ordering::Relaxed))
            .count();
        drop(clients);
        self.waveform_demand.store(n, Ordering::Relaxed);
    }

    /// live を全クライアントへ配る(`frames` は drain されるが容量は呼び手が使い回す)。
    fn publish_live(&self, stream: Stream, frames: &mut Vec<Vec<u8>>) {
        for bytes in frames.drain(..) {
            // 受け手ゼロの Err は正常(誰も見ていない)。
            let _ = self.live.send(Arc::new(LiveFrame {
                stream,
                bytes: Bytes::from(bytes),
            }));
        }
    }

    /// JSON(reliable)を配る。`status_only` の分は購読フラグで絞る。
    ///
    /// 詰まったクライアントは**切る**(黙って無限に溜めない — SPEC §10.3)。
    /// 切り方はエントリを落とすだけ: 送信タスクの `json` チャンネルが閉じ、
    /// そのタスクが socket ごと畳む。
    fn broadcast_json(&self, text: &str, status_only: bool) {
        let mut slow: Vec<u64> = Vec::new();
        {
            let clients = lock(&self.clients);
            for (id, client) in clients.iter() {
                if status_only && !client.streams.status.load(Ordering::Relaxed) {
                    continue;
                }
                if client.json.try_send(text.to_string()).is_err() {
                    slow.push(*id);
                }
            }
        }
        for id in slow {
            warn!(
                client = id,
                "monitor: JSON control queue is full — dropping this client (SPEC §10.3 reliable)"
            );
            self.counters
                .clients_dropped_slow
                .fetch_add(1, Ordering::Relaxed);
            self.remove_client(id);
        }
    }

    fn register(
        &self,
    ) -> (
        u64,
        Arc<ClientStreams>,
        mpsc::Receiver<String>,
        broadcast::Receiver<Arc<LiveFrame>>,
    ) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let streams = Arc::new(ClientStreams::new(StreamSet::default()));
        let (tx, rx) = mpsc::channel(JSON_QUEUE);
        let live_rx = self.live.subscribe();
        {
            let mut clients = lock(&self.clients);
            clients.insert(
                id,
                Client {
                    json: tx,
                    streams: Arc::clone(&streams),
                },
            );
            self.counters
                .clients
                .store(clients.len() as u64, Ordering::Relaxed);
        }
        self.counters.clients_total.fetch_add(1, Ordering::Relaxed);
        self.refresh_waveform_demand();
        info!(
            client = id,
            clients = self.counters.clients.load(Ordering::Relaxed),
            "monitor: WS client connected"
        );
        (id, streams, rx, live_rx)
    }

    fn remove_client(&self, id: u64) {
        {
            let mut clients = lock(&self.clients);
            if clients.remove(&id).is_none() {
                return;
            }
            self.counters
                .clients
                .store(clients.len() as u64, Ordering::Relaxed);
        }
        self.refresh_waveform_demand();
        info!(
            client = id,
            clients = self.counters.clients.load(Ordering::Relaxed),
            "monitor: WS client disconnected"
        );
    }

    /// status 1 通を処理する: run/state の遷移を検知して `run`(+ run 変化なら `meta`)を
    /// 送り、`status` を配る(SPEC §10.3)。
    fn on_status(&self, status: &StatusPayload) {
        let previous = lock(&self.last_run_state).clone();
        let changed_run = previous.as_ref().map(|(run, _)| *run) != Some(status.run);
        let changed_state =
            previous.as_ref().map(|(_, state)| state.as_str()) != Some(status.state.as_str());

        if changed_run {
            self.meta_run.store(status.run, Ordering::Relaxed);
            if let Some(text) = self.meta_json() {
                self.broadcast_json(&text, false);
            }
        }
        if changed_run || changed_state {
            let ts = now_rfc3339_millis();
            match run_json(&status.state, status.run, &ts) {
                Ok(text) => self.broadcast_json(&text, false),
                Err(e) => error!(error = %e, "monitor: cannot encode the run message"),
            }
            *lock(&self.last_run_state) = Some((status.run, status.state.clone()));
        }

        match status_json(status, self.stats()) {
            Ok(text) => self.broadcast_json(&text, true),
            Err(e) => error!(error = %e, "monitor: cannot encode the status message"),
        }
    }

    /// C→S の `subscribe` を適用する。
    fn apply_subscribe(&self, id: u64, text: &str) {
        let Some(subscribe) = parse_subscribe(text) else {
            self.counters
                .bad_client_messages
                .fetch_add(1, Ordering::Relaxed);
            warn!(
                client = id,
                message = text,
                "monitor: client message is not a SPEC §10.3 subscribe — ignored"
            );
            return;
        };
        if !subscribe.unknown.is_empty() {
            warn!(
                client = id,
                unknown = ?subscribe.unknown,
                "monitor: subscribe named unknown streams — ignored"
            );
        }
        {
            let clients = lock(&self.clients);
            match clients.get(&id) {
                Some(client) => client.streams.store(subscribe.streams),
                None => return,
            }
        }
        self.refresh_waveform_demand();
        info!(client = id, streams = ?subscribe.streams, "monitor: subscribe applied");
    }
}

/// SPEC §9.1 と同じ書式(RFC3339・ミリ秒・ローカルオフセット)。
fn now_rfc3339_millis() -> String {
    chrono::Local::now()
        .format("%Y-%m-%dT%H:%M:%S%.3f%:z")
        .to_string()
}

// ---------------------------------------------------------------------
// SUB 側(専用 OS スレッド)
// ---------------------------------------------------------------------

/// PUB を 1 通受けてから WS へ配るまで(= 表示変換つきの取り込み)。
///
/// ZMQ を知らない([`Ingest::handle`] は生バイト列を受けるだけ)ので、統合テストは
/// 実 ZMQ、単体テストは直接呼び出しの両方ができる。
struct Ingest {
    hub: Arc<Hub>,
    converter: DisplayConverter,
    gaps: GapTracker,
    /// 変換結果の入れ物。**使い回す**(ホットパスで確保し直さない)。
    frames: Vec<Vec<u8>>,
    logged_decode_error: bool,
    logged_unknown_kind: bool,
}

impl Ingest {
    fn new(hub: Arc<Hub>, geometry: Arc<Geometry>) -> Self {
        Self {
            hub,
            converter: DisplayConverter::new(geometry),
            gaps: GapTracker::new(),
            frames: Vec::new(),
            logged_decode_error: false,
            logged_unknown_kind: false,
        }
    }

    fn handle(&mut self, raw: &[u8]) {
        let counters = Arc::clone(&self.hub.counters);
        MonitorCounters::bump(&counters.messages_in, 1);

        let message = match decode_message(raw) {
            Ok(message) => message,
            Err(e) => {
                MonitorCounters::bump(&counters.decode_errors, 1);
                if !self.logged_decode_error {
                    self.logged_decode_error = true;
                    warn!(error = %e, "monitor: cannot decode a monitor PUB message");
                }
                return;
            }
        };

        let missed = self.gaps.observe(message.sequence_number);
        if missed > 0 {
            MonitorCounters::bump(&counters.monitor_gaps, missed);
        }
        counters
            .source_restarts
            .store(self.gaps.restarts(), Ordering::Relaxed);

        match &message.payload {
            MonitorPayload::Status(status) => {
                MonitorCounters::bump(&counters.status_in, 1);
                self.hub.on_status(status);
            }
            MonitorPayload::HistSnapshot(snapshot) => {
                MonitorCounters::bump(&counters.snapshots_in, 1);
                self.frames.clear();
                self.converter.histos(snapshot, &mut self.frames);
                self.hub.publish_live(Stream::Histos, &mut self.frames);
            }
            MonitorPayload::BuiltEvent(event) => {
                MonitorCounters::bump(&counters.events_in, 1);
                self.frames.clear();
                self.converter.uvw(event, &mut self.frames);
                self.hub.publish_live(Stream::Uvw, &mut self.frames);
                // 0x03 は**購読しているクライアントが居るときだけ**作る(SPEC §10.3 の帯域制御)。
                if self.hub.waveforms_wanted() {
                    self.frames.clear();
                    self.converter.waveforms(event, &mut self.frames);
                    self.hub.publish_live(Stream::Waveforms, &mut self.frames);
                }
            }
            MonitorPayload::Unknown(kind) => {
                MonitorCounters::bump(&counters.unknown_kind, 1);
                if !self.logged_unknown_kind {
                    self.logged_unknown_kind = true;
                    info!(
                        kind,
                        "monitor: unknown payload kind on the monitor PUB — counted and ignored"
                    );
                }
            }
        }
    }
}

/// SUB の poll 待ち上限(ms)。停止合図の確認周期でもある(decoder / root_sink と同じ 100 ms 級)。
const SUB_POLL_TIMEOUT_MS: i64 = 100;

fn spawn_sub_thread(
    endpoint: String,
    hub: Arc<Hub>,
    geometry: Arc<Geometry>,
    stop: Arc<AtomicBool>,
) -> Result<std::thread::JoinHandle<()>, MonitorError> {
    std::thread::Builder::new()
        .name("monitor-sub".to_string())
        .spawn(move || {
            let context = zmq::Context::new();
            let socket = match context.socket(zmq::SUB) {
                Ok(socket) => socket,
                Err(e) => {
                    error!(error = %e, "monitor: cannot create the SUB socket");
                    return;
                }
            };
            // 取りこぼしは seq のギャップとして可視化する(SPEC §2.2)。
            if let Err(e) = crate::zmq_helper::apply_sub_hwm(&socket) {
                error!(error = %e, "monitor: cannot set the SUB high-water mark");
                return;
            }
            if let Err(e) = socket.set_subscribe(b"") {
                // トピックフレームなし(SPEC §2.1)= 全部購読。
                error!(error = %e, "monitor: cannot subscribe");
                return;
            }
            if let Err(e) = socket.set_rcvtimeo(SUB_POLL_TIMEOUT_MS as i32) {
                error!(error = %e, "monitor: cannot set the SUB receive timeout");
                return;
            }
            if let Err(e) = socket.set_linger(0) {
                error!(error = %e, "monitor: cannot set SUB linger");
                return;
            }
            if let Err(e) = socket.connect(&endpoint) {
                error!(endpoint, error = %e, "monitor: cannot connect the SUB socket");
                return;
            }
            info!(endpoint, "monitor: subscribed to the root-sink monitor PUB");

            let mut ingest = Ingest::new(hub, geometry);
            let mut buffer = zmq::Message::new();
            while !stop.load(Ordering::Relaxed) {
                match socket.recv(&mut buffer, 0) {
                    Ok(()) => ingest.handle(&buffer),
                    Err(zmq::Error::EAGAIN) => continue,
                    Err(zmq::Error::ETERM) => break,
                    Err(e) => {
                        error!(error = %e, "monitor: SUB receive failed");
                        break;
                    }
                }
            }
            info!("monitor: SUB thread stopped");
        })
        .map_err(MonitorError::Spawn)
}

// ---------------------------------------------------------------------
// WS 側(axum)
// ---------------------------------------------------------------------

/// WS ルータ(SPEC §10。monitor は **REP を持たない** — 純コンシューマ)。
///
/// `/` と `/ws` の両方で upgrade を受ける(SPEC は WS のパスを定めていない。
/// ポート 9000 は monitor 専用なので、UI がどちらを叩いても繋がるようにしておく)。
fn ws_router(hub: Arc<Hub>) -> Router {
    Router::new()
        .route("/", get(ws_upgrade))
        .route("/ws", get(ws_upgrade))
        .with_state(hub)
}

async fn ws_upgrade(State(hub): State<Arc<Hub>>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| serve_client(socket, hub))
}

/// 1 クライアント分の面倒を見る。
///
/// 送信(live + JSON)と受信(subscribe)は同じソケットを触るので split して 2 つに分ける。
/// どちらかが終わればもう片方も畳んで、クライアントを掃除する。
async fn serve_client(socket: WebSocket, hub: Arc<Hub>) {
    let (mut sink, mut stream) = socket.split();
    let (id, streams, mut json_rx, mut live_rx) = hub.register();
    let counters = Arc::clone(&hub.counters);
    let meta = hub.meta_json();

    let writer = tokio::spawn(async move {
        // 接続時の 1 通目は meta(SPEC §10.3)。
        if let Some(text) = meta {
            if sink.send(WsMessage::Text(text.into())).await.is_err() {
                return;
            }
        }
        loop {
            tokio::select! {
                json = json_rx.recv() => match json {
                    Some(text) => {
                        if sink.send(WsMessage::Text(text.into())).await.is_err() {
                            return;
                        }
                    }
                    None => return, // hub がこのクライアントを切った
                },
                live = live_rx.recv() => match live {
                    Ok(frame) => {
                        if !streams.wants(frame.stream) {
                            continue;
                        }
                        if sink.send(WsMessage::Binary(frame.bytes.clone())).await.is_err() {
                            return;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(missed)) => {
                        // drop-oldest(SPEC §10.3)。落ちた分は必ず数える。
                        counters.ws_dropped.fetch_add(missed, Ordering::Relaxed);
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                },
            }
        }
    });

    while let Some(Ok(message)) = stream.next().await {
        match message {
            WsMessage::Text(text) => hub.apply_subscribe(id, &text),
            WsMessage::Close(_) => break,
            _ => {}
        }
    }

    writer.abort();
    hub.remove_client(id);
}

/// monitor を起動し、`shutdown` が来るまで WS を提供する(SPEC §5.4 / §10)。
///
/// `bound` は実 bind 先を 1 度だけ通知する(ポート 0 を使うテスト用。production は `None`)。
pub async fn run_monitor(
    params: MonitorParams,
    mut shutdown: broadcast::Receiver<()>,
    bound: Option<oneshot::Sender<SocketAddr>>,
) -> Result<(), MonitorError> {
    let geometry =
        Arc::new(
            geometry::load(&params.geometry).map_err(|source| MonitorError::Geometry {
                path: params.geometry.clone(),
                source,
            })?,
        );
    // ジオメトリの素性ログ(root_sink.cxx と同じ項目)。
    info!(
        geometry = %params.geometry.display(),
        cobos = geometry.cobo_count(),
        max_strip_u = geometry.max_strip[0],
        max_strip_v = geometry.max_strip[1],
        max_strip_w = geometry.max_strip[2],
        duplicates = geometry.duplicate_warnings().len(),
        malformed = geometry.malformed_lines().len(),
        "monitor: geometry loaded"
    );

    let meta = Meta {
        n_buckets: N_BUCKETS,
        planes: geometry.max_strip,
        geometry: params.geometry.display().to_string(),
        angles_deg: geometry.header.angles_deg,
        detector: params.detector.clone(),
        cobos: params.cobos.clone(),
        run: 0,
    };
    let counters = Arc::new(MonitorCounters::default());
    let hub = Hub::new(meta, params.live_queue, Arc::clone(&counters));

    let stop = Arc::new(AtomicBool::new(false));
    let sub_thread = spawn_sub_thread(
        params.sub_endpoint.clone(),
        Arc::clone(&hub),
        Arc::clone(&geometry),
        Arc::clone(&stop),
    )?;

    let listener = tokio::net::TcpListener::bind(&params.ws_listen)
        .await
        .map_err(|source| MonitorError::Bind {
            listen: params.ws_listen.clone(),
            source,
        })?;
    let local = listener.local_addr().map_err(|source| MonitorError::Bind {
        listen: params.ws_listen.clone(),
        source,
    })?;
    info!(%local, "monitor WS listening");
    if let Some(tx) = bound {
        let _ = tx.send(local);
    }

    let result = axum::serve(listener, ws_router(Arc::clone(&hub)))
        .with_graceful_shutdown(async move {
            let _ = shutdown.recv().await;
        })
        .await
        .map_err(MonitorError::Serve);

    stop.store(true, Ordering::Relaxed);
    if sub_thread.join().is_err() {
        error!("monitor: SUB thread panicked");
    }
    info!(
        messages_in = counters.messages_in.load(Ordering::Relaxed),
        monitor_gaps = counters.monitor_gaps.load(Ordering::Relaxed),
        ws_dropped = counters.ws_dropped.load(Ordering::Relaxed),
        unknown_kind = counters.unknown_kind.load(Ordering::Relaxed),
        decode_errors = counters.decode_errors.load(Ordering::Relaxed),
        clients_total = counters.clients_total.load(Ordering::Relaxed),
        clients_dropped_slow = counters.clients_dropped_slow.load(Ordering::Relaxed),
        unmapped_hits = geometry.unmapped_hit_count(),
        "monitor stopped"
    );
    result
}

// =====================================================================
// テスト(仕様書 — SPEC §10.1/§10.2 の表を直接写す)
// =====================================================================

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
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

    // ---- 投入データ(非対称に選ぶ)----

    /// 1 サンプル = (aget, raw_ch, bucket, adc)。
    type Sample = (u8, u8, u16, u16);

    fn fragment(cobo: u8, asad: u8, samples: &[Sample]) -> Fragment {
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

    fn built_event(
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
    const GEOMETRY_TEXT: &str = "\
ANGLES: 11.0 22.0 33.0
DIAMOND SIZE: 2.5
U\t0\t1\t0\t0\t0\t0\t0.0\t0.0\t10
U\t0\t2\t0\t0\t0\t1\t1.0\t0.1\t10
U\t1\t2\t0\t0\t0\t2\t2.0\t0.2\t10
V\t0\t1\t0\t0\t1\t0\t0.0\t0.0\t10
W\t0\t3\t0\t0\t1\t5\t0.0\t0.0\t10
TestAux0\t0\t0\t0\t4
";

    fn converter() -> DisplayConverter {
        let geometry = Arc::new(geometry::parse(GEOMETRY_TEXT));
        assert_eq!(geometry.max_strip, [2, 1, 3], "U2/V1/W3(面の幅)");
        DisplayConverter::new(geometry)
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

    // -----------------------------------------------------------------
    // 4. hist_snapshot → Histo1d / Histo2d(SPEC §10.2 の 0x10/0x11)
    // -----------------------------------------------------------------

    fn hist(id: u8, name: &str, nx: u32, ny: u32, values: &[f64]) -> HistPayload {
        HistPayload {
            id,
            name: name.to_string(),
            nx,
            ny,
            bins: ByteBuf::from(f64_bytes(values)),
        }
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
    // 5. ギャップ計数(SPEC §5.4)
    // -----------------------------------------------------------------

    #[test]
    fn gap_tracker_counts_missing_sequence_numbers_only() {
        let mut tracker = GapTracker::new();
        // 初回は基準点(ギャップではない)。
        assert_eq!(tracker.observe(100), 0);
        // 連続
        assert_eq!(tracker.observe(101), 0);
        assert_eq!(tracker.observe(102), 0);
        assert_eq!(tracker.gaps(), 0);
        // 飛び: 103,104 が落ちた = 2
        assert_eq!(tracker.observe(105), 2);
        assert_eq!(tracker.gaps(), 2);
        // また飛び: 106..109 が落ちた = 4(累積 6)
        assert_eq!(tracker.observe(110), 4);
        assert_eq!(tracker.gaps(), 6);
        // 巻き戻り(root-sink 再起動)はギャップにしない
        assert_eq!(tracker.observe(0), 0);
        assert_eq!(tracker.gaps(), 6);
        assert_eq!(tracker.restarts(), 1);
        assert_eq!(tracker.observe(1), 0, "再起動後は新しい系列として続く");
    }

    // -----------------------------------------------------------------
    // 6. §5.3 パーサ(3 種 + 未知 kind)
    // -----------------------------------------------------------------

    /// SPEC §2.2 のエンベロープ(positional array(5))+ §5.3 の map 形式ペイロードを
    /// 手で組む(root_sink の monitor_pub.hpp と同じバイト構成)。
    fn envelope(source_id: u32, run: u32, seq: u64, created_ns: u64, payload: &[u8]) -> Vec<u8> {
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

    fn status_payload() -> StatusPayload {
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

    #[test]
    fn decode_message_reads_the_three_kinds_from_the_named_wire() {
        let status = status_payload();
        let raw = envelope(
            101,
            7,
            42,
            1_755_000_000_000_000_000,
            &rmp_serde::to_vec_named(&status).unwrap(),
        );
        let message = decode_message(&raw).unwrap();
        assert_eq!(message.source_id, 101, "SPEC §3.2: root-sink PUB = 101");
        assert_eq!(message.run_number, 7);
        assert_eq!(message.sequence_number, 42);
        assert_eq!(message.created_ns, 1_755_000_000_000_000_000);
        assert_eq!(message.status(), Some(&status));

        let snapshot = HistSnapshotPayload {
            kind: "hist_snapshot".to_string(),
            run: 7,
            hists: vec![hist(1, "StripTimeU", 2, 2, &[1.0, 2.0, 3.0, 4.0])],
        };
        let raw = envelope(101, 7, 43, 1, &rmp_serde::to_vec_named(&snapshot).unwrap());
        let message = decode_message(&raw).unwrap();
        assert_eq!(message.hist_snapshot(), Some(&snapshot));
        assert_eq!(
            message.hist_snapshot().unwrap().hists[0].values(),
            vec![1.0, 2.0, 3.0, 4.0]
        );

        let event = built_event(7, 5, false, vec![fragment(0, 0, &[(1, 2, 3, 4)])]);
        let raw = envelope(101, 7, 44, 2, &rmp_serde::to_vec_named(&event).unwrap());
        let message = decode_message(&raw).unwrap();
        assert_eq!(message.built_event(), Some(&event));
    }

    #[test]
    fn decode_message_keeps_unknown_kinds_for_forward_compatibility() {
        #[derive(Serialize)]
        struct Future {
            kind: String,
            answer: u32,
        }
        let payload = Future {
            kind: "psu_reading".to_string(),
            answer: 42,
        };
        let raw = envelope(101, 3, 7, 9, &rmp_serde::to_vec_named(&payload).unwrap());
        let message = decode_message(&raw).unwrap();
        assert_eq!(message.sequence_number, 7, "エンベロープは読める");
        assert_eq!(
            message.payload,
            MonitorPayload::Unknown("psu_reading".to_string()),
            "未知 kind はエラーにせず名前を残す(数えて無視する材料)"
        );
    }

    #[test]
    fn decode_message_rejects_garbage_and_non_data_messages() {
        assert!(matches!(
            decode_message(&[0x00, 0x01, 0x02]),
            Err(WireError::Envelope { .. })
        ));
        let eos: crate::msg::Message<crate::msg::Fragments> = crate::msg::Message::EndOfStream {
            source_id: 101,
            run_number: 7,
        };
        assert!(matches!(
            decode_message(&eos.to_msgpack().unwrap()),
            Err(WireError::NotData)
        ));
    }

    // -----------------------------------------------------------------
    // 7. JSON(SPEC §10.3)
    // -----------------------------------------------------------------

    #[test]
    fn status_json_is_the_spec_5_3_status_plus_the_monitor_three() {
        let status = status_payload();
        let text = status_json(
            &status,
            MonitorStats {
                monitor_gaps: 12,
                clients: 3,
                ws_dropped: 45,
            },
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();

        assert_eq!(value["type"], "status");
        // §5.3 のフィールドがそのまま
        assert_eq!(value["run"], 7);
        assert_eq!(value["state"], "running");
        assert_eq!(value["events_built"], 11);
        assert_eq!(value["events_incomplete"], 2);
        assert_eq!(value["late_fragments"], 3);
        assert_eq!(value["pending_events"], 4);
        assert_eq!(value["frames_per_cobo"]["0"], 108);
        assert_eq!(value["bytes_written"], 123_456);
        assert_eq!(value["saturation"]["W"]["saturated"], 4);
        assert_eq!(value["saturation"]["W"]["counted"], 5);
        assert_eq!(value["publish_drops"], 9);
        // + monitor の 3 つ(SPEC §10.3)
        assert_eq!(value["monitorGaps"], 12);
        assert_eq!(value["clients"], 3);
        assert_eq!(value["wsDropped"], 45);
    }

    #[test]
    fn meta_json_matches_the_spec_10_3_shape() {
        let meta = Meta {
            n_buckets: 512,
            planes: [72, 92, 93],
            geometry: "config/geometry_mini_eTPC.dat".to_string(),
            angles_deg: Some([11.0, 22.0, 33.0]),
            detector: "mini_eTPC".to_string(),
            cobos: vec![0, 1],
            run: 57,
        };
        let value: serde_json::Value = serde_json::from_str(&meta_json(&meta).unwrap()).unwrap();
        assert_eq!(value["type"], "meta");
        assert_eq!(value["nBuckets"], 512);
        assert_eq!(value["planes"]["U"], 72);
        assert_eq!(value["planes"]["V"], 92);
        assert_eq!(value["planes"]["W"], 93);
        assert_eq!(value["geometry"], "config/geometry_mini_eTPC.dat");
        assert_eq!(value["anglesDeg"][0], 11.0);
        assert_eq!(value["anglesDeg"][2], 33.0);
        assert_eq!(value["detector"], "mini_eTPC");
        assert_eq!(value["cobos"][1], 1);
        assert_eq!(value["run"], 57);

        // ANGLES を持たないジオメトリでは null(発注書「無ければ null」)。
        let mut without = meta.clone();
        without.angles_deg = None;
        let value: serde_json::Value = serde_json::from_str(&meta_json(&without).unwrap()).unwrap();
        assert!(value["anglesDeg"].is_null());
    }

    #[test]
    fn run_json_carries_the_state_run_and_timestamp() {
        let value: serde_json::Value =
            serde_json::from_str(&run_json("running", 8, "2026-08-14T10:00:00.123+09:00").unwrap())
                .unwrap();
        assert_eq!(value["type"], "run");
        assert_eq!(value["state"], "running");
        assert_eq!(value["run"], 8);
        assert_eq!(value["ts"], "2026-08-14T10:00:00.123+09:00");
    }

    #[test]
    fn subscribe_defaults_to_everything_but_waveforms() {
        // SPEC §10.3: 既定は waveforms **以外** ON。
        let default = StreamSet::default();
        assert!(default.uvw && default.histos && default.status);
        assert!(!default.waveforms, "波形は開いたクライアントだけ");
        assert!(default.wants(Stream::Uvw));
        assert!(!default.wants(Stream::Waveforms));
        assert!(default.wants(Stream::Histos));
    }

    #[test]
    fn parse_subscribe_reads_the_streams_array() {
        let parsed =
            parse_subscribe(r#"{"streams":["uvw","waveforms","histos","status"]}"#).unwrap();
        assert_eq!(
            parsed.streams,
            StreamSet {
                uvw: true,
                waveforms: true,
                histos: true,
                status: true
            }
        );
        assert!(parsed.unknown.is_empty());

        // 配列に無いものは OFF(「波形を閉じた」が表現できる)
        let parsed = parse_subscribe(r#"{"type":"subscribe","streams":["status"]}"#).unwrap();
        assert_eq!(
            parsed.streams,
            StreamSet {
                uvw: false,
                waveforms: false,
                histos: false,
                status: true
            }
        );

        // 知らない名前は数えるために残す(捨てるが silent にしない)
        let parsed = parse_subscribe(r#"{"streams":["uvw","tracks3d"]}"#).unwrap();
        assert!(parsed.streams.uvw);
        assert_eq!(parsed.unknown, vec!["tracks3d".to_string()]);

        // subscribe ではないもの
        assert!(parse_subscribe(r#"{"cmd":"start"}"#).is_none());
        assert!(parse_subscribe("not json").is_none());
        assert!(parse_subscribe(r#"{"streams":"uvw"}"#).is_none());
    }

    // -----------------------------------------------------------------
    // 8. 設定 → 起動パラメタ(SPEC §3.2 の既定表)
    // -----------------------------------------------------------------

    #[test]
    fn params_come_from_the_config_including_the_spec_defaults() {
        // `[monitor]` を空にして SPEC §3.2 の既定(PUB 47004 / WS 9000)が入ることを見る。
        // geometry はリポ内フィクスチャ(config は実在確認をする)。
        let toml_str = r#"
[system]
experiment = "mini_eTPC"
output_root = "/data/tpcdaq"
geometry = "tests/fixtures/geometry_mini_reduced.dat"

[[cobo]]
id = 0
data_sender_id = "CoBo[0]"

[[cobo]]
id = 1
data_sender_id = "CoBo[1]"

[decoder]
workers = 1

[root_sink]
snapshot_hz = 1.0
event_publish_hz = 20.0
build_timeout_ms = 1000

[monitor]

[controller]
passphrase = "change-me"
ecc_proxy = "Ecc:tcp -h 127.0.0.1 -p 46002"
config_id = "default"
"#;
        let config = crate::config::parse(toml_str).unwrap();
        let params = MonitorParams::from_config(&config);

        assert_eq!(params.sub_endpoint, "tcp://127.0.0.1:47004");
        assert_eq!(params.ws_listen, "0.0.0.0:9000");
        assert_eq!(
            params.geometry,
            PathBuf::from("tests/fixtures/geometry_mini_reduced.dat"),
            "[monitor] geometry 省略時は [system] geometry"
        );
        assert_eq!(params.live_queue, 64);
        assert_eq!(params.detector, "mini_eTPC", "meta.detector = experiment");
        assert_eq!(params.cobos, vec![0, 1], "meta.cobos = [[cobo]] の id");
    }

    // -----------------------------------------------------------------
    // 9. §10.4-1 のサンプルストリーム(決定性)
    // -----------------------------------------------------------------

    #[test]
    fn ws_sample_messages_are_deterministic_and_cover_every_type() {
        let first = ws_sample_messages();
        let second = ws_sample_messages();
        assert_eq!(first, second, "同一入力 → 同一バイト");

        let types: Vec<u8> = first.iter().map(|m| m[2]).collect();
        assert_eq!(
            types,
            vec![
                ws::TYPE_UVW,
                ws::TYPE_WAVEFORMS,
                ws::TYPE_HISTO1D,
                ws::TYPE_HISTO2D
            ],
            "全メッセージ型が 1 通ずつ"
        );
        for message in &first {
            assert_eq!(&message[0..2], b"TP", "magic");
            assert_eq!(message[3], ws::VERSION);
        }
        // incomplete の例が入っていること(flags bit0 の検証材料)。
        assert_eq!(first[1][4], ws::FLAG_INCOMPLETE);
    }
}
