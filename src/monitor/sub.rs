use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tracing::{error, info, warn};

use super::*;
use crate::geometry::Geometry;

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
    /// **送り手ごと**の seq 追跡(SPEC §2.2 の seq はソース毎の系列)。
    /// v1.31 で SUB に slowctl(source_id 200)が加わり、root-sink(101)と seq が
    /// 交互に来るようになったため、1 本の `GapTracker` では飛びも巻き戻りも意味を失う。
    gaps: std::collections::BTreeMap<u32, GapTracker>,
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
            gaps: std::collections::BTreeMap::new(),
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

        let tracker = self.gaps.entry(message.source_id).or_default();
        let missed = tracker.observe(message.sequence_number);
        if missed > 0 {
            MonitorCounters::bump(&counters.monitor_gaps, missed);
        }
        let restarts: u64 = self.gaps.values().map(GapTracker::restarts).sum();
        counters.source_restarts.store(restarts, Ordering::Relaxed);

        match message.payload {
            MonitorPayload::Status(status) => {
                MonitorCounters::bump(&counters.status_in, 1);
                self.hub.on_status(&status);
            }
            MonitorPayload::HistSnapshot(snapshot) => {
                MonitorCounters::bump(&counters.snapshots_in, 1);
                self.frames.clear();
                self.converter.histos(&snapshot, &mut self.frames);
                self.hub.publish_live(Stream::Histos, &mut self.frames);
            }
            MonitorPayload::BuiltEvent(event) => {
                MonitorCounters::bump(&counters.events_in, 1);
                // Box → Arc は所有権の移動(items のバイト列はコピーしない)。
                let event: Arc<BuiltEventPayload> = Arc::from(event);
                self.hub.on_event(Arc::clone(&event));
                self.frames.clear();
                self.converter.uvw(&event, &mut self.frames);
                self.hub.publish_live(Stream::Uvw, &mut self.frames);
                // 0x03 は**購読しているクライアントが居るときだけ**作る(SPEC §10.3 の帯域制御)。
                if self.hub.waveforms_wanted() {
                    self.frames.clear();
                    self.converter.waveforms(&event, &mut self.frames);
                    self.hub.publish_live(Stream::Waveforms, &mut self.frames);
                }
            }
            MonitorPayload::Slow(slow) => {
                MonitorCounters::bump(&counters.slow_in, 1);
                self.hub.on_slow(&slow);
            }
            MonitorPayload::Unknown(kind) => {
                MonitorCounters::bump(&counters.unknown_kind, 1);
                if !self.logged_unknown_kind {
                    self.logged_unknown_kind = true;
                    info!(
                        kind = %kind,
                        "monitor: unknown payload kind on the monitor PUB — counted and ignored"
                    );
                }
            }
        }
    }
}

/// SUB の poll 待ち上限(ms)。停止合図の確認周期でもある(decoder / root_sink と同じ 100 ms 級)。
const SUB_POLL_TIMEOUT_MS: i64 = 100;

pub(super) fn spawn_sub_thread(
    endpoint: String,
    slow_endpoint: Option<String>,
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
            if let Err(e) = socket.set_linger(0) {
                error!(error = %e, "monitor: cannot set SUB linger");
                return;
            }
            if let Err(e) = socket.connect(&endpoint) {
                error!(endpoint, error = %e, "monitor: cannot connect the SUB socket");
                return;
            }
            info!(endpoint, "monitor: subscribed to the root-sink monitor PUB");
            if let Some(slow_endpoint) = &slow_endpoint {
                // SPEC §3.1 v1.31: slowctl の PUB を **同じ SUB ソケットに追加 connect**。
                if let Err(e) = socket.connect(slow_endpoint) {
                    error!(endpoint = slow_endpoint, error = %e,
                           "monitor: cannot connect the SUB socket to slowctl");
                    return;
                }
                info!(
                    endpoint = slow_endpoint,
                    "monitor: subscribed to the slowctl status PUB"
                );
            }

            let mut ingest = Ingest::new(hub, geometry);
            while !stop.load(Ordering::Relaxed) {
                // **recv の前に必ず poll する**(TODO/013 の fair-queue 飢餓)。libzmq は
                // `recv` が成功し続ける間 `process_commands()` を 100 通に 1 度しか走らせず、
                // fair-queue から外れた上流(= もう 1 本の PUB)がそのぶん戻れない。
                // v1.31 で上流が 2 本になったので、単一上流時代の `recv` 直呼びは使えない。
                let readable = {
                    let mut items = [socket.as_poll_item(zmq::POLLIN)];
                    match zmq::poll(&mut items, SUB_POLL_TIMEOUT_MS) {
                        Ok(_) => items[0].is_readable(),
                        Err(zmq::Error::ETERM) => break,
                        // シグナルによる中断は失敗ではない(次の周回で待ち直す)。
                        Err(zmq::Error::EINTR) => false,
                        Err(e) => {
                            error!(error = %e, "monitor: SUB poll failed");
                            break;
                        }
                    }
                };
                if !readable {
                    continue;
                }
                // 待つのは poll の仕事なので recv はノンブロッキング。
                match socket.recv_bytes(zmq::DONTWAIT) {
                    Ok(raw) => ingest.handle(&raw),
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::monitor::test_util::*;
    use std::path::Path;

    /// v1.31 で SUB の上流が 2 本になった —— seq は**ソース毎**に追う(混ぜない)。
    #[test]
    fn sequence_gaps_are_tracked_per_source_id() {
        let counters = Arc::new(MonitorCounters::default());
        let geometry = Arc::new(
            geometry::load(Path::new("tests/fixtures/geometry_mini_reduced.dat"))
                .expect("fixture geometry"),
        );
        let hub = Hub::new(
            Meta {
                n_buckets: 512,
                planes: [4, 3, 4],
                geometry: "tests/fixtures/geometry_mini_reduced.dat".to_string(),
                angles_deg: None,
                detector: "mini_eTPC".to_string(),
                cobos: vec![0],
                run: 7,
            },
            None,
            4,
            Arc::clone(&counters),
        );
        let mut ingest = Ingest::new(hub, geometry);

        let slow = crate::slowctl::SlowPayload {
            kind: crate::slowctl::SLOW_KIND.to_string(),
            ts_ms: 1,
            samples: Vec::new(),
            counters: crate::slowctl::SlowCounters::default(),
        };
        let status = status_payload();

        // root-sink(101)は 10, 11 と連続、slowctl(200)は 0, 1 と連続。
        ingest.handle(&envelope(
            101,
            7,
            10,
            0,
            &rmp_serde::to_vec_named(&status).unwrap(),
        ));
        ingest.handle(&envelope(
            200,
            0,
            0,
            0,
            &rmp_serde::to_vec_named(&slow).unwrap(),
        ));
        ingest.handle(&envelope(
            101,
            7,
            11,
            0,
            &rmp_serde::to_vec_named(&status).unwrap(),
        ));
        ingest.handle(&envelope(
            200,
            0,
            1,
            0,
            &rmp_serde::to_vec_named(&slow).unwrap(),
        ));
        assert_eq!(
            counters.monitor_gaps.load(Ordering::Relaxed),
            0,
            "系列を混ぜたら 10→0→11→1 が巨大なギャップに見えてしまう"
        );
        assert_eq!(counters.source_restarts.load(Ordering::Relaxed), 0);
        assert_eq!(counters.slow_in.load(Ordering::Relaxed), 2);

        // root-sink だけ 2 通落ちた(12, 13)。
        ingest.handle(&envelope(
            101,
            7,
            14,
            0,
            &rmp_serde::to_vec_named(&status).unwrap(),
        ));
        assert_eq!(counters.monitor_gaps.load(Ordering::Relaxed), 2);
    }
}
