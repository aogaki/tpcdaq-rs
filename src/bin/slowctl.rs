//! slowctl — ガス系 / 環境 / HV の読み取り・記録・アラーム(TODO/079、SPEC §1.1 / §11)。
//!
//! 使い方: `slowctl --config <config.toml>`
//!
//! `[slowctl]` が無い設定では**何もせずに終了**する(その設定では slowctl を走らせない、
//! という意思表示。SPEC §3.1 でセクションごと省略可)。
//!
//! **監視・記録・アラームのみ**(SPEC §11 の安全境界)—— setpoint を書く経路は無い。
//! ログは `RUST_LOG`(既定 `info`)で絞れる。SIGINT でデバイススレッドと
//! PUB / PUSH を畳んで終了する。

use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::broadcast;
use tpcdaq::bin_support::{init_tracing, spawn_sigint};
use tpcdaq::config;
use tpcdaq::slowctl::component::{run_slowctl, SlowctlParams};
use tracing::{error, info};

const USAGE: &str = "usage: slowctl --config <config.toml>";

/// 本体は同期(`component` のモジュール doc)。tokio はここの SIGINT 待ちだけに使い、
/// 本体へは `AtomicBool` を 1 本渡す。
#[tokio::main]
async fn main() -> ExitCode {
    init_tracing();

    let raw: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&raw) {
        Ok(args) => args,
        Err(message) => {
            error!("{message}");
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    let config = match config::load(&args.config) {
        Ok(config) => config,
        Err(e) => {
            error!(config = args.config, error = %e, "cannot load config");
            return ExitCode::FAILURE;
        }
    };
    let Some(params) = SlowctlParams::from_config(&config) else {
        info!(config = args.config, "no [slowctl] section — nothing to do");
        return ExitCode::SUCCESS;
    };

    let stop = Arc::new(AtomicBool::new(false));
    let (shutdown_tx, mut shutdown_rx) = broadcast::channel(1);
    spawn_sigint(shutdown_tx);
    tokio::spawn({
        let stop = Arc::clone(&stop);
        async move {
            let _ = shutdown_rx.recv().await;
            stop.store(true, Ordering::Relaxed);
        }
    });

    // ブロッキング IO の塊なので、tokio のワーカを塞がないよう専用スレッドで回す。
    let result = tokio::task::spawn_blocking(move || run_slowctl(params, stop)).await;
    match result {
        Ok(Ok(())) => ExitCode::SUCCESS,
        Ok(Err(e)) => {
            error!(error = %e, "slowctl failed");
            ExitCode::FAILURE
        }
        Err(e) => {
            error!(error = %e, "the slowctl thread did not finish cleanly");
            ExitCode::FAILURE
        }
    }
}

struct Args {
    config: String,
}

/// 手書きの引数パース(追加依存なし。receiver / monitor と同じ流儀)。
fn parse_args(raw: &[String]) -> Result<Args, String> {
    let mut config: Option<String> = None;

    let mut iter = raw.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--config" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--config requires a path".to_string())?;
                config = Some(value.clone());
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    Ok(Args {
        config: config.ok_or_else(|| "--config is required".to_string())?,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::parse_args;

    fn strs(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_the_config_path() {
        let args = parse_args(&strs(&["--config", "config/slowctl_example.toml"])).unwrap();
        assert_eq!(args.config, "config/slowctl_example.toml");
    }

    #[test]
    fn rejects_missing_or_malformed_arguments() {
        assert!(parse_args(&strs(&[])).is_err()); // 引数なし
        assert!(parse_args(&strs(&["--config"])).is_err()); // 値のないフラグ
        assert!(parse_args(&strs(&["--bogus", "x"])).is_err());
    }
}
