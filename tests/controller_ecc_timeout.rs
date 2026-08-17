//! TODO/057 — controller の ECC REQ タイムアウトが**本当に発火する**ことの実証。
//!
//! 056 の実測(2026-08-16)で、実 ECC の `configure` が 261 s 掛かったのに controller の
//! `DEFAULT_ECC_TIMEOUT`(60 s)が発火せず、run/start が 261 s 待って完走した。
//! ここは「遅い ecc-bridge」を相手に、**`ecc_timeout` を過ぎたら Err で返る**ことを
//! transport 単体で押さえる(発火経路の特定 = 発注書 §調べること 1)。

#![allow(clippy::unwrap_used)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::{broadcast, oneshot};
use tpcdaq::command::{Command, CommandResponse, ComponentState};
use tpcdaq::config::ConfigIds;
use tpcdaq::controller::{
    run_controller, BoundEndpoints, CoboSpec, ComponentEndpoint, ComponentKind, ControllerParams,
    Transport, ZmqTransport,
};

/// 応答を `delay` だけ遅らせる ecc-bridge 役(REP)。1 回応答したら終わる。
struct SlowEcc {
    endpoint: String,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl SlowEcc {
    fn spawn(delay: Duration) -> Self {
        Self::spawn_slow_on(delay, None)
    }

    /// `slow_action` が `Some(a)` なら **その action だけ** `delay` 固まる(他は即答)。
    /// `None` なら全 action が固まる。状態機械は実 ECC 準拠
    /// (`tests/controller_integration.rs` の `FakeEcc` と同じ出典 =
    /// `reference/20190315_patched` の `GetBench/src/get/rc/BackEnd.cpp`)。
    fn spawn_slow_on(delay: Duration, slow_action: Option<&'static str>) -> Self {
        let context = zmq::Context::new();
        let socket = context.socket(zmq::REP).unwrap();
        socket.set_rcvtimeo(100).unwrap();
        socket.set_linger(0).unwrap();
        socket.bind("tcp://127.0.0.1:*").unwrap();
        let endpoint = socket.get_last_endpoint().unwrap().unwrap();

        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            let mut state = "Off".to_string();
            while !thread_stop.load(Ordering::Relaxed) {
                let message = match socket.recv_bytes(0) {
                    Ok(message) => message,
                    Err(_) => continue, // EAGAIN = 100 ms の目覚まし
                };
                let request: Value = serde_json::from_slice(&message).unwrap_or(Value::Null);
                let action = request
                    .get("action")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();

                // 実 ECC の遅い相の役: ここで長く固まる(その間 REP は無言)。
                if slow_action.is_none_or(|slow| slow == action) {
                    let began = Instant::now();
                    while began.elapsed() < delay && !thread_stop.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                }

                let next = match (action.as_str(), state.as_str()) {
                    ("describe", "Off" | "Idle" | "Described") => Some("Described"),
                    ("prepare", "Described" | "Prepared") => Some("Prepared"),
                    ("configure", "Prepared") => Some("Ready"),
                    ("start", "Ready") => Some("Running"),
                    ("stop", "Running" | "Paused") => Some("Ready"),
                    ("breakup", "Ready" | "Running" | "Paused") => Some("Prepared"),
                    ("reset", "Described") => Some("Idle"),
                    ("reset", "Prepared") => Some("Described"),
                    _ => None,
                };
                if let Some(next) = next {
                    state = next.to_string();
                }
                let reply = json!({"ok": true, "state": state, "error": "", "ecc_error": "NO_ERR"});
                let _ = socket.send(reply.to_string().as_bytes(), 0);
            }
        });
        Self {
            endpoint,
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for SlowEcc {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// **057 の核**: `ecc_timeout` を過ぎても応答が来なければ `Err` で返る。
///
/// 数字の出典(手計算): 相手は 4 s 固まる / `ecc_timeout` = 1 s。
/// 正しく発火するなら 1 s 前後で Err。上限 3 s は「相手の 4 s より確実に短い」ための境界で、
/// 遅い CI でも余裕がある(1 s + 2 s のマージン)。
#[test]
fn the_ecc_request_gives_up_after_the_ecc_timeout() {
    let ecc = SlowEcc::spawn(Duration::from_secs(4));
    let mut transport = ZmqTransport::new(
        ecc.endpoint.clone(),
        Duration::from_secs(1), // command_timeout(ここでは使わない)
        Duration::from_secs(1), // ecc_timeout = 発火させたい上限
    );

    let began = Instant::now();
    let result = transport.ecc(&json!({"action": "configure"}));
    let elapsed = began.elapsed();

    assert!(
        result.is_err(),
        "ecc_timeout を過ぎたのに Ok で返った(= タイムアウトが発火していない): {result:?}"
    );
    assert!(
        elapsed < Duration::from_secs(3),
        "ecc_timeout 1 s のはずが {} ms 待った(相手の 4 s に付き合っている)",
        elapsed.as_millis()
    );
}

/// 相手が `ecc_timeout` 内に返すなら、当然そのまま `Ok` で通る(上のテストが
/// 「常に Err」で通ってしまわないための対照)。
#[test]
fn a_prompt_ecc_reply_still_comes_back_ok() {
    let ecc = SlowEcc::spawn(Duration::from_millis(100));
    let mut transport = ZmqTransport::new(
        ecc.endpoint.clone(),
        Duration::from_secs(1),
        Duration::from_secs(5),
    );

    // Off --describe--> Described(実 ECC の SM。SlowEcc の遷移表が出典)。
    let reply = transport.ecc(&json!({"action": "describe"})).unwrap();
    assert!(reply.ok);
    assert_eq!(reply.state, "Described");
}

// ---------------------------------------------------------------------
// REST 層まで含めた実配線(遅い ECC → run/start が諦める意味論)
// ---------------------------------------------------------------------

/// 1 リクエスト = 1 接続。`(status, body)`。`tests/controller_integration.rs` と同じ手書き。
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
    (status, serde_json::from_str(&body).unwrap_or(Value::Null))
}

fn post(addr: SocketAddr, path: &str, body: Value) -> (u16, Value) {
    http("POST", addr, path, Some(&body.to_string()))
}

/// 本物の状態機械で応じるフェイクコンポーネント(`command::run_command_task` を使う)。
async fn spawn_fake(metrics: Value, shutdown: broadcast::Receiver<()>) -> String {
    let mut state = ComponentState::Idle;
    let (bound_tx, bound_rx) = oneshot::channel();
    tokio::spawn(tpcdaq::command::run_command_task(
        "tcp://127.0.0.1:0".to_string(),
        "fake",
        shutdown,
        Some(bound_tx),
        move |command: Command| {
            let Some(target) = command.target_state() else {
                return CommandResponse::success(state, "status".to_string())
                    .with_metrics(metrics.clone());
            };
            if !state.can_transition_to(target) {
                return CommandResponse::error(state, format!("illegal {state} -> {target}"))
                    .with_metrics(metrics.clone());
            }
            state = target;
            CommandResponse::success(state, "ok".to_string()).with_metrics(metrics.clone())
        },
    ));
    bound_rx.await.unwrap()
}

fn temp_root(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("tpcdaq-057-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// **057 の出口**: run/start の途中で ECC が `ecc_timeout` を超えて黙ったら、
/// controller は待ち続けず **`ecc` 段の失敗として REST に返す**(SPEC §8.2 の
/// 「ecc 不達」= §1.3 の巻き戻し)。監査ログにも失敗として残る(silent にしない)。
///
/// 数字の出典(手計算): fake ECC は `configure` だけ **6 s** 固まる / `ecc_timeout` = **1 s**。
/// 正しく諦めるなら run/start は 1 s + 前段(component コマンド数十 ms)で返る。
/// 上限 **5 s** は「相手の 6 s より確実に短い」ための境界。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_run_start_gives_up_when_the_ecc_stalls_past_the_ecc_timeout() {
    let (shutdown, _) = broadcast::channel(1);
    let output_root = temp_root("run-start-stall");

    let graw = spawn_fake(json!({"files_open": 0, "files": []}), shutdown.subscribe()).await;
    let decoder = spawn_fake(
        json!({"eos_in": 1, "eos_out": 1, "malformed": 0}),
        shutdown.subscribe(),
    )
    .await;
    // receiver は「実際に bind した口」を Arm 応答で返す(DataLinkSet の材料)。
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let receiver = spawn_fake(
        json!({
            "cobo_id": 0,
            "bind_address": address.to_string(),
            "router_port": address.port(),
            "frames": 3852,
            "bytes": 30_108_684u64,
        }),
        shutdown.subscribe(),
    )
    .await;

    let ecc = SlowEcc::spawn_slow_on(Duration::from_secs(6), Some("configure"));
    let params = ControllerParams {
        rest_listen: "127.0.0.1:0".to_string(),
        passphrase: "change-me".to_string(),
        log_pull_bind: "tcp://127.0.0.1:*".to_string(),
        ui_dir: None,
        config_ids: ConfigIds::same("057"),
        output_root: output_root.clone(),
        geometry_path: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/geometry_2cobo_fake.dat"),
        cobos: vec![CoboSpec {
            id: 0,
            listen: "0.0.0.0:46005".to_string(),
            data_sender_id: "CoBo[0]".to_string(),
        }],
        components: vec![
            ComponentEndpoint {
                name: "graw-writer".to_string(),
                endpoint: graw,
                kind: ComponentKind::GrawWriter,
            },
            ComponentEndpoint {
                name: "decoder".to_string(),
                endpoint: decoder,
                kind: ComponentKind::Decoder,
            },
            ComponentEndpoint {
                name: "receiver0".to_string(),
                endpoint: receiver,
                kind: ComponentKind::Receiver { cobo_id: 0 },
            },
        ],
        ecc_endpoint: ecc.endpoint.clone(),
        router_ip: None,
        eos_timeout: Duration::from_millis(400),
        eos_quiesce: Duration::from_millis(200),
        eos_poll: Duration::from_millis(50),
        command_timeout: Duration::from_secs(5),
        ecc_timeout: Duration::from_secs(1),
        status_timeout: Duration::from_secs(2),
    };

    let (bound_tx, bound_rx) = oneshot::channel();
    let shutdown_rx = shutdown.subscribe();
    tokio::spawn(async move {
        run_controller(params, shutdown_rx, Some(bound_tx))
            .await
            .expect("controller");
    });
    let BoundEndpoints { rest, .. } = bound_rx.await.unwrap();

    let (status, body) = post(
        rest,
        "/api/control/acquire",
        json!({"operator": "057", "passphrase": "change-me"}),
    );
    assert_eq!(status, 200, "{body}");
    let token = body["token"].as_str().unwrap().to_string();

    let began = Instant::now();
    let (status, body) = post(rest, "/api/run/start", json!({"token": token}));
    let elapsed = began.elapsed();

    assert_eq!(status, 500, "遅い ECC なのに run/start が成功した: {body}");
    let error = body["error"].as_str().unwrap_or_default().to_string();
    assert!(
        error.contains("ecc stage failed") && error.contains("ecc configure unreachable"),
        "ecc 段の失敗として返っていない: {error}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "ecc_timeout 1 s のはずが run/start が {} ms 掛かった(ECC の 6 s に付き合っている)",
        elapsed.as_millis()
    );

    // 監査ログにも失敗として残る(SPEC §8.1「状態変更系はすべて監査ログ」)。
    let (status, body) = http("GET", rest, "/api/logbook?since_seq=0", None);
    assert_eq!(status, 200, "{body}");
    let audit = body["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["type"] == "audit" && r["action"] == "run/start")
        .cloned()
        .expect("run/start の audit 行");
    assert_eq!(audit["ok"], json!(false), "{audit}");
    assert!(
        audit["error"]
            .as_str()
            .unwrap_or_default()
            .contains("ecc configure unreachable"),
        "{audit}"
    );

    let _ = shutdown.send(());
    let _ = std::fs::remove_dir_all(&output_root);
}
