//! monitor — root-sink のモニタ PUB を購読し、表示用に変換して WS で配るコンポーネント。
//! SPEC §5.3(PUB ワイヤ — 受け側)/ §5.4(責務)/ §10(WS プロトコル)/ §3.2(WS 9000、PUB 47004)。
//!
//! # 構成
//!
//! ```text
//!   root-sink ──PUB(47004)──▶ [SUB 専用 OS スレッド]
//!                                 │  decode(§5.3)→ ギャップ計数 → 表示変換(§5.4)
//!                                 │  → 同じ格子で飛跡の再構成(`crate::reco`、0x04 — TODO/127)
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

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::{broadcast, oneshot};
use tracing::{error, info, warn};

use crate::config::Config;
use crate::geometry::{self, Geometry};
use crate::reco::RecoGeometry;
use crate::strip_xy;

/// 1 チャンネルの time bucket 数(GET ハードウェア定数。SPEC §2.4 の bucket 0–511)。
pub const N_BUCKETS: u16 = 512;

/// 平面の数(U/V/W)。
pub const N_PLANES: usize = 3;

pub mod ws;

mod display;
mod http;
mod hub;
mod json;
mod sample;
mod sub;
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test_util;
mod wire;

use http::ws_router;
use sub::spawn_sub_thread;

pub use display::*;
pub use hub::*;
pub use json::*;
pub use sample::*;
pub use wire::*;

/// f64 LE の 8 バイトを読む(長さが足りなければ 0.0)。
fn f64_le(chunk: &[u8]) -> f64 {
    match chunk.try_into() {
        Ok(bytes) => f64::from_le_bytes(bytes),
        Err(_) => 0.0,
    }
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
    /// slowctl の status PUB への SUB 接続先(SPEC §3.1 v1.31)。
    /// **`None` = 購読しない**(WS の `status` JSON に `slow` は出ない = 従来と同一)。
    pub slow_sub: Option<String>,
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
            slow_sub: config.monitor.slow_sub.clone(),
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

/// 起動時に 1 回: strip 線分の mm を作って `stripGeometry` JSON(TODO/125)と、
/// 飛跡の再構成に使う [`RecoGeometry`](TODO/127)にする。
///
/// 線分にしなかった行・差し替えた行・作れなかった理由は**ここで warn を 1 回ずつ**出す
/// (silent 禁止)。作れなくても monitor は止めない(UI に `unavailable` を見せる)。
/// そのときは再構成もしない(`RecoGeometry` が `None` = 0x04 を送らない)。
fn strip_geometry_at_startup(geometry: &Geometry) -> (Option<String>, Option<RecoGeometry>) {
    let built = match strip_xy::strip_geometry(geometry) {
        Ok((built, notes)) => {
            if !notes.short_rows.is_empty() {
                warn!(
                    rows = notes.short_rows.len(),
                    lines = ?notes.short_rows,
                    "monitor: geometry rows with LEN_PADS < 1 are not strip segments (TPCReco drops them too)"
                );
            }
            if !notes.replaced_rows.is_empty() {
                warn!(
                    rows = notes.replaced_rows.len(),
                    lines = ?notes.replaced_rows,
                    "monitor: repeated (plane, section, strip) rows — the later row wins (TPCReco mapByStrip)"
                );
            }
            info!(
                u = built.planes.u.segments.len(),
                v = built.planes.v.segments.len(),
                w = built.planes.w.segments.len(),
                "monitor: strip segments ready for the XY projection (stripGeometry)"
            );
            Ok(built)
        }
        Err(reason) => {
            warn!(
                %reason,
                "monitor: stripGeometry unavailable — the UI shows the reason; \
                 track reconstruction (0x04) is off"
            );
            Err(reason)
        }
    };
    let reco = built.as_ref().ok().map(RecoGeometry::new);
    let json = match strip_geometry_json(&built) {
        Ok(text) => Some(text),
        Err(e) => {
            error!(error = %e, "monitor: cannot encode the stripGeometry message");
            None
        }
    };
    (json, reco)
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
    let (strip_geometry, reco_geometry) = strip_geometry_at_startup(&geometry);
    let counters = Arc::new(MonitorCounters::default());
    let hub = Hub::new(
        meta,
        strip_geometry,
        params.live_queue,
        Arc::clone(&counters),
    );

    let stop = Arc::new(AtomicBool::new(false));
    let sub_thread = spawn_sub_thread(
        params.sub_endpoint.clone(),
        params.slow_sub.clone(),
        Arc::clone(&hub),
        Arc::clone(&geometry),
        reco_geometry,
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

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
}
