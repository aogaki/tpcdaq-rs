//! monitor の E2E —— **root_sink 実バイナリ**の PUB → monitor → WS(TODO/026、SPEC §12-10)。
//!
//! 022(tests/root_sink_monitor_pub.rs)が「root_sink の PUB が SPEC §5.3 どおり」を
//! 固定したので、ここは**その先**だけを見る: 実際に C++ が出したバイトを Rust の
//! 本番パーサが読み、表示変換(§5.4)を通って WS(§10)に出るところまで。
//!
//! **env `TPCDAQ_ROOT_SINK_BIN` 未設定なら skip**(C++ ビルドを cargo test の前提にしない):
//!
//! ```text
//! TPCDAQ_TPCRECO_DIR=<repo>/reference/TPCReco/TPCReco-HIGS2026_online make -C tools/root_sink root_sink
//! TPCDAQ_ROOT_SINK_BIN=$PWD/tools/root_sink/root_sink cargo test --test monitor_e2e
//! ```
//!
//! 実 .graw があれば `TPCDAQ_REAL_GRAW` で実データ経路も回る(実 .graw はローカルのみ)。

#![allow(clippy::unwrap_used)]

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use futures_util::StreamExt;
use serde_bytes::ByteBuf;
use tokio::sync::{broadcast, oneshot};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tpcdaq::geometry::{self, ChannelRole, Geometry};
use tpcdaq::monitor::{self, run_monitor, MonitorParams};
use tpcdaq::msg::{self, Batch, Fragment, Fragments, Message};

/// 合成ジオメトリ(リポ内フィクスチャ)。
/// aget0 raw0..3 = U strip1..4 / aget1 raw0..2 = V strip1..3 / aget1 raw3..6 = W strip1..4、
/// aget0 raw4 と aget1 raw7 = AUX、raw{11,22,45,56} = FPN。
const GEOMETRY: &str = "tests/fixtures/geometry_mini_reduced.dat";
/// root-sink の期待ソース(SPEC §3.2: decoder = 100)。
const DECODER_SOURCE_ID: u32 = 100;
const RUN_NUMBER: u32 = 7;
const BUCKETS: usize = 512;
const RECV_TIMEOUT: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------
// root_sink の起動(022 の流儀 —— Drop で必ず殺す + bind TOCTOU リトライ)
// ---------------------------------------------------------------------

fn sink_bin() -> Option<PathBuf> {
    std::env::var_os("TPCDAQ_ROOT_SINK_BIN").map(PathBuf::from)
}

fn free_endpoint() -> String {
    let probe = TcpListener::bind("127.0.0.1:0").expect("bind probe listener");
    let port = probe.local_addr().expect("local_addr").port();
    drop(probe);
    format!("tcp://127.0.0.1:{port}")
}

/// C++ 側の zmq bind 失敗の終了コード(`free_endpoint()` の TOCTOU)。
const EXIT_ZMQ: i32 = 4;

struct Sink {
    child: Child,
    data_ep: String,
    pub_ep: String,
}

impl Drop for Sink {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

impl Sink {
    fn spawn(bin: &PathBuf) -> Sink {
        for _ in 0..3 {
            let data_ep = free_endpoint();
            let pub_ep = free_endpoint();
            let mut child = Command::new(bin)
                .args([
                    "--bind",
                    &data_ep,
                    "--pub",
                    &pub_ep,
                    "--no-root",
                    "--geometry",
                    GEOMETRY,
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .expect("spawn root_sink");
            std::thread::sleep(Duration::from_millis(300));
            if let Some(status) = child.try_wait().expect("try_wait") {
                if status.code() == Some(EXIT_ZMQ) {
                    eprintln!("root_sink lost the bind race on {data_ep}/{pub_ep} — retrying");
                    continue;
                }
            }
            return Sink {
                child,
                data_ep,
                pub_ep,
            };
        }
        panic!("root_sink failed to bind after 3 attempts");
    }
}

fn connect_push(context: &zmq::Context, endpoint: &str) -> zmq::Socket {
    let push = context.socket(zmq::PUSH).expect("PUSH socket");
    push.set_sndhwm(100).expect("set sndhwm");
    push.set_sndtimeo(15_000).expect("set sndtimeo");
    push.set_linger(2_000).expect("set linger");
    push.connect(endpoint).expect("connect PUSH");
    push
}

fn batch_of(sequence_number: u64, payload: Fragments) -> Vec<u8> {
    let message: Message<Fragments> = Message::Data(Batch {
        source_id: DECODER_SOURCE_ID,
        run_number: RUN_NUMBER,
        sequence_number,
        created_ns: 1_755_000_000_000_000_000 + sequence_number,
        payload,
    });
    message.to_msgpack().expect("encode Data batch")
}

fn end_of_stream() -> Vec<u8> {
    let message: Message<Fragments> = Message::EndOfStream {
        source_id: DECODER_SOURCE_ID,
        run_number: RUN_NUMBER,
    };
    message.to_msgpack().expect("encode EndOfStream")
}

/// 1 サンプル = (aget, raw_ch, bucket, adc)。
type Sample = (u8, u8, u16, u16);

fn fragment_of(event_idx: u32, samples: &[Sample]) -> Fragment {
    let words: Vec<u32> = samples
        .iter()
        .map(|(aget, chan, bucket, adc)| msg::pack_item(*aget, *chan, *bucket, *adc).unwrap())
        .collect();
    Fragment {
        event_idx,
        event_time: 0x0000_1234_5678_9abc + u64::from(event_idx),
        cobo: 0,
        asad: 0,
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

// ---------------------------------------------------------------------
// monitor + WS クライアント
// ---------------------------------------------------------------------

struct Monitor {
    ws_url: String,
    shutdown: broadcast::Sender<()>,
    handle: tokio::task::JoinHandle<Result<(), monitor::MonitorError>>,
}

impl Monitor {
    async fn start(sub_endpoint: &str) -> Monitor {
        let params = MonitorParams {
            sub_endpoint: sub_endpoint.to_string(),
            ws_listen: "127.0.0.1:0".to_string(),
            geometry: PathBuf::from(GEOMETRY),
            live_queue: 256,
            detector: "mini_eTPC_e2e".to_string(),
            cobos: vec![0],
        };
        let (shutdown, shutdown_rx) = broadcast::channel(1);
        let (bound_tx, bound_rx) = oneshot::channel();
        let handle = tokio::spawn(run_monitor(params, shutdown_rx, Some(bound_tx)));
        let addr = tokio::time::timeout(RECV_TIMEOUT, bound_rx)
            .await
            .expect("monitor did not bind")
            .expect("monitor failed before binding");
        tokio::time::sleep(Duration::from_millis(400)).await; // SUB の slow joiner
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

    async fn next(&mut self, timeout: Duration) -> Option<WsMessage> {
        match tokio::time::timeout(timeout, self.socket.next()).await {
            Ok(Some(Ok(message))) => Some(message),
            _ => None,
        }
    }

    async fn next_json(&mut self, expected_type: &str, timeout: Duration) -> serde_json::Value {
        let deadline = tokio::time::Instant::now() + timeout;
        while let Some(message) = self.next(deadline - tokio::time::Instant::now()).await {
            if let WsMessage::Text(text) = message {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                if value["type"] == expected_type {
                    return value;
                }
            }
        }
        panic!("no `{expected_type}` JSON within {timeout:?}");
    }

    /// 指定の msgType + id/plane を持つバイナリが来るまで読む。
    async fn next_binary_where(
        &mut self,
        msg_type: u8,
        matches: impl Fn(&[u8]) -> bool,
        timeout: Duration,
    ) -> Vec<u8> {
        let deadline = tokio::time::Instant::now() + timeout;
        while let Some(message) = self.next(deadline - tokio::time::Instant::now()).await {
            if let WsMessage::Binary(bytes) = message {
                assert_eq!(&bytes[0..2], b"TP");
                assert_eq!(bytes[3], 2);
                if bytes[2] == msg_type && matches(&bytes) {
                    return bytes.to_vec();
                }
            }
        }
        panic!("no matching binary of type {msg_type:#04x} within {timeout:?}");
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

/// Histo2d(0x11)の本体を `(strip, bucket)` で引く。
///
/// WS の 2D は **iy 外側 row-major**(SPEC §10.2)なので `idx = bucket*nx + (strip-1)`。
/// PUB 側(SPEC §5.3)は `(strip-1)*512 + bucket` —— **この転置が効いていることの照合**でもある。
fn histo2d_at(message: &[u8], strip: u16, bucket: u16) -> f32 {
    let nx = u16_at(message, 15) as usize;
    let idx = bucket as usize * nx + (strip as usize - 1);
    f32_at(message, 35 + idx * 4)
}

// =====================================================================
// 1. 合成イベント: root_sink → monitor → WS
// =====================================================================

/// 投入イベント(022 と同素材 —— 同じ材料が WS の向こうで同じ値になることを見る)。
///
/// event 0: U strip1 に 100(b5)と 250(b6)、W strip1 に 4095(b2)、AUX 900・FPN 800(除外)
/// event 1: V strip1 に 40(b0)、V strip2 に 41(b1)
/// event 2: U strip4 に 4095(b9)、W strip4 に 7(b9)
fn synthetic_events() -> Vec<Vec<Fragment>> {
    vec![
        vec![fragment_of(
            0,
            &[
                (0, 0, 5, 100),
                (0, 0, 6, 250),
                (1, 3, 2, 4095),
                (0, 4, 2, 900),
                (0, 11, 2, 800),
            ],
        )],
        vec![fragment_of(1, &[(1, 0, 0, 40), (1, 1, 1, 41)])],
        vec![fragment_of(2, &[(0, 3, 9, 4095), (1, 6, 9, 7)])],
    ]
}

#[tokio::test]
async fn root_sink_pub_reaches_the_ws_as_uvw_and_histos() {
    let Some(bin) = sink_bin() else {
        eprintln!("SKIP: TPCDAQ_ROOT_SINK_BIN not set (C++ build is not a cargo test dependency)");
        return;
    };
    let sink = Sink::spawn(&bin);
    let monitor = Monitor::start(&sink.pub_ep).await;
    let mut client = Client::connect(&monitor).await;

    let meta = client.next_json("meta", RECV_TIMEOUT).await;
    assert_eq!(meta["planes"]["U"], 4);
    assert_eq!(meta["planes"]["V"], 3);
    assert_eq!(meta["planes"]["W"], 4);
    assert_eq!(meta["nBuckets"], 512);

    let context = zmq::Context::new();
    let push = connect_push(&context, &sink.data_ep);
    let events = synthetic_events();
    for (seq, fragments) in events.iter().enumerate() {
        push.send(batch_of(seq as u64, fragments.clone()), 0)
            .expect("send batch");
        // event_publish_hz(既定 20)に間引かれないよう間隔を空ける(最新優先 = §5.3)。
        tokio::time::sleep(Duration::from_millis(150)).await;
    }

    // --- event 0 の U 面(0x02)---
    let u_plane = client
        .next_binary_where(0x02, |b| b[13] == 0 && u32_at(b, 9) == 0, RECV_TIMEOUT)
        .await;
    assert_eq!(u32_at(&u_plane, 5), RUN_NUMBER, "runNumber");
    assert_eq!(u16_at(&u_plane, 14), 4, "nStrips = U 面の幅");
    assert_eq!(u16_at(&u_plane, 16), 512, "nBuckets");
    assert_eq!(u_plane[4], 0, "1 フラグメントで complete");
    let grid_at =
        |strip: usize, bucket: usize| u16_at(&u_plane, 18 + ((strip - 1) * 512 + bucket) * 2);
    assert_eq!(grid_at(1, 5), 100, "U strip1 bucket5");
    assert_eq!(grid_at(1, 6), 250, "U strip1 bucket6");
    let u_total: u32 = u_plane[18..]
        .chunks_exact(2)
        .map(|c| u32::from(u16::from_le_bytes([c[0], c[1]])))
        .sum();
    assert_eq!(u_total, 350, "AUX(900)も FPN(800)も U 面に入っていない");

    push.send(end_of_stream(), 0).expect("send EndOfStream");

    // --- EOS 後の hist_snapshot(0x11 の StripTimeU = id 1)---
    let striptime_u = client
        .next_binary_where(
            0x11,
            |b| u16_at(b, 13) == 1 && histo2d_at(b, 4, 9) > 0.0,
            RECV_TIMEOUT,
        )
        .await;
    assert_eq!(u16_at(&striptime_u, 15), 4, "nx = U のストリップ数");
    assert_eq!(u16_at(&striptime_u, 17), 512, "ny = 512 バケット");
    assert_eq!(u32_at(&striptime_u, 9), 0, "ヒストの eventNumber は 0");
    assert_eq!(f32_at(&striptime_u, 19), 1.0, "xmin = 1");
    assert_eq!(f32_at(&striptime_u, 23), 5.0, "xmax = nx + 1");
    assert_eq!(f32_at(&striptime_u, 27), 0.0, "ymin");
    assert_eq!(f32_at(&striptime_u, 31), 512.0, "ymax");
    // 3 イベント積算(SPEC §5.2 の StripTime = 全サンプルを weight=ADC で加算)
    assert_eq!(histo2d_at(&striptime_u, 1, 5), 100.0, "strip1 bucket5");
    assert_eq!(histo2d_at(&striptime_u, 1, 6), 250.0, "strip1 bucket6");
    assert_eq!(histo2d_at(&striptime_u, 4, 9), 4095.0, "strip4 bucket9");
    assert_eq!(histo2d_at(&striptime_u, 2, 0), 0.0, "触っていないビンは 0");

    // --- 1D(ChargeU = id 4、軸 [0,4096) 固定)---
    let charge_u = client
        .next_binary_where(0x10, |b| u16_at(b, 13) == 4, RECV_TIMEOUT)
        .await;
    assert_eq!(u32_at(&charge_u, 15), 512, "1D は 512 ビン");
    assert_eq!(f32_at(&charge_u, 19), 0.0);
    assert_eq!(f32_at(&charge_u, 23), 4096.0, "オートレンジ禁止");
    // 波高 250 → bin31、4095 → bin511(bin 幅 8)
    assert_eq!(f32_at(&charge_u, 27 + 31 * 4), 1.0, "ChargeU bin31 = 250/8");
    assert_eq!(
        f32_at(&charge_u, 27 + 511 * 4),
        1.0,
        "ChargeU bin511 = 4095/8"
    );

    // --- status(§5.3 そのまま + monitor の 3 つ)---
    let status = client.next_json("status", RECV_TIMEOUT).await;
    assert_eq!(status["state"], "idle", "EOS で run は閉じている");
    assert_eq!(status["run"], RUN_NUMBER);
    assert_eq!(status["events_built"], 3);
    assert_eq!(status["events_incomplete"], 0);
    assert_eq!(status["late_fragments"], 0);
    assert_eq!(status["pending_events"], 0);
    assert_eq!(status["frames_per_cobo"]["0"], 3);
    assert_eq!(
        status["saturation"]["U"]["saturated"], 1,
        "U は 4095 が 1 本"
    );
    assert_eq!(status["saturation"]["U"]["counted"], 2);
    assert_eq!(status["monitorGaps"], 0, "この規模で取りこぼしは無いはず");
    assert_eq!(status["clients"], 1);
    assert_eq!(status["wsDropped"], 0);

    monitor.stop().await;
}

// =====================================================================
// 2. 実データ E2E(env `TPCDAQ_REAL_GRAW` gate)
// =====================================================================

/// StripTime{U,V,W} を SPEC §5.2 のフィル規則から**独立に**再計算する
/// (022 と同じ二重実装照合。ここでは WS に出た f32 と突き合わせる)。
fn recompute_striptime(geometry: &Geometry, events: &[Vec<Fragment>]) -> [Vec<f64>; 3] {
    let mut bins = [
        vec![0.0f64; geometry.max_strip[0] as usize * BUCKETS],
        vec![0.0f64; geometry.max_strip[1] as usize * BUCKETS],
        vec![0.0f64; geometry.max_strip[2] as usize * BUCKETS],
    ];
    for fragments in events {
        for f in fragments {
            for word in msg::items_from_bytes(&f.items).expect("items") {
                let item = msg::unpack_item(word);
                let role = geometry.lookup(
                    u32::from(f.cobo),
                    u32::from(f.asad),
                    u32::from(item.aget),
                    u32::from(item.chan),
                );
                if let ChannelRole::Strip { plane, strip, .. } = role {
                    bins[plane as usize][(strip as usize - 1) * BUCKETS + item.bucket as usize] +=
                        f64::from(item.adc);
                }
            }
        }
    }
    bins
}

#[tokio::test]
async fn real_graw_reaches_the_ws_with_bins_matching_an_independent_recomputation() {
    let Some(bin) = sink_bin() else {
        eprintln!("SKIP: TPCDAQ_ROOT_SINK_BIN not set");
        return;
    };
    let Ok(path) = std::env::var("TPCDAQ_REAL_GRAW") else {
        eprintln!("SKIP: TPCDAQ_REAL_GRAW が未設定(実 .graw はローカルのみ)");
        return;
    };

    // --- デコード(本番の framer + decoder)---
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
    let mut framer = tpcdaq::framer::Framer::new();
    let mut decoder = tpcdaq::decode::Decoder::new();
    let mut events: Vec<Vec<Fragment>> = Vec::new();
    for chunk in bytes.chunks(1 << 16) {
        framer.push(chunk);
        while let Some(frame) = framer.next() {
            if let Some(f) = decoder.decode(frame) {
                events.push(vec![f]); // mini = 1 CoBo × 1 AsAd
            }
        }
    }
    assert_eq!(events.len(), 108, "P1 オラクル(events)");
    assert_eq!(decoder.items(), 15_040_512, "P1 オラクル(items)");

    let sink = Sink::spawn(&bin);
    let monitor = Monitor::start(&sink.pub_ep).await;
    let mut client = Client::connect(&monitor).await;
    client.next_json("meta", RECV_TIMEOUT).await;

    let context = zmq::Context::new();
    let push = connect_push(&context, &sink.data_ep);
    for (seq, fragments) in events.iter().enumerate() {
        push.send(batch_of(seq as u64, fragments.clone()), 0)
            .expect("send batch");
    }
    push.send(end_of_stream(), 0).expect("send EndOfStream");

    // --- 独立再計算(SPEC §5.2)---
    let geometry = geometry::load(GEOMETRY).expect("load geometry fixture");
    let expected = recompute_striptime(&geometry, &events);
    let expected_total: f64 = expected.iter().flat_map(|p| p.iter()).sum();
    assert!(expected_total > 0.0, "実データなのに Strip の総和が 0");

    // --- EOS 後のスナップショットを WS で受けて全ビン照合(f32 なので相対 1e-5)---
    for (plane, wanted) in expected.iter().enumerate() {
        let id = (plane + 1) as u16;
        let message = client
            .next_binary_where(
                0x11,
                |b| u16_at(b, 13) == id && u32_at(b, 5) == RUN_NUMBER,
                RECV_TIMEOUT,
            )
            .await;
        let nx = u16_at(&message, 15) as usize;
        let ny = u16_at(&message, 17) as usize;
        assert_eq!(nx, geometry.max_strip[plane] as usize, "nx = 面の幅");
        assert_eq!(ny, BUCKETS, "ny = 512");
        let mut mismatches = 0usize;
        for strip in 1..=nx {
            for bucket in 0..ny {
                let got = f64::from(histo2d_at(&message, strip as u16, bucket as u16));
                let want = wanted[(strip - 1) * BUCKETS + bucket];
                if (got - want).abs() > 1e-5 * want.abs().max(1.0) {
                    mismatches += 1;
                    if mismatches <= 3 {
                        eprintln!("plane {plane} strip {strip} bucket {bucket}: {got} != {want}");
                    }
                }
            }
        }
        assert_eq!(mismatches, 0, "面 {plane} のビンが独立再計算と違う");
    }

    let status = client.next_json("status", RECV_TIMEOUT).await;
    assert_eq!(status["events_built"], 108, "events_built");
    assert_eq!(status["events_incomplete"], 0);
    assert_eq!(status["frames_per_cobo"]["0"], 108);
    eprintln!(
        "real graw → WS: monitorGaps={} wsDropped={} publish_drops={}",
        status["monitorGaps"], status["wsDropped"], status["publish_drops"]
    );

    monitor.stop().await;
}
