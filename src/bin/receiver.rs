//! receiver — CoBo 毎の受信コンポーネント(TODO/006、SPEC §1.1)。
//!
//! 使い方: `receiver --config <config.toml> --cobo-id <k>`
//!
//! `[[cobo]]` の該当ブロックと `[receiver]` から起動パラメタを解決し(SPEC §3)、
//! コマンド REP を `tcp://*:47110+k`(SPEC §3.2)で開いて controller の指示を待つ。
//! データ用ポートの bind は `Arm` まで行わない(listen-before-start、SPEC §1.3)。
//!
//! ログは `RUST_LOG`(既定 `info`)で絞れる。

use std::process::ExitCode;

use tokio::sync::broadcast;
use tpcdaq::config;
use tpcdaq::receiver::{run_receiver, ReceiverParams};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

const USAGE: &str = "usage: receiver --config <config.toml> --cobo-id <k>";

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

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

    let Some(params) = ReceiverParams::from_config(&config, args.cobo_id) else {
        error!(
            cobo_id = args.cobo_id,
            config = args.config,
            "no [[cobo]] block with this id in the config"
        );
        return ExitCode::FAILURE;
    };

    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    tokio::spawn(async move {
        match tokio::signal::ctrl_c().await {
            Ok(()) => {
                info!("SIGINT — shutting down");
                let _ = shutdown_tx.send(());
            }
            // 待てないなら黙って諦めるのではなく残す(kill での停止は依然可能)。
            Err(e) => error!(error = %e, "cannot listen for SIGINT"),
        }
    });

    run_receiver(params, shutdown_rx, None).await;
    ExitCode::SUCCESS
}

struct Args {
    config: String,
    cobo_id: u32,
}

/// 手書きの引数パース(追加依存なし。graw_replay と同じ流儀)。
fn parse_args(raw: &[String]) -> Result<Args, String> {
    let mut config: Option<String> = None;
    let mut cobo_id: Option<u32> = None;

    let mut iter = raw.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--config" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--config requires a path".to_string())?;
                config = Some(value.clone());
            }
            "--cobo-id" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--cobo-id requires a number".to_string())?;
                cobo_id = Some(
                    value
                        .parse()
                        .map_err(|_| format!("--cobo-id is not a number: {value}"))?,
                );
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    Ok(Args {
        config: config.ok_or_else(|| "--config is required".to_string())?,
        cobo_id: cobo_id.ok_or_else(|| "--cobo-id is required".to_string())?,
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
    fn parses_config_and_cobo_id_in_any_order() {
        let args = parse_args(&strs(&["--cobo-id", "1", "--config", "config/mini.toml"])).unwrap();
        assert_eq!(args.config, "config/mini.toml");
        assert_eq!(args.cobo_id, 1);
    }

    #[test]
    fn rejects_missing_or_malformed_arguments() {
        assert!(parse_args(&strs(&[])).is_err()); // 引数なし
        assert!(parse_args(&strs(&["--config", "x.toml"])).is_err()); // cobo-id 欠落
        assert!(parse_args(&strs(&["--cobo-id", "0"])).is_err()); // config 欠落
        assert!(parse_args(&strs(&["--config"])).is_err()); // 値のないフラグ
        assert!(parse_args(&strs(&["--config", "x.toml", "--cobo-id", "one"])).is_err());
        assert!(parse_args(&strs(&["--bogus"])).is_err());
    }
}
