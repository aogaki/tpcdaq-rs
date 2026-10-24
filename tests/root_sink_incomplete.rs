//! root-sink: EOS が来ないまま次の run が来たとき(SPEC v1.53 §6.2-5、TODO/186)。
//!
//! 古い run を **inprogress のまま閉じ**(完成名にしない、`run{N}_monitor.root` を書かない)、
//! 新しい run を普通に開く。close 行・status・終了 JSON のカウンタも見る。
//!
//! 実バイナリが要る: env `TPCDAQ_ROOT_SINK_BIN` 未設定なら skip(root_sink_intake.rs と同じ流儀)。
//!
//! ```text
//! make -C tools/root_sink
//! TPCDAQ_ROOT_SINK_BIN=$PWD/tools/root_sink/root_sink cargo test --test root_sink_incomplete
//! ```

#![allow(clippy::unwrap_used)]

use std::io::Read;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_bytes::ByteBuf;
use tpcdaq::msg::{self, Batch, Fragment, Fragments, Message};

const DECODER_SOURCE_ID: u32 = 100;
const GEOMETRY: &str = "tests/fixtures/geometry_mini_reduced.dat";
const RUN_A: u32 = 7;
const RUN_B: u32 = 8;

fn sink_bin() -> Option<PathBuf> {
    std::env::var_os("TPCDAQ_ROOT_SINK_BIN").map(PathBuf::from)
}

fn free_endpoint() -> String {
    let probe = TcpListener::bind("127.0.0.1:0").expect("bind probe listener");
    let port = probe.local_addr().expect("local_addr").port();
    drop(probe);
    format!("tcp://127.0.0.1:{port}")
}

/// 起動した root_sink。Drop で必ず殺す(置き去り防止)。stderr はファイルへ(close 行を読む)。
struct Sink {
    child: Child,
    data_ep: String,
    pub_ep: String,
    stderr_path: PathBuf,
}

impl Drop for Sink {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

impl Sink {
    /// bind 失敗(free_endpoint の TOCTOU、exit 4)は新しいポートで最大 3 回やり直す。
    fn spawn(bin: &Path, scratch: &Path, extra: &[&str]) -> Sink {
        for attempt in 1..=3 {
            let data_ep = free_endpoint();
            let pub_ep = free_endpoint();
            let stderr_path = scratch.join(format!("stderr_{attempt}.log"));
            let stderr = std::fs::File::create(&stderr_path).expect("create stderr log");
            let mut child = Command::new(bin)
                .arg("--bind")
                .arg(&data_ep)
                .arg("--pub")
                .arg(&pub_ep)
                .args(extra)
                .stdout(Stdio::piped())
                .stderr(Stdio::from(stderr))
                .spawn()
                .expect("spawn root_sink");
            std::thread::sleep(Duration::from_millis(200));
            if let Some(status) = child.try_wait().expect("try_wait") {
                if status.code() == Some(4) {
                    eprintln!("Sink::spawn: attempt {attempt} bind failed — retrying");
                    continue;
                }
            }
            return Sink {
                child,
                data_ep,
                pub_ep,
                stderr_path,
            };
        }
        panic!("root_sink failed to bind after 3 attempts");
    }

    /// SIGTERM → 終了 JSON 1 行。
    fn terminate(&mut self) -> serde_json::Value {
        let status = Command::new("kill")
            .arg("-TERM")
            .arg(self.child.id().to_string())
            .status()
            .expect("run kill(1)");
        assert!(status.success());
        let deadline = Instant::now() + Duration::from_secs(20);
        let exit = loop {
            if let Some(s) = self.child.try_wait().expect("try_wait") {
                break s;
            }
            assert!(Instant::now() < deadline, "root_sink did not exit");
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(exit.success(), "SIGTERM は exit 0 であるべき: {exit:?}");
        let mut out = String::new();
        self.child
            .stdout
            .take()
            .expect("piped stdout")
            .read_to_string(&mut out)
            .expect("read stdout");
        serde_json::from_str(out.lines().last().unwrap_or_default())
            .unwrap_or_else(|e| panic!("stdout is not one JSON line: {out:?} ({e})"))
    }

    fn stderr_text(&self) -> String {
        std::fs::read_to_string(&self.stderr_path).unwrap_or_default()
    }
}

fn count(counts: &serde_json::Value, key: &str) -> u64 {
    counts[key]
        .as_u64()
        .unwrap_or_else(|| panic!("counter {key} missing in {counts}"))
}

/// 非対称な合成フラグメント(1 イベント = 1 フラグメント、既定の期待集合 0:0)。
fn fragment(event_idx: u32) -> Fragment {
    let words: Vec<u32> = (0..4u32)
        .map(|k| {
            msg::pack_item(
                0,
                (k + 1) as u8,
                (k * 5 + event_idx) as u16,
                (100 + k * 11) as u16,
            )
            .expect("pack_item")
        })
        .collect();
    Fragment {
        event_idx,
        event_time: 0x0000_1234_5678_9abc + u64::from(event_idx),
        cobo: 0,
        asad: 0,
        frame_type: 2,
        revision: 5,
        read_offset: 136,
        status: 0,
        mult: [68, 0, 17, 3],
        window_out: 512,
        last_cell: [7, 9, 11, 13],
        items: ByteBuf::from(msg::items_to_bytes(&words)),
    }
}

/// `events` = このバッチに載せる event_idx 列。
fn batch(run: u32, seq: u64, events: &[u32]) -> Vec<u8> {
    let payload: Fragments = events.iter().map(|e| fragment(*e)).collect();
    let message: Message<Fragments> = Message::Data(Batch {
        source_id: DECODER_SOURCE_ID,
        run_number: run,
        sequence_number: seq,
        created_ns: 1_755_000_000_000_000_000 + seq,
        payload,
    });
    message.to_msgpack().expect("encode Data batch")
}

fn eos(run: u32) -> Vec<u8> {
    let message: Message<Fragments> = Message::EndOfStream {
        source_id: DECODER_SOURCE_ID,
        run_number: run,
    };
    message.to_msgpack().expect("encode EndOfStream")
}

fn connect_push(ctx: &zmq::Context, endpoint: &str) -> zmq::Socket {
    let push = ctx.socket(zmq::PUSH).expect("PUSH socket");
    push.set_sndhwm(100).expect("set sndhwm");
    push.set_sndtimeo(15_000).expect("set sndtimeo");
    push.set_linger(2_000).expect("set linger");
    push.connect(endpoint).expect("connect PUSH");
    push
}

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tpcdaq_root_sink_incomplete_{}_{tag}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
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

fn wait_for_file(path: &Path, secs: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if path.is_file() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    path.is_file()
}

/// status の payload(JSON 値)を `until` まで集める。
fn collect_statuses(sub: &zmq::Socket, until: Instant) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    while Instant::now() < until {
        let Ok(raw) = sub.recv_bytes(0) else { continue };
        let Ok(Message::Data(b)) = rmp_serde::from_slice::<Message<serde_json::Value>>(&raw) else {
            continue;
        };
        if b.payload["kind"] == "status" {
            out.push(b.payload);
        }
    }
    out
}

fn connect_sub(ctx: &zmq::Context, endpoint: &str) -> zmq::Socket {
    let sub = ctx.socket(zmq::SUB).expect("SUB socket");
    sub.set_subscribe(b"").expect("subscribe all");
    sub.set_rcvtimeo(200).expect("set rcvtimeo");
    sub.set_linger(0).expect("set linger");
    sub.connect(endpoint).expect("connect SUB");
    std::thread::sleep(Duration::from_millis(300));
    sub
}

/// 受け入れ 4 の本体。run A に 3 事象(EOS なし)→ run B に 2 事象 + EOS。
/// `with_pevent` = false なら `--no-root --raw-tree`。
fn run_incomplete_then_next(with_pevent: bool, tag: &str) {
    let Some(bin) = sink_bin() else {
        eprintln!("SKIP: TPCDAQ_ROOT_SINK_BIN not set");
        return;
    };
    let out_root = scratch_dir(tag);
    let out = out_root.to_string_lossy().into_owned();
    let mut extra = vec!["--raw-tree", "--geometry", GEOMETRY, "--output-root", &out];
    if !with_pevent {
        extra.insert(0, "--no-root");
    }
    let mut sink = Sink::spawn(&bin, &out_root, &extra);
    let ctx = zmq::Context::new();
    let push = connect_push(&ctx, &sink.data_ep);
    std::thread::sleep(Duration::from_millis(300));

    // run A: event 0,1,2(バッチ 2 通)。EOS を送らない。
    push.send(batch(RUN_A, 0, &[0, 1]), 0).expect("send A0");
    push.send(batch(RUN_A, 1, &[2]), 0).expect("send A1");
    // run B: seq は 0 から。event 0,1 + EOS。
    push.send(batch(RUN_B, 0, &[0]), 0).expect("send B0");
    push.send(batch(RUN_B, 1, &[1]), 0).expect("send B1");
    push.send(eos(RUN_B), 0).expect("send EOS B");

    let dir_a = out_root.join(format!("run{RUN_A:04}"));
    let dir_b = out_root.join(format!("run{RUN_B:04}"));
    let monitor_b = dir_b.join(format!("run{RUN_B:04}_monitor.root"));
    assert!(
        wait_for_file(&monitor_b, 10),
        "{} が無い。dir={:?}",
        monitor_b.display(),
        names_starting_with(&dir_b, "")
    );
    let counts = sink.terminate(); // 落ちていれば exit 0 にならない

    // 落ちない・数える
    assert_eq!(counts["fatal"], "", "counts={counts}");
    assert_eq!(count(&counts, "runs_closed_without_eos"), 1, "{counts}");
    assert_eq!(count(&counts, "run_number_mismatch"), 0, "{counts}");
    assert_eq!(count(&counts, "runs"), 1, "{counts}"); // 完了した run は B だけ
    assert_eq!(count(&counts, "events_complete"), 5, "{counts}");

    // run A: 未完 = inprogress のまま、完成名も monitor.root も無い
    assert!(!dir_a.join(format!("run{RUN_A:04}.root")).exists());
    assert!(!dir_a.join(format!("run{RUN_A:04}_raw.root")).exists());
    assert!(!dir_a.join(format!("run{RUN_A:04}_monitor.root")).exists());
    assert_eq!(
        names_starting_with(&dir_a, "run_raw_inprogress_").len(),
        1,
        "{:?}",
        names_starting_with(&dir_a, "")
    );
    assert_eq!(
        names_starting_with(&dir_a, "run_inprogress_").len(),
        usize::from(with_pevent),
        "{:?}",
        names_starting_with(&dir_a, "")
    );
    // run B: 揃う
    assert!(dir_b.join(format!("run{RUN_B:04}_raw.root")).is_file());
    assert_eq!(
        dir_b.join(format!("run{RUN_B:04}.root")).is_file(),
        with_pevent
    );
    assert!(names_starting_with(&dir_b, "run_inprogress_").is_empty());
    assert!(names_starting_with(&dir_b, "run_raw_inprogress_").is_empty());

    // close 行
    let log = sink.stderr_text();
    assert!(
        log.contains("root_sink: run 7 closed without EOS (incomplete)"),
        "close 行が無い:\n{log}"
    );
    assert!(log.contains("root_sink: run 8 closed ("), "{log}");
    let _ = std::fs::remove_dir_all(&out_root);
}

/// 受け入れ 4: PEventTPC + raw。
#[test]
fn a_new_run_closes_the_old_run_incomplete_and_opens_the_new_one() {
    run_incomplete_then_next(true, "both");
}

/// 受け入れ 4: `--no-root --raw-tree` でも同じ。
#[test]
fn a_new_run_closes_the_old_run_incomplete_with_no_root_raw_tree() {
    run_incomplete_then_next(false, "rawonly");
}

/// 受け入れ 5: close 行と status の `fragments` が送ったフラグメント数と一致する。
/// 手計算: run 7 = 3 バッチで 2 + 1 + 3 = 6 フラグメント(EOS で完了)。続けて EOS 無しの
/// run 8 を 2 フラグメント送ると、status は run 8 の 2 を示す(run ごと)。
#[test]
fn close_line_and_status_report_the_fragment_count_of_the_run() {
    let Some(bin) = sink_bin() else {
        eprintln!("SKIP: TPCDAQ_ROOT_SINK_BIN not set");
        return;
    };
    let scratch = scratch_dir("fragments");
    let mut sink = Sink::spawn(&bin, &scratch, &["--no-root", "--geometry", GEOMETRY]);
    let ctx = zmq::Context::new();
    let sub = connect_sub(&ctx, &sink.pub_ep);
    let push = connect_push(&ctx, &sink.data_ep);
    std::thread::sleep(Duration::from_millis(300));

    push.send(batch(RUN_A, 0, &[0, 1]), 0).expect("send 0");
    push.send(batch(RUN_A, 1, &[2]), 0).expect("send 1");
    push.send(batch(RUN_A, 2, &[3, 4, 5]), 0).expect("send 2");
    push.send(eos(RUN_A), 0).expect("send EOS");

    // 閉じた run の status は直近 run の値を持ち続ける(idle)。
    let st = collect_statuses(&sub, Instant::now() + Duration::from_millis(2500));
    let last = st.last().expect("status が 1 通も来ない");
    assert_eq!(last["run"], RUN_A, "{last}");
    assert_eq!(last["state"], "idle", "{last}");
    assert_eq!(last["fragments"], 6, "{last}");
    assert_eq!(last["runs_closed_without_eos"], 0, "{last}");

    // EOS 無しの run 8(2 フラグメント)→ run 8 の値に切り替わる。
    push.send(batch(RUN_B, 0, &[0, 1]), 0).expect("send B0");
    let st = collect_statuses(&sub, Instant::now() + Duration::from_millis(2500));
    let last = st.last().expect("status が 1 通も来ない");
    assert_eq!(last["run"], RUN_B, "{last}");
    assert_eq!(last["state"], "running", "{last}");
    assert_eq!(last["fragments"], 2, "{last}");
    assert_eq!(last["runs_closed_without_eos"], 0, "{last}");

    let counts = sink.terminate();
    assert_eq!(counts["fatal"], "", "{counts}");
    assert_eq!(count(&counts, "fragments"), 8, "{counts}"); // 通算 6 + 2
    let log = sink.stderr_text();
    assert!(
        log.contains(
            "root_sink: run 7 closed (batches=3 events_complete=6 events_incomplete=0 \
             late_fragments=0 restarts=0 events_before_restart=0 restart_duplicates=0 \
             fragments=6)"
        ),
        "close 行に fragments=6 が無い:\n{log}"
    );
    let _ = std::fs::remove_dir_all(&scratch);
}
