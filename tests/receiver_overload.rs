//! receiver の過負荷規約(SPEC §1.4-1/3、CLAUDE.md「receiver は drain のみ・never-stop」)。
//!
//! PULL 側を止めて送信タスクを詰まらせ、内部キューを溢れさせたときに
//! - `overflow_frames` が増える(silent に落とさない)
//! - コンポーネントが Error を報告する
//! - **その間も drain は止まらない**(fake CoBo の送信がブロックされず、受信バイト数が伸び続ける)
//!
//! ことを実測する。ここが「下流の遅延がソケットへ逆圧してはならない」の回帰テスト。

#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::{broadcast, oneshot};
use tpcdaq::command::{Command, CommandResponse, ComponentState, RunConfig};
use tpcdaq::receiver::{run_receiver, ReceiverParams};
use tpcdaq::zmq_helper;

/// 送信タスクが数バッチで詰まるように、HWM は極小にする(既定 1000 だと詰まるまでが長いだけ)。
const TEST_HWM: i32 = 1;
/// 1 フレーム 8 KiB × 512 = 4 MiB。TCP の送信バッファよりずっと大きいので、
/// drain が止まれば fake CoBo の write は必ずブロックする。
const FRAME_BYTES: usize = 8 * 1024;
const FRAME_COUNT: usize = 512;

fn bind_stalled_pull(ctx: &zmq::Context) -> (zmq::Socket, String) {
    let sock = ctx.socket(zmq::PULL).unwrap();
    sock.set_linger(0).unwrap();
    zmq_helper::apply_hwm(&sock, TEST_HWM).unwrap(); // HWM は bind 前に設定する
    sock.bind("tcp://127.0.0.1:0").unwrap();
    let endpoint = sock.get_last_endpoint().unwrap().unwrap();
    (sock, endpoint) // 以後 recv しない = 受け手が止まっている状態
}

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

fn make_frame(total_bytes: usize, fill: u8) -> Vec<u8> {
    let mut f = vec![fill; total_bytes];
    f[0] = 0x80; // little-endian, blkSize=1
    let blk = total_bytes as u32;
    f[1] = (blk & 0xFF) as u8;
    f[2] = ((blk >> 8) & 0xFF) as u8;
    f[3] = ((blk >> 16) & 0xFF) as u8;
    f
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

/// SPEC §1.2 v1.4(ロスレス PUSH は `ZMQ_IMMEDIATE`)が作る新しい状況の回帰テスト:
/// **下流が「止まっている」のではなく「そもそも居ない」**とき。
///
/// v1.4 以前は libzmq が未接続の相手にも HWM 分を積んで send が成功を返していたため、
/// 下流不在でもしばらく「送れたこと」になっていた(= 不可視の喪失)。IMMEDIATE 後は
/// 1 通も送れない。その状態でも **never-stop が保たれる**こと —— drain はソケットを読み切り、
/// 溢れは `overflow_frames` + Error として可視化される —— を固定する。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_absent_downstream_never_counts_as_sent_and_still_does_not_stop_the_drain() {
    // 誰も bind していないアドレス 2 本(TcpListener で確保して即座に手放す)。
    let dead_endpoint = || {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        format!("tcp://{addr}")
    };

    let params = ReceiverParams {
        cobo_id: 0,
        listen: "127.0.0.1:0".to_string(),
        command_listen: "tcp://127.0.0.1:0".to_string(),
        graw_writer_endpoint: dead_endpoint(),
        decoder_endpoint: dead_endpoint(),
        batch_max_bytes: 4 * 1024,
        batch_max_ms: 10,
        queue_frames: 2,
        heartbeat_ms: 1_000,
        hwm: TEST_HWM,
    };

    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    let (ep_tx, ep_rx) = oneshot::channel();
    let task = tokio::spawn(run_receiver(params, shutdown_rx, Some(ep_tx)));
    let cmd_ep = ep_rx.await.unwrap();

    rpc(
        &cmd_ep,
        &Command::Configure(RunConfig {
            run_number: 98,
            comment: "absent downstream".to_string(),
            config: serde_json::Value::Null,
        }),
    )
    .await;
    let armed = rpc(&cmd_ep, &Command::Arm).await;
    let data_addr = armed.metrics.as_ref().unwrap()["bind_address"]
        .as_str()
        .unwrap()
        .to_string();
    rpc(&cmd_ep, &Command::Start { run_number: 98 }).await;

    // fake CoBo: 4 MiB を全速で流し込む。drain が止まれば TCP バッファが詰まってここが返らない。
    let total_bytes = FRAME_BYTES * FRAME_COUNT;
    tokio::time::timeout(Duration::from_secs(20), async {
        let mut stream = TcpStream::connect(&data_addr).await.unwrap();
        for i in 0..FRAME_COUNT {
            let frame = make_frame(FRAME_BYTES, (i % 251) as u8);
            stream.write_all(&frame).await.expect("fake CoBo write");
        }
        stream.flush().await.unwrap();
        stream
    })
    .await
    .expect("fake CoBo の送信がブロックされた = drain が止まっている(never-stop 違反)");

    let status = poll_status(
        &cmd_ep,
        Duration::from_secs(10),
        "all bytes drained even with no downstream at all",
        |r| metric_u64(r, "bytes") >= total_bytes as u64,
    )
    .await;
    assert_eq!(metric_u64(&status, "bytes"), total_bytes as u64);
    assert_eq!(metric_u64(&status, "frames"), FRAME_COUNT as u64);
    assert!(
        metric_u64(&status, "overflow_frames") > 0,
        "下流不在なら内部キューは必ず溢れる — それが数えられていること(silent 禁止)"
    );
    assert_eq!(
        metric_u64(&status, "batches"),
        0,
        "下流不在では 1 バッチも「送れたこと」にしない(SPEC §1.2 v1.4)"
    );
    assert_eq!(
        status.state,
        ComponentState::Error,
        "内部キュー満杯はシステム過負荷 = Error(SPEC §1.4-3)"
    );
    println!(
        "absent downstream: 受信 bytes={} frames={} overflow_frames={} batches={}",
        metric_u64(&status, "bytes"),
        metric_u64(&status, "frames"),
        metric_u64(&status, "overflow_frames"),
        metric_u64(&status, "batches"),
    );

    let reset = rpc(&cmd_ep, &Command::Reset).await;
    assert!(reset.success, "{}", reset.message);

    shutdown_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("receiver did not stop within 5 s")
        .unwrap();
}

/// TODO/023-2(= P2 レビュー R-P2-3): **下流不在で送出がブロックしている最中でも
/// コンポーネントを畳める**こと。
///
/// 以前の `send_on` は SNDTIMEO 無しのブロッキング送信だったので、下流が全部死ぬと
/// 送信スレッドが `zmq_send` の中で永久に固まり(broadcast stop では起こせない)、
/// しかも失敗が 1 つも数えられていなかった。修正後は decoder と同じ形:
/// **捨てない・ただし中断可能**(`Reset` / 畳み込みのみ)+ 打ち切りは可視カウント。
///
/// `Reset` の応答が返ってきた時点で送信スレッドは **join 済み**(`do_reset` が join する)
/// なので、応答に載る `messages_abandoned` は確定値であり、スレッドが残っていないことの
/// 直接の証拠でもある。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_blocked_send_is_abandoned_on_reset_counted_and_the_sender_thread_is_joined() {
    let dead_endpoint = || {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        format!("tcp://{addr}")
    };

    let params = ReceiverParams {
        cobo_id: 0,
        listen: "127.0.0.1:0".to_string(),
        command_listen: "tcp://127.0.0.1:0".to_string(),
        graw_writer_endpoint: dead_endpoint(),
        decoder_endpoint: dead_endpoint(),
        batch_max_bytes: 4 * 1024, // 1 フレームで 1 バッチ = すぐ送信側がブロックする
        batch_max_ms: 10,
        queue_frames: 256, // 溢れさせない(見たいのは送信ブロックであって overflow ではない)
        heartbeat_ms: 60_000, // Heartbeat では起こさない(打ち切りの粒度は SNDTIMEO)
        hwm: TEST_HWM,
    };

    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    let (ep_tx, ep_rx) = oneshot::channel();
    let task = tokio::spawn(run_receiver(params, shutdown_rx, Some(ep_tx)));
    let cmd_ep = ep_rx.await.unwrap();

    rpc(
        &cmd_ep,
        &Command::Configure(RunConfig {
            run_number: 97,
            comment: "blocked send".to_string(),
            config: serde_json::Value::Null,
        }),
    )
    .await;
    let armed = rpc(&cmd_ep, &Command::Arm).await;
    let data_addr = armed.metrics.as_ref().unwrap()["bind_address"]
        .as_str()
        .unwrap()
        .to_string();
    rpc(&cmd_ep, &Command::Start { run_number: 97 }).await;

    // 数フレームだけ流す(内部キューは 256 段なので溢れない)。送信スレッドは
    // 最初のバッチで下流不在にぶつかってブロックする。
    let mut stream = TcpStream::connect(&data_addr).await.unwrap();
    for i in 0..4 {
        stream
            .write_all(&make_frame(FRAME_BYTES, 0xB0 + i))
            .await
            .expect("fake CoBo write");
    }
    stream.flush().await.unwrap();

    // 全フレームがソケットから読み切られている(never-stop は保たれたまま)。
    let status = poll_status(
        &cmd_ep,
        Duration::from_secs(10),
        "all frames drained",
        |r| metric_u64(r, "frames") >= 4,
    )
    .await;
    assert_eq!(
        metric_u64(&status, "messages_abandoned"),
        0,
        "まだ諦めていない(下流の背圧で待っている = ロスレス)"
    );
    assert_eq!(metric_u64(&status, "overflow_frames"), 0, "溢れてはいない");

    // Stop は打ち切らない(強制 EOS を流す経路 — SPEC §1.3 v1.6)。即座に返ること。
    let stop_started = Instant::now();
    let stopped = rpc(&cmd_ep, &Command::Stop).await;
    let stop_elapsed = stop_started.elapsed();
    assert!(stopped.success, "{}", stopped.message);
    assert!(
        stop_elapsed < Duration::from_secs(2),
        "Stop に {stop_elapsed:?} 掛かった(送出待ちで固まっている)"
    );

    // Reset は打ち切ってよい唯一の経路。応答が返る = 送信スレッドを join できた。
    let reset_started = Instant::now();
    let reset = rpc(&cmd_ep, &Command::Reset).await;
    let reset_elapsed = reset_started.elapsed();
    assert!(reset.success, "{}", reset.message);
    assert_eq!(reset.state, ComponentState::Idle);
    assert!(
        reset_elapsed < Duration::from_secs(2),
        "Reset に {reset_elapsed:?} 掛かった(送信スレッドが畳めていない)"
    );

    // 破棄は必ず数えられ、metrics に出る(数えられていることが打ち切りを許す条件)。
    assert!(
        metric_u64(&reset, "messages_abandoned") >= 1,
        "打ち切ったのに数えていない: {:?}",
        reset.metrics
    );
    for key in ["messages_abandoned", "encode_errors", "send_errors"] {
        assert!(
            reset.metrics.as_ref().unwrap().get(key).is_some(),
            "metrics.{key} が GetStatus に出ていない: {:?}",
            reset.metrics
        );
    }
    assert_eq!(
        metric_u64(&reset, "batches"),
        0,
        "下流不在では 1 バッチも「送れたこと」にしない(SPEC §1.2 v1.4)"
    );
    println!(
        "blocked send: Stop {stop_elapsed:?} / Reset {reset_elapsed:?}、\
         messages_abandoned={} encode_errors={} send_errors={}",
        metric_u64(&reset, "messages_abandoned"),
        metric_u64(&reset, "encode_errors"),
        metric_u64(&reset, "send_errors"),
    );
    drop(stream);

    shutdown_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("receiver did not stop within 5 s")
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn queue_overflow_counts_frames_enters_error_and_keeps_draining() {
    let ctx = zmq::Context::new();
    let (_graw_pull, graw_ep) = bind_stalled_pull(&ctx);
    let (_decoder_pull, decoder_ep) = bind_stalled_pull(&ctx);

    let params = ReceiverParams {
        cobo_id: 0,
        listen: "127.0.0.1:0".to_string(),
        command_listen: "tcp://127.0.0.1:0".to_string(),
        graw_writer_endpoint: graw_ep,
        decoder_endpoint: decoder_ep,
        batch_max_bytes: 4 * 1024, // 1 フレームで 1 バッチ = すぐ送信側が詰まる
        batch_max_ms: 10,
        queue_frames: 2, // 極小の内部キュー
        heartbeat_ms: 1_000,
        hwm: TEST_HWM,
    };

    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    let (ep_tx, ep_rx) = oneshot::channel();
    let task = tokio::spawn(run_receiver(params, shutdown_rx, Some(ep_tx)));
    let cmd_ep = ep_rx.await.unwrap();

    rpc(
        &cmd_ep,
        &Command::Configure(RunConfig {
            run_number: 99,
            comment: "overload".to_string(),
            config: serde_json::Value::Null,
        }),
    )
    .await;
    let armed = rpc(&cmd_ep, &Command::Arm).await;
    let data_addr = armed.metrics.as_ref().unwrap()["bind_address"]
        .as_str()
        .unwrap()
        .to_string();
    rpc(&cmd_ep, &Command::Start { run_number: 99 }).await;

    // fake CoBo: 4 MiB を全速で流し込む。drain が止まっていれば TCP バッファが詰まって
    // ここが返ってこない(= タイムアウトでテストが落ちる)。
    let total_bytes = FRAME_BYTES * FRAME_COUNT;
    let write_started = Instant::now();
    tokio::time::timeout(Duration::from_secs(20), async {
        let mut stream = TcpStream::connect(&data_addr).await.unwrap();
        for i in 0..FRAME_COUNT {
            let frame = make_frame(FRAME_BYTES, (i % 251) as u8);
            stream.write_all(&frame).await.expect("fake CoBo write");
        }
        stream.flush().await.unwrap();
        stream
    })
    .await
    .expect("fake CoBo の送信がブロックされた = drain が止まっている(never-stop 違反)");
    let write_elapsed = write_started.elapsed();

    // 溢れが可視化され、Error に落ちていること(SPEC §1.4-3)
    let status = poll_status(
        &cmd_ep,
        Duration::from_secs(10),
        "overflow_frames > 0",
        |r| metric_u64(r, "overflow_frames") > 0,
    )
    .await;
    assert_eq!(
        status.state,
        ComponentState::Error,
        "内部キュー満杯はシステム過負荷 = Error(SPEC §1.4-3)"
    );
    assert!(!status.success || status.state == ComponentState::Error);

    // drain は溢れている間も止まっていない: 4 MiB 全部をソケットから読み切る
    let status = poll_status(
        &cmd_ep,
        Duration::from_secs(10),
        "all bytes drained from the socket",
        |r| metric_u64(r, "bytes") >= total_bytes as u64,
    )
    .await;
    assert_eq!(metric_u64(&status, "bytes"), total_bytes as u64);
    assert_eq!(metric_u64(&status, "frames"), FRAME_COUNT as u64);
    assert!(
        metric_u64(&status, "overflow_frames") > 0,
        "溢れたフレームは数えられていること(silent 禁止)"
    );
    assert!(
        metric_u64(&status, "overflow_frames") < FRAME_COUNT as u64,
        "全滅ではなく、詰まるまでの分は送れているはず"
    );
    assert!(
        write_elapsed < Duration::from_secs(20),
        "送信に {write_elapsed:?} かかった"
    );
    println!(
        "overload: fake CoBo が {total_bytes} B を {write_elapsed:?} で送出、\
         受信 bytes={} frames={} overflow_frames={} batches={}",
        metric_u64(&status, "bytes"),
        metric_u64(&status, "frames"),
        metric_u64(&status, "overflow_frames"),
        metric_u64(&status, "batches"),
    );

    // Error からの脱出は Reset だけ(SPEC §1.3)
    let bad = rpc(&cmd_ep, &Command::Stop).await;
    assert!(!bad.success, "Error からの Stop は通らない");
    let reset = rpc(&cmd_ep, &Command::Reset).await;
    assert!(reset.success, "{}", reset.message);
    assert_eq!(reset.state, ComponentState::Idle);

    shutdown_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("receiver did not stop within 5 s")
        .unwrap();
}
