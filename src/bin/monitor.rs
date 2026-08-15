//! monitor — root-sink のモニタ PUB を購読し、WS で配るコンポーネント(TODO/026、SPEC §5.4/§10)。
//!
//! 使い方: `monitor --config <config.toml>`
//!
//! `[monitor]`(と `[system]`・`[[cobo]]`)から SUB 接続先・WS listen・ジオメトリ・
//! live キュー段数を解決し(SPEC §3.2 の既定 + 上書き)、WS を開いて待つ。
//! monitor は**純コンシューマ**でコマンド REP を持たない(run 制御の外 — SPEC §8.1)。
//!
//! ログは `RUST_LOG`(既定 `info`)で絞れる。

use std::process::ExitCode;

use tokio::sync::broadcast;
use tpcdaq::bin_support::{init_tracing, spawn_sigint};
use tpcdaq::config;
use tpcdaq::monitor::{run_monitor, MonitorParams};
use tracing::error;

const USAGE: &str = "usage: monitor --config <config.toml>";

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
    let params = MonitorParams::from_config(&config);

    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    spawn_sigint(shutdown_tx);

    match run_monitor(params, shutdown_rx, None).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!(error = %e, "monitor failed");
            ExitCode::FAILURE
        }
    }
}

struct Args {
    config: String,
}

/// 手書きの引数パース(追加依存なし。receiver / controller と同じ流儀)。
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
        let args = parse_args(&strs(&["--config", "config/mini.toml"])).unwrap();
        assert_eq!(args.config, "config/mini.toml");
    }

    #[test]
    fn rejects_missing_or_malformed_arguments() {
        assert!(parse_args(&strs(&[])).is_err()); // 引数なし
        assert!(parse_args(&strs(&["--config"])).is_err()); // 値のないフラグ
        assert!(parse_args(&strs(&["--bogus", "x"])).is_err());
    }
}
