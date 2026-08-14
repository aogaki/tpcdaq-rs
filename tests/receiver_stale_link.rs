//! receiver の**単一データリンク規約**(SPEC §1.4-6 / 受け入れ §12-13、TODO/032)の統合テスト。
//!
//! 実機のデータリンクは CoBo が `configure` 時に張る 1 本だけで、ECC は probe を張らない。
//! それでも「CoBo の無 FIN 消滅 → 復旧後の再 configure」や迷い込み接続で 2 本目が来ることは
//! あり、旧実装ではその 1 本が backlog に滞留して **run が丸ごと無言で空振り**した。
//! ここで固定するのは次の 3 点(GET 純正 DataRouter の ECONNREFUSED と同型の fail-fast):
//!
//! 1. 現接続は 1 バイトも影響を受けない(バイト一致 + フレーム境界一致)。
//! 2. 余分な接続は **即 close** され、`extra_connections` として数えられる(silent stall なし)。
//! 3. **1 バイトも運ばなかった接続の終了は run 境界ではない** — 偽 EOS で run を閉じない。
//!
//! ポートはすべて 0(動的)。条件待ちは固定 sleep ではなくポーリング + タイムアウト。

#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, oneshot};
use tpcdaq::command::{Command, CommandResponse, ComponentState, RunConfig};
use tpcdaq::msg::{Message, RawFrames};
use tpcdaq::receiver::{run_receiver, ReceiverParams};
use tpcdaq::zmq_helper;

// ---------------------------------------------------------------------
// 配線ヘルパ(receiver_integration.rs と同じ流儀)
// ---------------------------------------------------------------------

fn bind_pull(ctx: &zmq::Context) -> (zmq::Socket, String) {
    let sock = ctx.socket(zmq::PULL).unwrap();
    sock.set_linger(0).unwrap();
    zmq_helper::apply_pull_hwm(&sock).unwrap();
    sock.bind("tcp://127.0.0.1:0").unwrap();
    sock.set_rcvtimeo(10_000).unwrap(); // ハングではなく失敗で終わらせる
    let endpoint = sock.get_last_endpoint().unwrap().unwrap();
    (sock, endpoint)
}

/// テスト用パラメタ。**Heartbeat は事実上止める**(60 s): このファイルは
/// 「下流に何も出ていないこと」を何度も主張するので、アイドル通知が混ざると読めなくなる。
fn test_params(cobo_id: u32, graw: &str, decoder: &str) -> ReceiverParams {
    ReceiverParams {
        cobo_id,
        listen: "127.0.0.1:0".to_string(),
        command_listen: "tcp://127.0.0.1:0".to_string(),
        graw_writer_endpoint: graw.to_string(),
        decoder_endpoint: decoder.to_string(),
        batch_max_bytes: 64 * 1024,
        batch_max_ms: 10,
        queue_frames: 256,
        heartbeat_ms: 60_000,
        hwm: zmq_helper::DEFAULT_HWM,
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

fn metric(resp: &CommandResponse, key: &str) -> serde_json::Value {
    resp.metrics.as_ref().unwrap()[key].clone()
}

fn bind_address(resp: &CommandResponse) -> String {
    resp.metrics.as_ref().unwrap()["bind_address"]
        .as_str()
        .unwrap_or_else(|| panic!("Arm response has no bind_address: {:?}", resp.metrics))
        .to_string()
}

/// Configure → Arm → Start を通し、`(コマンド endpoint, データ listen アドレス, …)` を返す。
async fn armed_and_started(
    cobo_id: u32,
    run_number: u32,
    graw: &str,
    decoder: &str,
) -> (
    String,
    String,
    broadcast::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let (cmd_ep, shutdown_tx, task) = start_receiver(test_params(cobo_id, graw, decoder)).await;
    let configured = rpc(
        &cmd_ep,
        &Command::Configure(RunConfig {
            run_number,
            comment: "single data link (SPEC §1.4-6)".to_string(),
            config: serde_json::Value::Null,
        }),
    )
    .await;
    assert!(configured.success, "{}", configured.message);
    let armed = rpc(&cmd_ep, &Command::Arm).await;
    assert!(armed.success, "{}", armed.message);
    let data_addr = bind_address(&armed);
    let started = rpc(&cmd_ep, &Command::Start { run_number }).await;
    assert!(started.success, "{}", started.message);
    assert_eq!(started.state, ComponentState::Running);
    (cmd_ep, data_addr, shutdown_tx, task)
}

/// GetStatus を叩き続け、`want` が満たされた応答を返す(満たされなければ panic)。
async fn poll_status(
    cmd_ep: &str,
    timeout: Duration,
    what: &str,
    want: impl Fn(&CommandResponse) -> bool,
) -> CommandResponse {
    let deadline = Instant::now() + timeout;
    let mut last = rpc(cmd_ep, &Command::GetStatus).await;
    loop {
        if want(&last) {
            return last;
        }
        if Instant::now() >= deadline {
            panic!(
                "{what} が {timeout:?} 以内に成立しなかった: metrics={:?}",
                last.metrics
            );
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        last = rpc(cmd_ep, &Command::GetStatus).await;
    }
}

async fn shutdown(shutdown_tx: broadcast::Sender<()>, task: tokio::task::JoinHandle<()>) {
    shutdown_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("receiver did not stop within 5 s")
        .unwrap();
}

// ---------------------------------------------------------------------
// 合成 MFM フレーム(receiver_integration.rs と同じ作り方)
// ---------------------------------------------------------------------

fn make_frame(total_bytes: usize, little: bool, fill: u8) -> Vec<u8> {
    assert!((8..=0xFF_FFFF).contains(&total_bytes));
    let mut f = vec![fill; total_bytes];
    f[0] = if little { 0x80 } else { 0x00 };
    let blk = total_bytes as u32;
    if little {
        f[1] = (blk & 0xFF) as u8;
        f[2] = ((blk >> 8) & 0xFF) as u8;
        f[3] = ((blk >> 16) & 0xFF) as u8;
    } else {
        f[1] = ((blk >> 16) & 0xFF) as u8;
        f[2] = ((blk >> 8) & 0xFF) as u8;
        f[3] = (blk & 0xFF) as u8;
    }
    f
}

/// 長さもエンディアンも中身も非対称な 4 フレーム(取り違えが目に見えるように)。
fn sample_frames() -> Vec<Vec<u8>> {
    vec![
        make_frame(100_000, true, 0x44),
        make_frame(24, true, 0x11),
        make_frame(4096, false, 0x22),
        make_frame(12, true, 0x33),
    ]
}

// ---------------------------------------------------------------------
// PULL 側の収集
// ---------------------------------------------------------------------

#[derive(Debug, Default)]
struct Collected {
    frames: Vec<Vec<u8>>,
    eos_run_number: Option<u32>,
}

fn collect_until_eos(sock: &zmq::Socket, link: &str) -> Collected {
    let mut c = Collected::default();
    loop {
        let bytes = sock
            .recv_bytes(0)
            .unwrap_or_else(|e| panic!("{link}: PULL recv failed/timed out: {e}"));
        match Message::<RawFrames>::from_msgpack(&bytes).unwrap() {
            Message::Data(batch) => {
                for frame in batch.payload {
                    c.frames.push(frame.into_vec());
                }
            }
            Message::EndOfStream { run_number, .. } => {
                c.eos_run_number = Some(run_number);
                return c;
            }
            Message::Heartbeat { .. } => panic!("{link}: Heartbeat は止めてあるはず"),
        }
    }
}

/// 下流に**まだ何も届いていない**ことを主張する(ノンブロッキング read = EAGAIN)。
fn assert_nothing_downstream(sock: &zmq::Socket, link: &str, what: &str) {
    match sock.recv_bytes(zmq::DONTWAIT) {
        Err(zmq::Error::EAGAIN) => {}
        Ok(bytes) => {
            let msg = Message::<RawFrames>::from_msgpack(&bytes).unwrap();
            panic!("{link}: {what} — なのに下流へ出ていた: {msg:?}");
        }
        Err(e) => panic!("{link}: PULL recv が想定外の失敗: {e}"),
    }
}

fn unix_nanos_now() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    )
    .unwrap()
}

// ---------------------------------------------------------------------
// T1 — 本丸: 余分な接続は即 close され、現接続は 1 バイトも影響を受けない
// ---------------------------------------------------------------------

/// SPEC §12-13 の受け入れそのもの。
/// 接続 A で流通中に接続 B を張る → B は即 close(B 側 read が EOF)+ `extra_connections` = 1 +
/// A のフレーム列はバイト一致で無影響 + EOS は流通中に出ていない(EOF まで 1 回も出ない)。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_extra_connection_is_closed_at_once_and_the_live_link_is_untouched() {
    let ctx = zmq::Context::new();
    let (graw_pull, graw_ep) = bind_pull(&ctx);
    let (decoder_pull, decoder_ep) = bind_pull(&ctx);
    let (cmd_ep, data_addr, shutdown_tx, task) =
        armed_and_started(3, 77, &graw_ep, &decoder_ep).await;

    let frames = sample_frames();

    // --- 接続 A: 先勝ちのデータリンク。1 本目のフレームを流して「流通中」にする
    let mut link_a = TcpStream::connect(&data_addr).await.unwrap();
    link_a.write_all(&frames[0]).await.unwrap();
    poll_status(
        &cmd_ep,
        Duration::from_secs(5),
        "A の 1 フレーム目が読まれる",
        |r| metric_u64(r, "frames") >= 1,
    )
    .await;

    // --- 接続 B: 余分な接続。accept された上で**即 close**されること(= B 側で EOF)
    let mut link_b = TcpStream::connect(&data_addr).await.unwrap();
    let mut sink = [0u8; 16];
    let n = tokio::time::timeout(Duration::from_secs(5), link_b.read(&mut sink))
        .await
        .expect("余分な接続が閉じられない = backlog に滞留している(silent stall)")
        .expect("余分な接続の read が失敗");
    assert_eq!(n, 0, "余分な接続には 1 バイトも書かず、即 close するだけ");

    let status = poll_status(
        &cmd_ep,
        Duration::from_secs(5),
        "extra_connections = 1",
        |r| metric_u64(r, "extra_connections") == 1,
    )
    .await;
    assert_eq!(
        metric_u64(&status, "empty_connections"),
        0,
        "余分な接続は empty_connections ではない(カウンタを取り違えない)"
    );

    // --- A は何事もなかったように続きを流し、EOF で run を閉じる
    for frame in &frames[1..] {
        link_a.write_all(frame).await.unwrap();
    }
    link_a.shutdown().await.unwrap();

    let expected_bytes: Vec<u8> = frames.concat();
    for (sock, link) in [(&graw_pull, "graw-writer"), (&decoder_pull, "decoder")] {
        let got = collect_until_eos(sock, link);
        // 「EOS は流通中に出ていない」= EOS までに 4 フレーム全部が揃っていること
        assert_eq!(got.frames, frames, "{link}: フレーム境界が入力と違う");
        assert_eq!(
            got.frames.concat(),
            expected_bytes,
            "{link}: バイト列が一致しない"
        );
        assert_eq!(got.eos_run_number, Some(77), "{link}: EOF → EOS");
    }

    let stopped = rpc(&cmd_ep, &Command::Stop).await;
    assert!(stopped.success, "{}", stopped.message);
    assert_eq!(metric_u64(&stopped, "frames"), frames.len() as u64);
    assert_eq!(metric_u64(&stopped, "bytes"), expected_bytes.len() as u64);
    assert_eq!(metric_u64(&stopped, "extra_connections"), 1);
    assert_eq!(metric_u64(&stopped, "overflow_frames"), 0);
    // A の EOF で EOS は出し切っている → Stop で 2 通目は出ない
    for (sock, link) in [(&graw_pull, "graw-writer"), (&decoder_pull, "decoder")] {
        assert_nothing_downstream(sock, link, "EOS は EOF で 1 回出したきり");
    }

    shutdown(shutdown_tx, task).await;
}

// ---------------------------------------------------------------------
// T2 — 余分な接続は毎回数える
// ---------------------------------------------------------------------

/// 3 本の余分な接続はすべて拒否され、すべて数えられる(warn が 1 回きりなのは
/// `src/receiver.rs` の単体テストが `record_extra_connection` の戻り値で固定する)。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_extra_connection_is_counted() {
    let ctx = zmq::Context::new();
    let (graw_pull, graw_ep) = bind_pull(&ctx);
    let (decoder_pull, decoder_ep) = bind_pull(&ctx);
    let (cmd_ep, data_addr, shutdown_tx, task) =
        armed_and_started(0, 5, &graw_ep, &decoder_ep).await;

    let frame = make_frame(512, true, 0xA5);
    let mut link_a = TcpStream::connect(&data_addr).await.unwrap();
    link_a.write_all(&frame).await.unwrap();
    poll_status(
        &cmd_ep,
        Duration::from_secs(5),
        "A の 1 フレーム目",
        |r| metric_u64(r, "frames") >= 1,
    )
    .await;

    for i in 0..3 {
        let mut extra = TcpStream::connect(&data_addr).await.unwrap();
        let mut sink = [0u8; 4];
        let n = tokio::time::timeout(Duration::from_secs(5), extra.read(&mut sink))
            .await
            .unwrap_or_else(|_| panic!("{i} 本目の余分な接続が閉じられない"))
            .unwrap();
        assert_eq!(n, 0, "{i} 本目: close されるだけ");
    }

    let status = poll_status(
        &cmd_ep,
        Duration::from_secs(5),
        "extra_connections = 3",
        |r| metric_u64(r, "extra_connections") == 3,
    )
    .await;
    assert_eq!(metric_u64(&status, "empty_connections"), 0);
    assert_eq!(metric_u64(&status, "frames"), 1, "A は無影響");

    link_a.shutdown().await.unwrap();
    for (sock, link) in [(&graw_pull, "graw-writer"), (&decoder_pull, "decoder")] {
        let got = collect_until_eos(sock, link);
        assert_eq!(got.frames, vec![frame.clone()], "{link}");
        assert_eq!(got.eos_run_number, Some(5), "{link}");
    }

    shutdown(shutdown_tx, task).await;
}

// ---------------------------------------------------------------------
// T3 — 0 バイト接続は run 境界ではない
// ---------------------------------------------------------------------

/// 迷い込み接続(connect → 即 close)は `empty_connections` として数えるだけで、
/// **EOS を出さない**(偽 run 境界を作らない)。その後に本物が来て EOF したとき、
/// EOS はちょうど 1 回。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_connection_that_carried_no_byte_does_not_close_the_run() {
    let ctx = zmq::Context::new();
    let (graw_pull, graw_ep) = bind_pull(&ctx);
    let (decoder_pull, decoder_ep) = bind_pull(&ctx);
    let (cmd_ep, data_addr, shutdown_tx, task) =
        armed_and_started(1, 12, &graw_ep, &decoder_ep).await;

    // --- 迷い込み: 繋いで 1 バイトも送らずに閉じる
    let stray = TcpStream::connect(&data_addr).await.unwrap();
    drop(stray);

    let status = poll_status(
        &cmd_ep,
        Duration::from_secs(5),
        "empty_connections = 1",
        |r| metric_u64(r, "empty_connections") == 1,
    )
    .await;
    assert_eq!(
        metric_u64(&status, "extra_connections"),
        0,
        "先勝ちの 1 本目は余分ではない"
    );
    assert_eq!(metric_u64(&status, "bytes"), 0);
    for (sock, link) in [(&graw_pull, "graw-writer"), (&decoder_pull, "decoder")] {
        assert_nothing_downstream(sock, link, "0 バイト接続の終了は run 境界ではない");
    }

    // --- 本物: データを流して EOF。EOS はここで初めて、ちょうど 1 回
    let frames = vec![make_frame(64, true, 0x5A), make_frame(300, false, 0xC3)];
    let mut link_a = TcpStream::connect(&data_addr).await.unwrap();
    for frame in &frames {
        link_a.write_all(frame).await.unwrap();
    }
    link_a.shutdown().await.unwrap();

    for (sock, link) in [(&graw_pull, "graw-writer"), (&decoder_pull, "decoder")] {
        let got = collect_until_eos(sock, link);
        assert_eq!(got.frames, frames, "{link}");
        assert_eq!(got.eos_run_number, Some(12), "{link}");
    }

    // Stop しても 2 通目の EOS は出ない(EOF で既に出し切っている)
    let stopped = rpc(&cmd_ep, &Command::Stop).await;
    assert!(stopped.success, "{}", stopped.message);
    assert_eq!(metric_u64(&stopped, "empty_connections"), 1);
    assert_eq!(metric_u64(&stopped, "frames"), 2);
    tokio::time::sleep(Duration::from_millis(200)).await;
    for (sock, link) in [(&graw_pull, "graw-writer"), (&decoder_pull, "decoder")] {
        assert_nothing_downstream(sock, link, "EOS はちょうど 1 回");
    }

    shutdown(shutdown_tx, task).await;
}

// ---------------------------------------------------------------------
// T4 — 後方互換(既存の EOS 意味論を壊していない)
// ---------------------------------------------------------------------

/// データを運んだ接続の EOF は、これまでどおり run 境界(EOS)。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn eof_after_data_still_ends_the_run() {
    let ctx = zmq::Context::new();
    let (graw_pull, graw_ep) = bind_pull(&ctx);
    let (decoder_pull, decoder_ep) = bind_pull(&ctx);
    let (cmd_ep, data_addr, shutdown_tx, task) =
        armed_and_started(2, 31, &graw_ep, &decoder_ep).await;

    let frames = vec![make_frame(128, true, 0x77)];
    let mut link = TcpStream::connect(&data_addr).await.unwrap();
    link.write_all(&frames[0]).await.unwrap();
    link.shutdown().await.unwrap();

    for (sock, link_name) in [(&graw_pull, "graw-writer"), (&decoder_pull, "decoder")] {
        let got = collect_until_eos(sock, link_name);
        assert_eq!(got.frames, frames, "{link_name}");
        assert_eq!(got.eos_run_number, Some(31), "{link_name}");
    }
    let stopped = rpc(&cmd_ep, &Command::Stop).await;
    assert_eq!(metric_u64(&stopped, "empty_connections"), 0);
    assert_eq!(metric_u64(&stopped, "extra_connections"), 0);

    shutdown(shutdown_tx, task).await;
}

/// 誰も繋いでこなかった run の `Stop` は、これまでどおり**強制 EOS を 1 回**出す
/// (SPEC §1.3 の正規経路 — ここは今回の変更で一切触らない)。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stop_still_forces_exactly_one_end_of_stream_when_nobody_connected() {
    let ctx = zmq::Context::new();
    let (graw_pull, graw_ep) = bind_pull(&ctx);
    let (decoder_pull, decoder_ep) = bind_pull(&ctx);
    let (cmd_ep, _data_addr, shutdown_tx, task) =
        armed_and_started(4, 64, &graw_ep, &decoder_ep).await;

    let stopped = rpc(&cmd_ep, &Command::Stop).await;
    assert!(stopped.success, "{}", stopped.message);
    assert_eq!(stopped.state, ComponentState::Configured);

    for (sock, link) in [(&graw_pull, "graw-writer"), (&decoder_pull, "decoder")] {
        let got = collect_until_eos(sock, link);
        assert!(got.frames.is_empty(), "{link}: データは無い");
        assert_eq!(got.eos_run_number, Some(64), "{link}: 強制 EOS");
        assert_nothing_downstream(sock, link, "強制 EOS はちょうど 1 回");
    }

    shutdown(shutdown_tx, task).await;
}

/// 0 バイト接続だけで終わった run も、`Stop` の強制 EOS で**ちょうど 1 回**閉じる
/// (`owes_eos` を迷い込みが降ろしてしまわないこと)。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stop_forces_the_end_of_stream_even_after_a_stray_zero_byte_connection() {
    let ctx = zmq::Context::new();
    let (graw_pull, graw_ep) = bind_pull(&ctx);
    let (decoder_pull, decoder_ep) = bind_pull(&ctx);
    let (cmd_ep, data_addr, shutdown_tx, task) =
        armed_and_started(5, 65, &graw_ep, &decoder_ep).await;

    let stray = TcpStream::connect(&data_addr).await.unwrap();
    drop(stray);
    poll_status(
        &cmd_ep,
        Duration::from_secs(5),
        "empty_connections = 1",
        |r| metric_u64(r, "empty_connections") == 1,
    )
    .await;

    let stopped = rpc(&cmd_ep, &Command::Stop).await;
    assert!(stopped.success, "{}", stopped.message);

    for (sock, link) in [(&graw_pull, "graw-writer"), (&decoder_pull, "decoder")] {
        let got = collect_until_eos(sock, link);
        assert!(got.frames.is_empty(), "{link}");
        assert_eq!(got.eos_run_number, Some(65), "{link}: 強制 EOS");
        assert_nothing_downstream(sock, link, "強制 EOS はちょうど 1 回");
    }

    shutdown(shutdown_tx, task).await;
}

// ---------------------------------------------------------------------
// T5 — GetStatus の可視化(0 Hz と stale link の切り分け材料)
// ---------------------------------------------------------------------

/// `peer` / `last_read_unix_ns` / `extra_connections` / `empty_connections` が
/// GetStatus に載ること。Start 前は peer / last_read が null、カウンタは 0。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn get_status_exposes_the_current_peer_and_the_last_read_time() {
    let ctx = zmq::Context::new();
    let (_graw_pull, graw_ep) = bind_pull(&ctx);
    let (_decoder_pull, decoder_ep) = bind_pull(&ctx);
    let (cmd_ep, shutdown_tx, task) = start_receiver(test_params(6, &graw_ep, &decoder_ep)).await;

    rpc(
        &cmd_ep,
        &Command::Configure(RunConfig {
            run_number: 90,
            comment: String::new(),
            config: serde_json::Value::Null,
        }),
    )
    .await;

    // Start 前: 接続も受信も無い
    let idle = rpc(&cmd_ep, &Command::GetStatus).await;
    assert!(
        metric(&idle, "peer").is_null(),
        "Start 前に peer が載っている"
    );
    assert!(
        metric(&idle, "last_read_unix_ns").is_null(),
        "受信前の last_read_unix_ns は null(0 と混同しない)"
    );
    assert_eq!(metric_u64(&idle, "extra_connections"), 0);
    assert_eq!(metric_u64(&idle, "empty_connections"), 0);

    let data_addr = bind_address(&rpc(&cmd_ep, &Command::Arm).await);
    rpc(&cmd_ep, &Command::Start { run_number: 90 }).await;

    let before_ns = unix_nanos_now();
    let mut link = TcpStream::connect(&data_addr).await.unwrap();
    let client_addr = link.local_addr().unwrap().to_string();
    link.write_all(&make_frame(96, true, 0x0F)).await.unwrap();

    let status = poll_status(
        &cmd_ep,
        Duration::from_secs(5),
        "1 フレーム受信",
        |r| metric_u64(r, "frames") >= 1,
    )
    .await;
    let after_ns = unix_nanos_now();

    assert_eq!(
        metric(&status, "peer").as_str(),
        Some(client_addr.as_str()),
        "現接続の peer(= クライアントのローカルアドレス)が載ること"
    );
    let last_read = metric_u64(&status, "last_read_unix_ns");
    assert!(
        (before_ns..=after_ns).contains(&last_read),
        "last_read_unix_ns={last_read} が受信の前後({before_ns}..={after_ns})に入っていない"
    );

    // 接続が閉じれば peer は null に戻る(= 「今は誰も繋がっていない」が読める)
    link.shutdown().await.unwrap();
    drop(link);
    let closed = poll_status(
        &cmd_ep,
        Duration::from_secs(5),
        "peer が null に戻る",
        |r| metric(r, "peer").is_null(),
    )
    .await;
    assert_eq!(
        metric_u64(&closed, "last_read_unix_ns"),
        last_read,
        "最終受信時刻は接続が閉じても残る(stale link の判別材料)"
    );

    shutdown(shutdown_tx, task).await;
}
