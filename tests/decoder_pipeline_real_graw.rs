//! decoder の実 .graw E2E(TODO/009、SPEC §12-1 の P1 オラクル)。
//!
//! `TPCDAQ_REAL_GRAW` に実ファイルのパスが入っているときだけ走る任意テスト
//! (実 .graw はリポに入れない — CLAUDE.md)。graw_replay(全速)→ receiver(006)→ decoder の
//! 全経路を実配線し、テスト側 PULL(= root-sink の代役)で全 Fragments を受けて
//! **events(distinct event_idx)= 108 / items 合計 = 15,040,512 / malformed = 0 /
//! unsupported = 1** を照合する。
//!
//! オラクルの出自: C++ 版 tpcdaq(events=108 / items=15,040,512)+ `tests/decoder_real_graw.rs`
//! (同じ実ファイルを framer+decode の純コアで通したときの unsupported=1 = 実 2025 run 先頭の
//! frameType 7・12 B 制御フレーム)。純コアの値がコンポーネント経路(ZMQ・バッチ詰め・
//! EOS 集約)を通しても一致することが、本テストの主張。
//!
//! 実行: `TPCDAQ_REAL_GRAW=/path/to/CoBo_....graw cargo test --test decoder_pipeline_real_graw -- --nocapture`

#![allow(clippy::unwrap_used)]

use std::collections::HashSet;
use std::process::Command as ProcessCommand;
use std::time::{Duration, Instant};

use tokio::sync::{broadcast, oneshot};
use tpcdaq::command::{Command, CommandResponse, ComponentState, RunConfig};
use tpcdaq::decoder::{run_decoder, DecoderParams};
use tpcdaq::msg::{Fragments, Message, RawFrames};
use tpcdaq::receiver::{run_receiver, ReceiverParams};
use tpcdaq::zmq_helper;

/// decoder の source_id(SPEC §3.2)。
const DECODER_SOURCE_ID: u32 = 100;

fn bind_pull(ctx: &zmq::Context, timeout_ms: i32) -> (zmq::Socket, String) {
    let sock = ctx.socket(zmq::PULL).unwrap();
    sock.set_linger(0).unwrap();
    zmq_helper::apply_pull_hwm(&sock).unwrap();
    sock.bind("tcp://127.0.0.1:0").unwrap();
    sock.set_rcvtimeo(timeout_ms).unwrap();
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

async fn configure_arm_start(endpoint: &str, run: u32, comment: &str) -> String {
    rpc(
        endpoint,
        &Command::Configure(RunConfig {
            run_number: run,
            comment: comment.to_string(),
            config: serde_json::Value::Null,
        }),
    )
    .await;
    let armed = rpc(endpoint, &Command::Arm).await;
    assert!(armed.success, "Arm failed: {}", armed.message);
    armed.metrics.as_ref().unwrap()["bind_address"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_graw_replayed_through_receiver_and_decoder_matches_the_p1_oracle() {
    let Ok(path) = std::env::var("TPCDAQ_REAL_GRAW") else {
        eprintln!("SKIP: TPCDAQ_REAL_GRAW が未設定(実 .graw はローカルのみ)");
        return;
    };
    let source_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    println!("real .graw: {path} ({source_bytes} bytes)");

    const RUN: u32 = 1;
    let ctx = zmq::Context::new();

    // --- root-sink の代役(テスト側 PULL)。decoder より先に bind しておく ---
    let (fragment_pull, fragment_ep) = bind_pull(&ctx, 60_000);

    // --- decoder 起動 + Configure/Arm/Start ---
    let decoder_params = DecoderParams {
        pull_bind: "tcp://127.0.0.1:0".to_string(),
        push_connect: fragment_ep,
        command_listen: "tcp://127.0.0.1:0".to_string(),
        batch_max_bytes: tpcdaq::config::DEFAULT_BATCH_MAX_BYTES,
        batch_max_ms: tpcdaq::config::DEFAULT_BATCH_MAX_MS,
        heartbeat_ms: tpcdaq::config::DEFAULT_HEARTBEAT_MS,
        send_timeout_ms: tpcdaq::config::DEFAULT_DECODER_SEND_TIMEOUT_MS,
        workers: 1,
        expected_sources: vec![0],
    };
    let (dec_shutdown_tx, dec_shutdown_rx) = broadcast::channel(1);
    let (dec_ep_tx, dec_ep_rx) = oneshot::channel();
    let dec_task = tokio::spawn(run_decoder(
        decoder_params,
        dec_shutdown_rx,
        Some(dec_ep_tx),
    ));
    let dec_cmd_ep = dec_ep_rx
        .await
        .expect("decoder never reported its command endpoint");
    let decoder_pull_bind = configure_arm_start(&dec_cmd_ep, RUN, "decoder real graw e2e").await;
    let started = rpc(&dec_cmd_ep, &Command::Start { run_number: RUN }).await;
    assert!(started.success, "{}", started.message);

    // --- graw-writer 行きはただ drain するだけ(HWM ブロック回避。007 E2E の流儀) ---
    let (writer_pull, writer_ep) = bind_pull(&ctx, 60_000);
    let writer_drain = std::thread::spawn(move || {
        while let Ok(raw) = writer_pull.recv_bytes(0) {
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
        graw_writer_endpoint: writer_ep,
        decoder_endpoint: decoder_pull_bind,
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
    let data_addr = configure_arm_start(&recv_cmd_ep, RUN, "decoder real graw e2e").await;
    rpc(&recv_cmd_ep, &Command::Start { run_number: RUN }).await;

    // --- graw_replay(全速、ペーシングなし) ---
    let started_at = Instant::now();
    let mut child = ProcessCommand::new(env!("CARGO_BIN_EXE_graw_replay"))
        .arg(&data_addr)
        .arg(&path)
        .spawn()
        .expect("spawn graw_replay");
    let status = child.wait().expect("wait for graw_replay");
    assert!(status.success(), "graw_replay failed: {status:?}");

    // --- 全 Fragments を受ける(decoder 自身の EOS が終端)---
    let mut events: HashSet<u32> = HashSet::new();
    let mut fragments = 0u64;
    let mut items = 0u64;
    let mut batches = 0u64;
    let mut heartbeats = 0u64;
    let mut eos_count = 0u64;
    let mut next_seq = 0u64;
    let mut cobos: HashSet<u8> = HashSet::new();
    loop {
        let raw = fragment_pull
            .recv_bytes(0)
            .expect("no Fragments within 60 s");
        match Message::<Fragments>::from_msgpack(&raw).unwrap() {
            Message::Data(batch) => {
                assert_eq!(
                    batch.source_id, DECODER_SOURCE_ID,
                    "decoder は単一ストリーム(SPEC §2.3)"
                );
                assert_eq!(batch.run_number, RUN);
                assert_eq!(batch.sequence_number, next_seq, "自前 seq は 0 から連続");
                next_seq += 1;
                batches += 1;
                for fragment in &batch.payload {
                    events.insert(fragment.event_idx);
                    cobos.insert(fragment.cobo);
                    items += (fragment.items.len() / 4) as u64;
                    fragments += 1;
                }
            }
            Message::EndOfStream {
                source_id,
                run_number,
            } => {
                assert_eq!(source_id, DECODER_SOURCE_ID);
                assert_eq!(run_number, RUN);
                eos_count += 1;
                break;
            }
            Message::Heartbeat { .. } => heartbeats += 1,
        }
    }
    let elapsed = started_at.elapsed();

    // EOS の後には何も来ない(自分の EOS はちょうど 1 本 — SPEC §2.3)。
    fragment_pull.set_rcvtimeo(500).unwrap();
    while let Ok(raw) = fragment_pull.recv_bytes(0) {
        match Message::<Fragments>::from_msgpack(&raw).unwrap() {
            Message::EndOfStream { .. } => eos_count += 1,
            Message::Data(batch) => panic!("EOS の後に Data が来た: seq={}", batch.sequence_number),
            Message::Heartbeat { .. } => heartbeats += 1,
        }
    }

    let status = rpc(&dec_cmd_ep, &Command::GetStatus).await;
    let metric = |key: &str| status.metrics.as_ref().unwrap()[key].as_u64();
    println!(
        "replay+decode: {:.3} s, batches={batches}, fragments={fragments}, \
         events(distinct event_idx)={}, items={items}, heartbeats={heartbeats}, eos={eos_count}, \
         cobos={cobos:?}, metrics: frames_in={:?} fragments_out={:?} items_out={:?} \
         malformed={:?} unsupported={:?} seq_gaps={:?} run_mismatches={:?} \
         batches_abandoned={:?} eos_abandoned={:?}",
        elapsed.as_secs_f64(),
        events.len(),
        metric("frames_in"),
        metric("fragments_out"),
        metric("items_out"),
        metric("malformed"),
        metric("unsupported"),
        metric("seq_gaps"),
        metric("run_mismatches"),
        metric("batches_abandoned"),
        metric("eos_abandoned"),
    );

    // --- P1 オラクル(SPEC §12-1 / tests/decoder_real_graw.rs と同一の値)---
    assert_eq!(events.len(), 108, "events(distinct event_idx)オラクル");
    assert_eq!(items, 15_040_512, "items オラクル");
    assert_eq!(metric("malformed"), Some(0), "malformed オラクル");
    assert_eq!(
        metric("unsupported"),
        Some(1),
        "unsupported オラクル(実 2025 run 先頭の frameType 7 制御フレーム ×1)"
    );
    // 経路の健全性(ロスレス系として当然満たすべき値)
    assert_eq!(eos_count, 1, "自分の EOS はちょうど 1 本");
    assert_eq!(
        fragments, 108,
        "mini は 1 CoBo × 1 AsAd なので 1 event = 1 frame"
    );
    assert_eq!(cobos.len(), 1, "mini は CoBo 1 台");
    assert_eq!(metric("fragments_out"), Some(fragments));
    assert_eq!(metric("items_out"), Some(items));
    assert_eq!(
        metric("frames_in"),
        Some(109),
        "108 データ + 1 制御フレーム"
    );
    assert_eq!(metric("seq_gaps"), Some(0));
    assert_eq!(metric("run_mismatches"), Some(0));
    assert_eq!(metric("batches_abandoned"), Some(0));
    assert_eq!(metric("eos_abandoned"), Some(0));
    assert_ne!(
        status.state,
        ComponentState::Error,
        "unsupported だけでは Error にしない(SPEC v1.2 §7)"
    );

    let _ = recv_shutdown_tx.send(());
    tokio::time::timeout(Duration::from_secs(5), recv_task)
        .await
        .expect("receiver did not stop within 5 s")
        .unwrap();
    let _ = dec_shutdown_tx.send(());
    tokio::time::timeout(Duration::from_secs(5), dec_task)
        .await
        .expect("decoder did not stop within 5 s")
        .unwrap();
    let _ = writer_drain.join();
}
