//! bin 共通のプロセス起動・終了ボイラープレート(TODO/047-A)。
//!
//! `src/bin/{receiver,decoder,graw_writer,monitor,controller}.rs` で重複していた
//! tracing 初期化と SIGINT ハンドラだけをここへ抽出する。`parse_args` と bin 内テストは
//! 各 bin に残る(receiver だけ `--cobo-id` があり、テストが bin 内に住むため)。

use tokio::sync::broadcast;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

/// `RUST_LOG`(既定 `info`)で絞れる tracing subscriber を初期化する。
pub fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
}

/// SIGINT を待ち、受信したら `shutdown_tx` へ通知するタスクを spawn する。
pub fn spawn_sigint(shutdown_tx: broadcast::Sender<()>) {
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
}
