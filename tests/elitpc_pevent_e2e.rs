//! ELITPC 同 run 実データ E2E(TODO/021、SPEC §12-3 v1.8 ③)。
//!
//! 実 ELITPC の graw 4 本組(1 論理 CoBo × 4 AsAd)を graw_replay の eventIdx マージ送出で
//! フルチェーン(receiver → decoder → root_sink)に通し、出来た run.root を
//! **同一 run を実機側 grawToEventTPC が変換した PEventTPC ファイル**と compare_pevent で
//! 全イベント突き合わせる。これが「我々の出力 = 先方のオフライン変換出力」の最終照合。
//!
//! 必要 env(すべて揃わないと skip — 実データはローカルのみ、CLAUDE.md):
//!   TPCDAQ_ROOT_SINK_BIN          = tools/root_sink/root_sink(compare_pevent も同じ場所)
//!   TPCDAQ_REAL_GRAW_DIR          = CoBo0_AsAd{0..3}_*_0000.graw の 4 本があるディレクトリ
//!   TPCDAQ_REAL_PEVENT            = 同 run の PEventTPC_*_0000.root(読み取り専用で開く)
//!   TPCDAQ_REAL_GEOMETRY_ELITPC   = 実 ELITPC ジオメトリ .dat
//!   TPCDAQ_ELITPC_RATE_MBPS      = 任意(既定 40)。root_sink の chargeMap 構築 + ZLIB が
//!                                   ボトルネックなので、receiver の有界キューを溢れさせない
//!                                   ペースで送る(溢れ = overflow カウンタで検出される)。
//!
//! 実行例(4.3 GiB を 40 Mbps で流すので **20 分超**かかる。--release 必須):
//! make -C tools/root_sink && make -C tools/root_sink compare_pevent
//! TPCDAQ_ROOT_SINK_BIN=$PWD/tools/root_sink/root_sink \
//! TPCDAQ_REAL_GRAW_DIR=reference/exp_data/2026 \
//! TPCDAQ_REAL_PEVENT=reference/exp_data/2026/PEventTPC_2026-08-11T07-47-37.051_0000.root \
//! TPCDAQ_REAL_GEOMETRY_ELITPC=reference/TPCReco/TPCReco-HIGS2026_online/resources/geometry_ELITPC.dat \
//! cargo test --release --test elitpc_pevent_e2e -- --nocapture

#![allow(clippy::unwrap_used)]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use tokio::sync::{broadcast, oneshot};
use tpcdaq::command::{Command as DaqCommand, CommandResponse, RunConfig};
use tpcdaq::decoder::{run_decoder, DecoderParams};
use tpcdaq::msg::{Message, RawFrames};
use tpcdaq::receiver::{run_receiver, ReceiverParams};
use tpcdaq::zmq_helper;

const RUN: u32 = 1;
/// 実測普遍値(TODO/019): フル読み出し ELITPC の _0000 4 本組。
const EVENTS: u64 = 3852;
const FRAGMENTS: u64 = EVENTS * 4;
const ITEMS: u64 = 536_444_928 * 4;

// ---------------------------------------------------------------------
// 共通ヘルパ(tests/root_sink_intake.rs の流儀をそのまま)
// ---------------------------------------------------------------------

fn free_endpoint() -> String {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe listener");
    let port = probe.local_addr().expect("local_addr").port();
    drop(probe);
    format!("tcp://127.0.0.1:{port}")
}

fn send_term(pid: u32) {
    let status = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()
        .expect("run kill(1)");
    assert!(status.success(), "kill -TERM {pid} failed");
}

fn wait_for_exit(child: &mut Child, timeout: Duration) -> ExitStatus {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("root_sink did not exit within {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn read_counts(child: &mut Child) -> serde_json::Value {
    let mut out = String::new();
    child
        .stdout
        .take()
        .expect("piped stdout")
        .read_to_string(&mut out)
        .expect("read stdout");
    let line = out.lines().last().unwrap_or_default();
    serde_json::from_str(line)
        .unwrap_or_else(|e| panic!("stdout is not one JSON line: {out:?} ({e})"))
}

fn count(counts: &serde_json::Value, key: &str) -> u64 {
    counts[key]
        .as_u64()
        .unwrap_or_else(|| panic!("counter {key} missing in {counts}"))
}

async fn rpc(endpoint: &str, cmd: &DaqCommand) -> CommandResponse {
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

async fn configure_arm(endpoint: &str, run: u32, comment: &str) -> String {
    rpc(
        endpoint,
        &DaqCommand::Configure(RunConfig {
            run_number: run,
            comment: comment.to_string(),
            config: serde_json::Value::Null,
        }),
    )
    .await;
    let armed = rpc(endpoint, &DaqCommand::Arm).await;
    assert!(armed.success, "Arm failed: {}", armed.message);
    armed.metrics.as_ref().unwrap()["bind_address"]
        .as_str()
        .unwrap()
        .to_string()
}

fn bind_pull(ctx: &zmq::Context, timeout_ms: i32) -> (zmq::Socket, String) {
    let sock = ctx.socket(zmq::PULL).unwrap();
    sock.set_linger(0).unwrap();
    zmq_helper::apply_pull_hwm(&sock).unwrap();
    sock.bind("tcp://127.0.0.1:0").unwrap();
    sock.set_rcvtimeo(timeout_ms).unwrap();
    let endpoint = sock.get_last_endpoint().unwrap().unwrap();
    (sock, endpoint)
}

fn wait_for_file(path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.is_file() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

// ---------------------------------------------------------------------
// 入力の解決
// ---------------------------------------------------------------------

/// 4 本組を AsAd 昇順で返す(TODO/019 の回帰と同じ規則)。
fn real_graw_files(dir: &str) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("cannot read {dir}: {e}"))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "graw"))
        .collect();
    paths.sort();
    paths
}

/// runId = 先頭ファイル(AsAd0)名の TS を `%Y%m%d%H%M%S` に潰した long
/// (TPCReco `RunIdParser` と同じ導出。ms は落とす)。
/// 例: `CoBo0_AsAd0_2026-08-11T07:47:37.043_0000.graw` → 20260811074737。
fn run_id_from_filename(path: &Path) -> u64 {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap();
    let ts = name.split('_').nth(2).unwrap_or_else(|| {
        panic!("not a DataRouter name: {name}");
    });
    let seconds = ts.split('.').next().unwrap();
    let digits: String = seconds.chars().filter(|c| c.is_ascii_digit()).collect();
    assert_eq!(
        digits.len(),
        14,
        "TS は %Y%m%d%H%M%S の 14 桁のはず: {name}"
    );
    digits.parse().unwrap()
}

// ---------------------------------------------------------------------
// 本体
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn same_run_pevent_output_matches_the_real_grawtoeventtpc_file() {
    let Ok(sink_bin) = std::env::var("TPCDAQ_ROOT_SINK_BIN") else {
        eprintln!("SKIP: TPCDAQ_ROOT_SINK_BIN が未設定(make -C tools/root_sink)");
        return;
    };
    let Ok(graw_dir) = std::env::var("TPCDAQ_REAL_GRAW_DIR") else {
        eprintln!("SKIP: TPCDAQ_REAL_GRAW_DIR が未設定(実 .graw はローカルのみ)");
        return;
    };
    let Ok(oracle) = std::env::var("TPCDAQ_REAL_PEVENT") else {
        eprintln!("SKIP: TPCDAQ_REAL_PEVENT が未設定(同 run の実機変換 .root)");
        return;
    };
    let Ok(geometry) = std::env::var("TPCDAQ_REAL_GEOMETRY_ELITPC") else {
        eprintln!("SKIP: TPCDAQ_REAL_GEOMETRY_ELITPC が未設定(実 .dat はローカルのみ)");
        return;
    };
    let sink_bin = PathBuf::from(sink_bin);
    // 比較ツールは root_sink と同じディレクトリ。無ければ**落とす**(黙って比較を
    // 省略した green を作らない — p2_e2e の compare_gdataframe と同じ流儀)。
    let compare = sink_bin.with_file_name("compare_pevent");
    assert!(
        compare.is_file(),
        "{} が無い。`make -C tools/root_sink compare_pevent` でビルドすること",
        compare.display()
    );

    let files = real_graw_files(&graw_dir);
    assert_eq!(files.len(), 4, "ELITPC は 1 CoBo × 4 AsAd = 4 ファイル");
    let run_id = run_id_from_filename(&files[0]);
    let rate_mbps = std::env::var("TPCDAQ_ELITPC_RATE_MBPS").unwrap_or_else(|_| "40".to_string());

    let out_root = {
        let dir = std::env::temp_dir().join(format!("tpcdaq-elitpc-pevent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    };
    let ctx = zmq::Context::new();

    // --- root_sink(本物のプロセス)を先に上げる。listen-before-start と同じ理屈 ---
    let sink_ep = free_endpoint();
    let mut sink = Command::new(&sink_bin)
        .arg("--bind")
        .arg(&sink_ep)
        .arg("--output-root")
        .arg(&out_root)
        .arg("--expect")
        .arg("0:0,0:1,0:2,0:3")
        .arg("--geometry")
        .arg(&geometry)
        .arg("--run-id")
        .arg(run_id.to_string())
        // run 毎単一ファイルで比較する(ロールオーバさせない)。
        .arg("--max-root-bytes")
        .arg("200000000000")
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn root_sink");
    let sink_pid = sink.id();
    std::thread::sleep(Duration::from_millis(300));

    // --- decoder: PUSH 先は root_sink ---
    let decoder_params = DecoderParams {
        pull_bind: "tcp://127.0.0.1:0".to_string(),
        push_connect: sink_ep.clone(),
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
    let dec_cmd_ep = dec_ep_rx.await.expect("decoder command endpoint");
    let decoder_pull_bind = configure_arm(&dec_cmd_ep, RUN, "elitpc pevent e2e").await;
    let started = rpc(&dec_cmd_ep, &DaqCommand::Start { run_number: RUN }).await;
    assert!(started.success, "{}", started.message);

    // --- graw-writer 行きは drain するだけ(intake E2E の流儀) ---
    let (writer_pull, writer_ep) = bind_pull(&ctx, 600_000);
    let writer_drain = std::thread::spawn(move || {
        while let Ok(raw) = writer_pull.recv_bytes(0) {
            if let Ok(Message::<RawFrames>::EndOfStream { .. }) =
                Message::<RawFrames>::from_msgpack(&raw)
            {
                break;
            }
        }
    });

    // --- receiver(cobo 0 のみ — 実機どおり 1 論理 CoBo) ---
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
    let recv_cmd_ep = recv_ep_rx.await.expect("receiver command endpoint");
    let data_addr = configure_arm(&recv_cmd_ep, RUN, "elitpc pevent e2e").await;
    rpc(&recv_cmd_ep, &DaqCommand::Start { run_number: RUN }).await;

    // --- graw_replay: 4 本組を eventIdx マージ送出(TODO/021)。ペーシング必須 ---
    // root_sink の chargeMap 構築 + ZLIB が下流ボトルネックで、receiver は never-stop
    // (TCP は止めない)なので、全速では有界キューが溢れる(溢れ = overflow で可視)。
    let started_at = Instant::now();
    let mut replay = Command::new(env!("CARGO_BIN_EXE_graw_replay"))
        .arg(&data_addr)
        .args(&files)
        .arg("--rate-mbps")
        .arg(&rate_mbps)
        .spawn()
        .expect("spawn graw_replay");
    let replay_status = replay.wait().expect("wait for graw_replay");
    assert!(
        replay_status.success(),
        "graw_replay failed: {replay_status:?}"
    );
    eprintln!(
        "replayed 4 files at {rate_mbps} Mbps in {:.1}s",
        started_at.elapsed().as_secs_f64()
    );

    // --- finalize(rename 完了)を待つ。3852 イベント分の flush + ZLIB があるので長め ---
    let run_file = out_root.join(format!("run{RUN:04}/run{RUN:04}.root"));
    assert!(
        wait_for_file(&run_file, Duration::from_secs(600)),
        "{} が 600 s 以内に出来なかった",
        run_file.display()
    );
    eprintln!("finalized after {:.1}s", started_at.elapsed().as_secs_f64());

    // --- receiver の overflow を先に確認(ペーシング不足の検出を比較より先に) ---
    let recv_status = rpc(&recv_cmd_ep, &DaqCommand::GetStatus).await;
    let overflow = recv_status.metrics.as_ref().unwrap()["overflow_frames"]
        .as_u64()
        .unwrap_or(u64::MAX);
    assert_eq!(
        overflow, 0,
        "receiver queue overflow — TPCDAQ_ELITPC_RATE_MBPS={rate_mbps} を下げること"
    );

    // --- root_sink のカウンタ(SPEC §12-1 の ELITPC 実測普遍値) ---
    send_term(sink_pid);
    let sink_status = wait_for_exit(&mut sink, Duration::from_secs(60));
    assert!(sink_status.success(), "root_sink: {sink_status:?}");
    let counts = read_counts(&mut sink);
    assert_eq!(count(&counts, "fragments"), FRAGMENTS, "counts={counts}");
    assert_eq!(count(&counts, "items"), ITEMS, "counts={counts}");
    assert_eq!(count(&counts, "events_complete"), EVENTS, "counts={counts}");
    assert_eq!(count(&counts, "events_incomplete"), 0, "counts={counts}");
    assert_eq!(count(&counts, "late_fragments"), 0, "counts={counts}");
    assert_eq!(count(&counts, "entries_written"), EVENTS, "counts={counts}");
    assert_eq!(count(&counts, "duplicate_event_ids"), 0, "counts={counts}");
    assert_eq!(
        count(&counts, "charge_keys_out_of_range"),
        0,
        "counts={counts}"
    );
    assert_eq!(
        count(&counts, "frames_outside_geometry"),
        0,
        "counts={counts}"
    );
    assert_eq!(counts["fatal"], "", "counts={counts}");
    eprintln!(
        "channels_without_strip={} (FPN 以外の非 strip ch × フレーム数 — 参考値)",
        count(&counts, "channels_without_strip")
    );

    // --- 最終照合: compare_pevent で実機 grawToEventTPC 出力と全イベント突き合わせ ---
    let compare_at = Instant::now();
    let output = Command::new(&compare)
        .arg(&run_file)
        .arg(&oracle)
        .output()
        .expect("run compare_pevent");
    let stdout = String::from_utf8_lossy(&output.stdout);
    eprintln!(
        "--- compare_pevent ({:.1}s) ---\n{stdout}",
        compare_at.elapsed().as_secs_f64()
    );
    assert!(
        output.status.success(),
        "我々の PEventTPC 出力が実機 grawToEventTPC 出力と一致しない(SPEC §12-3 v1.8 ③)"
    );

    // --- 後始末 ---
    let _ = recv_shutdown_tx.send(());
    let _ = tokio::time::timeout(Duration::from_secs(5), recv_task).await;
    let _ = dec_shutdown_tx.send(());
    let _ = tokio::time::timeout(Duration::from_secs(5), dec_task).await;
    let _ = writer_drain.join();
    let _ = std::fs::remove_dir_all(&out_root);
}
