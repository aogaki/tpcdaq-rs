//! receiver の decoder リンクは「待たずに送って数えて落とす」(TODO/184、SPEC v1.53 §1.4-7)。
//!
//! run 197 では receiver の送信スレッド 1 本が graw-writer → decoder の順に**ブロック送信**して
//! いたため、decoder 側の遅れが graw への送信まで止め、内部キューが溢れて graw まで失った。
//! v1.53 からロスレスなのは graw-writer リンクだけで、decoder リンクの Data / Heartbeat は
//! `ZMQ_DONTWAIT` で 1 回だけ送り、満杯・相手不在ならそのバッチを decoder へは送らずに数える:
//!
//! - graw-writer は全フレームを受け取る(背圧で待つ = ロスレス)。receiver は Error にならない
//! - `decoder_link_dropped_frames` / `_batches` / `_heartbeats` に数える(silent にしない)
//! - 落としたバッチでは decoder リンクの sequence_number を進めない(decoder の seq 検査は
//!   「意図しない喪失」の検出に残す)
//! - EndOfStream は落とさない(空くまで待つ、SPEC §2.2)
//!
//! 発注書 184 §2 のテスト 1 / 2 / 3 / 7。テスト 4・6 は既存ファイル
//! (`receiver_overload.rs` / `receiver_integration.rs`)、テスト 5 は `stop_in_error.rs`。
//! ポートはすべて 0(動的)。条件待ちは固定 sleep ではなくポーリング + タイムアウト。

#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::{broadcast, oneshot};
use tpcdaq::command::{Command, CommandResponse, ComponentState, RunConfig};
use tpcdaq::msg::{Message, RawFrames};
use tpcdaq::receiver::{run_receiver, ReceiverParams};
use tpcdaq::zmq_helper;

/// 受け手を止めたリンクがすぐ満杯になるよう HWM は極小(receiver_overload.rs と同じ)。
const TEST_HWM: i32 = 1;
/// CoBo の番号 = Batch の source_id(SPEC §3.2)。0 以外にして取り違えを見えるように。
const COBO_ID: u32 = 2;

// ---------------------------------------------------------------------
// 配線ヘルパ
// ---------------------------------------------------------------------

/// `tcp://127.0.0.1:0` に PULL を bind する(receiver の PUSH が connect してくる先)。
fn bind_pull(ctx: &zmq::Context, hwm: i32) -> (zmq::Socket, String) {
    let sock = ctx.socket(zmq::PULL).unwrap();
    sock.set_linger(0).unwrap();
    zmq_helper::apply_hwm(&sock, hwm).unwrap(); // HWM は bind 前に設定する
    sock.bind("tcp://127.0.0.1:0").unwrap();
    sock.set_rcvtimeo(10_000).unwrap(); // ハングではなく失敗で終わらせる
    let endpoint = sock.get_last_endpoint().unwrap().unwrap();
    (sock, endpoint)
}

/// 誰も bind していないアドレス(TcpListener で確保して即座に手放す)= decoder 不在。
fn dead_endpoint() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    format!("tcp://{addr}")
}

fn params(graw: &str, decoder: &str) -> ReceiverParams {
    ReceiverParams {
        cobo_id: COBO_ID,
        listen: "127.0.0.1:0".to_string(),
        command_listen: "tcp://127.0.0.1:0".to_string(),
        graw_writer_endpoint: graw.to_string(),
        decoder_endpoint: decoder.to_string(),
        batch_max_bytes: 40 * 1024, // 16 KiB フレームなら最大 3 フレーム / バッチ
        batch_max_ms: 10,
        // 溢れさせない(見たいのは decoder リンクの落としであって overflow ではない)
        queue_frames: 16_384,
        heartbeat_ms: 1_000,
        hwm: TEST_HWM,
    }
}

async fn start_receiver(
    params: ReceiverParams,
) -> (String, broadcast::Sender<()>, tokio::task::JoinHandle<()>) {
    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    let (ep_tx, ep_rx) = oneshot::channel();
    let handle = tokio::spawn(run_receiver(params, shutdown_rx, Some(ep_tx)));
    let endpoint = ep_rx
        .await
        .expect("receiver never reported its command endpoint");
    (endpoint, shutdown_tx, handle)
}

async fn shutdown_and_join(shutdown: broadcast::Sender<()>, task: tokio::task::JoinHandle<()>) {
    shutdown.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("receiver did not stop within 5 s")
        .unwrap();
}

/// REQ で 1 往復する(毎回新しい REQ ソケット = REQ の送受交互制約を避ける)。
async fn rpc(endpoint: &str, cmd: &Command) -> CommandResponse {
    let ctx = tmq::Context::new();
    let sender = tmq::request(&ctx).connect(endpoint).unwrap();
    {
        use tmq::AsZmqSocket;
        sender.get_socket().set_linger(0).unwrap();
    }
    let receiver = sender
        .send(vec![cmd.to_json().unwrap()].into())
        .await
        .unwrap();
    let (mut reply, _sender) = receiver.recv().await.unwrap();
    CommandResponse::from_json(&reply.pop_front().unwrap()).unwrap()
}

fn metric_u64(resp: &CommandResponse, key: &str) -> u64 {
    resp.metrics.as_ref().unwrap()[key]
        .as_u64()
        .unwrap_or_else(|| panic!("metrics.{key} is not a number: {:?}", resp.metrics))
}

/// Configure → Arm → Start まで進め、CoBo が繋ぐデータアドレスを返す。
async fn configure_arm_start(cmd_ep: &str, run_number: u32) -> String {
    let configured = rpc(
        cmd_ep,
        &Command::Configure(RunConfig {
            run_number,
            comment: "decoder link counted drop".to_string(),
            config: serde_json::Value::Null,
        }),
    )
    .await;
    assert!(configured.success, "{}", configured.message);
    let armed = rpc(cmd_ep, &Command::Arm).await;
    assert!(armed.success, "{}", armed.message);
    let data_addr = armed.metrics.as_ref().unwrap()["bind_address"]
        .as_str()
        .unwrap()
        .to_string();
    let started = rpc(cmd_ep, &Command::Start { run_number }).await;
    assert!(started.success, "{}", started.message);
    data_addr
}

/// 条件が満たされるまで GetStatus をポーリングする(固定 sleep に依存しない)。
async fn poll_status(
    cmd_ep: &str,
    timeout: Duration,
    what: &str,
    mut done: impl FnMut(&CommandResponse) -> bool,
) -> CommandResponse {
    let deadline = Instant::now() + timeout;
    let mut last = rpc(cmd_ep, &Command::GetStatus).await;
    while Instant::now() < deadline {
        if done(&last) {
            return last;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        last = rpc(cmd_ep, &Command::GetStatus).await;
    }
    panic!("timed out after {timeout:?} waiting for {what}; last status = {last:?}");
}

/// 合成 MFM フレーム(blkSize=1 little-endian、全長 `total_bytes`、中身は `fill`)。
/// 末尾バイトが `fill` のままなので、どのフェーズのフレームかを受け側で見分けられる。
fn make_frame(total_bytes: usize, fill: u8) -> Vec<u8> {
    let mut f = vec![fill; total_bytes];
    f[0] = 0x80;
    let blk = total_bytes as u32;
    f[1] = (blk & 0xFF) as u8;
    f[2] = ((blk >> 8) & 0xFF) as u8;
    f[3] = ((blk >> 16) & 0xFF) as u8;
    f
}

// ---------------------------------------------------------------------
// PULL 側の収集(別スレッドで EOS まで読む)
// ---------------------------------------------------------------------

#[derive(Debug, Default)]
struct Collected {
    /// Data バッチが運んできた生フレーム(到着順)。
    frames: Vec<Vec<u8>>,
    /// Data バッチの sequence_number(到着順)。
    seqs: Vec<u64>,
    heartbeats: u64,
    /// EndOfStream の (source_id, run_number)。
    eos: Option<(u32, u32)>,
}

/// EndOfStream が来るまで PULL を読み切る(rcvtimeo 10 s で失敗に倒す)。
fn collect_until_eos(sock: &zmq::Socket, link: &str) -> Collected {
    let mut c = Collected::default();
    loop {
        let bytes = sock
            .recv_bytes(0)
            .unwrap_or_else(|e| panic!("{link}: PULL recv failed/timed out: {e}"));
        match Message::<RawFrames>::from_msgpack(&bytes).unwrap() {
            Message::Data(batch) => {
                assert_eq!(batch.source_id, COBO_ID, "{link}: source_id");
                c.seqs.push(batch.sequence_number);
                c.frames
                    .extend(batch.payload.into_iter().map(|f| f.into_vec()));
            }
            Message::EndOfStream {
                source_id,
                run_number,
            } => {
                c.eos = Some((source_id, run_number));
                return c;
            }
            Message::Heartbeat { .. } => c.heartbeats += 1,
        }
    }
}

/// PULL を別スレッドへ渡して EOS まで読ませる(テスト本体は fake CoBo とコマンドを回す)。
fn spawn_collector(sock: zmq::Socket, link: &'static str) -> std::thread::JoinHandle<Collected> {
    std::thread::spawn(move || collect_until_eos(&sock, link))
}

/// sequence_number が 0 から 1 ずつ連続していること(= decoder の seq 検査が gap を見ない)。
fn assert_contiguous_from_zero(seqs: &[u64], link: &str) {
    let expected: Vec<u64> = (0..seqs.len() as u64).collect();
    assert_eq!(seqs, expected.as_slice(), "{link}: sequence_number に gap");
}

// ---------------------------------------------------------------------
// decoder リンクを満杯にする fake CoBo
// ---------------------------------------------------------------------

/// 満杯にする段のフレーム(16 KiB、中身 0xA1)。
const FLOOD_FRAME_BYTES: usize = 16 * 1024;
const FLOOD_FILL: u8 = 0xA1;
/// 1 回に書くフレーム数(書くたびに GetStatus で落とし始めたかを見る)。
const FLOOD_CHUNK_FRAMES: u64 = 64;

/// decoder リンクが満杯になって receiver が落とし始めるまで流し込む。戻り値 = 書いたフレーム数。
///
/// 満杯になるまでの量は PUSH/PULL の HWM(1 通ずつ)+ loopback の TCP バッファで決まり、
/// OS の自動調整で変わるので、固定量ではなく「落とし始めたこと」を観測して止める。
async fn flood_until_the_decoder_link_drops(stream: &mut TcpStream, cmd_ep: &str) -> u64 {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut written = 0u64;
    loop {
        for _ in 0..FLOOD_CHUNK_FRAMES {
            stream
                .write_all(&make_frame(FLOOD_FRAME_BYTES, FLOOD_FILL))
                .await
                .expect("fake CoBo write");
        }
        written += FLOOD_CHUNK_FRAMES;
        let status = rpc(cmd_ep, &Command::GetStatus).await;
        if metric_u64(&status, "decoder_link_dropped_frames") > 0 {
            return written;
        }
        assert!(
            Instant::now() < deadline,
            "{written} フレーム流しても decoder リンクが落とし始めない: {status:?}"
        );
    }
}

// ---------------------------------------------------------------------
// テスト 1: decoder 不在 → graw は全フレーム、decoder 分は数えて落とす、Error にならない
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_absent_decoder_is_dropped_and_counted_while_graw_gets_every_frame() {
    const RUN: u32 = 184;
    let ctx = zmq::Context::new();
    let (graw_pull, graw_ep) = bind_pull(&ctx, zmq_helper::DEFAULT_HWM);
    let graw_reader = spawn_collector(graw_pull, "graw-writer");

    let (cmd_ep, shutdown, task) = start_receiver(params(&graw_ep, &dead_endpoint())).await;
    let data_addr = configure_arm_start(&cmd_ep, RUN).await;

    // 非対称な 37 フレーム(長さ 3 種 × 中身ばらばら)。バッチの切れ目はタイミング次第なので、
    // バッチ数の正解は graw 側が受け取ったバッチ列から取る。
    let frames: Vec<Vec<u8>> = (0..37u8)
        .map(|i| make_frame([9_000, 17_000, 30_000][usize::from(i % 3)], 0x10 + i))
        .collect();
    let mut stream = TcpStream::connect(&data_addr).await.unwrap();
    for frame in &frames {
        stream.write_all(frame).await.unwrap();
    }
    stream.shutdown().await.unwrap(); // EOF = run 境界 → EOS

    let graw = graw_reader.join().unwrap();
    assert_eq!(
        graw.frames, frames,
        "graw は 1 フレームも欠けない(ロスレス)"
    );
    assert_contiguous_from_zero(&graw.seqs, "graw-writer");
    assert_eq!(graw.eos, Some((COBO_ID, RUN)), "graw へ EOS が届く");

    let status = rpc(&cmd_ep, &Command::GetStatus).await;
    assert_eq!(
        status.state,
        ComponentState::Running,
        "decoder リンクの落としは Error にしない(SPEC §1.4-7)"
    );
    assert_eq!(
        metric_u64(&status, "overflow_frames"),
        0,
        "graw は溢れていない"
    );
    assert_eq!(
        metric_u64(&status, "decoder_link_dropped_frames"),
        37,
        "decoder 不在なら全バッチを落とす = 全 37 フレーム"
    );
    assert_eq!(
        metric_u64(&status, "decoder_link_dropped_batches"),
        graw.seqs.len() as u64,
        "落としたバッチ数 = graw へ出たバッチ数(同じバッチ列)"
    );
    assert_eq!(
        metric_u64(&status, "batches"),
        graw.seqs.len() as u64,
        "batches は「どちらかのリンクへ出たバッチ」= graw へ出た分(意味は不変)"
    );
    for key in ["send_errors", "encode_errors", "messages_abandoned"] {
        assert_eq!(metric_u64(&status, key), 0, "metrics.{key}");
    }

    // decoder への EOS は落とさず待っている(SPEC §2.2)。Reset で初めて打ち切られ、1 通と数える。
    let reset = rpc(&cmd_ep, &Command::Reset).await;
    assert!(reset.success, "{}", reset.message);
    assert_eq!(reset.state, ComponentState::Idle);
    assert_eq!(
        metric_u64(&reset, "messages_abandoned"),
        1,
        "待っていた decoder 宛 EOS 1 通だけが打ち切られる"
    );
    println!(
        "absent decoder: graw frames={} batches={} / decoder_link_dropped frames={} batches={}",
        graw.frames.len(),
        graw.seqs.len(),
        metric_u64(&status, "decoder_link_dropped_frames"),
        metric_u64(&status, "decoder_link_dropped_batches"),
    );
    drop(stream);
    shutdown_and_join(shutdown, task).await;
}

// ---------------------------------------------------------------------
// テスト 2: 受け手を止めて落とさせた後、受信を再開 → decoder の seq に gap が無い
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stalled_decoder_that_resumes_sees_no_sequence_gap() {
    const RUN: u32 = 185;
    /// 再開後に流すフレーム(12 KiB、中身 0xE2 — 満杯段のフレームと長さも中身も違う)。
    const RESUME_FRAME_BYTES: usize = 12 * 1024;
    const RESUME_FILL: u8 = 0xE2;
    const RESUME_FRAMES: u64 = 6;

    let ctx = zmq::Context::new();
    let (graw_pull, graw_ep) = bind_pull(&ctx, zmq_helper::DEFAULT_HWM);
    let graw_reader = spawn_collector(graw_pull, "graw-writer");
    let (decoder_pull, decoder_ep) = bind_pull(&ctx, TEST_HWM); // まだ読まない = 止まった受け手

    let (cmd_ep, shutdown, task) = start_receiver(params(&graw_ep, &decoder_ep)).await;
    let data_addr = configure_arm_start(&cmd_ep, RUN).await;

    let mut stream = TcpStream::connect(&data_addr).await.unwrap();
    let flooded = flood_until_the_decoder_link_drops(&mut stream, &cmd_ep).await;

    // 受信を再開し、その後のフレームは間隔を空けて 1 バッチずつ流す(10 ms のバッチ期限で閉じる)。
    let decoder_reader = spawn_collector(decoder_pull, "decoder");
    for _ in 0..RESUME_FRAMES {
        stream
            .write_all(&make_frame(RESUME_FRAME_BYTES, RESUME_FILL))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await; // ペーシング(待ち合わせではない)
    }
    stream.shutdown().await.unwrap(); // EOF → EOS
    let total = flooded + RESUME_FRAMES;

    let graw = graw_reader.join().unwrap();
    let decoder = decoder_reader.join().unwrap();
    let status = rpc(&cmd_ep, &Command::GetStatus).await;
    let dropped_frames = metric_u64(&status, "decoder_link_dropped_frames");
    let dropped_batches = metric_u64(&status, "decoder_link_dropped_batches");

    // 本題: 落としたバッチで seq を進めていない = decoder から見て gap が無い(SPEC §1.4-7)
    assert_contiguous_from_zero(&decoder.seqs, "decoder");
    assert!(dropped_frames > 0, "満杯の間は落としている");
    assert!(
        decoder
            .frames
            .iter()
            .any(|f| f.len() == RESUME_FRAME_BYTES && f[f.len() - 1] == RESUME_FILL),
        "再開後のフレームが decoder に届いている(落とした後も流れは戻る)"
    );
    assert_eq!(decoder.eos, Some((COBO_ID, RUN)), "decoder へ EOS が届く");

    // 勘定: graw は全部、decoder は「届いた + 数えて落とした」で全部
    assert_eq!(
        graw.frames.len() as u64,
        total,
        "graw は全フレーム(ロスレス)"
    );
    assert_contiguous_from_zero(&graw.seqs, "graw-writer");
    assert_eq!(
        decoder.frames.len() as u64 + dropped_frames,
        total,
        "decoder に届いたフレーム + decoder_link_dropped_frames = 全フレーム"
    );
    assert_eq!(
        decoder.seqs.len() as u64 + dropped_batches,
        graw.seqs.len() as u64,
        "decoder に届いたバッチ + 落としたバッチ = graw へ出たバッチ"
    );
    assert_eq!(status.state, ComponentState::Running, "Error にならない");
    assert_eq!(metric_u64(&status, "overflow_frames"), 0);
    println!(
        "stalled → resumed: total frames={total} graw batches={} / decoder batches={} \
         frames={} / dropped batches={dropped_batches} frames={dropped_frames}",
        graw.seqs.len(),
        decoder.seqs.len(),
        decoder.frames.len(),
    );

    let stopped = rpc(&cmd_ep, &Command::Stop).await;
    assert!(stopped.success, "{}", stopped.message);
    let reset = rpc(&cmd_ep, &Command::Reset).await;
    assert_eq!(reset.state, ComponentState::Idle);
    drop(stream);
    shutdown_and_join(shutdown, task).await;
}

// ---------------------------------------------------------------------
// テスト 3: decoder リンクが満杯でも EOS は落とさず、空くのを待って届く
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_end_of_stream_waits_for_a_full_decoder_link_and_is_delivered() {
    const RUN: u32 = 186;
    let ctx = zmq::Context::new();
    let (graw_pull, graw_ep) = bind_pull(&ctx, zmq_helper::DEFAULT_HWM);
    let graw_reader = spawn_collector(graw_pull, "graw-writer");
    let (decoder_pull, decoder_ep) = bind_pull(&ctx, TEST_HWM); // 読まない = 満杯のまま

    let (cmd_ep, shutdown, task) = start_receiver(params(&graw_ep, &decoder_ep)).await;
    let data_addr = configure_arm_start(&cmd_ep, RUN).await;

    let mut stream = TcpStream::connect(&data_addr).await.unwrap();
    let flooded = flood_until_the_decoder_link_drops(&mut stream, &cmd_ep).await;
    stream.shutdown().await.unwrap(); // EOF → EOS(decoder リンクは満杯のまま)

    // graw は先に EOS まで受け取る(EOS の送出は graw → decoder の順)
    let graw = graw_reader.join().unwrap();
    assert_eq!(graw.eos, Some((COBO_ID, RUN)));
    assert_eq!(graw.frames.len() as u64, flooded);

    // decoder 宛の EOS は満杯のリンクで待っている(打ち切っても落としてもいない)
    let waiting = rpc(&cmd_ep, &Command::GetStatus).await;
    assert_eq!(
        metric_u64(&waiting, "messages_abandoned"),
        0,
        "EOS は打ち切らない"
    );
    assert_eq!(waiting.state, ComponentState::Running);

    // 受け手が読み始めれば、待っていた EOS が届く
    let decoder = collect_until_eos(&decoder_pull, "decoder");
    assert_eq!(
        decoder.eos,
        Some((COBO_ID, RUN)),
        "満杯のリンクでも EOS は空くのを待って届く(SPEC §2.2 / §1.4-7)"
    );
    assert_contiguous_from_zero(&decoder.seqs, "decoder");

    let status = rpc(&cmd_ep, &Command::GetStatus).await;
    let dropped_frames = metric_u64(&status, "decoder_link_dropped_frames");
    assert!(dropped_frames > 0);
    assert_eq!(decoder.frames.len() as u64 + dropped_frames, flooded);
    assert_eq!(metric_u64(&status, "messages_abandoned"), 0);
    assert_eq!(metric_u64(&status, "overflow_frames"), 0);
    println!(
        "EOS on a full link: flooded={flooded} decoder frames={} dropped frames={dropped_frames}",
        decoder.frames.len()
    );

    let stopped = rpc(&cmd_ep, &Command::Stop).await;
    assert!(stopped.success, "{}", stopped.message);
    let reset = rpc(&cmd_ep, &Command::Reset).await;
    assert_eq!(reset.state, ComponentState::Idle);
    drop(stream);
    shutdown_and_join(shutdown, task).await;
}

// ---------------------------------------------------------------------
// テスト 7: decoder 不在のアイドル中、Heartbeat の落としを数える
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn heartbeats_to_an_absent_decoder_are_dropped_and_counted_while_idle() {
    const RUN: u32 = 187;
    let ctx = zmq::Context::new();
    let (graw_pull, graw_ep) = bind_pull(&ctx, zmq_helper::DEFAULT_HWM);
    let graw_reader = spawn_collector(graw_pull, "graw-writer");

    let mut p = params(&graw_ep, &dead_endpoint());
    p.heartbeat_ms = 50; // アイドル Heartbeat を速く回す
    let (cmd_ep, shutdown, task) = start_receiver(p).await;
    configure_arm_start(&cmd_ep, RUN).await; // CoBo は繋がない = アイドル

    let idle = poll_status(
        &cmd_ep,
        Duration::from_secs(5),
        "decoder_link_dropped_heartbeats >= 3",
        |r| metric_u64(r, "decoder_link_dropped_heartbeats") >= 3,
    )
    .await;
    assert_eq!(
        idle.state,
        ComponentState::Running,
        "Heartbeat の落としも Error にしない"
    );
    assert_eq!(metric_u64(&idle, "decoder_link_dropped_frames"), 0);
    assert_eq!(metric_u64(&idle, "decoder_link_dropped_batches"), 0);
    assert_eq!(metric_u64(&idle, "batches"), 0);

    // Stop の強制 EOS で graw 側を閉じ、graw が受けた Heartbeat 数と照合する
    let stopped = rpc(&cmd_ep, &Command::Stop).await;
    assert!(stopped.success, "{}", stopped.message);
    let graw = graw_reader.join().unwrap();
    assert_eq!(graw.eos, Some((COBO_ID, RUN)));
    let status = rpc(&cmd_ep, &Command::GetStatus).await;
    assert_eq!(
        metric_u64(&status, "decoder_link_dropped_heartbeats"),
        graw.heartbeats,
        "Heartbeat 1 回 = graw へ 1 通 + decoder で 1 回の落とし"
    );
    println!(
        "idle heartbeats: graw received {} / decoder dropped {}",
        graw.heartbeats,
        metric_u64(&status, "decoder_link_dropped_heartbeats")
    );

    let reset = rpc(&cmd_ep, &Command::Reset).await;
    assert_eq!(reset.state, ComponentState::Idle);
    shutdown_and_join(shutdown, task).await;
}
