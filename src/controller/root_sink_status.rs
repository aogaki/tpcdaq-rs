//! controller が root-sink の monitor PUB の `status` を購読する(SPEC §5.3 v1.54、TODO/204 A)。
//!
//! root-sink は REP を持たない(SPEC §1.3)ので、停止の第二段が root-sink の進み
//! (`events_built` / `bytes_written` / `fragments`)と run の close(`state == "idle"`)を見る
//! 口はこの PUB だけである(monitor と同じ宛先 = `[monitor] sub_endpoint`)。専用 OS スレッド
//! 1 本が **最新の 1 通だけ**を共有の箱に置き、[`super::ZmqTransport`] がそれを読む
//! ([`super::Sequencer`] は IO を持たない、という切り方を崩さない)。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tracing::{error, info, warn};

use super::lock;
use crate::monitor::{decode_message, MonitorPayload, StatusPayload};

/// root-sink の status 1 通と、それを受け取った時刻。
#[derive(Debug, Clone, PartialEq)]
pub struct RootSinkStatus {
    pub received_at: Instant,
    pub payload: StatusPayload,
}

/// 最新の 1 通の置き場(SUB スレッドが書き、`ZmqTransport::root_sink_status` が読む)。
/// `None` = まだ 1 通も受け取っていない(root-sink 不在 / 未接続)。
pub type LatestRootSinkStatus = Arc<Mutex<Option<RootSinkStatus>>>;

/// SUB の受信 HWM。使うのは最新の 1 通だけなので小さく取る(monitor の SUB の 1000 とは別)。
const ROOT_SINK_SUB_RCVHWM: i32 = 16;

/// poll の待ち上限(ms)= 停止合図を見る周期(monitor の SUB と同じ 100 ms 級)。
const ROOT_SINK_SUB_POLL_MS: i64 = 100;

/// SUB スレッドの勘定。終了ログに出し、`join` で呼び手へ返す。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RootSinkStatusCounts {
    /// 受け取った `status`。
    pub status_in: u64,
    /// `status` 以外(hist_snapshot / built_event / 未知の kind)= 読み捨てた数。
    pub ignored: u64,
    /// 復号できなかった 1 通(最初の 1 回だけ warn、以後は数えるだけ)。
    pub decode_errors: u64,
}

/// 生の 1 通を処理する(ZMQ を知らない —— スレッド本体から呼ぶ)。`status` なら箱を上書きし、
/// 他の kind は数えて捨てる。復号できない 1 通は最初の 1 回だけ warn(silent にしない)。
fn ingest(raw: &[u8], latest: &LatestRootSinkStatus, counts: &mut RootSinkStatusCounts) {
    match decode_message(raw) {
        Ok(message) => match message.payload {
            MonitorPayload::Status(status) => {
                counts.status_in += 1;
                *lock(latest) = Some(RootSinkStatus {
                    received_at: Instant::now(),
                    payload: *status,
                });
            }
            _ => counts.ignored += 1,
        },
        Err(e) => {
            counts.decode_errors += 1;
            if counts.decode_errors == 1 {
                warn!(error = %e, "controller: cannot decode a root-sink PUB message (counted)");
            }
        }
    }
}

/// SUB スレッドを起動する。`stop` が立つと 100 ms 以内に抜け、勘定を返す。
///
/// ソケットが作れない・繋げないときはスレッドが error を出して抜ける(controller は止めない ——
/// そのときの停止は root-sink を「見ていない」= 従来の 3 点判定になる)。
pub fn spawn_root_sink_status_thread(
    endpoint: String,
    latest: LatestRootSinkStatus,
    stop: Arc<AtomicBool>,
) -> std::io::Result<std::thread::JoinHandle<RootSinkStatusCounts>> {
    std::thread::Builder::new()
        .name("controller-root-sink-sub".to_string())
        .spawn(move || {
            let mut counts = RootSinkStatusCounts::default();
            let context = zmq::Context::new();
            let socket = match subscribe(&context, &endpoint) {
                Ok(socket) => socket,
                Err(e) => {
                    error!(endpoint, error = %e,
                           "controller: cannot subscribe to the root-sink status — stops will \
                            not see root-sink (SPEC §1.3 v1.54 falls back to the 3-point EOS)");
                    return counts;
                }
            };
            info!(
                endpoint,
                "controller: subscribed to the root-sink status PUB"
            );
            while !stop.load(Ordering::Relaxed) {
                // 待つのは poll の仕事(停止合図を 100 ms ごとに見る)。上流は 1 本だけ。
                let readable = {
                    let mut items = [socket.as_poll_item(zmq::POLLIN)];
                    match zmq::poll(&mut items, ROOT_SINK_SUB_POLL_MS) {
                        Ok(_) => items[0].is_readable(),
                        Err(zmq::Error::EINTR) => false,
                        Err(e) => {
                            error!(error = %e, "controller: root-sink SUB poll failed");
                            break;
                        }
                    }
                };
                if !readable {
                    continue;
                }
                // 溜まっている分を全部読む(最後の status が勝つ)。
                loop {
                    match socket.recv_bytes(zmq::DONTWAIT) {
                        Ok(raw) => ingest(&raw, &latest, &mut counts),
                        Err(zmq::Error::EAGAIN) => break,
                        Err(e) => {
                            error!(error = %e, "controller: root-sink SUB receive failed");
                            return counts;
                        }
                    }
                }
            }
            info!(
                status_in = counts.status_in,
                ignored = counts.ignored,
                decode_errors = counts.decode_errors,
                "controller: root-sink status thread stopped"
            );
            counts
        })
}

/// SUB を作って `endpoint` に繋ぐ(全部購読・受信 HWM 小・linger 0)。
fn subscribe(context: &zmq::Context, endpoint: &str) -> zmq::Result<zmq::Socket> {
    let socket = context.socket(zmq::SUB)?;
    socket.set_rcvhwm(ROOT_SINK_SUB_RCVHWM)?;
    socket.set_linger(0)?;
    // トピックフレームなし(SPEC §2.1)= 全部購読。kind の選別は `ingest` が行う。
    socket.set_subscribe(b"")?;
    socket.connect(endpoint)?;
    Ok(socket)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// SPEC §2.2 の Data エンベロープ(`{"Data": [source_id, run, seq, created_ns, payload]}`)に
    /// map 形式の payload を被せる(`monitor::test_util::envelope` と同じ組み方 —— あちらは
    /// monitor の中だけの部品なのでここに写す)。
    fn envelope(run: u32, seq: u64, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0x81]; // fixmap(1)
        out.extend_from_slice(&[0xa4, b'D', b'a', b't', b'a']); // "Data"
        out.push(0x95); // fixarray(5)
        for value in [101u64, u64::from(run), seq, 1] {
            out.push(0xcf); // uint64
            out.extend_from_slice(&value.to_be_bytes());
        }
        out.extend_from_slice(payload);
        out
    }

    /// root-sink の status(186 以降の形 = `fragments` 付き)。値は非対称。
    fn status(run: u32, state: &str, events_built: u64) -> Vec<u8> {
        let payload = serde_json::json!({
            "kind": "status",
            "run": run,
            "state": state,
            "events_built": events_built,
            "events_incomplete": 3,
            "late_fragments": 2,
            "pending_events": 5,
            "frames_per_cobo": {"0": 1109},
            "bytes_written": 70_368_744u64,
            "saturation": {"U": {"saturated": 1, "counted": 4}},
            "publish_drops": 0,
            "fragments": events_built + 7,
        });
        rmp_serde::to_vec_named(&payload).unwrap()
    }

    fn hist_snapshot(run: u32) -> Vec<u8> {
        let payload = serde_json::json!({"kind": "hist_snapshot", "run": run, "hists": []});
        rmp_serde::to_vec_named(&payload).unwrap()
    }

    fn built_event(run: u32) -> Vec<u8> {
        let payload = serde_json::json!({
            "kind": "built_event", "run": run, "event_idx": 4, "complete": true, "fragments": [],
        });
        rmp_serde::to_vec_named(&payload).unwrap()
    }

    fn wait_for(what: &str, mut predicate: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if predicate() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for {what}");
    }

    /// テスト 10(TODO/204): 本物の PUB に hist_snapshot / status / built_event / 壊れた 1 通を
    /// 混ぜて流すと、箱には**最新の status だけ**が残り(`fragments` も読める)、他の kind は
    /// 数えて捨てる。停止合図でスレッドは抜け、`join` で勘定が返る。
    #[test]
    fn the_reader_keeps_only_the_latest_status_and_ignores_other_kinds() {
        let context = zmq::Context::new();
        let publisher = context.socket(zmq::PUB).unwrap();
        publisher.set_linger(0).unwrap();
        publisher.bind("tcp://127.0.0.1:*").unwrap();
        let endpoint = publisher.get_last_endpoint().unwrap().unwrap();

        let latest: LatestRootSinkStatus = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let reader =
            spawn_root_sink_status_thread(endpoint, Arc::clone(&latest), Arc::clone(&stop))
                .unwrap();

        // slow joiner: SUB の接続が立つまで最初の組を撃ち直す(どの 1 通が落ちても照合は
        // 「最後に読めた status」だけを見る)。
        let mut seq = 0u64;
        wait_for("the first status", || {
            for payload in [
                hist_snapshot(214),
                status(214, "running", 100),
                built_event(214),
            ] {
                publisher.send(envelope(214, seq, &payload), 0).unwrap();
                seq += 1;
            }
            std::thread::sleep(Duration::from_millis(20));
            latest.lock().unwrap().is_some()
        });
        let first = latest.lock().unwrap().clone().unwrap();

        // 最後の status(閉じた run 214)の後ろに hist_snapshot と壊れた 1 通を置く ——
        // 箱が status 以外で上書きされないこと。
        publisher
            .send(envelope(214, seq, &status(214, "idle", 17_000)), 0)
            .unwrap();
        publisher
            .send(envelope(214, seq + 1, &hist_snapshot(214)), 0)
            .unwrap();
        publisher.send(&[0x00u8, 0x01, 0x02][..], 0).unwrap();
        wait_for("the closing status", || {
            latest
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|s| s.payload.state == "idle")
        });
        // 壊れた 1 通まで読み終えるのを待ってから止める(勘定の照合のため)。
        std::thread::sleep(Duration::from_millis(200));
        stop.store(true, Ordering::Relaxed);
        let counts = reader.join().unwrap();

        let last = latest.lock().unwrap().clone().unwrap();
        assert_eq!(last.payload.run, 214);
        assert_eq!(last.payload.state, "idle");
        assert_eq!(last.payload.events_built, 17_000);
        assert_eq!(last.payload.events_incomplete, 3);
        // 17 000 + 7(上の `status` の組み方)。
        assert_eq!(last.payload.fragments, Some(17_007));
        assert!(last.received_at >= first.received_at);
        assert!(counts.status_in >= 2, "{counts:?}");
        // 少なくとも閉じた run の後ろの hist_snapshot 1 通 + 最初の組の残り。
        assert!(counts.ignored >= 1, "{counts:?}");
        assert_eq!(counts.decode_errors, 1, "{counts:?}");
    }
}
