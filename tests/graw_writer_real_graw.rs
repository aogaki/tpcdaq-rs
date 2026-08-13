//! graw-writer の実 .graw E2E(TODO/007、SPEC §12-2 v1.2)。
//!
//! `TPCDAQ_REAL_GRAW` に実ファイルのパスが入っているときだけ走る任意テスト
//! (実 .graw はリポに入れない — CLAUDE.md)。graw_replay(全速)→ receiver(006)→ graw-writer の
//! 全経路を実配線し、**per-AsAd 出力 + ctrl 出力の合計が入力の完全ロスレス分割**になることを
//! 確かめる(SPEC §12-2 v1.2)。
//!
//! この実ファイル(2025 run)は先頭に frameType 7・12 B の非 AsAd 制御フレームを 1 本含む
//! (`decoder_real_graw.rs` オラクルの `unsupported=1` と同一物)。v1.2 以前は「mini なら
//! 元ファイルと完全一致」だったが、これはそのような制御フレームが無い場合の言明に過ぎないと
//! 判明したため、v1.2 で ctrl/ サブディレクトリへの保全が規定された(SPEC §7)。
//! オラクル: AsAd ファイル 30,108,672 B + ctrl 12 B(frameType 7 ×1)= 30,108,684 B。
//!
//! 実行: `TPCDAQ_REAL_GRAW=/path/to/CoBo_....graw cargo test --test graw_writer_real_graw -- --nocapture`

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;
use std::process::Command as ProcessCommand;
use std::time::{Duration, Instant};

use tokio::sync::{broadcast, oneshot};
use tpcdaq::command::{Command, CommandResponse, RunConfig};
use tpcdaq::decode::peek_asad;
use tpcdaq::framer::Framer;
use tpcdaq::graw_writer::{run_graw_writer, GrawWriterParams};
use tpcdaq::msg::{Message, RawFrames};
use tpcdaq::receiver::{run_receiver, ReceiverParams};
use tpcdaq::zmq_helper;

/// `bytes` を MFM フレームへ分割し、frameType 1/2 の asadIdx で分別した列(= per-AsAd 出力の
/// オラクル)と、それ以外(非 AsAd 制御フレーム、= ctrl/ 出力のオラクル)の列を分けて返す
/// (SPEC §12-2 v1.2「全出力の合計 = 入力の完全ロスレス分割」)。
fn split_by_asad(bytes: &[u8]) -> (Vec<u8>, Vec<u8>, usize, usize) {
    let mut framer = Framer::new();
    let mut asad_column = Vec::with_capacity(bytes.len());
    let mut ctrl_column = Vec::new();
    let mut asad_frames = 0usize;
    let mut ctrl_frames = 0usize;
    for chunk in bytes.chunks(8192) {
        framer.push(chunk);
        while let Some(frame) = framer.next() {
            if peek_asad(frame).is_some() {
                asad_column.extend_from_slice(frame);
                asad_frames += 1;
            } else {
                ctrl_column.extend_from_slice(frame);
                ctrl_frames += 1;
            }
        }
    }
    (asad_column, ctrl_column, asad_frames, ctrl_frames)
}

fn bind_pull(ctx: &zmq::Context) -> (zmq::Socket, String) {
    let sock = ctx.socket(zmq::PULL).unwrap();
    sock.set_linger(0).unwrap();
    zmq_helper::apply_pull_hwm(&sock).unwrap();
    sock.bind("tcp://127.0.0.1:0").unwrap();
    sock.set_rcvtimeo(60_000).unwrap();
    let endpoint = sock.get_last_endpoint().unwrap().unwrap();
    (sock, endpoint)
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

fn temp_root() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tpcdaq-graw-writer-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_graw_replay_through_receiver_matches_the_source_file_byte_for_byte() {
    let Ok(path) = std::env::var("TPCDAQ_REAL_GRAW") else {
        eprintln!("SKIP: TPCDAQ_REAL_GRAW が未設定(実 .graw はローカルのみ)");
        return;
    };
    let expected = std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
    println!("real .graw: {path} ({} bytes)", expected.len());

    let root = temp_root();
    const RUN: u32 = 1;

    // --- graw-writer 起動 + Configure/Arm/Start ---
    let gw_params = GrawWriterParams {
        pull_bind: "tcp://127.0.0.1:0".to_string(),
        command_listen: "tcp://127.0.0.1:0".to_string(),
        output_root: root.clone(),
        max_file_bytes: tpcdaq::config::DEFAULT_GRAW_WRITER_MAX_FILE_BYTES,
        flush_interval_ms: 200,
        expected_sources: vec![0],
    };
    let (gw_shutdown_tx, gw_shutdown_rx) = broadcast::channel(1);
    let (gw_ep_tx, gw_ep_rx) = oneshot::channel();
    let gw_task = tokio::spawn(run_graw_writer(gw_params, gw_shutdown_rx, Some(gw_ep_tx)));
    let gw_cmd_ep = gw_ep_rx
        .await
        .expect("graw-writer never reported its command endpoint");

    rpc(
        &gw_cmd_ep,
        &Command::Configure(RunConfig {
            run_number: RUN,
            comment: "graw-writer real graw e2e".to_string(),
            config: serde_json::Value::Null,
        }),
    )
    .await;
    let gw_armed = rpc(&gw_cmd_ep, &Command::Arm).await;
    let gw_pull_bind = gw_armed.metrics.as_ref().unwrap()["bind_address"]
        .as_str()
        .unwrap()
        .to_string();
    rpc(&gw_cmd_ep, &Command::Start { run_number: RUN }).await;

    // --- decoder 行きはただ drain するだけ(HWM ブロック回避。006 の流儀)。
    // EndOfStream を見たら抜ける(受信スレッドを無期限にブロックさせない)。
    let ctx = zmq::Context::new();
    let (decoder_pull, decoder_ep) = bind_pull(&ctx);
    let decoder_drain = std::thread::spawn(move || {
        while let Ok(raw) = decoder_pull.recv_bytes(0) {
            if let Ok(Message::<RawFrames>::EndOfStream { .. }) =
                Message::<RawFrames>::from_msgpack(&raw)
            {
                break;
            }
        }
    });

    // --- receiver(006)起動 + Configure/Arm/Start ---
    let recv_params = ReceiverParams {
        cobo_id: 0,
        listen: "127.0.0.1:0".to_string(),
        command_listen: "tcp://127.0.0.1:0".to_string(),
        graw_writer_endpoint: gw_pull_bind,
        decoder_endpoint: decoder_ep,
        batch_max_bytes: tpcdaq::config::DEFAULT_BATCH_MAX_BYTES,
        batch_max_ms: tpcdaq::config::DEFAULT_BATCH_MAX_MS,
        queue_frames: tpcdaq::config::DEFAULT_QUEUE_FRAMES,
        heartbeat_ms: tpcdaq::config::DEFAULT_HEARTBEAT_MS,
        hwm: zmq_helper::DEFAULT_HWM,
    };
    let (recv_shutdown_tx, recv_shutdown_rx) = broadcast::channel(1);
    let (recv_ep_tx, recv_ep_rx) = oneshot::channel();
    let recv_task = tokio::spawn(run_receiver(
        recv_params,
        recv_shutdown_rx,
        Some(recv_ep_tx),
    ));
    let recv_cmd_ep = recv_ep_rx
        .await
        .expect("receiver never reported its command endpoint");

    rpc(
        &recv_cmd_ep,
        &Command::Configure(RunConfig {
            run_number: RUN,
            comment: "graw-writer real graw e2e".to_string(),
            config: serde_json::Value::Null,
        }),
    )
    .await;
    let armed = rpc(&recv_cmd_ep, &Command::Arm).await;
    let data_addr = armed.metrics.as_ref().unwrap()["bind_address"]
        .as_str()
        .unwrap()
        .to_string();
    rpc(&recv_cmd_ep, &Command::Start { run_number: RUN }).await;

    // --- graw_replay(全速、ペーシングなし) ---
    let started = Instant::now();
    let mut child = ProcessCommand::new(env!("CARGO_BIN_EXE_graw_replay"))
        .arg(&data_addr)
        .arg(&path)
        .spawn()
        .expect("spawn graw_replay");
    let status = child.wait().expect("wait for graw_replay");
    assert!(status.success(), "graw_replay failed: {status:?}");

    // receiver: TCP EOF → 両リンクへ EndOfStream(006 の never-stop 経路、明示 Stop 不要)。
    // graw-writer: 期待ソース(cobo 0)全部の EOS 受領 → finalize(flush+fsync+close)。
    // files_open == 0 まで待てば、書き込み・close の完了を確実に観測できる。
    // 2 ファイル = per-AsAd 1(mini = 1 AsAd)+ ctrl 1(frameType 7 の 12 B 制御フレーム、v1.2)。
    let done = poll_until(
        &gw_cmd_ep,
        &Command::GetStatus,
        Duration::from_secs(30),
        |r| {
            let m = r.metrics.as_ref();
            m.and_then(|m| m["files_open"].as_u64()) == Some(0)
                && m.and_then(|m| m["files_closed"].as_u64()) == Some(2)
        },
    )
    .await;
    let elapsed = started.elapsed();

    let files = done.metrics.as_ref().unwrap()["files"].as_array().unwrap();
    assert_eq!(
        files.len(),
        2,
        "mini 実 graw は 1 AsAd + ctrl 1 のはず(SPEC §7/§12-2 v1.2): {files:?}"
    );
    let asad_path = PathBuf::from(
        files
            .iter()
            .find(|f| !f["asad"].is_null())
            .unwrap_or_else(|| panic!("no per-AsAd file in {files:?}"))["path"]
            .as_str()
            .unwrap(),
    );
    let ctrl_path = PathBuf::from(
        files
            .iter()
            .find(|f| f["asad"].is_null())
            .unwrap_or_else(|| panic!("no ctrl file in {files:?}"))["path"]
            .as_str()
            .unwrap(),
    );
    let actual_asad = std::fs::read(&asad_path).unwrap();
    let actual_ctrl = std::fs::read(&ctrl_path).unwrap();

    // SPEC §12-2 v1.2: 「全出力(per-AsAd + ctrl)の合計 = 入力の完全ロスレス分割」。
    let (expected_asad, expected_ctrl, asad_frames, ctrl_frames_expected) =
        split_by_asad(&expected);

    let ctrl_frames_metric = done.metrics.as_ref().unwrap()["ctrl_frames"].as_u64();
    println!(
        "replay: {:.3} s, files=2 (asad={} B, ctrl={} B, total={} B == source {} B), \
         asad_frames={asad_frames}, ctrl_frames={ctrl_frames_metric:?}, \
         seq_gaps={}, write_errors={}",
        elapsed.as_secs_f64(),
        actual_asad.len(),
        actual_ctrl.len(),
        actual_asad.len() + actual_ctrl.len(),
        expected.len(),
        done.metrics.as_ref().unwrap()["seq_gaps"],
        done.metrics.as_ref().unwrap()["write_errors"],
    );

    // per-AsAd 出力(asadIdx で分別した列)
    assert_eq!(actual_asad.len(), expected_asad.len(), "AsAd バイト数");
    assert!(
        actual_asad == expected_asad,
        "AsAd 出力のバイト内容が不一致"
    );
    // ctrl 出力(非 AsAd フレームの列 — v1.2 の新規保全先)
    assert_eq!(actual_ctrl.len(), expected_ctrl.len(), "ctrl バイト数");
    assert!(
        actual_ctrl == expected_ctrl,
        "ctrl 出力のバイト内容が不一致"
    );
    // 全出力の合計 = 入力の完全ロスレス分割(SPEC §12-2 v1.2 の核心オラクル)
    assert_eq!(
        actual_asad.len() + actual_ctrl.len(),
        expected.len(),
        "AsAd + ctrl の合計が元 .graw のバイト数と不一致(ロスレス分割が崩れている)"
    );
    // オラクル値(2026-08-13、この実ファイル固有)
    assert_eq!(expected.len(), 30_108_684, "元 .graw のバイト数オラクル");
    assert_eq!(actual_asad.len(), 30_108_672, "AsAd 出力バイト数オラクル");
    assert_eq!(actual_ctrl.len(), 12, "ctrl 出力バイト数オラクル");
    assert_eq!(asad_frames, 108, "AsAd フレーム数オラクル");
    assert_eq!(
        ctrl_frames_expected, 1,
        "ctrl フレーム数オラクル(frameType 7 ×1)"
    );
    assert_eq!(
        ctrl_frames_metric,
        Some(1),
        "graw-writer 自身の ctrl_frames カウンタもオラクルと一致"
    );
    assert_eq!(done.metrics.as_ref().unwrap()["seq_gaps"].as_u64(), Some(0));
    assert_eq!(
        done.metrics.as_ref().unwrap()["write_errors"].as_u64(),
        Some(0)
    );
    assert_eq!(
        done.metrics.as_ref().unwrap()["run_mismatches"].as_u64(),
        Some(0)
    );
    assert_ne!(
        done.state,
        tpcdaq::command::ComponentState::Error,
        "非 AsAd 制御フレームは Error にしない(SPEC §7 v1.2)"
    );

    let _ = recv_shutdown_tx.send(());
    tokio::time::timeout(Duration::from_secs(5), recv_task)
        .await
        .expect("receiver did not stop within 5 s")
        .unwrap();
    let _ = gw_shutdown_tx.send(());
    tokio::time::timeout(Duration::from_secs(5), gw_task)
        .await
        .expect("graw-writer did not stop within 5 s")
        .unwrap();
    let _ = decoder_drain.join();
    let _ = std::fs::remove_dir_all(&root);
}
