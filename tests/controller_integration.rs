//! controller の統合テスト(実 ZMQ + 実 REST、すべて動的ポート)。TODO/016 §テスト。
//!
//! フェイクのコンポーネントは既存の [`tpcdaq::command::run_command_task`] をそのまま起動し、
//! **本物の状態機械([`ComponentState::can_transition_to`])で不許可遷移を断る**。
//! したがって「controller が §1.3 の順序どおりに叩いているか」だけでなく
//! 「その順序が状態機械として合法か」も同時に機械照合される。
//!
//! HTTP クライアントは std だけの手書き(`Connection: close` で 1 往復 = 応答本体は EOF まで)。
//! 依存を増やさないため。

#![allow(clippy::unwrap_used)]

// 112: 本物のコンポーネント 3 種をプロセス内で起動する部品(`spawn_components_listening`)だけ使う。
mod common;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::{broadcast, oneshot};
use tpcdaq::command::{Command, CommandResponse, ComponentState};
use tpcdaq::config::ConfigIds;
use tpcdaq::controller::{
    run_controller, BoundEndpoints, CoboSpec, ComponentEndpoint, ComponentKind, ControllerParams,
};

// ---------------------------------------------------------------------
// 手書き HTTP クライアント
// ---------------------------------------------------------------------

/// 1 リクエスト = 1 接続。`(status, body)` を返す。
fn http(method: &str, addr: SocketAddr, path: &str, body: Option<&str>) -> (u16, Value) {
    let payload = body.unwrap_or("");
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{payload}",
        payload.len()
    );
    let mut stream = TcpStream::connect(addr).expect("connect to the controller");
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
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
    let value = serde_json::from_str(&body).unwrap_or(Value::String(body));
    (status, value)
}

fn get(addr: SocketAddr, path: &str) -> (u16, Value) {
    http("GET", addr, path, None)
}

fn post(addr: SocketAddr, path: &str, body: Value) -> (u16, Value) {
    http("POST", addr, path, Some(&body.to_string()))
}

fn put(addr: SocketAddr, path: &str, body: Value) -> (u16, Value) {
    http("PUT", addr, path, Some(&body.to_string()))
}

// ---------------------------------------------------------------------
// フェイクのコンポーネント(本物の状態機械 + コマンド記録)
// ---------------------------------------------------------------------

#[derive(Default)]
struct Recorder {
    /// 受けたコマンド名(`GetStatus` は観測なので記録しない)。
    commands: Mutex<Vec<String>>,
    /// `Configure` で受け取った設定断片(run 開始 TS の配布を見るため)。
    configs: Mutex<Vec<Value>>,
    /// 113: コマンド名 → 残り「断る回数」(`u32::MAX` = 常に断る)。停止時の Reset 不達や
    /// Configure の失敗を作るため。断ったコマンドも `commands` には記録される。
    refusals: Mutex<Vec<(&'static str, u32)>>,
}

impl Recorder {
    /// 113: 次の `times` 回の `command` を断らせる(`u32::MAX` = 常に)。
    fn refuse(&self, command: &'static str, times: u32) {
        self.refusals.lock().unwrap().push((command, times));
    }

    /// 断る番なら残り回数を 1 減らして `true`。
    fn take_refusal(&self, command: &str) -> bool {
        let mut refusals = self.refusals.lock().unwrap();
        for (name, left) in refusals.iter_mut() {
            if *name == command && *left > 0 {
                if *left != u32::MAX {
                    *left -= 1;
                }
                return true;
            }
        }
        false
    }

    fn push(&self, name: &str) {
        self.commands.lock().unwrap().push(name.to_string());
    }

    fn commands(&self) -> Vec<String> {
        self.commands.lock().unwrap().clone()
    }

    fn last_config(&self) -> Value {
        self.configs
            .lock()
            .unwrap()
            .last()
            .cloned()
            .unwrap_or(Value::Null)
    }
}

struct FakeComponent {
    state: ComponentState,
    run_number: Option<u32>,
    metrics: Value,
    recorder: Arc<Recorder>,
}

impl FakeComponent {
    fn handle(&mut self, command: Command) -> CommandResponse {
        let name = match &command {
            Command::Configure(_) => "Configure",
            Command::Arm => "Arm",
            Command::Start { .. } => "Start",
            Command::Stop => "Stop",
            Command::Reset => "Reset",
            Command::GetStatus => "GetStatus",
        };
        if name != "GetStatus" {
            self.recorder.push(name);
        }
        let respond = |fake: &Self, ok: bool, message: String| {
            let mut response = if ok {
                CommandResponse::success(fake.state, message)
            } else {
                CommandResponse::error(fake.state, message)
            };
            if let Some(run) = fake.run_number {
                response = response.with_run_number(run);
            }
            response.with_metrics(fake.metrics.clone())
        };

        let Some(target) = command.target_state() else {
            return respond(self, true, "status".to_string());
        };
        if self.recorder.take_refusal(name) {
            return respond(self, false, format!("fake refuses {name}"));
        }
        if !self.state.can_transition_to(target) {
            return respond(
                self,
                false,
                format!("illegal transition {} -> {target}", self.state),
            );
        }
        if let Command::Configure(config) = &command {
            self.run_number = Some(config.run_number);
            self.recorder
                .configs
                .lock()
                .unwrap()
                .push(config.config.clone());
        }
        if let Command::Start { run_number } = &command {
            self.run_number = Some(*run_number);
        }
        self.state = target;
        respond(self, true, format!("{name} ok"))
    }
}

/// フェイクを 1 個起動し、実 bind エンドポイントを返す。
async fn spawn_fake(
    metrics: Value,
    shutdown: broadcast::Receiver<()>,
) -> (String, Arc<Recorder>, tokio::task::JoinHandle<()>) {
    let recorder = Arc::new(Recorder::default());
    let mut fake = FakeComponent {
        state: ComponentState::Idle,
        run_number: None,
        metrics,
        recorder: Arc::clone(&recorder),
    };
    let (bound_tx, bound_rx) = oneshot::channel();
    let handle = tokio::spawn(tpcdaq::command::run_command_task(
        "tcp://127.0.0.1:0".to_string(),
        "fake",
        shutdown,
        Some(bound_tx),
        move |command| fake.handle(command),
    ));
    let endpoint = bound_rx.await.unwrap();
    (endpoint, recorder, handle)
}

// ---------------------------------------------------------------------
// フェイクの ecc-bridge(SPEC §8.2 の JSON REQ/REP)
// ---------------------------------------------------------------------

struct FakeEcc {
    endpoint: String,
    requests: Arc<Mutex<Vec<Value>>>,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl FakeEcc {
    fn spawn() -> Self {
        let context = zmq::Context::new();
        let socket = context.socket(zmq::REP).unwrap();
        socket.set_rcvtimeo(200).unwrap();
        socket.bind("tcp://127.0.0.1:*").unwrap();
        let endpoint = socket.get_last_endpoint().unwrap().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));

        let thread_requests = Arc::clone(&requests);
        let thread_stop = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            // ECC の状態遷移(`tools/ecc_bridge/ecc_core.hpp` = 実 ECC の SM と同じ意味論)。
            let mut state = "Off".to_string();
            // 043: GET の error フラグ。初期値は NO_ERR(`BackEnd::BackEnd()`、
            // BackEnd.cpp:132)。**遷移が渡ったときだけ**消える(`resetErrorFlag` は全遷移の
            // 1 番目のアクション、BackEnd.cpp:249-290 / :1146-1149)—— 無音の `Ignored` では
            // アクションが 1 つも走らないので残る。この fake は失敗しないので立つことは無い。
            let mut ecc_error = "NO_ERR".to_string();
            while !thread_stop.load(Ordering::Relaxed) {
                let message = match socket.recv_bytes(0) {
                    Ok(message) => message,
                    Err(_) => continue, // EAGAIN = 200 ms の目覚まし
                };
                let request: Value = serde_json::from_slice(&message).unwrap_or(Value::Null);
                let action = request
                    .get("action")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                thread_requests.lock().unwrap().push(request);
                // **036: 実 ECC の状態機械そのもの**(reference/20190315_patched の
                // `GetBench/src/get/rc/BackEnd.cpp:250-270` / `:924` / `:955-962`)。
                // 以前ここは「reset はどこからでも Idle」だった —— その甘さが 034 の
                // 誤実装を green で通した。**ダブルを実機より甘くしない**。
                // 遷移が無い組み合わせは実 ECC と同じく**無音**(状態は動かず ok は true。
                // `StateMachine/src/dhsm/Engine.cpp:334-352` の `Ignored`)。
                let next = match (action.as_str(), state.as_str()) {
                    ("describe", "Off" | "Idle" | "Described") => Some("Described"),
                    ("prepare", "Described" | "Prepared") => Some("Prepared"),
                    // BackEnd::configure の ST_PREPARED ガード = それ以外は黙ってスキップ。
                    ("configure", "Prepared") => Some("Ready"),
                    ("start", "Ready") => Some("Running"),
                    ("stop", "Running" | "Paused") => Some("Ready"),
                    // EV_BREAK: Active(Ready/Running/Paused)→ Prepared。
                    ("breakup", "Ready" | "Running" | "Paused") => Some("Prepared"),
                    // EV_UNDO は **1 段戻すだけ**(Active からは遷移が無い)。
                    ("reset", "Described") => Some("Idle"),
                    ("reset", "Prepared") => Some("Described"),
                    _ => None,
                };
                if let Some(next) = next {
                    state = next.to_string();
                    ecc_error = "NO_ERR".to_string();
                }
                let reply =
                    json!({"ok": true, "state": state, "error": "", "ecc_error": ecc_error});
                let _ = socket.send(reply.to_string().as_bytes(), 0);
            }
        });
        Self {
            endpoint,
            requests,
            stop,
            handle: Some(handle),
        }
    }

    /// ECC を**動かした**操作の列(`status` は `GET /api/status` が撃つ観測なので除く)。
    fn actions(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter_map(|r| r.get("action").and_then(Value::as_str))
            .filter(|action| *action != "status")
            .map(str::to_string)
            .collect()
    }

    /// その action の要求を**受けた順に全部**(112: 周回ごとの configure / prepare の id を見る)。
    fn requests_for(&self, action: &str) -> Vec<Value> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.get("action").and_then(Value::as_str) == Some(action))
            .cloned()
            .collect()
    }

    fn request(&self, action: &str) -> Option<Value> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.get("action").and_then(Value::as_str) == Some(action))
            .cloned()
    }
}

impl Drop for FakeEcc {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

// ---------------------------------------------------------------------
// ハーネス
// ---------------------------------------------------------------------

struct Harness {
    rest: SocketAddr,
    log_pull: String,
    output_root: PathBuf,
    graw_writer: Arc<Recorder>,
    decoder: Arc<Recorder>,
    receivers: Vec<Arc<Recorder>>,
    /// receiver が「実際に bind した」ポート(DataLinkSet に載る値のオラクル)。
    router_ports: Vec<u16>,
    ecc: FakeEcc,
    shutdown: broadcast::Sender<()>,
    /// receiver 役が握っているデータポート(落とすとポートが再利用されうるので保持する)。
    _data_listeners: Vec<TcpListener>,
}

fn temp_root(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "tpcdaq-controller-it-{tag}-{}-{n}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// フェイク一式 + controller を起動する。`eos_closed` が false なら
/// 「EOS がまだ流れていない」形の metrics を返す(強制 EOS 分岐の再現用)。
///
/// ConfigId は 3 相同値(`"integration"`)—— 相ごとに変える版は [`harness_with_config_ids`]。
async fn harness(
    tag: &str,
    cobo_ids: &[u32],
    eos_closed: bool,
    ui_dir: Option<PathBuf>,
) -> Harness {
    harness_with_config_ids(
        tag,
        cobo_ids,
        eos_closed,
        ui_dir,
        ConfigIds::same("integration"),
    )
    .await
}

/// [`harness`] の ConfigId 指定版(042: describe / prepare / configure が別名の実運用)。
async fn harness_with_config_ids(
    tag: &str,
    cobo_ids: &[u32],
    eos_closed: bool,
    ui_dir: Option<PathBuf>,
    config_ids: ConfigIds,
) -> Harness {
    harness_full(tag, cobo_ids, eos_closed, ui_dir, config_ids, None).await
}

/// [`harness_with_config_ids`] の `ecc_config_dir` 指定版(078 レーン B: params 系 REST の
/// テスト用 — `None` なら従来どおり params REST は 503)。
async fn harness_full(
    tag: &str,
    cobo_ids: &[u32],
    eos_closed: bool,
    ui_dir: Option<PathBuf>,
    config_ids: ConfigIds,
    ecc_config_dir: Option<PathBuf>,
) -> Harness {
    let (shutdown, _) = broadcast::channel(1);
    let output_root = temp_root(tag);

    let (graw_endpoint, graw_writer, _) = spawn_fake(
        json!({
            "files_open": if eos_closed { 0 } else { 1 },
            "files_closed": 2,
            // 非対称なバイト数(取り違え検出用)。
            "files": [
                {"path": "run0001/CoBo0_AsAd0_ts_0000.graw", "bytes": 30_108_672u64},
                {"path": "run0001/CoBo0_AsAd1_ts_0000.graw", "bytes": 30_108_100u64},
            ],
        }),
        shutdown.subscribe(),
    )
    .await;
    let (decoder_endpoint, decoder, _) = spawn_fake(
        json!({
            "eos_in": if eos_closed { cobo_ids.len() } else { 0 },
            // TODO/033-C: **自分の EOS を送り終えたか**(チェーンの最後のホップ)。
            // controller の `eos_complete` は eos_in と対でこれも読む。
            "eos_out": if eos_closed { 1 } else { 0 },
            "malformed": 4,
            "fragments_out": 3852,
        }),
        shutdown.subscribe(),
    )
    .await;

    let mut components = vec![
        ComponentEndpoint {
            name: "graw-writer".to_string(),
            endpoint: graw_endpoint,
            kind: ComponentKind::GrawWriter,
        },
        ComponentEndpoint {
            name: "decoder".to_string(),
            endpoint: decoder_endpoint,
            kind: ComponentKind::Decoder,
        },
    ];
    let mut receivers = Vec::new();
    let mut router_ports = Vec::new();
    let mut cobos = Vec::new();
    let mut data_listeners = Vec::new();
    for (i, &id) in cobo_ids.iter().enumerate() {
        // 「listen-before-start で実際に bind した口」を本物のポートで模す。
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        data_listeners.push(listener);
        router_ports.push(address.port());

        let (endpoint, recorder, _) = spawn_fake(
            json!({
                "cobo_id": id,
                "bind_address": address.to_string(),
                "router_port": address.port(),
                // 非対称なフレーム数(CoBo 毎の取り違え検出用)。
                "frames": 3852 - i as u64,
                "overflow_frames": i as u64,
                // TODO/033-E: 受信静止(quiesce)判定の材料。フェイクは値が**動かない**
                // ので、EOS が来ない構成では第一段が静止で切り上がる(= 実機の姿)。
                "bytes": 30_108_684u64 - i as u64,
            }),
            shutdown.subscribe(),
        )
        .await;
        components.push(ComponentEndpoint {
            name: format!("receiver{id}"),
            endpoint,
            kind: ComponentKind::Receiver { cobo_id: id },
        });
        receivers.push(recorder);
        cobos.push(CoboSpec {
            id,
            listen: format!("0.0.0.0:{}", 46005 + id),
            data_sender_id: format!("CoBo[{id}]"),
        });
    }

    let (BoundEndpoints { rest, log_pull }, ecc, _) = spawn_controller(
        &shutdown,
        &output_root,
        cobos,
        components,
        ui_dir,
        config_ids,
        ecc_config_dir,
    )
    .await;

    Harness {
        rest,
        log_pull,
        output_root,
        graw_writer,
        decoder,
        receivers,
        router_ports,
        ecc,
        shutdown,
        _data_listeners: data_listeners,
    }
}

/// fake ECC + controller(REST)を起動する。コンポーネントはフェイク([`harness_full`])でも
/// 本物([`real_chain`]、112)でもよい —— controller から見えるのはコマンド REP だけ。
/// 使った [`ControllerParams`] も返す(114: 同じ params で controller だけを作り直すため)。
async fn spawn_controller(
    shutdown: &broadcast::Sender<()>,
    output_root: &Path,
    cobos: Vec<CoboSpec>,
    components: Vec<ComponentEndpoint>,
    ui_dir: Option<PathBuf>,
    config_ids: ConfigIds,
    ecc_config_dir: Option<PathBuf>,
) -> (BoundEndpoints, FakeEcc, ControllerParams) {
    let ecc = FakeEcc::spawn();
    let params = ControllerParams {
        rest_listen: "127.0.0.1:0".to_string(),
        passphrase: "change-me".to_string(),
        log_pull_bind: "tcp://127.0.0.1:*".to_string(),
        ui_dir,
        config_ids,
        output_root: output_root.to_path_buf(),
        geometry_path: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/geometry_2cobo_fake.dat"),
        cobos,
        components,
        ecc_endpoint: ecc.endpoint.clone(),
        router_ip: None,
        ecc_config_dir,
        // 既定(`<output_root>/params`)をそのまま使う — テストごとに output_root が
        // 一意なので、これだけで test 間の干渉は無い。
        params_dir: None,
        // 強制 EOS 分岐を秒単位で待たずに試せるよう、テストでは短くする。
        eos_timeout: Duration::from_millis(400),
        // TODO/033-E で新設(SPEC §1.3 v1.12)。eos_timeout と同じく短く ——
        // ここは静止検出そのものではなく既存シナリオの再現が目的。
        eos_quiesce: Duration::from_millis(200),
        eos_poll: Duration::from_millis(50),
        command_timeout: Duration::from_secs(5),
        ecc_timeout: Duration::from_secs(5),
        status_timeout: Duration::from_secs(2),
    };

    let bound = launch_controller(params.clone(), shutdown).await;
    (bound, ecc, params)
}

/// controller を `params` のまま 1 つ起動し、`shutdown` で止まるようにする。
async fn launch_controller(
    params: ControllerParams,
    shutdown: &broadcast::Sender<()>,
) -> BoundEndpoints {
    let (bound_tx, bound_rx) = oneshot::channel();
    let shutdown_rx = shutdown.subscribe();
    tokio::spawn(async move {
        if let Err(e) = run_controller(params, shutdown_rx, Some(bound_tx)).await {
            panic!("controller failed: {e}");
        }
    });
    bound_rx.await.unwrap()
}

impl Harness {
    fn acquire(&self, operator: &str) -> String {
        let (status, body) = post(
            self.rest,
            "/api/control/acquire",
            json!({"operator": operator, "passphrase": "change-me"}),
        );
        assert_eq!(status, 200, "{body}");
        body["token"].as_str().unwrap().to_string()
    }

    fn logbook(&self) -> Vec<Value> {
        let (status, body) = get(self.rest, "/api/logbook?since_seq=0");
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["tail_corrupt"], json!(false));
        body["records"].as_array().unwrap().clone()
    }

    fn shutdown(&self) {
        let _ = self.shutdown.send(());
        let _ = std::fs::remove_dir_all(&self.output_root);
    }
}

/// 述語が真になるまで待つ(実 ZMQ / 実スレッド相手のポーリング)。
fn wait_until(what: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for {what}");
}

// ---------------------------------------------------------------------
// テスト
// ---------------------------------------------------------------------

/// run 開始 → 停止の全経路。コマンド順・DataLinkSet の中身・ログブックの並びを一度に固定する。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_full_run_drives_every_component_in_spec_order_and_writes_the_logbook() {
    let harness = harness("full-run", &[0, 1], true, None).await;

    // --- token 無しの状態変更は 401(SPEC §8.1 の二層アクセス制御)---
    let (status, body) = post(harness.rest, "/api/run/start", json!({"token": "bogus"}));
    assert_eq!(status, 401, "{body}");

    // --- 閲覧系は認証なしで通る ---
    let (status, _) = get(harness.rest, "/api/status");
    assert_eq!(status, 200);

    // --- パスフレーズ違いは 403 ---
    let (status, _) = post(
        harness.rest,
        "/api/control/acquire",
        json!({"operator": "intruder", "passphrase": "wrong"}),
    );
    assert_eq!(status, 403);

    // --- 操作権取得 → run 開始 ---
    let token = harness.acquire("aogaki");
    let (status, body) = post(
        harness.rest,
        "/api/run/start",
        json!({"token": token, "comment": "gas test"}),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["run"],
        json!(1),
        "state ファイル不在なら run は 1 から"
    );

    // --- §1.3 の順序: 各コンポーネントが Configure → Arm → Start を 1 回ずつ ---
    for (name, recorder) in [
        ("graw-writer", &harness.graw_writer),
        ("decoder", &harness.decoder),
        ("receiver0", &harness.receivers[0]),
        ("receiver1", &harness.receivers[1]),
    ] {
        assert_eq!(
            recorder.commands(),
            vec!["Configure", "Arm", "Start"],
            "{name}"
        );
    }
    // --- run 開始 TS の一元化(SPEC §6.4 v1.8): controller が 1 つだけ決めた TS を
    //     全コンポーネントへ**同じ値で**配る(各自のローカル時刻で数秒ずれない)---
    let ts = harness.graw_writer.last_config()["run_start_ts"]
        .as_str()
        .expect("Configure carries run_start_ts")
        .to_string();
    assert_eq!(ts.len(), 23, "graw-writer のファイル名 TS と同じ書式: {ts}");
    for recorder in [
        &harness.decoder,
        &harness.receivers[0],
        &harness.receivers[1],
    ] {
        assert_eq!(recorder.last_config()["run_start_ts"], json!(ts));
    }

    // --- ecc は describe → prepare → configure → start ---
    // 034/036: run 開始は必ず「ECC を Idle へ歩き戻す」段から始まるが、このフェイク ECC は
    // 起動直後 = `Off`(これ以上戻れない底)なので、歩き戻しは `status` の 1 発だけで済み
    // **1 コマンドも発行されない**。歩く姿は 2 本目以降が見せる
    // (`three_consecutive_runs_…` を参照)。`actions()` は `status` を除いている。
    assert_eq!(
        harness.ecc.actions(),
        vec!["describe", "prepare", "configure", "start"]
    );
    // --- DataLinkSet は receiver が**実際に bind したポート**で組まれる(SPEC §8.2)---
    let configure = harness.ecc.request("configure").unwrap();
    assert_eq!(
        configure,
        json!({
            "action": "configure",
            "config_id": "integration",
            "links": [
                {"sender": "CoBo[0]", "router_ip": "127.0.0.1",
                 "router_port": harness.router_ports[0], "type": "TCP"},
                {"sender": "CoBo[1]", "router_ip": "127.0.0.1",
                 "router_port": harness.router_ports[1], "type": "TCP"},
            ],
        })
    );

    // --- status は自身の相 + 全コンポーネント + ecc を集約する ---
    let (status, body) = get(harness.rest, "/api/status");
    assert_eq!(status, 200);
    assert_eq!(body["phase"], json!("Running"));
    assert_eq!(body["run"]["run"], json!(1));
    assert_eq!(body["operator"], json!("aogaki"));
    assert_eq!(body["ecc"]["state"], json!("Running"));
    // 043: GET の error フラグは `/api/status` の ecc に**素通しで**載る(SPEC §8.2 v1.14)。
    // ここは正常系なので `NO_ERR` —— 空文字や欠落なら素通しの配線が切れている。
    assert_eq!(body["ecc"]["ecc_error"], json!("NO_ERR"), "{}", body["ecc"]);
    let components = body["components"].as_array().unwrap();
    assert_eq!(components.len(), 4);
    for component in components {
        assert_eq!(component["reachable"], json!(true), "{component}");
        assert_eq!(component["state"], json!("Running"), "{component}");
    }

    // --- run 停止 ---
    let (status, body) = post(harness.rest, "/api/run/stop", json!({"token": token}));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], json!(true));
    assert_eq!(body["reason"], json!("normal"));
    assert_eq!(
        body["forced_eos"],
        json!(false),
        "EOS が来ていれば強制しない"
    );
    assert_eq!(body["eos_closed"], json!(true));

    // 畳む順は上流から。Stop → Reset で Idle まで戻す(次の run のため)。
    for (name, recorder) in [
        ("graw-writer", &harness.graw_writer),
        ("decoder", &harness.decoder),
        ("receiver0", &harness.receivers[0]),
        ("receiver1", &harness.receivers[1]),
    ] {
        assert_eq!(
            recorder.commands(),
            vec!["Configure", "Arm", "Start", "Stop", "Reset"],
            "{name}"
        );
    }
    assert_eq!(
        harness.ecc.actions(),
        vec!["describe", "prepare", "configure", "start", "stop"]
    );

    // --- ログブック(SPEC §9.2)---
    let records = harness.logbook();
    let kinds: Vec<&str> = records
        .iter()
        .map(|r| r["type"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        vec![
            "audit",     // 1: token 無しの run/start
            "audit",     // 2: パスフレーズ違いの acquire
            "audit",     // 3: acquire 成功
            "run_start", // 4
            "audit",     // 5: run/start 成功
            "run_stop",  // 6
            "audit",     // 7: run/stop 成功
        ]
    );
    // seq は 1 から単調増加(015 の writer が付ける)。
    assert_eq!(
        records
            .iter()
            .map(|r| r["seq"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5, 6, 7]
    );
    assert_eq!(records[0]["action"], json!("run/start"));
    assert_eq!(records[0]["ok"], json!(false));
    assert_eq!(records[0]["operator"], json!("<unauthorized>"));
    assert_eq!(records[1]["action"], json!("control/acquire"));
    assert_eq!(records[1]["ok"], json!(false));
    assert_eq!(records[2]["params"]["preempted"], Value::Null);

    // run_start(§9.2 の全フィールド)
    let run_start = &records[3];
    assert_eq!(run_start["run"], json!(1));
    assert_eq!(run_start["config_id"], json!("integration"));
    assert_eq!(run_start["operator"], json!("aogaki"));
    assert_eq!(run_start["comment"], json!("gas test"));
    assert_eq!(run_start["actor"], json!("controller"));
    assert_eq!(
        run_start["cobos"],
        json!([{"id": 0, "listen": "0.0.0.0:46005"}, {"id": 1, "listen": "0.0.0.0:46006"}])
    );
    assert_eq!(run_start["geometry"]["sha256"].as_str().unwrap().len(), 64);
    // 合成 2-CoBo フィクスチャの申告どおり(CoBo 0 = AsAd 1 枚、CoBo 1 = 2 枚)。
    assert_eq!(
        run_start["expected_fragments"],
        json!([[0, 0], [1, 0], [1, 1]])
    );

    // run_stop(counters / files は GetStatus 実測から)
    let run_stop = &records[5];
    assert_eq!(run_stop["run"], json!(1));
    assert_eq!(run_stop["ok"], json!(true));
    assert_eq!(run_stop["reason"], json!("normal"));
    // TODO/033-A(SPEC §9.2 v1.12): 停止の顛末は **run_stop にも**載る
    // (以前は audit と REST 応答にしか無く、041 の実 run で不履行が確認された)。
    // このハーネスは EOS が最初から流れ切っている形なので、注入していない = false。
    assert_eq!(run_stop["forced_eos"], json!(false), "{run_stop}");
    assert_eq!(run_stop["eos_closed"], json!(true), "{run_stop}");
    assert_eq!(
        run_stop["counters"]["frames"],
        json!({"0": 3852, "1": 3851})
    );
    assert_eq!(run_stop["counters"]["overflow_frames"], json!(1)); // 0 + 1
    assert_eq!(run_stop["counters"]["malformed"], json!(4));
    // root-sink は REP を持たない = 取得手段が無いので null(025, SPEC v1.10 §9.2 —
    // `Counters` の Option 化。016 時点は非 Option で 0 埋めだった)。
    assert_eq!(run_stop["counters"]["events_built"], json!(null));
    assert_eq!(run_stop["counters"]["events_incomplete"], json!(null));
    assert_eq!(run_stop["counters"]["late_fragments"], json!(null));
    assert_eq!(
        run_stop["files"],
        json!([
            {"path": "run0001/CoBo0_AsAd0_ts_0000.graw", "bytes": 30_108_672u64},
            {"path": "run0001/CoBo0_AsAd1_ts_0000.graw", "bytes": 30_108_100u64},
        ])
    );
    assert!(run_stop["duration_s"].as_f64().unwrap() >= 0.0);

    harness.shutdown();
}

/// EOS が来なければ receiver へ Stop を撃って強制 EOS(SPEC §1.3)。
/// 実 ZMQ でも「強制した receiver へ Stop を二重に撃たない」ことを確認する。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stop_without_eos_forces_it_through_the_receivers() {
    let harness = harness("forced-eos", &[0], false, None).await;
    let token = harness.acquire("aogaki");
    let (status, _) = post(
        harness.rest,
        "/api/run/start",
        json!({"token": token, "comment": ""}),
    );
    assert_eq!(status, 200);

    let (status, body) = post(harness.rest, "/api/run/stop", json!({"token": token}));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["forced_eos"], json!(true));
    // フェイクは EOS を最後まで流さないので `error:eos-timeout`(silent にしない)。
    assert_eq!(body["eos_closed"], json!(false));
    assert_eq!(body["ok"], json!(false));
    assert_eq!(body["reason"], json!("error:eos-timeout"));

    assert_eq!(
        harness.receivers[0].commands(),
        vec!["Configure", "Arm", "Start", "Stop", "Reset"],
        "強制 EOS の Stop と畳む段の Stop を二重に撃たない"
    );

    let records = harness.logbook();
    let run_stop = records
        .iter()
        .find(|r| r["type"] == json!("run_stop"))
        .unwrap();
    assert_eq!(run_stop["ok"], json!(false));
    assert_eq!(run_stop["reason"], json!("error:eos-timeout"));
    // TODO/033-A: **audit と同じ 2 値が run_stop にも載る**(REST 応答と一致すること)。
    // `eos_closed=false` が異常の印、`forced_eos=true` は実機なら常態(SPEC §9.2 v1.14)。
    assert_eq!(run_stop["forced_eos"], body["forced_eos"], "{run_stop}");
    assert_eq!(run_stop["eos_closed"], body["eos_closed"], "{run_stop}");
    assert_eq!(run_stop["forced_eos"], json!(true), "{run_stop}");
    assert_eq!(run_stop["eos_closed"], json!(false), "{run_stop}");

    harness.shutdown();
}

/// controller を通さずにコンポーネント(`/api/status` の `components[index]`)へ
/// `Configure` を直接撃ち、Idle から動かしておく(113 以前の巻き戻し試験の前提作り)。
fn configure_directly(harness: &Harness, index: usize) {
    let context = zmq::Context::new();
    let endpoint = {
        let (_, body) = get(harness.rest, "/api/status");
        body["components"][index]["endpoint"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let response = tpcdaq::command::request(
        &context,
        &endpoint,
        &Command::Configure(tpcdaq::command::RunConfig {
            run_number: 99,
            comment: String::new(),
            config: Value::Null,
        }),
        Duration::from_secs(5),
    )
    .unwrap();
    assert!(response.success, "前提: Configured になっている");
}

/// 開始シーケンスが途中で失敗したら巻き戻す(実 ZMQ 版)。
/// receiver0 が controller の `Configure` を断る —— そこから先へ進まず、実行済みだけが畳まれる。
///
/// 113 以前は「receiver を先に Configure 済みにしておく」で断らせていたが、113 から start は
/// Idle でないコンポーネントを先に Reset で歩き戻すので、その前提では start が**通る**
/// (= [`a_component_left_configured_by_a_lost_reset_is_walked_back_at_the_next_start`])。
/// ここではフェイクに Configure そのものを断らせる。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_start_rolls_back_and_leaves_no_half_started_run() {
    let harness = harness("rollback", &[0], true, None).await;
    harness.receivers[0].refuse("Configure", u32::MAX);

    let token = harness.acquire("aogaki");
    let (status, body) = post(
        harness.rest,
        "/api/run/start",
        json!({"token": token, "comment": ""}),
    );
    assert_eq!(status, 500, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("Configure"),
        "{body}"
    );

    // 実行済み(graw-writer, decoder)だけが Stop → Reset される。
    assert_eq!(
        harness.graw_writer.commands(),
        vec!["Configure", "Stop", "Reset"]
    );
    assert_eq!(
        harness.decoder.commands(),
        vec!["Configure", "Stop", "Reset"]
    );
    // ECC を**走らせる**段(describe 以降)へは進まない。歩き戻しの段は、この ECC が
    // 起動直後 = `Off` なので `status` だけ(= `actions()` は空)。失敗しても ECC は
    // 一番安全な状態に置かれたままなので巻き戻す必要がない(034 論点 A / 036)。
    assert!(
        harness.ecc.actions().is_empty(),
        "ECC を走らせる段へ進まない: {:?}",
        harness.ecc.actions()
    );

    // 相は Idle に戻り、もう一度 start できる状態。
    let (_, body) = get(harness.rest, "/api/status");
    assert_eq!(body["phase"], json!("Idle"));

    // **run 番号は空費しない**(034 論点 C)。`run_start` をログブックへ書く前に落ちたので
    // 採番を巻き戻す —— 状態ファイルを直接読んで、次の run がまた 1 番であることを見る。
    let state: Value = serde_json::from_str(
        &std::fs::read_to_string(harness.output_root.join("tpcdaq_state.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(state["next_run"], json!(1), "採番が巻き戻っていない");

    let records = harness.logbook();
    let audit = records.last().unwrap();
    assert_eq!(audit["type"], json!("audit"));
    assert_eq!(audit["action"], json!("run/start"));
    assert_eq!(audit["ok"], json!(false));
    assert_eq!(audit["params"]["stage"], json!("Configure"));
    // 巻き戻したこと自体も監査に残す(silent にしない)。
    assert_eq!(audit["params"]["next_run_rewound"], json!(true));
    assert!(!records.iter().any(|r| r["type"] == json!("run_start")));

    harness.shutdown();
}

/// **113 の本体**(112 レーン A で実測した穴): 停止の畳む段で receiver0 への `Reset` が
/// 届かないと、`run/stop` は `ok:true` + `notes` で返り receiver0 は `Configured` のまま残る。
/// 113 以前は次の `run/start` が `Configure refused in state Configured` で 500 になり、
/// プロセス再起動まで直らなかった。113 からは start が **Idle でないものを Reset してから**
/// Configure するので通り、歩いた跡が audit `run/start.component_walk_back` に残る。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_component_left_configured_by_a_lost_reset_is_walked_back_at_the_next_start() {
    let harness = harness("walk-back", &[0], true, None).await;
    let token = harness.acquire("aogaki");

    let (status, body) = post(
        harness.rest,
        "/api/run/start",
        json!({"token": token, "comment": "first"}),
    );
    assert_eq!(status, 200, "{body}");

    // 停止の畳む段の Reset を 1 回だけ落とす(実測ではエフェメラルポート枯渇で不達)。
    harness.receivers[0].refuse("Reset", 1);
    let (status, body) = post(harness.rest, "/api/run/stop", json!({"token": token}));
    assert_eq!(status, 200, "{body}");
    // stop の契約は不変: ok = EOS と保存の完結、Reset の失敗は notes に載る。
    assert_eq!(body["ok"], json!(true), "{body}");
    let notes = body["notes"].as_array().unwrap();
    assert!(
        notes
            .iter()
            .any(|n| n.as_str().unwrap().starts_with("receiver0: Reset refused")),
        "{body}"
    );
    let (_, status_body) = get(harness.rest, "/api/status");
    assert_eq!(status_body["phase"], json!("Idle"));
    assert_eq!(
        status_body["components"][2]["state"],
        json!("Configured"),
        "前提: receiver0 は Configured のまま残っている"
    );

    let (status, body) = post(
        harness.rest,
        "/api/run/start",
        json!({"token": token, "comment": "second"}),
    );
    assert_eq!(status, 200, "歩き戻せば 2 本目が通る: {body}");
    assert_eq!(body["run"], json!(2));

    // receiver0 だけが 2 本目の Configure の前に Reset を受け直す(1 本目の Stop の後の
    // Reset は断られた分)。Idle に戻っていた graw-writer / decoder には余計な Reset が無い。
    assert_eq!(
        harness.receivers[0].commands(),
        vec![
            "Configure",
            "Arm",
            "Start",
            "Stop",
            "Reset",
            "Reset",
            "Configure",
            "Arm",
            "Start"
        ]
    );
    assert_eq!(
        harness.decoder.commands(),
        vec![
            "Configure",
            "Arm",
            "Start",
            "Stop",
            "Reset",
            "Configure",
            "Arm",
            "Start"
        ]
    );

    let records = harness.logbook();
    let audit = records
        .iter()
        .rev()
        .find(|r| r["type"] == json!("audit") && r["action"] == json!("run/start"))
        .unwrap();
    assert_eq!(audit["ok"], json!(true), "{audit}");
    assert_eq!(audit["params"]["run"], json!(2));
    assert_eq!(
        audit["params"]["component_walk_back"],
        json!(["receiver0: Configured->Idle"])
    );
    // 1 本目(全員 Idle)は空の配列で残る(キーは常にある)。
    let first = records
        .iter()
        .find(|r| r["type"] == json!("audit") && r["action"] == json!("run/start"))
        .unwrap();
    assert_eq!(first["params"]["component_walk_back"], json!([]));

    let (status, body) = post(harness.rest, "/api/run/stop", json!({"token": token}));
    assert_eq!(status, 200, "{body}");
    harness.shutdown();
}

/// 歩き戻しの `Reset` まで断られたら run を始めない: 500 の文言に**どれが・何の状態で**
/// 残ったかが載り、どのコンポーネントにも Configure は出ず、採番は巻き戻る。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_component_that_cannot_be_walked_back_refuses_the_start_and_says_which() {
    let harness = harness("walk-back-refused", &[0], true, None).await;
    configure_directly(&harness, 2); // receiver0 → Configured
    harness.receivers[0].refuse("Reset", u32::MAX);

    let token = harness.acquire("aogaki");
    let (status, body) = post(
        harness.rest,
        "/api/run/start",
        json!({"token": token, "comment": ""}),
    );
    assert_eq!(status, 500, "{body}");
    let error = body["error"].as_str().unwrap();
    assert!(error.contains("receiver0"), "{body}");
    assert!(error.contains("Configured"), "{body}");

    // controller からの Configure は誰も受けていない(receiver0 の 1 発目は直撃ちの前提作り)。
    assert!(harness.graw_writer.commands().is_empty());
    assert!(harness.decoder.commands().is_empty());
    assert_eq!(harness.receivers[0].commands(), vec!["Configure", "Reset"]);

    let (_, status_body) = get(harness.rest, "/api/status");
    assert_eq!(status_body["phase"], json!("Idle"));
    let records = harness.logbook();
    let audit = records.last().unwrap();
    assert_eq!(audit["action"], json!("run/start"));
    assert_eq!(audit["ok"], json!(false));
    assert_eq!(audit["params"]["stage"], json!("walk-back"));
    assert_eq!(audit["params"]["next_run_rewound"], json!(true));

    harness.shutdown();
}

/// **034 の出口(モック版)**: `POST /api/ecc/reset` を**手で挟まずに**連続 3 run が通り、
/// run 番号が 1, 2, 3 と飛ばずに進む。
///
/// 030 の実測では 2 本目の `run/start` が `ecc describe failed in state Ready` で必ず失敗し、
/// 失敗のたびに番号が 1 つ消えていた(run 1 → run 4)。ここは controller が毎回 `reset` を
/// 撃つことと、失敗しない限り番号が連番であることを固定する。**実 ECC 遷移表相手の
/// 出口は `tests/p3_e2e.rs` の E2E-H**(このフェイク ECC は順序違反を断らないため)。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_consecutive_runs_need_no_manual_ecc_reset_and_keep_the_run_numbers_consecutive() {
    let harness = harness("consecutive", &[0], true, None).await;
    let token = harness.acquire("aogaki");

    let mut runs = Vec::new();
    for pass in 0..3 {
        let (status, body) = post(
            harness.rest,
            "/api/run/start",
            json!({"token": token, "comment": format!("consecutive {pass}")}),
        );
        assert_eq!(status, 200, "{pass} 本目の run/start が落ちた: {body}");
        runs.push(body["run"].as_u64().unwrap());

        let (status, body) = post(harness.rest, "/api/run/stop", json!({"token": token}));
        assert_eq!(status, 200, "{pass} 本目の run/stop が落ちた: {body}");
        assert_eq!(body["ok"], json!(true), "{body}");
    }

    // 番号は飛ばない(030 実測の run 1 → run 4 が起きない)。
    assert_eq!(runs, vec![1, 2, 3]);

    // ECC の呼び出し列(036 の出口。`status` は `actions()` が除いている):
    //
    //   1 本目: ECC は起動直後の `Off` = これ以上戻れない底なので**歩かない**。
    //   2 本目以降: 前の run の `ecc stop` が `Ready` に置いているので
    //     **breakup → reset → reset** で `Idle` まで歩き戻してから describe に入る
    //     (実 ECC の `EV_UNDO` は 1 段ずつ、`Active` からは無音で無視 —— 036)。
    let mut expected: Vec<&str> = vec!["describe", "prepare", "configure", "start", "stop"];
    for _ in 0..2 {
        expected.extend([
            "breakup",
            "reset",
            "reset",
            "describe",
            "prepare",
            "configure",
            "start",
            "stop",
        ]);
    }
    assert_eq!(harness.ecc.actions(), expected);

    // ログブックにも 3 本分が揃う(run_start / run_stop が run 毎に 1 組ずつ)。
    let records = harness.logbook();
    let starts: Vec<u64> = records
        .iter()
        .filter(|r| r["type"] == json!("run_start"))
        .map(|r| r["run"].as_u64().unwrap())
        .collect();
    let stops: Vec<u64> = records
        .iter()
        .filter(|r| r["type"] == json!("run_stop"))
        .map(|r| r["run"].as_u64().unwrap())
        .collect();
    assert_eq!(starts, vec![1, 2, 3]);
    assert_eq!(stops, vec![1, 2, 3]);

    harness.shutdown();
}

/// 操作権は常に横取りでき、横取りは監査に残る(SPEC §8.1)。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_control_token_is_preemptible_and_the_takeover_is_audited() {
    let harness = harness("preempt", &[0], true, None).await;

    let first = harness.acquire("aogaki");
    let second = harness.acquire("student");
    assert_ne!(first, second);

    // 横取りされた側の token はもう効かない。
    let (status, _) = post(harness.rest, "/api/run/stop", json!({"token": first}));
    assert_eq!(status, 401);

    // release は握っている側だけができる。
    let (status, _) = post(
        harness.rest,
        "/api/control/release",
        json!({"token": second}),
    );
    assert_eq!(status, 200);
    let (status, _) = post(
        harness.rest,
        "/api/control/release",
        json!({"token": second}),
    );
    assert_eq!(status, 401, "release 後の token は無効");

    let records = harness.logbook();
    let acquires: Vec<&Value> = records
        .iter()
        .filter(|r| r["action"] == json!("control/acquire"))
        .collect();
    assert_eq!(acquires.len(), 2);
    assert_eq!(acquires[0]["params"]["preempted"], Value::Null);
    assert_eq!(acquires[1]["params"]["preempted"], json!("aogaki"));
    assert_eq!(acquires[1]["operator"], json!("student"));

    harness.shutdown();
}

/// run 番号の手動設定(SPEC v1.10 §8.1、025)。token 不正は 401、0/負/非数は 400、
/// run 実行中は 409、成功した設定は次の `run/start` でそのまま使われ、成否どちらも監査される。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn run_next_sets_the_manual_run_number_and_is_gated_by_auth_input_and_phase() {
    let harness = harness("run-next", &[0], true, None).await;
    let token = harness.acquire("aogaki");

    // token 不正は 401(状態変更系 = 二層アクセス制御 — SPEC §8.1)。
    let (status, body) = post(
        harness.rest,
        "/api/run/next",
        json!({"token": "bogus", "next_run": 5}),
    );
    assert_eq!(status, 401, "{body}");

    // 0・負・非数(小数・文字列・null)はどれも 400(SPEC §8.1 v1.10「next_run は正整数のみ」)。
    let bad_values = [
        json!(0),
        json!(-1),
        json!("banana"),
        json!(3.5),
        json!(null),
    ];
    for bad in &bad_values {
        let (status, body) = post(
            harness.rest,
            "/api/run/next",
            json!({"token": token, "next_run": bad}),
        );
        assert_eq!(status, 400, "next_run={bad} => {status} {body}");
    }

    // 正常設定(非対称な値 777、既定の連番 1 から始まる採番と衝突しない)。
    let (status, body) = post(
        harness.rest,
        "/api/run/next",
        json!({"token": token, "next_run": 777}),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["next_run"], json!(777));

    // 次の run/start がその番号で走る。
    let (status, body) = post(
        harness.rest,
        "/api/run/start",
        json!({"token": token, "comment": ""}),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["run"], json!(777));

    // run 実行中(Idle 以外)は拒否(409 相当)。
    let (status, body) = post(
        harness.rest,
        "/api/run/next",
        json!({"token": token, "next_run": 900}),
    );
    assert_eq!(status, 409, "{body}");

    let (status, body) = post(harness.rest, "/api/run/stop", json!({"token": token}));
    assert_eq!(status, 200, "{body}");

    // audit: 成功 1 件(next_run=777)+ 失敗が積み上がる(silent にしない — SPEC §8.1)。
    // 手計算: token 不正 1 + 0/負/非数 5 + run 中 1 = 失敗 7 件。
    let records = harness.logbook();
    let run_next_audits: Vec<&Value> = records
        .iter()
        .filter(|r| r["type"] == json!("audit") && r["action"] == json!("run/next"))
        .collect();
    assert!(
        run_next_audits
            .iter()
            .any(|a| a["ok"] == json!(true) && a["params"]["next_run"] == json!(777)),
        "{run_next_audits:?}"
    );
    let failed = run_next_audits
        .iter()
        .filter(|a| a["ok"] == json!(false))
        .count();
    assert_eq!(failed, 7, "{run_next_audits:?}");

    harness.shutdown();
}

/// LogPost(§2.3)で投稿した JSON に controller が ts/seq を付けて記録する。
/// 壊れた投稿は捨てるが**数える**(silent にしない)。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn log_posts_are_stamped_with_ts_and_seq_and_bad_ones_are_counted() {
    let harness = harness("logpost", &[0], true, None).await;

    // REST 経由のコメント(token 不要 = 誰でも書ける R11 のログ)。
    let (status, _) = post(
        harness.rest,
        "/api/logbook/comment",
        json!({"author": "aogaki", "text": "beam tuned, 4 Hz"}),
    );
    assert_eq!(status, 200);

    let context = zmq::Context::new();
    let push = context.socket(zmq::PUSH).unwrap();
    push.set_sndtimeo(2000).unwrap();
    push.connect(&harness.log_pull).unwrap();
    // slowctl からの投稿を模す(ts / seq は付けない = controller が付ける。SPEC §9.2 v1.31)。
    push.send(
        json!({
            "type": "slow", "actor": "slowctl", "device": "gas", "node": 4,
            "event": "ALARM", "values": {"fmeasure": {"value": 130.5, "unit": "mbar"}}
        })
        .to_string()
        .as_bytes(),
        0,
    )
    .unwrap();
    // 壊れた投稿(ログブックのレコードではない)。
    push.send(b"{\"type\":\"nonsense\"}".as_slice(), 0).unwrap();

    wait_until("the slow log post to land", || {
        harness.logbook().iter().any(|r| r["type"] == json!("slow"))
    });

    let records = harness.logbook();
    let comment = records
        .iter()
        .find(|r| r["type"] == json!("comment"))
        .unwrap();
    assert_eq!(comment["author"], json!("aogaki"));
    assert_eq!(comment["actor"], json!("ui"));
    let slow = records.iter().find(|r| r["type"] == json!("slow")).unwrap();
    assert_eq!(slow["device"], json!("gas"));
    assert_eq!(slow["node"], json!(4));
    assert_eq!(slow["values"]["fmeasure"]["value"], json!(130.5));
    assert_eq!(slow["values"]["fmeasure"]["unit"], json!("mbar"));
    // ts / seq は controller が付ける(SPEC §9.1)。
    assert_eq!(
        slow["seq"].as_u64().unwrap(),
        comment["seq"].as_u64().unwrap() + 1
    );
    assert_eq!(
        slow["ts"].as_str().unwrap().len(),
        29,
        "RFC3339 ミリ秒 + オフセット"
    );

    // 壊れた投稿は捨てるが数える。
    wait_until("the bad log post to be counted", || {
        let (_, body) = get(harness.rest, "/api/status");
        body["log_posts"]["rejected"] == json!(1)
    });
    let (_, body) = get(harness.rest, "/api/status");
    assert_eq!(body["log_posts"]["accepted"], json!(1));

    // since_seq のフィルタ(UI の増分取得)。
    let (status, body) = get(harness.rest, "/api/logbook?since_seq=1");
    assert_eq!(status, 200);
    let seqs: Vec<u64> = body["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["seq"].as_u64().unwrap())
        .collect();
    assert!(seqs.iter().all(|&s| s > 1), "{seqs:?}");

    harness.shutdown();
}

/// SPEC §9.2 v1.31: slowctl が投げた直近値が `run_start` / `run_stop` の `slow` に載る。
/// **投稿が無ければキーごと出ない**(slowctl 不在と「全部 0」を混同しない)。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_run_records_carry_the_latest_slow_values() {
    let harness = harness("run-slow", &[0], true, None).await;

    let context = zmq::Context::new();
    let push = context.socket(zmq::PUSH).unwrap();
    push.set_sndtimeo(2000).unwrap();
    push.connect(&harness.log_pull).unwrap();
    for (param, value, unit) in [("fmeasure", 130.5, "mbar"), ("temperature", 21.5, "degC")] {
        push.send(
            json!({
                "type": "slow", "actor": "slowctl", "device": "gas", "node": 4,
                "event": "VALUE", "values": {param: {"value": value, "unit": unit}}
            })
            .to_string()
            .as_bytes(),
            0,
        )
        .unwrap();
    }
    // 番兵: PUSH -> PULL は同一経路で順序が保たれるので、これがログブックに現れた時点で
    // 手前の VALUE 2 本は処理済み(TODO/121 §B: VALUE はログブックに書かない)。
    push.send(
        json!({
            "type": "slow", "actor": "slowctl", "device": "gas", "node": null,
            "event": "IDENT", "values": {"model": "MKR1000"}
        })
        .to_string()
        .as_bytes(),
        0,
    )
    .unwrap();
    wait_until("the sentinel IDENT to land", || {
        harness
            .logbook()
            .iter()
            .any(|r| r["type"] == json!("slow") && r["event"] == json!("IDENT"))
    });
    let slow_rows: Vec<Value> = harness
        .logbook()
        .into_iter()
        .filter(|r| r["type"] == json!("slow"))
        .collect();
    assert_eq!(slow_rows.len(), 1, "VALUE は書かれない: {slow_rows:?}");
    // 書かなかった VALUE は数える(silent にしない)。
    let (_, body) = get(harness.rest, "/api/status");
    assert_eq!(body["log_posts"]["slow_values_not_logged"], json!(2));
    assert_eq!(body["log_posts"]["accepted"], json!(1));

    let token = harness.acquire("aogaki");
    let (status, body) = post(
        harness.rest,
        "/api/run/start",
        json!({"token": token, "comment": "gas test"}),
    );
    assert_eq!(status, 200, "{body}");

    let records = harness.logbook();
    let run_start = records
        .iter()
        .find(|r| r["type"] == json!("run_start"))
        .unwrap();
    // キーは `"<device>/<node>/<param>"`(SPEC §9.2)。
    assert_eq!(run_start["slow"]["gas/4/fmeasure"]["value"], json!(130.5));
    assert_eq!(run_start["slow"]["gas/4/fmeasure"]["unit"], json!("mbar"));
    assert_eq!(run_start["slow"]["gas/4/temperature"]["value"], json!(21.5));
    assert_eq!(
        run_start["slow"]["gas/4/fmeasure"]["ts"]
            .as_str()
            .unwrap()
            .len(),
        29,
        "SPEC §9.1 と同じ RFC3339 ミリ秒 + オフセット"
    );

    let (status, body) = post(harness.rest, "/api/run/stop", json!({"token": token}));
    assert_eq!(status, 200, "{body}");
    let records = harness.logbook();
    let run_stop = records
        .iter()
        .find(|r| r["type"] == json!("run_stop"))
        .unwrap();
    assert_eq!(run_stop["slow"]["gas/4/fmeasure"]["value"], json!(130.5));

    harness.shutdown();
}

/// slowctl が 1 本も投げていなければ `slow` は省略される(nullable 規律)。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_run_without_any_slow_post_omits_the_slow_field() {
    let harness = harness("run-noslow", &[0], true, None).await;
    let token = harness.acquire("aogaki");
    let (status, body) = post(
        harness.rest,
        "/api/run/start",
        json!({"token": token, "comment": "no slowctl"}),
    );
    assert_eq!(status, 200, "{body}");

    let records = harness.logbook();
    let run_start = records
        .iter()
        .find(|r| r["type"] == json!("run_start"))
        .unwrap();
    assert!(run_start.get("slow").is_none(), "{run_start}");

    harness.shutdown();
}

/// ECC の段階操作(R6)。`configure` は receiver の実 bind ポートで組む。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_ecc_endpoints_drive_the_bridge_step_by_step() {
    let harness = harness("ecc-steps", &[0], true, None).await;
    let token = harness.acquire("aogaki");

    for action in [
        "describe",
        "prepare",
        "configure",
        "start",
        "stop",
        "breakup",
        "reset",
    ] {
        let (status, body) = post(
            harness.rest,
            &format!("/api/ecc/{action}"),
            json!({"token": token}),
        );
        assert_eq!(status, 200, "{action}: {body}");
        assert_eq!(body["ok"], json!(true), "{action}");
    }
    assert_eq!(
        harness.ecc.actions(),
        vec![
            "describe",
            "prepare",
            "configure",
            "start",
            "stop",
            "breakup",
            "reset"
        ]
    );
    // configure は receiver の実 bind ポート(GetStatus 実測)で組まれる。
    let configure = harness.ecc.request("configure").unwrap();
    assert_eq!(
        configure["links"][0]["router_port"],
        json!(harness.router_ports[0])
    );
    assert_eq!(configure["links"][0]["sender"], json!("CoBo[0]"));

    // 知らない action は 400、token 無しは 401。
    let (status, _) = post(harness.rest, "/api/ecc/launch", json!({"token": token}));
    assert_eq!(status, 400);
    let (status, _) = post(harness.rest, "/api/ecc/start", json!({"token": "bogus"}));
    assert_eq!(status, 401);

    harness.shutdown();
}

// ---------------------------------------------------------------------
// ConfigId の 3 相化(SPEC §3.1 / §8.2 / §9.2 v1.13、TODO/042)
// ---------------------------------------------------------------------

/// 3 相に**非対称な** id を持つ設定(実運用例: describe だけハードウェア記述、
/// configure は測定条件 —— TODO/038 レーン A の実測)。相の取り違えを検出する。
fn per_phase_ids() -> ConfigIds {
    ConfigIds {
        describe: "zCobo-ZC706".to_string(),
        prepare: "prep-xcfg".to_string(),
        configure: "pulser".to_string(),
    }
}

/// run シーケンスは describe / prepare / configure に**その相の id** を載せ、
/// logbook `run_start` は `config_id` = configure 相 + `config_ids` に 3 相全部を残す。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn per_phase_config_ids_ride_the_run_sequence_and_the_run_start_record() {
    let harness = harness_with_config_ids("cfgids-run", &[0], true, None, per_phase_ids()).await;
    let token = harness.acquire("aogaki");

    let (status, body) = post(
        harness.rest,
        "/api/run/start",
        json!({"token": token, "comment": "3-phase ids"}),
    );
    assert_eq!(status, 200, "{body}");

    // --- ecc へ渡った id(SPEC §8.2 v1.13)---
    assert_eq!(
        harness.ecc.request("describe").unwrap()["config_id"],
        json!("zCobo-ZC706")
    );
    assert_eq!(
        harness.ecc.request("prepare").unwrap()["config_id"],
        json!("prep-xcfg")
    );
    assert_eq!(
        harness.ecc.request("configure").unwrap()["config_id"],
        json!("pulser")
    );
    // id を使わないアクションには載せない。
    assert_eq!(
        harness.ecc.request("start").unwrap().get("config_id"),
        None,
        "start は config_id を取らない"
    );

    // --- logbook run_start(SPEC §9.2 v1.13)---
    let records = harness.logbook();
    let run_start = records
        .iter()
        .find(|r| r["type"] == json!("run_start"))
        .expect("run_start record");
    assert_eq!(run_start["config_id"], json!("pulser"));
    assert_eq!(
        run_start["config_ids"],
        json!({"describe": "zCobo-ZC706", "prepare": "prep-xcfg", "configure": "pulser"})
    );

    harness.shutdown();
}

/// 段階操作(`/api/ecc/<action>`)も**踏んだ相の id** を渡す。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_ecc_endpoints_send_the_id_of_the_phase_being_stepped() {
    let harness = harness_with_config_ids("cfgids-steps", &[0], true, None, per_phase_ids()).await;
    let token = harness.acquire("aogaki");

    for action in ["describe", "prepare", "configure", "start"] {
        let (status, body) = post(
            harness.rest,
            &format!("/api/ecc/{action}"),
            json!({"token": token}),
        );
        assert_eq!(status, 200, "{action}: {body}");
    }

    assert_eq!(
        harness.ecc.request("describe").unwrap()["config_id"],
        json!("zCobo-ZC706")
    );
    assert_eq!(
        harness.ecc.request("prepare").unwrap()["config_id"],
        json!("prep-xcfg")
    );
    assert_eq!(
        harness.ecc.request("configure").unwrap()["config_id"],
        json!("pulser")
    );
    assert_eq!(harness.ecc.request("start").unwrap().get("config_id"), None);

    harness.shutdown();
}

/// 3 相同値(既存の文字列形設定)なら `config_ids` は**出ない**
/// (省略 ≠ 空値 —— nullable 規律、SPEC §9.2)。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_single_config_id_leaves_no_config_ids_in_the_run_start_record() {
    let harness = harness("cfgids-same", &[0], true, None).await;
    let token = harness.acquire("aogaki");

    let (status, body) = post(
        harness.rest,
        "/api/run/start",
        json!({"token": token, "comment": ""}),
    );
    assert_eq!(status, 200, "{body}");

    let records = harness.logbook();
    let run_start = records
        .iter()
        .find(|r| r["type"] == json!("run_start"))
        .expect("run_start record");
    assert_eq!(run_start["config_id"], json!("integration"));
    assert_eq!(run_start.get("config_ids"), None, "{run_start}");

    harness.shutdown();
}

/// `ui_dir` を設定すると `/` から静的ファイルを配信する(UI は後波 = 枠だけ)。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_ui_directory_is_served_when_configured() {
    let ui_dir = temp_root("ui");
    std::fs::create_dir_all(&ui_dir).unwrap();
    std::fs::write(ui_dir.join("index.html"), "<h1>tpcdaq</h1>").unwrap();

    let harness = harness("ui", &[0], true, Some(ui_dir.clone())).await;
    let (status, body) = get(harness.rest, "/index.html");
    assert_eq!(status, 200);
    assert_eq!(body, Value::String("<h1>tpcdaq</h1>".to_string()));

    // API は静的配信に食われない。
    let (status, _) = get(harness.rest, "/api/status");
    assert_eq!(status, 200);

    let _ = std::fs::remove_dir_all(&ui_dir);
    harness.shutdown();
}

// ---------------------------------------------------------------------
// パラメータ JSON REST(078 レーン B §10、SPEC §3.3)
// ---------------------------------------------------------------------

/// レーン A の合成フィクスチャを `ecc_config_dir` に置いた params 専用ハーネス。
/// `tests/fixtures/params/params_roundtrip.json` の `"template"` は
/// `"configure-mini-fixture.xcfg"` なので、`template_mini.xcfg` を**その名前で**コピーする
/// (078 §9 発注のとおり「一時ディレクトリにコピーして使う」)。
async fn params_harness(tag: &str) -> (Harness, PathBuf) {
    let ecc_config_dir = temp_root(&format!("{tag}-ecc-config"));
    std::fs::create_dir_all(&ecc_config_dir).unwrap();
    let template_src =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/params/template_mini.xcfg");
    std::fs::copy(
        &template_src,
        ecc_config_dir.join("configure-mini-fixture.xcfg"),
    )
    .unwrap();

    let harness = harness_full(
        tag,
        &[0],
        true,
        None,
        ConfigIds::same("integration"),
        Some(ecc_config_dir.clone()),
    )
    .await;
    (harness, ecc_config_dir)
}

/// `tests/fixtures/params/params_roundtrip.json`(078 §5-1 の往復オラクル — レーン A の
/// テンプレート現行値そのまま、`exceptions: []`)を `Value` として読む。
fn roundtrip_params_json() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/params/params_roundtrip.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap()
}

/// ディレクトリのファイル名一覧(ソート済み)。「dry run が書かなかった」ことの証拠に使う。
fn dir_listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    names
}

/// ① GET 404(未 PUT)→ PUT(有効 JSON)→ 2 ファイル生成 + GET 200(exceptions 件数含む)。
/// 監査 `params/put` の記録も確認する。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn params_get_404_then_put_creates_the_xcfg_pair_and_get_returns_the_same_shape() {
    let (harness, ecc_config_dir) = params_harness("params-roundtrip").await;

    let (status, body) = get(harness.rest, "/api/params");
    assert_eq!(status, 404, "{body}");

    let token = harness.acquire("aogaki");
    let (status, put_body) = put(
        harness.rest,
        "/api/params",
        json!({"token": token, "params": roundtrip_params_json()}),
    );
    assert_eq!(status, 200, "{put_body}");
    let id = put_body["id"].as_str().expect("id string").to_string();
    // stem = template_stem("configure-mini-fixture.xcfg") = "mini-fixture"(078 §3-3)。
    assert!(id.starts_with("mini-fixture__"), "{id}");
    assert_eq!(put_body["sha256"].as_str().unwrap().len(), 64, "{put_body}");
    assert_eq!(put_body["exceptions"], json!(0), "{put_body}");
    assert_eq!(
        put_body["params"]["template"],
        json!("configure-mini-fixture.xcfg")
    );

    let configure_path = ecc_config_dir.join(format!("configure-{id}.xcfg"));
    let prepare_path = ecc_config_dir.join(format!("prepare-{id}.xcfg"));
    assert_eq!(
        put_body["files"],
        json!({
            "configure": configure_path.display().to_string(),
            "prepare": prepare_path.display().to_string(),
        })
    );
    assert!(configure_path.exists(), "configure xcfg が生成されていない");
    assert!(prepare_path.exists(), "prepare xcfg が生成されていない");
    // レーン A の往復規則: knobs がテンプレート現行値のままなら生成 xcfg はテンプレートと
    // バイト一致(078 §5-1)。
    let template_text = std::fs::read_to_string(template_src(&ecc_config_dir)).unwrap();
    assert_eq!(
        std::fs::read_to_string(&configure_path).unwrap(),
        template_text
    );
    assert_eq!(
        std::fs::read_to_string(&prepare_path).unwrap(),
        template_text
    );

    let (status, get_body) = get(harness.rest, "/api/params");
    assert_eq!(status, 200, "{get_body}");
    assert_eq!(get_body, put_body, "GET は PUT と同じ形を返す(078 §10)");

    // 113 §2-3: current.json は一時ファイル + rename で差し替える —— 一時ファイルは残らない。
    assert_eq!(
        dir_listing(&harness.output_root.join("params")),
        vec!["current.json".to_string(), format!("{id}.json")]
    );

    let records = harness.logbook();
    let audit = records
        .iter()
        .find(|r| r["action"] == json!("params/put"))
        .expect("params/put audit record");
    assert_eq!(audit["ok"], json!(true));
    assert_eq!(audit["params"], json!({"id": id, "exceptions": 0}));

    let _ = std::fs::remove_dir_all(&ecc_config_dir);
    harness.shutdown();
}

fn template_src(ecc_config_dir: &Path) -> PathBuf {
    ecc_config_dir.join("configure-mini-fixture.xcfg")
}

/// ② スキーマ違反は 400 + `ParamsError` の文言(範囲外 / スコープより細かい例外 / 未知ノブ)。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn params_put_rejects_schema_violations_with_400_and_the_params_error_text() {
    let (harness, ecc_config_dir) = params_harness("params-400").await;
    let token = harness.acquire("aogaki");

    // 範囲外(globalThreshold は 0..7 — schema.rs の KNOBS)。
    let mut out_of_range = roundtrip_params_json();
    out_of_range["knobs"]["globalThreshold"] = json!(99);
    let (status, body) = put(
        harness.rest,
        "/api/params",
        json!({"token": token, "params": out_of_range}),
    );
    assert_eq!(status, 400, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("globalThreshold"),
        "{body}"
    );

    // スコープより細かい例外(zeroSuppression.enabled は CoBo スコープ = 深さ 0、
    // どんな `at` も細かすぎる — 078 §2.1)。
    let mut too_fine = roundtrip_params_json();
    too_fine["exceptions"] = json!([
        {"knob": "zeroSuppression.enabled", "at": {"asad": 0}, "value": true}
    ]);
    let (status, body) = put(
        harness.rest,
        "/api/params",
        json!({"token": token, "params": too_fine}),
    );
    assert_eq!(status, 400, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("zeroSuppression.enabled"),
        "{body}"
    );

    // 未知のノブ。
    let mut unknown_knob = roundtrip_params_json();
    unknown_knob["exceptions"] = json!([
        {"knob": "bogusKnob", "at": {"asad": 0}, "value": 1}
    ]);
    let (status, body) = put(
        harness.rest,
        "/api/params",
        json!({"token": token, "params": unknown_knob}),
    );
    assert_eq!(status, 400, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("bogusKnob"),
        "{body}"
    );

    // 3 件とも書かない(400 は失敗であって半端な生成物を残さない)。
    assert_eq!(
        dir_listing(&ecc_config_dir),
        vec!["configure-mini-fixture.xcfg"]
    );

    let records = harness.logbook();
    let failed = records
        .iter()
        .filter(|r| r["action"] == json!("params/put") && r["ok"] == json!(false))
        .count();
    assert_eq!(failed, 3, "{records:?}");

    let _ = std::fs::remove_dir_all(&ecc_config_dir);
    harness.shutdown();
}

/// ③ 同じ JSON を再 PUT してもファイルのタイムスタンプは動かない(078 §3-3「同じ JSON →
/// 同じファイル」)。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn params_put_is_idempotent_and_leaves_existing_file_timestamps_alone() {
    let (harness, ecc_config_dir) = params_harness("params-idempotent").await;
    let token = harness.acquire("aogaki");
    let body = json!({"token": token, "params": roundtrip_params_json()});

    let (status, first) = put(harness.rest, "/api/params", body.clone());
    assert_eq!(status, 200, "{first}");
    let id = first["id"].as_str().unwrap().to_string();
    let configure_path = ecc_config_dir.join(format!("configure-{id}.xcfg"));
    let prepare_path = ecc_config_dir.join(format!("prepare-{id}.xcfg"));
    let mtime = |p: &PathBuf| {
        std::fs::metadata(p)
            .unwrap()
            .modified()
            .expect("mtime is supported")
    };
    let configure_mtime_1 = mtime(&configure_path);
    let prepare_mtime_1 = mtime(&prepare_path);

    // ファイルシステムの mtime 分解能(秒単位のことがある)に負けないよう一呼吸置く。
    std::thread::sleep(Duration::from_millis(1100));

    let (status, second) = put(harness.rest, "/api/params", body);
    assert_eq!(status, 200, "{second}");
    assert_eq!(second["id"], first["id"]);
    assert_eq!(
        mtime(&configure_path),
        configure_mtime_1,
        "configure が再書き込みされた(078 §3-3 違反)"
    );
    assert_eq!(
        mtime(&prepare_path),
        prepare_mtime_1,
        "prepare が再書き込みされた(078 §3-3 違反)"
    );

    let _ = std::fs::remove_dir_all(&ecc_config_dir);
    harness.shutdown();
}

/// ④ `ecc_config_dir` 未設定なら params 系 REST(GET/PUT/render)はすべて 503。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn params_rest_returns_503_when_ecc_config_dir_is_not_configured() {
    // `harness()` は `ecc_config_dir: None`(harness_full の既定)。
    let harness = harness("params-no-eccdir", &[0], true, None).await;

    let (status, body) = get(harness.rest, "/api/params");
    assert_eq!(status, 503, "{body}");
    assert_eq!(body["error"], json!("ecc_config_dir is not configured"));

    let (status, body) = get(harness.rest, "/api/params/render");
    assert_eq!(status, 503, "{body}");

    let token = harness.acquire("aogaki");
    let (status, body) = put(
        harness.rest,
        "/api/params",
        json!({"token": token, "params": roundtrip_params_json()}),
    );
    assert_eq!(status, 503, "{body}");

    harness.shutdown();
}

/// ⑤ current.json があれば run/start の prepare/configure の config_id が `<id>`、
/// describe は TOML の値のまま(078 §10-3)。logbook `run_start.params` と
/// `audit run/start` の `params_id` にも同じ `<id>` が載る。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn params_id_drives_prepare_and_configure_config_id_at_run_start() {
    let (harness, ecc_config_dir) = params_harness("params-run-start").await;
    let token = harness.acquire("aogaki");
    let (status, put_body) = put(
        harness.rest,
        "/api/params",
        json!({"token": token, "params": roundtrip_params_json()}),
    );
    assert_eq!(status, 200, "{put_body}");
    let id = put_body["id"].as_str().unwrap().to_string();

    let (status, body) = post(
        harness.rest,
        "/api/run/start",
        json!({"token": token, "comment": "params wired"}),
    );
    assert_eq!(status, 200, "{body}");

    // describe は TOML の config_id("integration")、prepare/configure は params の id。
    assert_eq!(
        harness.ecc.request("describe").unwrap()["config_id"],
        json!("integration")
    );
    assert_eq!(
        harness.ecc.request("prepare").unwrap()["config_id"],
        json!(id)
    );
    assert_eq!(
        harness.ecc.request("configure").unwrap()["config_id"],
        json!(id)
    );

    let records = harness.logbook();
    let run_start = records
        .iter()
        .find(|r| r["type"] == json!("run_start"))
        .expect("run_start record");
    assert_eq!(run_start["params"]["id"], json!(id));
    assert_eq!(run_start["params"]["exceptions"], json!(0));
    assert_eq!(run_start["params"]["sha256"], put_body["sha256"]);
    // 3 相が非同値(describe != prepare/configure)なので §9.2 v1.13 の分岐で
    // `config_id` は configure 相、`config_ids` に 3 相全部が載る。
    assert_eq!(run_start["config_id"], json!(id));
    assert_eq!(
        run_start["config_ids"],
        json!({"describe": "integration", "prepare": id, "configure": id})
    );

    let run_start_audit = records
        .iter()
        .rev()
        .find(|r| r["action"] == json!("run/start") && r["ok"] == json!(true))
        .expect("run/start audit record");
    assert_eq!(run_start_audit["params"]["params_id"], json!(id));

    let (status, body) = post(harness.rest, "/api/run/stop", json!({"token": token}));
    assert_eq!(status, 200, "{body}");

    let _ = std::fs::remove_dir_all(&ecc_config_dir);
    harness.shutdown();
}

/// ⑥ run 実行中(phase ≠ Idle)の PUT は 409(次 run から効くという半端を作らない)。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn params_put_is_refused_with_409_while_a_run_is_active() {
    let (harness, ecc_config_dir) = params_harness("params-running").await;
    let token = harness.acquire("aogaki");
    let (status, body) = post(
        harness.rest,
        "/api/run/start",
        json!({"token": token, "comment": ""}),
    );
    assert_eq!(status, 200, "{body}");

    let (status, body) = put(
        harness.rest,
        "/api/params",
        json!({"token": token, "params": roundtrip_params_json()}),
    );
    assert_eq!(status, 409, "{body}");

    let (status, body) = post(harness.rest, "/api/run/stop", json!({"token": token}));
    assert_eq!(status, 200, "{body}");

    let _ = std::fs::remove_dir_all(&ecc_config_dir);
    harness.shutdown();
}

/// ⑦ dry-run render(`GET /api/params/render`)はファイルを 1 つも書かない
/// (UI の diff 表示用、078 §10)。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn params_render_dry_run_does_not_write_any_file() {
    let (harness, ecc_config_dir) = params_harness("params-dry-run").await;
    let token = harness.acquire("aogaki");
    let (status, put_body) = put(
        harness.rest,
        "/api/params",
        json!({"token": token, "params": roundtrip_params_json()}),
    );
    assert_eq!(status, 200, "{put_body}");

    let before = dir_listing(&ecc_config_dir);

    let (status, body) = get(harness.rest, "/api/params/render");
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], put_body["id"]);
    assert_eq!(body["exceptions"], json!(0));
    assert_eq!(
        body["configure_xcfg"],
        Value::String(std::fs::read_to_string(template_src(&ecc_config_dir)).unwrap())
    );

    // テンプレート省略時は current.json の template(= `configure-mini-fixture.xcfg`)を使う。
    let (status, explicit_body) = get(
        harness.rest,
        "/api/params/render?template=configure-mini-fixture.xcfg",
    );
    assert_eq!(status, 200, "{explicit_body}");
    assert_eq!(explicit_body["configure_xcfg"], body["configure_xcfg"]);

    let after = dir_listing(&ecc_config_dir);
    assert_eq!(before, after, "dry run がディレクトリの中身を変えた");

    let _ = std::fs::remove_dir_all(&ecc_config_dir);
    harness.shutdown();
}

/// 「作るもの」④: `GET /api/status` の `params`(PUT 前は null、PUT 後は `{id, exceptions}`)。
/// status バーの `params <id> · exceptions N` の元(UI はレーン C)。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn status_reports_the_current_params_id_and_exceptions_or_null() {
    let (harness, ecc_config_dir) = params_harness("params-status").await;

    let (status, body) = get(harness.rest, "/api/status");
    assert_eq!(status, 200);
    assert_eq!(body["params"], Value::Null, "{body}");

    let token = harness.acquire("aogaki");
    let (status, put_body) = put(
        harness.rest,
        "/api/params",
        json!({"token": token, "params": roundtrip_params_json()}),
    );
    assert_eq!(status, 200, "{put_body}");

    let (status, body) = get(harness.rest, "/api/status");
    assert_eq!(status, 200);
    assert_eq!(
        body["params"],
        json!({"id": put_body["id"], "exceptions": 0})
    );

    let _ = std::fs::remove_dir_all(&ecc_config_dir);
    harness.shutdown();
}

// ---------------------------------------------------------------------
// テンプレートからの初期値(TODO/111 §5.0)
// ---------------------------------------------------------------------

/// `GET /api/params/templates` = `ecc_config_dir` 直下の `configure-*.xcfg` を辞書順。
/// prepare-* / describe-* / 生成物 `*__<hash8>.xcfg` / 他の拡張子 / ディレクトリは出さない。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn params_templates_lists_the_configure_templates_but_not_generated_or_other_phases() {
    let (harness, ecc_config_dir) = params_harness("params-templates").await;
    for name in [
        "configure-elinp.xcfg",
        "prepare-elinp.xcfg",
        "describe-elinp.xcfg",
        "configure-elinp__0123abcd.xcfg",
        "configure-notes.txt",
        // `__` があっても 8 桁 16 進でなければ生成物ではない(テンプレート名の一部)。
        "configure-a__b.xcfg",
    ] {
        std::fs::write(ecc_config_dir.join(name), "<Setup/>").unwrap();
    }
    std::fs::create_dir_all(ecc_config_dir.join("configure-dir.xcfg")).unwrap();

    let (status, body) = get(harness.rest, "/api/params/templates");

    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({"templates": [
            "configure-a__b.xcfg",
            "configure-elinp.xcfg",
            "configure-mini-fixture.xcfg",
        ]})
    );

    let _ = std::fs::remove_dir_all(&ecc_config_dir);
    harness.shutdown();
}

/// `GET /api/params/defaults?template=` の 200 = 抽出結果(§5.0 の形)。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn params_defaults_returns_the_knobs_extracted_from_the_template() {
    let (harness, ecc_config_dir) = params_harness("params-defaults").await;

    let (status, body) = get(
        harness.rest,
        "/api/params/defaults?template=configure-mini-fixture.xcfg",
    );

    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({
            "template": "configure-mini-fixture.xcfg",
            "knobs": roundtrip_params_json()["knobs"],
            "exceptions": [],
            "unset": [],
            "unsupported": [],
        })
    );

    let _ = std::fs::remove_dir_all(&ecc_config_dir);
    harness.shutdown();
}

/// 400(名前の形 / ディレクトリの外 / XML として読めない)・404(無い)。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn params_defaults_rejects_bad_names_with_400_and_missing_files_with_404() {
    let (harness, ecc_config_dir) = params_harness("params-defaults-errors").await;
    std::fs::write(
        ecc_config_dir.join("configure-broken.xcfg"),
        "<Setup><Node id=\"CoBo\"></Setup>",
    )
    .unwrap();

    for query in [
        "",
        "?template=",
        "?template=../x.xcfg",
        "?template=..",
        "?template=a/b.xcfg",
        "?template=a%5Cb.xcfg",
        "?template=configure-mini-fixture",
        "?template=configure-broken.xcfg",
    ] {
        let (status, body) = get(harness.rest, &format!("/api/params/defaults{query}"));
        assert_eq!(status, 400, "{query}: {body}");
        assert!(body["error"].is_string(), "{query}: {body}");
    }

    let (status, body) = get(
        harness.rest,
        "/api/params/defaults?template=configure-nope.xcfg",
    );
    assert_eq!(status, 404, "{body}");
    assert!(body["error"].is_string(), "{body}");

    // シンボリックリンクで外へ出るもの = canonicalize 後に ecc_config_dir の外 → 400。
    #[cfg(unix)]
    {
        let outside = temp_root("params-defaults-outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::copy(
            template_src(&ecc_config_dir),
            outside.join("configure-outside.xcfg"),
        )
        .unwrap();
        std::os::unix::fs::symlink(
            outside.join("configure-outside.xcfg"),
            ecc_config_dir.join("configure-link.xcfg"),
        )
        .unwrap();
        let (status, body) = get(
            harness.rest,
            "/api/params/defaults?template=configure-link.xcfg",
        );
        assert_eq!(status, 400, "{body}");
        let _ = std::fs::remove_dir_all(&outside);
    }

    let _ = std::fs::remove_dir_all(&ecc_config_dir);
    harness.shutdown();
}

/// `ecc_config_dir` 未設定 → templates / defaults とも 503(既存と同文言)。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn params_templates_and_defaults_return_503_without_ecc_config_dir() {
    let harness = harness("params-defaults-no-eccdir", &[0], true, None).await;

    for path in [
        "/api/params/templates",
        "/api/params/defaults?template=configure-mini-fixture.xcfg",
    ] {
        let (status, body) = get(harness.rest, path);
        assert_eq!(status, 503, "{path}: {body}");
        assert_eq!(body["error"], json!("ecc_config_dir is not configured"));
    }

    harness.shutdown();
}

// ---------------------------------------------------------------------
// Apply & restart の遷移(TODO/112 §2 レーン A)
// ---------------------------------------------------------------------

/// triggerDelay だけ変えた params JSON(112: 周ごとに id を変えるための 1 ノブ)。
fn params_with_trigger_delay(delay: u64) -> Value {
    let mut params = roundtrip_params_json();
    params["knobs"]["triggerDelay"] = json!(delay);
    params
}

/// ログブックの audit のうち、指定の action だけを `(action, ok)` で受けた順に。
fn audit_trail(records: &[Value], actions: &[&str]) -> Vec<(String, bool)> {
    records
        .iter()
        .filter(|r| r["type"] == json!("audit"))
        .filter(|r| actions.iter().any(|a| r["action"] == json!(a)))
        .map(|r| {
            (
                r["action"].as_str().unwrap().to_string(),
                r["ok"].as_bool().unwrap(),
            )
        })
        .collect()
}

/// 112 §2-1 (a)(b): `run/stop` の 200 を受けた**直後に待ち時間ゼロで** `PUT /api/params`
/// (triggerDelay を 1 つ変える)→ `run/start` を 5 周。
///
/// 各周: stop 200(`forced_eos=false` / `eos_closed=true` — このハーネスは EOS が最初から
/// 流れ切っている形)→ PUT 200(新しい id)→ start 200(run 番号 +1)→ ECC(fake)が受けた
/// prepare / configure の `config_id` も logbook `run_start.config_id` も新しい id。
/// 最後に audit の順序(run/stop → params/put → run/start)と run 台帳・ECC 列・
/// コンポーネントのコマンド列を全周まとめて照合する。周ごとの所要 ms は stderr に出す
/// (`--nocapture` で読む)。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn back_to_back_stop_put_start_switches_the_params_id_every_round_for_five_rounds() {
    let (harness, ecc_config_dir) = params_harness("apply-restart").await;
    let token = harness.acquire("aogaki");

    // 周 0: テンプレート現行値(triggerDelay 1748)で run 1 を始める。
    let (status, first_put) = put(
        harness.rest,
        "/api/params",
        json!({"token": token, "params": roundtrip_params_json()}),
    );
    assert_eq!(status, 200, "{first_put}");
    let mut ids = vec![first_put["id"].as_str().unwrap().to_string()];
    let (status, body) = post(
        harness.rest,
        "/api/run/start",
        json!({"token": token, "comment": "apply-restart 0"}),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["run"], json!(1));

    // 毎周違う triggerDelay(単調でない非対称な値 = 周の取り違えが id に出る)。
    let delays: [u64; 5] = [1700, 1832, 1650, 1999, 1777];
    for (round, delay) in delays.iter().enumerate() {
        let stopping = round as u64 + 1; // この周で止める run(1..=5)

        let began = Instant::now();
        let (status, stop) = post(harness.rest, "/api/run/stop", json!({"token": token}));
        let stop_ms = began.elapsed().as_millis();
        assert_eq!(status, 200, "round {round}: {stop}");
        assert_eq!(stop["run"], json!(stopping), "round {round}: {stop}");
        assert_eq!(stop["ok"], json!(true), "round {round}: {stop}");
        assert_eq!(stop["forced_eos"], json!(false), "round {round}: {stop}");
        assert_eq!(stop["eos_closed"], json!(true), "round {round}: {stop}");
        assert_eq!(stop["notes"], json!([]), "round {round}: {stop}");

        // 待ち時間ゼロ(sleep も status ポーリングも挟まない)。
        let began = Instant::now();
        let (status, put_body) = put(
            harness.rest,
            "/api/params",
            json!({"token": token, "params": params_with_trigger_delay(*delay)}),
        );
        let put_ms = began.elapsed().as_millis();
        assert_eq!(status, 200, "round {round}: {put_body}");
        let id = put_body["id"].as_str().unwrap().to_string();
        assert!(
            !ids.contains(&id),
            "round {round}: triggerDelay {delay} の id {id} が既出 {ids:?}"
        );
        ids.push(id.clone());

        let began = Instant::now();
        let (status, start) = post(
            harness.rest,
            "/api/run/start",
            json!({"token": token, "comment": format!("apply-restart {}", round + 1)}),
        );
        let start_ms = began.elapsed().as_millis();
        assert_eq!(status, 200, "round {round}: {start}");
        assert_eq!(start["run"], json!(stopping + 1), "round {round}: {start}");

        // ECC(fake)が**最後に**受けた prepare / configure は新しい id(describe は TOML のまま)。
        let last = |action: &str| harness.ecc.requests_for(action).last().cloned().unwrap();
        assert_eq!(last("prepare")["config_id"], json!(id), "round {round}");
        assert_eq!(last("configure")["config_id"], json!(id), "round {round}");
        assert_eq!(last("describe")["config_id"], json!("integration"));

        let records = harness.logbook();
        let run_start = records
            .iter()
            .rev()
            .find(|r| r["type"] == json!("run_start"))
            .unwrap();
        assert_eq!(run_start["run"], json!(stopping + 1), "round {round}");
        assert_eq!(run_start["config_id"], json!(id), "round {round}");
        assert_eq!(run_start["params"]["id"], json!(id), "round {round}");

        eprintln!(
            "112 back-to-back round {}: stop {stop_ms} ms / put {put_ms} ms / start {start_ms} ms \
             / total {} ms (run {stopping} -> {}, triggerDelay {delay}, id {id})",
            round + 1,
            stop_ms + put_ms + start_ms,
            stopping + 1,
        );
    }

    let (status, body) = post(harness.rest, "/api/run/stop", json!({"token": token}));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["run"], json!(6));

    // --- 監査の順序: PUT → start → (stop → PUT → start)×5 → stop、全部 ok ---
    let records = harness.logbook();
    let mut expected = vec![
        ("params/put".to_string(), true),
        ("run/start".to_string(), true),
    ];
    for _ in 0..5 {
        for action in ["run/stop", "params/put", "run/start"] {
            expected.push((action.to_string(), true));
        }
    }
    expected.push(("run/stop".to_string(), true));
    assert_eq!(
        audit_trail(&records, &["run/stop", "params/put", "run/start"]),
        expected
    );

    // --- run 台帳: 番号は 1..=6 で飛ばず、run_start の id は PUT の順そのまま ---
    let starts: Vec<&Value> = records
        .iter()
        .filter(|r| r["type"] == json!("run_start"))
        .collect();
    assert_eq!(
        starts
            .iter()
            .map(|r| r["run"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5, 6]
    );
    assert_eq!(
        starts
            .iter()
            .map(|r| r["config_id"].as_str().unwrap().to_string())
            .collect::<Vec<_>>(),
        ids
    );
    let stops: Vec<&Value> = records
        .iter()
        .filter(|r| r["type"] == json!("run_stop"))
        .collect();
    assert_eq!(
        stops
            .iter()
            .map(|r| r["run"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5, 6]
    );
    for run_stop in &stops {
        assert_eq!(run_stop["ok"], json!(true), "{run_stop}");
        assert_eq!(run_stop["forced_eos"], json!(false), "{run_stop}");
        assert_eq!(run_stop["eos_closed"], json!(true), "{run_stop}");
    }

    // --- ECC: 1 本目は Off から、2 本目以降は stop の Ready から Idle へ歩き戻す ---
    let mut expected_ecc: Vec<&str> = vec!["describe", "prepare", "configure", "start"];
    for _ in 0..5 {
        expected_ecc.extend([
            "stop",
            "breakup",
            "reset",
            "reset",
            "describe",
            "prepare",
            "configure",
            "start",
        ]);
    }
    expected_ecc.push("stop");
    assert_eq!(harness.ecc.actions(), expected_ecc);

    // --- コンポーネント: 6 周とも Configure → Arm → Start → Stop → Reset を 1 回ずつ ---
    let one_run = ["Configure", "Arm", "Start", "Stop", "Reset"];
    let six_runs: Vec<String> = (0..6)
        .flat_map(|_| one_run.iter().map(|c| c.to_string()))
        .collect();
    for (name, recorder) in [
        ("graw-writer", &harness.graw_writer),
        ("decoder", &harness.decoder),
        ("receiver0", &harness.receivers[0]),
    ] {
        assert_eq!(recorder.commands(), six_runs, "{name}");
    }

    // --- 生成物: 6 つの id ぶんの configure/prepare(+ テンプレート)---
    let listing = dir_listing(&ecc_config_dir);
    for id in &ids {
        assert!(
            listing.contains(&format!("configure-{id}.xcfg")),
            "{listing:?}"
        );
        assert!(
            listing.contains(&format!("prepare-{id}.xcfg")),
            "{listing:?}"
        );
    }
    assert_eq!(listing.len(), 1 + 2 * ids.len(), "{listing:?}");

    let _ = std::fs::remove_dir_all(&ecc_config_dir);
    harness.shutdown();
}

/// 112 §2-1 (c): stop の直後に**別の operator が acquire で token を奪った**ら、元の token の
/// PUT も start も 401 で、本文の `error` がそのまま UI に出せる文言であること。
/// 奪った側は普通に PUT → start できる(遷移が固まっていない)。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_token_taken_between_stop_and_put_makes_put_and_start_401_with_the_reason() {
    let (harness, ecc_config_dir) = params_harness("apply-restart-401").await;
    let token = harness.acquire("aogaki");
    let (status, body) = put(
        harness.rest,
        "/api/params",
        json!({"token": token, "params": roundtrip_params_json()}),
    );
    assert_eq!(status, 200, "{body}");
    let (status, body) = post(
        harness.rest,
        "/api/run/start",
        json!({"token": token, "comment": ""}),
    );
    assert_eq!(status, 200, "{body}");

    let (status, body) = post(harness.rest, "/api/run/stop", json!({"token": token}));
    assert_eq!(status, 200, "{body}");
    let student = harness.acquire("student");

    let (status, body) = put(
        harness.rest,
        "/api/params",
        json!({"token": token, "params": params_with_trigger_delay(1700)}),
    );
    assert_eq!(status, 401, "{body}");
    assert_eq!(body, json!({"error": "stale or unknown token"}));

    let (status, body) = post(
        harness.rest,
        "/api/run/start",
        json!({"token": token, "comment": "should not start"}),
    );
    assert_eq!(status, 401, "{body}");
    assert_eq!(body, json!({"error": "stale or unknown token"}));

    // 何も動いていない: 相は Idle、操作者は奪った側、run_start は 1 本のまま。
    let (_, status_body) = get(harness.rest, "/api/status");
    assert_eq!(status_body["phase"], json!("Idle"));
    assert_eq!(status_body["operator"], json!("student"));
    let records = harness.logbook();
    assert_eq!(
        records
            .iter()
            .filter(|r| r["type"] == json!("run_start"))
            .count(),
        1
    );
    // 拒否も監査に残る(silent にしない)。
    let rejected: Vec<&Value> = records
        .iter()
        .filter(|r| r["type"] == json!("audit") && r["operator"] == json!("<unauthorized>"))
        .collect();
    assert_eq!(
        rejected
            .iter()
            .map(|r| r["action"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["params/put", "run/start"]
    );
    for r in &rejected {
        assert_eq!(r["error"], json!("stale or unknown token"), "{r}");
    }

    // 奪った側はそのまま PUT → start できる。
    let (status, body) = put(
        harness.rest,
        "/api/params",
        json!({"token": student, "params": params_with_trigger_delay(1700)}),
    );
    assert_eq!(status, 200, "{body}");
    let (status, body) = post(
        harness.rest,
        "/api/run/start",
        json!({"token": student, "comment": "taken over"}),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["run"], json!(2));
    let (status, body) = post(harness.rest, "/api/run/stop", json!({"token": student}));
    assert_eq!(status, 200, "{body}");

    let _ = std::fs::remove_dir_all(&ecc_config_dir);
    harness.shutdown();
}

/// 112 §2-1 (d): stop の 200 を受けた瞬間の `GET /api/status` は必ず `phase: Idle`
/// (タイミング穴 ① の直接検証)。start → stop → status を 100 回、一度も Idle 以外を見ない。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_status_right_after_a_stop_reply_is_idle_in_a_hundred_cycles() {
    let harness = harness("stop-then-status", &[0], true, None).await;
    let token = harness.acquire("aogaki");

    const CYCLES: usize = 100;
    let mut not_idle = Vec::new();
    let mut cycle_ms = Vec::new();
    for cycle in 0..CYCLES {
        let began = Instant::now();
        let (status, body) = post(
            harness.rest,
            "/api/run/start",
            json!({"token": token, "comment": ""}),
        );
        assert_eq!(status, 200, "cycle {cycle}: {body}");
        let (status, body) = post(harness.rest, "/api/run/stop", json!({"token": token}));
        assert_eq!(status, 200, "cycle {cycle}: {body}");
        // 畳む段(Stop / Reset)で何か落ちていれば notes に出る。ここで止めないと、
        // 次周の start が「Configure refused in state Configured」で落ちて原因が見えない。
        assert_eq!(body["notes"], json!([]), "cycle {cycle}: {body}");
        let (_, status_body) = get(harness.rest, "/api/status");
        cycle_ms.push(began.elapsed().as_millis());
        if status_body["phase"] != json!("Idle") {
            not_idle.push((cycle, status_body["phase"].clone()));
        }
    }
    assert!(
        not_idle.is_empty(),
        "stop の 200 直後に Idle 以外: {not_idle:?}"
    );
    cycle_ms.sort_unstable();
    eprintln!(
        "112 stop-then-status: {CYCLES} cycles, start+stop+status ms min {} / median {} / max {}",
        cycle_ms[0],
        cycle_ms[CYCLES / 2],
        cycle_ms[CYCLES - 1],
    );

    harness.shutdown();
}

// ---------------------------------------------------------------------
// Apply & restart の実データ版(TODO/112 §2-1 (b): 本物の receiver / decoder / graw-writer)
// ---------------------------------------------------------------------

/// decoder の下流(root-sink の席)に座る吸い込み。PULL を握って全部読み、
/// `EndOfStream` の run 番号だけを受けた順に記録する(= root-sink が run を閉じる合図の回数)。
struct SinkDrain {
    endpoint: String,
    eos_runs: Arc<Mutex<Vec<u32>>>,
    data_batches: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl SinkDrain {
    fn spawn() -> Self {
        let context = zmq::Context::new();
        let socket = context.socket(zmq::PULL).unwrap();
        socket.set_rcvtimeo(100).unwrap();
        socket.bind("tcp://127.0.0.1:*").unwrap();
        let endpoint = socket.get_last_endpoint().unwrap().unwrap();
        let eos_runs = Arc::new(Mutex::new(Vec::new()));
        let data_batches = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));

        let thread_eos = Arc::clone(&eos_runs);
        let thread_batches = Arc::clone(&data_batches);
        let thread_stop = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                let Ok(bytes) = socket.recv_bytes(0) else {
                    continue; // EAGAIN = 100 ms の目覚まし
                };
                let message = tpcdaq::msg::Message::<tpcdaq::msg::Fragments>::from_msgpack(&bytes)
                    .expect("decoder output is a msgpack Message<Fragments>");
                match message {
                    tpcdaq::msg::Message::EndOfStream { run_number, .. } => {
                        thread_eos.lock().unwrap().push(run_number)
                    }
                    tpcdaq::msg::Message::Data(_) => {
                        thread_batches.fetch_add(1, Ordering::Relaxed);
                    }
                    tpcdaq::msg::Message::Heartbeat { .. } => {}
                }
            }
        });
        Self {
            endpoint,
            eos_runs,
            data_batches,
            stop,
            handle: Some(handle),
        }
    }

    fn eos_runs(&self) -> Vec<u32> {
        self.eos_runs.lock().unwrap().clone()
    }
}

impl Drop for SinkDrain {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// 本物の graw-writer → decoder → receiver + fake ECC + controller。receiver のデータポートは
/// **固定**(production の `0.0.0.0:46005` と同じく毎 run 同じポートへ bind し直す)。
struct RealChain {
    rest: SocketAddr,
    ecc: FakeEcc,
    output_root: PathBuf,
    data_port: u16,
    sink: SinkDrain,
    components: Vec<common::Component>,
    /// controller の起動パラメタ(114: [`RealChain::restart_controller`] が同じものを使う)。
    params: ControllerParams,
    /// **controller だけ**を止める合図(コンポーネントはそれぞれ自分の合図を持つ)。
    shutdown: broadcast::Sender<()>,
}

async fn real_chain(tag: &str) -> (RealChain, PathBuf) {
    let ecc_config_dir = temp_root(&format!("{tag}-ecc-config"));
    std::fs::create_dir_all(&ecc_config_dir).unwrap();
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/params/template_mini.xcfg"),
        ecc_config_dir.join("configure-mini-fixture.xcfg"),
    )
    .unwrap();

    let output_root = temp_root(tag);
    std::fs::create_dir_all(&output_root).unwrap();
    let sink = SinkDrain::spawn();
    let data_port = common::free_port();
    let components = common::spawn_components_listening(
        &output_root,
        &sink.endpoint,
        &format!("127.0.0.1:{data_port}"),
    )
    .await;
    let endpoint = |name: &str| common::endpoint_of(&components, name);
    let endpoints = vec![
        ComponentEndpoint {
            name: "graw-writer".to_string(),
            endpoint: endpoint("graw-writer"),
            kind: ComponentKind::GrawWriter,
        },
        ComponentEndpoint {
            name: "decoder".to_string(),
            endpoint: endpoint("decoder"),
            kind: ComponentKind::Decoder,
        },
        ComponentEndpoint {
            name: "receiver0".to_string(),
            endpoint: endpoint("receiver0"),
            kind: ComponentKind::Receiver { cobo_id: 0 },
        },
    ];
    let cobos = vec![CoboSpec {
        id: 0,
        listen: format!("127.0.0.1:{data_port}"),
        data_sender_id: "CoBo[0]".to_string(),
    }];

    let (shutdown, _) = broadcast::channel(1);
    let (BoundEndpoints { rest, .. }, ecc, params) = spawn_controller(
        &shutdown,
        &output_root,
        cobos,
        endpoints,
        None,
        ConfigIds::same("integration"),
        Some(ecc_config_dir.clone()),
    )
    .await;

    let chain = RealChain {
        rest,
        ecc,
        output_root,
        data_port,
        sink,
        components,
        params,
        shutdown,
    };
    (chain, ecc_config_dir)
}

impl RealChain {
    /// **controller だけ**を作り直す(114: run 中に controller が落ちて再起動した姿)。
    ///
    /// 古い controller を止める —— controller は止まるときコンポーネントにも ECC にも何も
    /// 送らないので、kill -9 と同じく**全員 Running のまま**残る。新しい controller は同じ
    /// params(同じ output_root = ログブックと採番を引き継ぐ、同じ fake ECC)で起動し、相は Idle。
    async fn restart_controller(&mut self) {
        let _ = self.shutdown.send(());
        let (shutdown, _) = broadcast::channel(1);
        let BoundEndpoints { rest, .. } = launch_controller(self.params.clone(), &shutdown).await;
        self.rest = rest;
        self.shutdown = shutdown;
    }

    fn acquire(&self, operator: &str) -> String {
        let (status, body) = post(
            self.rest,
            "/api/control/acquire",
            json!({"operator": operator, "passphrase": "change-me"}),
        );
        assert_eq!(status, 200, "{body}");
        body["token"].as_str().unwrap().to_string()
    }

    fn logbook(&self) -> Vec<Value> {
        let (status, body) = get(self.rest, "/api/logbook?since_seq=0");
        assert_eq!(status, 200, "{body}");
        body["records"].as_array().unwrap().clone()
    }

    /// `GET /api/status` に載る receiver0 の `frames`(GetStatus 実測)。
    fn receiver_frames(&self) -> u64 {
        let (_, body) = get(self.rest, "/api/status");
        body["components"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == json!("receiver0"))
            .and_then(|c| c["metrics"]["frames"].as_u64())
            .unwrap_or(0)
    }

    fn shutdown(self) {
        let _ = self.shutdown.send(());
        for component in &self.components {
            let _ = component.shutdown.send(());
        }
        let _ = std::fs::remove_dir_all(&self.output_root);
    }
}

/// frameType 1(blkSize=1、big-endian)の最小の正しい CoBo フレーム = ヘッダ 88 B + item 2 個
/// (4 B × 2)= **96 B**。decoder が malformed にしない形(src/decode.rs の `make_cobo_frame` と
/// 同じ配置)。AsAd は呼び手が交互に振る(graw-writer が AsAd 毎ファイルに分ける)。
const SYNTH_FRAME_BYTES: usize = 96;

fn synth_frame(event_idx: u32, asad: u8) -> Vec<u8> {
    let put_be = |b: &mut [u8], at: usize, value: u64, len: usize| {
        for i in 0..len {
            b[at + i] = (value >> (8 * (len - 1 - i))) as u8;
        }
    };
    let mut b = vec![0u8; SYNTH_FRAME_BYTES];
    b[0] = 0x00; // big-endian, blkSize=1
    put_be(&mut b, 1, SYNTH_FRAME_BYTES as u64, 3); // frameSize
    put_be(&mut b, 5, 1, 2); // frameType 1
    put_be(&mut b, 8, 88, 2); // headerSize
    put_be(&mut b, 10, 4, 2); // itemSize
    put_be(&mut b, 12, 2, 4); // nItems
    put_be(&mut b, 22, u64::from(event_idx), 4);
    b[26] = 0; // CoBo 0
    b[27] = asad;
    // item = aget<<30 | chan<<23 | bucket<<14 | adc(非対称な 2 個)
    let items: [(u32, u32, u32, u32); 2] = [(0, 3, 10, 500), (1, 64, 200, 1234)];
    for (i, (aget, chan, bucket, adc)) in items.iter().enumerate() {
        let word = (aget << 30) | (chan << 23) | (bucket << 14) | adc;
        put_be(&mut b, 88 + i * 4, u64::from(word), 4);
    }
    b
}

/// 112 §2-1 (b) の実データ版: 本物の receiver / decoder / graw-writer で「run 中に数十フレーム →
/// stop の 200 → 待ち 0 で PUT → start」を 5 周。
///
/// 実 CoBo と同じく **データリンクは stop で閉じない**(`daqStop` は TCP を close しない —
/// SPEC §1.3 v1.12)。前の周の接続は**次の start が返るまで握ったまま**にする(実機で CoBo が
/// close するのは次 run の breakup)。したがって停止は静止検出 → 強制 EOS の実機経路
/// (`forced_eos=true` / `eos_closed=true`)を通り、receiver は**毎周同じポート**へ
/// bind し直す(穴 ③ の listen 解放 → 再 listen)。
///
/// 照合: 各 run の `frames` = 送った数 / graw のバイト数 = 送った数 × 96 B / decoder 下流
/// (root-sink の席)が受けた EOS の run 番号 = 1..=6(run close 件数 = 周回数)/
/// ECC configure の DataLinkSet は毎周同じポート。周ごとの所要 ms と Arm の粘りを stderr に出す。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn back_to_back_apply_and_restart_with_real_components_relistens_every_round() {
    let (chain, ecc_config_dir) = real_chain("apply-restart-real").await;
    let token = chain.acquire("aogaki");
    let (status, body) = put(
        chain.rest,
        "/api/params",
        json!({"token": token, "params": roundtrip_params_json()}),
    );
    assert_eq!(status, 200, "{body}");
    let (status, body) = post(
        chain.rest,
        "/api/run/start",
        json!({"token": token, "comment": "real 0"}),
    );
    assert_eq!(status, 200, "{body}");

    // run 1..=6 で送るフレーム数(非対称・「数十」)。バイト数は × 96 B。
    let frames_per_run: [u64; 6] = [24, 31, 27, 40, 22, 35];
    let delays: [u64; 5] = [1700, 1832, 1650, 1999, 1777];
    let mut event_idx = 0u32;
    let mut previous_link: Option<std::net::TcpStream> = None;

    for (index, &frames) in frames_per_run.iter().enumerate() {
        let run = index as u64 + 1;

        // --- fake CoBo: この run のデータリンクを張って frames 個送る(close しない)---
        let mut link = std::net::TcpStream::connect(("127.0.0.1", chain.data_port))
            .unwrap_or_else(|e| panic!("run {run}: receiver が listen していない: {e}"));
        // 前の run の接続は次の start が返るまで握っていた(実機の CoBo は breakup で close)。
        drop(previous_link.take());
        for i in 0..frames {
            link.write_all(&synth_frame(event_idx, (i % 2) as u8))
                .unwrap();
            event_idx += 1;
        }
        wait_until(&format!("run {run}: receiver frames == {frames}"), || {
            chain.receiver_frames() == frames
        });

        // --- stop の 200 → 待ち 0 で PUT → start(最後の run は stop だけ)---
        let began = Instant::now();
        let (status, stop) = post(chain.rest, "/api/run/stop", json!({"token": token}));
        let stop_ms = began.elapsed().as_millis();
        assert_eq!(status, 200, "run {run}: {stop}");
        assert_eq!(stop["run"], json!(run), "run {run}: {stop}");
        assert_eq!(stop["ok"], json!(true), "run {run}: {stop}");
        assert_eq!(stop["eos_closed"], json!(true), "run {run}: {stop}");
        assert_eq!(stop["notes"], json!([]), "run {run}: {stop}");
        assert_eq!(
            stop["forced_eos"],
            json!(true),
            "run {run}: リンクを握ったままなので実機経路(静止 → 強制 EOS): {stop}"
        );
        previous_link = Some(link);

        let Some(&delay) = delays.get(index) else {
            eprintln!("112 real chain run {run}: final stop {stop_ms} ms ({frames} frames)");
            break;
        };
        let began = Instant::now();
        let (status, put_body) = put(
            chain.rest,
            "/api/params",
            json!({"token": token, "params": params_with_trigger_delay(delay)}),
        );
        let put_ms = began.elapsed().as_millis();
        assert_eq!(status, 200, "run {run}: {put_body}");
        let id = put_body["id"].as_str().unwrap().to_string();

        let began = Instant::now();
        let (status, start) = post(
            chain.rest,
            "/api/run/start",
            json!({"token": token, "comment": format!("real {}", index + 1)}),
        );
        let start_ms = began.elapsed().as_millis();
        assert_eq!(status, 200, "run {run}: {start}");
        assert_eq!(start["run"], json!(run + 1), "run {run}: {start}");
        let configure = chain.ecc.requests_for("configure").last().cloned().unwrap();
        assert_eq!(configure["config_id"], json!(id), "run {run}");
        assert_eq!(
            configure["links"][0]["router_port"],
            json!(chain.data_port),
            "run {}: receiver が同じポートで listen し直していない",
            run + 1
        );

        let records = chain.logbook();
        let start_audit = records
            .iter()
            .rev()
            .find(|r| r["action"] == json!("run/start") && r["ok"] == json!(true))
            .unwrap();
        eprintln!(
            "112 real chain run {run} -> {}: {frames} frames / stop {stop_ms} ms / put {put_ms} ms \
             / start {start_ms} ms / total {} ms / arm_retries {}",
            run + 1,
            stop_ms + put_ms + start_ms,
            start_audit["params"]["arm_retries"],
        );
    }
    drop(previous_link);

    // --- run 台帳: frames と graw のバイト数が送った数と一致(run 間で混ざらない)---
    let records = chain.logbook();
    let stops: Vec<&Value> = records
        .iter()
        .filter(|r| r["type"] == json!("run_stop"))
        .collect();
    assert_eq!(stops.len(), frames_per_run.len());
    for (run_stop, &frames) in stops.iter().zip(frames_per_run.iter()) {
        let run = run_stop["run"].as_u64().unwrap();
        assert_eq!(run_stop["ok"], json!(true), "{run_stop}");
        assert_eq!(
            run_stop["counters"]["frames"],
            json!({"0": frames}),
            "run {run}"
        );
        let files = run_stop["files"].as_array().unwrap();
        let bytes: u64 = files.iter().map(|f| f["bytes"].as_u64().unwrap()).sum();
        // 手計算: frames × 96 B(例: run 1 = 24 × 96 = 2304 B)。
        assert_eq!(
            bytes,
            frames * SYNTH_FRAME_BYTES as u64,
            "run {run}: {files:?}"
        );
        assert_eq!(files.len(), 2, "AsAd 0 / 1 の 2 ファイル: {files:?}");
        for file in files {
            // graw-writer は `<output_root>/run{run:04}/CoBo0_AsAd{a}_<ts>_0000.graw` の絶対パスを申告する。
            assert!(
                file["path"]
                    .as_str()
                    .unwrap()
                    .contains(&format!("/run{run:04}/CoBo0_AsAd")),
                "run {run}: {file}"
            );
        }
    }

    // --- decoder 下流(root-sink の席)は run 毎に EOS を 1 回ずつ受けた ---
    wait_until("sink received 6 EOS", || chain.sink.eos_runs().len() == 6);
    assert_eq!(chain.sink.eos_runs(), vec![1, 2, 3, 4, 5, 6]);
    assert!(chain.sink.data_batches.load(Ordering::Relaxed) > 0);

    let _ = std::fs::remove_dir_all(&ecc_config_dir);
    chain.shutdown();
}

/// **114 の統合版**(ELITPC 実機ベースライン 2026-10-12 の再現): 本物の receiver / decoder /
/// graw-writer で run 1 を走らせ、29 フレーム流したところで **controller だけ**を作り直す
/// (相 Idle、コンポーネント 3 つと ECC は Running のまま)。次の `run/start` はチェーンが
/// 無傷なので、Reset の前に停止経路の 2 段(受信静止 → receiver `Stop` → EOS 待ち → 下流
/// `Stop`)で run 1 を閉じ、通る。リンクは握ったままなので第一段は静止で切り上がり、強制 EOS。
///
/// 照合: audit `component_walk_back` = `"eos: closed"` + 各コンポーネントの
/// `Running->Configured->Idle` / `ecc_walk_back` = Running からの 5 段 / decoder 下流
/// (root-sink の席)が run 1 の EOS を受けた / run 1 の graw は AsAd 毎に送った数 × 96 B。
/// 115: ログブックは `run_start 1 → run_stop 1(abort:controller-restart)→ run_start 2 →
/// run_stop 2` の順、run_stop 1 の counters / files は送った数どおり、`logbook2runs` は
/// run 1 に `no run_stop` を付けない。
///
/// このテストの低レートでは 113 の「Stop 直後の Reset」でも EOS が競争に勝ちうる
/// (実機ベースラインと同じ)。114 の差は、待って確かめたことが跡に `eos: closed` で残る点。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restarted_controller_closes_the_running_run_with_eos_before_the_next_start() {
    let (mut chain, ecc_config_dir) = real_chain("walk-back-eos").await;
    let token = chain.acquire("aogaki");
    let (status, body) = post(
        chain.rest,
        "/api/run/start",
        json!({"token": token, "comment": "before the crash"}),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["run"], json!(1));

    // --- fake CoBo: run 1 のデータリンクを張って 29 フレーム送る(close しない)---
    // AsAd は偶奇で交互 → AsAd0 = 15 フレーム、AsAd1 = 14 フレーム(非対称)。
    let frames = 29u32;
    let mut link = TcpStream::connect(("127.0.0.1", chain.data_port)).unwrap();
    for i in 0..frames {
        link.write_all(&synth_frame(i, (i % 2) as u8)).unwrap();
    }
    wait_until("run 1: receiver frames == 29", || {
        chain.receiver_frames() == u64::from(frames)
    });

    // --- controller だけが落ちて再起動する ---
    chain.restart_controller().await;
    let (_, status_body) = get(chain.rest, "/api/status");
    assert_eq!(status_body["phase"], json!("Idle"), "前提: {status_body}");
    for component in status_body["components"].as_array().unwrap() {
        assert_eq!(component["state"], json!("Running"), "前提: {component}");
    }
    assert!(
        chain.sink.eos_runs().is_empty(),
        "前提: run 1 はまだ閉じていない"
    );

    // 操作権はプロセスと一緒に消えているので取り直す。
    let token = chain.acquire("aogaki");
    let (status, body) = post(
        chain.rest,
        "/api/run/start",
        json!({"token": token, "comment": "after the restart"}),
    );
    assert_eq!(status, 200, "歩き戻せば start が通る: {body}");
    assert_eq!(body["run"], json!(2), "採番は状態ファイルから引き継ぐ");
    drop(link); // 実 CoBo が前 run のリンクを close するのは breakup

    let records = chain.logbook();
    let audit = records
        .iter()
        .rev()
        .find(|r| r["action"] == json!("run/start") && r["ok"] == json!(true))
        .unwrap();
    assert_eq!(audit["params"]["run"], json!(2));
    assert_eq!(
        audit["params"]["component_walk_back"],
        json!([
            "eos: closed",
            "receiver0: Running->Configured->Idle",
            "decoder: Running->Configured->Idle",
            "graw-writer: Running->Configured->Idle",
        ]),
        "{audit}"
    );
    assert_eq!(
        audit["params"]["ecc_walk_back"],
        json!([
            "status->Running",
            "stop->Ready",
            "breakup->Prepared",
            "reset->Described",
            "reset->Idle",
        ]),
        "{audit}"
    );

    // --- run 1 は EOS で閉じた: root-sink の席が EOS を受け、graw-writer は全ファイルを閉じた ---
    wait_until("sink received the EOS of run 1", || {
        chain.sink.eos_runs() == vec![1]
    });
    let mut graw: Vec<(String, u64)> = std::fs::read_dir(chain.output_root.join("run0001"))
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().to_string();
            (name, entry.metadata().unwrap().len())
        })
        .collect();
    graw.sort();
    assert_eq!(graw.len(), 2, "{graw:?}");
    // 手計算: AsAd0 = 15 × 96 = 1440 B、AsAd1 = 14 × 96 = 1344 B(合計 29 × 96 = 2784 B)。
    assert!(graw[0].0.starts_with("CoBo0_AsAd0_"), "{graw:?}");
    assert_eq!(graw[0].1, 1440, "{graw:?}");
    assert!(graw[1].0.starts_with("CoBo0_AsAd1_"), "{graw:?}");
    assert_eq!(graw[1].1, 1344, "{graw:?}");

    // --- run 2 は通常どおり止まる(歩き戻した後のチェーンが正しく動く)---
    let (status, stop) = post(chain.rest, "/api/run/stop", json!({"token": token}));
    assert_eq!(status, 200, "{stop}");
    assert_eq!(stop["run"], json!(2), "{stop}");
    assert_eq!(stop["ok"], json!(true), "{stop}");
    wait_until("sink received the EOS of run 2", || {
        chain.sink.eos_runs() == vec![1, 2]
    });

    // --- 115: 歩き戻しで閉じた run 1 の run_stop が、run 2 の run_start より前にある ---
    let records = chain.logbook();
    let ledger: Vec<(String, u64)> = records
        .iter()
        .filter(|r| r["type"] == json!("run_start") || r["type"] == json!("run_stop"))
        .map(|r| {
            (
                r["type"].as_str().unwrap().to_string(),
                r["run"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        ledger,
        vec![
            ("run_start".to_string(), 1),
            ("run_stop".to_string(), 1),
            ("run_start".to_string(), 2),
            ("run_stop".to_string(), 2),
        ]
    );
    let run_stop_1 = records
        .iter()
        .find(|r| r["type"] == json!("run_stop") && r["run"] == json!(1))
        .unwrap();
    assert_eq!(run_stop_1["ok"], json!(false), "{run_stop_1}");
    assert_eq!(
        run_stop_1["reason"],
        json!("abort:controller-restart"),
        "{run_stop_1}"
    );
    assert_eq!(run_stop_1["forced_eos"], json!(true), "{run_stop_1}");
    assert_eq!(run_stop_1["eos_closed"], json!(true), "{run_stop_1}");
    assert_eq!(
        run_stop_1["counters"]["frames"],
        json!({"0": 29}),
        "{run_stop_1}"
    );
    // 手計算: 29 × 96 = 2784 B を AsAd 毎の 2 本に(Reset の前に採ったので手放されていない)。
    let files = run_stop_1["files"].as_array().unwrap();
    assert_eq!(files.len(), 2, "{run_stop_1}");
    let bytes: u64 = files.iter().map(|f| f["bytes"].as_u64().unwrap()).sum();
    assert_eq!(bytes, 2784, "{run_stop_1}");
    assert!(
        run_stop_1["duration_s"].as_f64().unwrap() >= 0.0,
        "{run_stop_1}"
    );
    let start_2 = records
        .iter()
        .find(|r| r["action"] == json!("run/start") && r["params"]["run"] == json!(2))
        .unwrap();
    assert_eq!(
        start_2["params"]["previous_run_stop"],
        json!({"run": 1, "notes": []}),
        "{start_2}"
    );

    // --- runs.yaml 変換(082)は run 1 に「no run_stop」を付けない ---
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_logbook2runs"))
        .arg(chain.output_root.join("logbook.jsonl"))
        .output()
        .unwrap();
    let yaml = String::from_utf8_lossy(&output.stdout);
    let warnings = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{warnings}");
    assert!(!yaml.contains("no run_stop"), "{yaml}");
    assert!(yaml.contains("abort:controller-restart"), "{yaml}");
    assert!(!warnings.contains("got a run_stop"), "{warnings}");

    let _ = std::fs::remove_dir_all(&ecc_config_dir);
    chain.shutdown();
}
