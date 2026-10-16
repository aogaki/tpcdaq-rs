use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tracing::{error, info, warn};

use super::*;
use crate::reco::{Reco, RecoGeometry};

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
    /// 飛跡の再構成(TODO/127)。stripGeometry が作れなかったときは `None` = reco も 0x04 も無し。
    reco: Option<Reco>,
    /// 再構成で、ヒットがあったのに stripGeometry に線分の無い strip の累計(終了ログに出す)。
    reco_no_segment: u64,
    /// **送り手ごと**の seq 追跡(SPEC §2.2 の seq はソース毎の系列)。
    /// v1.31 で SUB に slowctl(source_id 200)が加わり、root-sink(101)と seq が
    /// 交互に来るようになったため、1 本の `GapTracker` では飛びも巻き戻りも意味を失う。
    gaps: std::collections::BTreeMap<u32, GapTracker>,
    /// 変換結果の入れ物。**使い回す**(ホットパスで確保し直さない)。
    frames: Vec<Vec<u8>>,
    logged_decode_error: bool,
    logged_unknown_kind: bool,
    logged_no_segment: bool,
    logged_track_encode: bool,
}

impl Ingest {
    /// `converter` は起動時に組んだもの(pedestal 表の前計算込み — TODO/152)。
    fn new(hub: Arc<Hub>, converter: DisplayConverter, reco: Option<RecoGeometry>) -> Self {
        Self {
            hub,
            converter,
            reco: reco.map(Reco::new),
            reco_no_segment: 0,
            gaps: std::collections::BTreeMap::new(),
            frames: Vec::new(),
            logged_decode_error: false,
            logged_unknown_kind: false,
            logged_no_segment: false,
            logged_track_encode: false,
        }
    }

    /// 0x02 を作った直後に、**同じ格子**で再構成して 0x04 を `frames` に足す(TODO/127)。
    /// 単一スレッドなので live の並びは 0x02 ×3 → 0x04。
    fn push_track(&mut self, event: &BuiltEventPayload) {
        let Some(reco) = &mut self.reco else {
            return; // stripGeometry 無し(起動時に warn 済み)
        };
        reco.run(self.converter.grids(), N_BUCKETS as usize);
        let result = reco.event();
        if result.no_segment > 0 {
            self.reco_no_segment += result.no_segment as u64;
            if !self.logged_no_segment {
                self.logged_no_segment = true;
                warn!(
                    strips = result.no_segment,
                    "monitor: track reco skipped strips with a hit but no stripGeometry segment \
                     (counted; total in the stop log)"
                );
            }
        }
        let header = ws::Header::event(ws::TYPE_TRACK, event.run, event.event_idx, !event.complete);
        match ws::track(header, &result) {
            Ok(bytes) => self.frames.push(bytes),
            Err(e) => {
                if !self.logged_track_encode {
                    self.logged_track_encode = true;
                    error!(error = %e, "monitor: cannot encode the 0x04 Track message");
                }
            }
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
                // ZS 復元のカウンタ(TODO/152)を status JSON / 停止ログが読む場所へ写す。
                let convert = self.converter.counts();
                counters
                    .uvw_zs_fragments_restored
                    .store(convert.uvw_zs_fragments_restored, Ordering::Relaxed);
                counters
                    .uvw_zs_fragments_no_table
                    .store(convert.uvw_zs_fragments_no_table, Ordering::Relaxed);
                counters
                    .uvw_table_missing_channels
                    .store(convert.uvw_table_missing_channels, Ordering::Relaxed);
                // 0x04 は `uvw` ストリームに含める(SPEC §10.3 — 新しいストリーム名は作らない)。
                self.push_track(&event);
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
    converter: DisplayConverter,
    reco: Option<RecoGeometry>,
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

            let mut ingest = Ingest::new(hub, converter, reco);
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
            info!(
                track_no_segment_strips = ingest.reco_no_segment,
                "monitor: SUB thread stopped"
            );
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
        let mut ingest = Ingest::new(hub, DisplayConverter::new(geometry), None);

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

    // -----------------------------------------------------------------
    // 飛跡の再構成のフック(TODO/127)
    // -----------------------------------------------------------------

    const MINI_REDUCED: &str = "tests/fixtures/geometry_mini_reduced.dat";

    /// `geometry_mini_reduced.dat`(合成)で Ingest を作り、live の受け口も返す。
    /// stripGeometry は起動時と同じ [`strip_geometry_at_startup`] で作る。
    fn ingest_with(
        text: Option<&str>,
    ) -> (
        Ingest,
        tokio::sync::broadcast::Receiver<Arc<LiveFrame>>,
        bool,
    ) {
        let geometry = Arc::new(match text {
            Some(text) => geometry::parse(text),
            None => geometry::load(Path::new(MINI_REDUCED)).expect("fixture geometry"),
        });
        let (_json, reco) = strip_geometry_at_startup(&geometry);
        let has_reco = reco.is_some();
        let hub = Hub::new(
            Meta {
                n_buckets: 512,
                planes: geometry.max_strip,
                geometry: MINI_REDUCED.to_string(),
                angles_deg: None,
                detector: "mini_eTPC".to_string(),
                cobos: vec![0],
                run: 0,
            },
            None,
            16,
            Arc::new(MonitorCounters::default()),
        );
        let (_id, _streams, _json_rx, live_rx) = hub.register();
        (
            Ingest::new(hub, DisplayConverter::new(geometry), reco),
            live_rx,
            has_reco,
        )
    }

    /// U 1–4 / V 1–3 / W 1–4 の全 strip に bucket 200 の山(1000/1500/1000 → q = 3500 ≥ 2000)。
    /// フィクスチャの ch: U strip k = (aget0, raw k−1)、V strip k = (aget1, raw k−1)、
    /// W strip k = (aget1, raw k+2)(信号 0–10 は raw == 信号番号)。
    fn every_strip_pulsed(run: u32, event_idx: u32, complete: bool) -> Vec<u8> {
        let mut samples: Vec<Sample> = Vec::new();
        let strips: [(u8, u8, u8); 3] = [(0, 0, 4), (1, 0, 3), (1, 3, 4)]; // (aget, 最初の raw, 本数)
        for (aget, first, n) in strips {
            for raw in first..first + n {
                for (bucket, adc) in [(199, 1000), (200, 1500), (201, 1000)] {
                    samples.push((aget, raw, bucket, adc));
                }
            }
        }
        let event = built_event(run, event_idx, complete, vec![fragment(0, 0, &samples)]);
        envelope(101, run, 0, 0, &rmp_serde::to_vec_named(&event).unwrap())
    }

    fn drain(rx: &mut tokio::sync::broadcast::Receiver<Arc<LiveFrame>>) -> Vec<Arc<LiveFrame>> {
        let mut frames = Vec::new();
        while let Ok(frame) = rx.try_recv() {
            frames.push(frame);
        }
        frames
    }

    fn u32_at(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
    }

    #[test]
    fn built_event_sends_three_uvw_then_one_track_from_the_same_grids() {
        let (mut ingest, mut live, has_reco) = ingest_with(None);
        assert!(
            has_reco,
            "フィクスチャには ANGLES / DIAMOND SIZE / REFERENCE POINT がある"
        );
        ingest.handle(&every_strip_pulsed(70, 1234, true));

        let frames = drain(&mut live);
        let types: Vec<u8> = frames.iter().map(|f| f.bytes[2]).collect();
        assert_eq!(types, vec![0x02, 0x02, 0x02, 0x04], "0x02 ×3 → 0x04");
        assert!(
            frames.iter().all(|f| f.stream == Stream::Uvw),
            "uvw ストリーム"
        );
        let track = &frames[3].bytes;
        assert_eq!(u32_at(track, 5), 70, "runNumber = イベントの run");
        assert_eq!(u32_at(track, 9), 1234, "eventNumber = 0x02 と同じ");
        assert_eq!(track[4], 0, "complete");
        // 同じ格子を見ている: halo cut 後 U 4 / V 3 / W 4 本 → V で足りない。
        // 帯内は U 4(同じ時刻 = 帯の中)、V 0(帯フィットをしない)、W 4。
        let body: serde_json::Value = serde_json::from_slice(&track[ws::HEADER_LEN..]).unwrap();
        assert_eq!(
            body,
            serde_json::json!({
                "ok": false,
                "reason": "V has 3 strip hits after the halo cut (needs 4)",
                "hits": [4, 0, 4],
                "unplaced": 0,
                "tracks": [],
                "points": [],
            })
        );
    }

    /// TS「incomplete はヘッダから引き継ぐ」の Rust 版: 0x04 の flags は 0x02 と同じくイベントから。
    #[test]
    fn track_message_carries_the_incomplete_flag_of_the_event() {
        let (mut ingest, mut live, _) = ingest_with(None);
        ingest.handle(&every_strip_pulsed(70, 1235, false));
        let frames = drain(&mut live);
        assert_eq!(frames.len(), 4);
        for frame in &frames {
            assert_eq!(
                frame.bytes[4],
                ws::FLAG_INCOMPLETE,
                "{:#04x}",
                frame.bytes[2]
            );
        }
    }

    /// TODO/152: 0x02 を作るたびに表示変換の ZS カウンタが `MonitorCounters` へ写る
    /// (`status` JSON と停止ログはそこを読む)。表なしの monitor に ZS らしいイベント 2 通。
    #[test]
    fn zs_counts_of_the_display_conversion_reach_the_monitor_counters() {
        let geometry = Arc::new(geometry::load(Path::new(MINI_REDUCED)).expect("fixture geometry"));
        let counters = Arc::new(MonitorCounters::default());
        let hub = Hub::new(
            Meta {
                n_buckets: 512,
                planes: geometry.max_strip,
                geometry: MINI_REDUCED.to_string(),
                angles_deg: None,
                detector: "mini_eTPC".to_string(),
                cobos: vec![0],
                run: 0,
            },
            None,
            16,
            Arc::clone(&counters),
        );
        let mut ingest = Ingest::new(hub, DisplayConverter::new(geometry), None);
        // U strip2(aget0 raw1)に pre-trigger 外の 1 サンプルだけ = ZS らしい。
        for seq in 0..2 {
            let event = built_event(
                70,
                seq as u32,
                true,
                vec![fragment(0, 0, &[(0, 1, 200, 400)])],
            );
            ingest.handle(&envelope(
                101,
                70,
                seq,
                0,
                &rmp_serde::to_vec_named(&event).unwrap(),
            ));
        }

        assert_eq!(
            counters.uvw_zs_fragments_no_table.load(Ordering::Relaxed),
            2
        );
        assert_eq!(
            counters.uvw_zs_fragments_restored.load(Ordering::Relaxed),
            0
        );
        assert_eq!(
            counters.uvw_table_missing_channels.load(Ordering::Relaxed),
            0
        );
    }

    /// stripGeometry が作れない(DIAMOND SIZE 無し)→ reco を呼ばず 0x04 も送らない。
    #[test]
    fn without_strip_geometry_there_is_no_reco_and_no_track_message() {
        let text = "ANGLES: 0.0 -120.0 -60.0\nREFERENCE POINT: 1.0 2.0\n\
                    U\t0\t1\t0\t0\t0\t0\t0.0\t0.0\t3\n";
        let (mut ingest, mut live, has_reco) = ingest_with(Some(text));
        assert!(!has_reco);
        ingest.handle(&every_strip_pulsed(70, 1236, true));
        let types: Vec<u8> = drain(&mut live).iter().map(|f| f.bytes[2]).collect();
        assert_eq!(types, vec![0x02, 0x02, 0x02], "0x04 は無い");
    }
}
