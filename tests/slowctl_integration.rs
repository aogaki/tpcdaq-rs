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
use tpcdaq::slowctl::fake_hv::FakeHv;
use tpcdaq::slowctl::hivolta::{mon_bd, mon_ch, BdPar, ChPar, ALL_CH};
use tpcdaq::slowctl::SlowPayload;

/// 1 本の観測にかける上限。poll は 20 Hz なのでスクリプト全体は 1 秒未満で流れる。
const DEADLINE: Duration = Duration::from_secs(20);

/// しきい値(TOML と同じ値をテスト側にも置いて、オラクルを 1 箇所にまとめない)。
const ALARM_LO: f64 = 90.0;
const ALARM_HI: f64 = 110.0;

/// TODO/088: 壁時計固定の `sleep` の代わりに使う条件待ちの上限(発注書 §1)。
const CONDITION_DEADLINE: Duration = Duration::from_secs(10);

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

/// controller 役の PULL だけを開く(`bind_log_pull` を分けてあるのは、後から上げる
/// 起動順のテストが**わざと開かずに** slowctl を先に立てるため)。
fn bind_log_pull(context: &zmq::Context, log_port: u16) -> zmq::Socket {
    let log_pull = context.socket(zmq::PULL).unwrap();
    log_pull.set_rcvtimeo(200).unwrap();
    log_pull.set_linger(0).unwrap();
    log_pull
        .bind(&format!("tcp://127.0.0.1:{log_port}"))
        .unwrap();
    log_pull
}

/// テスト側で controller 役の PULL と monitor 役の SUB を先に開く(listen-before-start)。
fn open_sockets(
    context: &zmq::Context,
    pub_port: u16,
    log_port: u16,
) -> (zmq::Socket, zmq::Socket) {
    let log_pull = bind_log_pull(context, log_port);

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

/// **実運用の起動順**(controller が最後 — docs/NEW_DAQ_SETUP_ja.md)で IDENT を落とさない。
///
/// PULL を **bind せずに** slowctl を起動 → スクリプトが流れ切ったことを status PUB の
/// `counters.poll_ok` で確認してから bind → 1 通目が IDENT。
/// 再送キューは投函順なので、停止中に起きた ALARM / RECOVER もその後ろに続く。
/// 停止中の VALUE は再送しない(次の VALUE が来る)= 古い値で logbook を埋めない。
///
/// TODO/088: 元は固定 `sleep(1s)` だった(poll_hz = 20 でスクリプト 8 行は 0.4 s で流れ切る、
/// という壁時計前提)。全体テスト負荷下ではプロセス起動やスケジューリングが 1 s を超えることが
/// あり、まれに赤くなっていた。status PUB(SPEC §2.3、1 Hz 集約)の `poll_ok` が
/// スクリプト行数以上になるまで**条件待ち**することで、実際に流れ切ったことを直接確認する。
#[test]
fn records_from_before_the_controller_was_up_arrive_in_order_starting_with_ident() {
    let dir = scratch_dir("late-controller");
    let script = dir.join("gas_script.txt");
    std::fs::write(&script, SCRIPT.as_bytes()).unwrap();
    let script_lines = SCRIPT.lines().count() as u64;

    let pub_port = free_port();
    let log_port = free_port();
    let config = write_config(&dir, &script, pub_port, log_port);

    let context = zmq::Context::new();
    // status PUB は slowctl を起動する前に SUB しておく(slow-joiner で最初の 1 通を
    // 取りこぼさないため — listen-before-start と同じ流儀)。
    // **LogPost の PULL はまだ開かない**(controller が居ない状態で slowctl が先に立つ)。
    let sub = context.socket(zmq::SUB).unwrap();
    sub.set_subscribe(b"").unwrap();
    sub.set_rcvtimeo(200).unwrap();
    sub.set_linger(0).unwrap();
    sub.connect(&format!("tcp://127.0.0.1:{pub_port}")).unwrap();

    let child = Command::new(env!("CARGO_BIN_EXE_slowctl"))
        .args(["--config", &config.to_string_lossy()])
        .env("RUST_LOG", "warn")
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn slowctl");
    let _guard = Proc(child);

    // 固定 sleep の代わりに条件待ち: status PUB の poll_ok がスクリプト行数以上になれば、
    // スクリプトは(controller 不在のまま)最後まで流れ切っている。
    let wait_deadline = Instant::now() + CONDITION_DEADLINE;
    let mut drained = false;
    while Instant::now() < wait_deadline {
        let Some(payload) = next_slow_payload(&sub, wait_deadline) else {
            break;
        };
        if payload.counters.poll_ok >= script_lines {
            drained = true;
            break;
        }
    }
    assert!(
        drained,
        "status PUB の poll_ok がスクリプト行数({script_lines})に届かない(スクリプトが流れ切っていない)"
    );

    let log_pull = bind_log_pull(&context, log_port);
    let deadline = Instant::now() + CONDITION_DEADLINE;

    let mut events = Vec::new();
    while events.len() < 3 {
        let Some(record) = next_log_record(&log_pull, deadline) else {
            break;
        };
        let (event, device, _) = event_of(&record);
        assert_eq!(device, "gas");
        events.push(event.to_string());
    }
    assert_eq!(
        events,
        vec!["IDENT", "ALARM", "RECOVER"],
        "controller が後から上がっても 1 通目は IDENT、以後は投函順"
    );

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

// ---------------------------------------------------------------------
// TODO/122: `[slowctl.influx]` —— 全サンプルを InfluxDB v2 write API へ
// ---------------------------------------------------------------------

/// 偽 InfluxDB が受けた要求 1 本の (要求行, Authorization, 本文)。
type InfluxRequest = (String, String, String);

/// 1 接続 1 要求で受け、記録して 204 を返す偽 InfluxDB(`TcpListener` だけで組む)。
fn fake_influx(listener: TcpListener) -> std::sync::mpsc::Receiver<InfluxRequest> {
    use std::io::{BufRead, BufReader, Read};
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { return };
            let mut reader = BufReader::new(&stream);
            let mut request_line = String::new();
            if reader.read_line(&mut request_line).is_err() {
                continue;
            }
            let mut authorization = String::new();
            let mut length = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    break;
                }
                let line = line.trim_end();
                if line.is_empty() {
                    break;
                }
                if let Some((name, value)) = line.split_once(':') {
                    match name.trim().to_ascii_lowercase().as_str() {
                        "authorization" => authorization = value.trim().to_string(),
                        "content-length" => length = value.trim().parse().unwrap_or(0),
                        _ => {}
                    }
                }
            }
            let mut body = vec![0; length];
            if reader.read_exact(&mut body).is_err() {
                continue;
            }
            let _ = (&stream).write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
            let request = (
                request_line.trim_end().to_string(),
                authorization,
                String::from_utf8_lossy(&body).into_owned(),
            );
            if tx.send(request).is_err() {
                return;
            }
        }
    });
    rx
}

/// `[slowctl.influx]` があれば、読み出した値が line protocol で偽 InfluxDB に届き、
/// status PUB の counters に `influx_points_written` が出る(配線の端から端まで)。
#[test]
fn with_an_influx_section_samples_reach_influxdb_and_the_counters_show_it() {
    let dir = scratch_dir("influx");
    let script = dir.join("gas_script.txt");
    std::fs::write(&script, SCRIPT.as_bytes()).unwrap();
    let token = dir.join("influx.token");
    std::fs::write(&token, b"  tok-e2e-122\n").unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let influx_addr = listener.local_addr().unwrap();
    let requests = fake_influx(listener);

    let pub_port = free_port();
    let log_port = free_port();
    let config = write_config(&dir, &script, pub_port, log_port);
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&config)
        .unwrap();
    write!(
        file,
        "\n[slowctl.influx]\nurl = \"http://{influx_addr}\"\norg = \"elinp\"\n\
         bucket = \"slowctl\"\ntoken_file = \"{}\"\nbatch_ms = 100\n",
        token.display()
    )
    .unwrap();
    drop(file);

    let context = zmq::Context::new();
    let (_log_pull, sub) = open_sockets(&context, pub_port, log_port);

    let child = Command::new(env!("CARGO_BIN_EXE_slowctl"))
        .args(["--config", &config.to_string_lossy()])
        .env("RUST_LOG", "warn")
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn slowctl");
    let _guard = Proc(child);

    // スクリプト 1 行目の 100.0 が `gas,ch=3 … fmeasure=100 …` で届く(node 3 = ch タグ)。
    let deadline = Instant::now() + CONDITION_DEADLINE;
    let mut seen = Vec::new();
    let mut found = None;
    while found.is_none() && Instant::now() < deadline {
        let Ok(request) = requests.recv_timeout(Duration::from_millis(200)) else {
            continue;
        };
        if request
            .2
            .lines()
            .any(|l| l.starts_with("gas,ch=3 ") && l.contains("fmeasure=100,"))
        {
            found = Some(request);
        } else {
            seen.push(request);
        }
    }
    let (request_line, authorization, body) =
        found.unwrap_or_else(|| panic!("no fmeasure=100 line reached InfluxDB: {seen:?}"));
    assert_eq!(
        request_line,
        "POST /api/v2/write?org=elinp&bucket=slowctl&precision=ms HTTP/1.1"
    );
    assert_eq!(
        authorization, "Token tok-e2e-122",
        "token_file は trim して読む"
    );
    assert!(
        body.lines().all(|l| l.starts_with("gas,ch=3 ")),
        "IDENT(文字列)は書かない — 行はサンプルだけ: {body:?}"
    );

    // status PUB の counters にも出る。
    let mut written = 0;
    while written == 0 && Instant::now() < deadline {
        let Some(payload) = next_slow_payload(&sub, deadline) else {
            break;
        };
        written = payload.counters.influx_points_written;
        assert_eq!(payload.counters.influx_write_errors, 0);
        assert_eq!(payload.counters.influx_last_error, None);
    }
    assert!(
        written > 0,
        "influx_points_written が status PUB に出ていない"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// レビュー 2026-10-12 (a): トークンファイルが無い daq.toml でも `config::load` は通る
/// (= 同じ TOML を読む controller / monitor は起動できる)。起動に失敗するのは slowctl だけ。
#[test]
fn a_missing_influx_token_fails_only_the_slowctl_startup() {
    let dir = scratch_dir("influx-no-token");
    let script = dir.join("gas_script.txt");
    std::fs::write(&script, SCRIPT.as_bytes()).unwrap();
    let missing_token = dir.join("no-such.token");

    let config = write_config(&dir, &script, free_port(), free_port());
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&config)
        .unwrap();
    write!(
        file,
        "\n[slowctl.influx]\nurl = \"http://127.0.0.1:8086\"\norg = \"elinp\"\n\
         bucket = \"slowctl\"\ntoken_file = \"{}\"\n",
        missing_token.display()
    )
    .unwrap();
    drop(file);

    // controller / monitor と同じ入口(config::load)は通る。
    let loaded = tpcdaq::config::load(&config).expect("the config must still load");
    assert!(loaded.slowctl.unwrap().influx.is_some());

    // slowctl だけが起動エラーで終わる(待ち続けない —— 期限付きで終了を見る)。
    let child = Command::new(env!("CARGO_BIN_EXE_slowctl"))
        .args(["--config", &config.to_string_lossy()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn slowctl");
    let mut guard = Proc(child);
    let deadline = Instant::now() + CONDITION_DEADLINE;
    let status = loop {
        if let Some(status) = guard.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "slowctl kept running without a readable token_file"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(!status.success(), "exit status = {status:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------
// TODO/124: power ON/OFF の REP(実バイナリ + TCP で話す偽 DT1415ET)
// ---------------------------------------------------------------------

/// 偽 DT1415ET(`FakeHv`)を TCP(:1470 の代わり)で話させる。接続は 1 本ずつ順に受ける。
/// 中身はライブラリの `FakeHv` そのもの(ON / OFF で内部の STATUS が変わる — Fable 9)。
fn fake_hv_over_tcp(mut fake: FakeHv) -> u16 {
    use std::io::Read;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut buf = [0u8; 256];
            loop {
                let n = match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                fake.write_all(&buf[..n]).unwrap();
                let mut out = Vec::new();
                let mut chunk = [0u8; 64];
                while let Ok(m) = fake.read(&mut chunk) {
                    if m == 0 {
                        break;
                    }
                    out.extend_from_slice(&chunk[..m]);
                }
                if !out.is_empty() && stream.write_all(&out).is_err() {
                    break;
                }
            }
        }
    });
    port
}

/// ident と poll(全 ch 一括)に答える偽 DT1415ET。STATUS は内部の値(ch5 は DIS = 4096)。
fn fake_dt1415et() -> FakeHv {
    let mut fake = FakeHv::new();
    fake.set_response(mon_bd(BdPar::Name), "#CMD:OK,VAL:DT1415ET\r\n");
    fake.set_response(mon_bd(BdPar::Nch), "#CMD:OK,VAL:8\r\n");
    fake.set_response(mon_bd(BdPar::Frel), "#CMD:OK,VAL:1.5\r\n");
    fake.set_response(mon_bd(BdPar::Snum), "#CMD:OK,VAL:SN-124\r\n");
    fake.set_response(mon_bd(BdPar::Ctr), "#CMD:OK,VAL:REMOTE\r\n");
    fake.set_response(mon_bd(BdPar::IlkMode), "#CMD:OK,VAL:DISABLE\r\n");
    let eight = |value: &str| format!("#CMD:OK,VAL:{}\r\n", [value; 8].join(";"));
    fake.set_response(mon_ch(ALL_CH, ChPar::Vmon), eight("0000.00"));
    fake.set_response(mon_ch(ALL_CH, ChPar::Imon), eight("+0000.000"));
    fake.set_response(mon_ch(ALL_CH, ChPar::Vset), eight("0350.00"));
    fake.set_response(mon_ch(ALL_CH, ChPar::Iset), eight("0015.00"));
    fake.set_response(mon_bd(BdPar::Alarm), "#CMD:OK,VAL:0\r\n");
    fake.set_response(mon_bd(BdPar::Ilk), "#CMD:OK,VAL:NO\r\n");
    fake.set_status(5, 4096);
    fake
}

/// controller と同じ使い方: 1 要求ごとに REQ を作って 1 通受ける(来なければ panic)。
fn power_request(context: &zmq::Context, cmd_port: u16, body: &str) -> serde_json::Value {
    let req = context.socket(zmq::REQ).unwrap();
    req.set_rcvtimeo(10_000).unwrap();
    req.set_linger(0).unwrap();
    req.connect(&format!("tcp://127.0.0.1:{cmd_port}")).unwrap();
    req.send(body, 0).unwrap();
    let raw = req
        .recv_bytes(0)
        .expect("the REP answers every request once");
    serde_json::from_slice(&raw).expect("the reply is JSON")
}

/// 発注書 §1 の REP 契約を実バイナリで: ON → OFF の before / after、前面スイッチ OFF(DIS)の
/// 拒否、不正入力(device 無し・ch 範囲外・未知 op・power の無い型・壊れた JSON)にも 1 通ずつ。
#[test]
fn the_power_rep_switches_a_hv_channel_and_refuses_bad_requests() {
    let dir = scratch_dir("power-rep");
    let script = dir.join("gas_script.txt");
    std::fs::write(&script, b"100.0\n").unwrap();
    let hv_port = fake_hv_over_tcp(fake_dt1415et());
    let cmd_port = free_port();

    let config = write_config(&dir, &script, free_port(), free_port());
    let text = std::fs::read_to_string(&config).unwrap().replace(
        "[slowctl]\n",
        &format!("[slowctl]\ncmd_bind = \"tcp://127.0.0.1:{cmd_port}\"\n"),
    );
    let text = format!(
        "{text}\n[[slowctl.device]]\ntype = \"hivolta\"\nname = \"hv\"\n\
         addr = \"127.0.0.1:{hv_port}\"\n"
    );
    std::fs::write(&config, text).unwrap();

    let child = Command::new(env!("CARGO_BIN_EXE_slowctl"))
        .args(["--config", &config.to_string_lossy()])
        .env("RUST_LOG", "warn")
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn slowctl");
    let _guard = Proc(child);
    let context = zmq::Context::new();

    // 起動直後の接続待ちの間は `device not connected` がありうる —— 識別されるまで繰り返す。
    let on = r#"{"op":"power","device":"hv","ch":3,"on":true}"#;
    let deadline = Instant::now() + CONDITION_DEADLINE;
    let reply = loop {
        let reply = power_request(&context, cmd_port, on);
        if reply["error"] != "device not connected" || Instant::now() > deadline {
            break reply;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(
        reply,
        serde_json::json!({"ok": true, "device": "hv", "ch": 3, "on": true,
                           "before": {"status": 0, "flags": []},
                           "after": {"status": 1, "flags": ["ON"]}})
    );

    let off = r#"{"op":"power","device":"hv","ch":3,"on":false}"#;
    assert_eq!(
        power_request(&context, cmd_port, off),
        serde_json::json!({"ok": true, "device": "hv", "ch": 3, "on": false,
                           "before": {"status": 1, "flags": ["ON"]},
                           "after": {"status": 0, "flags": []}})
    );

    // ch5 は前面スイッチ OFF(DIS): SET を送らずに拒否、before は載る。
    assert_eq!(
        power_request(
            &context,
            cmd_port,
            r#"{"op":"power","device":"hv","ch":5,"on":true}"#
        ),
        serde_json::json!({"ok": false, "error": "front switch is OFF (DIS)",
                           "before": {"status": 4096, "flags": ["DIS"]}})
    );

    for (body, reason) in [
        (
            r#"{"op":"power","device":"nope","ch":3,"on":true}"#,
            "unknown device",
        ),
        (
            r#"{"op":"power","device":"hv","ch":8,"on":true}"#,
            "out of range",
        ),
        (
            r#"{"op":"reboot","device":"hv","ch":3,"on":true}"#,
            "unknown op",
        ),
        (
            r#"{"op":"power","device":"gas","ch":3,"on":true}"#,
            "no power on/off",
        ),
        ("{not json", "bad request"),
    ] {
        let reply = power_request(&context, cmd_port, body);
        assert_eq!(reply["ok"], false, "{body}");
        assert_eq!(reply["before"], serde_json::Value::Null, "{body}");
        let error = reply["error"].as_str().unwrap();
        assert!(error.contains(reason), "{body}: error = {error:?}");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// Fable 8: `cmd_bind` を書かなければ REP は開かない —— 起動済みの slowctl が既定の 47007 を
/// 掴んでいないことを見る(起動前から 47007 が使われている環境では判定できないので飛ばす)。
#[test]
fn without_cmd_bind_the_power_rep_is_not_opened() {
    let default_rep: std::net::SocketAddr = ([127, 0, 0, 1], 47007).into();
    let listening =
        || std::net::TcpStream::connect_timeout(&default_rep, Duration::from_millis(300)).is_ok();
    if listening() {
        eprintln!("skip: something already listens on 127.0.0.1:47007");
        return;
    }

    let dir = scratch_dir("no-cmd-bind");
    let script = dir.join("gas_script.txt");
    std::fs::write(&script, b"100.0\n").unwrap();
    let pub_port = free_port();
    let log_port = free_port();
    let config = write_config(&dir, &script, pub_port, log_port);
    assert!(!std::fs::read_to_string(&config)
        .unwrap()
        .contains("cmd_bind"));

    let context = zmq::Context::new();
    let (_log_pull, sub) = open_sockets(&context, pub_port, log_port);
    let child = Command::new(env!("CARGO_BIN_EXE_slowctl"))
        .args(["--config", &config.to_string_lossy()])
        .env("RUST_LOG", "warn")
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn slowctl");
    let _guard = Proc(child);

    // status PUB が届いた = 起動は済んでいる(REP を開くなら PUB より前に開いている)。
    let deadline = Instant::now() + CONDITION_DEADLINE;
    assert!(next_slow_payload(&sub, deadline).is_some(), "slowctl is up");
    assert!(!listening(), "cmd_bind 省略なのに 47007 が開いている");

    let _ = std::fs::remove_dir_all(&dir);
}
