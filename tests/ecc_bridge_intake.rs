//! ecc-bridge(C++/Ice)の JSON REQ/REP 契約を **controller 側(Rust)から**固定する
//! env-gated テスト(TODO/017 §テスト、SPEC §8.2)。
//!
//! 016(controller)がここに繋ぐので、「Rust から見た ecc-bridge の形」——
//! リクエストのキー名、レスポンスの 3 フィールド、ECC 不達時の `state: "Unknown"` ——
//! を Rust 側の回帰として置いておく。状態遷移そのものの網羅は C++ 側の
//! `tools/ecc_bridge/run_ecc_e2e.sh` が担当(こちらは契約の口だけ見る)。
//!
//! **env 未設定なら skip** —— C++/Ice ビルドを `cargo test` の前提にしない:
//!
//! ```text
//! make -C tools/ecc_bridge TPCDAQ_ICE_DIR=$PWD/reference/20190315_patched
//! TPCDAQ_ECC_BRIDGE_BIN=$PWD/tools/ecc_bridge/ecc_bridge \
//! TPCDAQ_FAKE_ECC_BIN=$PWD/tools/ecc_bridge/fake_ecc \
//!   cargo test --test ecc_bridge_intake
//! ```

#![allow(clippy::unwrap_used)]

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use serde_json::Value;

/// 2 つの env が揃っていなければ skip(片方だけなら「揃っていない」と分かるように出す)。
fn bins() -> Option<(PathBuf, PathBuf)> {
    let bridge = std::env::var_os("TPCDAQ_ECC_BRIDGE_BIN").map(PathBuf::from);
    let fake = std::env::var_os("TPCDAQ_FAKE_ECC_BIN").map(PathBuf::from);
    match (bridge, fake) {
        (Some(b), Some(f)) => Some((b, f)),
        (b, f) => {
            eprintln!(
                "skip: TPCDAQ_ECC_BRIDGE_BIN={:?} TPCDAQ_FAKE_ECC_BIN={:?}",
                b, f
            );
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
    /// `prefix` で始まる 1 行を stdout から読むまで待つ。
    fn spawn(mut cmd: Command, prefix: &str) -> Proc {
        let mut child = cmd
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

/// REQ ソケットで 1 往復し、レスポンスを JSON として返す。
fn call(sock: &zmq::Socket, request: &str) -> Value {
    sock.send(request, 0).expect("send request");
    let reply = sock
        .recv_string(0)
        .expect("recv reply")
        .expect("utf-8 reply");
    serde_json::from_str(&reply).unwrap_or_else(|e| panic!("reply is not JSON: {reply:?} ({e})"))
}

fn req_socket(endpoint: &str) -> zmq::Socket {
    let ctx = zmq::Context::new();
    let sock = ctx.socket(zmq::REQ).expect("REQ socket");
    sock.set_rcvtimeo(5_000).expect("rcvtimeo");
    sock.set_sndtimeo(5_000).expect("sndtimeo");
    sock.set_linger(0).expect("linger");
    sock.connect(endpoint).expect("connect bridge");
    sock
}

/// レスポンスの 3 フィールド(SPEC §8.2)を機械照合する。
fn assert_reply(reply: &Value, want_ok: bool, want_state: &str) {
    assert_eq!(reply["ok"], Value::Bool(want_ok), "ok mismatch: {reply}");
    assert_eq!(
        reply["state"],
        Value::String(want_state.into()),
        "state mismatch: {reply}"
    );
    assert!(
        reply["error"].is_string(),
        "error must always be present: {reply}"
    );
    if want_ok {
        assert_eq!(
            reply["error"],
            Value::String(String::new()),
            "ok reply must carry no error"
        );
    } else {
        assert!(
            !reply["error"].as_str().unwrap().is_empty(),
            "failed reply must say why: {reply}"
        );
    }
}

/// fake-ECC → ecc_bridge を上げ、controller が使う一連の JSON を Rust から流す。
/// listen-before-start の負性テストもここで 1 本だけ再現する(実体の網羅は C++ 側 e2e)。
#[test]
fn bridge_speaks_the_json_contract_controller_expects() {
    let Some((bridge_bin, fake_bin)) = bins() else {
        return;
    };

    let mut fake_cmd = Command::new(&fake_bin);
    fake_cmd.args(["--port", "0"]);
    let fake = Proc::spawn(fake_cmd, "PROXY");

    let mut bridge_cmd = Command::new(&bridge_bin);
    bridge_cmd.args(["--bind", "tcp://127.0.0.1:*", "--ecc-proxy", &fake.banner]);
    let bridge = Proc::spawn(bridge_cmd, "BIND");

    let sock = req_socket(&bridge.banner);

    // 初期状態(fake-ECC は Off から)
    assert_reply(&call(&sock, r#"{"action":"status"}"#), true, "Off");

    // describe → prepare
    assert_reply(
        &call(&sock, r#"{"action":"describe","config_id":"rust-intake"}"#),
        true,
        "Described",
    );
    assert_reply(
        &call(&sock, r#"{"action":"prepare","config_id":"rust-intake"}"#),
        true,
        "Prepared",
    );

    // configure —— controller は receiver が **実際に bind したポート**を渡す(§8.2)。
    // ここでは「誰も listen していないポート」を借りて即座に手放す。
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    let dead_port = probe.local_addr().unwrap().port();
    drop(probe);
    let configure = |port: u16| {
        format!(
            r#"{{"action":"configure","config_id":"rust-intake","links":[{{"sender":"CoBo[0]","router_ip":"127.0.0.1","router_port":{port},"type":"TCP"}}]}}"#
        )
    };
    assert_reply(&call(&sock, &configure(dead_port)), true, "Ready");

    // **listen-before-start**: listen していないのに start → ECC がデータリンクを張れない。
    let failed = call(&sock, r#"{"action":"start"}"#);
    assert_reply(&failed, false, "Ready");
    let error = failed["error"].as_str().unwrap();
    assert!(
        error.contains("Could not establish data link"),
        "expected the real ECC wording, got {error:?}"
    );

    // listen してから configure → start は通り、CoBo 役が実際に繋いで来る。
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let port = listener.local_addr().unwrap().port();
    // 直前の configure 失敗で ECC は Active(Ready)。**実 ECC の configure は `ST_PREPARED`
    // からしか効かない**(黙ってスキップ)ので、張り替えの前に breakup で Prepared へ戻す
    // (SPEC v1.12 §1.3 / TODO/036)。
    assert_reply(&call(&sock, r#"{"action":"breakup"}"#), true, "Prepared");
    assert_reply(&call(&sock, &configure(port)), true, "Ready");
    assert_reply(&call(&sock, r#"{"action":"start"}"#), true, "Running");
    let (conn, _) = listener.accept().expect("fake CoBo connects after start");
    drop(conn);

    assert_reply(&call(&sock, r#"{"action":"stop"}"#), true, "Ready");
    assert_reply(&call(&sock, r#"{"action":"breakup"}"#), true, "Prepared");
    // 実 ECC の reset は `EV_UNDO` = **1 段戻す**。Prepared → Idle は 2 段(TODO/036)。
    assert_reply(&call(&sock, r#"{"action":"reset"}"#), true, "Described");
    assert_reply(&call(&sock, r#"{"action":"reset"}"#), true, "Idle");

    // 壊れたリクエストでもブリッジは死なない(状態は返るし、次も通る)。
    assert_reply(&call(&sock, r#"{"action":"launch"}"#), false, "Idle");
    assert_reply(&call(&sock, "{not json"), false, "Idle");
    assert_reply(&call(&sock, r#"{"action":"status"}"#), true, "Idle");
}

/// ECC 不達は例外ではなく `{"ok": false, "state": "Unknown"}`(SPEC §8.2)。
#[test]
fn unreachable_ecc_answers_unknown_instead_of_dying() {
    let Some((bridge_bin, _fake_bin)) = bins() else {
        return;
    };

    // 誰も listen していないポートを ECC のふりで指す。
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    let dead_port = probe.local_addr().unwrap().port();
    drop(probe);

    let mut bridge_cmd = Command::new(&bridge_bin);
    bridge_cmd.args([
        "--bind",
        "tcp://127.0.0.1:*",
        "--ecc-proxy",
        &format!("Ecc:tcp -h 127.0.0.1 -p {dead_port}"),
    ]);
    let bridge = Proc::spawn(bridge_cmd, "BIND");

    let sock = req_socket(&bridge.banner);
    for request in [r#"{"action":"status"}"#, r#"{"action":"describe"}"#] {
        let reply = call(&sock, request);
        assert_reply(&reply, false, "Unknown");
    }
    // 落ちずに応答し続ける。
    assert_reply(&call(&sock, r#"{"action":"status"}"#), false, "Unknown");
}
