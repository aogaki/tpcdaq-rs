use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::SystemTime;

use axum::body::Bytes;
use tokio::sync::{broadcast, mpsc};
use tracing::{error, info, warn};

use super::*;

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
    /// slowctl の `slow` ペイロード受信数(SPEC §2.3 v1.31)。
    pub slow_in: AtomicU64,
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
    /// `GET /api/last-event` の要求数(200 / 404 とも)。
    pub last_event_requests: AtomicU64,
    /// 表示変換([`ConvertCounts`])の ZS 復元カウンタの写し(TODO/152)。SUB スレッドが
    /// 0x02 を作るたびに store し、`status` JSON と停止ログが読む。
    pub uvw_zs_fragments_restored: AtomicU64,
    pub uvw_zs_fragments_no_table: AtomicU64,
    pub uvw_table_missing_channels: AtomicU64,
}

impl MonitorCounters {
    pub(super) fn bump(counter: &AtomicU64, by: u64) {
        counter.fetch_add(by, Ordering::Relaxed);
    }
}

/// live 1 通(全クライアント共有 — [`Bytes`] なのでクライアント毎のコピーは参照数だけ)。
#[derive(Debug)]
pub(super) struct LiveFrame {
    pub(super) stream: Stream,
    pub(super) bytes: Bytes,
}

/// クライアント毎の購読フラグ(WS 受信タスクが書き、送信タスクが読む)。
#[derive(Debug)]
pub(super) struct ClientStreams {
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

    pub(super) fn wants(&self, stream: Stream) -> bool {
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
    pub(super) counters: Arc<MonitorCounters>,
    /// waveforms を購読しているクライアント数(0 なら 0x03 を**作らない** — 帯域と CPU)。
    waveform_demand: AtomicUsize,
    meta: Mutex<Meta>,
    /// `meta` の run(直近の status 由来)。
    meta_run: AtomicU32,
    /// `stripGeometry` JSON(起動時に 1 回作る。接続時に `meta` の次に送る — TODO/125)。
    /// JSON 化に失敗したときだけ `None`(起動時に error ログ済み)。
    pub(super) strip_geometry: Option<String>,
    /// 直近に見た `(run, state)`(遷移検知用)。
    last_run_state: Mutex<Option<(u32, String)>>,
    /// slowctl の最新 `slow` ペイロード(SPEC §10.3 v1.31)。購読していなければ `None` の
    /// まま = `status` JSON に `slow` は出ない(従来と同一の JSON)。
    last_slow: Mutex<Option<crate::slowctl::SlowPayload>>,
    /// 直近に受けた built event と受信時刻(`GET /api/last-event` 用、TODO/123)。
    /// WS 購読の有無に関係なく常に差し替える(Arc の差し替えだけ。JSON 化は要求時)。
    pub(super) last_event: Mutex<Option<(Arc<BuiltEventPayload>, SystemTime)>>,
}

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // 毒された Mutex でも monitor を止めない(中身は monitor 自身が作ったものだけ)。
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Hub {
    pub(super) fn new(
        meta: Meta,
        strip_geometry: Option<String>,
        live_queue: usize,
        counters: Arc<MonitorCounters>,
    ) -> Arc<Hub> {
        let (live, _) = broadcast::channel(live_queue.max(1));
        Arc::new(Hub {
            live,
            clients: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(0),
            counters,
            waveform_demand: AtomicUsize::new(0),
            meta: Mutex::new(meta),
            meta_run: AtomicU32::new(0),
            strip_geometry,
            last_run_state: Mutex::new(None),
            last_slow: Mutex::new(None),
            last_event: Mutex::new(None),
        })
    }

    fn stats(&self) -> MonitorStats {
        MonitorStats {
            monitor_gaps: self.counters.monitor_gaps.load(Ordering::Relaxed),
            clients: self.counters.clients.load(Ordering::Relaxed),
            ws_dropped: self.counters.ws_dropped.load(Ordering::Relaxed),
            uvw_zs_fragments_restored: self
                .counters
                .uvw_zs_fragments_restored
                .load(Ordering::Relaxed),
            uvw_zs_fragments_no_table: self
                .counters
                .uvw_zs_fragments_no_table
                .load(Ordering::Relaxed),
            uvw_table_missing_channels: self
                .counters
                .uvw_table_missing_channels
                .load(Ordering::Relaxed),
        }
    }

    /// 現在の `meta` JSON(接続時 / run 変化時に送るもの)。
    pub(super) fn meta_json(&self) -> Option<String> {
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
    pub(super) fn waveforms_wanted(&self) -> bool {
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
    pub(super) fn publish_live(&self, stream: Stream, frames: &mut Vec<Vec<u8>>) {
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

    pub(super) fn register(
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

    pub(super) fn remove_client(&self, id: u64) {
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
    pub(super) fn on_status(&self, status: &StatusPayload) {
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

        let slow = lock(&self.last_slow).clone();
        match status_json(status, self.stats(), slow.as_ref()) {
            Ok(text) => self.broadcast_json(&text, true),
            Err(e) => error!(error = %e, "monitor: cannot encode the status message"),
        }
    }

    /// slowctl の `slow` を 1 通受けて保持する(SPEC §10.3 v1.31)。
    ///
    /// **ここでは WS へ配らない** —— `slow` は root-sink の 1 Hz `status` に同梱して出る
    /// (新しい WS メッセージ型を作らない、SPEC §2.1 の「受け皿は既存の口」)。
    pub(super) fn on_event(&self, event: Arc<BuiltEventPayload>) {
        *lock(&self.last_event) = Some((event, SystemTime::now()));
    }

    pub(super) fn on_slow(&self, slow: &crate::slowctl::SlowPayload) {
        *lock(&self.last_slow) = Some(slow.clone());
    }

    /// C→S の `subscribe` を適用する。
    pub(super) fn apply_subscribe(&self, id: u64, text: &str) {
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
