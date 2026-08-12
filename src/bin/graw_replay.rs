//! graw_replay — 記録済み `.graw` ファイルのバイト列をそのまま TCP で receiver へ送出する
//! リプレイツール(TODO/005)。検出器が無くても、記録済み .graw を tpcdaq の受信ポートへ
//! 再送して受信〜デコードを検証できる(C++ 版 `tools/graw_replay.cpp` の後継。
//! `--rate-mbps` によるペーシングと `--loop` を追加。SPEC §12 末尾)。
//!
//! 使い方: `graw_replay <host:port> <file.graw> [--rate-mbps <f64>] [--loop] [--chunk-bytes <n=65536>]`
//!
//! - `--loop` なし: ファイル全体を送り切ったら接続を閉じて終了(受信側は EOF = run 境界)。
//! - `--loop` あり: EOF に達したらファイル先頭へ戻って送り続ける(Ctrl-C か受信側切断で停止)。
//! - 接続失敗・送出中の切断は明確なエラーメッセージ + 非 0 exit(silent failure を作らない)。

use std::fmt;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

fn main() -> ExitCode {
    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    let parsed = match args::parse(&raw_args) {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("graw_replay: {msg}");
            eprintln!("{}", args::USAGE);
            return ExitCode::from(2);
        }
    };

    match run(&parsed) {
        Ok(total) => {
            println!("replayed {total} bytes to {}", parsed.endpoint);
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("graw_replay: {err}");
            ExitCode::FAILURE
        }
    }
}

/// 実際のリプレイ本体。テスト容易性のため main から分離してあるが、I/O を伴うので
/// bin 内ユニットテストの対象は `args::parse` と `pace` の純粋関数側に絞る(KISS)。
fn run(cfg: &args::Args) -> Result<u64, ReplayError> {
    let mut file = File::open(&cfg.file).map_err(|source| ReplayError::FileOpen {
        path: cfg.file.clone(),
        source,
    })?;

    let mut stream = TcpStream::connect(&cfg.endpoint).map_err(|source| ReplayError::Connect {
        endpoint: cfg.endpoint.clone(),
        source,
    })?;

    let rate_bytes_per_sec = cfg
        .rate_mbps
        .map(pace::mbps_to_bytes_per_sec)
        .unwrap_or(0.0);
    let mut buf = vec![0u8; cfg.chunk_bytes];
    let start = Instant::now();
    let mut total: u64 = 0;

    loop {
        let n = file.read(&mut buf).map_err(ReplayError::Read)?;
        if n == 0 {
            if cfg.loop_replay {
                file.seek(SeekFrom::Start(0)).map_err(ReplayError::Read)?;
                continue;
            }
            break;
        }

        stream.write_all(&buf[..n]).map_err(ReplayError::Send)?;
        total += n as u64;

        if rate_bytes_per_sec > 0.0 {
            let wait = pace::sleep_for(total, rate_bytes_per_sec, start.elapsed());
            if wait > Duration::ZERO {
                std::thread::sleep(wait);
            }
        }
    }

    Ok(total)
}

/// 発生しうるエラーを利用者に分かるメッセージへ落とす(接続失敗・途中切断を隠さない)。
#[derive(Debug)]
enum ReplayError {
    FileOpen { path: PathBuf, source: io::Error },
    Connect { endpoint: String, source: io::Error },
    Read(io::Error),
    Send(io::Error),
}

impl fmt::Display for ReplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReplayError::FileOpen { path, source } => {
                write!(f, "failed to open {}: {source}", path.display())
            }
            ReplayError::Connect { endpoint, source } => {
                write!(f, "failed to connect to {endpoint}: {source}")
            }
            ReplayError::Read(source) => write!(f, "failed to read graw file: {source}"),
            ReplayError::Send(source) => {
                write!(
                    f,
                    "failed to send to receiver (connection closed?): {source}"
                )
            }
        }
    }
}

impl std::error::Error for ReplayError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ReplayError::FileOpen { source, .. } => Some(source),
            ReplayError::Connect { source, .. } => Some(source),
            ReplayError::Read(source) | ReplayError::Send(source) => Some(source),
        }
    }
}

/// CLI 引数パース(手書き。追加依存なし)。
mod args {
    use std::path::PathBuf;

    pub const USAGE: &str =
        "usage: graw_replay <host:port> <file.graw> [--rate-mbps <f64>] [--loop] [--chunk-bytes <n=65536>]";

    const DEFAULT_CHUNK_BYTES: usize = 65536;

    #[derive(Debug, Clone, PartialEq)]
    pub struct Args {
        pub endpoint: String,
        pub file: PathBuf,
        pub rate_mbps: Option<f64>,
        pub loop_replay: bool,
        pub chunk_bytes: usize,
    }

    /// `raw`(プログラム名を除いた argv)を解釈する。エラーメッセージは利用者向けの文字列。
    pub fn parse(raw: &[String]) -> Result<Args, String> {
        let mut positional: Vec<&String> = Vec::new();
        let mut rate_mbps: Option<f64> = None;
        let mut loop_replay = false;
        let mut chunk_bytes = DEFAULT_CHUNK_BYTES;

        let mut iter = raw.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--rate-mbps" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "--rate-mbps requires a value".to_string())?;
                    let parsed: f64 = value
                        .parse()
                        .map_err(|_| format!("--rate-mbps value is not a number: {value}"))?;
                    if !(parsed.is_finite() && parsed > 0.0) {
                        return Err(format!(
                            "--rate-mbps must be a positive finite number, got {value}"
                        ));
                    }
                    rate_mbps = Some(parsed);
                }
                "--loop" => loop_replay = true,
                "--chunk-bytes" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "--chunk-bytes requires a value".to_string())?;
                    let parsed: usize = value
                        .parse()
                        .map_err(|_| format!("--chunk-bytes value is not an integer: {value}"))?;
                    if parsed == 0 {
                        return Err("--chunk-bytes must be greater than 0".to_string());
                    }
                    chunk_bytes = parsed;
                }
                other if other.starts_with("--") => {
                    return Err(format!("unknown option: {other}"));
                }
                _ => positional.push(arg),
            }
        }

        if positional.len() != 2 {
            return Err(format!(
                "expected 2 positional arguments (host:port, file.graw), got {}",
                positional.len()
            ));
        }

        Ok(Args {
            endpoint: positional[0].clone(),
            file: PathBuf::from(positional[1]),
            rate_mbps,
            loop_replay,
            chunk_bytes,
        })
    }
}

/// ペーシング計算(純粋関数。I/O なしでユニットテストできる)。
mod pace {
    use std::time::Duration;

    /// `--rate-mbps` は「メガビット/秒」(ネットワーク慣習の bit 単位。1 Mbps = 1_000_000 bit/s)。
    /// byte/秒 に変換して返す。
    pub fn mbps_to_bytes_per_sec(rate_mbps: f64) -> f64 {
        rate_mbps * 1_000_000.0 / 8.0
    }

    /// これまでの累計送出バイト数 `bytes_sent_total` と実経過時間 `elapsed` から、
    /// 目標レート `rate_bytes_per_sec` に追いつくために送出後に挟むべき sleep 時間を返す
    /// (閉ループ: 目標消化時刻 - 実経過時間。負なら追いついていないので Duration::ZERO)。
    pub fn sleep_for(
        bytes_sent_total: u64,
        rate_bytes_per_sec: f64,
        elapsed: Duration,
    ) -> Duration {
        if rate_bytes_per_sec <= 0.0 {
            return Duration::ZERO;
        }
        let target_secs = bytes_sent_total as f64 / rate_bytes_per_sec;
        let elapsed_secs = elapsed.as_secs_f64();
        if target_secs > elapsed_secs {
            Duration::from_secs_f64(target_secs - elapsed_secs)
        } else {
            Duration::ZERO
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::args::{self, Args};
    use super::pace;
    use std::path::PathBuf;
    use std::time::Duration;

    fn strs(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_minimal_positional_args_uses_defaults() {
        let got = args::parse(&strs(&["127.0.0.1:9000", "run.graw"])).unwrap();
        assert_eq!(
            got,
            Args {
                endpoint: "127.0.0.1:9000".to_string(),
                file: PathBuf::from("run.graw"),
                rate_mbps: None,
                loop_replay: false,
                chunk_bytes: 65536,
            }
        );
    }

    #[test]
    fn parse_accepts_all_optional_flags_in_any_order() {
        let got = args::parse(&strs(&[
            "--loop",
            "host:1234",
            "--chunk-bytes",
            "4096",
            "run.graw",
            "--rate-mbps",
            "28.0",
        ]))
        .unwrap();
        assert_eq!(
            got,
            Args {
                endpoint: "host:1234".to_string(),
                file: PathBuf::from("run.graw"),
                rate_mbps: Some(28.0),
                loop_replay: true,
                chunk_bytes: 4096,
            }
        );
    }

    #[test]
    fn parse_rejects_wrong_positional_count() {
        assert!(args::parse(&strs(&["only-one-arg"])).is_err());
        assert!(args::parse(&strs(&["a", "b", "c"])).is_err());
        assert!(args::parse(&strs(&[])).is_err());
    }

    #[test]
    fn parse_rejects_non_positive_rate() {
        assert!(args::parse(&strs(&["h:1", "f", "--rate-mbps", "0"])).is_err());
        assert!(args::parse(&strs(&["h:1", "f", "--rate-mbps", "-1.0"])).is_err());
        assert!(args::parse(&strs(&["h:1", "f", "--rate-mbps", "nope"])).is_err());
    }

    #[test]
    fn parse_rejects_zero_chunk_bytes() {
        assert!(args::parse(&strs(&["h:1", "f", "--chunk-bytes", "0"])).is_err());
        assert!(args::parse(&strs(&["h:1", "f", "--chunk-bytes", "abc"])).is_err());
    }

    #[test]
    fn parse_rejects_unknown_option() {
        assert!(args::parse(&strs(&["h:1", "f", "--bogus"])).is_err());
    }

    #[test]
    fn parse_rejects_dangling_flag_without_value() {
        assert!(args::parse(&strs(&["h:1", "f", "--rate-mbps"])).is_err());
        assert!(args::parse(&strs(&["h:1", "f", "--chunk-bytes"])).is_err());
    }

    #[test]
    fn mbps_to_bytes_per_sec_matches_hand_calculation() {
        // 手計算: 8 Mbps = 8,000,000 bit/s / 8 = 1,000,000 byte/s
        assert_eq!(pace::mbps_to_bytes_per_sec(8.0), 1_000_000.0);
        // 手計算: 28 Mbps = 28,000,000 / 8 = 3,500,000 byte/s
        assert_eq!(pace::mbps_to_bytes_per_sec(28.0), 3_500_000.0);
    }

    #[test]
    fn sleep_for_returns_zero_when_pacing_disabled() {
        assert_eq!(
            pace::sleep_for(1_000_000, 0.0, Duration::from_millis(1)),
            Duration::ZERO
        );
    }

    #[test]
    fn sleep_for_returns_zero_when_already_behind_schedule() {
        // 手計算: 100 byte / 1000 byte/s = 0.1s 分の予算に対し、実経過は既に 1.0s なので待たない。
        assert_eq!(
            pace::sleep_for(100, 1_000.0, Duration::from_secs(1)),
            Duration::ZERO
        );
    }

    #[test]
    fn sleep_for_waits_the_remaining_budget_when_ahead_of_schedule() {
        // 手計算: 500,000 byte / 500,000 byte/s = 1.0s 分の予算に対し、実経過はまだ 0.2s なので
        // 残り 0.8s 待つ。
        let got = pace::sleep_for(500_000, 500_000.0, Duration::from_millis(200));
        assert_eq!(got, Duration::from_millis(800));
    }
}
