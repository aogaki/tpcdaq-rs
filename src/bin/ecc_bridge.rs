//! ecc_bridge —— ECC(GET の run 制御 Ice サーバ)への薄い JSON REQ/REP ブリッジ。
//! SPEC §8.2 / §3.2(REP = `tcp://*:47200`)、TODO/017 → **076 Phase 2 で Rust 化**。
//!
//! ```text
//! controller(Rust) --ZMQ REQ/REP(JSON)--> ecc_bridge --Ice(encoding 1.1)--> ECC
//! ```
//!
//! * リクエスト: `{"action":"describe|prepare|configure|start|stop|breakup|reset|status",
//!   "config_id":"...", "links":[{"sender":"CoBo[0]","router_ip":"...","router_port":46005,
//!   "type":"TCP"}, ...]}`
//! * レスポンス: `{"ok":bool,"state":"Off|Idle|Described|Prepared|Ready|Running|Paused|Unknown",
//!   "error":"...","ecc_error":"NO_ERR|WHEN_DESCRIBE|...|Unknown"}`
//!   `error` は**輸送・ブリッジ層**のエラー、`ecc_error` は **GET の error フラグ**
//!   (043、SPEC §8.2 v1.14)。
//! * **never panic**: Ice の失敗は全部 `Result` 化。ECC 不達 = `{"ok":false,"state":"Unknown"}`。
//! * REP は 1 スレッド逐次(コマンドは人間スケール)。**必ず 1 リクエスト 1 レスポンス**を返す
//!   —— 返さないと REQ 側が永久に待つ。
//!
//! C++ 版(`tools/ecc_bridge/ecc_bridge.cpp`)は 076 Phase 3 で撤去済み(fake_ecc /
//! ecc_e2e_client は残る)。CLI と stdout の `BIND <endpoint>` 1 行は**そのまま**
//! (`run_ecc_e2e.sh` と Rust 統合テストが見ている)。ログは他の Rust bin と同じ
//! `RUST_LOG`(既定 `info`)で絞れる `tracing`(076 Phase 3 で `eprintln!` から移行)。
//! **`tpcdaq::bin_support::init_tracing` は使わない**——既定の書き込み先が stdout で、
//! それだと `BIND` 1 行きり(SPEC §8.2)の契約を壊す(実測: `info!` が BIND の直後に
//! stdout へ混ざる)ので、[`init_tracing_to_stderr`] で明示的に stderr へ向ける。
//!
//! **SIGINT/SIGTERM(076 Phase 3 追補)**: 実機(GDED)で SIGINT が効かず SIGTERM でしか
//! 止まらない事象を確認(ZMQ `recv` でブロックしたまま既定のシグナル動作を待っていたと
//! 推定)。他の Rust bin は SIGINT で止まる運用(`daq_down.sh`)なので揃える —— この bin は
//! `tokio` を起動しない同期 REP ループなので `tpcdaq::bin_support::spawn_sigint`(tokio 版)は
//! 使えず、代わりに `signal-hook`(唯一の追加依存)で `AtomicBool` を立てるだけの
//! async-signal-safe なハンドラを登録し、REP の `recv` を `ZMQ_RCVTIMEO` 付きにして
//! 定期的にフラグを見る(最大 500 ms 遅延で応答)。

use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use signal_hook::consts::{SIGINT, SIGTERM};
use tpcdaq::ecc::core;
use tpcdaq::ecc::proxy::{EccProxy, Observed, CONNECT_TIMEOUT};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

/// REP の `recv` がタイムアウト(`EAGAIN`/`EINTR`)したとき、ループを抜けるかどうか。
/// ZMQ に依存しない純粋関数 —— signal-hook 無しでも「終了フラグを見て 1 周で抜ける」を
/// 検査できる(076 Phase 3 追補)。
fn should_stop_after_timeout(stopping: &AtomicBool) -> bool {
    stopping.load(Ordering::Relaxed)
}

/// 他の Rust bin と同じ `RUST_LOG`(既定 `info`)フィルタだが、**stderr へ書く**
/// (`bin_support::init_tracing` は stdout に書くため、この bin にだけは使えない)。
fn init_tracing_to_stderr() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
}

/// REP の recv タイムアウト(SIGINT/SIGTERM をポーリングする周期、076 Phase 3 追補)。
const RECV_POLL_MS: i32 = 500;

struct Options {
    /// ZMQ REP の bind(SPEC §3.2)。
    bind: String,
    /// ECC 状態機械の Ice プロキシ文字列(SPEC §3.1 の既定。identity は実測で `Ecc` — TODO/038)。
    ecc_proxy: String,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            bind: "tcp://*:47200".to_string(),
            ecc_proxy: "Ecc:tcp -h 127.0.0.1 -p 46002".to_string(),
        }
    }
}

/// コマンドラインの読み取り結果(C++ の `main` の 3 分岐そのまま)。
enum Cli {
    Run(Options),
    Help,
    /// 使い方の誤り(理由つき)。exit 2。
    Bad(String),
}

const USAGE: &str = "\
usage: ecc_bridge [--bind ENDPOINT] [--ecc-proxy PROXY]

  --bind ENDPOINT   ZMQ REP bind (default tcp://*:47200)
  --ecc-proxy PROXY Ice proxy of the ECC state machine
                    (default \"Ecc:tcp -h 127.0.0.1 -p 46002\")

Prints one line \"BIND <endpoint>\" on stdout once the REP socket is bound
(so a test can use an ephemeral port), then serves JSON requests (SPEC 8.2)
until SIGINT/SIGTERM.
";

fn parse_args(args: &[String]) -> Cli {
    let mut options = Options::default();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        let value = args.get(i + 1);
        match (arg, value) {
            ("--help" | "-h", _) => return Cli::Help,
            ("--bind", Some(v)) => {
                options.bind = v.clone();
                i += 2;
            }
            ("--ecc-proxy", Some(v)) => {
                options.ecc_proxy = v.clone();
                i += 2;
            }
            _ => return Cli::Bad(format!("bad argument '{arg}'")),
        }
    }
    Cli::Run(options)
}

/// 応答に載せる状態。取得できなければ `Unknown`/`Unknown` +
/// **理由を stderr に残す**(C++ `EccClient::observe` は捨てていたが、
/// 黙って捨てないのが CLAUDE.md の作法。応答の `error` は直前の操作のものが正)。
fn observe(ecc: &mut EccProxy) -> Observed {
    match ecc.get_status() {
        Ok(observed) => observed,
        Err(e) => {
            error!(error = %e, "getStatus failed");
            Observed::default()
        }
    }
}

/// action を ECC 呼び出しに割り付ける(`status` は [`handle`] が直接扱う)。
fn dispatch(ecc: &mut EccProxy, request: &core::Request) -> Result<(), String> {
    match request.action.as_str() {
        "describe" => ecc.describe(&request.config_id),
        "prepare" => ecc.prepare(&request.config_id),
        "configure" => ecc.configure(&request.config_id, &core::data_link_set_xml(&request.links)),
        "start" => ecc.start(),
        "stop" => ecc.stop(),
        "breakup" => ecc.breakup(),
        "reset" => ecc.reset(),
        // parse_request が弾くので到達しない。
        other => Err(format!("unknown action: {other}")),
    }
}

/// リクエスト 1 本 → レスポンス 1 本。**絶対に応答を返す**(返さないと REQ が永久に待つ)。
fn handle(ecc: &mut EccProxy, request_json: &str) -> String {
    let request = match core::parse_request(request_json) {
        Ok(request) => request,
        Err(reason) => {
            // 壊れたリクエストでも ECC の状態は返す(状態が見えないと運用が止まる)。
            warn!(reason = %reason, "rejected request");
            let observed = observe(ecc);
            return core::make_response(false, observed.state, &reason, observed.error);
        }
    };

    // status は「状態が取れたか」がそのまま ok(ECC 不達なら ok=false + Unknown)。
    if request.action == "status" {
        let (ok, observed, error) = match ecc.get_status() {
            Ok(observed) => (true, observed, String::new()),
            Err(e) => (false, Observed::default(), e),
        };
        info!(
            ok,
            state = %observed.state,
            ecc_error = %observed.error,
            error = %error,
            "status"
        );
        return core::make_response(ok, observed.state, &error, observed.error);
    }

    let result = dispatch(ecc, &request);
    // `ok` は**呼び出しが成功したか**であって「理由の文字列が空か」ではない
    // (reason が空の CmdException でも失敗は失敗 —— C++ の `Result::ok` と同じ)。
    let ok = result.is_ok();
    let observed = observe(ecc);
    let error = result.err().unwrap_or_default();
    info!(
        action = %request.action,
        ok,
        state = %observed.state,
        ecc_error = %observed.error,
        error = %error,
        "action"
    );
    core::make_response(ok, observed.state, &error, observed.error)
}

fn main() -> ExitCode {
    init_tracing_to_stderr();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match parse_args(&args) {
        Cli::Run(options) => options,
        Cli::Help => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Cli::Bad(reason) => {
            error!(reason = %reason, "bad arguments");
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    // プロキシ文字列は **bind の前に**検査する(誰も待っていないうちに落ちる)。
    let mut ecc = match EccProxy::from_proxy_string(&options.ecc_proxy, CONNECT_TIMEOUT) {
        Ok(ecc) => ecc,
        Err(reason) => {
            error!(reason = %reason, "bad --ecc-proxy");
            return ExitCode::from(2);
        }
    };

    // SIGINT/SIGTERM は「フラグを立てるだけ」(signal-hook の async-signal-safe な既製ハンドラ)。
    // REP の recv がタイムアウト付きで定期的にこのフラグを見る(下記ループ)。
    let stopping = Arc::new(AtomicBool::new(false));
    if let Err(e) = signal_hook::flag::register(SIGINT, Arc::clone(&stopping)) {
        error!(error = %e, "cannot register SIGINT handler");
        return ExitCode::FAILURE;
    }
    if let Err(e) = signal_hook::flag::register(SIGTERM, Arc::clone(&stopping)) {
        error!(error = %e, "cannot register SIGTERM handler");
        return ExitCode::FAILURE;
    }

    let context = zmq::Context::new();
    let rep = match context.socket(zmq::REP) {
        Ok(socket) => socket,
        Err(e) => {
            error!(error = %e, "cannot create the REP socket");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = rep.set_linger(0) {
        error!(error = %e, "cannot set ZMQ_LINGER");
        return ExitCode::FAILURE;
    }
    // SIGINT/SIGTERM をポーリングできるよう、recv を無限ブロックにしない(076 Phase 3 追補)。
    if let Err(e) = rep.set_rcvtimeo(RECV_POLL_MS) {
        error!(error = %e, "cannot set ZMQ_RCVTIMEO");
        return ExitCode::FAILURE;
    }
    if let Err(e) = rep.bind(&options.bind) {
        error!(bind = %options.bind, error = %e, "zmq bind failed");
        return ExitCode::FAILURE;
    }

    // 実際に bind したエンドポイント(ポート 0 を指定したときの実ポート)を 1 行で出す。
    let endpoint = match rep.get_last_endpoint() {
        Ok(Ok(resolved)) => resolved,
        _ => options.bind.clone(),
    };
    println!("BIND {endpoint}");
    if let Err(e) = std::io::Write::flush(&mut std::io::stdout()) {
        error!(error = %e, "cannot flush the BIND line");
        return ExitCode::FAILURE;
    }
    info!(ecc_proxy = %options.ecc_proxy, "starting");

    // 起動時に一度だけ繋ぎに行く。失敗しても**起動は続ける**(ECC が後から上がる運用がある)。
    if let Err(e) = ecc.connect() {
        warn!(error = %e, "ECC not reachable yet");
    }
    let mut connected = ecc.is_connected();
    if connected {
        info!(ecc_proxy = %options.ecc_proxy, "connected to ECC (encoding 1.1)");
    }

    loop {
        let request = match rep.recv_bytes(0) {
            Ok(bytes) => bytes,
            // タイムアウト(RECV_POLL_MS 毎)またはシグナルによる中断: 停止フラグが
            // 立っていればここで抜ける、立っていなければ黙って `recv` に戻るだけ
            // (silent ではない —— 通常運転の一部で、ログにする話ではない)。
            Err(zmq::Error::EAGAIN) | Err(zmq::Error::EINTR) => {
                if should_stop_after_timeout(&stopping) {
                    info!("stopping (signal)");
                    break;
                }
                continue;
            }
            Err(e) => {
                error!(error = %e, "recv failed");
                break;
            }
        };
        // UTF-8 でないリクエストも**答える**(parse_request が理由つきで弾く)。
        let request = String::from_utf8_lossy(&request);
        let reply = handle(&mut ecc, &request);
        if let Err(e) = rep.send(reply.as_bytes(), 0) {
            error!(error = %e, "send failed");
            break;
        }
        // 接続の張り直し/切断は運用で読みたいので、変化したときだけ 1 行出す。
        if ecc.is_connected() != connected {
            connected = ecc.is_connected();
            info!(
                connected,
                "ECC connection {}",
                if connected { "established" } else { "dropped" }
            );
        }
    }

    info!("shutting down");
    ExitCode::SUCCESS
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn run_options(list: &[&str]) -> Options {
        match parse_args(&args(list)) {
            Cli::Run(options) => options,
            Cli::Help => panic!("expected Run, got Help"),
            Cli::Bad(reason) => panic!("expected Run, got Bad({reason})"),
        }
    }

    /// 既定値は SPEC §3.2(REP bind)と §3.1(ECC プロキシ)。
    #[test]
    fn the_defaults_are_the_spec_endpoints() {
        let options = run_options(&[]);
        assert_eq!(options.bind, "tcp://*:47200");
        assert_eq!(options.ecc_proxy, "Ecc:tcp -h 127.0.0.1 -p 46002");
    }

    /// `run_ecc_e2e.sh` と Rust 統合テストが渡す形(動的ポート + fake_ecc のプロキシ)。
    #[test]
    fn both_flags_are_read() {
        let options = run_options(&[
            "--bind",
            "tcp://127.0.0.1:*",
            "--ecc-proxy",
            "Ecc:tcp -h 127.0.0.1 -p 58250",
        ]);
        assert_eq!(options.bind, "tcp://127.0.0.1:*");
        assert_eq!(options.ecc_proxy, "Ecc:tcp -h 127.0.0.1 -p 58250");
    }

    #[test]
    fn help_is_requested_by_both_spellings() {
        assert!(matches!(parse_args(&args(&["--help"])), Cli::Help));
        assert!(matches!(parse_args(&args(&["-h"])), Cli::Help));
        // 他の引数の後ろでも効く。
        assert!(matches!(
            parse_args(&args(&["--bind", "tcp://*:1", "--help"])),
            Cli::Help
        ));
    }

    /// 知らない引数と**値の無いフラグ**は使い方の誤り(黙って既定値で走らない)。
    #[test]
    fn unknown_arguments_and_missing_values_are_rejected() {
        for bad in [
            vec!["--nope"],
            vec!["--bind"],
            vec!["--ecc-proxy"],
            vec!["tcp://*:47200"],
        ] {
            match parse_args(&args(&bad)) {
                Cli::Bad(reason) => assert!(!reason.is_empty(), "{bad:?}"),
                _ => panic!("accepted bad arguments: {bad:?}"),
            }
        }
    }

    /// 誰も listen していないポートを ECC のふりで指した [`EccProxy`]。
    fn unreachable_proxy() -> EccProxy {
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let dead = probe.local_addr().unwrap();
        drop(probe);
        EccProxy::from_proxy_string(
            &format!("Ecc:tcp -h 127.0.0.1 -p {}", dead.port()),
            CONNECT_TIMEOUT,
        )
        .unwrap()
    }

    /// ECC 不達は panic ではなく `{"ok":false,"state":"Unknown","ecc_error":"Unknown"}`
    /// (SPEC §8.2)。**`NO_ERR` と言い切らない** —— 「エラー無しと申告した」と
    /// 「届いていない」は別物(043)。
    #[test]
    fn an_unreachable_ecc_answers_unknown_instead_of_dying() {
        let mut ecc = unreachable_proxy();
        for request in [
            r#"{"action":"status"}"#,
            r#"{"action":"describe"}"#,
            r#"{"action":"start"}"#,
        ] {
            let reply: Value = serde_json::from_str(&handle(&mut ecc, request)).unwrap();
            assert_eq!(reply["ok"], Value::Bool(false), "{request}");
            assert_eq!(reply["state"], Value::String("Unknown".into()), "{request}");
            assert_eq!(
                reply["ecc_error"],
                Value::String("Unknown".into()),
                "{request}"
            );
            assert!(
                !reply["error"].as_str().unwrap_or_default().is_empty(),
                "失敗の理由を残さない応答を返さない: {reply}"
            );
        }
    }

    /// 壊れたリクエストでもブリッジは答える(REQ を永久に待たせない)。
    /// 理由は `error` に載り、`ok` は false。
    #[test]
    fn a_broken_request_is_still_answered_with_a_reason() {
        let mut ecc = unreachable_proxy();
        for (request, want) in [
            ("{not json", "parse error"),
            (r#"{"action":"launch"}"#, "unknown action"),
            (r#"{"action":"configure"}"#, "links"),
        ] {
            let reply: Value = serde_json::from_str(&handle(&mut ecc, request)).unwrap();
            assert_eq!(reply["ok"], Value::Bool(false), "{request}");
            let error = reply["error"].as_str().unwrap_or_default();
            assert!(error.contains(want), "{request} -> {error}");
        }
    }

    /// 4 キーは**常に**載る(controller の `EccReply` が読む契約、SPEC §8.2 v1.14)。
    #[test]
    fn every_reply_carries_the_four_keys() {
        let mut ecc = unreachable_proxy();
        let reply: Value =
            serde_json::from_str(&handle(&mut ecc, r#"{"action":"status"}"#)).unwrap();
        for key in ["ok", "state", "error", "ecc_error"] {
            assert!(reply.get(key).is_some(), "{key} が無い: {reply}");
        }
    }

    /// SIGINT/SIGTERM 対応(076 Phase 3 追補): recv タイムアウトのたびに 1 回だけ
    /// フラグを見る —— 立っていれば即座にループを抜ける判断になる、立っていなければ
    /// 抜けない判断になる。ZMQ もシグナルも使わない純粋関数なので、実機依存の
    /// signal-hook 経路を持ち出さずに「1 周で抜ける」条件を固定できる。
    #[test]
    fn should_stop_after_timeout_reads_the_flag_each_cycle() {
        let stopping = AtomicBool::new(false);
        assert!(
            !should_stop_after_timeout(&stopping),
            "フラグが立っていないのに抜ける判断になった"
        );
        stopping.store(true, Ordering::Relaxed);
        assert!(
            should_stop_after_timeout(&stopping),
            "フラグが立っているのに抜けない判断になった"
        );
    }
}
