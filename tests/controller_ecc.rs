//! controller ↔ **実 ecc-bridge**(C++/Ice)↔ fake-ECC の実配線テスト(TODO/016 §テスト)。
//!
//! `tests/controller_integration.rs` の ECC はテスト内のフェイク REP なので「controller が
//! 何を送るか」しか見ない。ここは 017 の実バイナリを立て、**controller の run 開始シーケンス
//! (describe → prepare → configure → start)が本物のブリッジ越しに通る**ことを見る。
//! DataLinkSet は receiver が実際に bind した口で組まれるので、fake-ECC の CoBo 役が
//! その口へ実際に繋いで来ることまで確認できる(= listen-before-start の実証)。
//!
//! **env 未設定なら skip** —— C++/Ice ビルドを `cargo test` の前提にしない:
//!
//! ```text
//! make -C tools/ecc_bridge TPCDAQ_ICE_DIR=$PWD/reference/20190315_patched
//! TPCDAQ_ECC_BRIDGE_BIN=$PWD/tools/ecc_bridge/ecc_bridge \
//! TPCDAQ_FAKE_ECC_BIN=$PWD/tools/ecc_bridge/fake_ecc \
//!   cargo test --test controller_ecc
//! ```

#![allow(clippy::unwrap_used)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command as OsCommand, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::{broadcast, oneshot};
use tpcdaq::command::{Command, CommandResponse, ComponentState};
use tpcdaq::controller::{
    run_controller, BoundEndpoints, CoboSpec, ComponentEndpoint, ComponentKind, ControllerParams,
};

/// 2 つの env が揃っていなければ skip(片方だけなら「揃っていない」と分かるように出す)。
fn bins() -> Option<(PathBuf, PathBuf)> {
    let bridge = std::env::var_os("TPCDAQ_ECC_BRIDGE_BIN").map(PathBuf::from);
    let fake = std::env::var_os("TPCDAQ_FAKE_ECC_BIN").map(PathBuf::from);
    match (bridge, fake) {
        (Some(bridge), Some(fake)) => Some((bridge, fake)),
        (bridge, fake) => {
            eprintln!("skip: TPCDAQ_ECC_BRIDGE_BIN={bridge:?} TPCDAQ_FAKE_ECC_BIN={fake:?}");
            None
        }
    }
}

/// 子プロセスを必ず殺すためのラッパ(テストが落ちても孤児にしない)。
struct Proc {
    child: Child,
    /// 起動時に stdout へ 1 行だけ出る自己申告(fake_ecc = "PROXY ..."、bridge = "BIND ...")。
    banner: String,
}

impl Proc {
    fn spawn(mut command: OsCommand, prefix: &str) -> Proc {
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
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------------
// 手書き HTTP クライアント(依存を増やさない — 統合テストと同じ流儀)
// ---------------------------------------------------------------------

fn http(method: &str, addr: SocketAddr, path: &str, body: Option<&str>) -> (u16, Value) {
    let payload = body.unwrap_or("");
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{payload}",
        payload.len()
    );
    let mut stream = TcpStream::connect(addr).expect("connect to the controller");
    stream
        .set_read_timeout(Some(Duration::from_secs(60)))
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

// ---------------------------------------------------------------------
// フェイクのコンポーネント(本物の状態機械)
// ---------------------------------------------------------------------

struct FakeComponent {
    state: ComponentState,
    metrics: Value,
    commands: Arc<Mutex<Vec<String>>>,
}

impl FakeComponent {
    fn handle(&mut self, command: Command) -> CommandResponse {
        let name = format!("{command}");
        if !matches!(command, Command::GetStatus) {
            self.commands.lock().unwrap().push(name.clone());
        }
        let Some(target) = command.target_state() else {
            return CommandResponse::success(self.state, "status")
                .with_metrics(self.metrics.clone());
        };
        if !self.state.can_transition_to(target) {
            return CommandResponse::error(self.state, format!("illegal: {name}"))
                .with_metrics(self.metrics.clone());
        }
        self.state = target;
        CommandResponse::success(self.state, name).with_metrics(self.metrics.clone())
    }
}

async fn spawn_fake(
    metrics: Value,
    shutdown: broadcast::Receiver<()>,
) -> (String, Arc<Mutex<Vec<String>>>) {
    let commands = Arc::new(Mutex::new(Vec::new()));
    let mut fake = FakeComponent {
        state: ComponentState::Idle,
        metrics,
        commands: Arc::clone(&commands),
    };
    let (bound_tx, bound_rx) = oneshot::channel();
    tokio::spawn(tpcdaq::command::run_command_task(
        "tcp://127.0.0.1:0".to_string(),
        "fake",
        shutdown,
        Some(bound_tx),
        move |command| fake.handle(command),
    ));
    (bound_rx.await.unwrap(), commands)
}

fn temp_root(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "tpcdaq-controller-ecc-{tag}-{}-{n}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

// ---------------------------------------------------------------------
// テスト
// ---------------------------------------------------------------------

/// controller の run 開始が **実 ecc-bridge 越し**に describe → prepare → configure → start を
/// 通し、fake-ECC の CoBo 役が receiver の実 bind 口へ接続してくること。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_run_start_drives_the_real_bridge_through_to_ecc_start() {
    let Some((bridge_bin, fake_bin)) = bins() else {
        return;
    };

    // fake-ECC(Ice servant)→ ecc_bridge(ZMQ REP)を起動する。
    let mut fake_cmd = OsCommand::new(&fake_bin);
    fake_cmd.args(["--port", "0"]);
    let fake_ecc = Proc::spawn(fake_cmd, "PROXY");

    let mut bridge_cmd = OsCommand::new(&bridge_bin);
    bridge_cmd.args([
        "--bind",
        "tcp://127.0.0.1:*",
        "--ecc-proxy",
        &fake_ecc.banner,
    ]);
    let bridge = Proc::spawn(bridge_cmd, "BIND");

    // フェイクのコンポーネント一式。receiver は**本物の TCP listener** を握る
    // (= listen-before-start。fake-ECC の CoBo 役はこの口へ繋いで来る)。
    let (shutdown, _) = broadcast::channel(1);
    let (graw_endpoint, graw_commands) =
        spawn_fake(json!({"files_open": 0, "files": []}), shutdown.subscribe()).await;
    let (decoder_endpoint, decoder_commands) =
        spawn_fake(json!({"eos_in": 1, "malformed": 0}), shutdown.subscribe()).await;

    let data_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let data_address = data_listener.local_addr().unwrap();
    let (receiver_endpoint, receiver_commands) = spawn_fake(
        json!({
            "cobo_id": 0,
            "bind_address": data_address.to_string(),
            "router_port": data_address.port(),
            "frames": 0,
            "overflow_frames": 0,
        }),
        shutdown.subscribe(),
    )
    .await;

    let output_root = temp_root("run-start");
    let params = ControllerParams {
        rest_listen: "127.0.0.1:0".to_string(),
        passphrase: "change-me".to_string(),
        log_pull_bind: "tcp://127.0.0.1:*".to_string(),
        ui_dir: None,
        config_id: "controller-ecc".to_string(),
        output_root: output_root.clone(),
        geometry_path: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/geometry_mini_reduced.dat"),
        cobos: vec![CoboSpec {
            id: 0,
            listen: data_address.to_string(),
            data_sender_id: "CoBo[0]".to_string(),
        }],
        components: vec![
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
            ComponentEndpoint {
                name: "receiver0".to_string(),
                endpoint: receiver_endpoint,
                kind: ComponentKind::Receiver { cobo_id: 0 },
            },
        ],
        ecc_endpoint: bridge.banner.clone(),
        router_ip: None,
        eos_timeout: Duration::from_millis(400),
        eos_poll: Duration::from_millis(50),
        command_timeout: Duration::from_secs(5),
        ecc_timeout: Duration::from_secs(30),
        status_timeout: Duration::from_secs(5),
    };

    let (bound_tx, bound_rx) = oneshot::channel();
    let shutdown_rx = shutdown.subscribe();
    tokio::spawn(async move {
        if let Err(e) = run_controller(params, shutdown_rx, Some(bound_tx)).await {
            panic!("controller failed: {e}");
        }
    });
    let BoundEndpoints { rest, .. } = bound_rx.await.unwrap();

    // --- 操作権 → run 開始(ここで実ブリッジ越しに ECC が動く)---
    let (status, body) = http(
        "POST",
        rest,
        "/api/control/acquire",
        Some(&json!({"operator": "aogaki", "passphrase": "change-me"}).to_string()),
    );
    assert_eq!(status, 200, "{body}");
    let token = body["token"].as_str().unwrap().to_string();

    let (status, body) = http(
        "POST",
        rest,
        "/api/run/start",
        Some(&json!({"token": token, "comment": "ecc wiring"}).to_string()),
    );
    assert_eq!(status, 200, "run start failed: {body}");

    // コンポーネントは §1.3 どおり動いた。
    assert_eq!(
        *graw_commands.lock().unwrap(),
        vec!["Configure(run=1)", "Arm", "Start(run=1)"]
    );
    assert_eq!(
        *decoder_commands.lock().unwrap(),
        vec!["Configure(run=1)", "Arm", "Start(run=1)"]
    );
    assert_eq!(
        *receiver_commands.lock().unwrap(),
        vec!["Configure(run=1)", "Arm", "Start(run=1)"]
    );

    // ECC は Running(実ブリッジの status で確かめる)。
    let (status, body) = http("GET", rest, "/api/status", None);
    assert_eq!(status, 200);
    assert_eq!(body["ecc"]["ok"], json!(true), "{}", body["ecc"]);
    assert_eq!(body["ecc"]["state"], json!("Running"), "{}", body["ecc"]);
    assert_eq!(body["phase"], json!("Running"));

    // listen-before-start の実証: fake-ECC の CoBo 役が receiver の口へ実際に繋いで来る。
    data_listener
        .set_nonblocking(false)
        .expect("blocking accept");
    let (connection, peer) = data_listener
        .accept()
        .expect("the fake CoBo connects to the port the controller published");
    eprintln!("fake CoBo connected from {peer}");
    drop(connection);

    // --- 停止(ecc stop も実ブリッジ越し)---
    let (status, body) = http(
        "POST",
        rest,
        "/api/run/stop",
        Some(&json!({"token": token}).to_string()),
    );
    assert_eq!(status, 200, "{body}");
    let (_, body) = http("GET", rest, "/api/status", None);
    assert_eq!(body["ecc"]["state"], json!("Ready"), "ecc stop で Ready へ");
    assert_eq!(body["phase"], json!("Idle"));

    let _ = shutdown.send(());
    let _ = std::fs::remove_dir_all(&output_root);
}

/// 段階操作(`POST /api/ecc/{action}`)も実ブリッジ越しに通る(R6)。
/// **listen していないポートでの start は失敗する**(listen-before-start の負性)。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_step_by_step_ecc_endpoints_reach_the_real_bridge() {
    let Some((bridge_bin, fake_bin)) = bins() else {
        return;
    };

    let mut fake_cmd = OsCommand::new(&fake_bin);
    fake_cmd.args(["--port", "0"]);
    let fake_ecc = Proc::spawn(fake_cmd, "PROXY");
    let mut bridge_cmd = OsCommand::new(&bridge_bin);
    bridge_cmd.args([
        "--bind",
        "tcp://127.0.0.1:*",
        "--ecc-proxy",
        &fake_ecc.banner,
    ]);
    let bridge = Proc::spawn(bridge_cmd, "BIND");

    let (shutdown, _) = broadcast::channel(1);
    let (graw_endpoint, _) =
        spawn_fake(json!({"files_open": 0, "files": []}), shutdown.subscribe()).await;
    let (decoder_endpoint, _) =
        spawn_fake(json!({"eos_in": 1, "malformed": 0}), shutdown.subscribe()).await;

    // 誰も listen していないポートを receiver が「bind した」と申告する状況を作る。
    let dead_port = {
        let probe = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        port
    };
    let (receiver_endpoint, _) = spawn_fake(
        json!({
            "cobo_id": 0,
            "bind_address": format!("127.0.0.1:{dead_port}"),
            "router_port": dead_port,
        }),
        shutdown.subscribe(),
    )
    .await;

    let output_root = temp_root("steps");
    let params = ControllerParams {
        rest_listen: "127.0.0.1:0".to_string(),
        passphrase: "change-me".to_string(),
        log_pull_bind: "tcp://127.0.0.1:*".to_string(),
        ui_dir: None,
        config_id: "controller-ecc-steps".to_string(),
        output_root: output_root.clone(),
        geometry_path: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/geometry_mini_reduced.dat"),
        cobos: vec![CoboSpec {
            id: 0,
            listen: format!("127.0.0.1:{dead_port}"),
            data_sender_id: "CoBo[0]".to_string(),
        }],
        components: vec![
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
            ComponentEndpoint {
                name: "receiver0".to_string(),
                endpoint: receiver_endpoint,
                kind: ComponentKind::Receiver { cobo_id: 0 },
            },
        ],
        ecc_endpoint: bridge.banner.clone(),
        router_ip: None,
        eos_timeout: Duration::from_millis(400),
        eos_poll: Duration::from_millis(50),
        command_timeout: Duration::from_secs(5),
        ecc_timeout: Duration::from_secs(30),
        status_timeout: Duration::from_secs(5),
    };

    let (bound_tx, bound_rx) = oneshot::channel();
    let shutdown_rx = shutdown.subscribe();
    tokio::spawn(async move {
        if let Err(e) = run_controller(params, shutdown_rx, Some(bound_tx)).await {
            panic!("controller failed: {e}");
        }
    });
    let BoundEndpoints { rest, .. } = bound_rx.await.unwrap();

    let (_, body) = http(
        "POST",
        rest,
        "/api/control/acquire",
        Some(&json!({"operator": "aogaki", "passphrase": "change-me"}).to_string()),
    );
    let token = body["token"].as_str().unwrap().to_string();
    let body_token = json!({"token": token}).to_string();

    for (action, state) in [
        ("describe", "Described"),
        ("prepare", "Prepared"),
        ("configure", "Ready"),
    ] {
        let (status, body) = http(
            "POST",
            rest,
            &format!("/api/ecc/{action}"),
            Some(&body_token),
        );
        assert_eq!(status, 200, "{action}: {body}");
        assert_eq!(body["ok"], json!(true), "{action}: {body}");
        assert_eq!(body["state"], json!(state), "{action}: {body}");
    }

    // listen-before-start の負性: 誰も listen していない口では ECC が start できない。
    let (status, body) = http("POST", rest, "/api/ecc/start", Some(&body_token));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], json!(false), "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("Could not establish data link"),
        "実 ECC の文言をそのまま運ぶこと: {body}"
    );

    let _ = shutdown.send(());
    let _ = std::fs::remove_dir_all(&output_root);
}
