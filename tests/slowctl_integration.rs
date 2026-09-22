//! slowctl の統合テスト(TODO/079 §2.6 テスト節、SPEC §2.3 / §9.2)。
//!
//! **実バイナリを子プロセスで起動**し、SPEC のワイヤ 2 本(status PUB と LogPost PUSH)を
//! テスト側が受ける。デバイスは `type = "fake"` —— 中身は本物の Bronkhorst ドライバに
//! インメモリの ProPar 応答器を噛ませたもの(`src/slowctl/fake_device.rs`)なので、
//! コーデック・chaining・カウンタまで実経路を通る。
//!
//! 値の時系列は**スクリプトファイル**で注入する(1 行 = 1 poll、`silent` = 無応答)。
//! ポートは OS に選ばせる(固定ポートを書かない = 並列実行に耐える)。
//!
//! 見るのは発注書の 4 点:
//! ① SUB に `kind == "slow"` が来て `fmeasure` が載っている
//! ② PULL に IDENT → VALUE の順で届く
//! ③ 閾値を跨ぐと ALARM、戻ると RECOVER
//! ④ 無応答で `poll_err` が増え、復帰後に VALUE が再開する

#![allow(clippy::unwrap_used)]

use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use tpcdaq::logbook::LogbookRecord;
use tpcdaq::monitor::decode_message;
use tpcdaq::slowctl::SlowPayload;

/// 1 本の観測にかける上限。poll は 20 Hz なのでスクリプト全体は 1 秒未満で流れる。
const DEADLINE: Duration = Duration::from_secs(20);

/// しきい値(TOML と同じ値をテスト側にも置いて、オラクルを 1 箇所にまとめない)。
const ALARM_LO: f64 = 90.0;
const ALARM_HI: f64 = 110.0;

// ---------------------------------------------------------------------
// 小道具
// ---------------------------------------------------------------------

/// 空きポートを 1 つ確保して即座に手放す(既存統合テストの流儀)。
fn free_port() -> u16 {
    let probe = TcpListener::bind("127.0.0.1:0").expect("bind probe listener");
    let port = probe.local_addr().expect("local_addr").port();
    drop(probe);
    port
}

/// 子プロセスを必ず殺すラッパ(テストが落ちても孤児にしない)。
struct Proc(Child);

impl Drop for Proc {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tpcdaq-slowctl-{}-{}-{name}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// テストが動かすシナリオ(1 行 = 1 poll)。
///
/// * 100 → 範囲内(VALUE)
/// * 120 → `hi = 110` を超える(ALARM)
/// * 100 → 戻る(RECOVER)
/// * silent ×2 → 無応答(`poll_err` が 2 増える)
/// * 105 → 復帰(VALUE が再開)。以後スクリプトは末尾を保持する。
const SCRIPT: &str = "\
100.0
100.0
120.0
120.0
100.0
silent
silent
105.0
";

fn write_config(dir: &Path, script: &Path, pub_port: u16, log_port: u16) -> PathBuf {
    // ジオメトリは `[system]` の必須キー(存在検証がある)。slowctl は読まないので
    // 中身は空でよい。
    let geometry = dir.join("geometry.dat");
    std::fs::write(
        &geometry,
        b"# slowctl integration test does not read this\n",
    )
    .unwrap();

    let path = dir.join("slowctl.toml");
    let toml = format!(
        r#"
[system]
experiment = "slowctl_test"
output_root = "{root}"
geometry = "{geometry}"

[[cobo]]
id = 0
data_sender_id = "CoBo[0]"

[decoder]
workers = 1

[root_sink]
snapshot_hz = 1.0
event_publish_hz = 20.0
build_timeout_ms = 1000

[monitor]
ws_listen = "127.0.0.1:0"

[controller]
passphrase = "test"
ecc_proxy = "Ecc:tcp -h 127.0.0.1 -p 46002"
config_id = "default"

[slowctl]
status_pub = "tcp://127.0.0.1:{pub_port}"
log_push = "tcp://127.0.0.1:{log_port}"
poll_hz = 20.0
log_every_s = 1
[[slowctl.device]]
type = "fake"
name = "gas"
port = "{script}"
nodes = [3]
alarms = [{{ param = "fmeasure", lo = {ALARM_LO}, hi = {ALARM_HI} }}]
"#,
        root = dir.display(),
        geometry = geometry.display(),
        script = script.display(),
    );
    std::fs::write(&path, toml).unwrap();
    path
}

/// テスト側で controller 役の PULL と monitor 役の SUB を先に開く(listen-before-start)。
fn open_sockets(
    context: &zmq::Context,
    pub_port: u16,
    log_port: u16,
) -> (zmq::Socket, zmq::Socket) {
    let log_pull = context.socket(zmq::PULL).unwrap();
    log_pull.set_rcvtimeo(200).unwrap();
    log_pull.set_linger(0).unwrap();
    log_pull
        .bind(&format!("tcp://127.0.0.1:{log_port}"))
        .unwrap();

    let sub = context.socket(zmq::SUB).unwrap();
    sub.set_subscribe(b"").unwrap();
    sub.set_rcvtimeo(200).unwrap();
    sub.set_linger(0).unwrap();
    sub.connect(&format!("tcp://127.0.0.1:{pub_port}")).unwrap();

    (log_pull, sub)
}

/// `slow` レコードを 1 本受ける(期限まで待つ)。
fn next_log_record(log_pull: &zmq::Socket, deadline: Instant) -> Option<LogbookRecord> {
    while Instant::now() < deadline {
        match log_pull.recv_bytes(0) {
            Ok(raw) => {
                return Some(serde_json::from_slice(&raw).expect("LogPost is a logbook record"))
            }
            Err(zmq::Error::EAGAIN) => continue,
            Err(e) => panic!("log PULL failed: {e}"),
        }
    }
    None
}

/// status PUB を 1 通受けて `slow` ペイロードを取り出す。
fn next_slow_payload(sub: &zmq::Socket, deadline: Instant) -> Option<SlowPayload> {
    while Instant::now() < deadline {
        match sub.recv_bytes(0) {
            Ok(raw) => {
                let message = decode_message(&raw).expect("monitor PUB message");
                assert_eq!(message.source_id, 200, "SPEC §3.2: slowctl = 200");
                return message.slow().cloned();
            }
            Err(zmq::Error::EAGAIN) => continue,
            Err(e) => panic!("SUB failed: {e}"),
        }
    }
    None
}

fn event_of(record: &LogbookRecord) -> (&str, &str, Option<u8>) {
    match record {
        LogbookRecord::Slow {
            event,
            device,
            node,
            ..
        } => (event.as_str(), device.as_str(), *node),
        other => panic!("expected a slow record, got {other:?}"),
    }
}

fn fmeasure_of(record: &LogbookRecord) -> f64 {
    match record {
        LogbookRecord::Slow { values, .. } => values["fmeasure"]["value"].as_f64().unwrap(),
        other => panic!("expected a slow record, got {other:?}"),
    }
}

// ---------------------------------------------------------------------
// テスト
// ---------------------------------------------------------------------

/// 発注書 §2.6 テスト節の ①〜④ を 1 本の run で通す(プロセス起動は 1 回で足りる)。
#[test]
fn the_slowctl_process_publishes_samples_and_posts_ident_value_alarm_and_recover() {
    let dir = scratch_dir("e2e");
    let script = dir.join("gas_script.txt");
    let mut file = std::fs::File::create(&script).unwrap();
    file.write_all(SCRIPT.as_bytes()).unwrap();
    drop(file);

    let pub_port = free_port();
    let log_port = free_port();
    let config = write_config(&dir, &script, pub_port, log_port);

    let context = zmq::Context::new();
    let (log_pull, sub) = open_sockets(&context, pub_port, log_port);

    let child = Command::new(env!("CARGO_BIN_EXE_slowctl"))
        .args(["--config", &config.to_string_lossy()])
        .env("RUST_LOG", "info")
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn slowctl");
    let _guard = Proc(child);

    let deadline = Instant::now() + DEADLINE;

    // ② LogPost は **IDENT が先頭**(起動時の識別)。
    let first = next_log_record(&log_pull, deadline).expect("IDENT within the deadline");
    let (event, device, node) = event_of(&first);
    assert_eq!(event, "IDENT");
    assert_eq!(device, "gas");
    assert_eq!(node, Some(3), "FLOW-BUS ノード番号がそのまま載る");
    match &first {
        LogbookRecord::Slow { values, .. } => {
            assert_eq!(
                values["capacity_unit"], "mbar",
                "単位は機器から読んだ文字列"
            );
            assert!(values["serial"].is_string());
        }
        other => panic!("expected a slow record, got {other:?}"),
    }

    // ② の続き + ③: IDENT のあとは VALUE → ALARM → RECOVER の順で届く。
    let mut events = Vec::new();
    let mut alarm_value = None;
    let mut recover_value = None;
    while Instant::now() < deadline && !events.contains(&"RECOVER".to_string()) {
        let Some(record) = next_log_record(&log_pull, deadline) else {
            break;
        };
        let (event, _, _) = event_of(&record);
        match event {
            "ALARM" if alarm_value.is_none() => alarm_value = Some(fmeasure_of(&record)),
            "RECOVER" if recover_value.is_none() => recover_value = Some(fmeasure_of(&record)),
            _ => {}
        }
        events.push(event.to_string());
    }
    assert!(
        events.first().map(String::as_str) == Some("VALUE"),
        "IDENT の次は VALUE のはず: {events:?}"
    );
    assert!(events.contains(&"ALARM".to_string()), "{events:?}");
    assert!(events.contains(&"RECOVER".to_string()), "{events:?}");
    assert!(
        events.iter().position(|e| e == "ALARM") < events.iter().position(|e| e == "RECOVER"),
        "ALARM → RECOVER の順: {events:?}"
    );
    // ③ 跨いだ値そのもの(スクリプトの 120 で出て、100 で戻る)。
    assert_eq!(alarm_value, Some(120.0));
    assert_eq!(recover_value, Some(100.0));
    assert!(120.0 > ALARM_HI && (ALARM_LO..=ALARM_HI).contains(&100.0));

    // ① status PUB に kind=`slow` が来て fmeasure が載っている。
    // ④ 無応答 2 回ぶんが poll_err に立ち、復帰後も poll_ok が伸び続ける。
    let mut saw_fmeasure = false;
    let mut last: Option<SlowPayload> = None;
    while Instant::now() < deadline {
        let Some(payload) = next_slow_payload(&sub, deadline) else {
            break;
        };
        assert_eq!(payload.kind, "slow");
        if let Some(sample) = payload.samples.iter().find(|s| s.param == "fmeasure") {
            saw_fmeasure = true;
            assert_eq!(sample.device, "gas");
            assert_eq!(sample.node, Some(3));
            assert_eq!(sample.unit, "mbar");
        }
        let done = payload.counters.poll_err >= 2;
        last = Some(payload);
        if done && saw_fmeasure {
            break;
        }
    }
    let payload = last.expect("at least one slow payload");
    assert!(saw_fmeasure, "samples に fmeasure が無い: {payload:?}");
    assert_eq!(
        payload.counters.poll_err, 2,
        "スクリプトの `silent` 2 行ぶんだけタイムアウトする: {:?}",
        payload.counters
    );
    assert_eq!(payload.counters.parse_err, 0);
    assert_eq!(payload.counters.identity_mismatch, 0);
    assert!(payload.counters.poll_ok > 0);

    // ④ 復帰後に VALUE が再開する(スクリプト末尾 105.0 が最新値として載る)。
    let recovery_deadline = Instant::now() + DEADLINE;
    let mut resumed = false;
    while Instant::now() < recovery_deadline {
        let Some(payload) = next_slow_payload(&sub, recovery_deadline) else {
            break;
        };
        if payload
            .samples
            .iter()
            .any(|s| s.param == "fmeasure" && (s.value - 105.0).abs() < 1e-9)
        {
            resumed = true;
            break;
        }
    }
    assert!(resumed, "無応答から復帰したあとの値が PUB に出ていない");

    let _ = std::fs::remove_dir_all(&dir);
}

/// `[slowctl]` の無い設定では**何もせずに正常終了**する(既存 TOML を壊さない)。
#[test]
fn a_config_without_a_slowctl_section_exits_cleanly() {
    let dir = scratch_dir("nosection");
    let script = dir.join("unused.txt");
    std::fs::write(&script, b"100.0\n").unwrap();
    let config = write_config(&dir, &script, free_port(), free_port());
    let text = std::fs::read_to_string(&config).unwrap();
    // `[slowctl]` 以降を丸ごと落とす。
    let cut = text.find("[slowctl]").expect("the fixture has a [slowctl]");
    std::fs::write(&config, &text[..cut]).unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_slowctl"))
        .args(["--config", &config.to_string_lossy()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("run slowctl");
    assert!(status.success(), "exit status = {status:?}");

    let _ = std::fs::remove_dir_all(&dir);
}
