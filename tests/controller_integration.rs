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

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
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

// ---------------------------------------------------------------------
// フェイクのコンポーネント(本物の状態機械 + コマンド記録)
// ---------------------------------------------------------------------

#[derive(Default)]
struct Recorder {
    /// 受けたコマンド名(`GetStatus` は観測なので記録しない)。
    commands: Mutex<Vec<String>>,
    /// `Configure` で受け取った設定断片(run 開始 TS の配布を見るため)。
    configs: Mutex<Vec<Value>>,
}

impl Recorder {
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
                }
                let reply = json!({"ok": true, "state": state, "error": ""});
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

    let ecc = FakeEcc::spawn();
    let params = ControllerParams {
        rest_listen: "127.0.0.1:0".to_string(),
        passphrase: "change-me".to_string(),
        log_pull_bind: "tcp://127.0.0.1:*".to_string(),
        ui_dir,
        config_ids,
        output_root: output_root.clone(),
        geometry_path: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/geometry_2cobo_fake.dat"),
        cobos,
        components,
        ecc_endpoint: ecc.endpoint.clone(),
        router_ip: None,
        // 強制 EOS 分岐を秒単位で待たずに試せるよう、テストでは短くする。
        eos_timeout: Duration::from_millis(400),
        eos_poll: Duration::from_millis(50),
        command_timeout: Duration::from_secs(5),
        ecc_timeout: Duration::from_secs(5),
        status_timeout: Duration::from_secs(2),
    };

    let (bound_tx, bound_rx) = oneshot::channel();
    let shutdown_rx = shutdown.subscribe();
    tokio::spawn(async move {
        if let Err(e) = run_controller(params, shutdown_rx, Some(bound_tx)).await {
            panic!("controller failed: {e}");
        }
    });
    let BoundEndpoints { rest, log_pull } = bound_rx.await.unwrap();

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

    harness.shutdown();
}

/// 開始シーケンスが途中で失敗したら巻き戻す(実 ZMQ 版)。
/// receiver を先に Configure 済みにしておくと controller の `Configure` が
/// 不許可遷移で断られる —— そこから先へ進まず、実行済みだけが畳まれる。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_start_rolls_back_and_leaves_no_half_started_run() {
    let harness = harness("rollback", &[0], true, None).await;

    // receiver0 を Idle から動かしておく(controller の Configure が断られる状況を作る)。
    let context = zmq::Context::new();
    let receiver_endpoint = {
        let (_, body) = get(harness.rest, "/api/status");
        body["components"][2]["endpoint"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let response = tpcdaq::command::request(
        &context,
        &receiver_endpoint,
        &Command::Configure(tpcdaq::command::RunConfig {
            run_number: 99,
            comment: String::new(),
            config: Value::Null,
        }),
        Duration::from_secs(5),
    )
    .unwrap();
    assert!(
        response.success,
        "前提: receiver は Configured になっている"
    );

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
    // psu からの投稿を模す(ts / seq は付けない = controller が付ける)。
    push.send(
        json!({
            "type": "psu", "actor": "psu", "device": "CAEN0", "channel": 3,
            "event": "TRIP", "values": {"vmon": 350.2, "imon": 0.008, "vset": 350.0}
        })
        .to_string()
        .as_bytes(),
        0,
    )
    .unwrap();
    // 壊れた投稿(ログブックのレコードではない)。
    push.send(b"{\"type\":\"nonsense\"}".as_slice(), 0).unwrap();

    wait_until("the psu log post to land", || {
        harness.logbook().iter().any(|r| r["type"] == json!("psu"))
    });

    let records = harness.logbook();
    let comment = records
        .iter()
        .find(|r| r["type"] == json!("comment"))
        .unwrap();
    assert_eq!(comment["author"], json!("aogaki"));
    assert_eq!(comment["actor"], json!("ui"));
    let psu = records.iter().find(|r| r["type"] == json!("psu")).unwrap();
    assert_eq!(psu["device"], json!("CAEN0"));
    assert_eq!(psu["values"]["vmon"], json!(350.2));
    // ts / seq は controller が付ける(SPEC §9.1)。
    assert_eq!(
        psu["seq"].as_u64().unwrap(),
        comment["seq"].as_u64().unwrap() + 1
    );
    assert_eq!(
        psu["ts"].as_str().unwrap().len(),
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
