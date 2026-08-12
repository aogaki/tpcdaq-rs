//! receiver コンポーネントの統合テスト(TODO/006 テスト (a)〜(f))。
//!
//! fake CoBo(テスト内 TCP クライアント)→ receiver → PULL ×2 の実配線で、
//! バイト忠実性・sequence_number・EOS・状態機械・listen-before-start・run 跨ぎを検証する。
//!
//! ポートはすべて 0(動的)。実 bind アドレスは Arm 応答の metrics から取る
//! (= controller が DataLinkSet を組む経路そのもの、SPEC §8.2)。
//! 条件待ちは固定 sleep ではなくポーリング + タイムアウト。

#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::{broadcast, oneshot};
use tpcdaq::command::{Command, CommandResponse, ComponentState, RunConfig};
use tpcdaq::msg::{Message, RawFrames};
use tpcdaq::receiver::{run_receiver, ReceiverParams};
use tpcdaq::zmq_helper;

// ---------------------------------------------------------------------
// 配線ヘルパ
// ---------------------------------------------------------------------

/// `tcp://127.0.0.1:0` に PULL を bind し、実エンドポイントを返す。
/// receiver の PUSH は connect 側なので、**先に bind しておけば** slow joiner 問題は起きない。
fn bind_pull(ctx: &zmq::Context) -> (zmq::Socket, String) {
    let sock = ctx.socket(zmq::PULL).unwrap();
    sock.set_linger(0).unwrap();
    zmq_helper::apply_pull_hwm(&sock).unwrap();
    sock.bind("tcp://127.0.0.1:0").unwrap();
    sock.set_rcvtimeo(10_000).unwrap(); // ハングではなく失敗で終わらせる
    let endpoint = sock.get_last_endpoint().unwrap().unwrap();
    (sock, endpoint)
}

/// テスト用 ReceiverParams。データ listen とコマンド REP はどちらもポート 0。
fn test_params(cobo_id: u32, listen: &str, graw: &str, decoder: &str) -> ReceiverParams {
    ReceiverParams {
        cobo_id,
        listen: listen.to_string(),
        command_listen: "tcp://127.0.0.1:0".to_string(),
        graw_writer_endpoint: graw.to_string(),
        decoder_endpoint: decoder.to_string(),
        batch_max_bytes: 64 * 1024, // 小さめ = 1 run で複数バッチになる
        batch_max_ms: 10,
        queue_frames: 256,
        heartbeat_ms: 1_000,
        hwm: zmq_helper::DEFAULT_HWM,
    }
}

/// receiver を起動し、コマンド REP の実エンドポイントを返す。
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

/// 応答の metrics から数値を取り出す。
fn metric_u64(resp: &CommandResponse, key: &str) -> u64 {
    resp.metrics.as_ref().unwrap()[key]
        .as_u64()
        .unwrap_or_else(|| panic!("metrics.{key} is not a number: {:?}", resp.metrics))
}

/// Arm 応答が報告した実 bind アドレス(SPEC §8.2: controller はこれを DataLinkSet に使う)。
fn bind_address(resp: &CommandResponse) -> String {
    resp.metrics.as_ref().unwrap()["bind_address"]
        .as_str()
        .unwrap_or_else(|| panic!("Arm response has no bind_address: {:?}", resp.metrics))
        .to_string()
}

// ---------------------------------------------------------------------
// 合成 MFM フレーム(framer のテストと同じ作り方 — 実 GRAW でなくてよい)
// ---------------------------------------------------------------------

/// blkSize=1(metaType 下位ニブル 0)、frameSize_blk = total_bytes の合成フレーム。
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
///
/// 先頭を大物にしてあるのは、`batch_max_bytes` = 64 KiB を 1 フレーム目で越えさせて
/// 「サイズで閉じるバッチ」と「残りを閉じるバッチ」の 2 通を必ず作るため。
fn sample_frames() -> Vec<Vec<u8>> {
    vec![
        make_frame(100_000, true, 0x44),
        make_frame(24, true, 0x11),
        make_frame(4096, false, 0x22), // big-endian ヘッダも混ぜる
        make_frame(12, true, 0x33),    // 最小級
    ]
}

/// fake CoBo: 接続 → 半端なチャンク境界で送出 → close(= EOF = run 境界)。
async fn replay_frames(addr: &str, frames: &[Vec<u8>]) {
    let mut stream = TcpStream::connect(addr).await.expect("fake CoBo connect");
    let stream_bytes: Vec<u8> = frames.concat();
    // フレーム境界と一致しないチャンクで送る(framer の境界跨ぎを実配線で突く)
    for chunk in stream_bytes.chunks(7_000) {
        stream.write_all(chunk).await.expect("fake CoBo write");
    }
    stream.shutdown().await.expect("fake CoBo close");
}

// ---------------------------------------------------------------------
// PULL 側の収集
// ---------------------------------------------------------------------

#[derive(Debug, Default)]
struct Collected {
    /// Data バッチが運んできた生フレーム(到着順)。
    frames: Vec<Vec<u8>>,
    sequence_numbers: Vec<u64>,
    run_numbers: Vec<u32>,
    source_ids: Vec<u32>,
    heartbeats: u64,
    eos_run_number: Option<u32>,
    eos_source_id: Option<u32>,
}

impl Collected {
    fn concatenated(&self) -> Vec<u8> {
        self.frames.concat()
    }
}

/// EndOfStream が来るまで PULL を読み切る(ロスレスリンクなので取りこぼしは無い前提)。
fn collect_until_eos(sock: &zmq::Socket, link: &str) -> Collected {
    let mut c = Collected::default();
    loop {
        let bytes = sock
            .recv_bytes(0)
            .unwrap_or_else(|e| panic!("{link}: PULL recv failed/timed out: {e}"));
        match Message::<RawFrames>::from_msgpack(&bytes).unwrap() {
            Message::Data(batch) => {
                c.sequence_numbers.push(batch.sequence_number);
                c.run_numbers.push(batch.run_number);
                c.source_ids.push(batch.source_id);
                for frame in batch.payload {
                    c.frames.push(frame.into_vec());
                }
            }
            Message::EndOfStream {
                source_id,
                run_number,
            } => {
                c.eos_run_number = Some(run_number);
                c.eos_source_id = Some(source_id);
                return c;
            }
            Message::Heartbeat { .. } => c.heartbeats += 1,
        }
    }
}

/// sequence_number が 0 から連続していること(SPEC §2.2: ロスレスリンクは連続性を検証する)。
fn assert_sequence_is_contiguous_from_zero(seqs: &[u64], link: &str) {
    let expected: Vec<u64> = (0..seqs.len() as u64).collect();
    assert_eq!(
        seqs,
        expected.as_slice(),
        "{link}: sequence_number が不連続"
    );
}

// ---------------------------------------------------------------------
// (a)(b)(c) バイト再構成 + seq 連続 + EOS
// ---------------------------------------------------------------------

/// fake CoBo が流したバイト列が、両リンクで**フレーム境界ごと完全一致**で再構成されること。
/// 併せて sequence_number の連続(b)と EOF → EOS(c)も確認する。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn both_links_reassemble_the_cobo_bytes_exactly() {
    let ctx = zmq::Context::new();
    let (graw_pull, graw_ep) = bind_pull(&ctx);
    let (decoder_pull, decoder_ep) = bind_pull(&ctx);

    let (cmd_ep, shutdown_tx, task) =
        start_receiver(test_params(3, "127.0.0.1:0", &graw_ep, &decoder_ep)).await;

    let resp = rpc(
        &cmd_ep,
        &Command::Configure(RunConfig {
            run_number: 41,
            comment: "byte fidelity".to_string(),
            config: serde_json::Value::Null,
        }),
    )
    .await;
    assert!(resp.success, "{}", resp.message);

    let armed = rpc(&cmd_ep, &Command::Arm).await;
    assert!(armed.success, "{}", armed.message);
    let data_addr = bind_address(&armed);

    let started = rpc(&cmd_ep, &Command::Start { run_number: 41 }).await;
    assert!(started.success, "{}", started.message);
    assert_eq!(started.state, ComponentState::Running);

    let frames = sample_frames();
    replay_frames(&data_addr, &frames).await;

    let expected_bytes: Vec<u8> = frames.concat();
    for (sock, link) in [(&graw_pull, "graw-writer"), (&decoder_pull, "decoder")] {
        let got = collect_until_eos(sock, link);

        // (a) バイト再構成一致 — 連結だけでなくフレーム境界も一致すること
        assert_eq!(got.concatenated(), expected_bytes, "{link}: バイト列不一致");
        assert_eq!(got.frames, frames, "{link}: フレーム境界が入力と違う");

        // (b) sequence_number は 0 から連続
        assert_sequence_is_contiguous_from_zero(&got.sequence_numbers, link);
        assert!(
            got.sequence_numbers.len() >= 2,
            "{link}: 104 KB / batch_max_bytes 64 KiB なら 2 バッチ以上になるはず(got {})",
            got.sequence_numbers.len()
        );

        // 全バッチが run_number と source_id(= cobo_id、SPEC §3.2)を運ぶ
        assert!(
            got.run_numbers.iter().all(|r| *r == 41),
            "{link}: run_number"
        );
        assert!(got.source_ids.iter().all(|s| *s == 3), "{link}: source_id");

        // (c) EOF → EOS(両リンク)
        assert_eq!(got.eos_run_number, Some(41), "{link}: EOS の run_number");
        assert_eq!(got.eos_source_id, Some(3), "{link}: EOS の source_id");
    }

    // カウンタ: 4 フレーム / 入力バイト全部 / 溢れ 0
    let stopped = rpc(&cmd_ep, &Command::Stop).await;
    assert!(stopped.success, "{}", stopped.message);
    assert_eq!(metric_u64(&stopped, "frames"), frames.len() as u64);
    assert_eq!(metric_u64(&stopped, "bytes"), expected_bytes.len() as u64);
    assert_eq!(metric_u64(&stopped, "overflow_frames"), 0);
    assert_eq!(metric_u64(&stopped, "framer_resets"), 0);
    assert!(metric_u64(&stopped, "batches") >= 2);

    shutdown_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("receiver did not stop within 5 s")
        .unwrap();
}

/// (c) データが 1 バイトも無い接続でも、EOF は run 境界として両リンクに EOS を出す。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn eof_without_any_data_still_delivers_end_of_stream() {
    let ctx = zmq::Context::new();
    let (graw_pull, graw_ep) = bind_pull(&ctx);
    let (decoder_pull, decoder_ep) = bind_pull(&ctx);

    let (cmd_ep, shutdown_tx, task) =
        start_receiver(test_params(0, "127.0.0.1:0", &graw_ep, &decoder_ep)).await;

    rpc(
        &cmd_ep,
        &Command::Configure(RunConfig {
            run_number: 5,
            comment: String::new(),
            config: serde_json::Value::Null,
        }),
    )
    .await;
    let data_addr = bind_address(&rpc(&cmd_ep, &Command::Arm).await);
    rpc(&cmd_ep, &Command::Start { run_number: 5 }).await;

    // 接続してすぐ閉じる
    let stream = TcpStream::connect(&data_addr).await.unwrap();
    drop(stream);

    for (sock, link) in [(&graw_pull, "graw-writer"), (&decoder_pull, "decoder")] {
        let got = collect_until_eos(sock, link);
        assert!(got.frames.is_empty(), "{link}: データは無いはず");
        assert_eq!(got.eos_run_number, Some(5), "{link}");
    }

    shutdown_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("receiver did not stop within 5 s")
        .unwrap();
}

/// CoBo が繋がったまま(EOF が来ないまま)`Stop` された場合、receiver が**未送の EOS**を
/// 自分で出すこと(SPEC §1.3 の run 停止シーケンス 2「EOF が来なければ Stop で強制 EOS」)。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stop_emits_the_pending_end_of_stream_when_the_cobo_never_closes() {
    let ctx = zmq::Context::new();
    let (graw_pull, graw_ep) = bind_pull(&ctx);
    let (decoder_pull, decoder_ep) = bind_pull(&ctx);

    let (cmd_ep, shutdown_tx, task) =
        start_receiver(test_params(0, "127.0.0.1:0", &graw_ep, &decoder_ep)).await;

    rpc(
        &cmd_ep,
        &Command::Configure(RunConfig {
            run_number: 21,
            comment: String::new(),
            config: serde_json::Value::Null,
        }),
    )
    .await;
    let data_addr = bind_address(&rpc(&cmd_ep, &Command::Arm).await);
    rpc(&cmd_ep, &Command::Start { run_number: 21 }).await;

    // 接続したまま(close せずに)データを送る
    let frames = vec![make_frame(256, true, 0x91)];
    let mut stream = TcpStream::connect(&data_addr).await.unwrap();
    stream.write_all(&frames[0]).await.unwrap();

    let stopped = rpc(&cmd_ep, &Command::Stop).await;
    assert!(stopped.success, "{}", stopped.message);
    assert_eq!(stopped.state, ComponentState::Configured);

    for (sock, link) in [(&graw_pull, "graw-writer"), (&decoder_pull, "decoder")] {
        let got = collect_until_eos(sock, link);
        assert_eq!(got.frames, frames, "{link}: Stop 前のデータは落とさない");
        assert_eq!(got.eos_run_number, Some(21), "{link}: 強制 EOS");
    }
    drop(stream);

    shutdown_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("receiver did not stop within 5 s")
        .unwrap();
}

// ---------------------------------------------------------------------
// (d) 全シーケンス + bind アドレス報告
// ---------------------------------------------------------------------

/// Configure → Arm → Start → データ → EOF → Stop → Reset の全順路。
/// Arm 応答が**実際に bind したアドレス**を metrics で返すこと(SPEC §8.2)を含む。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_command_sequence_reports_the_bound_address() {
    let ctx = zmq::Context::new();
    let (graw_pull, graw_ep) = bind_pull(&ctx);
    let (decoder_pull, decoder_ep) = bind_pull(&ctx);

    let (cmd_ep, shutdown_tx, task) =
        start_receiver(test_params(1, "127.0.0.1:0", &graw_ep, &decoder_ep)).await;

    // Idle では Start は通らない(状態機械、SPEC §1.3)
    let bad = rpc(&cmd_ep, &Command::Start { run_number: 9 }).await;
    assert!(!bad.success);
    assert_eq!(bad.state, ComponentState::Idle);

    let configured = rpc(
        &cmd_ep,
        &Command::Configure(RunConfig {
            run_number: 9,
            comment: "full sequence".to_string(),
            config: serde_json::Value::Null,
        }),
    )
    .await;
    assert_eq!(configured.state, ComponentState::Configured);
    assert_eq!(configured.run_number, Some(9));

    let armed = rpc(&cmd_ep, &Command::Arm).await;
    assert_eq!(armed.state, ComponentState::Armed);
    let data_addr = bind_address(&armed);
    // port 0 を渡したので、実ポートは 0 ではない具体値になっていること
    let port: u16 = data_addr.rsplit(':').next().unwrap().parse().unwrap();
    assert!(port > 0, "bind_address がポート 0 のまま: {data_addr}");
    assert_eq!(
        armed.metrics.as_ref().unwrap()["router_port"]
            .as_u64()
            .unwrap(),
        u64::from(port),
        "router_port は bind_address のポートと一致すること(SPEC §8.2)"
    );

    let started = rpc(&cmd_ep, &Command::Start { run_number: 9 }).await;
    assert_eq!(started.state, ComponentState::Running);
    assert_eq!(started.run_number, Some(9));

    let frames = vec![make_frame(64, true, 0x55), make_frame(128, false, 0x66)];
    replay_frames(&data_addr, &frames).await;

    for (sock, link) in [(&graw_pull, "graw-writer"), (&decoder_pull, "decoder")] {
        let got = collect_until_eos(sock, link);
        assert_eq!(got.frames, frames, "{link}");
    }

    // GetStatus は遷移しない + カウンタを返す
    let status = rpc(&cmd_ep, &Command::GetStatus).await;
    assert_eq!(status.state, ComponentState::Running);
    assert_eq!(metric_u64(&status, "frames"), 2);

    let stopped = rpc(&cmd_ep, &Command::Stop).await;
    assert!(stopped.success, "{}", stopped.message);
    assert_eq!(stopped.state, ComponentState::Configured);

    let reset = rpc(&cmd_ep, &Command::Reset).await;
    assert_eq!(reset.state, ComponentState::Idle);

    shutdown_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("receiver did not stop within 5 s")
        .unwrap();
}

// ---------------------------------------------------------------------
// (e) listen-before-start の負性テスト
// ---------------------------------------------------------------------

/// Arm する前は誰も listen していない(= CoBo は繋げない)。Arm した瞬間から繋がる。
/// 空きポートは「port 0 で bind → 実ポートを読む → 手放す」で採る(固定ポートを書かない)。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nothing_listens_before_arm() {
    let ctx = zmq::Context::new();
    let (_graw_pull, graw_ep) = bind_pull(&ctx);
    let (_decoder_pull, decoder_ep) = bind_pull(&ctx);

    // 空いているポートを 1 つ確保して即座に手放す
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = probe.local_addr().unwrap();
    drop(probe);

    let (cmd_ep, shutdown_tx, task) =
        start_receiver(test_params(0, &addr.to_string(), &graw_ep, &decoder_ep)).await;

    rpc(
        &cmd_ep,
        &Command::Configure(RunConfig {
            run_number: 1,
            comment: String::new(),
            config: serde_json::Value::Null,
        }),
    )
    .await;

    // Configured(= Arm 前)では接続できない
    let before = TcpStream::connect(addr).await;
    assert!(
        before.is_err(),
        "Arm 前に {addr} へ接続できてしまった(listen-before-start 違反)"
    );

    // Arm すると同じアドレスで listen が立つ
    let armed = rpc(&cmd_ep, &Command::Arm).await;
    assert!(armed.success, "{}", armed.message);
    assert_eq!(bind_address(&armed), addr.to_string());
    let after = TcpStream::connect(addr).await;
    assert!(after.is_ok(), "Arm 後に接続できない: {:?}", after.err());

    shutdown_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("receiver did not stop within 5 s")
        .unwrap();
}

// ---------------------------------------------------------------------
// (f) run 跨ぎ
// ---------------------------------------------------------------------

/// EOF 後も次の accept に戻ること(同一 run 内の再接続)と、次 run で
/// sequence_number が 0 に戻り run_number が更新されること。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn eof_returns_to_accept_and_the_next_run_resets_sequence_numbers() {
    let ctx = zmq::Context::new();
    let (graw_pull, graw_ep) = bind_pull(&ctx);
    let (decoder_pull, decoder_ep) = bind_pull(&ctx);

    let (cmd_ep, shutdown_tx, task) =
        start_receiver(test_params(0, "127.0.0.1:0", &graw_ep, &decoder_ep)).await;

    // --- run 7 ---
    rpc(
        &cmd_ep,
        &Command::Configure(RunConfig {
            run_number: 7,
            comment: String::new(),
            config: serde_json::Value::Null,
        }),
    )
    .await;
    let addr7 = bind_address(&rpc(&cmd_ep, &Command::Arm).await);
    rpc(&cmd_ep, &Command::Start { run_number: 7 }).await;

    let first = vec![make_frame(32, true, 0x71)];
    replay_frames(&addr7, &first).await;
    for (sock, link) in [(&graw_pull, "graw-writer"), (&decoder_pull, "decoder")] {
        let got = collect_until_eos(sock, link);
        assert_eq!(got.frames, first, "{link}: 1 接続目");
        assert_eq!(got.sequence_numbers, vec![0], "{link}: 1 接続目の seq");
    }

    // EOF の後も listener は生きていて、同じ run のまま次の接続を受ける
    let second = vec![make_frame(48, false, 0x72), make_frame(16, true, 0x73)];
    replay_frames(&addr7, &second).await;
    for (sock, link) in [(&graw_pull, "graw-writer"), (&decoder_pull, "decoder")] {
        let got = collect_until_eos(sock, link);
        assert_eq!(got.frames, second, "{link}: 2 接続目(再 accept)");
        assert_eq!(
            got.sequence_numbers,
            vec![1],
            "{link}: run 内では seq は継続する"
        );
        assert_eq!(got.eos_run_number, Some(7), "{link}");
    }

    let stopped = rpc(&cmd_ep, &Command::Stop).await;
    assert!(stopped.success, "{}", stopped.message);
    assert_eq!(metric_u64(&stopped, "frames"), 3);

    // --- run 8(Arm し直し = 新しい listener)---
    let armed = rpc(&cmd_ep, &Command::Arm).await;
    assert!(armed.success, "{}", armed.message);
    let addr8 = bind_address(&armed);
    let started = rpc(&cmd_ep, &Command::Start { run_number: 8 }).await;
    assert!(started.success, "{}", started.message);

    let third = vec![make_frame(80, true, 0x81)];
    replay_frames(&addr8, &third).await;
    for (sock, link) in [(&graw_pull, "graw-writer"), (&decoder_pull, "decoder")] {
        let got = collect_until_eos(sock, link);
        assert_eq!(got.frames, third, "{link}: 次 run");
        assert_eq!(
            got.sequence_numbers,
            vec![0],
            "{link}: run 開始で seq は 0 に戻る(SPEC §2.2)"
        );
        assert!(
            got.run_numbers.iter().all(|r| *r == 8),
            "{link}: 新 run 番号"
        );
        assert_eq!(got.eos_run_number, Some(8), "{link}");
    }

    // カウンタは run 毎(Start でリセット)
    let stopped = rpc(&cmd_ep, &Command::Stop).await;
    assert_eq!(metric_u64(&stopped, "frames"), 1);

    shutdown_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("receiver did not stop within 5 s")
        .unwrap();
}

// ---------------------------------------------------------------------
// Heartbeat(SPEC §2.2「アイドル時 1 Hz」— テストでは周期を縮めて確認)
// ---------------------------------------------------------------------

/// Running でデータが来ない間、両リンクに Heartbeat が出続けること。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_idle_running_receiver_emits_heartbeats_on_both_links() {
    let ctx = zmq::Context::new();
    let (graw_pull, graw_ep) = bind_pull(&ctx);
    let (decoder_pull, decoder_ep) = bind_pull(&ctx);

    let mut params = test_params(2, "127.0.0.1:0", &graw_ep, &decoder_ep);
    params.heartbeat_ms = 100; // 1 Hz のままだとテストが遅いだけなので縮める
    let (cmd_ep, shutdown_tx, task) = start_receiver(params).await;

    rpc(
        &cmd_ep,
        &Command::Configure(RunConfig {
            run_number: 12,
            comment: String::new(),
            config: serde_json::Value::Null,
        }),
    )
    .await;
    rpc(&cmd_ep, &Command::Arm).await;
    rpc(&cmd_ep, &Command::Start { run_number: 12 }).await;

    let started = Instant::now();
    for (sock, link) in [(&graw_pull, "graw-writer"), (&decoder_pull, "decoder")] {
        for expected_counter in 0..2u64 {
            let bytes = sock
                .recv_bytes(0)
                .unwrap_or_else(|e| panic!("{link}: heartbeat を受け取れない: {e}"));
            match Message::<RawFrames>::from_msgpack(&bytes).unwrap() {
                Message::Heartbeat {
                    source_id,
                    run_number,
                    counter,
                } => {
                    assert_eq!(source_id, 2, "{link}");
                    assert_eq!(run_number, 12, "{link}");
                    assert_eq!(counter, expected_counter, "{link}: counter は単調増加");
                }
                other => panic!("{link}: Heartbeat 以外が来た: {other:?}"),
            }
        }
    }
    // 100 ms 周期で 2 通 = 最低 100 ms は経っている(即時連打していない)
    assert!(started.elapsed() >= Duration::from_millis(100));

    shutdown_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("receiver did not stop within 5 s")
        .unwrap();
}
