//! 実 .graw リプレイ回帰(TODO/006、SPEC §12-1)。
//!
//! `TPCDAQ_REAL_GRAW` に実ファイルのパスが入っているときだけ走る任意テスト
//! (実 .graw はリポに入れない — CLAUDE.md)。graw_replay バイナリで**全速**リプレイし、
//! receiver の両リンクへ出たバイト列がファイルと完全一致し、overflow が 0 であることを確かめる。
//!
//! オラクル(004 / 発注書 006): フレーム 109(CoBo 108 + 制御 1)、byte 完全一致、overflow 0。
//!
//! 実行: `TPCDAQ_REAL_GRAW=/path/to/CoBo_....graw cargo test --test receiver_real_graw -- --nocapture`

#![allow(clippy::unwrap_used)]

use std::process::Command as ProcessCommand;
use std::time::{Duration, Instant};

use tokio::sync::{broadcast, oneshot};
use tpcdaq::command::{Command, CommandResponse, RunConfig};
use tpcdaq::msg::{Message, RawFrames};
use tpcdaq::receiver::{run_receiver, ReceiverParams};
use tpcdaq::zmq_helper;

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

fn metric_u64(resp: &CommandResponse, key: &str) -> u64 {
    resp.metrics.as_ref().unwrap()[key]
        .as_u64()
        .unwrap_or_else(|| panic!("metrics.{key} is not a number: {:?}", resp.metrics))
}

/// PULL を EOS まで読み切り、(連結バイト列, フレーム数, バッチ数) を返す。
/// sequence_number の連続性もここで検証する(ロスレスリンクの取りこぼし検出、SPEC §2.2)。
fn drain_link(sock: zmq::Socket, link: &'static str, run_number: u32) -> (Vec<u8>, usize, usize) {
    let mut bytes = Vec::new();
    let mut frames = 0usize;
    let mut batches = 0usize;
    loop {
        let raw = sock
            .recv_bytes(0)
            .unwrap_or_else(|e| panic!("{link}: PULL recv failed/timed out: {e}"));
        match Message::<RawFrames>::from_msgpack(&raw).unwrap() {
            Message::Data(batch) => {
                assert_eq!(batch.run_number, run_number, "{link}: run_number");
                assert_eq!(
                    batch.sequence_number, batches as u64,
                    "{link}: sequence_number が不連続 = 取りこぼし"
                );
                batches += 1;
                for frame in batch.payload {
                    frames += 1;
                    bytes.extend_from_slice(&frame);
                }
            }
            Message::EndOfStream { run_number: r, .. } => {
                assert_eq!(r, run_number, "{link}: EOS の run_number");
                return (bytes, frames, batches);
            }
            Message::Heartbeat { .. } => {}
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_graw_replay_is_byte_identical_on_both_links() {
    let Ok(path) = std::env::var("TPCDAQ_REAL_GRAW") else {
        eprintln!("SKIP: TPCDAQ_REAL_GRAW が未設定(実 .graw はローカルのみ)");
        return;
    };
    let expected = std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
    println!("real .graw: {path} ({} bytes)", expected.len());

    let ctx = zmq::Context::new();
    let (graw_pull, graw_ep) = bind_pull(&ctx);
    let (decoder_pull, decoder_ep) = bind_pull(&ctx);

    // 実運用の既定値そのまま(8 MiB / 10 ms / HWM 1000)で走らせる
    let params = ReceiverParams {
        cobo_id: 0,
        listen: "127.0.0.1:0".to_string(),
        command_listen: "tcp://127.0.0.1:0".to_string(),
        graw_writer_endpoint: graw_ep,
        decoder_endpoint: decoder_ep,
        batch_max_bytes: tpcdaq::config::DEFAULT_BATCH_MAX_BYTES,
        batch_max_ms: tpcdaq::config::DEFAULT_BATCH_MAX_MS,
        queue_frames: tpcdaq::config::DEFAULT_QUEUE_FRAMES,
        heartbeat_ms: tpcdaq::config::DEFAULT_HEARTBEAT_MS,
        hwm: zmq_helper::DEFAULT_HWM,
    };

    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    let (ep_tx, ep_rx) = oneshot::channel();
    let task = tokio::spawn(run_receiver(params, shutdown_rx, Some(ep_tx)));
    let cmd_ep = ep_rx.await.unwrap();

    const RUN: u32 = 1;
    rpc(
        &cmd_ep,
        &Command::Configure(RunConfig {
            run_number: RUN,
            comment: "real graw replay".to_string(),
            config: serde_json::Value::Null,
        }),
    )
    .await;
    let armed = rpc(&cmd_ep, &Command::Arm).await;
    let data_addr = armed.metrics.as_ref().unwrap()["bind_address"]
        .as_str()
        .unwrap()
        .to_string();
    rpc(&cmd_ep, &Command::Start { run_number: RUN }).await;

    // 両リンクを同時に drain する(片側だけ読むと HWM で送信側が止まる)
    let graw_reader = std::thread::spawn(move || drain_link(graw_pull, "graw-writer", RUN));
    let decoder_reader = std::thread::spawn(move || drain_link(decoder_pull, "decoder", RUN));

    let started = Instant::now();
    let mut child = ProcessCommand::new(env!("CARGO_BIN_EXE_graw_replay"))
        .arg(&data_addr)
        .arg(&path)
        .spawn()
        .expect("spawn graw_replay");
    let status = child.wait().expect("wait for graw_replay");
    assert!(status.success(), "graw_replay failed: {status:?}");

    let (graw_bytes, graw_frames, graw_batches) = graw_reader.join().unwrap();
    let (decoder_bytes, decoder_frames, decoder_batches) = decoder_reader.join().unwrap();
    let elapsed = started.elapsed();

    let stopped = rpc(&cmd_ep, &Command::Stop).await;
    assert!(stopped.success, "{}", stopped.message);

    println!(
        "replay: {:.3} s, frames graw-writer={graw_frames} decoder={decoder_frames}, \
         batches graw-writer={graw_batches} decoder={decoder_batches}, \
         metrics bytes={} frames={} batches={} overflow_frames={} framer_resets={}",
        elapsed.as_secs_f64(),
        metric_u64(&stopped, "bytes"),
        metric_u64(&stopped, "frames"),
        metric_u64(&stopped, "batches"),
        metric_u64(&stopped, "overflow_frames"),
        metric_u64(&stopped, "framer_resets"),
    );

    // オラクル: byte 完全一致 / フレーム 109(CoBo 108 + 制御 1)/ 溢れ 0
    assert_eq!(graw_bytes.len(), expected.len(), "graw-writer: バイト数");
    assert!(graw_bytes == expected, "graw-writer: バイト内容が不一致");
    assert_eq!(decoder_bytes.len(), expected.len(), "decoder: バイト数");
    assert!(decoder_bytes == expected, "decoder: バイト内容が不一致");
    assert_eq!(graw_frames, 109, "graw-writer: フレーム数");
    assert_eq!(decoder_frames, 109, "decoder: フレーム数");
    assert_eq!(metric_u64(&stopped, "frames"), 109);
    assert_eq!(metric_u64(&stopped, "bytes"), expected.len() as u64);
    assert_eq!(metric_u64(&stopped, "overflow_frames"), 0, "溢れ 0");
    assert_eq!(metric_u64(&stopped, "framer_resets"), 0, "framer reset 0");

    shutdown_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("receiver did not stop within 5 s")
        .unwrap();
}
