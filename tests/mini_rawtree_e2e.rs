//! mini eTPC raw ツリー E2E(TODO/075、SPEC §6.7 の受け入れ)。
//!
//! 実 mini .graw(2025、1 AsAd、compact)をフルチェーン(receiver → decoder →
//! root_sink `--raw-tree`)に通し、出来た `run{N}_raw.root` を **同じ .graw を
//! macros/graw2root.C が変換した出力**と compare_raw で全エントリ突き合わせる。
//! graw2root.C は**凍結されたオラクル**(契約の現物)—— 一致すれば、下流
//! (analyzeUVW.C → makeTracks.C)は無変更で DAQ 出力に繋がる。
//!
//! 必要 env(すべて揃わないと skip — 実データはローカルのみ、CLAUDE.md):
//!   TPCDAQ_ROOT_SINK_BIN        = tools/root_sink/root_sink(compare_raw も同じ場所)
//!   TPCDAQ_REAL_GRAW            = 2025 mini 実 .graw(108 frames、compact)
//!   TPCDAQ_REAL_GEOMETRY_MINI   = 実 mini ジオメトリ .dat
//!   `root` が PATH にあること(graw2root.C を走らせる)
//!
//! **~/TPC には何も書かない**: .graw は一時 dir に**シンボリックリンク**を張って
//! graw2root.C にはそのリンクを渡す(graw2root.C は入力の隣に `<stem>_raw.root` を書く)。
//!
//! 実行例(Mac は SDKROOT が要る —— cling が libc++ ヘッダを見失う。メモリ
//! mac-root-cling-sdkroot。テストは環境を継承する。**どの SDK かは ROOT の版で変わる**:
//! cling の std_darwin.modulemap が要求するヘッダを持つ最新の CLT SDK を指すこと ——
//! ROOT 6.36 は MacOSX26.sdk、6.40.04 は MacOSX27.sdk で通ることを実測 2026-09-14):
//! make -C tools/root_sink && make -C tools/root_sink compare
//! export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX27.sdk
//! TPCDAQ_ROOT_SINK_BIN=$PWD/tools/root_sink/root_sink \
//! TPCDAQ_REAL_GRAW=$HOME/TPC/CoBo_2025-09-01T08_51_06.203_0000.graw \
//! TPCDAQ_REAL_GEOMETRY_MINI=$HOME/TPC/miniTPC_UVW_pcb_info/new_geometry_mini_eTPC.dat \
//! cargo test --test mini_rawtree_e2e -- --nocapture

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
/// 実測普遍値(TODO/067 と同じ 2025 mini ファイル): 1 AsAd フル読み出し 108 フレーム。
const EVENTS: u64 = 108;
/// EventInfo.runId は raw ツリーには載らないが、PEventTPC 側を壁時計依存にしない
/// ため固定する(ファイル名の TS `2025-09-01T08:51:06` を %Y%m%d%H%M%S に潰した値)。
const FIXED_RUN_ID: &str = "20250901085106";
/// 30 MB を ~2.4 s で流す。mini 1 AsAd なら root_sink が十分追いつくペース。
const REPLAY_RATE_MBPS: &str = "100";

// ---------------------------------------------------------------------
// 共通ヘルパ(tests/elitpc_pevent_e2e.rs の流儀をそのまま)
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

/// PATH 上の実行ファイルを探す(`root` が無ければ SKIP したいだけなので起動はしない)。
fn which(program: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let candidate = dir.join(program);
            if candidate.is_file() {
                Some(candidate)
            } else {
                None
            }
        })
    })
}

/// graw2root.C が書き出すファイル名(macros/graw2root.C:111-117 と同じ規則):
/// ROOT が嫌う ':' を '-' に潰し、末尾 ".graw" を "_raw.root" に置き換える。
fn graw2root_output(input: &Path) -> PathBuf {
    let text = input.to_string_lossy().replace(':', "-");
    let stem = text.strip_suffix(".graw").expect("入力は .graw");
    PathBuf::from(format!("{stem}_raw.root"))
}

/// `root -l -b -q 'macros/graw2root.C("<input>"[, maxEvents])'`。
/// **環境は継承する**(Mac は SDKROOT が要る —— 冒頭の注記)。
fn run_graw2root(input: &Path, max_events: Option<i64>, workdir: &Path) {
    let macro_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("macros/graw2root.C");
    let call = match max_events {
        Some(n) => format!("{}(\"{}\",{n})", macro_path.display(), input.display()),
        None => format!("{}(\"{}\")", macro_path.display(), input.display()),
    };
    let out = Command::new("root")
        .args(["-l", "-b", "-q"])
        .arg(&call)
        .current_dir(workdir)
        .output()
        .expect("spawn root");
    eprintln!(
        "--- root {call} ---\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success(),
        "graw2root.C が失敗した({:?})。Mac は SDKROOT に cling が要求する libc++ を持つ \
         CLT SDK を指すこと(ROOT 6.40 なら .../MacOSX27.sdk。メモリ mac-root-cling-sdkroot)",
        out.status
    );
}

// ---------------------------------------------------------------------
// 本体
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn raw_tree_output_matches_graw2root_on_the_same_mini_graw() {
    let Ok(sink_bin) = std::env::var("TPCDAQ_ROOT_SINK_BIN") else {
        eprintln!("SKIP: TPCDAQ_ROOT_SINK_BIN が未設定(make -C tools/root_sink)");
        return;
    };
    let Ok(graw) = std::env::var("TPCDAQ_REAL_GRAW") else {
        eprintln!("SKIP: TPCDAQ_REAL_GRAW が未設定(実 .graw はローカルのみ)");
        return;
    };
    let Ok(geometry) = std::env::var("TPCDAQ_REAL_GEOMETRY_MINI") else {
        eprintln!("SKIP: TPCDAQ_REAL_GEOMETRY_MINI が未設定(実 .dat はローカルのみ)");
        return;
    };
    if which("root").is_none() {
        eprintln!("SKIP: `root` が PATH に無い(オラクル graw2root.C を走らせられない)");
        return;
    }
    let sink_bin = PathBuf::from(sink_bin);
    // 比較ツールは root_sink と同じディレクトリ。無ければ**落とす**(黙って比較を
    // 省略した green を作らない — elitpc_pevent_e2e と同じ流儀)。
    let compare = sink_bin.with_file_name("compare_raw");
    assert!(
        compare.is_file(),
        "{} が無い。`make -C tools/root_sink compare` でビルドすること",
        compare.display()
    );
    let graw = PathBuf::from(graw);
    assert!(graw.is_file(), "{} が無い", graw.display());

    // --- 一時 dir。実 .graw はリンクで持ち込む(~/TPC を汚さない)---
    let scratch = {
        let dir = std::env::temp_dir().join(format!("tpcdaq-mini-rawtree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    };
    let link = scratch.join(graw.file_name().unwrap());
    std::os::unix::fs::symlink(&graw, &link).expect("symlink the real .graw");
    let out_root = scratch.join("out");
    std::fs::create_dir_all(&out_root).unwrap();
    let ctx = zmq::Context::new();

    // --- root_sink を先に上げる(listen-before-start と同じ理屈)---
    let sink_ep = free_endpoint();
    let mut sink = Command::new(&sink_bin)
        .arg("--bind")
        .arg(&sink_ep)
        .arg("--output-root")
        .arg(&out_root)
        .arg("--expect")
        .arg("0:0")
        .arg("--geometry")
        .arg(&geometry)
        .arg("--raw-tree")
        .arg("--run-id")
        .arg(FIXED_RUN_ID)
        // run 毎単一ファイルで比較する(PEventTPC 側をロールオーバさせない)。
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
    let decoder_pull_bind = configure_arm(&dec_cmd_ep, RUN, "mini rawtree e2e").await;
    let started = rpc(&dec_cmd_ep, &DaqCommand::Start { run_number: RUN }).await;
    assert!(started.success, "{}", started.message);

    // --- graw-writer 行きは drain するだけ ---
    let (writer_pull, writer_ep) = bind_pull(&ctx, 300_000);
    let writer_drain = std::thread::spawn(move || {
        while let Ok(raw) = writer_pull.recv_bytes(0) {
            if let Ok(Message::<RawFrames>::EndOfStream { .. }) =
                Message::<RawFrames>::from_msgpack(&raw)
            {
                break;
            }
        }
    });

    // --- receiver(mini = 1 CoBo)---
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
    let data_addr = configure_arm(&recv_cmd_ep, RUN, "mini rawtree e2e").await;
    rpc(&recv_cmd_ep, &DaqCommand::Start { run_number: RUN }).await;

    // --- graw_replay(実 .graw は読むだけ。書き出しは一切しない)---
    let started_at = Instant::now();
    let mut replay = Command::new(env!("CARGO_BIN_EXE_graw_replay"))
        .arg(&data_addr)
        .arg(&graw)
        .arg("--rate-mbps")
        .arg(REPLAY_RATE_MBPS)
        .spawn()
        .expect("spawn graw_replay");
    let replay_status = replay.wait().expect("wait for graw_replay");
    assert!(
        replay_status.success(),
        "graw_replay failed: {replay_status:?}"
    );
    eprintln!(
        "replayed at {REPLAY_RATE_MBPS} Mbps in {:.1}s",
        started_at.elapsed().as_secs_f64()
    );

    // --- finalize(rename 完了)を待つ。raw と PEventTPC の**両方**が閉じてから
    //     止める(SIGTERM が finalize を追い越すと inprogress で残る)---
    let run_dir = out_root.join(format!("run{RUN:04}"));
    let raw_file = run_dir.join(format!("run{RUN:04}_raw.root"));
    let pevent_file = run_dir.join(format!("run{RUN:04}.root"));
    assert!(
        wait_for_file(&raw_file, Duration::from_secs(300)),
        "{} が 300 s 以内に出来なかった",
        raw_file.display()
    );
    assert!(
        wait_for_file(&pevent_file, Duration::from_secs(300)),
        "{} が 300 s 以内に出来なかった",
        pevent_file.display()
    );
    eprintln!("finalized after {:.1}s", started_at.elapsed().as_secs_f64());

    // --- root_sink のカウンタ(SPEC §6.7 の raw_* を含む)---
    send_term(sink_pid);
    let sink_status = wait_for_exit(&mut sink, Duration::from_secs(60));
    assert!(sink_status.success(), "root_sink: {sink_status:?}");
    let counts = read_counts(&mut sink);
    eprintln!("--- root_sink counters ---\n{counts}");
    assert_eq!(count(&counts, "events_complete"), EVENTS, "counts={counts}");
    assert_eq!(count(&counts, "events_incomplete"), 0, "counts={counts}");
    assert_eq!(count(&counts, "entries_written"), EVENTS, "counts={counts}");
    assert_eq!(count(&counts, "raw_tree"), 1, "counts={counts}");
    assert_eq!(
        count(&counts, "raw_entries_written"),
        EVENTS,
        "counts={counts}"
    );
    assert_eq!(
        count(&counts, "raw_items_out_of_range"),
        0,
        "counts={counts}"
    );
    assert_eq!(
        count(&counts, "raw_unexpected_fragments"),
        0,
        "counts={counts}"
    );
    assert_eq!(counts["fatal"], "", "counts={counts}");

    // --- オラクル: 同じ .graw を graw2root.C で変換(一時 dir のリンク経由)---
    run_graw2root(&link, None, &scratch);
    let oracle = graw2root_output(&link);
    assert!(oracle.is_file(), "{} が出来ていない", oracle.display());

    // --- 本題: 全エントリ一致(eventId / eventTime / adc 全要素)---
    let output = Command::new(&compare)
        .arg(&raw_file)
        .arg(&oracle)
        .output()
        .expect("run compare_raw");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    eprintln!(
        "--- compare_raw ---\n{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "raw ツリーが graw2root.C の出力と一致しない(SPEC §6.7)"
    );
    assert!(
        stdout.contains(&format!("compared {EVENTS} events, 0 differences")),
        "compare_raw の出力が想定と違う: {stdout}"
    );

    // --- 負性対照: 比較器が差を見られることの証明(107 イベントだけ変換し直す)---
    run_graw2root(&link, Some((EVENTS - 1) as i64), &scratch);
    let short_output = Command::new(&compare)
        .arg(&raw_file)
        .arg(&oracle)
        .output()
        .expect("run compare_raw (negative control)");
    eprintln!(
        "--- compare_raw (negative control) ---\n{}{}",
        String::from_utf8_lossy(&short_output.stdout),
        String::from_utf8_lossy(&short_output.stderr)
    );
    assert!(
        !short_output.status.success(),
        "107 イベントのオラクルと一致してしまった = 比較器が中身を見ていない"
    );

    // --- 後始末 ---
    let _ = recv_shutdown_tx.send(());
    let _ = tokio::time::timeout(Duration::from_secs(5), recv_task).await;
    let _ = dec_shutdown_tx.send(());
    let _ = tokio::time::timeout(Duration::from_secs(5), dec_task).await;
    let _ = writer_drain.join();
    let _ = std::fs::remove_dir_all(&scratch);
}
