//! monitor(026)の実配線試験 —— 実 ZMQ PUB → monitor → 実 WS クライアント。
//! SPEC §5.3(受け側)/ §5.4(責務)/ §10(WS プロトコル)/ §12-10(WS 適合性)。
//!
//! テスト内 PUB が SPEC §5.3 のメッセージを**手で組んだエンベロープ**
//! (positional array(5) + map 形式ペイロード = root_sink の monitor_pub.hpp と同じ
//! バイト構成)で流し、monitor が WS へ出したものを tokio-tungstenite で受けて機械検証する。
//! SPEC §10.4-5 の「ライブ経路 probe」の先行形。
//!
//! 全ポート動的(PUB は zmq のワイルドカード bind、WS は 0)。

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_bytes::ByteBuf;
use tokio::sync::{broadcast, oneshot};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tpcdaq::monitor::{
    self, run_monitor, BuiltEventPayload, HistPayload, HistSnapshotPayload, MonitorParams,
    SaturationPayload, StatusPayload,
};
use tpcdaq::msg::{self, Fragment};

/// 合成ジオメトリ(リポ内フィクスチャ。実 .dat は入れない — CLAUDE.md)。
/// U 最大 4 / V 最大 3 / W 最大 4、AUX 2 本、cobo0・asad0 のみ。
/// aget0 raw0..3 = U strip1..4 / aget1 raw0..2 = V strip1..3 / aget1 raw3..6 = W strip1..4。
const GEOMETRY: &str = "tests/fixtures/geometry_mini_reduced.dat";

/// root-sink のモニタ PUB の source_id(SPEC §3.2 v1.9)。
const MONITOR_SOURCE_ID: u32 = 101;

const RECV_TIMEOUT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------
// テスト内 PUB(SPEC §5.3 のワイヤを手で組む)
// ---------------------------------------------------------------------

struct Publisher {
    socket: zmq::Socket,
    endpoint: String,
    seq: u64,
}

impl Publisher {
    fn bind() -> Publisher {
        let context = zmq::Context::new();
        let socket = context.socket(zmq::PUB).unwrap();
        socket.set_linger(0).unwrap();
        // ワイルドカード bind → 実ポートを socket に聞く(固定ポートを書かない)。
        socket.bind("tcp://127.0.0.1:*").unwrap();
        let endpoint = socket.get_last_endpoint().unwrap().unwrap();
        // `zmq::Socket` は Context を内部で参照カウントして持つ(zmq-0.10 lib.rs 483–490)ので、
        // ここで Context を落としても socket は生き続ける。
        drop(context);
        Publisher {
            socket,
            endpoint,
            seq: 0,
        }
    }

    /// SPEC §2.2 のエンベロープ(positional array(5))+ §5.3 の map 形式ペイロード。
    fn send_payload(&mut self, run: u32, payload: &[u8]) {
        let mut out = vec![0x81]; // fixmap(1)
        out.extend_from_slice(&[0xa4, b'D', b'a', b't', b'a']); // "Data"
        out.push(0x95); // fixarray(5)
        for value in [
            u64::from(MONITOR_SOURCE_ID),
            u64::from(run),
            self.seq,
            1_755_000_000_000_000_000 + self.seq,
        ] {
            out.push(0xcf); // uint64
            out.extend_from_slice(&value.to_be_bytes());
        }
        out.extend_from_slice(payload);
        self.seq += 1;
        self.socket.send(out, 0).unwrap();
    }

    fn status(&mut self, run: u32, state: &str) {
        let payload = status_payload(run, state);
        self.send_payload(run, &rmp_serde::to_vec_named(&payload).unwrap());
    }

    fn snapshot(&mut self, run: u32, hists: Vec<HistPayload>) {
        let payload = HistSnapshotPayload {
            kind: "hist_snapshot".to_string(),
            run,
            hists,
        };
        self.send_payload(run, &rmp_serde::to_vec_named(&payload).unwrap());
    }

    fn event(&mut self, run: u32, event_idx: u32, complete: bool, fragments: Vec<Fragment>) {
        let payload = BuiltEventPayload {
            kind: "built_event".to_string(),
            run,
            event_idx,
            complete,
            fragments,
        };
        self.send_payload(run, &rmp_serde::to_vec_named(&payload).unwrap());
    }

    /// 将来 kind(前方互換の材料)。
    fn unknown_kind(&mut self, run: u32) {
        #[derive(serde::Serialize)]
        struct Future {
            kind: String,
            whatever: u32,
        }
        let payload = Future {
            kind: "some_future_kind".to_string(),
            whatever: 1,
        };
        self.send_payload(run, &rmp_serde::to_vec_named(&payload).unwrap());
    }

    /// seq を `n` 個進める(= n 通落ちたことにする)。
    fn skip(&mut self, n: u64) {
        self.seq += n;
    }
}

fn status_payload(run: u32, state: &str) -> StatusPayload {
    let mut frames = std::collections::BTreeMap::new();
    frames.insert("0".to_string(), 108u64);
    let mut saturation = std::collections::BTreeMap::new();
    for (plane, saturated, counted) in [("U", 1u64, 2u64), ("V", 0, 3), ("W", 4, 5)] {
        saturation.insert(plane.to_string(), SaturationPayload { saturated, counted });
    }
    StatusPayload {
        kind: "status".to_string(),
        run,
        state: state.to_string(),
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

/// 1 サンプル = (aget, raw_ch, bucket, adc)。
type Sample = (u8, u8, u16, u16);

fn fragment(cobo: u8, asad: u8, samples: &[Sample]) -> Fragment {
    let words: Vec<u32> = samples
        .iter()
        .map(|(aget, chan, bucket, adc)| msg::pack_item(*aget, *chan, *bucket, *adc).unwrap())
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
        items: ByteBuf::from(msg::items_to_bytes(&words)),
    }
}

fn hist(id: u8, name: &str, nx: u32, ny: u32, values: &[f64]) -> HistPayload {
    let mut bins = Vec::with_capacity(values.len() * 8);
    for v in values {
        bins.extend_from_slice(&v.to_le_bytes());
    }
    HistPayload {
        id,
        name: name.to_string(),
        nx,
        ny,
        bins: ByteBuf::from(bins),
    }
}

// ---------------------------------------------------------------------
// monitor の起動 / WS クライアント
// ---------------------------------------------------------------------

struct Monitor {
    ws_url: String,
    shutdown: broadcast::Sender<()>,
    handle: tokio::task::JoinHandle<Result<(), monitor::MonitorError>>,
}

impl Monitor {
    async fn start(sub_endpoint: &str, live_queue: usize) -> Monitor {
        let params = MonitorParams {
            sub_endpoint: sub_endpoint.to_string(),
            ws_listen: "127.0.0.1:0".to_string(), // 動的ポート
            geometry: PathBuf::from(GEOMETRY),
            live_queue,
            detector: "mini_eTPC_test".to_string(),
            cobos: vec![0],
        };
        let (shutdown, shutdown_rx) = broadcast::channel(1);
        let (bound_tx, bound_rx) = oneshot::channel();
        let handle = tokio::spawn(run_monitor(params, shutdown_rx, Some(bound_tx)));
        let addr = tokio::time::timeout(RECV_TIMEOUT, bound_rx)
            .await
            .expect("monitor did not bind in time")
            .expect("monitor failed before binding");
        // SUB が connect して購読が届くまでの猶予(slow joiner)。
        tokio::time::sleep(Duration::from_millis(300)).await;
        Monitor {
            ws_url: format!("ws://{addr}/ws"),
            shutdown,
            handle,
        }
    }

    async fn stop(self) {
        let _ = self.shutdown.send(());
        let _ = tokio::time::timeout(RECV_TIMEOUT, self.handle).await;
    }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct Client {
    socket: Socket,
}

impl Client {
    async fn connect(monitor: &Monitor) -> Client {
        let (socket, _) = tokio_tungstenite::connect_async(&monitor.ws_url)
            .await
            .expect("WS connect");
        Client { socket }
    }

    async fn send_text(&mut self, text: &str) {
        self.socket
            .send(WsMessage::Text(text.to_string().into()))
            .await
            .expect("send subscribe");
    }

    /// 次の 1 通(テキストなら Ok、バイナリなら Err で返す)。
    async fn next(&mut self, timeout: Duration) -> Option<WsMessage> {
        match tokio::time::timeout(timeout, self.socket.next()).await {
            Ok(Some(Ok(message))) => Some(message),
            _ => None,
        }
    }

    /// `type` が一致する JSON が来るまで読む。
    async fn next_json(&mut self, expected_type: &str, timeout: Duration) -> serde_json::Value {
        let deadline = tokio::time::Instant::now() + timeout;
        while let Some(message) = self.next(deadline - tokio::time::Instant::now()).await {
            if let WsMessage::Text(text) = message {
                let value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
                if value["type"] == expected_type {
                    return value;
                }
            }
        }
        panic!("no `{expected_type}` JSON within {timeout:?}");
    }

    /// msgType が一致するバイナリが来るまで読む。
    async fn next_binary(&mut self, msg_type: u8, timeout: Duration) -> Vec<u8> {
        let deadline = tokio::time::Instant::now() + timeout;
        while let Some(message) = self.next(deadline - tokio::time::Instant::now()).await {
            if let WsMessage::Binary(bytes) = message {
                assert_eq!(&bytes[0..2], b"TP", "magic(SPEC §10.1)");
                assert_eq!(bytes[3], 2, "version = 2");
                if bytes[2] == msg_type {
                    return bytes.to_vec();
                }
            }
        }
        panic!("no binary message of type {msg_type:#04x} within {timeout:?}");
    }

    /// 溜まっている在庫を捨てて静止させる。
    ///
    /// `warm_up` は「最初の status が WS に出る」まで PUB を叩き続けるので、返った時点で
    /// **既に流れた分の status がまだ配管に残っている**ことがある。カウンタ(monitorGaps /
    /// clients)を見るテストはその古い 1 通を読んでしまうと偽陰性になるため、観測の前に
    /// ここで空にする。
    async fn settle(&mut self) {
        let _ = self.drain(Duration::from_millis(400)).await;
    }

    /// `timeout` の間に来たものを全部集める。
    async fn drain(&mut self, timeout: Duration) -> Vec<WsMessage> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut out = Vec::new();
        while tokio::time::Instant::now() < deadline {
            match self.next(deadline - tokio::time::Instant::now()).await {
                Some(message) => out.push(message),
                None => break,
            }
        }
        out
    }
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn f32_at(bytes: &[u8], at: usize) -> f32 {
    f32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// status が WS に出るまで PUB を叩き続ける(SUB の slow joiner 対策)。返すのは受けた status。
async fn warm_up(publisher: &mut Publisher, client: &mut Client, run: u32) -> serde_json::Value {
    for _ in 0..40 {
        publisher.status(run, "idle");
        if let Some(WsMessage::Text(text)) = client.next(Duration::from_millis(200)).await {
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            if value["type"] == "status" {
                return value;
            }
        }
    }
    panic!("monitor never forwarded a status to the WS client");
}

// =====================================================================
// 1. meta → status → uvw → histo が届く(§10.4-5 probe の先行形)
// =====================================================================

#[tokio::test]
async fn meta_status_uvw_and_histos_reach_a_client() {
    let mut publisher = Publisher::bind();
    let monitor = Monitor::start(&publisher.endpoint, 64).await;
    let mut client = Client::connect(&monitor).await;

    // 接続時の 1 通目は meta(SPEC §10.3)。
    let meta = client.next_json("meta", RECV_TIMEOUT).await;
    assert_eq!(meta["nBuckets"], 512);
    assert_eq!(meta["planes"]["U"], 4, "フィクスチャの U 最大ストリップ");
    assert_eq!(meta["planes"]["V"], 3);
    assert_eq!(meta["planes"]["W"], 4);
    assert_eq!(meta["geometry"], GEOMETRY);
    assert_eq!(meta["detector"], "mini_eTPC_test");
    assert_eq!(meta["cobos"][0], 0);
    // フィクスチャの ANGLES: 11.0 22.0 33.0
    assert_eq!(meta["anglesDeg"][0], 11.0);
    assert_eq!(meta["anglesDeg"][1], 22.0);
    assert_eq!(meta["anglesDeg"][2], 33.0);

    // status は §5.3 そのまま + monitor の 3 つ(SPEC §10.3)。
    let status = warm_up(&mut publisher, &mut client, 7).await;
    assert_eq!(status["run"], 7);
    assert_eq!(status["state"], "idle");
    assert_eq!(status["events_built"], 11);
    assert_eq!(status["pending_events"], 4, "§5.3 v1.10 の 11 キー目");
    assert_eq!(status["frames_per_cobo"]["0"], 108);
    assert_eq!(status["saturation"]["W"]["counted"], 5);
    assert_eq!(status["monitorGaps"], 0, "まだ飛びは見ていない");
    assert_eq!(status["clients"], 1);
    assert_eq!(status["wsDropped"], 0);

    // built_event → Uvw ×3(SPEC §10.2 の 0x02)。
    // U strip2(aget0 raw1)の bucket 5 に 300、W strip1(aget1 raw3)の bucket 9 に 77。
    publisher.event(
        7,
        42,
        true,
        vec![fragment(0, 0, &[(0, 1, 5, 300), (1, 3, 9, 77)])],
    );
    let u_plane = client.next_binary(0x02, RECV_TIMEOUT).await;
    assert_eq!(u_plane[13], 0, "1 通目は U 面");
    assert_eq!(u16_at(&u_plane, 14), 4, "nStrips = 4");
    assert_eq!(u16_at(&u_plane, 16), 512, "nBuckets = 512");
    assert_eq!(u32_at(&u_plane, 5), 7, "runNumber");
    assert_eq!(u32_at(&u_plane, 9), 42, "eventNumber");
    assert_eq!(u_plane[4], 0, "complete = flags 0");
    // idx = (strip-1)*512 + bucket
    assert_eq!(u16_at(&u_plane, 18 + (512 + 5) * 2), 300, "strip2 bucket5");
    let v_plane = client.next_binary(0x02, RECV_TIMEOUT).await;
    assert_eq!(v_plane[13], 1);
    assert_eq!(u16_at(&v_plane, 14), 3);
    let w_plane = client.next_binary(0x02, RECV_TIMEOUT).await;
    assert_eq!(w_plane[13], 2);
    assert_eq!(u16_at(&w_plane, 14), 4);
    assert_eq!(u16_at(&w_plane, 18 + 9 * 2), 77, "W strip1 bucket9");

    // hist_snapshot → Histo2d / Histo1d(SPEC §10.2 の 0x11 / 0x10)。
    publisher.snapshot(
        7,
        vec![
            hist(1, "StripTimeU", 2, 3, &[10.0, 11.0, 12.0, 20.0, 21.0, 22.0]),
            hist(4, "ChargeU", 4, 1, &[0.0, 1.5, 2.25, 3.75]),
        ],
    );
    let h2 = client.next_binary(0x11, RECV_TIMEOUT).await;
    assert_eq!(u16_at(&h2, 13), 1, "id");
    assert_eq!(u16_at(&h2, 15), 2, "nx");
    assert_eq!(u16_at(&h2, 17), 3, "ny");
    assert_eq!(f32_at(&h2, 19), 1.0, "xmin = 1");
    assert_eq!(f32_at(&h2, 23), 3.0, "xmax = nx + 1");
    assert_eq!(f32_at(&h2, 27), 0.0, "ymin");
    assert_eq!(f32_at(&h2, 31), 3.0, "ymax = ny");
    assert_eq!(u32_at(&h2, 9), 0, "ヒストの eventNumber は 0");
    // PUB は strip-major(ix 外側)、WS は iy 外側 → 転置される。
    let body: Vec<f32> = h2[35..].chunks_exact(4).map(|c| f32_at(c, 0)).collect();
    assert_eq!(body, vec![10.0, 20.0, 11.0, 21.0, 12.0, 22.0]);

    let h1 = client.next_binary(0x10, RECV_TIMEOUT).await;
    assert_eq!(u16_at(&h1, 13), 4, "id");
    assert_eq!(u32_at(&h1, 15), 4, "nbins");
    assert_eq!(f32_at(&h1, 19), 0.0, "xmin");
    assert_eq!(f32_at(&h1, 23), 4096.0, "xmax は 4096 固定");
    let body: Vec<f32> = h1[27..].chunks_exact(4).map(|c| f32_at(c, 0)).collect();
    assert_eq!(body, vec![0.0, 1.5, 2.25, 3.75]);

    monitor.stop().await;
}

// =====================================================================
// 2. waveforms は既定 OFF、subscribe で ON(SPEC §10.3 の帯域制御)
// =====================================================================

#[tokio::test]
async fn waveforms_are_off_by_default_and_arrive_after_subscribing() {
    let mut publisher = Publisher::bind();
    let monitor = Monitor::start(&publisher.endpoint, 64).await;
    let mut client = Client::connect(&monitor).await;
    client.next_json("meta", RECV_TIMEOUT).await;
    warm_up(&mut publisher, &mut client, 1).await;

    // 既定(waveforms 以外 ON)では 0x03 は来ない。
    publisher.event(1, 1, true, vec![fragment(0, 0, &[(0, 0, 3, 100)])]);
    let messages = client.drain(Duration::from_millis(700)).await;
    let types: Vec<u8> = messages
        .iter()
        .filter_map(|m| match m {
            WsMessage::Binary(bytes) => Some(bytes[2]),
            _ => None,
        })
        .collect();
    assert!(types.contains(&0x02), "uvw は既定 ON: {types:?}");
    assert!(!types.contains(&0x03), "waveforms は既定 OFF: {types:?}");

    // subscribe で ON にすると届く。
    client
        .send_text(r#"{"streams":["uvw","waveforms","histos","status"]}"#)
        .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    publisher.event(
        1,
        2,
        false, // incomplete = flags bit0
        vec![
            fragment(0, 0, &[(0, 0, 3, 100)]),
            fragment(0, 1, &[(2, 9, 4, 55)]),
        ],
    );
    let first = client.next_binary(0x03, RECV_TIMEOUT).await;
    assert_eq!(first[13], 0, "cobo");
    assert_eq!(first[14], 0, "asad");
    assert_eq!(first[15], 4, "nAget");
    assert_eq!(first[16], 68, "nCh(FPN 込み)");
    assert_eq!(u16_at(&first, 17), 512, "nBuckets");
    assert_eq!(first[4], 1, "incomplete = flags bit0");
    assert_eq!(first.len(), 19 + 4 * 68 * 512 * 2);
    // idx = ((aget*68)+ch)*512 + bucket
    assert_eq!(u16_at(&first, 19 + 3 * 2), 100, "aget0 ch0 bucket3");
    let second = client.next_binary(0x03, RECV_TIMEOUT).await;
    assert_eq!((second[13], second[14]), (0, 1), "2 通目は AsAd 1");
    assert_eq!(u16_at(&second, 19 + ((2 * 68 + 9) * 512 + 4) * 2), 55);

    // subscribe を戻すとまた止まる(「波形ビューを閉じた」)。
    client.send_text(r#"{"streams":["uvw","status"]}"#).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    publisher.event(1, 3, true, vec![fragment(0, 0, &[(0, 0, 3, 100)])]);
    let messages = client.drain(Duration::from_millis(700)).await;
    let types: Vec<u8> = messages
        .iter()
        .filter_map(|m| match m {
            WsMessage::Binary(bytes) => Some(bytes[2]),
            _ => None,
        })
        .collect();
    assert!(
        !types.contains(&0x03),
        "購読を外したのに来ている: {types:?}"
    );

    monitor.stop().await;
}

// =====================================================================
// 3. ギャップ計数(SPEC §5.4「モニタ取りこぼし数」)+ 未知 kind
// =====================================================================

#[tokio::test]
async fn sequence_gaps_and_unknown_kinds_are_counted_not_hidden() {
    let mut publisher = Publisher::bind();
    let monitor = Monitor::start(&publisher.endpoint, 64).await;
    let mut client = Client::connect(&monitor).await;
    client.next_json("meta", RECV_TIMEOUT).await;

    warm_up(&mut publisher, &mut client, 5).await;
    client.settle().await;
    publisher.status(5, "idle");
    let status = client.next_json("status", RECV_TIMEOUT).await;
    let before = status["monitorGaps"].as_u64().unwrap();

    // 3 通落ちたことにして次を送る → monitorGaps が 3 増える。
    publisher.skip(3);
    publisher.status(5, "idle");
    let status = client.next_json("status", RECV_TIMEOUT).await;
    assert_eq!(
        status["monitorGaps"].as_u64().unwrap(),
        before + 3,
        "seq の飛びが monitorGaps に出ていない(silent drop)"
    );

    // 未知 kind は数えて無視 —— その後のメッセージは普通に流れ続ける(前方互換)。
    publisher.unknown_kind(5);
    publisher.status(5, "idle");
    let status = client.next_json("status", RECV_TIMEOUT).await;
    assert_eq!(
        status["monitorGaps"].as_u64().unwrap(),
        before + 3,
        "未知 kind はギャップではない(受け取れている)"
    );
    publisher.event(5, 9, true, vec![fragment(0, 0, &[(0, 0, 1, 42)])]);
    let plane = client.next_binary(0x02, RECV_TIMEOUT).await;
    assert_eq!(u32_at(&plane, 9), 9, "未知 kind の後もイベントが届く");

    monitor.stop().await;
}

// =====================================================================
// 4. run 遷移で `run` と `meta` が飛ぶ(SPEC §10.3)
// =====================================================================

#[tokio::test]
async fn run_transitions_produce_run_and_meta_messages() {
    let mut publisher = Publisher::bind();
    let monitor = Monitor::start(&publisher.endpoint, 64).await;
    let mut client = Client::connect(&monitor).await;
    let meta = client.next_json("meta", RECV_TIMEOUT).await;
    assert_eq!(meta["run"], 0, "接続時はまだ run を知らない");

    warm_up(&mut publisher, &mut client, 0).await;

    // idle(run 0)→ running(run 12): meta(run 更新)と run(遷移)の両方。
    publisher.status(12, "running");
    let meta = client.next_json("meta", RECV_TIMEOUT).await;
    assert_eq!(meta["run"], 12, "run 変化で meta を配り直す");
    let run = client.next_json("run", RECV_TIMEOUT).await;
    assert_eq!(run["state"], "running");
    assert_eq!(run["run"], 12);
    assert!(
        run["ts"].as_str().unwrap().len() >= 20,
        "ts は RFC3339: {}",
        run["ts"]
    );

    // 同じ状態のまま status が続いても run は飛ばない。
    publisher.status(12, "running");
    publisher.status(12, "running");
    let messages = client.drain(Duration::from_millis(700)).await;
    let runs = messages
        .iter()
        .filter(|m| match m {
            WsMessage::Text(text) => text.contains("\"type\":\"run\""),
            _ => false,
        })
        .count();
    assert_eq!(runs, 0, "遷移していないのに run が飛んでいる");

    // running → idle は遷移(run 番号は同じ)。
    publisher.status(12, "idle");
    let run = client.next_json("run", RECV_TIMEOUT).await;
    assert_eq!(run["state"], "idle");
    assert_eq!(run["run"], 12);

    monitor.stop().await;
}

// =====================================================================
// 5. 遅いクライアント: live は drop-oldest + wsDropped、JSON は落ちない
// =====================================================================

#[tokio::test]
async fn a_slow_client_drops_live_frames_but_never_the_json_control_channel() {
    let mut publisher = Publisher::bind();
    // live キューを 2 段にして、読まないクライアントが確実に溢れるようにする。
    let monitor = Monitor::start(&publisher.endpoint, 2).await;
    let mut client = Client::connect(&monitor).await;
    client.next_json("meta", RECV_TIMEOUT).await;
    warm_up(&mut publisher, &mut client, 3).await;

    // **読まずに**大量のイベントを流し込む(1 イベント = 3 面 = 約 22 KB)。
    for idx in 0..400u32 {
        publisher.event(3, idx, true, vec![fragment(0, 0, &[(0, 0, 3, 100)])]);
    }
    tokio::time::sleep(Duration::from_millis(300)).await;

    // ここから読み始める → 送信タスクが Lagged を観測して ws_dropped に積む。
    let _ = client.drain(Duration::from_millis(500)).await;

    // status(JSON = reliable)は落ちずに届き、wsDropped が立っている。
    let mut dropped = 0u64;
    for _ in 0..20 {
        publisher.status(3, "running");
        let status = client.next_json("status", RECV_TIMEOUT).await;
        dropped = status["wsDropped"].as_u64().unwrap();
        assert_eq!(status["events_built"], 11, "status の中身は壊れない");
        assert_eq!(status["clients"], 1);
        if dropped > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        dropped > 0,
        "live を落としたのに wsDropped が 0(silent drop になっている)"
    );

    monitor.stop().await;
}

// =====================================================================
// 6. クライアントの出入りが clients に出る
// =====================================================================

#[tokio::test]
async fn client_count_tracks_connections_and_disconnections() {
    let mut publisher = Publisher::bind();
    let monitor = Monitor::start(&publisher.endpoint, 64).await;
    let mut first = Client::connect(&monitor).await;
    first.next_json("meta", RECV_TIMEOUT).await;
    warm_up(&mut publisher, &mut first, 1).await;
    first.settle().await;
    publisher.status(1, "idle");
    let status = first.next_json("status", RECV_TIMEOUT).await;
    assert_eq!(status["clients"], 1);

    let mut second = Client::connect(&monitor).await;
    second.next_json("meta", RECV_TIMEOUT).await;
    first.settle().await;
    publisher.status(1, "idle");
    let status = first.next_json("status", RECV_TIMEOUT).await;
    assert_eq!(status["clients"], 2, "2 本目の接続が数えられていない");

    drop(second);
    tokio::time::sleep(Duration::from_millis(300)).await;
    first.settle().await;
    publisher.status(1, "idle");
    let status = first.next_json("status", RECV_TIMEOUT).await;
    assert_eq!(status["clients"], 1, "切断が掃除されていない");

    monitor.stop().await;
}
