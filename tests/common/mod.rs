//! E2E ハーネス共通部品(TODO/031 追記 1 — 044 レビューからの移管)。
//!
//! `tests/p3_e2e.rs` と `tests/p3_error_paths.rs` は **834 行が逐語一致**していた
//! (`comm -12` 実測)。TODO/031 のスモーク(`tests/soak_smoke.rs`)が 3 番目の利用者に
//! なるので、rule of three でここへ抜き出す。
//!
//! **ここに置くのは「両方の E2E で 1 文字も違わなかった部品」だけ**である:
//!
//! * env ゲート([`E2eEnv`] / [`e2e_env`])
//! * ポート確保([`free_port`] / [`free_endpoint`])
//! * 子プロセス小道具([`Proc`] / [`signal`])
//! * root_sink の実プロセス([`Sink`] —— **型差はスーパーセットで吸収**、下記)
//! * 手書き HTTP([`http`] / [`http_async`] —— **この 2 ファイルのペア専用**。
//!   `tests/controller_integration.rs` 側の同名関数は read timeout とパース失敗時の型が
//!   違う別物なので、そちらとは共有しない)
//! * ファイル小道具([`scratch_dir`] / [`names_in`] / [`wait_for_file`] / [`cleanup`])
//! * ログブック読み([`read_logbook`])/ コンポーネント直叩き([`get_status_blocking`])
//! * プロセス内 Rust コンポーネント 3 種の起動([`Component`] / [`spawn_components`])
//!
//! **ここに置かないもの**(意図的に各テストへ残す): `Topology` 本体(monitor / WS /
//! 別プロセス controller の有無でフィールドも起動順も違う)、`TopologyOptions`、
//! `controller_toml`、WS プローブ、`LinkHoldingCobo`、シナリオ固有のオラクル。
//!
//! # `Sink` の型差(発注書の「スーパーセット署名で吸収、無理に潰さない」)
//!
//! 030 は stderr 行に**経過秒**を打って跨 run の時系列を測り(`SinkLog =
//! Vec<(f64, String)>`)、033 は行そのものを grep するだけ(`Vec<String>`)だった。
//! ここでは**経過秒つき**(情報量の多い方)に揃え、033 側が使う `stderr_has` /
//! `wait_for_stderr` は行部分だけを見る。`spawn` は 030 の `extra: &[&str]` 付き署名に
//! 揃える(033 の呼び出しは `&[]` を渡すだけ)。

// 各テストバイナリはこの共通部品の一部しか使わない(soak_smoke は Sink を上げない等)。
#![allow(dead_code)]
#![allow(clippy::unwrap_used)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as OsCommand, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::{broadcast, oneshot};
use tpcdaq::command::{Command, CommandResponse};
use tpcdaq::decoder::{run_decoder, DecoderParams};
use tpcdaq::graw_writer::{run_graw_writer, GrawWriterParams};
use tpcdaq::receiver::{run_receiver, ReceiverParams};

// =====================================================================
// 実 .graw のオラクル(SPEC §12-1 / §12-2、2026-08-13 実測)
// =====================================================================

pub const REAL_GRAW_BYTES: u64 = 30_108_684;
pub const REAL_GRAW_EVENTS: u64 = 108;
pub const REAL_GRAW_ITEMS: u64 = 15_040_512;
/// receiver が数えるフレーム = AsAd データ 108 + 先頭の制御フレーム 1(SPEC §12-1)。
pub const REAL_GRAW_FRAMES: u64 = REAL_GRAW_EVENTS + 1;

// =====================================================================
// env ゲート
// =====================================================================

/// 5 つの env(root_sink / ecc_bridge / fake_ecc / 実 .graw / 実ジオメトリ)。
pub struct E2eEnv {
    pub root_sink: PathBuf,
    pub ecc_bridge: PathBuf,
    pub fake_ecc: PathBuf,
    pub graw: PathBuf,
    pub geometry: PathBuf,
}

/// 欠けた env 名を **stderr に列挙して** `None`(silent skip を作らない — CLAUDE.md)。
pub fn e2e_env(test: &str) -> Option<E2eEnv> {
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
// ポート・プロセス小道具(024/026 の流儀)
// =====================================================================

/// 空きポートを 1 つ確保して即座に手放す(固定ポートを書かない = 並列実行に耐える)。
pub fn free_port() -> u16 {
    let probe = TcpListener::bind("127.0.0.1:0").expect("bind probe listener");
    let port = probe.local_addr().expect("local_addr").port();
    drop(probe);
    port
}

pub fn free_endpoint() -> String {
    format!("tcp://127.0.0.1:{}", free_port())
}

/// 子プロセスを必ず殺すラッパ(テストが落ちても孤児にしない)。
pub struct Proc {
    pub child: Child,
    /// 起動時に stdout へ 1 行だけ出る自己申告(fake_ecc = "PROXY …" / bridge = "BIND …")。
    pub banner: String,
}

impl Proc {
    pub fn spawn_with_banner(mut command: OsCommand, prefix: &str) -> Proc {
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

/// `kill(1)` で 1 シグナル送る(`Child::kill` は SIGKILL 固定なので使えない)。
pub fn signal(pid: u32, name: &str) {
    let status = OsCommand::new("kill")
        .arg(format!("-{name}"))
        .arg(pid.to_string())
        .status()
        .expect("run kill(1)");
    assert!(status.success(), "kill -{name} {pid} failed");
}

/// C++ 側の zmq bind 失敗の終了コード(`free_port()` の TOCTOU)。
pub const EXIT_ZMQ: i32 = 4;

/// root_sink が stderr に出した 1 行と、テスト起点からの経過秒。
///
/// root_sink は run の開閉・finalize・monitor.root 書き出しを stderr に 1 行ずつ出す。
/// **C++ 側に計測用の細工を入れずに** run 境界の時系列を測る唯一の口なので、ここで
/// タイムスタンプを打って残す(E2E-D の跨 run)。読んだ行はそのまま stderr へ素通しする。
pub type SinkLog = Arc<Mutex<Vec<(f64, String)>>>;

/// root_sink の実プロセス。`--bind` と `--pub` は動的、TOCTOU は 3 回まで張り直す。
pub struct Sink {
    pub child: Child,
    pub data_ep: String,
    pub pub_ep: String,
    pub pid: u32,
    /// stderr の 1 行ずつ(経過秒つき)。
    pub log: SinkLog,
    /// 経過秒の起点。
    pub epoch: Instant,
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
    pub fn spawn(bin: &Path, geometry: &Path, output_root: &Path, extra: &[&str]) -> Sink {
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
                .args(extra)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn root_sink");
            let epoch = Instant::now();
            let log: SinkLog = Arc::new(Mutex::new(Vec::new()));
            let stderr = child.stderr.take().expect("root_sink stderr");
            std::thread::spawn({
                let log = Arc::clone(&log);
                move || {
                    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                        eprintln!("{line}"); // 素通し(既存の見え方を変えない)
                        log.lock()
                            .expect("SinkLog mutex")
                            .push((epoch.elapsed().as_secs_f64(), line));
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
                pub_ep,
                pid,
                log,
                epoch,
            };
        }
        panic!("root_sink failed to bind after 3 attempts");
    }

    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// SIGTERM → 終了 JSON 1 行を読む(root_sink は SIGINT/SIGTERM とも graceful)。
    pub fn terminate(&mut self, timeout: Duration) -> Value {
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

    pub fn wait_for_exit(&mut self, timeout: Duration) -> std::process::ExitStatus {
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
    pub fn read_counts(&mut self) -> Value {
        let mut out = String::new();
        if let Some(mut stdout) = self.child.stdout.take() {
            let _ = stdout.read_to_string(&mut out);
        }
        let line = out.lines().last().unwrap_or_default();
        serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("root_sink stdout is not one JSON line: {out:?} ({e})"))
    }

    /// `needle` を含む最初の stderr 行の経過秒(無ければ `None`)。
    pub fn first_at(&self, needle: &str) -> Option<f64> {
        self.log
            .lock()
            .expect("SinkLog mutex")
            .iter()
            .find(|(_, line)| line.contains(needle))
            .map(|(at, _)| *at)
    }

    /// テスト側の `Instant` を root_sink ログと同じ時間軸(経過秒)へ写す。
    pub fn seconds_since_epoch(&self, at: Instant) -> f64 {
        at.duration_since(self.epoch).as_secs_f64()
    }

    pub fn stderr_has(&self, needle: &str) -> bool {
        self.log
            .lock()
            .expect("SinkLog mutex")
            .iter()
            .any(|(_, line)| line.contains(needle))
    }

    pub fn wait_for_stderr(&self, needle: &str, timeout: Duration) -> bool {
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

pub fn count(counts: &Value, key: &str) -> u64 {
    counts[key]
        .as_u64()
        .unwrap_or_else(|| panic!("counter {key} missing in {counts}"))
}

// =====================================================================
// 手書き HTTP クライアント(依存を増やさない — 016 の統合テストと同じ流儀)
// =====================================================================

pub fn http(method: &str, addr: SocketAddr, path: &str, body: Option<&str>) -> (u16, Value) {
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
pub async fn http_async(
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

/// `prefix` はテスト毎の名前空間(030 = `tpcdaq_p3_e2e` / 033 = `tpcdaq_p3_err`)。
pub fn scratch_dir(prefix: &str, tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("{prefix}_{}_{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

pub fn names_in(dir: &Path) -> Vec<String> {
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

pub fn wait_for_file(path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.is_file() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    path.is_file()
}

/// `Topology::shutdown` の後に呼ぶ後片付け(scratch を消す)。
pub fn cleanup(scratch: &Path) {
    let _ = std::fs::remove_dir_all(scratch);
}

// =====================================================================
// ログブック / コンポーネントへの直接 GetStatus
// =====================================================================

pub fn read_logbook(path: &Path) -> Vec<Value> {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("read logbook {}: {e}", path.display()));
    text.lines()
        .map(|line| {
            serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("logbook line is not JSON: {line:?} ({e})"))
        })
        .collect()
}

pub fn get_status_blocking(endpoint: &str) -> CommandResponse {
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
// プロセス内 Rust コンポーネント(graw-writer / decoder / receiver)
// =====================================================================

/// テストプロセス内で動く Rust コンポーネント 1 個分の後始末。
pub struct Component {
    pub name: &'static str,
    pub command_endpoint: String,
    pub shutdown: broadcast::Sender<()>,
    pub handle: tokio::task::JoinHandle<()>,
}

/// graw-writer → decoder → receiver をこの順にプロセス内タスクとして起動する。
///
/// **全ポート動的**: コマンド REP は `tcp://127.0.0.1:0`(実ポートは各コンポーネントが
/// oneshot で自己申告)、PULL の bind だけは receiver に先に教える必要があるので
/// [`free_endpoint`] で確保した実ポートを使う(production の TOML と同じ配線。
/// 固定値は書かない)。receiver のデータポートも動的で、Arm で実ポートが決まり
/// controller が Arm 応答から回収する。
pub async fn spawn_components(data_root: &Path, sink_data_ep: &str) -> Vec<Component> {
    let gw_pull = free_endpoint();
    let dec_pull = free_endpoint();
    let mut components = Vec::new();

    let gw_params = GrawWriterParams {
        pull_bind: gw_pull.clone(),
        command_listen: "tcp://127.0.0.1:0".to_string(),
        output_root: data_root.to_path_buf(),
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
        command_endpoint: gw_command,
        shutdown: gw_shutdown,
        handle: gw_handle,
    });

    let dec_params = DecoderParams {
        pull_bind: dec_pull.clone(),
        push_connect: sink_data_ep.to_string(),
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
        command_endpoint: dec_command,
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
        command_endpoint: receiver_command,
        shutdown: recv_shutdown,
        handle: recv_handle,
    });

    components
}

/// 名前で 1 個引く(`components[2]` のような添字を書かない)。
pub fn endpoint_of(components: &[Component], name: &str) -> String {
    components
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("component {name} が居ない"))
        .command_endpoint
        .clone()
}
