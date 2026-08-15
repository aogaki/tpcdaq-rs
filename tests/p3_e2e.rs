//! P3 E2E —— run 制御の全通し・モニタ非干渉の正式計測・異常系(TODO/030、
//! SPEC §12-7 / §12-8 / §12-9 / §12-10 / §12-11)。
//!
//! 012(`tests/p2_e2e.rs`)の続き。**production コードは 1 行も足さない** —— ここは
//! 完成品(005–026)を実トポロジーで配線し、run 制御(controller REST + ecc-bridge)で
//! 駆動するだけのテストである。統合で見つかった不具合は回避せず結果節に残す。
//!
//! * **E2E-C**(§12-7 + §12-10): fake-ECC + ecc-bridge + {receiver, decoder, graw-writer,
//!   root-sink, monitor} を `POST /api/run/start` で駆動 → 実 .graw をリプレイ → EOS →
//!   `POST /api/run/stop`。logbook / 生 graw バイト一致 / `run{N}.root` / `run{N}_monitor.root` /
//!   WS 全メッセージ型を機械照合。**E2E-E(§12-9 = R10)は発注書どおりこの run クローズに
//!   同居**(最終 EOS → `run{N}_monitor.root` 完成 ≤ 10 s)。
//! * **E2E-D**(§12-8、release): モニタ非干渉の正式計測。実ジオメトリ + monitor + WS
//!   クライアント接続の完全モニタ経路で、snapshot **0 Hz / 既定 1 Hz / 2 Hz** の 3 点 ×5 回。
//!   中央値で ±2% 判定。あわせて **freeze 相当**(binary を読まない遅い WS クライアント)と
//!   **跨 run**(連続 2 run の per-run スループット + 境界の intake 停滞)。
//!
//! * **E2E-G**(§12-11): run 中に controller を SIGKILL → JSONL は最終行以外 parse 可能 →
//!   再起動後 `next_run` が重複しない。
//!
//! # 発注側の裁定(2026-08-14、TODO/030 追補)で確定したこと
//!
//! * 実 fake-ECC の `start()` が DataLinkSet の宛先へ張って**保持する** TCP は、
//!   リプレイ試験では「**CoBo が 2 台**いる」状態になる(receiver の accept ループは
//!   1 接続ずつ)。→ `fake_ecc` に **`--no-data-link`(既定 OFF)** を足し、データを流す
//!   シナリオはそれを使う。**制御プレーンは実配線のまま**通す(§12-7 の主張を保つ)。
//!   016/017 の listen-before-start 負性テストは既定 ON のままなので不変。
//! * **E2E-F(eos-timeout → root-sink fatal)は 030 から外れた**(TODO/033 へ)。
//!
//! **全ポート動的**(`TcpListener::bind(":0")` で確保 → 解放 → 使う)。同一マシンで別の
//! リプレイ経路が SPEC §3.2 の既定ポート(46005 / 47001–47005 / 47100–47110 / 9000 / 8080)を
//! 使っていても衝突しない。**この理由から Rust コンポーネント(receiver / decoder /
//! graw-writer)はテストプロセス内タスクとして起動する** —— `--config` 起動ではコマンド REP
//! ポートが定数(47100 / 47101 / 47110+k)に焼かれており動的化できないため(`src/config.rs`)。
//!
//! env が欠けたら**欠けた変数名を stderr に出して** skip する(silent skip を作らない):
//!
//! ```text
//! TPCDAQ_ROOT_SINK_BIN=$PWD/tools/root_sink/root_sink \
//! TPCDAQ_ECC_BRIDGE_BIN=$PWD/tools/ecc_bridge/ecc_bridge \
//! TPCDAQ_FAKE_ECC_BIN=$PWD/tools/ecc_bridge/fake_ecc \
//! TPCDAQ_REAL_GRAW=$HOME/TPC/CoBo_2025-09-01T08_51_06.203_0000.graw \
//! TPCDAQ_REAL_GEOMETRY_MINI=$HOME/TPC/miniTPC_UVW_pcb_info/new_geometry_mini_eTPC.dat \
//!   cargo test --release --test p3_e2e -- --test-threads=1 --nocapture
//! ```

#![allow(clippy::unwrap_used)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as OsCommand, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{broadcast, oneshot};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tpcdaq::command::{Command, CommandResponse};
use tpcdaq::config::ConfigIds;
use tpcdaq::controller::{
    run_controller, BoundEndpoints, CoboSpec, ComponentEndpoint, ComponentKind, ControllerParams,
};
use tpcdaq::decoder::{run_decoder, DecoderParams};
use tpcdaq::graw_writer::{run_graw_writer, GrawWriterParams};
use tpcdaq::monitor::{run_monitor, MonitorParams};
use tpcdaq::receiver::{run_receiver, ReceiverParams};

/// SPEC §12 末尾「mini 100 Hz 相当 ≈ 28 MB/s = 224 Mbps」。E2E-C の既定ペース。
const REPLAY_RATE_MBPS: f64 = 224.0;

/// 実 .graw の P1 オラクル(SPEC §12-1/§12-2、2026-08-13 実測)。
const REAL_GRAW_BYTES: u64 = 30_108_684;
const REAL_GRAW_EVENTS: u64 = 108;
const REAL_GRAW_ITEMS: u64 = 15_040_512;

const OPERATOR: &str = "p3-e2e";
const PASSPHRASE: &str = "p3-e2e-passphrase";

// =====================================================================
// env ゲート
// =====================================================================

/// 5 つの env(root_sink / ecc_bridge / fake_ecc / 実 .graw / 実ジオメトリ)。
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
// ポート・プロセス小道具(024/026 の流儀)
// =====================================================================

/// 空きポートを 1 つ確保して即座に手放す(固定ポートを書かない = 並列実行に耐える)。
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

/// `kill(1)` で 1 シグナル送る(`Child::kill` は SIGKILL 固定なので使えない)。
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

/// root_sink が stderr に出した 1 行と、テスト起点からの経過秒。
///
/// root_sink は run の開閉・finalize・monitor.root 書き出しを stderr に 1 行ずつ出す。
/// **C++ 側に計測用の細工を入れずに** run 境界の時系列を測る唯一の口なので、ここで
/// タイムスタンプを打って残す(E2E-D の跨 run)。読んだ行はそのまま stderr へ素通しする。
type SinkLog = Arc<Mutex<Vec<(f64, String)>>>;

/// root_sink の実プロセス。`--bind` と `--pub` は動的、TOCTOU は 3 回まで張り直す。
struct Sink {
    child: Child,
    data_ep: String,
    pub_ep: String,
    pid: u32,
    /// stderr の 1 行ずつ(経過秒つき)。
    log: SinkLog,
    /// 経過秒の起点。
    epoch: Instant,
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
    fn spawn(bin: &Path, geometry: &Path, output_root: &Path, extra: &[&str]) -> Sink {
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

    /// SIGTERM → 終了 JSON 1 行を読む(root_sink は SIGINT/SIGTERM とも graceful)。
    fn terminate(&mut self, timeout: Duration) -> Value {
        if matches!(self.child.try_wait(), Ok(None)) {
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

    fn read_counts(&mut self) -> Value {
        let mut out = String::new();
        if let Some(mut stdout) = self.child.stdout.take() {
            let _ = stdout.read_to_string(&mut out);
        }
        let line = out.lines().last().unwrap_or_default();
        serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("root_sink stdout is not one JSON line: {out:?} ({e})"))
    }

    /// `needle` を含む最初の stderr 行の経過秒(無ければ `None`)。
    fn first_at(&self, needle: &str) -> Option<f64> {
        self.log
            .lock()
            .expect("SinkLog mutex")
            .iter()
            .find(|(_, line)| line.contains(needle))
            .map(|(at, _)| *at)
    }

    /// テスト側の `Instant` を root_sink ログと同じ時間軸(経過秒)へ写す。
    fn seconds_since_epoch(&self, at: Instant) -> f64 {
        at.duration_since(self.epoch).as_secs_f64()
    }
}

fn count(counts: &Value, key: &str) -> u64 {
    counts[key]
        .as_u64()
        .unwrap_or_else(|| panic!("counter {key} missing in {counts}"))
}

// =====================================================================
// 手書き HTTP クライアント(依存を増やさない — 016 の統合テストと同じ流儀)
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
    let dir = std::env::temp_dir().join(format!("tpcdaq_p3_e2e_{}_{tag}", std::process::id()));
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

/// `bytes` を MFM フレームへ分割し、AsAd データフレームの列と非 AsAd 制御フレームの列に分ける
/// (SPEC §12-2 v1.2 のオラクル側。`tests/p2_e2e.rs` と同じ手続き)。
fn split_by_asad(bytes: &[u8]) -> (Vec<u8>, Vec<u8>, usize, usize) {
    let mut framer = tpcdaq::framer::Framer::new();
    let mut asad_column = Vec::with_capacity(bytes.len());
    let mut ctrl_column = Vec::new();
    let (mut asad_frames, mut ctrl_frames) = (0usize, 0usize);
    for chunk in bytes.chunks(8192) {
        framer.push(chunk);
        while let Some(frame) = framer.next() {
            if tpcdaq::decode::peek_asad(frame).is_some() {
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

// =====================================================================
// WS プローブ(§10.4-5 のライブ probe。026 のクライアント部品を流用)
// =====================================================================

/// 受け取った WS メッセージの要約。**バイナリは型毎に最初の 1 通だけ**保持する
/// (0x03 は 1 通 278 KB — 全部持つとテストがメモリを食う)。
#[derive(Default, Clone)]
struct WsSeen {
    json_by_type: std::collections::BTreeMap<String, Vec<Value>>,
    binary_first: std::collections::BTreeMap<u8, Vec<u8>>,
    binary_count: std::collections::BTreeMap<u8, u64>,
    bad_header: u64,
}

impl WsSeen {
    fn json(&self, kind: &str) -> &[Value] {
        self.json_by_type
            .get(kind)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    fn first(&self, msg_type: u8) -> &[u8] {
        self.binary_first
            .get(&msg_type)
            .unwrap_or_else(|| {
                panic!(
                    "WS にバイナリ型 {msg_type:#04x} が 1 通も来ていない(来た型 = {:?})",
                    self.binary_count
                )
            })
            .as_slice()
    }

    fn count_of(&self, msg_type: u8) -> u64 {
        self.binary_count.get(&msg_type).copied().unwrap_or(0)
    }
}

/// WS クライアント 1 本。`stop()` まで全メッセージを読み続けて要約を貯める。
struct WsProbe {
    seen: Arc<Mutex<WsSeen>>,
    stop: Arc<AtomicBool>,
    handle: tokio::task::JoinHandle<()>,
}

impl WsProbe {
    /// `subscribe` を送ってから読み始める(`None` なら既定購読のまま)。
    async fn connect(ws_addr: SocketAddr, subscribe: Option<&str>) -> WsProbe {
        let url = format!("ws://{ws_addr}/ws");
        let (mut socket, _) = tokio_tungstenite::connect_async(&url)
            .await
            .unwrap_or_else(|e| panic!("WS connect to {url}: {e}"));
        if let Some(streams) = subscribe {
            socket
                .send(WsMessage::Text(streams.to_string().into()))
                .await
                .expect("send subscribe");
        }
        let seen = Arc::new(Mutex::new(WsSeen::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let handle = tokio::spawn({
            let seen = Arc::clone(&seen);
            let stop = Arc::clone(&stop);
            async move {
                while !stop.load(Ordering::Relaxed) {
                    let next =
                        tokio::time::timeout(Duration::from_millis(200), socket.next()).await;
                    let Ok(Some(Ok(message))) = next else {
                        continue;
                    };
                    let mut seen = seen.lock().expect("WsSeen mutex");
                    match message {
                        WsMessage::Text(text) => {
                            if let Ok(value) = serde_json::from_str::<Value>(&text) {
                                let kind = value["type"].as_str().unwrap_or("?").to_string();
                                seen.json_by_type.entry(kind).or_default().push(value);
                            }
                        }
                        WsMessage::Binary(bytes) => {
                            if bytes.len() < 13 || &bytes[0..2] != b"TP" || bytes[3] != 2 {
                                seen.bad_header += 1;
                                continue;
                            }
                            let msg_type = bytes[2];
                            *seen.binary_count.entry(msg_type).or_insert(0) += 1;
                            seen.binary_first
                                .entry(msg_type)
                                .or_insert_with(|| bytes.to_vec());
                        }
                        _ => {}
                    }
                }
            }
        });
        WsProbe { seen, stop, handle }
    }

    /// 観測結果の**所有コピー**を返す(ロックを await 越しに持たない)。
    fn snapshot(&self) -> WsSeen {
        self.seen.lock().expect("WsSeen mutex").clone()
    }

    async fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = tokio::time::timeout(Duration::from_secs(5), self.handle).await;
    }
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn f32_at(bytes: &[u8], at: usize) -> f32 {
    f32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

// =====================================================================
// トポロジー(実配線)
// =====================================================================

/// テストプロセス内で動く Rust コンポーネント 1 個分の後始末。
struct Component {
    name: &'static str,
    command_endpoint: String,
    shutdown: broadcast::Sender<()>,
    handle: tokio::task::JoinHandle<()>,
}

/// 実配線一式。`start` で fake-ECC → ecc-bridge → root-sink → monitor →
/// graw-writer / decoder / receiver → controller の順に上げる(SPEC §1.3 の推奨起動順)。
struct Topology {
    scratch: PathBuf,
    data_root: PathBuf,
    #[allow(dead_code)]
    fake_ecc: Proc,
    #[allow(dead_code)]
    bridge: Proc,
    sink: Sink,
    monitor_ws: SocketAddr,
    monitor_shutdown: broadcast::Sender<()>,
    monitor_handle: tokio::task::JoinHandle<Result<(), tpcdaq::monitor::MonitorError>>,
    components: Vec<Component>,
    rest: SocketAddr,
    controller_shutdown: broadcast::Sender<()>,
    /// `spawned_controller` のときだけ埋まる(E2E-G が kill -9 → 再起動する相手)。
    controller_proc: Option<Child>,
    controller_config: PathBuf,
}

/// トポロジーの起動オプション(シナリオ毎に変えたいところだけ)。
struct TopologyOptions {
    /// root_sink の `--snapshot-hz`(`None` = 既定 1 Hz)。
    snapshot_hz: Option<u32>,
    /// controller の EOS 待ちタイムアウト(SPEC §1.3 既定 5 s)。
    eos_timeout: Duration,
    /// `true` = fake-ECC を `--no-data-link` で起動する(TODO/030 裁定①)。
    ///
    /// 実機では **CoBo が張る 1 本の TCP がそのままデータリンク**で、そこにデータが流れる。
    /// リプレイ試験では `graw_replay` が CoBo 役なので、fake-ECC が別途 connect すると
    /// **CoBo が 2 台**いる状態になり、receiver の accept ループ(1 接続ずつ)を
    /// 「幻の CoBo」が占有してデータが 1 バイトも読まれない(2026-08-14 実測)。
    /// データを流すシナリオ(E2E-C / E2E-D)はこれを立てる。
    /// **制御プレーンは実配線のまま**(controller → ecc-bridge → Ice → fake-ECC servant)。
    no_data_link: bool,
    /// `true` = controller を **`--config` で別プロセス起動**する(E2E-G の kill -9 用)。
    /// `false` = 同一プロセスの tokio タスク(REST ポートは動的に自己申告される)。
    spawned_controller: bool,
}

impl Default for TopologyOptions {
    fn default() -> Self {
        Self {
            snapshot_hz: None,
            eos_timeout: Duration::from_secs(5),
            no_data_link: false,
            spawned_controller: false,
        }
    }
}

impl Topology {
    async fn start(env: &E2eEnv, tag: &str, options: TopologyOptions) -> Topology {
        let scratch = scratch_dir(tag);
        let data_root = scratch.join("data");
        std::fs::create_dir_all(&data_root).expect("create data root");

        // --- 1. fake-ECC(Ice servant)→ ecc-bridge(ZMQ REP)---
        let mut fake_cmd = OsCommand::new(&env.fake_ecc);
        fake_cmd.args(["--port", "0"]);
        if options.no_data_link {
            fake_cmd.arg("--no-data-link");
        }
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
        let hz = options.snapshot_hz.map(|h| h.to_string());
        let extra: Vec<&str> = match &hz {
            Some(h) => vec!["--snapshot-hz", h.as_str()],
            None => Vec::new(),
        };
        let sink = Sink::spawn(&env.root_sink, &env.geometry, &data_root, &extra);

        // --- 3. monitor(SUB → WS)---
        let monitor_params = MonitorParams {
            sub_endpoint: sink.pub_ep.clone(),
            ws_listen: "127.0.0.1:0".to_string(),
            geometry: env.geometry.clone(),
            live_queue: tpcdaq::config::DEFAULT_MONITOR_LIVE_QUEUE,
            detector: "mini_eTPC_p3".to_string(),
            cobos: vec![0],
        };
        let (monitor_shutdown, monitor_rx) = broadcast::channel(1);
        let (monitor_bound_tx, monitor_bound_rx) = oneshot::channel();
        let monitor_handle = tokio::spawn(run_monitor(
            monitor_params,
            monitor_rx,
            Some(monitor_bound_tx),
        ));
        let monitor_ws = tokio::time::timeout(Duration::from_secs(10), monitor_bound_rx)
            .await
            .expect("monitor did not bind in time")
            .expect("monitor failed before binding");
        // SUB は slow joiner(PUB 接続が届くまでの猶予 —— 026 と同じ margin)。
        tokio::time::sleep(Duration::from_millis(400)).await;

        // --- 4. graw-writer / decoder / receiver(プロセス内タスク、コマンド REP は動的)---
        //
        // PULL の bind ポートだけは receiver に先に教える必要があるので確保した実ポートを使う
        // (production の TOML と同じ配線。固定値は書かない)。
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
            // データポートは動的。Arm で実ポートが決まり、controller が Arm 応答から回収する。
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
        let receiver_command_for_controller = receiver_command.clone();
        components.push(Component {
            name: "receiver0",
            command_endpoint: receiver_command.clone(),
            shutdown: recv_shutdown,
            handle: recv_handle,
        });

        // --- 5. controller(REST)---
        let params = ControllerParams {
            rest_listen: "127.0.0.1:0".to_string(),
            passphrase: PASSPHRASE.to_string(),
            log_pull_bind: "tcp://127.0.0.1:*".to_string(),
            ui_dir: None,
            config_ids: ConfigIds::same("p3-e2e"),
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
                    endpoint: receiver_command_for_controller,
                    kind: ComponentKind::Receiver { cobo_id: 0 },
                },
            ],
            ecc_endpoint: bridge.banner.clone(),
            router_ip: None,
            eos_timeout: options.eos_timeout,
            // TODO/033-E で新設(SPEC §1.3 v1.12)。ここは production の既定のまま
            // = 実運用と同じ停止の畳み方でシナリオを回す。
            eos_quiesce: Duration::from_millis(tpcdaq::config::DEFAULT_CONTROLLER_EOS_QUIESCE_MS),
            eos_poll: Duration::from_millis(100),
            command_timeout: Duration::from_secs(5),
            ecc_timeout: Duration::from_secs(30),
            status_timeout: Duration::from_secs(2),
        };
        let (controller_shutdown, controller_rx) = broadcast::channel(1);
        let mut controller_proc = None;
        let controller_config = scratch.join("controller.toml");
        let rest = if options.spawned_controller {
            // 実バイナリ起動(E2E-G が SIGKILL する相手)。**コンポーネントのコマンド REP は
            // 上で動的に取った実エンドポイント**を設定で渡すので、固定ポートは 1 つも使わない。
            let rest_port = free_port();
            std::fs::write(
                &controller_config,
                controller_toml(env, &data_root, &params, rest_port),
            )
            .expect("write controller config");
            let child = OsCommand::new(env!("CARGO_BIN_EXE_controller"))
                .arg("--config")
                .arg(&controller_config)
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .expect("spawn controller");
            let rest: SocketAddr = format!("127.0.0.1:{rest_port}").parse().unwrap();
            wait_for_rest(rest, Duration::from_secs(20));
            controller_proc = Some(child);
            rest
        } else {
            let (rest_tx, rest_rx) = oneshot::channel();
            tokio::spawn(async move {
                if let Err(e) = run_controller(params, controller_rx, Some(rest_tx)).await {
                    panic!("controller failed: {e}");
                }
            });
            let BoundEndpoints { rest, .. } = rest_rx.await.expect("controller REST bind");
            rest
        };

        Topology {
            scratch,
            data_root,
            fake_ecc,
            bridge,
            sink,
            monitor_ws,
            monitor_shutdown,
            monitor_handle,
            components,
            rest,
            controller_shutdown,
            controller_proc,
            controller_config,
        }
    }

    /// 操作権を取り、token を返す。
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

    /// receiver が Arm で実際に bind したデータポート(graw_replay の宛先)。
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

    fn logbook_path(&self) -> PathBuf {
        self.data_root.join("logbook.jsonl")
    }

    fn run_dir(&self, run: u32) -> PathBuf {
        self.data_root.join(format!("run{run:04}"))
    }

    /// 全部畳む(root_sink は呼び手が先に `terminate` していることが多いので寛容に)。
    async fn shutdown(mut self) -> Value {
        let counts = if matches!(self.sink.child.try_wait(), Ok(None)) {
            self.sink.terminate(Duration::from_secs(60))
        } else {
            self.sink.read_counts()
        };
        let _ = self.controller_shutdown.send(());
        if let Some(child) = self.controller_proc.as_mut() {
            if matches!(child.try_wait(), Ok(None)) {
                signal(child.id(), "INT"); // Rust バイナリの graceful stop は SIGINT のみ
            }
            let _ = child.wait();
        }
        for component in self.components.drain(..).rev() {
            let _ = component.shutdown.send(());
            if tokio::time::timeout(Duration::from_secs(15), component.handle)
                .await
                .is_err()
            {
                panic!("{} did not stop within 15 s", component.name);
            }
        }
        let _ = self.monitor_shutdown.send(());
        let _ = tokio::time::timeout(Duration::from_secs(10), self.monitor_handle).await;
        counts
    }
}

impl Topology {
    /// controller を SIGKILL(E2E-G。graceful は SIGINT だけなので、これは本当の突然死)。
    fn kill_controller(&mut self) {
        let child = self
            .controller_proc
            .as_mut()
            .expect("spawned_controller が false のトポロジー");
        assert!(
            matches!(child.try_wait(), Ok(None)),
            "controller は既に死んでいる"
        );
        signal(child.id(), "KILL");
        let status = child.wait().expect("wait controller");
        assert!(
            !status.success(),
            "SIGKILL したのに正常終了になっている: {status:?}"
        );
    }

    /// 同じ設定でもう一度上げる(state / logbook は同じディレクトリ = 再読込の実証)。
    fn restart_controller(&mut self) {
        let child = OsCommand::new(env!("CARGO_BIN_EXE_controller"))
            .arg("--config")
            .arg(&self.controller_config)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("respawn controller");
        self.controller_proc = Some(child);
        wait_for_rest(self.rest, Duration::from_secs(20));
    }
}

/// controller の REST が上がるまで待つ(ログを grep しない —— tracing の出力は
/// リダイレクトしても ANSI 色つきで `key="value"` が一致しない)。
fn wait_for_rest(addr: SocketAddr, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
            // listen しただけでは axum がまだルーティングしていないことがある。
            std::thread::sleep(Duration::from_millis(100));
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("controller REST {addr} が {timeout:?} 以内に上がらなかった");
}

/// 別プロセス controller 用の設定 TOML(SPEC §3.1)。**ポートはすべて実測した空きポート**。
fn controller_toml(
    env: &E2eEnv,
    data_root: &Path,
    params: &ControllerParams,
    rest_port: u16,
) -> String {
    let endpoint = |kind: ComponentKind| -> String {
        params
            .components
            .iter()
            .find(|c| std::mem::discriminant(&c.kind) == std::mem::discriminant(&kind))
            .map(|c| c.endpoint.clone())
            .expect("component endpoint")
    };
    format!(
        r#"[system]
experiment = "mini_eTPC"
output_root = "{output_root}"
geometry = "{geometry}"

[[cobo]]
id = 0
listen = "{cobo_listen}"
data_sender_id = "CoBo[0]"

[decoder]
workers = 1

[root_sink]
snapshot_hz = 1.0
event_publish_hz = 20.0
build_timeout_ms = 1000

[monitor]
ws_listen = "127.0.0.1:{monitor_port}"

[controller]
rest_listen = "127.0.0.1:{rest_port}"
passphrase = "{passphrase}"
ecc_proxy = "unused-by-the-controller"
config_id = "{config_id}"
log_pull_bind = "tcp://127.0.0.1:{log_port}"
eos_timeout_s = {eos_timeout_s}
graw_writer_command = "{graw_writer_command}"
decoder_command = "{decoder_command}"
receiver_commands = ["{receiver_command}"]
ecc_command = "{ecc_command}"
router_ip = "127.0.0.1"
"#,
        output_root = data_root.display(),
        geometry = env.geometry.display(),
        cobo_listen = params.cobos[0].listen,
        // `[monitor] ws_listen` は controller が読まない(monitor は別プロセス)。
        // 設定検証を通すためだけに空きポートを 1 つ入れる。
        monitor_port = free_port(),
        rest_port = rest_port,
        passphrase = PASSPHRASE,
        // このハーネスは 3 相同値なので、TOML には文字列形(略記)を書く。
        config_id = params.config_ids.configure,
        log_port = free_port(),
        eos_timeout_s = params.eos_timeout.as_secs(),
        graw_writer_command = endpoint(ComponentKind::GrawWriter),
        decoder_command = endpoint(ComponentKind::Decoder),
        receiver_command = endpoint(ComponentKind::Receiver { cobo_id: 0 }),
        ecc_command = params.ecc_endpoint,
    )
}

/// `Topology::shutdown` の後に呼ぶ後片付け(scratch を消す)。
fn cleanup(scratch: &Path) {
    let _ = std::fs::remove_dir_all(scratch);
}

// =====================================================================
// コンポーネントへの直接 GetStatus(全カウンタの採取用)
// =====================================================================

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
// ログブック
// =====================================================================

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

fn records_of_type<'a>(lines: &'a [Value], kind: &str) -> Vec<&'a Value> {
    lines.iter().filter(|l| l["type"] == kind).collect()
}

// =====================================================================
// E2E-C — 全通し正常系(SPEC §12-7 + §12-10)
// =====================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn e2e_c_full_run_through_the_controller_rest_and_the_ws_probe() {
    let Some(env) = e2e_env("E2E-C") else {
        return;
    };
    let topology = Topology::start(
        &env,
        "c",
        TopologyOptions {
            no_data_link: true,
            ..TopologyOptions::default()
        },
    )
    .await;
    let geometry = tpcdaq::geometry::load(&env.geometry).expect("load the real mini geometry");

    // --- WS クライアント(波形も含めて全ストリーム購読)---
    let probe = WsProbe::connect(
        topology.monitor_ws,
        Some(r#"{"streams":["uvw","waveforms","histos","status"]}"#),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    // --- run 開始(controller が §1.3 の全段を回す)---
    let token = topology.acquire().await;
    let (status, body) = http_async(
        "POST",
        topology.rest,
        "/api/run/start",
        Some(json!({"token": token, "comment": "p3 e2e-c"}).to_string()),
    )
    .await;
    assert_eq!(status, 200, "run/start failed: {body}");
    let run = body["run"].as_u64().expect("run number") as u32;
    assert_eq!(run, 1, "state ファイル不在なので run は 1 から");

    // --- graw_replay(Arm が回収した receiver 実ポートへ)---
    let data_address = topology.receiver_data_address().await;
    let started_at = Instant::now();
    let mut replay = OsCommand::new(env!("CARGO_BIN_EXE_graw_replay"))
        .arg(&data_address)
        .arg(&env.graw)
        .arg("--rate-mbps")
        .arg(REPLAY_RATE_MBPS.to_string())
        .spawn()
        .expect("spawn graw_replay");

    // **統合で見つかった閉塞をここで即座に可視化する**(120 s ハングにしない):
    // receiver が 1 フレームも読めていないなら、受け口は誰か別の接続に占有されている。
    let receiver_endpoint = topology.components[2].command_endpoint.clone();
    let stall_deadline = Instant::now() + Duration::from_secs(10);
    let replay_status = loop {
        if let Some(status) = replay.try_wait().expect("try_wait graw_replay") {
            break status;
        }
        let frames = tokio::task::spawn_blocking({
            let endpoint = receiver_endpoint.clone();
            move || {
                get_status_blocking(&endpoint)
                    .metrics
                    .and_then(|m| m["frames"].as_u64())
                    .unwrap_or(0)
            }
        })
        .await
        .expect("receiver GetStatus task");
        if frames == 0 && Instant::now() > stall_deadline {
            let _ = replay.kill();
            let _ = replay.wait();
            panic!(
                "receiver が 10 s 経っても 1 フレームも読めていない({data_address})。\n\
                 まず疑うのは受け口の占有: fake-ECC が `--no-data-link` なしで起動していると、\n\
                 その start() が張って保持する TCP(= 幻の CoBo)が receiver の accept 枠を\n\
                 埋め、あとから繋いだ graw_replay のデータが 1 バイトも読まれない\n\
                 (TODO/030 裁定①。receiver は 1 接続ずつしか処理しない = TODO/032)。"
            );
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    assert!(
        replay_status.success(),
        "graw_replay failed: {replay_status:?}"
    );
    // **最終 EOS の壁時計**(§12-9 = E2E-E)。graw_replay が閉じた瞬間の TCP FIN を
    // receiver が run 境界と解釈して EOS を出す(SPEC §1.3)ので、ここが起点。
    let eos_wall = SystemTime::now();

    // --- EOS が伝播して ROOT が finalize されるまで待つ ---
    let run_file = topology.run_dir(run).join(format!("run{run:04}.root"));
    let monitor_file = topology
        .run_dir(run)
        .join(format!("run{run:04}_monitor.root"));
    assert!(
        wait_for_file(&run_file, Duration::from_secs(60)),
        "{} が 60 s 以内に出来なかった(dir={:?})",
        run_file.display(),
        names_in(&topology.run_dir(run))
    );
    assert!(
        wait_for_file(&monitor_file, Duration::from_secs(15)),
        "{} が出来なかった(dir={:?})",
        monitor_file.display(),
        names_in(&topology.run_dir(run))
    );
    let elapsed = started_at.elapsed();

    // --- E2E-E(§12-9 / R10): 最終 EOS → run{N}_monitor.root 完成が ≤ 10 s ---
    //
    // 発注書どおり「E2E-C の run クローズで」測る(022 は root-sink 単体で実測済み。
    // ここが全通し構成での正式値)。mtime を使うのは 022 と同じ手続き。
    let r10 = std::fs::metadata(&monitor_file)
        .expect("stat monitor.root")
        .modified()
        .expect("mtime")
        .duration_since(eos_wall)
        .unwrap_or(Duration::ZERO);
    assert!(
        r10 <= Duration::from_secs(10),
        "R10 違反: 最終 EOS から run{run:04}_monitor.root 完成まで {r10:?}"
    );
    eprintln!(
        "E2E-E(§12-9 R10): 最終 EOS → monitor.root = {:.3} s",
        r10.as_secs_f64()
    );

    // --- Stop の前に全コンポーネントのカウンタを採る ---
    let component_metrics: Vec<(String, Value)> = topology
        .components
        .iter()
        .map(|c| {
            let endpoint = c.command_endpoint.clone();
            let response = get_status_blocking(&endpoint);
            (c.name.to_string(), response.metrics.unwrap_or(Value::Null))
        })
        .collect();

    // --- run 停止 ---
    let (status, stop_body) = http_async(
        "POST",
        topology.rest,
        "/api/run/stop",
        Some(json!({"token": token}).to_string()),
    )
    .await;
    assert_eq!(status, 200, "run/stop failed: {stop_body}");
    assert_eq!(stop_body["ok"], json!(true), "stop: {stop_body}");
    assert_eq!(stop_body["reason"], json!("normal"), "stop: {stop_body}");
    assert_eq!(
        stop_body["forced_eos"],
        json!(false),
        "自然 EOS で閉じたのに強制 EOS が立っている: {stop_body}"
    );

    // --- WS の観測を確定させる ---
    tokio::time::sleep(Duration::from_millis(500)).await;
    let seen = probe.snapshot();

    // §10.3 meta(接続時 + run 変化時)
    let metas = seen.json("meta");
    assert!(!metas.is_empty(), "meta が 1 通も来ていない");
    let meta = metas.last().unwrap();
    assert_eq!(meta["nBuckets"], 512, "meta: {meta}");
    assert_eq!(
        meta["planes"]["U"], geometry.max_strip[0],
        "meta の U 面幅が実ジオメトリと違う: {meta}"
    );
    assert_eq!(meta["planes"]["V"], geometry.max_strip[1], "meta: {meta}");
    assert_eq!(meta["planes"]["W"], geometry.max_strip[2], "meta: {meta}");
    assert_eq!(meta["detector"], "mini_eTPC_p3", "meta: {meta}");
    assert_eq!(meta["cobos"][0], 0, "meta: {meta}");
    assert_eq!(
        meta["run"], run,
        "run 変化で meta が配り直されていない: {meta}"
    );

    // §10.3 status(1 Hz)
    let statuses = seen.json("status");
    assert!(!statuses.is_empty(), "status が 1 通も来ていない");
    let last_status = statuses.last().unwrap();
    assert_eq!(last_status["run"], run, "status: {last_status}");
    assert_eq!(
        last_status["events_built"], REAL_GRAW_EVENTS,
        "status: {last_status}"
    );
    assert_eq!(last_status["events_incomplete"], 0, "status: {last_status}");
    assert_eq!(last_status["late_fragments"], 0, "status: {last_status}");
    assert_eq!(last_status["pending_events"], 0, "status: {last_status}");
    assert_eq!(
        last_status["frames_per_cobo"]["0"], REAL_GRAW_EVENTS,
        "status: {last_status}"
    );
    assert_eq!(last_status["clients"], 1, "status: {last_status}");
    assert_eq!(
        last_status["monitorGaps"], 0,
        "モニタ PUB の取りこぼし: {last_status}"
    );

    // §10.3 run(遷移時)
    let runs = seen.json("run");
    assert!(!runs.is_empty(), "run 遷移メッセージが来ていない");
    assert!(
        runs.iter()
            .any(|r| r["state"] == "running" && r["run"] == run),
        "running への遷移が無い: {runs:?}"
    );
    assert!(
        runs.iter().any(|r| r["state"] == "idle" && r["run"] == run),
        "idle への遷移(EOS で run クローズ)が無い: {runs:?}"
    );

    // §10.2 0x02 Uvw — ヘッダ 13 B + 面の寸法
    let uvw = seen.first(0x02);
    assert_eq!(&uvw[0..2], b"TP", "magic");
    assert_eq!(uvw[3], 2, "version");
    assert_eq!(u32_at(uvw, 5), run, "0x02 の runNumber");
    let plane = uvw[13] as usize;
    assert!(plane < 3, "plane は 0..2");
    assert_eq!(
        u16_at(uvw, 14),
        geometry.max_strip[plane],
        "0x02 nStrips が実ジオメトリと違う"
    );
    assert_eq!(u16_at(uvw, 16), 512, "0x02 nBuckets");
    assert_eq!(
        uvw.len(),
        18 + geometry.max_strip[plane] as usize * 512 * 2,
        "0x02 の本体長"
    );

    // §10.2 0x03 Waveforms(subscribe ON のときだけ来る)
    let wave = seen.first(0x03);
    assert_eq!(u32_at(wave, 5), run, "0x03 の runNumber");
    assert_eq!(wave[13], 0, "cobo");
    assert_eq!(wave[14], 0, "asad");
    assert_eq!(wave[15], 4, "nAget");
    assert_eq!(wave[16], 68, "nCh(FPN 込み)");
    assert_eq!(u16_at(wave, 17), 512, "nBuckets");
    assert_eq!(wave.len(), 19 + 4 * 68 * 512 * 2, "0x03 の本体長");

    // §10.2 0x11 Histo2d / 0x10 Histo1d
    let h2 = seen.first(0x11);
    let id2 = u16_at(h2, 13);
    assert!((1..=3).contains(&id2), "2D の id は 1..3(SPEC §5.2)");
    assert_eq!(
        u16_at(h2, 15),
        geometry.max_strip[(id2 - 1) as usize],
        "0x11 nx が面の幅と違う"
    );
    assert_eq!(u16_at(h2, 17), 512, "0x11 ny");
    assert_eq!(f32_at(h2, 19), 1.0, "xmin = 1");
    assert_eq!(f32_at(h2, 23), (u16_at(h2, 15) + 1) as f32, "xmax = nx + 1");
    assert_eq!(f32_at(h2, 27), 0.0, "ymin");
    assert_eq!(f32_at(h2, 31), 512.0, "ymax");
    assert_eq!(
        h2.len(),
        35 + u16_at(h2, 15) as usize * 512 * 4,
        "0x11 本体長"
    );

    let h1 = seen.first(0x10);
    let id1 = u16_at(h1, 13);
    assert!((4..=9).contains(&id1), "1D の id は 4..9(SPEC §5.2)");
    assert_eq!(u32_at(h1, 15), 512, "0x10 nbins");
    assert_eq!(f32_at(h1, 19), 0.0, "xmin");
    assert_eq!(
        f32_at(h1, 23),
        4096.0,
        "xmax は 4096 固定(オートレンジ禁止)"
    );
    assert_eq!(seen.bad_header, 0, "13 B ヘッダが壊れた WS フレーム");

    eprintln!(
        "E2E-C WS: meta={} status={} run={} 0x02={} 0x03={} 0x10={} 0x11={}",
        metas.len(),
        statuses.len(),
        runs.len(),
        seen.count_of(0x02),
        seen.count_of(0x03),
        seen.count_of(0x10),
        seen.count_of(0x11),
    );
    probe.stop().await;

    // --- ログブック(SPEC §9.2)---
    let lines = read_logbook(&topology.logbook_path());
    let seqs: Vec<u64> = lines.iter().map(|l| l["seq"].as_u64().unwrap()).collect();
    assert!(
        seqs.windows(2).all(|w| w[1] > w[0]),
        "logbook の seq が単調増加でない: {seqs:?}"
    );
    let starts = records_of_type(&lines, "run_start");
    assert_eq!(starts.len(), 1, "run_start は 1 本: {lines:?}");
    let start = starts[0];
    assert_eq!(start["run"], run);
    assert_eq!(start["config_id"], "p3-e2e");
    assert_eq!(start["operator"], OPERATOR);
    assert_eq!(start["comment"], "p3 e2e-c");
    assert_eq!(
        start["geometry"]["path"],
        env.geometry.to_string_lossy().as_ref()
    );
    assert_eq!(
        start["geometry"]["sha256"].as_str().map(str::len),
        Some(64),
        "geometry.sha256 が 64 桁でない: {start}"
    );
    assert_eq!(start["expected_fragments"], json!([[0, 0]]), "{start}");

    let stops = records_of_type(&lines, "run_stop");
    assert_eq!(stops.len(), 1, "run_stop は 1 本");
    let stop = stops[0];
    assert_eq!(stop["run"], run);
    assert_eq!(stop["ok"], json!(true), "{stop}");
    assert_eq!(stop["reason"], json!("normal"), "{stop}");
    assert_eq!(
        stop["counters"]["frames"]["0"],
        REAL_GRAW_EVENTS + 1,
        "受信フレーム = 108 データ + ctrl 1: {stop}"
    );
    assert_eq!(stop["counters"]["overflow_frames"], json!(0), "{stop}");
    assert_eq!(stop["counters"]["malformed"], json!(0), "{stop}");

    // run_start は run_stop より前(順序)
    let start_seq = start["seq"].as_u64().unwrap();
    let stop_seq = stop["seq"].as_u64().unwrap();
    assert!(start_seq < stop_seq, "run_start が run_stop より後にある");

    // audit: acquire → run/start → run/stop がこの順で成功記録されている
    let audits = records_of_type(&lines, "audit");
    let actions: Vec<&str> = audits
        .iter()
        .map(|a| a["action"].as_str().unwrap_or("?"))
        .collect();
    assert_eq!(
        actions,
        vec!["control/acquire", "run/start", "run/stop"],
        "audit の並び"
    );
    assert!(
        audits.iter().all(|a| a["ok"] == json!(true)),
        "失敗した audit がある: {audits:?}"
    );

    // --- 生 graw のバイト一致(SPEC §12-2 の再現)---
    let files = stop["files"].as_array().expect("run_stop.files");
    assert_eq!(files.len(), 2, "AsAd 1 本 + ctrl 1 本: {files:?}");
    let source = std::fs::read(&env.graw).expect("read the real graw");
    assert_eq!(source.len() as u64, REAL_GRAW_BYTES, "入力バイト数オラクル");
    let (expected_asad, expected_ctrl, asad_frames, ctrl_frames) = split_by_asad(&source);
    assert_eq!(asad_frames, 108, "AsAd フレーム数オラクル");
    assert_eq!(ctrl_frames, 1, "ctrl フレーム数オラクル");
    let mut asad_seen = false;
    let mut ctrl_seen = false;
    for file in files {
        let path = PathBuf::from(file["path"].as_str().expect("path"));
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        assert_eq!(
            bytes.len() as u64,
            file["bytes"].as_u64().unwrap(),
            "logbook の bytes が実ファイルと違う: {file}"
        );
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if name.starts_with("CoBo0_AsAd0_") {
            assert!(bytes == expected_asad, "AsAd 出力のバイト内容が不一致");
            asad_seen = true;
        } else {
            assert!(
                name.starts_with("CoBo0_"),
                "実機 DataRouter 命名でない: {name}"
            );
            assert!(bytes == expected_ctrl, "ctrl 出力のバイト内容が不一致");
            ctrl_seen = true;
        }
    }
    assert!(
        asad_seen && ctrl_seen,
        "AsAd / ctrl の両方が要る: {files:?}"
    );

    // --- 全ロスレスカウンタ 0(Stop 前に採ったコンポーネント実測)---
    for (name, metrics) in &component_metrics {
        match name.as_str() {
            "receiver0" => {
                assert_eq!(metrics["frames"], REAL_GRAW_EVENTS + 1, "{name}: {metrics}");
                assert_eq!(metrics["overflow_frames"], 0, "{name}: {metrics}");
                assert_eq!(metrics["messages_abandoned"], 0, "{name}: {metrics}");
                assert_eq!(metrics["framer_resets"], 0, "{name}: {metrics}");
            }
            "decoder" => {
                assert_eq!(
                    metrics["frames_in"],
                    REAL_GRAW_EVENTS + 1,
                    "{name}: {metrics}"
                );
                assert_eq!(
                    metrics["fragments_out"], REAL_GRAW_EVENTS,
                    "{name}: {metrics}"
                );
                assert_eq!(metrics["items_out"], REAL_GRAW_ITEMS, "{name}: {metrics}");
                assert_eq!(metrics["seq_gaps"], 0, "{name}: {metrics}");
                assert_eq!(metrics["malformed"], 0, "{name}: {metrics}");
                // 実 2025 run の先頭にある frameType 7 制御フレーム ×1(SPEC §12-1 の
                // オラクル。`tests/decoder_pipeline_real_graw.rs` と同じ値)。
                assert_eq!(metrics["unsupported"], 1, "{name}: {metrics}");
                assert_eq!(metrics["batches_abandoned"], 0, "{name}: {metrics}");
                assert_eq!(metrics["eos_abandoned"], 0, "{name}: {metrics}");
                assert_eq!(metrics["eos_in"], 1, "{name}: {metrics}");
            }
            "graw-writer" => {
                assert_eq!(metrics["files_open"], 0, "{name}: {metrics}");
                assert_eq!(metrics["files_closed"], 2, "{name}: {metrics}");
            }
            other => panic!("知らないコンポーネント {other}"),
        }
    }

    // --- root-sink のカウンタ(全通しの保存系オラクル)---
    let scratch = topology.scratch.clone();
    let counts = topology.shutdown().await;
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
        "E2E-C: リプレイ + 保存完了 {:.3} s / {counts}",
        elapsed.as_secs_f64()
    );
    cleanup(&scratch);
}

// =====================================================================
// E2E-G — kill -9 統合(SPEC §12-11)
// =====================================================================

/// コンポーネントへ 1 コマンド送る(テストが「オペレータの手番」を代行するとき用)。
fn command_blocking(endpoint: &str, command: &Command) -> CommandResponse {
    let context = zmq::Context::new();
    tpcdaq::command::request(&context, endpoint, command, Duration::from_secs(5))
        .unwrap_or_else(|e| panic!("{command} {endpoint}: {e}"))
}

/// controller が突然死した後の**オペレータ復旧**。走行中のコンポーネントを上流から
/// `Stop` → `Reset` で Idle へ戻す(controller には自動復旧が無い —— SPEC §1.3 の
/// `Reset` は「run クローズ後の Error 復旧専用」で、その手番はオペレータにある)。
fn recover_components(topology: &Topology) {
    for component in topology.components.iter().rev() {
        for command in [Command::Stop, Command::Reset] {
            let response = command_blocking(&component.command_endpoint, &command);
            eprintln!(
                "recover: {} {command} -> success={} state={}",
                component.name, response.success, response.state
            );
        }
        // receiver の Stop で出た EOS が下流へ流れ切るまでの猶予(次を畳む前に)。
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// SPEC §12-11: run 中に controller を `kill -9` しても
/// ①JSONL は**最終行以外の全行が parse 可能** ②再起動後 `next_run` が重複しない。
///
/// 015 / 025 は同じ不変条件を単体(同一プロセス内の関数呼び出し)で固めた。ここはそれを
/// **実プロセスの突然死**でやる —— controller は `--config` で別プロセス起動し、
/// SIGKILL(Rust バイナリの graceful stop は SIGINT だけなので、これは本当の突然死)。
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn e2e_g_a_kill_9_leaves_a_readable_logbook_and_never_reuses_a_run_number() {
    let Some(env) = e2e_env("E2E-G") else {
        return;
    };
    let mut topology = Topology::start(
        &env,
        "g",
        TopologyOptions {
            spawned_controller: true,
            ..TopologyOptions::default()
        },
    )
    .await;

    // --- 1 本目の run を開始(実 ecc-bridge + fake-ECC 越し)---
    let token = topology.acquire().await;
    let (status, body) = http_async(
        "POST",
        topology.rest,
        "/api/run/start",
        Some(json!({"token": token, "comment": "p3 e2e-g run 1"}).to_string()),
    )
    .await;
    assert_eq!(status, 200, "run/start failed: {body}");
    let first_run = body["run"].as_u64().expect("run number") as u32;
    assert_eq!(first_run, 1, "state ファイル不在なので run は 1 から");
    let status_body = topology.status().await;
    assert_eq!(status_body["phase"], json!("Running"), "{status_body}");

    // --- run 中に SIGKILL ---
    topology.kill_controller();

    // --- ①JSONL 耐性(SPEC §9.1 / §12-11)---
    let logbook_path = topology.logbook_path();
    let raw = std::fs::read_to_string(&logbook_path).expect("read logbook");
    let lines: Vec<&str> = raw.lines().collect();
    assert!(lines.len() >= 2, "run_start + audit が最低 2 行: {lines:?}");
    for (index, line) in lines.iter().enumerate().take(lines.len() - 1) {
        serde_json::from_str::<Value>(line).unwrap_or_else(|e| {
            panic!(
                "最終行以外が parse できない(行 {}): {line:?} ({e})",
                index + 1
            )
        });
    }
    // production のリーダ(SPEC §9.1「最終行のみ parse 失敗を許容」)でも読めること。
    let result = tpcdaq::logbook::read_since(&logbook_path, 0)
        .expect("logbook reader: 最終行以外に壊れた行がある");
    eprintln!(
        "E2E-G: kill -9 後の logbook = {} 行 / 読めたレコード {} 件 / tail_corrupt={}",
        lines.len(),
        result.records.len(),
        result.tail_corrupt
    );
    let started: Vec<u32> = result
        .records
        .iter()
        .filter_map(|line| match &line.record {
            tpcdaq::logbook::LogbookRecord::RunStart { run, .. } => Some(*run),
            _ => None,
        })
        .collect();
    assert_eq!(
        started,
        vec![first_run],
        "run_start は 1 本(run {first_run})"
    );

    // --- オペレータ復旧 → controller 再起動 ---
    recover_components(&topology);
    topology.restart_controller();
    let status_body = topology.status().await;
    assert_eq!(
        status_body["phase"],
        json!("Idle"),
        "再起動直後は Idle: {status_body}"
    );

    // ECC は死んだ controller が Running のまま残している(実 ECC と同じ現れ方)。
    //
    // **036**: ここでオペレータが `POST /api/ecc/reset` を撃っても **何も起こらない** ——
    // 実 ECC の `reset` は `EV_UNDO` で、`Active`(Running)からは遷移が定義されておらず
    // **無音で無視**される(`GetBench/src/get/rc/BackEnd.cpp:924` / `:250-270`、
    // `StateMachine/src/dhsm/Engine.cpp:334-352`)。`ok:true` が返るのに状態が `Running` の
    // まま = 実機で一番騙されやすい形なので、**その形をここで固定する**。
    // (034 まではフェイクが「どこからでも Idle」だったのでこの罠が見えなかった。)
    let token = topology.acquire().await;
    let (status, body) = http_async(
        "POST",
        topology.rest,
        "/api/ecc/reset",
        Some(json!({"token": token}).to_string()),
    )
    .await;
    assert_eq!(status, 200, "ecc/reset failed: {body}");
    assert_eq!(body["ok"], json!(true), "ecc/reset: {body}");
    assert_eq!(
        body["state"],
        json!("Running"),
        "実 ECC の reset は Active から無音で無視される(036): {body}"
    );

    // 復旧はオペレータの手作業ではなく **run/start の歩き戻し**が引き受ける ——
    // 下の 2 本目の `run/start` が `Running` から `stop → breakup → reset → reset` で
    // ECC を畳んでから走り出す。それが 1 発で 200 を返すことがこの節の出口である。

    // --- ②next_run が重複しない ---
    let (status, body) = http_async(
        "POST",
        topology.rest,
        "/api/run/start",
        Some(json!({"token": token, "comment": "p3 e2e-g run 2"}).to_string()),
    )
    .await;
    assert_eq!(status, 200, "2 本目の run/start failed: {body}");
    let second_run = body["run"].as_u64().expect("run number") as u32;
    assert_eq!(
        second_run,
        first_run + 1,
        "kill -9 を跨いで run 番号が重複 or 巻き戻っている"
    );

    let (status, body) = http_async(
        "POST",
        topology.rest,
        "/api/run/stop",
        Some(json!({"token": token}).to_string()),
    )
    .await;
    assert_eq!(status, 200, "run/stop failed: {body}");

    // --- logbook 全体: run_start は 1 と 2、重複なし ---
    let lines = read_logbook(&topology.logbook_path());
    let runs: Vec<u64> = records_of_type(&lines, "run_start")
        .iter()
        .map(|r| r["run"].as_u64().unwrap())
        .collect();
    assert_eq!(runs, vec![1, 2], "run_start の run 番号列");
    let seqs: Vec<u64> = lines.iter().map(|l| l["seq"].as_u64().unwrap()).collect();
    assert!(
        seqs.windows(2).all(|w| w[1] > w[0]),
        "再起動を跨いで logbook の seq が単調増加でない: {seqs:?}"
    );

    // 状態ファイルの実測値(次に出るのは 3)。
    let state_path = topology.data_root.join("tpcdaq_state.json");
    let state: Value =
        serde_json::from_str(&std::fs::read_to_string(&state_path).expect("read state"))
            .expect("state json");
    assert_eq!(state["next_run"], json!(3), "state={state}");

    let scratch = topology.scratch.clone();
    let counts = topology.shutdown().await;
    assert_eq!(counts["fatal"], "", "root_sink: {counts}");
    cleanup(&scratch);
}

// =====================================================================
// E2E-D — モニタ非干渉の正式計測(SPEC §12-8、release 限定)
// =====================================================================

/// 計測系は release 限定(dev は最適化なしで保存経路が入力に律速されず、±2% の判定が
/// 意味を持たない —— 012 の教訓)。
fn release_only(test: &str) -> bool {
    if cfg!(debug_assertions) {
        eprintln!(
            "SKIP {test}: 計測系は release のみ(dev ビルドでは保存経路が最適化に支配され \
             ±2% 判定が成立しない)。`cargo test --release --test p3_e2e` で実行すること"
        );
        return true;
    }
    false
}

/// `uptime(1)` の load average(計測環境の記録 —— 他プロセスが動いていない確認)。
fn load_average() -> String {
    OsCommand::new("uptime")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "<uptime unavailable>".to_string())
}

/// 表示用に ms 精度へ丸める(生値の羅列を読めるようにするだけ)。
fn round3(values: &[f64]) -> Vec<f64> {
    values
        .iter()
        .map(|v| (v * 1000.0).round() / 1000.0)
        .collect()
}

fn median(samples: &[f64]) -> f64 {
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
    sorted[sorted.len() / 2]
}

/// 1 run 分の「フルスピード投入 → `run{N}.root` finalize」計測結果。
struct SaveSample {
    /// graw_replay 起動 → `run{N}.root` 出現までの wall time(= 保存完了時間)。
    wall: f64,
    /// root_sink の終了カウンタ JSON。
    counts: Value,
    /// 観測用 WS クライアントが見たもの。
    ws: WsSeen,
}

/// 固定入力(mini 実 graw)を**ペーシングなし全速**で 1 run 流し、保存完了時間を測る。
///
/// モニタ経路は**完全に生きたまま**(実ジオメトリ + root-sink のヒスト集計 + PUB +
/// monitor + WS クライアント 1 本)。`snapshot_hz` だけを 3 点で振る。
/// `frozen` を立てると **binary を 1 通も読まない WS クライアント**を 1 本足す(§5.4 の
/// freeze 相当。読まないだけで購読は張ったまま = drop-oldest が効く状況)。
async fn measure_save(env: &E2eEnv, tag: &str, snapshot_hz: u32, frozen: bool) -> SaveSample {
    let topology = Topology::start(
        env,
        tag,
        TopologyOptions {
            snapshot_hz: Some(snapshot_hz),
            no_data_link: true,
            ..TopologyOptions::default()
        },
    )
    .await;

    // 観測用(= 通常のオペレータ画面: waveforms 以外 ON = SPEC §10.3 の既定購読)。
    let observer = WsProbe::connect(topology.monitor_ws, None).await;

    // freeze 相当: 購読は張るが**読まない**クライアント。波形まで購読して帯域を上げる。
    let mut frozen_client = if frozen {
        let url = format!("ws://{}/ws", topology.monitor_ws);
        let (mut socket, _) = tokio_tungstenite::connect_async(&url)
            .await
            .expect("frozen WS connect");
        socket
            .send(WsMessage::Text(
                r#"{"streams":["uvw","waveforms","histos","status"]}"#
                    .to_string()
                    .into(),
            ))
            .await
            .expect("frozen subscribe");
        // 以降 `next()` を一切呼ばない = 受信を止めたクライアント。
        Some(socket)
    } else {
        None
    };
    tokio::time::sleep(Duration::from_millis(300)).await;

    let token = topology.acquire().await;
    let (status, body) = http_async(
        "POST",
        topology.rest,
        "/api/run/start",
        Some(json!({"token": token, "comment": tag}).to_string()),
    )
    .await;
    assert_eq!(status, 200, "run/start failed: {body}");
    let run = body["run"].as_u64().expect("run number") as u32;
    let data_address = topology.receiver_data_address().await;

    // --- 計測本体: 全速投入 → run{N}.root finalize ---
    let started = Instant::now();
    let replay = OsCommand::new(env!("CARGO_BIN_EXE_graw_replay"))
        .arg(&data_address)
        .arg(&env.graw)
        .stdout(Stdio::null())
        .status()
        .expect("run graw_replay");
    assert!(replay.success(), "graw_replay failed: {replay:?}");
    let run_file = topology.run_dir(run).join(format!("run{run:04}.root"));
    assert!(
        wait_for_file(&run_file, Duration::from_secs(120)),
        "{} が出来なかった",
        run_file.display()
    );
    let wall = started.elapsed().as_secs_f64();

    // **freeze 解除**(= UI が表示を再開した)。tokio の broadcast は「受け側が次に recv した
    // ときに `Lagged(n)` で落とした数を知る」意味論なので、読み始めて初めて `wsDropped` に
    // 積まれる。計測(`wall`)は既に確定しているのでスループットには影響しない。
    if let Some(mut socket) = frozen_client.take() {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            let _ = tokio::time::timeout(Duration::from_millis(100), socket.next()).await;
        }
    }
    // `wsDropped` を載せた status(1 Hz)が観測クライアントへ届くまで待つ。
    tokio::time::sleep(Duration::from_millis(1600)).await;

    let (status, body) = http_async(
        "POST",
        topology.rest,
        "/api/run/stop",
        Some(json!({"token": token}).to_string()),
    )
    .await;
    assert_eq!(status, 200, "run/stop failed: {body}");
    assert_eq!(body["ok"], json!(true), "stop: {body}");

    tokio::time::sleep(Duration::from_millis(400)).await;
    let ws = observer.snapshot();
    observer.stop().await;

    let scratch = topology.scratch.clone();
    let counts = topology.shutdown().await;
    cleanup(&scratch);

    // どの計測点でも**保存系は完全**でなければ、速さの比較に意味が無い。
    assert_eq!(count(&counts, "fragments"), REAL_GRAW_EVENTS, "{counts}");
    assert_eq!(count(&counts, "items"), REAL_GRAW_ITEMS, "{counts}");
    assert_eq!(
        count(&counts, "events_complete"),
        REAL_GRAW_EVENTS,
        "{counts}"
    );
    assert_eq!(count(&counts, "events_incomplete"), 0, "{counts}");
    assert_eq!(count(&counts, "late_fragments"), 0, "{counts}");
    assert_eq!(
        count(&counts, "entries_written"),
        REAL_GRAW_EVENTS,
        "{counts}"
    );
    assert_eq!(counts["fatal"], "", "{counts}");

    SaveSample { wall, counts, ws }
}

/// SPEC §12-8: スナップショット配信を 0 Hz / 既定(1 Hz)/ 2 倍(2 Hz)にしても
/// **保存スループットは測定誤差内(±2%)**。あわせて freeze 相当のクライアントが
/// 保存にもカウンタにも影響しないこと。
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn e2e_d_the_snapshot_rate_and_a_frozen_client_do_not_change_the_storage_throughput() {
    let Some(env) = e2e_env("E2E-D") else {
        return;
    };
    if release_only("E2E-D") {
        return;
    }
    /// 発注書「各点 5 回以上、中央値で ±2% 判定」。
    const REPS: usize = 7;

    eprintln!("E2E-D 計測開始: {}", load_average());

    let mut walls: std::collections::BTreeMap<u32, Vec<f64>> = std::collections::BTreeMap::new();
    let mut published: std::collections::BTreeMap<u32, Vec<u64>> =
        std::collections::BTreeMap::new();
    for hz in [0u32, 1, 2] {
        for rep in 0..REPS {
            let sample = measure_save(&env, &format!("d{hz}hz_{rep}"), hz, false).await;
            walls.entry(hz).or_default().push(sample.wall);
            published
                .entry(hz)
                .or_default()
                .push(count(&sample.counts, "snapshots_published"));
        }
    }
    eprintln!("E2E-D 3 点計測おわり: {}", load_average());

    // --- snapshot_hz の設定が実際に効いていること(効いていなければ比較に意味が無い)---
    let zero = published[&0].iter().sum::<u64>();
    assert_eq!(
        zero, 0,
        "--snapshot-hz 0 なのに snapshot が出ている: {published:?}"
    );
    assert!(
        published[&2].iter().sum::<u64>() > published[&1].iter().sum::<u64>(),
        "2 Hz が 1 Hz より多く publish していない: {published:?}"
    );

    let baseline = median(&walls[&1]);
    for hz in [0u32, 2] {
        let m = median(&walls[&hz]);
        let delta = (m - baseline) / baseline * 100.0;
        eprintln!(
            "E2E-D snapshot_hz={hz}: 中央値 {m:.3} s(既定 1 Hz {baseline:.3} s 比 {delta:+.2}%)\n  \
             生値 = {:?}",
            walls[&hz]
        );
        assert!(
            delta.abs() <= 2.0,
            "§12-8 違反: snapshot_hz={hz} の保存完了時間が既定比 {delta:+.2}%(±2% 超)\n\
             0Hz={:?}\n1Hz={:?}\n2Hz={:?}",
            walls[&0],
            walls[&1],
            walls[&2]
        );
    }
    eprintln!(
        "E2E-D 既定 1 Hz: 中央値 {baseline:.3} s / 生値 = {:?}",
        walls[&1]
    );

    // --- freeze 相当(§5.4): 読まないクライアントが居ても保存もカウンタも動く ---
    let mut frozen_walls = Vec::new();
    let mut frozen_ws = Vec::new();
    for rep in 0..REPS {
        let sample = measure_save(&env, &format!("dfrozen_{rep}"), 1, true).await;
        frozen_walls.push(sample.wall);
        frozen_ws.push(sample.ws);
    }
    let frozen_median = median(&frozen_walls);
    let frozen_delta = (frozen_median - baseline) / baseline * 100.0;
    eprintln!(
        "E2E-D freeze 相当: 中央値 {frozen_median:.3} s(既定比 {frozen_delta:+.2}%)/ 生値 = {frozen_walls:?}"
    );
    assert!(
        frozen_delta.abs() <= 2.0,
        "読まない WS クライアントが保存スループットを {frozen_delta:+.2}% 動かした"
    );

    // 観測クライアント側の status で: clients=2 / wsDropped が立つ / events は進み続ける
    let mut saw_two_clients = false;
    let mut saw_drops = false;
    let mut saw_progress = false;
    for ws in &frozen_ws {
        let statuses = ws.json("status");
        assert!(
            !statuses.is_empty(),
            "観測クライアントに status が来ていない"
        );
        let clients: Vec<u64> = statuses
            .iter()
            .filter_map(|s| s["clients"].as_u64())
            .collect();
        let dropped: Vec<u64> = statuses
            .iter()
            .filter_map(|s| s["wsDropped"].as_u64())
            .collect();
        let built: Vec<u64> = statuses
            .iter()
            .filter_map(|s| s["events_built"].as_u64())
            .collect();
        saw_two_clients |= clients.contains(&2);
        saw_drops |= dropped.iter().any(|d| *d > 0);
        // **freeze 中も events カウンタは進み続ける**(SPEC §5.4)。
        saw_progress |= built.windows(2).any(|w| w[1] > w[0]);
        assert_eq!(
            built.last().copied(),
            Some(REAL_GRAW_EVENTS),
            "freeze 中でも最終的に 108 まで進むこと: {built:?}"
        );
    }
    assert!(
        saw_two_clients,
        "読まないクライアントが clients に数えられていない"
    );
    assert!(
        saw_drops,
        "live を落としたはずなのに wsDropped が 1 度も立たない(silent drop になっている)"
    );
    assert!(saw_progress, "status の events_built が 1 度も進んでいない");

    eprintln!("E2E-D 計測終了: {}", load_average());
}

/// SPEC §12-8 v1.10(R-P2-14 = §6.2-8 の定量化): **連続 2 run** で run 境界の
/// rollover(TTree finalize + monitor.root 書き出し)が intake をブロックしないこと。
///
/// # 採った計測方法(実装裁量 —— 結果節に残すこと)
///
/// C++ 側に計測用の細工を入れられない(production 変更禁止)ので、**root_sink 自身が
/// stderr に出す run ライフサイクル行にテスト側でタイムスタンプを打つ**:
///
/// * `run N opened` … その run の最初の Data を intake が受理した瞬間
/// * `run N closed` / `finalized …` / `wrote …_monitor.root` … 保存側の後始末
///
/// **停滞値 = t(run N opened) − t(その run の graw_replay 開始)**。run 1 は「空の
/// root_sink に入る」ので下限、run 2 は「run 1 の finalize と重なる」ので、境界が
/// intake をブロックするならここが跳ねる。run 1 の finalize より **run 2 の open が
/// 先行する**ことも直接見る(= 保存の後始末が取り込みを待たせていない)。
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn e2e_d_a_run_boundary_does_not_stall_the_intake() {
    let Some(env) = e2e_env("E2E-D 跨 run") else {
        return;
    };
    if release_only("E2E-D 跨 run") {
        return;
    }
    eprintln!("E2E-D 跨 run: {}", load_average());

    let topology = Topology::start(
        &env,
        "dcross",
        TopologyOptions {
            no_data_link: true,
            ..TopologyOptions::default()
        },
    )
    .await;
    let observer = WsProbe::connect(topology.monitor_ws, None).await;
    let token = topology.acquire().await;

    let mut opened_lag = Vec::new();
    let mut run_wall = Vec::new();
    let mut replay_started = Vec::new();
    let mut runs = Vec::new();
    let mut restart_cost = Vec::new();
    let mut first_out: Vec<f64> = Vec::new();
    for pass in 0..2u32 {
        // **034 以降、2 本目も `run/start` 1 発で通る**(手動の `POST /api/ecc/reset` も
        // テスト側のリトライループも要らない)。030 の初版では:
        //   ① `ecc stop` 後の ECC は `Ready` で `describe` が順序違反 → オペレータが
        //      `ecc/reset` を手で挟んでいた
        //   ② `Arm` が `Address already in use`(libzmq の close が非同期)で落ちるので
        //      テスト側で run/start を撃ち直していた(失敗のたびに run 番号を空費)
        // どちらも controller 側の処置(先頭 reset / Arm の指数バックオフ / 採番の巻き戻し)に
        // 移った。**ここでは 1 発で通ることを要求する**(通らなければ 034 の退行)。
        let began = Instant::now();
        let (status, body) = http_async(
            "POST",
            topology.rest,
            "/api/run/start",
            Some(json!({"token": token, "comment": format!("p3 e2e-d cross {pass}")}).to_string()),
        )
        .await;
        assert_eq!(
            status, 200,
            "{pass} 本目の run/start が 1 発で通らない(034 の退行): {body}"
        );
        let run = body["run"].as_u64().expect("run number") as u32;
        restart_cost.push((1u32, began.elapsed().as_secs_f64()));
        runs.push(run);
        let data_address = topology.receiver_data_address().await;

        // decoder が **この run の最初のバッチを送り出した瞬間**を外から測る。
        // root_sink の "run N opened"(= intake がそれを受理した瞬間)との差が、
        // 「データは在るのに intake が動かなかった時間」= §6.2-8 の停滞値になる。
        let decoder_endpoint = topology.components[1].command_endpoint.clone();
        let baseline = {
            let endpoint = decoder_endpoint.clone();
            tokio::task::spawn_blocking(move || {
                get_status_blocking(&endpoint)
                    .metrics
                    .and_then(|m| m["batches_out"].as_u64())
                    .unwrap_or(0)
            })
            .await
            .expect("decoder GetStatus")
        };
        let first_batch_out: Arc<Mutex<Option<f64>>> = Arc::new(Mutex::new(None));
        let poller_stop = Arc::new(AtomicBool::new(false));
        let poller = tokio::spawn({
            let endpoint = decoder_endpoint.clone();
            let seen = Arc::clone(&first_batch_out);
            let stop = Arc::clone(&poller_stop);
            let epoch = topology.sink.epoch;
            async move {
                while !stop.load(Ordering::Relaxed) {
                    let endpoint = endpoint.clone();
                    let out = tokio::task::spawn_blocking(move || {
                        get_status_blocking(&endpoint)
                            .metrics
                            .and_then(|m| m["batches_out"].as_u64())
                            .unwrap_or(0)
                    })
                    .await
                    .unwrap_or(0);
                    if out > baseline {
                        *seen.lock().expect("first_batch_out mutex") =
                            Some(Instant::now().duration_since(epoch).as_secs_f64());
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        });

        // 実機に近いペースで流す(全速だと投入が一瞬で終わり、境界の重なりが作れない)。
        let at = Instant::now();
        replay_started.push(topology.sink.seconds_since_epoch(at));
        let replay = tokio::task::spawn_blocking({
            let graw = env.graw.clone();
            move || {
                OsCommand::new(env!("CARGO_BIN_EXE_graw_replay"))
                    .arg(&data_address)
                    .arg(&graw)
                    .arg("--rate-mbps")
                    .arg(REPLAY_RATE_MBPS.to_string())
                    .stdout(Stdio::null())
                    .status()
                    .expect("run graw_replay")
            }
        })
        .await
        .expect("graw_replay task");
        assert!(replay.success(), "graw_replay failed: {replay:?}");
        poller_stop.store(true, Ordering::Relaxed);
        let _ = tokio::time::timeout(Duration::from_secs(5), poller).await;
        first_out.push(
            (*first_batch_out.lock().expect("first_batch_out mutex"))
                .expect("decoder が 1 バッチも送っていない"),
        );

        // **`run{N}.root` の完成を待たずに** run を閉じ、すぐ次の run を開ける
        // (= run 1 の finalize と run 2 の intake を意図的に重ねる。ここが §6.2-8 の要)。
        let (status, body) = http_async(
            "POST",
            topology.rest,
            "/api/run/stop",
            Some(json!({"token": token}).to_string()),
        )
        .await;
        assert_eq!(status, 200, "run/stop failed: {body}");
        assert_eq!(body["ok"], json!(true), "stop: {body}");
        run_wall.push(at.elapsed().as_secs_f64());
    }

    eprintln!(
        "E2E-D 跨 run: 実 run 番号 = {runs:?} / run/stop → run/start が通るまで \
         (試行回数, 秒) = {restart_cost:?}"
    );
    // 034: 失敗した start が番号を空費しないので、連続 run の番号は連番になる。
    assert_eq!(runs, vec![1, 2], "run 番号が飛んだ: {runs:?}");

    // 2 本とも finalize されるまで待つ。
    for &run in &runs {
        for name in [
            format!("run{run:04}.root"),
            format!("run{run:04}_monitor.root"),
        ] {
            let path = topology.run_dir(run).join(&name);
            assert!(
                wait_for_file(&path, Duration::from_secs(60)),
                "{} が出来なかった(dir={:?})",
                path.display(),
                names_in(&topology.run_dir(run))
            );
        }
    }

    // --- 境界の時刻を root_sink 自身のログから拾う ---
    let mut opened_at = Vec::new();
    let mut closed_at = Vec::new();
    for &run in &runs {
        opened_at.push(
            topology
                .sink
                .first_at(&format!("run {run} opened"))
                .unwrap_or_else(|| panic!("root_sink の \"run {run} opened\" が無い")),
        );
        closed_at.push(
            topology
                .sink
                .first_at(&format!("run {run} closed"))
                .unwrap_or_else(|| panic!("root_sink の \"run {run} closed\" が無い")),
        );
    }
    let finalized_first = topology
        .sink
        .first_at(&format!("run{:04}.root", runs[0]))
        .expect("1 本目の finalize 行が無い");
    let monitor_root_first = topology
        .sink
        .first_at(&format!("run{:04}_monitor.root", runs[0]))
        .expect("1 本目の monitor.root 行が無い");

    // **停滞値 = t(2 本目 open) − t(1 本目 close)**。
    // 「run を閉じる処理(残イベント flush + RunClose マーカー投入 + ヒスト複製)が
    // 次の run の取り込みを待たせた時間」そのもの。
    let boundary_stall = opened_at[1] - closed_at[0];
    // 参考値: decoder が送り出してから intake が受理するまで。これは**背圧による滞留**
    // (ロスレス設計どおり)であって境界の停滞ではない —— 混同しないよう別名で残す。
    for (pass, opened) in opened_at.iter().enumerate() {
        opened_lag.push(opened - first_out[pass]);
    }

    eprintln!(
        "E2E-D 跨 run: **境界の停滞値 = t(2本目 open) - t(1本目 close) = {boundary_stall:.3} s**\n\
         E2E-D 跨 run: 時刻[s] close {:?} / open {:?} / 1本目 finalize {finalized_first:.3} / \
         1本目 monitor.root {monitor_root_first:.3}\n\
         E2E-D 跨 run: 2 本目 open は 1 本目 finalize より {:.3} s {}(ROOT IO は intake の外)\n\
         E2E-D 跨 run: 参考 = decoder 初回送出 -> open の滞留 {:?}(背圧による在庫であって停滞ではない)\n\
         E2E-D 跨 run: 投入 {:?} / decoder 初回送出 {:?} / per-run wall {:?}",
        round3(&closed_at),
        round3(&opened_at),
        (finalized_first - opened_at[1]).abs(),
        if opened_at[1] < finalized_first {
            "先行"
        } else {
            "後行"
        },
        round3(&opened_lag),
        round3(&replay_started),
        round3(&first_out),
        round3(&run_wall),
    );
    for (at, line) in topology.sink.log.lock().expect("SinkLog mutex").iter() {
        eprintln!("E2E-D 跨 run [root_sink t={at:7.3} s] {line}");
    }

    // §6.2-8「run 境界でのブロッキング外部 IO 禁止」の定量判定。
    // しきい値 0.2 s = ログのタイムスタンプ粒度に対して十分大きく、finalize 所要
    // (実測 0.5 s 超)に対して十分小さい値。
    assert!(
        boundary_stall.abs() < 0.2,
        "run 境界で intake が {boundary_stall:.3} s 停滞している(§6.2-8 違反)"
    );
    assert!(
        opened_at[1] < finalized_first,
        "2 本目の intake が 1 本目の finalize 完了を待っている(§6.2-8 = 境界のブロッキング)"
    );

    let ws = observer.snapshot();
    observer.stop().await;
    let run_messages = ws.json("run");
    for run in &runs {
        assert!(
            run_messages.iter().any(|r| r["run"] == *run),
            "WS に run {run} の遷移が出ていない: {run_messages:?}"
        );
    }

    let scratch = topology.scratch.clone();
    let counts = topology.shutdown().await;
    assert_eq!(count(&counts, "runs"), 2, "{counts}");
    assert_eq!(
        count(&counts, "fragments"),
        2 * REAL_GRAW_EVENTS,
        "{counts}"
    );
    assert_eq!(count(&counts, "items"), 2 * REAL_GRAW_ITEMS, "{counts}");
    assert_eq!(
        count(&counts, "events_complete"),
        2 * REAL_GRAW_EVENTS,
        "{counts}"
    );
    assert_eq!(count(&counts, "events_incomplete"), 0, "{counts}");
    assert_eq!(count(&counts, "late_fragments"), 0, "{counts}");
    assert_eq!(
        count(&counts, "entries_written"),
        2 * REAL_GRAW_EVENTS,
        "{counts}"
    );
    assert_eq!(counts["fatal"], "", "{counts}");
    let root_files = counts["root_files"].as_array().expect("root_files");
    assert_eq!(root_files.len(), 2, "run 毎単一 ROOT ×2: {counts}");
    for file in root_files {
        assert_eq!(file["entries"].as_u64(), Some(REAL_GRAW_EVENTS), "{counts}");
    }
    cleanup(&scratch);
}

// =====================================================================
// E2E-H — 連続 run の運用性(TODO/034、SPEC §1.3)
// =====================================================================

/// **034 の出口**: `POST /api/ecc/reset` を**オペレータが手で挟まずに**、
/// `run/stop` → `run/start` を **3 本 back-to-back** で回せること。
///
/// 030 の実測(このテストが塞ぐ穴):
///
/// 1. 2 本目の `run/start` が `ecc describe failed in state Ready` で**必ず**失敗した
///    (`ecc stop` 後の ECC は `Ready`、`describe` は `Off/Idle/Described` からのみ)。
/// 2. `Arm` が `Address already in use` で落ちた(**3 回に 2 回**。libzmq の close が非同期)。
/// 3. 失敗した `run/start` が run 番号を空費した(**run 1 → run 4**)。
///
/// したがって出口は 3 つ: **`run/start` が 1 発で 200 を返す**(テスト側にリトライ機構を
/// 置かない —— 置いたら壊れても気づけない)/ **run 番号が 1,2,3** / 各 run が実データを
/// 最後まで保存する。ECC は実配線(controller → ecc-bridge → Ice → fake-ECC servant)で、
/// **順序違反を本当に断つ `ecc_core.hpp` の遷移表**が相手なので、reset 段が無ければ
/// 2 本目でこのテストは必ず落ちる。
///
/// **036 で遷移表を実 ECC の SM に合わせた**(`reset` = `EV_UNDO` = 1 段戻すだけ、`Active`
/// からは無音で無視、`configure` は `Prepared` からのみ)。034 の「reset 1 発」実装は
/// この表の下で `ecc describe failed in state Ready` として**実測で落ちた** —— そのうえで
/// 歩き戻し(`breakup → reset → reset`)を入れて green にしたのが本テストの現在形である。
///
/// `Arm` のリトライ実績は controller が audit(`params.arm_retries`)に残すので、
/// ここでそれを読んで **実測値として stderr に出す**(034 のリトライ上限の根拠)。
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn e2e_h_consecutive_runs_need_no_manual_ecc_reset() {
    let Some(env) = e2e_env("E2E-H 連続 run") else {
        return;
    };
    let topology = Topology::start(
        &env,
        "h",
        TopologyOptions {
            no_data_link: true,
            ..TopologyOptions::default()
        },
    )
    .await;
    let token = topology.acquire().await;

    const PASSES: u32 = 3;
    let mut runs = Vec::new();
    let mut start_cost = Vec::new();
    for pass in 0..PASSES {
        // **手動の ecc/reset を挟まない。リトライループも置かない。**
        // ここが 1 発で通ることが 034 の出口そのもの。
        let began = Instant::now();
        let (status, body) = http_async(
            "POST",
            topology.rest,
            "/api/run/start",
            Some(json!({"token": token, "comment": format!("p3 e2e-h {pass}")}).to_string()),
        )
        .await;
        assert_eq!(
            status, 200,
            "{pass} 本目の run/start が 1 発で通らなかった(034 の出口): {body}"
        );
        start_cost.push(began.elapsed().as_secs_f64());
        let run = body["run"].as_u64().expect("run number") as u32;
        runs.push(run);

        let data_address = topology.receiver_data_address().await;
        let replay = tokio::task::spawn_blocking({
            let graw = env.graw.clone();
            move || {
                OsCommand::new(env!("CARGO_BIN_EXE_graw_replay"))
                    .arg(&data_address)
                    .arg(&graw)
                    .arg("--rate-mbps")
                    .arg(REPLAY_RATE_MBPS.to_string())
                    .stdout(Stdio::null())
                    .status()
                    .expect("run graw_replay")
            }
        })
        .await
        .expect("graw_replay task");
        assert!(replay.success(), "graw_replay failed: {replay:?}");

        let (status, body) = http_async(
            "POST",
            topology.rest,
            "/api/run/stop",
            Some(json!({"token": token}).to_string()),
        )
        .await;
        assert_eq!(status, 200, "{pass} 本目の run/stop: {body}");
        assert_eq!(body["ok"], json!(true), "{pass} 本目の run/stop: {body}");
        assert_eq!(body["reason"], json!("normal"), "{body}");
    }

    // **run 番号が飛ばない**(030 実測の run 1 → run 4 が起きない)。
    assert_eq!(
        runs,
        (1..=PASSES).collect::<Vec<u32>>(),
        "run 番号が飛んだ: {runs:?}"
    );

    // 2 本とも 3 本目とも finalize されること。
    for &run in &runs {
        for name in [
            format!("run{run:04}.root"),
            format!("run{run:04}_monitor.root"),
        ] {
            let path = topology.run_dir(run).join(&name);
            assert!(
                wait_for_file(&path, Duration::from_secs(60)),
                "{} が出来なかった(dir={:?})",
                path.display(),
                names_in(&topology.run_dir(run))
            );
        }
    }

    // --- ログブック: run_start / run_stop が run 毎に 1 組、audit は全部 ok ---
    let records = read_logbook(&topology.logbook_path());
    let started: Vec<u64> = records_of_type(&records, "run_start")
        .iter()
        .map(|r| r["run"].as_u64().expect("run"))
        .collect();
    let stopped: Vec<u64> = records_of_type(&records, "run_stop")
        .iter()
        .map(|r| r["run"].as_u64().expect("run"))
        .collect();
    assert_eq!(started, vec![1, 2, 3], "{records:?}");
    assert_eq!(stopped, vec![1, 2, 3], "{records:?}");

    let start_audits: Vec<&Value> = records
        .iter()
        .filter(|r| r["type"] == "audit" && r["action"] == "run/start")
        .collect();
    assert_eq!(
        start_audits.len(),
        PASSES as usize,
        "run/start の audit が {} 件でない(失敗が混ざっていない): {start_audits:?}",
        PASSES
    );
    // **036 の出口**: 歩き戻しの各段が audit に残り、**2 本目以降は実際に 3 手歩いている**。
    //
    // `ecc_core.hpp` の遷移表は実 ECC の SM そのもの(`reset` = `EV_UNDO` = 1 段戻すだけ、
    // `Active` からは無音で無視)なので、期待値は状態から一意に決まる:
    //
    //   1 本目 = fake-ECC は起動直後の `Off`(= これ以上戻れない底)→ **1 手も歩かない**
    //   2, 3 本目 = 前の run の `ecc stop` が `Ready` に置いている
    //               → `breakup`(→Prepared)→ `reset`(→Described)→ `reset`(→Idle)
    //
    // ここが `["status->Ready"]` だけになったら歩き戻しが死んでいる(= 034 の事故の再来)。
    for (pass, audit) in start_audits.iter().enumerate() {
        assert_eq!(audit["ok"], json!(true), "{audit}");
        let want_walk: Vec<&str> = if pass == 0 {
            vec!["status->Off"]
        } else {
            vec![
                "status->Ready",
                "breakup->Prepared",
                "reset->Described",
                "reset->Idle",
            ]
        };
        assert_eq!(
            audit["params"]["ecc_walk_back"],
            json!(want_walk),
            "{pass} 本目の歩き戻し: {audit}"
        );
        assert_eq!(
            audit["params"]["ecc_state_after_reset"],
            json!(if pass == 0 { "Off" } else { "Idle" }),
            "{audit}"
        );
    }

    // **034 論点 B の実測値**: Arm を撃ち直した実績は audit に載る(silent 禁止)。
    // 0 件でも「リトライが要らなかった」という実測なので、そのまま出す。
    let retries: Vec<&Value> = start_audits
        .iter()
        .filter_map(|a| a["params"]["arm_retries"].as_array())
        .flatten()
        .collect();
    eprintln!(
        "E2E-H: run 番号 = {runs:?} / run/start の所要[s] = {:?} / Arm リトライ実績 = {retries:?}",
        round3(&start_cost)
    );

    let scratch = topology.scratch.clone();
    let counts = topology.shutdown().await;
    assert_eq!(count(&counts, "runs"), PASSES as u64, "{counts}");
    assert_eq!(
        count(&counts, "events_complete"),
        PASSES as u64 * REAL_GRAW_EVENTS,
        "{counts}"
    );
    assert_eq!(count(&counts, "events_incomplete"), 0, "{counts}");
    assert_eq!(count(&counts, "late_fragments"), 0, "{counts}");
    assert_eq!(
        count(&counts, "entries_written"),
        PASSES as u64 * REAL_GRAW_EVENTS,
        "{counts}"
    );
    assert_eq!(counts["fatal"], "", "{counts}");
    let root_files = counts["root_files"].as_array().expect("root_files");
    assert_eq!(
        root_files.len(),
        PASSES as usize,
        "run 毎単一 ROOT: {counts}"
    );
    for file in root_files {
        assert_eq!(file["entries"].as_u64(), Some(REAL_GRAW_EVENTS), "{counts}");
    }
    cleanup(&scratch);
}
