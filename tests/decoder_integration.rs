//! decoder コンポーネントの統合テスト(TODO/009 テスト (a)〜(f))。
//!
//! PUSH で直接 `Batch<RawFrames>` を投入し(receiver は経由しない)、テスト側の PULL で
//! `Batch<Fragments>` を受けて、decoder の実配線(PULL bind / PUSH connect)・単一ストリーム化
//! (source_id=100・自前 seq)・EOS 集約・入力検証・停止設計を検証する。
//!
//! ポートはすべて 0(動的)。decoder の PULL 実 bind アドレスは `Arm` 応答の metrics から取る
//! (graw_writer_integration.rs と同じ流儀)。条件待ちは固定 sleep ではなくポーリング + タイムアウト。

#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use serde_bytes::ByteBuf;
use tokio::sync::{broadcast, oneshot};
use tpcdaq::command::{Command, CommandResponse, ComponentState, RunConfig};
use tpcdaq::decoder::{run_decoder, DecoderParams};
use tpcdaq::msg::{Batch, Fragment, Fragments, Message, RawFrames};

/// decoder の source_id(SPEC §3.2)。
const DECODER_SOURCE_ID: u32 = 100;

// ---------------------------------------------------------------------
// 合成フレーム(frameType 1、blkSize=1、big-endian)
// ---------------------------------------------------------------------

const HEADER_BYTES: usize = 88;

fn write_uint(buf: &mut [u8], offset: usize, value: u64, n: usize) {
    for (i, b) in buf[offset..offset + n].iter_mut().enumerate() {
        *b = ((value >> (8 * (n - 1 - i))) & 0xFF) as u8;
    }
}

/// frameType 1 の CoBo フレームを手組みする。ヘッダの cobo/asad/event_idx を明示できるのは、
/// **下流が CoBo を Fragment.cobo で識別する**(SPEC §2.3)ことを検証するため。
fn make_frame(cobo: u8, asad: u8, event_idx: u32, item_count: usize) -> Vec<u8> {
    let total = HEADER_BYTES + item_count * 4;
    let mut b = vec![0u8; total];
    b[0] = 0x00; // metaType: blkSize = 2^0 = 1、bit7=0 = big-endian
    write_uint(&mut b, 1, total as u64, 3);
    write_uint(&mut b, 5, 1, 2); // frameType = 1
    write_uint(&mut b, 8, HEADER_BYTES as u64, 2);
    write_uint(&mut b, 10, 4, 2); // itemSize
    write_uint(&mut b, 12, item_count as u64, 4);
    write_uint(&mut b, 22, u64::from(event_idx), 4);
    b[26] = cobo;
    b[27] = asad;
    for i in 0..item_count {
        // 非対称な item(aget/chan/bucket/adc がすべて違う値になるように)
        let aget = (i % 4) as u32;
        let chan = (i % 68) as u32;
        let bucket = (i % 512) as u32;
        let adc = (100 + i * 7) as u32 % 4096;
        let w = (aget << 30) | (chan << 23) | (bucket << 14) | adc;
        write_uint(&mut b, HEADER_BYTES + i * 4, u64::from(w), 4);
    }
    b
}

/// frameType ∉ {1,2} の制御フレーム(実 2025 run 先頭の frameType 7・12 B が実例)。
fn control_frame() -> Vec<u8> {
    let mut b = vec![0u8; 12];
    write_uint(&mut b, 5, 7, 2);
    b
}

// ---------------------------------------------------------------------
// 配線ヘルパ
// ---------------------------------------------------------------------

fn test_params(push_connect: String, expected_sources: Vec<u32>) -> DecoderParams {
    DecoderParams {
        pull_bind: "tcp://127.0.0.1:0".to_string(),
        push_connect,
        command_listen: "tcp://127.0.0.1:0".to_string(),
        batch_max_bytes: 8 * 1024 * 1024,
        batch_max_ms: 10,
        heartbeat_ms: 60_000, // 既定では邪魔しない(Heartbeat を見るテストだけ短くする)
        send_timeout_ms: 200,
        workers: 1,
        expected_sources,
    }
}

async fn start_decoder(
    params: DecoderParams,
) -> (String, broadcast::Sender<()>, tokio::task::JoinHandle<()>) {
    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    let (ep_tx, ep_rx) = oneshot::channel();
    let handle = tokio::spawn(run_decoder(params, shutdown_rx, Some(ep_tx)));
    let endpoint = ep_rx
        .await
        .expect("decoder never reported its command endpoint");
    (endpoint, shutdown_tx, handle)
}

async fn shutdown_and_join(shutdown: broadcast::Sender<()>, task: tokio::task::JoinHandle<()>) {
    let _ = shutdown.send(());
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("decoder did not stop within 5 s")
        .unwrap();
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

async fn poll_until(
    endpoint: &str,
    cmd: &Command,
    timeout: Duration,
    mut pred: impl FnMut(&CommandResponse) -> bool,
) -> CommandResponse {
    let start = Instant::now();
    loop {
        let resp = rpc(endpoint, cmd).await;
        if pred(&resp) {
            return resp;
        }
        if start.elapsed() > timeout {
            panic!("condition not satisfied within {timeout:?}; last response = {resp:?}");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn metric_u64(resp: &CommandResponse, key: &str) -> u64 {
    resp.metrics.as_ref().unwrap()[key]
        .as_u64()
        .unwrap_or_else(|| panic!("metrics.{key} is not a number: {:?}", resp.metrics))
}

fn bind_address(resp: &CommandResponse) -> String {
    resp.metrics.as_ref().unwrap()["bind_address"]
        .as_str()
        .unwrap_or_else(|| panic!("Arm response has no bind_address: {:?}", resp.metrics))
        .to_string()
}

/// テスト側の PULL(= root-sink の代役)。decoder が PUSH connect してくる先。
fn bind_fragment_pull(ctx: &zmq::Context) -> (zmq::Socket, String) {
    let sock = ctx.socket(zmq::PULL).unwrap();
    sock.set_linger(0).unwrap();
    tpcdaq::zmq_helper::apply_pull_hwm(&sock).unwrap();
    sock.bind("tcp://127.0.0.1:0").unwrap();
    sock.set_rcvtimeo(5_000).unwrap();
    let endpoint = sock.get_last_endpoint().unwrap().unwrap();
    (sock, endpoint)
}

fn connect_push(ctx: &zmq::Context, endpoint: &str) -> zmq::Socket {
    let push = ctx.socket(zmq::PUSH).unwrap();
    push.set_linger(0).unwrap();
    tpcdaq::zmq_helper::apply_push_hwm(&push).unwrap();
    push.connect(endpoint).unwrap();
    push
}

fn send_batch(
    push: &zmq::Socket,
    source_id: u32,
    run_number: u32,
    sequence_number: u64,
    frames: &[Vec<u8>],
) {
    let payload: RawFrames = frames.iter().cloned().map(ByteBuf::from).collect();
    let msg: Message<RawFrames> = Message::Data(Batch {
        source_id,
        run_number,
        sequence_number,
        created_ns: 0,
        payload,
    });
    push.send(msg.to_msgpack().unwrap(), 0).unwrap();
}

fn send_eos(push: &zmq::Socket, source_id: u32, run_number: u32) {
    let msg: Message<RawFrames> = Message::EndOfStream {
        source_id,
        run_number,
    };
    push.send(msg.to_msgpack().unwrap(), 0).unwrap();
}

fn recv_message(pull: &zmq::Socket) -> Option<Message<Fragments>> {
    match pull.recv_bytes(0) {
        Ok(raw) => Some(Message::<Fragments>::from_msgpack(&raw).unwrap()),
        Err(zmq::Error::EAGAIN) => None,
        Err(e) => panic!("PULL recv failed: {e}"),
    }
}

/// EOS が来るまで受け取り、(Fragments バッチ列, EOS の本数) を返す。
/// EOS の後にもう 1 回だけ短い待ちを入れて「EOS はちょうど 1 本」を確かめる。
fn collect_until_eos(pull: &zmq::Socket) -> (Vec<Batch<Fragments>>, usize) {
    let mut batches = Vec::new();
    let mut eos = 0usize;
    while let Some(message) = recv_message(pull) {
        match message {
            Message::Data(batch) => batches.push(batch),
            Message::EndOfStream { source_id, .. } => {
                assert_eq!(source_id, DECODER_SOURCE_ID, "EOS は decoder 自身のもの");
                eos += 1;
                break;
            }
            Message::Heartbeat { .. } => {}
        }
    }
    // EOS の後に何も来ないこと(ちょうど 1 本)。
    pull.set_rcvtimeo(300).unwrap();
    while let Some(message) = recv_message(pull) {
        match message {
            Message::EndOfStream { .. } => eos += 1,
            Message::Data(batch) => panic!("EOS の後に Data が来た: {batch:?}"),
            Message::Heartbeat { .. } => {}
        }
    }
    (batches, eos)
}

fn all_fragments(batches: &[Batch<Fragments>]) -> Vec<Fragment> {
    batches.iter().flat_map(|b| b.payload.clone()).collect()
}

async fn configure_arm(endpoint: &str, run_number: u32) -> String {
    let cfg = rpc(
        endpoint,
        &Command::Configure(RunConfig {
            run_number,
            comment: "decoder integration test".to_string(),
            config: serde_json::Value::Null,
        }),
    )
    .await;
    assert!(cfg.success, "Configure failed: {}", cfg.message);
    let armed = rpc(endpoint, &Command::Arm).await;
    assert!(armed.success, "Arm failed: {}", armed.message);
    bind_address(&armed)
}

// ---------------------------------------------------------------------
// (a) + (b): 2 ソース混在 → 単一ストリーム化、全 EOS で自分の EOS がちょうど 1 本
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_sources_become_one_fragment_stream_and_exactly_one_eos_closes_it() {
    const RUN: u32 = 11;
    let ctx = zmq::Context::new();
    let (fragment_pull, fragment_ep) = bind_fragment_pull(&ctx);
    let (cmd_ep, shutdown, task) = start_decoder(test_params(fragment_ep, vec![0, 1])).await;

    let pull_bind = configure_arm(&cmd_ep, RUN).await;
    let push = connect_push(&ctx, &pull_bind);
    let started = rpc(&cmd_ep, &Command::Start { run_number: RUN }).await;
    assert!(started.success, "{}", started.message);

    // CoBo 0 は 3 フレーム(event 1,2,3)、CoBo 1 は 2 フレーム(event 1,2)— 非対称。
    send_batch(
        &push,
        0,
        RUN,
        0,
        &[make_frame(0, 0, 1, 4), make_frame(0, 0, 2, 4)],
    );
    send_batch(&push, 1, RUN, 0, &[make_frame(1, 1, 1, 8)]);
    send_batch(&push, 0, RUN, 1, &[make_frame(0, 0, 3, 4)]);
    send_batch(&push, 1, RUN, 1, &[make_frame(1, 1, 2, 8)]);
    send_eos(&push, 0, RUN);
    send_eos(&push, 1, RUN);

    let (batches, eos_count) = collect_until_eos(&fragment_pull);
    assert_eq!(eos_count, 1, "自分の EOS はちょうど 1 本(SPEC §2.3)");
    assert!(!batches.is_empty(), "Fragments が 1 通も来ていない");

    // 単一ストリーム: source_id=100、seq は 0 から連続、run_number は入力から。
    for (i, batch) in batches.iter().enumerate() {
        assert_eq!(batch.source_id, DECODER_SOURCE_ID);
        assert_eq!(batch.run_number, RUN);
        assert_eq!(batch.sequence_number, i as u64, "自前 seq は 0 から連続");
        assert!(batch.created_ns > 0, "created_ns が付いていない");
    }

    // CoBo の識別は Fragment.cobo が担う(下流は上流の数を知らなくてよい)。
    let fragments = all_fragments(&batches);
    assert_eq!(fragments.len(), 5, "入力フレーム数と一致");
    let cobo0: Vec<&Fragment> = fragments.iter().filter(|f| f.cobo == 0).collect();
    let cobo1: Vec<&Fragment> = fragments.iter().filter(|f| f.cobo == 1).collect();
    assert_eq!(cobo0.len(), 3);
    assert_eq!(cobo1.len(), 2);
    assert_eq!(
        cobo0.iter().map(|f| f.event_idx).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(
        cobo1.iter().map(|f| f.event_idx).collect::<Vec<_>>(),
        [1, 2]
    );
    assert!(cobo1.iter().all(|f| f.asad == 1), "asad も運ばれる");
    // 手計算: CoBo0 は 4 item×3 フレーム = 12、CoBo1 は 8 item×2 フレーム = 16 → 計 28 item
    let items: usize = fragments.iter().map(|f| f.items.len() / 4).sum();
    assert_eq!(items, 28);

    let status = rpc(&cmd_ep, &Command::GetStatus).await;
    assert_eq!(metric_u64(&status, "fragments_out"), 5);
    assert_eq!(metric_u64(&status, "items_out"), 28);
    assert_eq!(metric_u64(&status, "eos_in"), 2);
    assert_eq!(metric_u64(&status, "seq_gaps"), 0);
    assert_eq!(metric_u64(&status, "malformed"), 0);
    assert_eq!(metric_u64(&status, "unsupported"), 0);
    assert_ne!(status.state, ComponentState::Error);

    shutdown_and_join(shutdown, task).await;
}

// ---------------------------------------------------------------------
// (b) 片ソースだけ EOS を出しても自分の EOS は出ない
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_source_eos_alone_does_not_close_the_stream() {
    const RUN: u32 = 12;
    let ctx = zmq::Context::new();
    let (fragment_pull, fragment_ep) = bind_fragment_pull(&ctx);
    let (cmd_ep, shutdown, task) = start_decoder(test_params(fragment_ep, vec![0, 1])).await;

    let pull_bind = configure_arm(&cmd_ep, RUN).await;
    let push = connect_push(&ctx, &pull_bind);
    rpc(&cmd_ep, &Command::Start { run_number: RUN }).await;

    send_batch(&push, 0, RUN, 0, &[make_frame(0, 0, 1, 4)]);
    send_eos(&push, 0, RUN); // CoBo 1 はまだ

    // Fragments は来るが EOS は来ない。
    fragment_pull.set_rcvtimeo(1_000).unwrap();
    let mut data = 0;
    while let Some(message) = recv_message(&fragment_pull) {
        match message {
            Message::Data(_) => data += 1,
            Message::EndOfStream { .. } => {
                panic!("上流 1 ソースだけの EOS で自分の EOS を出してはいけない")
            }
            Message::Heartbeat { .. } => {}
        }
    }
    assert_eq!(data, 1, "Fragments 自体は流れている");

    let status = rpc(&cmd_ep, &Command::GetStatus).await;
    assert_eq!(metric_u64(&status, "eos_in"), 1);

    shutdown_and_join(shutdown, task).await;
}

// ---------------------------------------------------------------------
// (c) 片ソース seq ギャップ → Error ラッチ + カウント、消費は継続
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sequence_gap_on_one_source_latches_error_but_consumption_continues() {
    const RUN: u32 = 13;
    let ctx = zmq::Context::new();
    let (fragment_pull, fragment_ep) = bind_fragment_pull(&ctx);
    let (cmd_ep, shutdown, task) = start_decoder(test_params(fragment_ep, vec![0, 1])).await;

    let pull_bind = configure_arm(&cmd_ep, RUN).await;
    let push = connect_push(&ctx, &pull_bind);
    rpc(&cmd_ep, &Command::Start { run_number: RUN }).await;

    send_batch(&push, 0, RUN, 0, &[make_frame(0, 0, 1, 4)]);
    send_batch(&push, 0, RUN, 2, &[make_frame(0, 0, 2, 4)]); // seq=1 を飛ばす
    send_batch(&push, 1, RUN, 0, &[make_frame(1, 0, 1, 4)]); // 別ソースは正常

    let errored = poll_until(&cmd_ep, &Command::GetStatus, Duration::from_secs(5), |r| {
        r.state == ComponentState::Error
    })
    .await;
    assert_eq!(metric_u64(&errored, "seq_gaps"), 1);

    // 消費は継続: 3 フレームすべてが Fragment になって届く。
    let done = poll_until(&cmd_ep, &Command::GetStatus, Duration::from_secs(5), |r| {
        metric_u64(r, "fragments_out") == 3
    })
    .await;
    assert_eq!(metric_u64(&done, "frames_in"), 3);
    fragment_pull.set_rcvtimeo(1_000).unwrap();
    let mut fragments = 0;
    while let Some(Message::Data(batch)) = recv_message(&fragment_pull) {
        fragments += batch.payload.len();
    }
    assert_eq!(fragments, 3, "ギャップ後も Fragment は送出され続ける");

    shutdown_and_join(shutdown, task).await;
}

// ---------------------------------------------------------------------
// (d) unsupported フレーム混在 → カウントのみ・Error にならず・Fragment は出ない
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unsupported_frame_is_counted_but_never_becomes_a_fragment_nor_an_error() {
    const RUN: u32 = 14;
    let ctx = zmq::Context::new();
    let (fragment_pull, fragment_ep) = bind_fragment_pull(&ctx);
    let (cmd_ep, shutdown, task) = start_decoder(test_params(fragment_ep, vec![0])).await;

    let pull_bind = configure_arm(&cmd_ep, RUN).await;
    let push = connect_push(&ctx, &pull_bind);
    rpc(&cmd_ep, &Command::Start { run_number: RUN }).await;

    // 実 run 先頭と同じ並び: 制御フレーム(frameType 7)→ 通常フレーム。
    send_batch(
        &push,
        0,
        RUN,
        0,
        &[
            control_frame(),
            make_frame(0, 0, 1, 4),
            make_frame(0, 0, 2, 4),
        ],
    );
    send_eos(&push, 0, RUN);

    let (batches, eos_count) = collect_until_eos(&fragment_pull);
    assert_eq!(eos_count, 1);
    let fragments = all_fragments(&batches);
    assert_eq!(fragments.len(), 2, "制御フレームは Fragment 化されない");
    assert_eq!(
        fragments.iter().map(|f| f.event_idx).collect::<Vec<_>>(),
        [1, 2]
    );

    let status = rpc(&cmd_ep, &Command::GetStatus).await;
    assert_eq!(metric_u64(&status, "unsupported"), 1);
    assert_eq!(metric_u64(&status, "malformed"), 0);
    assert_eq!(metric_u64(&status, "frames_in"), 3);
    assert_eq!(metric_u64(&status, "fragments_out"), 2);
    assert_ne!(
        status.state,
        ComponentState::Error,
        "unsupported は Error にしない(SPEC v1.2 §7)"
    );

    shutdown_and_join(shutdown, task).await;
}

// ---------------------------------------------------------------------
// (e) Configure→Arm→Start→Stop 全シーケンス + idle Heartbeat
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn configure_arm_start_stop_sequence_succeeds_and_idle_sends_heartbeats() {
    const RUN: u32 = 15;
    let ctx = zmq::Context::new();
    let (fragment_pull, fragment_ep) = bind_fragment_pull(&ctx);
    let mut params = test_params(fragment_ep, vec![0]);
    params.heartbeat_ms = 100; // アイドル判定を待てる長さに縮める(既定 1 Hz)
    let (cmd_ep, shutdown, task) = start_decoder(params).await;

    let cfg = rpc(
        &cmd_ep,
        &Command::Configure(RunConfig {
            run_number: RUN,
            comment: String::new(),
            config: serde_json::Value::Null,
        }),
    )
    .await;
    assert!(cfg.success, "{}", cfg.message);
    assert_eq!(cfg.state, ComponentState::Configured);

    let armed = rpc(&cmd_ep, &Command::Arm).await;
    assert!(armed.success, "{}", armed.message);
    assert_eq!(armed.state, ComponentState::Armed);
    assert!(bind_address(&armed).starts_with("tcp://"));

    let started = rpc(&cmd_ep, &Command::Start { run_number: RUN }).await;
    assert!(started.success, "{}", started.message);
    assert_eq!(started.state, ComponentState::Running);

    // データが来ないアイドル状態では Heartbeat が届く(SPEC §2.2)。
    fragment_pull.set_rcvtimeo(3_000).unwrap();
    let message = recv_message(&fragment_pull).expect("idle Heartbeat が届かない");
    match message {
        Message::Heartbeat {
            source_id,
            run_number,
            counter,
        } => {
            assert_eq!(source_id, DECODER_SOURCE_ID);
            assert_eq!(run_number, RUN);
            assert_eq!(counter, 0, "最初の Heartbeat は counter=0");
        }
        other => panic!("Heartbeat のはずが {other:?}"),
    }

    // 送出成功も数える(TODO/023-5 = R-P2-12。以前は `let _ =` で結果を捨てていた)。
    let status = poll_until(&cmd_ep, &Command::GetStatus, Duration::from_secs(5), |r| {
        metric_u64(r, "heartbeats_out") >= 1
    })
    .await;
    assert_eq!(
        metric_u64(&status, "heartbeats_abandoned"),
        0,
        "受け手が居るのに打ち切られてはいない"
    );

    let stopped = rpc(&cmd_ep, &Command::Stop).await;
    assert!(stopped.success, "{}", stopped.message);
    assert_eq!(stopped.state, ComponentState::Configured);

    shutdown_and_join(shutdown, task).await;
}

// ---------------------------------------------------------------------
// (f) 下流が詰まった状態で Reset → 破棄がカウントされ、プロセスは畳める
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reset_abandons_the_blocked_send_and_counts_what_was_dropped() {
    const RUN: u32 = 16;
    // 誰も listen していないポート(= PUSH に peer が無く send は必ずブロックする)。
    let dead = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        format!("tcp://{addr}")
    };
    let mut params = test_params(dead, vec![0]);
    // 時間による close が EOS 処理より先に走らないよう十分長くする(送出ブロックの契機を
    // 「全 EOS 受領 → flush + 自分の EOS」に固定して競合をなくす)。
    params.batch_max_ms = 60_000;
    let (cmd_ep, shutdown, task) = start_decoder(params).await;

    let pull_bind = configure_arm(&cmd_ep, RUN).await;
    let ctx = zmq::Context::new();
    let push = connect_push(&ctx, &pull_bind);
    rpc(&cmd_ep, &Command::Start { run_number: RUN }).await;

    send_batch(&push, 0, RUN, 0, &[make_frame(0, 0, 1, 4)]);
    send_eos(&push, 0, RUN);

    // 下流が詰まっているので送出はブロックする(ロスレス = Reset 以外では諦めない)。
    // その最中でも GetStatus は答えられること(送信はコアのロックを持たない)を確かめる。
    tokio::time::sleep(Duration::from_millis(300)).await;
    let blocked = rpc(&cmd_ep, &Command::GetStatus).await;
    assert_eq!(metric_u64(&blocked, "batches_out"), 0, "まだ送れていない");
    assert_eq!(
        metric_u64(&blocked, "batches_abandoned"),
        0,
        "まだ諦めていない"
    );

    let reset = rpc(&cmd_ep, &Command::Reset).await;
    assert!(reset.success, "{}", reset.message);
    assert_eq!(reset.state, ComponentState::Idle);

    // Reset 中の打ち切りは必ずカウント + 可視化される(破棄が見えることが許可の条件)。
    let done = poll_until(&cmd_ep, &Command::GetStatus, Duration::from_secs(5), |r| {
        metric_u64(r, "eos_abandoned") == 1
    })
    .await;
    assert_eq!(metric_u64(&done, "batches_abandoned"), 1);
    assert_eq!(metric_u64(&done, "batches_out"), 0);
    assert_eq!(metric_u64(&done, "fragments_out"), 0);

    // プロセスは畳める(スレッドが送出待ちから抜けている)。
    shutdown_and_join(shutdown, task).await;
}

// ---------------------------------------------------------------------
// (g) TODO/013: 追いつけない decoder が上流 2 ソースの片方を飢餓させない
// ---------------------------------------------------------------------

/// 出力 Fragment 列で「同じ CoBo が最大何通続いたか」。飢餓の直接の尺度。
fn longest_same_cobo_run(fragments: &[Fragment]) -> usize {
    let mut longest = 0usize;
    let mut run = 0usize;
    let mut prev = u8::MAX;
    for f in fragments {
        run = if f.cobo == prev { run + 1 } else { 1 };
        prev = f.cobo;
        longest = longest.max(run);
    }
    longest
}

/// 飢餓が起きない上限。修正後の実測は 2(= 相手のパイプが 1 通分だけ遅れて戻る過渡)なので、
/// スケジューラの揺れを見込んでも 8 で十分に厳しい。壊れると 60(= 片ソースを全部食う)になる。
const MAX_FAIR_RUN: usize = 8;

/// **TODO/013 の回帰**: decoder が入力に追いつけない状態でも、上流 2 ソースの片方を
/// run 丸ごと飢餓させてはならない(012 で発見。ELITPC では全イベント incomplete 化を招く)。
///
/// # 機構(013 で計測確定)
///
/// PULL の fair-queue は「自分の番で空だったパイプ」を**非活性化**し、その再活性化は libzmq の
/// **コマンド**として届く。コマンドは `process_commands()` でしか取り込まれず、`zmq_recv` は
/// 「成功 100 回に 1 度」か「EAGAIN を返したとき」しかそれを呼ばない(libzmq の
/// `inbound_poll_rate = 100`)。消費が供給より遅いと `recv` は EAGAIN を返さないので、
/// 一度外れたパイプは **100 通ぶん**戻ってこられない。修正は「recv の前に必ず poll する」
/// (`zmq_poll` は各ソケットの `ZMQ_EVENTS` を読む = そこで `process_commands()` が走る)。
///
/// # このテストが遅さを作る方法(production に細工用フックを足さない)
///
/// 1 フレーム 34,816 item = 139,352 B の**本物のデコード**を 2 ソースから 1 ms 間隔で浴びせる。
/// dev ビルドの `Decoder::decode` は実測 3.1 ms/frame なので、これだけで供給 > 消費になる
/// (= 修正前は必ず飢餓する条件)。release は 0.026 ms/frame と速いので飢餓条件そのものが
/// 成立せず、このテストは「速いときも公平」を確かめるだけになる(vacuous ではないが弱い)。
/// **profile に依存しない機構そのものの固定は [`the_fair_queue_locks_in_unless_commands_are_processed_between_receives`]。**
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_decoder_that_cannot_keep_up_still_never_starves_one_of_two_sources() {
    const RUN: u32 = 17;
    const BATCHES_PER_SOURCE: usize = 60;
    const ITEMS_PER_FRAME: usize = 34_816; // 実 .graw の 1/4 フレーム相当(139,352 B)
    const SEND_INTERVAL: Duration = Duration::from_millis(1);

    let ctx = zmq::Context::new();
    let (fragment_pull, fragment_ep) = bind_fragment_pull(&ctx);
    let (cmd_ep, shutdown, task) = start_decoder(test_params(fragment_ep, vec![0, 1])).await;

    let pull_bind = configure_arm(&cmd_ep, RUN).await;
    let started = rpc(&cmd_ep, &Command::Start { run_number: RUN }).await;
    assert!(started.success, "{}", started.message);

    // **ソース毎に独立した PUSH ソケット** — 1 本の PUSH を使い回すと decoder 側のパイプが
    // 1 本しかできず、fair-queue そのものが存在しない(= 何も試験していない)ことになる。
    // 送り終えた PUSH を**そのままスレッドの戻り値にして生かしておく**。`connect_push` は
    // linger(0) なので、ここで socket を落とすと未送出ぶんが黙って捨てられる。
    let producers: Vec<_> = (0u32..2)
        .map(|source_id| {
            let push = connect_push(&ctx, &pull_bind);
            std::thread::spawn(move || {
                let frame = make_frame(source_id as u8, 0, 1, ITEMS_PER_FRAME);
                let start = Instant::now();
                for seq in 0..BATCHES_PER_SOURCE {
                    let due = start + SEND_INTERVAL * seq as u32;
                    let now = Instant::now();
                    if due > now {
                        std::thread::sleep(due - now);
                    }
                    send_batch(
                        &push,
                        source_id,
                        RUN,
                        seq as u64,
                        std::slice::from_ref(&frame),
                    );
                }
                send_eos(&push, source_id, RUN);
                push
            })
        })
        .collect();

    let (batches, eos_count) = collect_until_eos(&fragment_pull);
    for p in producers {
        p.join().expect("producer thread panicked");
    }
    assert_eq!(eos_count, 1, "自分の EOS はちょうど 1 本");

    let fragments = all_fragments(&batches);
    let from_0 = fragments.iter().filter(|f| f.cobo == 0).count();
    let from_1 = fragments.iter().filter(|f| f.cobo == 1).count();
    assert_eq!(
        (from_0, from_1),
        (BATCHES_PER_SOURCE, BATCHES_PER_SOURCE),
        "ロスレス: 両ソースとも 1 フレームも欠けない"
    );

    // ここが 013 の本体。壊れていると片方が run 丸ごと後回しになり run 長 ≈ 60 になる。
    let longest = longest_same_cobo_run(&fragments);
    assert!(
        longest <= MAX_FAIR_RUN,
        "片ソースが {longest} 通連続で消費された(上限 {MAX_FAIR_RUN})— \
         decoder の PULL が fair-queue にロックインしている(TODO/013)"
    );

    let status = rpc(&cmd_ep, &Command::GetStatus).await;
    assert_eq!(
        metric_u64(&status, "cobo_mismatch"),
        0,
        "cobo は source と一致"
    );
    shutdown_and_join(shutdown, task).await;
}

/// **013 で確定した libzmq の機構そのものを固定する**(production コードは呼ばない —
/// `zmq_helper` の「IMMEDIATE なし PUSH は送れたふりをする」対比テストと同じ流儀)。
///
/// # 何を測るか
///
/// 「**一度 fair-queue から外れた上流が、戻ってくるまでに相手を何通消費させるか**」。
/// 状況は速度勝負ではなく**決定的に**組み立てるので、dev / release でも並列実行の負荷でも
/// 同じ数字が出る:
///
/// 1. 両ソースを 1 通ずつ流して EAGAIN まで読み切る → 2 本のパイプは attach 済みで、
///    どちらも「自分の番で空だった」ので **非活性化**されている。
/// 2. ソース 0 だけが大量に送る(消費は止めたまま)→ 0 のパイプは常にデータがある状態になる。
/// 3. そのあとソース 1 が **1 通だけ**送る → 再活性化コマンドがメールボックスに積まれる。
/// 4. 消費を再開し、ソース 1 の 1 通が出てくるまでにソース 0 を何通食ったかを数える。
///
/// `recv` だけのループでは `process_commands()` が「成功 100 回に 1 度」しか走らない
/// (libzmq の `inbound_poll_rate`)ので、ソース 0 に常にデータがある間はコマンドが取り込まれず、
/// ソース 1 は **100 通ぶん**待たされる。`recv` の前に `poll` を挟むと(`zmq_poll` は
/// `ZMQ_EVENTS` を読む = そこで `process_commands()` が走る)即座に戻る。
///
/// これが壊れる(= `recv` だけでも即座に戻る)なら libzmq の実装が変わったということなので、
/// そのときは `src/decoder.rs` の poll の理由書きを見直すこと。
#[test]
fn a_pipe_that_left_the_fair_queue_only_returns_when_commands_are_processed() {
    /// ソース 0 が積んでおく通数。libzmq の `inbound_poll_rate`(100)より十分多くする。
    const FLOOD: usize = 300;
    /// 配送待ち(ZMQ の IO スレッドがパイプへ書き終えるまで)。
    const SETTLE: Duration = Duration::from_millis(300);

    /// ソース 1 の 1 通が出てくるまでに消費したソース 0 の通数を返す。
    /// `poll_first` = 修正後のループ構造、`false` = 修正前(recv だけ)。
    fn distance_until_the_quiet_source_is_heard(poll_first: bool) -> usize {
        let ctx = zmq::Context::new();
        let pull = ctx.socket(zmq::PULL).unwrap();
        pull.set_linger(0).unwrap();
        tpcdaq::zmq_helper::apply_pull_hwm(&pull).unwrap();
        pull.bind("tcp://127.0.0.1:0").unwrap();
        pull.set_rcvtimeo(200).unwrap();
        let endpoint = pull.get_last_endpoint().unwrap().unwrap();
        let loud = connect_push(&ctx, &endpoint);
        let quiet = connect_push(&ctx, &endpoint);

        let recv_one = |poll_first: bool| -> Option<u8> {
            if poll_first {
                let mut items = [pull.as_poll_item(zmq::POLLIN)];
                zmq::poll(&mut items, 200).unwrap();
                if !items[0].is_readable() {
                    return None;
                }
            }
            match pull.recv_bytes(if poll_first { zmq::DONTWAIT } else { 0 }) {
                Ok(raw) => Some(raw[0]),
                Err(zmq::Error::EAGAIN) => None,
                Err(e) => panic!("PULL recv failed: {e}"),
            }
        };

        // 1. 両パイプを attach させ、EAGAIN まで読み切って両方とも非活性化させる。
        loud.send(&[0u8][..], 0).unwrap();
        quiet.send(&[1u8][..], 0).unwrap();
        std::thread::sleep(SETTLE);
        let mut warmed = 0;
        while warmed < 2 {
            if recv_one(poll_first).is_some() {
                warmed += 1;
            }
        }
        while recv_one(poll_first).is_some() {} // EAGAIN まで(= 両パイプが非活性化)

        // 2. ソース 0 だけが積む(消費は止めたまま)。
        for _ in 0..FLOOD {
            loud.send(&[0u8][..], 0).unwrap();
        }
        std::thread::sleep(SETTLE);

        // 3. 消費を再開してソース 0 のパイプ**だけ**を活性化させる。
        //    ここでソース 1 が先に送っていると、再活性化コマンドが 2 本まとめて
        //    取り込まれてしまい「片方だけ外れている」状況が作れない。
        for _ in 0..5 {
            assert_eq!(recv_one(poll_first), Some(0), "ソース 0 が読めない");
        }

        // 4. **そのあと**ソース 1 が 1 通だけ送る(= 再活性化コマンドが積まれるが、
        //    ソース 0 に常にデータがある限り recv はそれを取り込まない)。
        quiet.send(&[1u8][..], 0).unwrap();
        std::thread::sleep(SETTLE);

        // 5. ソース 1 が出てくるまでにソース 0 を何通食ったかを数える。
        let mut from_loud = 0usize;
        for _ in 0..FLOOD {
            match recv_one(poll_first) {
                Some(1) => return from_loud,
                Some(_) => from_loud += 1,
                None => panic!("消費が途切れた(ソース 0 の積み込みが足りない)"),
            }
        }
        panic!("ソース 1 の 1 通が {FLOOD} 通の間ずっと出てこなかった");
    }

    let starving = distance_until_the_quiet_source_is_heard(false);
    let fair = distance_until_the_quiet_source_is_heard(true);
    eprintln!(
        "外れたパイプが戻るまでに相手を食う通数: recv のみ = {starving} / poll を挟む = {fair}"
    );
    assert!(
        starving >= 50,
        "recv だけのループで飢餓が再現しなかった(相手を {starving} 通で聞けた)— libzmq の \
         inbound_poll_rate まわりの挙動が変わった可能性がある。src/decoder.rs の poll の\
         理由書きを見直すこと"
    );
    assert!(
        fair <= MAX_FAIR_RUN,
        "poll を挟んでも公平にならなかった(相手を聞くまでに {fair} 通)"
    );
}
