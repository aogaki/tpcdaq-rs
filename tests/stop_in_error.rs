//! Error 中の `Stop` は実行する(TODO/184 B、SPEC v1.53 §1.3「異常中止」②)。
//!
//! run 197 では Error の receiver が `Stop` を「illegal transition Error -> Configured」で拒否し、
//! 強制 EOS を出せずに run が EOS 未達で終わった。v1.53 から receiver / decoder / graw-writer は
//! **state == Error で `Stop` を受けたら通常の Stop の処理を実行し、状態は Error のまま、応答は成功**
//! (遷移表 `ComponentState::can_transition_to` は変えない — Error → Configured を合法にすると
//! 「Error が消える」)。続く `Reset` で Idle。
//!
//! 発注書 184 §2 のテスト 5。ポートはすべて 0(動的)。条件待ちはポーリング + タイムアウト。

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_bytes::ByteBuf;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::{broadcast, oneshot};
use tpcdaq::command::{Command, CommandResponse, ComponentState, RunConfig};
use tpcdaq::decoder::{run_decoder, DecoderParams};
use tpcdaq::graw_writer::{run_graw_writer, GrawWriterParams};
use tpcdaq::msg::{Batch, Message, RawFrames};
use tpcdaq::receiver::{run_receiver, ReceiverParams};
use tpcdaq::zmq_helper;

/// Error 中の Stop の応答メッセージに含まれる句(3 コンポーネント共通)。
const STOP_IN_ERROR: &str = "executed in Error state";

// ---------------------------------------------------------------------
// 共通ヘルパ
// ---------------------------------------------------------------------

/// REQ で 1 往復する(毎回新しい REQ ソケット)。
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

fn metric_u64(resp: &CommandResponse, key: &str) -> u64 {
    resp.metrics.as_ref().unwrap()[key]
        .as_u64()
        .unwrap_or_else(|| panic!("metrics.{key} is not a number: {:?}", resp.metrics))
}

fn bind_address(resp: &CommandResponse) -> serde_json::Value {
    resp.metrics.as_ref().unwrap()["bind_address"].clone()
}

async fn configure_arm(cmd_ep: &str, run_number: u32) -> String {
    let configured = rpc(
        cmd_ep,
        &Command::Configure(RunConfig {
            run_number,
            comment: "stop in error".to_string(),
            config: serde_json::Value::Null,
        }),
    )
    .await;
    assert!(configured.success, "{}", configured.message);
    let armed = rpc(cmd_ep, &Command::Arm).await;
    assert!(armed.success, "{}", armed.message);
    armed.metrics.as_ref().unwrap()["bind_address"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Error 中の Stop の受け入れ条件(3 コンポーネント共通): 成功・Error のまま・それと分かる文言。
fn assert_stop_executed_in_error(resp: &CommandResponse, who: &str) {
    assert!(
        resp.success,
        "{who}: Error 中の Stop は成功を返す: {resp:?}"
    );
    assert_eq!(
        resp.state,
        ComponentState::Error,
        "{who}: Stop を実行しても Error は消えない(Reset でのみ脱出)"
    );
    assert!(
        resp.message.contains(STOP_IN_ERROR),
        "{who}: 応答で「Error 中に Stop を実行した」と分かること: {:?}",
        resp.message
    );
}

/// Batch<RawFrames> を 1 通 PUSH する(decoder / graw-writer へ直接投入)。
fn send_batch(push: &zmq::Socket, source_id: u32, run_number: u32, seq: u64, frames: &[Vec<u8>]) {
    let payload: RawFrames = frames.iter().cloned().map(ByteBuf::from).collect();
    let msg: Message<RawFrames> = Message::Data(Batch {
        source_id,
        run_number,
        sequence_number: seq,
        created_ns: 0,
        payload,
    });
    push.send(msg.to_msgpack().unwrap(), 0).unwrap();
}

fn connect_push(ctx: &zmq::Context, endpoint: &str) -> zmq::Socket {
    let push = ctx.socket(zmq::PUSH).unwrap();
    push.set_linger(0).unwrap();
    zmq_helper::apply_push_hwm(&push).unwrap();
    push.connect(endpoint).unwrap();
    push
}

fn bind_pull(ctx: &zmq::Context, hwm: i32) -> (zmq::Socket, String) {
    let sock = ctx.socket(zmq::PULL).unwrap();
    sock.set_linger(0).unwrap();
    zmq_helper::apply_hwm(&sock, hwm).unwrap();
    sock.bind("tcp://127.0.0.1:0").unwrap();
    sock.set_rcvtimeo(10_000).unwrap();
    let endpoint = sock.get_last_endpoint().unwrap().unwrap();
    (sock, endpoint)
}

// ---------------------------------------------------------------------
// receiver: 溢れて Error → Stop で強制 EOS が両リンクへ出る、Error のまま、Reset で Idle
// ---------------------------------------------------------------------

/// 受け手が読むまで EOS を待つ(Data / Heartbeat は読み捨て)。EOS の (source_id, run_number)。
fn read_until_eos(sock: &zmq::Socket, link: &str) -> (u32, u32) {
    loop {
        let bytes = sock
            .recv_bytes(0)
            .unwrap_or_else(|e| panic!("{link}: no EndOfStream (recv failed/timed out: {e})"));
        if let Message::EndOfStream {
            source_id,
            run_number,
        } = Message::<RawFrames>::from_msgpack(&bytes).unwrap()
        {
            return (source_id, run_number);
        }
    }
}

fn make_receiver_frame(total_bytes: usize, fill: u8) -> Vec<u8> {
    let mut f = vec![fill; total_bytes];
    f[0] = 0x80; // little-endian, blkSize=1
    let blk = total_bytes as u32;
    f[1] = (blk & 0xFF) as u8;
    f[2] = ((blk >> 8) & 0xFF) as u8;
    f[3] = ((blk >> 16) & 0xFF) as u8;
    f
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_receiver_in_error_executes_stop_forces_eos_on_both_links_and_stays_in_error() {
    const RUN: u32 = 197; // run 197 の再現(Error の receiver が強制 EOS を出せなかった)
    const COBO: u32 = 1;
    let ctx = zmq::Context::new();
    // 両リンクとも受け手を止める(graw が詰まる = 内部キューが溢れる = Error、SPEC §1.4-3)
    let (graw_pull, graw_ep) = bind_pull(&ctx, 1);
    let (decoder_pull, decoder_ep) = bind_pull(&ctx, 1);

    let params = ReceiverParams {
        cobo_id: COBO,
        listen: "127.0.0.1:0".to_string(),
        command_listen: "tcp://127.0.0.1:0".to_string(),
        graw_writer_endpoint: graw_ep,
        decoder_endpoint: decoder_ep,
        batch_max_bytes: 4 * 1024, // 1 フレーム = 1 バッチ
        batch_max_ms: 10,
        queue_frames: 2, // 極小の内部キュー
        heartbeat_ms: 1_000,
        hwm: 1,
    };
    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    let (ep_tx, ep_rx) = oneshot::channel();
    let task = tokio::spawn(run_receiver(params, shutdown_rx, Some(ep_tx)));
    let cmd_ep = ep_rx.await.unwrap();

    let data_addr = configure_arm(&cmd_ep, RUN).await;
    let started = rpc(&cmd_ep, &Command::Start { run_number: RUN }).await;
    assert!(started.success, "{}", started.message);

    // 4 MiB を流し込み、接続は開けたまま(EOF の EOS ではなく Stop の強制 EOS を見たい)
    let mut stream = TcpStream::connect(&data_addr).await.unwrap();
    for i in 0..512usize {
        stream
            .write_all(&make_receiver_frame(8 * 1024, (i % 251) as u8))
            .await
            .unwrap();
    }
    stream.flush().await.unwrap();
    let errored = poll_status(&cmd_ep, Duration::from_secs(10), "Error", |r| {
        r.state == ComponentState::Error
    })
    .await;
    assert!(metric_u64(&errored, "overflow_frames") > 0);

    // 本題: Error でも Stop は実行される
    let stopped = rpc(&cmd_ep, &Command::Stop).await;
    assert_stop_executed_in_error(&stopped, "receiver");

    // 受け手が読み始めれば、強制 EOS が両リンクへ出る(Stop が drain を畳んだ証拠)
    let graw_reader = std::thread::spawn(move || read_until_eos(&graw_pull, "graw-writer"));
    let decoder_reader = std::thread::spawn(move || read_until_eos(&decoder_pull, "decoder"));
    assert_eq!(graw_reader.join().unwrap(), (COBO, RUN), "graw へ強制 EOS");
    assert_eq!(
        decoder_reader.join().unwrap(),
        (COBO, RUN),
        "decoder へ強制 EOS"
    );

    let status = rpc(&cmd_ep, &Command::GetStatus).await;
    assert_eq!(
        status.state,
        ComponentState::Error,
        "EOS を出した後も Error のまま"
    );
    // 2 度目の Stop も安全(二重に畳んでも panic しない)
    let again = rpc(&cmd_ep, &Command::Stop).await;
    assert_stop_executed_in_error(&again, "receiver (2nd Stop)");

    let reset = rpc(&cmd_ep, &Command::Reset).await;
    assert!(reset.success, "{}", reset.message);
    assert_eq!(reset.state, ComponentState::Idle, "Reset で Idle(今どおり)");

    drop(stream);
    shutdown_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("receiver did not stop within 5 s")
        .unwrap();
}

// ---------------------------------------------------------------------
// decoder: seq gap で Error → Stop を実行、Error のまま、Reset で Idle
// ---------------------------------------------------------------------

/// frameType 1 の CoBo フレーム(decoder_integration.rs と同じ手組み、big-endian、blkSize=1)。
fn make_decoder_frame(cobo: u8, event_idx: u32, item_count: usize) -> Vec<u8> {
    const HEADER_BYTES: usize = 88;
    fn put(buf: &mut [u8], offset: usize, value: u64, n: usize) {
        for (i, b) in buf[offset..offset + n].iter_mut().enumerate() {
            *b = ((value >> (8 * (n - 1 - i))) & 0xFF) as u8;
        }
    }
    let total = HEADER_BYTES + item_count * 4;
    let mut b = vec![0u8; total];
    put(&mut b, 1, total as u64, 3);
    put(&mut b, 5, 1, 2); // frameType = 1
    put(&mut b, 8, HEADER_BYTES as u64, 2);
    put(&mut b, 10, 4, 2); // itemSize
    put(&mut b, 12, item_count as u64, 4);
    put(&mut b, 22, u64::from(event_idx), 4);
    b[26] = cobo;
    for i in 0..item_count {
        // aget / chan / adc が非対称な item(bucket は 0)
        let (aget, chan, adc) = ((i % 4) as u32, (i % 68) as u32, (100 + i as u32 * 7) % 4096);
        let w = (aget << 30) | (chan << 23) | adc;
        put(&mut b, HEADER_BYTES + i * 4, u64::from(w), 4);
    }
    b
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_decoder_in_error_executes_stop_stays_in_error_and_resets_to_idle() {
    const RUN: u32 = 31;
    let ctx = zmq::Context::new();
    let (_fragment_pull, fragment_ep) = bind_pull(&ctx, zmq_helper::DEFAULT_HWM);
    let params = DecoderParams {
        pull_bind: "tcp://127.0.0.1:0".to_string(),
        push_connect: fragment_ep,
        command_listen: "tcp://127.0.0.1:0".to_string(),
        batch_max_bytes: 8 * 1024 * 1024,
        batch_max_ms: 10,
        heartbeat_ms: 60_000,
        send_timeout_ms: 200,
        workers: 1,
        expected_sources: vec![0],
    };
    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    let (ep_tx, ep_rx) = oneshot::channel();
    let task = tokio::spawn(run_decoder(params, shutdown_rx, Some(ep_tx)));
    let cmd_ep = ep_rx.await.unwrap();

    let pull_bind = configure_arm(&cmd_ep, RUN).await;
    let push = connect_push(&ctx, &pull_bind);
    let started = rpc(&cmd_ep, &Command::Start { run_number: RUN }).await;
    assert!(started.success, "{}", started.message);

    send_batch(&push, 0, RUN, 0, &[make_decoder_frame(0, 1, 4)]);
    send_batch(&push, 0, RUN, 2, &[make_decoder_frame(0, 2, 6)]); // seq=1 を飛ばす = Error
    let errored = poll_status(&cmd_ep, Duration::from_secs(5), "Error", |r| {
        r.state == ComponentState::Error
    })
    .await;
    assert_eq!(metric_u64(&errored, "seq_gaps"), 1);
    assert!(
        bind_address(&errored).is_string(),
        "Stop 前は bind_address を持つ"
    );

    let stopped = rpc(&cmd_ep, &Command::Stop).await;
    assert_stop_executed_in_error(&stopped, "decoder");
    assert!(
        bind_address(&stopped).is_null(),
        "通常の Stop の処理(do_stop)が実行された: {:?}",
        stopped.metrics
    );
    let again = rpc(&cmd_ep, &Command::Stop).await;
    assert_stop_executed_in_error(&again, "decoder (2nd Stop)");

    let reset = rpc(&cmd_ep, &Command::Reset).await;
    assert!(reset.success, "{}", reset.message);
    assert_eq!(reset.state, ComponentState::Idle);

    shutdown_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("decoder did not stop within 5 s")
        .unwrap();
}

// ---------------------------------------------------------------------
// graw-writer: seq gap で Error → Stop でファイルを閉じる、Error のまま、Reset で Idle
// ---------------------------------------------------------------------

fn temp_root(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("tpcdaq-stop-in-error-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// offset 27 = asadIdx の最小 frameType 1 フレーム(graw_writer_integration.rs と同じ形)。
fn make_graw_frame(asad: u8, extra_len: usize, fill: u8) -> Vec<u8> {
    let mut b = vec![fill; 28 + extra_len];
    b[0] = 0x00;
    b[5] = 0x00;
    b[6] = 0x01; // frameType = 1
    b[27] = asad;
    b
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_graw_writer_in_error_executes_stop_closes_its_files_and_resets_to_idle() {
    const RUN: u32 = 32;
    let root = temp_root("graw");
    let params = GrawWriterParams {
        pull_bind: "tcp://127.0.0.1:0".to_string(),
        command_listen: "tcp://127.0.0.1:0".to_string(),
        output_root: root.clone(),
        max_file_bytes: 10 * 1024 * 1024,
        flush_interval_ms: 50,
        expected_sources: vec![0],
        finalize_delay_ms: 0,
    };
    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    let (ep_tx, ep_rx) = oneshot::channel();
    let task = tokio::spawn(run_graw_writer(params, shutdown_rx, Some(ep_tx)));
    let cmd_ep = ep_rx.await.unwrap();

    let pull_bind = configure_arm(&cmd_ep, RUN).await;
    let ctx = zmq::Context::new();
    let push = connect_push(&ctx, &pull_bind);
    let started = rpc(&cmd_ep, &Command::Start { run_number: RUN }).await;
    assert!(started.success, "{}", started.message);

    // AsAd 0 と 1 に非対称に書かせてから seq gap(seq=1 を飛ばす)で Error にする
    send_batch(&push, 0, RUN, 0, &[make_graw_frame(0, 4, 0x01)]);
    send_batch(
        &push,
        0,
        RUN,
        2,
        &[make_graw_frame(1, 9, 0x02), make_graw_frame(0, 2, 0x03)],
    );
    let errored = poll_status(&cmd_ep, Duration::from_secs(5), "Error with 2 files", |r| {
        r.state == ComponentState::Error && metric_u64(r, "files_open") == 2
    })
    .await;
    assert_eq!(metric_u64(&errored, "seq_gaps"), 1);

    let stopped = rpc(&cmd_ep, &Command::Stop).await;
    assert_stop_executed_in_error(&stopped, "graw-writer");
    assert!(
        bind_address(&stopped).is_null(),
        "通常の Stop の処理(do_stop)が実行された: {:?}",
        stopped.metrics
    );

    // Stop が書き込みスレッドを畳み、全ファイルが close される(Error のまま)
    let closed = poll_status(&cmd_ep, Duration::from_secs(5), "all files closed", |r| {
        let files = r.metrics.as_ref().unwrap()["files"].as_array().cloned();
        files.is_some_and(|f| f.len() == 2 && f.iter().all(|e| e["closed"] == true))
    })
    .await;
    assert_eq!(closed.state, ComponentState::Error);
    assert_eq!(metric_u64(&closed, "files_open"), 0);
    let again = rpc(&cmd_ep, &Command::Stop).await;
    assert_stop_executed_in_error(&again, "graw-writer (2nd Stop)");

    let reset = rpc(&cmd_ep, &Command::Reset).await;
    assert!(reset.success, "{}", reset.message);
    assert_eq!(reset.state, ComponentState::Idle);

    shutdown_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("graw-writer did not stop within 5 s")
        .unwrap();
    let _ = std::fs::remove_dir_all(&root);
}
