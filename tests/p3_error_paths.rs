//! P3 異常系 E2E —— 停止経路の意味論(TODO/033 論点 3、SPEC §1.3 v1.12 / §9.2 v1.12+v1.14 /
//! §6.2-5)。
//!
//! 030(`tests/p3_e2e.rs`)から分離したシナリオ。**production コードはここでは 1 行も
//! 足さない** —— 完成品を実トポロジーで配線し、controller REST で駆動して停止の顛末を
//! 機械照合するだけである。
//!
//! * **F0 — 実機の正規停止経路**(リンク保持 → 強制 EOS)。TODO/032 の調査で
//!   「実 GET の `ecc stop` はデータリンクを close しない」= **実機では停止時に EOF が
//!   来ない**ことが確定した。よって「リンクを保持したまま黙る CoBo」+「receiver `Stop`
//!   による EOS 注入」が**実機の正常系**であり、E2E-C(graw_replay が閉じる = 自然 EOF)は
//!   実機とは別経路だったことになる。ここがその初照合。
//!   あわせて **033-E(受信静止検出)の効き**を停止所要の実測として残す。
//! * **F1 — eos-timeout**(TODO/033 論点 2 の照合)。mode 確定の**後**に receiver を落とし、
//!   `reason="error:eos-timeout"` / `forced_eos=true` / `eos_closed=false` と、root-sink の
//!   run が**開いたまま**残ること(`run_inprogress_*` 残存 / `run{N}.root` も
//!   `run{N}_monitor.root` も無い)を確かめる。
//! * **F2 — 遅発 fatal**(TODO/033 論点 1 の機械照合)。F1 が残した「開いたままの run」に
//!   **次の run の最初の Data とバイト等価な 1 通**を投げ込み、root-sink が
//!   **exit 6 = run-number-mismatch** で落ちること + run 1 のカウンタが保全されることを
//!   確かめる。SPEC §1.3 v1.6 が「同一 run 内 seq ギャップ → exit 3」と書いていた機序は
//!   誤りで、実際は**次 run 冒頭の遅発 fatal(exit 6)**である(v1.12 で訂正済み)。
//!   F1 の終了状態そのものが入力なので、**F1 と同じテスト関数の中で続けて**行う。
//!
//! # ここで検証しないこと(TODO/033「何を検証しないか」より)
//!
//! * 同一 run 内の Gap → exit 3 の実経路(EINTR 級 / 符号化失敗): E2E では作れない。
//!   SeqCheck の単体テスト(`tools/root_sink/test_rs_core`)が担保。
//! * SIGSTOP 特有の half-open REP: controller から見ればタイムアウトも接続拒否も同じ
//!   `Err` なので、**in-process task の abort で観測等価**(030 の発見)。
//! * root-sink 停滞による false-normal(= `eos_out` の穴): PUSH HWM を埋める規模が要る。
//!   `src/controller.rs` / `src/decoder.rs` の単体テストで固定し、負荷実走は 031 へ。
//!
//! **全ポート動的**。env が欠けたら**欠けた変数名を stderr に出して** skip する:
//!
//! ```text
//! TPCDAQ_ROOT_SINK_BIN=$PWD/tools/root_sink/root_sink \
//! TPCDAQ_ECC_BRIDGE_BIN=$PWD/tools/ecc_bridge/ecc_bridge \
//! TPCDAQ_FAKE_ECC_BIN=$PWD/tools/ecc_bridge/fake_ecc \
//! TPCDAQ_REAL_GRAW=$HOME/TPC/CoBo_2025-09-01T08_51_06.203_0000.graw \
//! TPCDAQ_REAL_GEOMETRY_MINI=$HOME/TPC/miniTPC_UVW_pcb_info/new_geometry_mini_eTPC.dat \
//!   cargo test --test p3_error_paths -- --test-threads=1 --nocapture
//! ```

#![allow(clippy::unwrap_used)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as OsCommand, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_bytes::ByteBuf;
use serde_json::{json, Value};
use tokio::sync::{broadcast, oneshot};
use tpcdaq::command::{Command, CommandResponse};
use tpcdaq::config::ConfigIds;
use tpcdaq::controller::{
    run_controller, BoundEndpoints, CoboSpec, ComponentEndpoint, ComponentKind, ControllerParams,
};
use tpcdaq::decoder::{run_decoder, DecoderParams};
use tpcdaq::graw_writer::{run_graw_writer, GrawWriterParams};
use tpcdaq::msg::{Batch, Fragment, Fragments, Message};
use tpcdaq::receiver::{run_receiver, ReceiverParams};

/// 実 .graw の P1 オラクル(SPEC §12-1/§12-2、2026-08-13 実測)。
const REAL_GRAW_BYTES: u64 = 30_108_684;
const REAL_GRAW_EVENTS: u64 = 108;
const REAL_GRAW_ITEMS: u64 = 15_040_512;
/// receiver が数えるフレーム = AsAd データ 108 + 先頭の制御フレーム 1(SPEC §12-1)。
const REAL_GRAW_FRAMES: u64 = REAL_GRAW_EVENTS + 1;

const OPERATOR: &str = "p3-error-paths";
const PASSPHRASE: &str = "p3-error-paths-passphrase";

/// TODO/041 の統合デモで実測した停止所要(033-E 実装前。`reference/_spike/demo`)。
/// F0 はこれを大きく下回るはず = 「毎停止 5 秒の空振り」が消えたことの実証。
const RUN_STOP_BEFORE_QUIESCE_S: f64 = 5.7;

// =====================================================================
// env ゲート(030 と同じ 5 本)
// =====================================================================

struct E2eEnv {
    root_sink: PathBuf,
    ecc_bridge: PathBuf,
    fake_ecc: PathBuf,
    graw: PathBuf,
    geometry: PathBuf,
}

/// 欠けた env 名を **stderr に列挙して** `None`(silent skip を作らない — CLAUDE.md)。
fn e2e_env(test: &str) -> Option<E2eEnv> {
    let mut missing = Vec::new();
    let mut get = |name: &str| -> Option<PathBuf> {
        match std::env::var_os(name) {
            Some(value) => Some(PathBuf::from(value)),
            None => {
                missing.push(name.to_string());
                None
            }
        }
    };
    let root_sink = get("TPCDAQ_ROOT_SINK_BIN");
    let ecc_bridge = get("TPCDAQ_ECC_BRIDGE_BIN");
    let fake_ecc = get("TPCDAQ_FAKE_ECC_BIN");
    let graw = get("TPCDAQ_REAL_GRAW");
    let geometry = get("TPCDAQ_REAL_GEOMETRY_MINI");
    if !missing.is_empty() {
        eprintln!("SKIP {test}: 未設定の env = {}", missing.join(", "));
        return None;
    }
    Some(E2eEnv {
        root_sink: root_sink?,
        ecc_bridge: ecc_bridge?,
        fake_ecc: fake_ecc?,
        graw: graw?,
        geometry: geometry?,
    })
}

// =====================================================================
// ポート・プロセス小道具(030 から必要最小限を複製 —— 共通化の判断は 031/044)
// =====================================================================

fn free_port() -> u16 {
    let probe = TcpListener::bind("127.0.0.1:0").expect("bind probe listener");
    let port = probe.local_addr().expect("local_addr").port();
    drop(probe);
    port
}

fn free_endpoint() -> String {
    format!("tcp://127.0.0.1:{}", free_port())
}

/// 子プロセスを必ず殺すラッパ(テストが落ちても孤児にしない)。
struct Proc {
    child: Child,
    /// 起動時に stdout へ 1 行だけ出る自己申告(fake_ecc = "PROXY …" / bridge = "BIND …")。
    banner: String,
}

impl Proc {
    fn spawn_with_banner(mut command: OsCommand, prefix: &str) -> Proc {
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn child");
        let stdout = child.stdout.take().expect("child stdout");
        let mut line = String::new();
        let read = BufReader::new(stdout)
            .read_line(&mut line)
            .expect("read banner");
        assert!(read > 0, "child exited before printing its {prefix} line");
        let line = line.trim_end().to_string();
        assert!(
            line.starts_with(prefix),
            "expected a {prefix} line, got {line:?}"
        );
        Proc {
            child,
            banner: line[prefix.len()..].trim().to_string(),
        }
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn signal(pid: u32, name: &str) {
    let status = OsCommand::new("kill")
        .arg(format!("-{name}"))
        .arg(pid.to_string())
        .status()
        .expect("run kill(1)");
    assert!(status.success(), "kill -{name} {pid} failed");
}

/// C++ 側の zmq bind 失敗の終了コード(`free_port()` の TOCTOU)。
const EXIT_ZMQ: i32 = 4;
/// run 中の run_number 食い違い(SPEC §6.2-5 v1.10、root_sink.cxx `kExitRunMismatch`)。
const EXIT_RUN_MISMATCH: i32 = 6;

type SinkLog = Arc<Mutex<Vec<String>>>;

/// root_sink の実プロセス。`--bind` と `--pub` は動的、TOCTOU は 3 回まで張り直す。
struct Sink {
    child: Child,
    data_ep: String,
    pid: u32,
    log: SinkLog,
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
    fn spawn(bin: &Path, geometry: &Path, output_root: &Path) -> Sink {
        for _ in 0..3 {
            let data_ep = free_endpoint();
            let pub_ep = free_endpoint();
            let mut child = OsCommand::new(bin)
                .args([
                    "--bind",
                    &data_ep,
                    "--pub",
                    &pub_ep,
                    "--geometry",
                    &geometry.to_string_lossy(),
                    "--output-root",
                    &output_root.to_string_lossy(),
                    "--expect",
                    "0:0",
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn root_sink");
            let log: SinkLog = Arc::new(Mutex::new(Vec::new()));
            let stderr = child.stderr.take().expect("root_sink stderr");
            std::thread::spawn({
                let log = Arc::clone(&log);
                move || {
                    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                        eprintln!("{line}"); // 素通し
                        log.lock().expect("SinkLog mutex").push(line);
                    }
                }
            });
            std::thread::sleep(Duration::from_millis(400));
            if let Some(status) = child.try_wait().expect("try_wait") {
                if status.code() == Some(EXIT_ZMQ) {
                    eprintln!("root_sink lost the bind race on {data_ep}/{pub_ep} — retrying");
                    continue;
                }
                panic!("root_sink exited early: {status:?}");
            }
            let pid = child.id();
            return Sink {
                child,
                data_ep,
                pid,
                log,
            };
        }
        panic!("root_sink failed to bind after 3 attempts");
    }

    fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn wait_for_exit(&mut self, timeout: Duration) -> std::process::ExitStatus {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait().expect("try_wait") {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "root_sink did not exit within {timeout:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// 終了時に stdout へ 1 行だけ出るカウンタ JSON。
    fn read_counts(&mut self) -> Value {
        let mut out = String::new();
        if let Some(mut stdout) = self.child.stdout.take() {
            let _ = stdout.read_to_string(&mut out);
        }
        let line = out.lines().last().unwrap_or_default();
        serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("root_sink stdout is not one JSON line: {out:?} ({e})"))
    }

    /// SIGTERM → 終了 JSON(正常終了を期待する)。
    fn terminate(&mut self, timeout: Duration) -> Value {
        if self.alive() {
            signal(self.pid, "TERM");
        }
        let status = self.wait_for_exit(timeout);
        let counts = self.read_counts();
        assert!(
            status.success(),
            "root_sink exited with {status:?}; counters={counts}"
        );
        counts
    }

    fn stderr_has(&self, needle: &str) -> bool {
        self.log
            .lock()
            .expect("SinkLog mutex")
            .iter()
            .any(|line| line.contains(needle))
    }

    fn wait_for_stderr(&self, needle: &str, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.stderr_has(needle) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        self.stderr_has(needle)
    }
}

fn count(counts: &Value, key: &str) -> u64 {
    counts[key]
        .as_u64()
        .unwrap_or_else(|| panic!("counter {key} missing in {counts}"))
}

// =====================================================================
// 手書き HTTP クライアント(依存を増やさない)
// =====================================================================

fn http(method: &str, addr: SocketAddr, path: &str, body: Option<&str>) -> (u16, Value) {
    let payload = body.unwrap_or("");
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{payload}",
        payload.len()
    );
    let mut stream = TcpStream::connect(addr).expect("connect to the controller");
    stream
        .set_read_timeout(Some(Duration::from_secs(180)))
        .unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).unwrap();
    let text = String::from_utf8_lossy(&raw).to_string();
    let status: u16 = text
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| panic!("no status line in {text:?}"));
    let body = text
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or_default();
    (status, serde_json::from_str(&body).unwrap_or(Value::Null))
}

/// blocking な HTTP をランタイムのワーカーで塞がない(controller も同じランタイム上に居る)。
async fn http_async(
    method: &str,
    addr: SocketAddr,
    path: &str,
    body: Option<String>,
) -> (u16, Value) {
    let method = method.to_string();
    let path = path.to_string();
    tokio::task::spawn_blocking(move || http(&method, addr, &path, body.as_deref()))
        .await
        .expect("http task")
}

// =====================================================================
// ファイル小道具
// =====================================================================

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tpcdaq_p3_err_{}_{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn names_in(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

fn wait_for_file(path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.is_file() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    path.is_file()
}

fn cleanup(scratch: &Path) {
    let _ = std::fs::remove_dir_all(scratch);
}

fn read_logbook(path: &Path) -> Vec<Value> {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("read logbook {}: {e}", path.display()));
    text.lines()
        .map(|line| {
            serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("logbook line is not JSON: {line:?} ({e})"))
        })
        .collect()
}

fn get_status_blocking(endpoint: &str) -> CommandResponse {
    let context = zmq::Context::new();
    tpcdaq::command::request(
        &context,
        endpoint,
        &Command::GetStatus,
        Duration::from_secs(5),
    )
    .unwrap_or_else(|e| panic!("GetStatus {endpoint}: {e}"))
}

// =====================================================================
// リンク保持 CoBo(**実機 zCoBo の忠実な模擬** —— TODO/032 の確定事実)
// =====================================================================

/// receiver のデータポートへ繋ぎ、`.graw` を丸ごと流したあとも **TCP を閉じずに持ち続ける**
/// クライアント。実 GET の `ecc stop` は `daqStop`(割り込み + flush)だけで
/// `daqDisconnect` を呼ばないので、実機の CoBo は停止後もリンクを張ったまま黙る ——
/// 「EOF が来ない」= 強制 EOS が正規経路、という 033 の前提そのものを作る。
///
/// `graw_replay` は転送後に**閉じる**(= 自然 EOF)ので、この経路の模擬には使えない。
struct LinkHoldingCobo {
    /// drop されるまで開いたまま(= EOF を送らない)。
    _stream: TcpStream,
    sent_bytes: u64,
}

impl LinkHoldingCobo {
    fn connect_and_send(address: &str, graw: &Path) -> LinkHoldingCobo {
        let bytes = std::fs::read(graw).unwrap_or_else(|e| panic!("read {}: {e}", graw.display()));
        let addr: SocketAddr = address
            .parse()
            .unwrap_or_else(|e| panic!("receiver data address {address:?}: {e}"));
        let mut stream = TcpStream::connect(addr).expect("connect to the receiver data port");
        stream.write_all(&bytes).expect("write the whole graw");
        stream.flush().expect("flush");
        LinkHoldingCobo {
            _stream: stream,
            sent_bytes: bytes.len() as u64,
        }
    }
}

// =====================================================================
// トポロジー(実配線。monitor / WS は 033 の関心外なので上げない)
// =====================================================================

struct Component {
    name: &'static str,
    command_endpoint: String,
    shutdown: broadcast::Sender<()>,
    handle: tokio::task::JoinHandle<()>,
}

struct Topology {
    scratch: PathBuf,
    data_root: PathBuf,
    #[allow(dead_code)]
    fake_ecc: Proc,
    #[allow(dead_code)]
    bridge: Proc,
    sink: Sink,
    components: Vec<Component>,
    rest: SocketAddr,
    controller_shutdown: broadcast::Sender<()>,
}

/// シナリオ毎に変えたいところだけ。
struct TopologyOptions {
    /// controller の EOS 待ちハード上限(SPEC §1.3、既定 5 s)。
    eos_timeout: Duration,
    /// 受信静止(quiesce)判定時間(SPEC §1.3 v1.12、既定 500 ms)。
    eos_quiesce: Duration,
    /// コンポーネントへの REQ 1 本の上限(**不達 receiver 相手ではこれがそのまま
    /// 待ち時間になる** —— 停止経路は `status_timeout` ではなくこちらを使う。
    /// `status_timeout` は REST の `/api/status` 専用)。
    command_timeout: Duration,
}

impl Default for TopologyOptions {
    fn default() -> Self {
        Self {
            eos_timeout: Duration::from_secs(5),
            eos_quiesce: Duration::from_millis(tpcdaq::config::DEFAULT_CONTROLLER_EOS_QUIESCE_MS),
            command_timeout: Duration::from_secs(2),
        }
    }
}

impl Topology {
    async fn start(env: &E2eEnv, tag: &str, options: TopologyOptions) -> Topology {
        let scratch = scratch_dir(tag);
        let data_root = scratch.join("data");
        std::fs::create_dir_all(&data_root).expect("create data root");

        // --- 1. fake-ECC(Ice servant)→ ecc-bridge(ZMQ REP)---
        //
        // `--no-data-link`: fake-ECC の start() が張って保持する TCP は receiver の
        // accept 枠を埋める「幻の CoBo」になる(030 裁定①)。ここでは
        // `LinkHoldingCobo` が CoBo 役なので必ず切る。制御プレーンは実配線のまま。
        let mut fake_cmd = OsCommand::new(&env.fake_ecc);
        fake_cmd.args(["--port", "0", "--no-data-link"]);
        let fake_ecc = Proc::spawn_with_banner(fake_cmd, "PROXY");
        let mut bridge_cmd = OsCommand::new(&env.ecc_bridge);
        bridge_cmd.args([
            "--bind",
            "tcp://127.0.0.1:*",
            "--ecc-proxy",
            &fake_ecc.banner,
        ]);
        let bridge = Proc::spawn_with_banner(bridge_cmd, "BIND");

        // --- 2. root-sink(C++ 実バイナリ)---
        let sink = Sink::spawn(&env.root_sink, &env.geometry, &data_root);

        // --- 3. graw-writer / decoder / receiver(プロセス内タスク)---
        let gw_pull = free_endpoint();
        let dec_pull = free_endpoint();
        let mut components = Vec::new();

        let gw_params = GrawWriterParams {
            pull_bind: gw_pull.clone(),
            command_listen: "tcp://127.0.0.1:0".to_string(),
            output_root: data_root.clone(),
            max_file_bytes: tpcdaq::config::DEFAULT_GRAW_WRITER_MAX_FILE_BYTES,
            flush_interval_ms: tpcdaq::config::DEFAULT_GRAW_WRITER_FLUSH_INTERVAL_MS,
            expected_sources: vec![0],
        };
        let (gw_shutdown, gw_rx) = broadcast::channel(1);
        let (gw_tx, gw_ep_rx) = oneshot::channel();
        let gw_handle = tokio::spawn(run_graw_writer(gw_params, gw_rx, Some(gw_tx)));
        let gw_command = gw_ep_rx.await.expect("graw-writer command endpoint");
        components.push(Component {
            name: "graw-writer",
            command_endpoint: gw_command.clone(),
            shutdown: gw_shutdown,
            handle: gw_handle,
        });

        let dec_params = DecoderParams {
            pull_bind: dec_pull.clone(),
            push_connect: sink.data_ep.clone(),
            command_listen: "tcp://127.0.0.1:0".to_string(),
            batch_max_bytes: tpcdaq::config::DEFAULT_BATCH_MAX_BYTES,
            batch_max_ms: tpcdaq::config::DEFAULT_BATCH_MAX_MS,
            heartbeat_ms: tpcdaq::config::DEFAULT_HEARTBEAT_MS,
            send_timeout_ms: tpcdaq::config::DEFAULT_DECODER_SEND_TIMEOUT_MS,
            workers: 1,
            expected_sources: vec![0],
        };
        let (dec_shutdown, dec_rx) = broadcast::channel(1);
        let (dec_tx, dec_ep_rx) = oneshot::channel();
        let dec_handle = tokio::spawn(run_decoder(dec_params, dec_rx, Some(dec_tx)));
        let dec_command = dec_ep_rx.await.expect("decoder command endpoint");
        components.push(Component {
            name: "decoder",
            command_endpoint: dec_command.clone(),
            shutdown: dec_shutdown,
            handle: dec_handle,
        });

        let recv_params = ReceiverParams {
            cobo_id: 0,
            listen: "127.0.0.1:0".to_string(),
            command_listen: "tcp://127.0.0.1:0".to_string(),
            graw_writer_endpoint: gw_pull,
            decoder_endpoint: dec_pull,
            batch_max_bytes: tpcdaq::config::DEFAULT_BATCH_MAX_BYTES,
            batch_max_ms: tpcdaq::config::DEFAULT_BATCH_MAX_MS,
            queue_frames: tpcdaq::config::DEFAULT_QUEUE_FRAMES,
            heartbeat_ms: tpcdaq::config::DEFAULT_HEARTBEAT_MS,
            hwm: tpcdaq::zmq_helper::DEFAULT_HWM,
        };
        let (recv_shutdown, recv_rx) = broadcast::channel(1);
        let (recv_tx, recv_ep_rx) = oneshot::channel();
        let recv_handle = tokio::spawn(run_receiver(recv_params, recv_rx, Some(recv_tx)));
        let receiver_command = recv_ep_rx.await.expect("receiver command endpoint");
        components.push(Component {
            name: "receiver0",
            command_endpoint: receiver_command.clone(),
            shutdown: recv_shutdown,
            handle: recv_handle,
        });

        // --- 4. controller(REST、同一プロセスの tokio タスク)---
        let params = ControllerParams {
            rest_listen: "127.0.0.1:0".to_string(),
            passphrase: PASSPHRASE.to_string(),
            log_pull_bind: "tcp://127.0.0.1:*".to_string(),
            ui_dir: None,
            config_ids: ConfigIds::same("p3-error-paths"),
            output_root: data_root.clone(),
            geometry_path: env.geometry.clone(),
            cobos: vec![CoboSpec {
                id: 0,
                listen: "127.0.0.1:0".to_string(),
                data_sender_id: "CoBo[0]".to_string(),
            }],
            components: vec![
                ComponentEndpoint {
                    name: "graw-writer".to_string(),
                    endpoint: gw_command,
                    kind: ComponentKind::GrawWriter,
                },
                ComponentEndpoint {
                    name: "decoder".to_string(),
                    endpoint: dec_command,
                    kind: ComponentKind::Decoder,
                },
                ComponentEndpoint {
                    name: "receiver0".to_string(),
                    endpoint: receiver_command,
                    kind: ComponentKind::Receiver { cobo_id: 0 },
                },
            ],
            ecc_endpoint: bridge.banner.clone(),
            router_ip: None,
            eos_timeout: options.eos_timeout,
            eos_quiesce: options.eos_quiesce,
            eos_poll: Duration::from_millis(100),
            command_timeout: options.command_timeout,
            ecc_timeout: Duration::from_secs(30),
            status_timeout: Duration::from_secs(1),
        };
        let (controller_shutdown, controller_rx) = broadcast::channel(1);
        let (rest_tx, rest_rx) = oneshot::channel();
        tokio::spawn(async move {
            if let Err(e) = run_controller(params, controller_rx, Some(rest_tx)).await {
                panic!("controller failed: {e}");
            }
        });
        let BoundEndpoints { rest, .. } = rest_rx.await.expect("controller REST bind");

        Topology {
            scratch,
            data_root,
            fake_ecc,
            bridge,
            sink,
            components,
            rest,
            controller_shutdown,
        }
    }

    async fn acquire(&self) -> String {
        let (status, body) = http_async(
            "POST",
            self.rest,
            "/api/control/acquire",
            Some(json!({"operator": OPERATOR, "passphrase": PASSPHRASE}).to_string()),
        )
        .await;
        assert_eq!(status, 200, "acquire failed: {body}");
        body["token"].as_str().expect("token").to_string()
    }

    async fn status(&self) -> Value {
        let (status, body) = http_async("GET", self.rest, "/api/status", None).await;
        assert_eq!(status, 200, "status failed: {body}");
        body
    }

    /// receiver が Arm で実際に bind したデータポート(CoBo 役の接続先)。
    async fn receiver_data_address(&self) -> String {
        let status = self.status().await;
        let components = status["components"].as_array().expect("components");
        let receiver = components
            .iter()
            .find(|c| c["name"] == "receiver0")
            .unwrap_or_else(|| panic!("receiver0 が /api/status に居ない: {status}"));
        receiver["metrics"]["bind_address"]
            .as_str()
            .unwrap_or_else(|| panic!("receiver0 に bind_address が無い(Arm 済みか): {receiver}"))
            .to_string()
    }

    fn endpoint_of(&self, name: &str) -> String {
        self.components
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("component {name} が居ない"))
            .command_endpoint
            .clone()
    }

    /// receiver の in-process task を **abort**(= 突然死)。
    ///
    /// SIGSTOP は使わない(REP ごと固まるので「到達可能だが EOS を出さない」の再現に
    /// ならない)。controller の観測面では REQ タイムアウトも接続拒否も同じ `Err` なので、
    /// task の abort で観測等価(030 の発見 / 033 論点 3)。
    fn kill_receiver(&self) {
        let receiver = self
            .components
            .iter()
            .find(|c| c.name == "receiver0")
            .expect("receiver0");
        receiver.handle.abort();
    }

    fn logbook_path(&self) -> PathBuf {
        self.data_root.join("logbook.jsonl")
    }

    fn run_dir(&self, run: u32) -> PathBuf {
        self.data_root.join(format!("run{run:04}"))
    }

    /// Rust 側を全部畳む(root_sink は呼び手が扱う)。
    async fn shutdown_rust(&mut self) {
        let _ = self.controller_shutdown.send(());
        for component in self.components.drain(..).rev() {
            let _ = component.shutdown.send(());
            if component.handle.is_finished() {
                continue; // abort 済み(F1)
            }
            if tokio::time::timeout(Duration::from_secs(15), component.handle)
                .await
                .is_err()
            {
                panic!("{} did not stop within 15 s", component.name);
            }
        }
    }
}

/// receiver が期待フレーム数を読み終えるまで待つ(閉塞を 120 s ハングにしない)。
async fn wait_for_frames(endpoint: &str, want: u64, timeout: Duration) -> u64 {
    let deadline = Instant::now() + timeout;
    let mut frames = 0;
    while Instant::now() < deadline {
        frames = tokio::task::spawn_blocking({
            let endpoint = endpoint.to_string();
            move || {
                get_status_blocking(&endpoint)
                    .metrics
                    .and_then(|m| m["frames"].as_u64())
                    .unwrap_or(0)
            }
        })
        .await
        .expect("receiver GetStatus task");
        if frames >= want {
            return frames;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    frames
}

// =====================================================================
// F0 — 実機の正規停止経路(リンク保持 → 強制 EOS)
// =====================================================================

/// TODO/033 論点 3 の F0。**「異常系」ではなく実機の正常系**の初照合である。
///
/// 手順: run/start → リンク保持クライアントで実 .graw を全量送信(**EOF しない**)→
/// 全フレーム到達を確認 → run/stop。
///
/// 照合:
/// * `reason="normal"` / `ok=true` / **`forced_eos=true`**(EOF が来ないので EOS を注入した)
///   / **`eos_closed=true`**(注入した EOS が流れ切った)—— REST 応答と `run_stop` レコードの
///   両方(033-A)。
/// * `run{N}.root`(entries=108)+ `run{N}_monitor.root` が出来る。
/// * 保存系のロスレスカウンタが全部 0 = **尻尾を落としていない**(033-E の静止検出が
///   入っても在飛データを飲み切ってから畳んでいることの受け入れ)。
/// * 停止所要を実測して stderr に出す(033-E の効き。assert はしない)。
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn f0_the_real_stop_path_holds_the_link_and_closes_the_run_with_a_forced_eos() {
    let Some(env) = e2e_env("F0") else {
        return;
    };
    let mut topology = Topology::start(&env, "f0", TopologyOptions::default()).await;

    let token = topology.acquire().await;
    let (status, body) = http_async(
        "POST",
        topology.rest,
        "/api/run/start",
        Some(json!({"token": token, "comment": "f0 link-holding cobo"}).to_string()),
    )
    .await;
    assert_eq!(status, 200, "run/start failed: {body}");
    let run = body["run"].as_u64().expect("run number") as u32;
    assert_eq!(run, 1, "state ファイル不在なので run は 1 から");

    // --- CoBo 役(リンクを閉じない)---
    let data_address = topology.receiver_data_address().await;
    let graw = env.graw.clone();
    let cobo = tokio::task::spawn_blocking(move || {
        LinkHoldingCobo::connect_and_send(&data_address, &graw)
    })
    .await
    .expect("link-holding cobo task");
    assert_eq!(
        cobo.sent_bytes, REAL_GRAW_BYTES,
        "入力バイト数オラクル(SPEC §12-1)"
    );

    let receiver_endpoint = topology.endpoint_of("receiver0");
    let frames = wait_for_frames(
        &receiver_endpoint,
        REAL_GRAW_FRAMES,
        Duration::from_secs(30),
    )
    .await;
    assert_eq!(
        frames, REAL_GRAW_FRAMES,
        "receiver が全フレームを読み切っていない(受け口の占有を疑う)"
    );

    // **EOF は来ない**: リンクは開いたままなので、この時点で run はまだ閉じていない。
    let run_file = topology.run_dir(run).join(format!("run{run:04}.root"));
    let monitor_file = topology
        .run_dir(run)
        .join(format!("run{run:04}_monitor.root"));
    assert!(
        !run_file.is_file(),
        "リンクを閉じていないのに run が finalize されている(EOF が来ている?)"
    );

    // --- run 停止(ここで初めて EOS が注入される)---
    let stop_started = Instant::now();
    let (status, stop_body) = http_async(
        "POST",
        topology.rest,
        "/api/run/stop",
        Some(json!({"token": token}).to_string()),
    )
    .await;
    let stop_elapsed = stop_started.elapsed();
    assert_eq!(status, 200, "run/stop failed: {stop_body}");

    assert_eq!(stop_body["ok"], json!(true), "stop: {stop_body}");
    assert_eq!(stop_body["reason"], json!("normal"), "stop: {stop_body}");
    assert_eq!(
        stop_body["forced_eos"],
        json!(true),
        "EOF が来ない実機経路なのに EOS を注入していない: {stop_body}"
    );
    assert_eq!(
        stop_body["eos_closed"],
        json!(true),
        "注入した EOS が流れ切っていない: {stop_body}"
    );

    // --- 保存の完成 ---
    assert!(
        wait_for_file(&run_file, Duration::from_secs(60)),
        "{} が出来なかった(dir={:?})",
        run_file.display(),
        names_in(&topology.run_dir(run))
    );
    assert!(
        wait_for_file(&monitor_file, Duration::from_secs(15)),
        "{} が出来なかった(dir={:?})",
        monitor_file.display(),
        names_in(&topology.run_dir(run))
    );

    // --- ログブック(033-A: run_stop に 2 値が載る)---
    let lines = read_logbook(&topology.logbook_path());
    let stop = lines
        .iter()
        .find(|l| l["type"] == "run_stop")
        .unwrap_or_else(|| panic!("run_stop が無い: {lines:?}"));
    assert_eq!(stop["run"], run);
    assert_eq!(stop["ok"], json!(true), "{stop}");
    assert_eq!(stop["reason"], json!("normal"), "{stop}");
    assert_eq!(stop["forced_eos"], json!(true), "{stop}");
    assert_eq!(stop["eos_closed"], json!(true), "{stop}");
    // REST 応答と 1 文字も違わないこと(台帳と応答が食い違わない)。
    assert_eq!(stop["forced_eos"], stop_body["forced_eos"]);
    assert_eq!(stop["eos_closed"], stop_body["eos_closed"]);

    // --- 尻尾を落としていないこと(run_stop.counters は Stop/Reset の前に採られる)---
    assert_eq!(
        stop["counters"]["frames"]["0"], REAL_GRAW_FRAMES,
        "受信フレーム = 108 データ + ctrl 1: {stop}"
    );
    assert_eq!(stop["counters"]["overflow_frames"], json!(0), "{stop}");
    assert_eq!(stop["counters"]["malformed"], json!(0), "{stop}");

    // decoder は Reset 後もコア(= カウンタ)を手放さないので停止後に読める。
    let decoder_endpoint = topology.endpoint_of("decoder");
    let decoder = tokio::task::spawn_blocking(move || {
        get_status_blocking(&decoder_endpoint)
            .metrics
            .unwrap_or(Value::Null)
    })
    .await
    .expect("decoder GetStatus task");
    assert_eq!(decoder["frames_in"], REAL_GRAW_FRAMES, "{decoder}");
    assert_eq!(decoder["fragments_out"], REAL_GRAW_EVENTS, "{decoder}");
    assert_eq!(decoder["items_out"], REAL_GRAW_ITEMS, "{decoder}");
    assert_eq!(decoder["seq_gaps"], 0, "{decoder}");
    assert_eq!(decoder["batches_abandoned"], 0, "{decoder}");
    assert_eq!(decoder["eos_abandoned"], 0, "{decoder}");
    assert_eq!(decoder["eos_in"], 1, "{decoder}");
    // 033-C: 最後のホップ(decoder → root-sink)を実配線で 1 回だけ通ったこと。
    assert_eq!(decoder["eos_out"], 1, "{decoder}");

    // --- root-sink のカウンタ(保存系オラクル)---
    topology.shutdown_rust().await;
    let counts = topology.sink.terminate(Duration::from_secs(60));
    assert_eq!(count(&counts, "fragments"), REAL_GRAW_EVENTS, "{counts}");
    assert_eq!(count(&counts, "items"), REAL_GRAW_ITEMS, "{counts}");
    assert_eq!(
        count(&counts, "events_complete"),
        REAL_GRAW_EVENTS,
        "{counts}"
    );
    assert_eq!(count(&counts, "events_incomplete"), 0, "{counts}");
    assert_eq!(count(&counts, "late_fragments"), 0, "{counts}");
    assert_eq!(count(&counts, "items_out_of_range"), 0, "{counts}");
    assert_eq!(
        count(&counts, "entries_written"),
        REAL_GRAW_EVENTS,
        "{counts}"
    );
    assert_eq!(count(&counts, "runs"), 1, "{counts}");
    assert_eq!(counts["fatal"], "", "{counts}");
    let root_files = counts["root_files"].as_array().expect("root_files");
    assert_eq!(root_files.len(), 1, "run 毎単一 ROOT: {counts}");
    assert_eq!(
        root_files[0]["entries"].as_u64(),
        Some(REAL_GRAW_EVENTS),
        "{counts}"
    );

    eprintln!(
        "F0: run/stop 所要 = {:.3} s(033-E 前の実測 = {RUN_STOP_BEFORE_QUIESCE_S} s / TODO/041)",
        stop_elapsed.as_secs_f64()
    );
    cleanup(&topology.scratch);
}

// =====================================================================
// F1 + F2 — eos-timeout と、その後の遅発 fatal
// =====================================================================

/// decoder のワイヤ形式そのままの Batch を 1 通組む(**次の run の decoder が送る
/// 最初の 1 通とバイト等価**)。source_id = 100(SPEC §3.2 の decoder)、seq = 0。
fn decoder_batch(run_number: u32) -> Vec<u8> {
    // well-formed な最小フラグメント(cobo 0 / asad 0 = root_sink の `--expect 0:0`)。
    // 値はすべて非対称(取り違え検出用)。
    let items = tpcdaq::msg::items_to_bytes(&[
        tpcdaq::msg::pack_item(0, 3, 7, 1234).expect("pack"),
        tpcdaq::msg::pack_item(1, 11, 9, 2345).expect("pack"),
    ]);
    let fragment = Fragment {
        event_idx: 0,
        event_time: 42,
        cobo: 0,
        asad: 0,
        frame_type: 2,
        revision: 5,
        read_offset: 0,
        status: 0,
        mult: [1, 2, 3, 4],
        window_out: 0,
        last_cell: [5, 6, 7, 8],
        items: ByteBuf::from(items),
    };
    let message: Message<Fragments> = Message::Data(Batch {
        source_id: tpcdaq::config::DECODER_SOURCE_ID,
        run_number,
        sequence_number: 0,
        created_ns: 1,
        payload: vec![fragment],
    });
    message.to_msgpack().expect("encode the decoder batch")
}

/// PUSH で 1 通だけ root-sink へ送る(decoder の口とバイト等価)。
fn push_one(endpoint: &str, bytes: &[u8]) {
    let context = zmq::Context::new();
    let socket = context.socket(zmq::PUSH).expect("PUSH socket");
    tpcdaq::zmq_helper::apply_push_hwm(&socket).expect("PUSH HWM");
    socket.set_sndtimeo(5_000).expect("sndtimeo");
    socket.connect(endpoint).expect("connect to root_sink");
    socket.send(bytes, 0).expect("send one batch");
    // PUSH は非同期なので、閉じる前に送り切る猶予を置く(linger 既定に任せない)。
    std::thread::sleep(Duration::from_millis(300));
}

/// **F1**: mode 確定の後に receiver を落とすと `error:eos-timeout` へ正直に到達する。
/// **F2**: その run が**開いたまま**残った root-sink へ、次 run の最初の Data 相当を
/// 1 通投げると **exit 6(run-number-mismatch)で遅発 fatal** になる。
///
/// F2 の入力は F1 が残した終了状態そのものなので、同じ root_sink プロセスに対して
/// 続けて行う(TODO/033 論点 3 の指定どおり)。
///
/// タイミング規律(033 の指定): `eos_timeout = 4 s` / `eos_quiesce_ms = 2000`。
/// kill 窓は「`collect_status` 完了(ms 級)〜 静止判定の下限 2 s」で、実際に落とすのは
/// **0.8 s** —— 両側に 0.8 s 以上のマージンがある = レースではない。
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn f1_eos_timeout_leaves_the_run_open_and_f2_the_next_run_hits_a_late_fatal() {
    let Some(env) = e2e_env("F1/F2") else {
        return;
    };
    let mut topology = Topology::start(
        &env,
        "f1",
        TopologyOptions {
            eos_timeout: Duration::from_secs(4),
            eos_quiesce: Duration::from_millis(2000),
            // 死んだ receiver への REQ はこの時間を丸ごと燃やす(判定そのものは
            // 変わらないが、実時間が伸びるだけ)。
            command_timeout: Duration::from_secs(1),
        },
    )
    .await;

    let token = topology.acquire().await;
    let (status, body) = http_async(
        "POST",
        topology.rest,
        "/api/run/start",
        Some(json!({"token": token, "comment": "f1 eos-timeout"}).to_string()),
    )
    .await;
    assert_eq!(status, 200, "run/start failed: {body}");
    let run = body["run"].as_u64().expect("run number") as u32;

    // --- データを 1 run 分流す(root-sink に run を開かせる)---
    let data_address = topology.receiver_data_address().await;
    let graw = env.graw.clone();
    let _cobo = tokio::task::spawn_blocking(move || {
        LinkHoldingCobo::connect_and_send(&data_address, &graw)
    })
    .await
    .expect("link-holding cobo task");
    let receiver_endpoint = topology.endpoint_of("receiver0");
    let frames = wait_for_frames(
        &receiver_endpoint,
        REAL_GRAW_FRAMES,
        Duration::from_secs(30),
    )
    .await;
    assert_eq!(
        frames, REAL_GRAW_FRAMES,
        "receiver が全フレームを読めていない"
    );
    assert!(
        topology
            .sink
            .wait_for_stderr(&format!("run {run} opened"), Duration::from_secs(30)),
        "root_sink が run {run} を開いていない"
    );

    // --- run/stop を投げ、mode 確定の後に receiver を殺す ---
    let rest = topology.rest;
    let stop_token = token.clone();
    let stop_started = Instant::now();
    let stop = tokio::spawn(async move {
        http_async(
            "POST",
            rest,
            "/api/run/stop",
            Some(json!({"token": stop_token}).to_string()),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(800)).await;
    topology.kill_receiver();

    let (status, stop_body) = stop.await.expect("run/stop task");
    let stop_elapsed = stop_started.elapsed();
    assert_eq!(status, 200, "run/stop failed: {stop_body}");
    eprintln!(
        "F1: run/stop 所要 = {:.3} s(静止判定 2 s + ハード上限 4 s + 不達 receiver への \
         GetStatus/Stop タイムアウトの合計。正常系 = F0 とは別物)",
        stop_elapsed.as_secs_f64()
    );

    // --- F1 の照合(033 論点 2 の意味論)---
    assert_eq!(stop_body["ok"], json!(false), "stop: {stop_body}");
    assert_eq!(
        stop_body["reason"],
        json!("error:eos-timeout"),
        "mode 確定後に殺したのだから abort ではなく eos-timeout: {stop_body}"
    );
    assert_eq!(stop_body["forced_eos"], json!(true), "stop: {stop_body}");
    assert_eq!(
        stop_body["eos_closed"],
        json!(false),
        "**異常の印はここ**: {stop_body}"
    );
    let notes = stop_body["notes"].as_array().expect("notes");
    assert!(
        notes.iter().any(|n| n
            .as_str()
            .is_some_and(|n| n.contains("EOS did not propagate within 4000 ms"))),
        "eos-timeout の事実が note に無い: {notes:?}"
    );
    assert!(
        notes.iter().any(|n| n
            .as_str()
            .is_some_and(|n| n.contains("receiver0") && n.contains("Stop"))),
        "強制 EOS の Stop が不達だった事実が note に無い: {notes:?}"
    );

    // ログブック: run_stop の 2 値 + audit の error 欄(033-A)。
    let lines = read_logbook(&topology.logbook_path());
    let stop_record = lines
        .iter()
        .find(|l| l["type"] == "run_stop")
        .unwrap_or_else(|| panic!("run_stop が無い: {lines:?}"));
    assert_eq!(stop_record["ok"], json!(false), "{stop_record}");
    assert_eq!(
        stop_record["reason"],
        json!("error:eos-timeout"),
        "{stop_record}"
    );
    assert_eq!(stop_record["forced_eos"], json!(true), "{stop_record}");
    assert_eq!(stop_record["eos_closed"], json!(false), "{stop_record}");
    let audit = lines
        .iter()
        .rfind(|l| l["type"] == "audit" && l["action"] == "run/stop")
        .unwrap_or_else(|| panic!("run/stop の audit が無い: {lines:?}"));
    assert_eq!(audit["ok"], json!(false), "{audit}");
    assert!(
        audit["error"]
            .as_str()
            .is_some_and(|e| e.contains("EOS did not propagate")),
        "audit の error 欄に理由が無い: {audit}"
    );

    // --- root-sink は run を**開いたまま**(EOS バリアが失われた状態)---
    let run_dir = topology.run_dir(run);
    let names = names_in(&run_dir);
    assert!(
        names.iter().any(|n| n.starts_with("run_inprogress_")),
        "書きかけの run が残っていない: {names:?}"
    );
    assert!(
        !run_dir.join(format!("run{run:04}.root")).is_file(),
        "EOS が流れ切っていないのに finalize されている: {names:?}"
    );
    assert!(
        !run_dir.join(format!("run{run:04}_monitor.root")).is_file(),
        "run が閉じていないのに monitor.root がある: {names:?}"
    );
    assert!(topology.sink.alive(), "root_sink がここで死んでいる");

    // Rust コンポーネントは全部回収できる(controller が畳み切っている)。
    topology.shutdown_rust().await;

    // =================================================================
    // F2 — 次 run の最初の Data で遅発 fatal(SPEC §1.3 v1.12 / §6.2-5)
    // =================================================================
    let next_run = run + 1;
    push_one(&topology.sink.data_ep, &decoder_batch(next_run));

    let status = topology.sink.wait_for_exit(Duration::from_secs(30));
    assert_eq!(
        status.code(),
        Some(EXIT_RUN_MISMATCH),
        "遅発 fatal は exit 6(run-number-mismatch)のはず: {status:?}"
    );
    assert!(
        topology.sink.stderr_has("FATAL run-number-mismatch"),
        "fatal の理由が stderr に出ていない"
    );

    let counts = topology.sink.read_counts();
    assert_eq!(counts["fatal"], "run-number-mismatch", "{counts}");
    // **run 1 のカウンタは保全されている**(落ちた瞬間の実績を捨てない)。
    assert_eq!(count(&counts, "fragments"), REAL_GRAW_EVENTS, "{counts}");
    assert_eq!(count(&counts, "items"), REAL_GRAW_ITEMS, "{counts}");
    assert_eq!(
        count(&counts, "events_complete"),
        REAL_GRAW_EVENTS,
        "{counts}"
    );
    assert_eq!(count(&counts, "run_number_mismatch"), 1, "{counts}");
    // 開いたままの run は finalize されない(完成 run 名に化けさせない)。
    let names = names_in(&run_dir);
    assert!(
        names.iter().any(|n| n.starts_with("run_inprogress_")),
        "fatal の後も書きかけのままであること: {names:?}"
    );
    assert!(
        !run_dir.join(format!("run{run:04}.root")).is_file(),
        "fatal なのに finalize されている: {names:?}"
    );

    eprintln!("F1/F2: {counts}");
    cleanup(&topology.scratch);
}
