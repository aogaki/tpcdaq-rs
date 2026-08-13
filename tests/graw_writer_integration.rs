//! graw-writer コンポーネントの統合テスト(TODO/007 テスト (a)〜(e))。
//!
//! PUSH で直接 `Batch<RawFrames>` を投入し(receiver は経由しない)、graw-writer の
//! PULL 実配線・asadIdx 振り分け・ローテーション・EOS 駆動の close・状態機械を検証する。
//!
//! ポートはすべて 0(動的)。実 bind アドレスは `Arm` 応答の metrics から取る
//! (receiver_integration.rs と同じ流儀)。条件待ちは固定 sleep ではなくポーリング + タイムアウト。

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_bytes::ByteBuf;
use tokio::sync::{broadcast, oneshot};
use tpcdaq::command::{Command, CommandResponse, ComponentState, RunConfig};
use tpcdaq::graw_writer::{run_graw_writer, GrawWriterParams};
use tpcdaq::msg::{Batch, Message, RawFrames};

// ---------------------------------------------------------------------
// 配線ヘルパ
// ---------------------------------------------------------------------

fn temp_root(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "tpcdaq-graw-writer-it-{tag}-{}-{}",
        std::process::id(),
        n
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// テスト用 GrawWriterParams。PULL bind とコマンド REP はどちらもポート 0。
fn test_params(
    output_root: PathBuf,
    expected_sources: Vec<u32>,
    max_file_bytes: u64,
) -> GrawWriterParams {
    GrawWriterParams {
        pull_bind: "tcp://127.0.0.1:0".to_string(),
        command_listen: "tcp://127.0.0.1:0".to_string(),
        output_root,
        max_file_bytes,
        flush_interval_ms: 50, // 待ち時間短縮(既定 1000 ms だとポーリングが遅い)
        expected_sources,
    }
}

/// graw-writer を起動し、コマンド REP の実エンドポイントを返す。
async fn start_graw_writer(
    params: GrawWriterParams,
) -> (String, broadcast::Sender<()>, tokio::task::JoinHandle<()>) {
    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    let (ep_tx, ep_rx) = oneshot::channel();
    let handle = tokio::spawn(run_graw_writer(params, shutdown_rx, Some(ep_tx)));
    let endpoint = ep_rx
        .await
        .expect("graw-writer never reported its command endpoint");
    (endpoint, shutdown_tx, handle)
}

async fn shutdown_and_join(shutdown: broadcast::Sender<()>, task: tokio::task::JoinHandle<()>) {
    let _ = shutdown.send(());
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("graw-writer did not stop within 5 s")
        .unwrap();
}

/// REQ で 1 往復する(毎回新しい REQ ソケット、receiver_integration.rs と同じ流儀)。
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

/// 条件を満たすまで `cmd` を投げ続ける(既定 20 ms 間隔)。データ到着 ⇔ REP 応答は非同期なので
/// 固定 sleep ではなくこれで待つ。
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

fn files_of(resp: &CommandResponse) -> Vec<serde_json::Value> {
    resp.metrics.as_ref().unwrap()["files"]
        .as_array()
        .unwrap_or_else(|| panic!("no files array in metrics: {:?}", resp.metrics))
        .clone()
}

fn file_path_for(resp: &CommandResponse, cobo: u32, asad: u8) -> PathBuf {
    let files = files_of(resp);
    let entry = files
        .iter()
        .find(|f| {
            f["cobo"].as_u64() == Some(u64::from(cobo))
                && f["asad"].as_u64() == Some(u64::from(asad))
        })
        .unwrap_or_else(|| panic!("no file for cobo={cobo} asad={asad}: {files:?}"));
    PathBuf::from(entry["path"].as_str().unwrap())
}

/// ctrl ファイル(非 AsAd、SPEC §7 v1.2)。`asad` は JSON 上 `null`。
fn ctrl_file_path_for(resp: &CommandResponse, cobo: u32) -> PathBuf {
    let files = files_of(resp);
    let entry = files
        .iter()
        .find(|f| f["cobo"].as_u64() == Some(u64::from(cobo)) && f["asad"].is_null())
        .unwrap_or_else(|| panic!("no ctrl file for cobo={cobo}: {files:?}"));
    PathBuf::from(entry["path"].as_str().unwrap())
}

/// Configure → Arm まで進め、実 PULL bind アドレスを返す。
async fn configure_arm(endpoint: &str, run_number: u32) -> String {
    let cfg = rpc(
        endpoint,
        &Command::Configure(RunConfig {
            run_number,
            comment: "graw-writer integration test".to_string(),
            config: serde_json::Value::Null,
        }),
    )
    .await;
    assert!(cfg.success, "Configure failed: {}", cfg.message);
    let armed = rpc(endpoint, &Command::Arm).await;
    assert!(armed.success, "Arm failed: {}", armed.message);
    bind_address(&armed)
}

fn connect_push(ctx: &zmq::Context, endpoint: &str) -> zmq::Socket {
    let push = ctx.socket(zmq::PUSH).unwrap();
    push.set_linger(0).unwrap();
    tpcdaq::zmq_helper::apply_push_hwm(&push).unwrap();
    push.connect(endpoint).unwrap();
    push
}

// ---------------------------------------------------------------------
// 合成フレーム(peek_asad が読めればよい — offset 27 だけが意味を持つ)
// ---------------------------------------------------------------------

/// offset 27 = asadIdx、残りは `fill` で埋めた最小 MFM フレーム。frameType=1 を明示する
/// (v1.2 — `peek_asad` が frameType-aware になったため)。長さ・内容が非対称になるよう
/// `extra_len` / `fill` を呼び出し側で変える(取り違えが目に見えるように)。
fn make_frame(asad: u8, extra_len: usize, fill: u8) -> Vec<u8> {
    let mut b = vec![fill; 28 + extra_len];
    b[0] = 0x00; // big-endian
    b[5] = 0x00;
    b[6] = 0x01; // frameType=1
    b[27] = asad;
    b
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

// ---------------------------------------------------------------------
// (a) + (c): 2 CoBo × 2 AsAd → 4 ファイルがバイト一致、全 EOS で metrics にファイル実績
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_cobo_two_asad_batches_produce_four_byte_identical_files_and_close_on_eos() {
    let root = temp_root("two-cobo-two-asad");
    let params = test_params(root.clone(), vec![0, 1], 10 * 1024 * 1024);
    let (cmd_ep, shutdown, task) = start_graw_writer(params).await;

    let pull_bind = configure_arm(&cmd_ep, 11).await;
    let ctx = zmq::Context::new();
    let push = connect_push(&ctx, &pull_bind);
    let started = rpc(&cmd_ep, &Command::Start { run_number: 11 }).await;
    assert!(started.success, "{}", started.message);

    let f0a0 = make_frame(0, 10, 0xAA);
    let f0a1 = make_frame(1, 15, 0xBB);
    let f1a0 = make_frame(0, 20, 0xCC);
    let f1a1 = make_frame(1, 25, 0xDD);
    // 実 2025 run 先頭の 12 B 制御フレーム相当(非 AsAd — v1.2 で ctrl/ に保全される)。
    let ctrl0 = vec![0xEEu8; 12];

    // 入力を asadIdx で分別した列(= 期待するファイル内容そのもの)。CoBo0 の先頭に非 AsAd
    // フレームを 1 本混ぜる(実機の実測パターンを模す、SPEC §7 v1.2)。
    send_batch(
        &push,
        0,
        11,
        0,
        &[ctrl0.clone(), f0a0.clone(), f0a1.clone()],
    );
    send_batch(&push, 1, 11, 0, &[f1a0.clone(), f1a1.clone()]);
    send_eos(&push, 0, 11);
    send_eos(&push, 1, 11);

    // `files_open == 0` を待つ(単なる `files.len() == N` だと、まだ BufWriter 内バッファに
    // 留まっている = ディスク未反映の状態でも真になりうる — flush はローテーションと close 時
    // のみで per-write ではない、SPEC §7)。全 EOS → finalize(flush+fsync+close)完了の合図。
    // 5 ファイル = per-AsAd 4(CoBo0×2 + CoBo1×2) + ctrl 1(CoBo0)。
    let done = poll_until(&cmd_ep, &Command::GetStatus, Duration::from_secs(5), |r| {
        let m = r.metrics.as_ref();
        m.and_then(|m| m["files_open"].as_u64()) == Some(0)
            && m.and_then(|m| m["files_closed"].as_u64()) == Some(5)
    })
    .await;

    assert_eq!(
        std::fs::read(file_path_for(&done, 0, 0)).unwrap(),
        f0a0,
        "CoBo0/AsAd0"
    );
    assert_eq!(
        std::fs::read(file_path_for(&done, 0, 1)).unwrap(),
        f0a1,
        "CoBo0/AsAd1"
    );
    assert_eq!(
        std::fs::read(file_path_for(&done, 1, 0)).unwrap(),
        f1a0,
        "CoBo1/AsAd0"
    );
    assert_eq!(
        std::fs::read(file_path_for(&done, 1, 1)).unwrap(),
        f1a1,
        "CoBo1/AsAd1"
    );
    let ctrl_path = ctrl_file_path_for(&done, 0);
    assert_eq!(
        std::fs::read(&ctrl_path).unwrap(),
        ctrl0,
        "CoBo0/ctrl — 非 AsAd フレームがバイトそのまま保全される"
    );
    assert_eq!(
        ctrl_path.parent().and_then(|p| p.file_name()),
        Some(std::ffi::OsStr::new("ctrl")),
        "ctrl/ サブディレクトリに置かれる: {ctrl_path:?}"
    );

    assert_eq!(metric_u64(&done, "ctrl_frames"), 1);
    assert_eq!(metric_u64(&done, "seq_gaps"), 0);
    assert_eq!(metric_u64(&done, "write_errors"), 0);
    assert_ne!(
        done.state,
        ComponentState::Error,
        "非 AsAd フレームは Error にしない(SPEC §7 v1.2)"
    );

    shutdown_and_join(shutdown, task).await;
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------
// (b) ローテーション跨ぎでも連結一致
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rotation_across_a_boundary_still_concatenates_to_the_input() {
    let root = temp_root("rotation-e2e");
    let f1 = make_frame(0, 40, 0x11); // 68 B
    let f2 = make_frame(0, 40, 0x22); // 68 B
    let max_file_bytes = f1.len() as u64 + 10; // f2 の書き込みで max を超える
    let params = test_params(root.clone(), vec![5], max_file_bytes);
    let (cmd_ep, shutdown, task) = start_graw_writer(params).await;

    let pull_bind = configure_arm(&cmd_ep, 3).await;
    let ctx = zmq::Context::new();
    let push = connect_push(&ctx, &pull_bind);
    rpc(&cmd_ep, &Command::Start { run_number: 3 }).await;

    send_batch(&push, 5, 3, 0, &[f1.clone(), f2.clone()]);
    send_eos(&push, 5, 3);

    // files_open == 0 を待つ(finalize 完了の合図 — 上のテストと同じ理由)。
    let done = poll_until(&cmd_ep, &Command::GetStatus, Duration::from_secs(5), |r| {
        let m = r.metrics.as_ref();
        m.and_then(|m| m["files_open"].as_u64()) == Some(0)
            && m.and_then(|m| m["files_closed"].as_u64()) == Some(2)
    })
    .await;

    let mut files = files_of(&done);
    files.sort_by_key(|f| f["idx"].as_u64().unwrap());
    let path0 = PathBuf::from(files[0]["path"].as_str().unwrap());
    let path1 = PathBuf::from(files[1]["path"].as_str().unwrap());

    // v1.7(実機一致): 境界を跨いだ f2 も現ファイルに残り(書き込み後判定)、
    // ローテーション直後に開かれた次ファイルは空のまま閉じられる。
    let mut concatenated = std::fs::read(&path0).unwrap();
    concatenated.extend_from_slice(&std::fs::read(&path1).unwrap());
    let mut expected = f1.clone();
    expected.extend_from_slice(&f2);
    assert_eq!(
        concatenated, expected,
        "フレームはファイル間で分割されず、連結すれば一致"
    );
    assert_eq!(
        std::fs::read(&path0).unwrap(),
        expected,
        "idx0 = f1 ++ f2(境界を跨いだフレームは現ファイルに残る)"
    );
    assert_eq!(
        std::fs::read(&path1).unwrap(),
        Vec::<u8>::new(),
        "idx1 はローテーション直後の空ファイル"
    );

    shutdown_and_join(shutdown, task).await;
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------
// (d) seq ギャップ → Error / EOS 前の run 変更 → Error
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sequence_gap_latches_error_and_counts_it() {
    let root = temp_root("seq-gap");
    let params = test_params(root.clone(), vec![0], 10 * 1024 * 1024);
    let (cmd_ep, shutdown, task) = start_graw_writer(params).await;

    let pull_bind = configure_arm(&cmd_ep, 1).await;
    let ctx = zmq::Context::new();
    let push = connect_push(&ctx, &pull_bind);
    rpc(&cmd_ep, &Command::Start { run_number: 1 }).await;

    send_batch(&push, 0, 1, 0, &[make_frame(0, 4, 0x01)]);
    send_batch(&push, 0, 1, 2, &[make_frame(0, 4, 0x02)]); // seq=1 を飛ばす(ギャップ)

    let errored = poll_until(&cmd_ep, &Command::GetStatus, Duration::from_secs(5), |r| {
        r.state == ComponentState::Error
    })
    .await;
    assert_eq!(metric_u64(&errored, "seq_gaps"), 1);

    shutdown_and_join(shutdown, task).await;
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn run_number_change_before_eos_latches_error_and_counts_it() {
    let root = temp_root("run-mismatch");
    let params = test_params(root.clone(), vec![0], 10 * 1024 * 1024);
    let (cmd_ep, shutdown, task) = start_graw_writer(params).await;

    let pull_bind = configure_arm(&cmd_ep, 9).await;
    let ctx = zmq::Context::new();
    let push = connect_push(&ctx, &pull_bind);
    rpc(&cmd_ep, &Command::Start { run_number: 9 }).await;

    send_batch(&push, 0, 9, 0, &[make_frame(0, 4, 0x03)]);
    send_batch(&push, 0, 10, 1, &[make_frame(0, 4, 0x04)]); // run_number が Start と食い違う(EOS 前)

    let errored = poll_until(&cmd_ep, &Command::GetStatus, Duration::from_secs(5), |r| {
        r.state == ComponentState::Error
    })
    .await;
    assert_eq!(metric_u64(&errored, "run_mismatches"), 1);

    shutdown_and_join(shutdown, task).await;
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------
// (e) Configure→Arm→Start→Stop の全シーケンス
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn configure_arm_start_stop_sequence_succeeds() {
    let root = temp_root("full-sequence");
    let params = test_params(root.clone(), vec![0], 10 * 1024 * 1024);
    let (cmd_ep, shutdown, task) = start_graw_writer(params).await;

    let cfg = rpc(
        &cmd_ep,
        &Command::Configure(RunConfig {
            run_number: 42,
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

    let started = rpc(&cmd_ep, &Command::Start { run_number: 42 }).await;
    assert!(started.success, "{}", started.message);
    assert_eq!(started.state, ComponentState::Running);

    let stopped = rpc(&cmd_ep, &Command::Stop).await;
    assert!(stopped.success, "{}", stopped.message);
    assert_eq!(stopped.state, ComponentState::Configured);

    shutdown_and_join(shutdown, task).await;
    let _ = std::fs::remove_dir_all(&root);
}
