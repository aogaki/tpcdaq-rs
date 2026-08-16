//! P2 出口の全経路照合(TODO/012、SPEC §12-4 + §6.3 v1.3)。
//!
//! ここまでの完成品(005 graw_replay / 006 receiver / 007 graw-writer / 009 decoder /
//! 010 event builder / 011→020 PEventTPC TTree)を**実トポロジーで配線するだけ**のテスト。
//! 新しい production コードは 1 行も足さない —— 足したくなったら統合で見つけた本物のバグ。
//!
//! * **E2E-B**: 2 ソースビルド(§12-4)。実 .graw と、その **coboIdx を 0→1 に書き換えた
//!   合成コピー**(テスト側で生成、リポには置かない)を並走リプレイ →
//!   receiver ×2 → decoder(期待 {0,1})→ root_sink(`--expect 0:0,1:0`)。
//!   全イベント complete / incomplete=0 / eventIdx 昇順 / CoBo 毎 109 フレーム受信 /
//!   entries=108(1 エントリ = 1 ビルド済みイベント)。さらに**同じ入力で 2 回走らせて
//!   2 本の run.root が全イベント全 key 一致**(= SPEC v1.3 の「イベント内 (cobo,asad)
//!   昇順」が効いている証明。`compare_pevent`)。
//!
//! **v1.17(TODO/054)で E2E-A を退役させた**: あれは §12-3 の**旧 GDataFrame オラクル**
//! (実機 .root と TTree 全値比較)で、SPEC v1.17 がその行ごと撤去した(削除条件 =
//! PEventTPC 実データオラクルの成立は 021 の `compared 3852 events, 0 differences` で満了)。
//! 現行の §12-3 は `tests/elitpc_pevent_e2e.rs` が担う。§12-2(graw バイト一致)は
//! `tests/root_sink_intake.rs` と graw-writer 側の試験が持っている。
//!
//! **env が 2 つ揃ったときだけ走る**(実 .graw はリポに入れない、C++ ビルドを
//! cargo test の前提にしない —— CLAUDE.md):
//!
//! ```text
//! make -C tools/root_sink && make -C tools/root_sink compare
//! TPCDAQ_ROOT_SINK_BIN=$PWD/tools/root_sink/root_sink \
//! TPCDAQ_REAL_GRAW=$HOME/TPC/CoBo_2025-09-01T08_51_06.203_0000.graw \
//!   cargo test --test p2_e2e -- --nocapture
//! ```

#![allow(clippy::unwrap_used)]

use std::io::Read;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as ProcessCommand, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use tokio::sync::{broadcast, oneshot};
use tpcdaq::command::{Command, CommandResponse, ComponentState, RunConfig};
use tpcdaq::decode::peek_asad;
use tpcdaq::decoder::{run_decoder, DecoderParams};
use tpcdaq::framer::Framer;
use tpcdaq::msg::{Message, RawFrames};
use tpcdaq::receiver::{run_receiver, ReceiverParams};
use tpcdaq::zmq_helper;

/// MFM ヘッダの coboIdx は 1 バイト・オフセット 26(`src/decode.rs` の `frame[26]` と対)。
const COBO_IDX_OFFSET: usize = 26;

/// **E2E-B のリプレイは目標レートでペーシングする**(SPEC §12 末尾:
/// 「mini 100 Hz 相当 ≈ 28 MB/s = 224 Mbps」)。理由:
///
/// 2 ソースを**全速**でぶつけると、受信バッチが `batch_max_bytes`(8 MiB ≈ 29 フレーム)
/// 一杯まで太り、decoder はそれを 1 個ずつ処理するため、ビルダの目から見た 2 ソースの
/// ずれがバッチ 1 個分(実測 1〜2 s)まで開く。既定 `build_timeout_ms` = 1000 ms
/// (SPEC §6.3)を超えるので**全イベントが incomplete で出る** —— これはビルダの仕様どおりの
/// 挙動(捨てずに incomplete + late で保全)であって欠陥ではないが、§12-4 が主張したい
/// 「2 ソースが揃って complete になる」を試験できない。
///
/// **build_timeout_ms を緩める方向では直さない**(それでは §12-4 の主張が薄まる)。
/// 実験室の実機は 2 CoBo が同一トリガで同時に送るので、目標レートでのペーシングこそが
/// 実機に近い。実測: 224 Mbps で `events_complete=108 / incomplete=0 / late=0`(既定
/// タイムアウトのまま)、バッチは 201 個に細分されて 2 ソースが噛み合う。
const REPLAY_RATE_MBPS: f64 = 224.0;

// ---------------------------------------------------------------------------
// env ゲート
// ---------------------------------------------------------------------------

/// 2 つの env が揃っていれば `(root_sink, 実 .graw)`。欠けたら skip 理由を印字。
fn e2e_env() -> Option<(PathBuf, PathBuf)> {
    let Some(bin) = std::env::var_os("TPCDAQ_ROOT_SINK_BIN").map(PathBuf::from) else {
        eprintln!("SKIP: TPCDAQ_ROOT_SINK_BIN が未設定(make -C tools/root_sink)");
        return None;
    };
    let Some(graw) = std::env::var_os("TPCDAQ_REAL_GRAW").map(PathBuf::from) else {
        eprintln!("SKIP: TPCDAQ_REAL_GRAW が未設定(実 .graw はローカルのみ)");
        return None;
    };
    Some((bin, graw))
}

/// `compare_pevent`(root_sink と同じディレクトリ)。無ければ**落とす** ——
/// 決定性の照合こそがこのテストの目的なので、黙って skip しては意味がない。
fn comparator(sink_bin: &Path) -> PathBuf {
    let path = sink_bin.with_file_name("compare_pevent");
    assert!(
        path.is_file(),
        "{} が無い。`make -C tools/root_sink compare` でビルドすること",
        path.display()
    );
    path
}

/// 2 ソースビルドに要る合成ジオメトリ((cobo 0, asad 0) と (cobo 1, asad 0) を含む)。
/// このテストの持ち場はビルダの決定性であって chargeMap の物理値ではない。
const GEOMETRY_2COBO: &str = "tests/fixtures/geometry_2cobo_fake.dat";

/// 2 回の走行で `EventInfo::runId` を揃えるための固定値(既定は run を開いた壁時計 ——
/// それだと 2 本の run.root が runId だけで差分になる)。
const FIXED_RUN_ID: &str = "20260101000000";

// ---------------------------------------------------------------------------
// プロセス操作(tests/root_sink_intake.rs の流儀をそのまま)
// ---------------------------------------------------------------------------

/// 空きポートを 1 つ確保して即座に手放す(固定ポートを書かない = 並列実行に耐える)。
fn free_endpoint() -> String {
    let probe = TcpListener::bind("127.0.0.1:0").expect("bind probe listener");
    let port = probe.local_addr().expect("local_addr").port();
    drop(probe);
    format!("tcp://127.0.0.1:{port}")
}

fn spawn_sink(bin: &Path, endpoint: &str, extra: &[&str]) -> Child {
    ProcessCommand::new(bin)
        .arg("--bind")
        .arg(endpoint)
        .args(extra)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn root_sink")
}

/// SIGTERM(std::process::Child::kill は SIGKILL なので終了時 JSON が読めない)。
fn send_term(pid: u32) {
    let status = ProcessCommand::new("kill")
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
        assert!(
            Instant::now() < deadline,
            "root_sink did not exit within {timeout:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// 終了時に stdout へ出る JSON 1 行。
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

// ---------------------------------------------------------------------------
// コマンド REP(Configure → Arm → Start → … → Stop)
// ---------------------------------------------------------------------------

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

/// Configure → Arm → Start。戻り値は Arm が報告する bind アドレス。
async fn configure_arm_start(endpoint: &str, run: u32, comment: &str) -> String {
    let configured = rpc(
        endpoint,
        &Command::Configure(RunConfig {
            run_number: run,
            comment: comment.to_string(),
            config: serde_json::Value::Null,
        }),
    )
    .await;
    assert!(
        configured.success,
        "Configure failed: {}",
        configured.message
    );
    let armed = rpc(endpoint, &Command::Arm).await;
    assert!(armed.success, "Arm failed: {}", armed.message);
    let bind = armed.metrics.as_ref().unwrap()["bind_address"]
        .as_str()
        .unwrap()
        .to_string();
    let started = rpc(endpoint, &Command::Start { run_number: run }).await;
    assert!(started.success, "Start failed: {}", started.message);
    bind
}

/// `Stop` を打って `Configured` へ戻ったことを確かめる(SPEC §1.3 の遷移)。
async fn stop(endpoint: &str, who: &str) {
    let stopped = rpc(endpoint, &Command::Stop).await;
    assert!(stopped.success, "{who}: Stop failed: {}", stopped.message);
    assert_eq!(
        stopped.state,
        ComponentState::Configured,
        "{who}: Stop 後は Configured(SPEC §1.3)"
    );
}

// ---------------------------------------------------------------------------
// ファイル小道具
// ---------------------------------------------------------------------------

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tpcdaq_p2_e2e_{}_{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn wait_for_file(path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.is_file() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn names_starting_with(dir: &Path, prefix: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(prefix))
        .collect();
    out.sort();
    out
}

/// 実 .graw を framer で切り、**各フレームのヘッダ 26 バイト目(coboIdx)を `to` に書き換えた**
/// コピーを作る(TODO/012 §3 — 2 本目のソースをテスト側で合成。production には触らない)。
///
/// フレーム境界は既存 [`Framer`]、書き換えはこの 1 バイトだけ。フレーム長も中身も変えないので、
/// 出来上がりは「別の CoBo から届いた同じ波形」に等しい。
/// 戻り値は (書いたバイト数, 書き換えたフレーム数, coboIdx を持たなかったフレーム数)。
fn rewrite_cobo_id(source: &Path, dest: &Path, to: u8) -> (u64, usize, usize) {
    let bytes = std::fs::read(source).unwrap_or_else(|e| panic!("read {}: {e}", source.display()));
    let mut framer = Framer::new();
    let mut out = Vec::with_capacity(bytes.len());
    let (mut rewritten, mut untouched) = (0usize, 0usize);
    for chunk in bytes.chunks(8192) {
        framer.push(chunk);
        while let Some(frame) = framer.next() {
            let mut copy = frame.to_vec();
            // AsAd データフレーム(frameType 1/2)だけが coboIdx を持つ。
            // 制御フレーム(12 B の frameType 7 等)は 1 バイトも触らずに通す。
            if peek_asad(frame).is_some() && copy.len() > COBO_IDX_OFFSET {
                copy[COBO_IDX_OFFSET] = to;
                rewritten += 1;
            } else {
                untouched += 1;
            }
            out.extend_from_slice(&copy);
        }
    }
    assert_eq!(
        out.len(),
        bytes.len(),
        "書き換えでバイト数が変わってはいけない"
    );
    std::fs::write(dest, &out).unwrap_or_else(|e| panic!("write {}: {e}", dest.display()));
    (out.len() as u64, rewritten, untouched)
}

// ---------------------------------------------------------------------------
// compare_pevent の呼び出し
// ---------------------------------------------------------------------------

/// 比較器を走らせ、突き合わせたイベント数を返す。**exit 0 でなければ落とす**
/// (`compare_pevent` は差分 0 のときだけ 0 で抜ける)。
fn compare_pevent_roots(tool: &Path, a: &Path, b: &Path) -> u64 {
    let out = ProcessCommand::new(tool)
        .arg(a)
        .arg(b)
        .output()
        .unwrap_or_else(|e| panic!("spawn {}: {e}", tool.display()));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    eprintln!("--- compare_pevent ---\n{stdout}");
    assert!(
        out.status.success(),
        "compare_pevent が一致しなかった({:?})\nstdout:\n{stdout}\nstderr:\n{stderr}",
        out.status
    );
    let line = stdout
        .lines()
        .find(|l| l.starts_with("compared "))
        .unwrap_or_else(|| panic!("要約行 \"compared ...\" が無い:\n{stdout}"));
    line.split_whitespace()
        .nth(1)
        .expect("compared <N> events")
        .parse()
        .expect("compared count")
}

// ===========================================================================
// E2E-B — 2 ソースビルド(§12-4)+ 決定性(SPEC v1.3)
// ===========================================================================

/// 1 回分の 2 ソース走行。書き上がった `run{run:04}.root` のパスと root_sink のカウンタを返す。
///
/// 配線: graw_replay ×2 並走 → receiver ×2(cobo 0 / 1)→ decoder(期待 {0,1})→
/// root_sink(`--expect 0:0,1:0`)。graw-writer は §12-4 の主張に関係しないので繋がない
/// (receiver の graw-writer 行きは drain スレッドが吸う —— HWM で receiver を止めないため)。
async fn run_two_source_build(
    sink_bin: &Path,
    sources: [&Path; 2],
    out_root: &Path,
    run: u32,
    tag: &str,
) -> (PathBuf, serde_json::Value, [u64; 2], Duration) {
    let ctx = zmq::Context::new();

    // --- root_sink ---
    let sink_ep = free_endpoint();
    let mut sink = spawn_sink(
        sink_bin,
        &sink_ep,
        // **--run-id は固定**: 2 回の走行を compare_pevent で突き合わせるので、
        // run を開いた壁時計由来の runId が違うと EventInfo だけで差分になる。
        &[
            "--geometry",
            GEOMETRY_2COBO,
            "--run-id",
            FIXED_RUN_ID,
            "--output-root",
            &out_root.to_string_lossy(),
            "--expect",
            "0:0,1:0",
        ],
    );
    let sink_pid = sink.id();
    std::thread::sleep(Duration::from_millis(300));

    // --- decoder(期待ソース {0,1}) ---
    let decoder_params = DecoderParams {
        pull_bind: "tcp://127.0.0.1:0".to_string(),
        push_connect: sink_ep.clone(),
        command_listen: "tcp://127.0.0.1:0".to_string(),
        batch_max_bytes: tpcdaq::config::DEFAULT_BATCH_MAX_BYTES,
        batch_max_ms: tpcdaq::config::DEFAULT_BATCH_MAX_MS,
        heartbeat_ms: tpcdaq::config::DEFAULT_HEARTBEAT_MS,
        send_timeout_ms: tpcdaq::config::DEFAULT_DECODER_SEND_TIMEOUT_MS,
        workers: 1,
        expected_sources: vec![0, 1],
    };
    let (dec_shutdown_tx, dec_shutdown_rx) = broadcast::channel(1);
    let (dec_ep_tx, dec_ep_rx) = oneshot::channel();
    let dec_task = tokio::spawn(run_decoder(
        decoder_params,
        dec_shutdown_rx,
        Some(dec_ep_tx),
    ));
    let dec_cmd_ep = dec_ep_rx.await.expect("decoder command endpoint");
    let dec_bind = configure_arm_start(&dec_cmd_ep, run, tag).await;

    // --- receiver ×2(CoBo 0 / 1)---
    let mut recv_cmd_eps = Vec::new();
    let mut data_addrs = Vec::new();
    let mut shutdowns = Vec::new();
    let mut tasks = Vec::new();
    let mut drains = Vec::new();
    for cobo_id in 0u32..2 {
        let sock = ctx.socket(zmq::PULL).unwrap();
        sock.set_linger(0).unwrap();
        zmq_helper::apply_pull_hwm(&sock).unwrap();
        sock.bind("tcp://127.0.0.1:0").unwrap();
        sock.set_rcvtimeo(120_000).unwrap();
        let writer_ep = sock.get_last_endpoint().unwrap().unwrap();
        drains.push(std::thread::spawn(move || {
            while let Ok(raw) = sock.recv_bytes(0) {
                if let Ok(Message::<RawFrames>::EndOfStream { .. }) =
                    Message::<RawFrames>::from_msgpack(&raw)
                {
                    break;
                }
            }
        }));

        let params = ReceiverParams {
            cobo_id,
            listen: "127.0.0.1:0".to_string(),
            command_listen: "tcp://127.0.0.1:0".to_string(),
            graw_writer_endpoint: writer_ep,
            decoder_endpoint: dec_bind.clone(),
            batch_max_bytes: tpcdaq::config::DEFAULT_BATCH_MAX_BYTES,
            batch_max_ms: tpcdaq::config::DEFAULT_BATCH_MAX_MS,
            queue_frames: tpcdaq::config::DEFAULT_QUEUE_FRAMES,
            heartbeat_ms: tpcdaq::config::DEFAULT_HEARTBEAT_MS,
            hwm: zmq_helper::DEFAULT_HWM,
        };
        let (tx, rx) = broadcast::channel(1);
        let (ep_tx, ep_rx) = oneshot::channel();
        tasks.push(tokio::spawn(run_receiver(params, rx, Some(ep_tx))));
        let cmd_ep = ep_rx.await.expect("receiver command endpoint");
        data_addrs.push(configure_arm_start(&cmd_ep, run, tag).await);
        recv_cmd_eps.push(cmd_ep);
        shutdowns.push(tx);
    }

    // --- graw_replay ×2 並走(全速)---
    let started_at = Instant::now();
    let mut replays: Vec<Child> = (0..2)
        .map(|i| {
            ProcessCommand::new(env!("CARGO_BIN_EXE_graw_replay"))
                .arg(&data_addrs[i])
                .arg(sources[i])
                .arg("--rate-mbps")
                .arg(REPLAY_RATE_MBPS.to_string())
                .spawn()
                .expect("spawn graw_replay")
        })
        .collect();
    for (i, child) in replays.iter_mut().enumerate() {
        let st = child.wait().expect("wait for graw_replay");
        assert!(st.success(), "graw_replay[{i}] failed: {st:?}");
    }

    // --- run ファイルの finalize を待つ ---
    let run_dir = out_root.join(format!("run{run:04}"));
    let run_file = run_dir.join(format!("run{run:04}.root"));
    assert!(
        wait_for_file(&run_file, Duration::from_secs(120)),
        "{} が 120 s 以内に出来なかった(dir={:?})",
        run_file.display(),
        names_starting_with(&run_dir, "")
    );
    let elapsed = started_at.elapsed();

    // --- receiver 毎のフレーム数(CoBo 毎フレーム数一致の独立系証拠)---
    let mut frames_per_cobo = [0u64; 2];
    for (i, ep) in recv_cmd_eps.iter().enumerate() {
        let st = rpc(ep, &Command::GetStatus).await;
        frames_per_cobo[i] = st.metrics.as_ref().unwrap()["frames"].as_u64().unwrap();
    }

    // --- EOS 確認後 Stop ---
    for ep in &recv_cmd_eps {
        stop(ep, "receiver").await;
    }
    stop(&dec_cmd_ep, "decoder").await;

    send_term(sink_pid);
    let sink_status = wait_for_exit(&mut sink, Duration::from_secs(30));
    assert!(sink_status.success(), "root_sink: {sink_status:?}");
    let counts = read_counts(&mut sink);

    // **完全に畳んでから戻る**(決定性チェックのために同じプロセスで 2 回走らせるので、
    // 前の run のタスク・スレッド・ZMQ コンテキストが残っていると次の run の
    // スケジューリングを乱す。畳めなかったら黙って進まず落とす)。
    for tx in &shutdowns {
        let _ = tx.send(());
    }
    for task in tasks {
        tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .expect("receiver did not stop within 10 s")
            .expect("receiver task panicked");
    }
    let _ = dec_shutdown_tx.send(());
    tokio::time::timeout(Duration::from_secs(10), dec_task)
        .await
        .expect("decoder did not stop within 10 s")
        .expect("decoder task panicked");
    for d in drains {
        d.join().expect("graw-writer drain thread panicked");
    }
    drop(ctx); // ZMQ の IO スレッドまで畳む(term は全ソケット close 後に返る)
    assert!(
        names_starting_with(&run_dir, "run_inprogress_").is_empty(),
        "inprogress が残っている: {:?}",
        names_starting_with(&run_dir, "run_inprogress_")
    );

    (run_file, counts, frames_per_cobo, elapsed)
}

/// §12-4 のオラクル(2 ソース = 実 .graw + coboIdx を 1 に書き換えた合成コピー):
///   events_complete = 108(全イベント complete)/ incomplete = 0
///   フラグメント = 216 = 108 イベント × 2 CoBo
///   **TTree entries = 108**(1 エントリ = 1 ビルド済みイベント、SPEC §6.4 v1.8)
///   receiver 毎の受信フレーム数 = 109 / 109(108 データ + 制御 1)
/// 加えて **2 回走らせて全イベント全 key 一致**(SPEC v1.3 の決定的順序)。
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn e2e_b_two_source_build_is_complete_ordered_and_deterministic() {
    let Some((sink_bin, graw)) = e2e_env() else {
        return;
    };
    // **dev / release の両方で走る**。以前は dev を skip していた: decoder が入力
    // (2 × 224 Mbps)に追いつけないと PULL が上流 2 ソースの片方を run 丸ごと飢餓させ、
    // ビルダが仕様どおり全イベントを incomplete + late にしていたため(TODO/012 の
    // 「発見した不具合」)。TODO/013 で原因(libzmq の fair-queue 非活性化が
    // `process_commands()` を待つ = `recv` だけでは 100 通に 1 度しか走らない)を計測で確定し、
    // decoder の受信ループを「recv の前に必ず poll」へ直したので、**追いつけない dev でも
    // 公平に消費され complete になる**。dev の skip を戻すことは修正の退行を意味する。
    let tool = comparator(&sink_bin);
    const RUN: u32 = 21;

    let scratch = scratch_dir("b");
    let synthetic = scratch.join("CoBo1_synthetic.graw");
    let (synth_bytes, rewritten, untouched) = rewrite_cobo_id(&graw, &synthetic, 1);
    let source_bytes = std::fs::metadata(&graw).unwrap().len();
    assert_eq!(synth_bytes, source_bytes, "合成ソースのバイト数は元と同じ");
    assert_eq!(rewritten, 108, "coboIdx を書き換えた AsAd フレーム数");
    assert_eq!(untouched, 1, "触っていない制御フレーム数(frameType 7 ×1)");

    // --- 1 回目 ---
    let out1 = scratch.join("run1");
    std::fs::create_dir_all(&out1).unwrap();
    let (file1, counts1, frames1, elapsed1) = run_two_source_build(
        &sink_bin,
        [&graw, &synthetic],
        &out1,
        RUN,
        "p2 e2e-b pass 1",
    )
    .await;

    // --- §12-4 の受け入れ数値 ---
    //
    // **ここが落ちて `events_incomplete=108 / late_fragments=108` になっていたら、
    // decoder の PULL が上流 2 ソースの片方を run 丸ごと飢餓させている**(TODO/012 の
    // 「## 結果 — 発見した不具合」参照。receiver 側は完全に足並みが揃っており、
    // decoder の `batches_in` が {"0":26,"1":1} のように偏る)。ビルダの側は仕様どおり
    // (捨てずに incomplete + late で保全)なので、直す場所は decoder の受信ループ。
    assert_eq!(count(&counts1, "fragments"), 216, "counts={counts1}");
    assert_eq!(count(&counts1, "items"), 2 * 15_040_512, "counts={counts1}");
    assert_eq!(count(&counts1, "events_complete"), 108, "counts={counts1}");
    assert_eq!(count(&counts1, "events_incomplete"), 0, "counts={counts1}");
    assert_eq!(count(&counts1, "late_fragments"), 0, "counts={counts1}");
    assert_eq!(
        count(&counts1, "unexpected_fragments"),
        0,
        "counts={counts1}"
    );
    assert_eq!(
        count(&counts1, "duplicate_fragments"),
        0,
        "counts={counts1}"
    );
    assert_eq!(count(&counts1, "pending_events"), 0, "counts={counts1}");
    // **1 エントリ = 1 ビルド済みイベント**(SPEC §6.4 v1.8)。2 フラグメントが 1 本に
    // マージされるので、フラグメント 216 に対してエントリは 108。
    assert_eq!(count(&counts1, "entries_written"), 108, "counts={counts1}");
    assert_eq!(counts1["fatal"], "", "counts={counts1}");
    // receiver の `frames` は**受け取った MFM フレーム全部**なので 108 データ + 1 制御 = 109。
    // 「CoBo 毎フレーム数一致」の本体(TTree 上で 108/108)は下の cobo_entries が見る。
    assert_eq!(
        frames1,
        [109, 109],
        "CoBo 毎受信フレーム数(108 データ + ctrl 1)"
    );
    let files1 = counts1["root_files"].as_array().expect("root_files");
    assert_eq!(files1.len(), 1, "run 毎単一 ROOT: {counts1}");
    assert_eq!(files1[0]["entries"].as_u64(), Some(108));

    // --- 2 回目(同じ入力・同じ配線。決定性の相手)---
    let out2 = scratch.join("run2");
    std::fs::create_dir_all(&out2).unwrap();
    let (file2, counts2, frames2, elapsed2) = run_two_source_build(
        &sink_bin,
        [&graw, &synthetic],
        &out2,
        RUN,
        "p2 e2e-b pass 2",
    )
    .await;
    assert_eq!(count(&counts2, "entries_written"), 108, "counts={counts2}");
    assert_eq!(count(&counts2, "events_complete"), 108, "counts={counts2}");
    assert_eq!(count(&counts2, "events_incomplete"), 0, "counts={counts2}");
    assert_eq!(frames2, [109, 109], "CoBo 毎受信フレーム数(2 回目)");

    // --- 決定性: 2 本の run.root が**全イベント全 key**で一致(SPEC v1.3)---
    //
    // (cobo,asad) 昇順が効いていないと、2 CoBo の充填順が run 毎に揺れて chargeMap の
    // 加算順が変わる —— 値が 1 bit でも違えば compare_pevent(許容差 0)が落ちる。
    let compared = compare_pevent_roots(&tool, &file1, &file2);
    assert_eq!(compared, 108, "突き合わせたイベント数");

    eprintln!(
        "E2E-B: pass1 {:.3} s / pass2 {:.3} s / 合成ソース {} B({} フレーム書き換え)\n\
         pass1 counters: {counts1}\npass2 counters: {counts2}",
        elapsed1.as_secs_f64(),
        elapsed2.as_secs_f64(),
        synth_bytes,
        rewritten,
    );

    let _ = std::fs::remove_dir_all(&scratch);
}
