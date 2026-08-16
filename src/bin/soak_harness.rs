//! soak_harness —— 連続負荷 / 瞬発負荷ハーネス(TODO/031、SPEC §12-5 v1.15 / §12-6)。
//!
//! **030 E2E-C と同じ controller 駆動の全通し配線を子プロセスで上げ、run を反復する。**
//! 中身は「書いて検証して消す」: 1 run = 実 mini `.graw` を `graw_replay --laps-until-s` で
//! N 周(周回毎に eventIdx を `+lap*(max_idx+1)` して送出)流し、run を閉じてから全カウンタ・
//! 全バイト・ROOT entries を機械照合し、合格したら出力を消して次の run へ進む。
//!
//! ```text
//!   soak_harness
//!     ├─ fake_ecc(Ice servant、--no-data-link)─ ecc_bridge(REP 47200)
//!     ├─ root_sink(C++)── PUB 47004 ──> monitor(WS 9000)<── 本ハーネスの WS probe
//!     ├─ graw_writer / decoder / receiver0(Rust bin、--config)
//!     ├─ controller(REST 8080)── REQ ──> 上の 3 つと ecc_bridge
//!     └─ 各 run: POST /api/run/start → graw_replay → POST /api/run/stop → 照合 → 削除
//! ```
//!
//! # 判定の一次データは CSV(SPEC §12-5 v1.15 の「トレンド駆動」)
//!
//! `--metrics-interval-s`(既定 60 = 1 分)毎に **1 行 1 サンプル**の CSV を追記 flush する。
//! 列は「各プロセスの RSS / open fd 数 / 全ロスレスカウンタ / モニタ系 drop / 空きディスク」。
//! 合否は**この CSV から機械的に**出す(終了時に全系列の始値・終値・傾きを要約)。
//! クラッシュしても CSV は残る = 証拠第一。
//!
//! # 使い方
//!
//! ```text
//! TPCDAQ_ROOT_SINK_BIN=$PWD/tools/root_sink/root_sink \
//! TPCDAQ_ECC_BRIDGE_BIN=$PWD/tools/ecc_bridge/ecc_bridge \
//! TPCDAQ_FAKE_ECC_BIN=$PWD/tools/ecc_bridge/fake_ecc \
//! TPCDAQ_REAL_GRAW=$HOME/TPC/CoBo_2025-09-01T08_51_06.203_0000.graw \
//! TPCDAQ_REAL_GEOMETRY_MINI=$HOME/TPC/miniTPC_UVW_pcb_info/new_geometry_mini_eTPC.dat \
//!   target/release/soak_harness --mode soak --duration-h 12 --out-dir /tmp/soak
//! ```
//!
//! 失敗したら **即停止し、その run の出力とログを残したまま** 非 0 で終わる(証拠保全)。

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// 実 mini `.graw` 1 周分のバイト数(SPEC §12-2 オラクル、2026-08-13 実測)。
const GRAW_BYTES_PER_LAP: u64 = 30_108_684;
/// 実 mini `.graw` 1 周分のイベント数(SPEC §12-1 オラクル)。
const EVENTS_PER_LAP: u64 = 108;
/// mini 実測の出力膨張率(ROOT / 入力 graw)。起動時のディスク見積もりに使う。
const ROOT_TO_GRAW_RATIO: f64 = 1.53;

const OPERATOR: &str = "soak-harness";
const PASSPHRASE: &str = "soak-harness-passphrase";

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let cfg = match args::parse(&raw) {
        Ok(cfg) => cfg,
        Err(msg) => {
            eprintln!("soak_harness: {msg}");
            eprintln!("{}", args::USAGE);
            return ExitCode::from(2);
        }
    };
    match run(&cfg) {
        Ok(report) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Err(msg) => {
            eprintln!("soak_harness: FAILED —— {msg}");
            eprintln!("(証拠は残してある。出力とログは --out-dir 配下をそのまま見ること)");
            ExitCode::FAILURE
        }
    }
}

// =====================================================================
// 引数
// =====================================================================

mod args {
    use std::path::PathBuf;

    pub const USAGE: &str = "usage: soak_harness --mode <soak|burst> [options]\n\
         \n\
           --mode soak|burst        soak = ペース付き耐久(SPEC §12-5)/ burst = 全速(§12-6)\n\
           --duration-h <f64>       soak の走行時間(既定 12 —— SPEC v1.15 の「一晩」)\n\
           --burst-min <f64>        burst の走行時間 分(既定 10)\n\
           --rate-mbps <f64>        リプレイのペース(既定: soak 224 / burst 0 = 全速)\n\
           --run-minutes <f64>      1 run の長さ 分(既定 10。graw_replay --laps-until-s に変換)\n\
           --runs <u64>             run 本数の上限(既定 なし = 時間で決まる。スモーク用)\n\
           --rss-limit-mib <u64>    1 プロセスの RSS 上限(既定 4096)\n\
           --metrics-interval-s <u64>  CSV サンプリング間隔 秒(既定 60)\n\
           --min-free-gib <u64>     走行中に守る空きディスクの床 GiB(既定 20)\n\
           --keep-outputs           合格した run の出力を消さない(既定 off)\n\
           --out-dir <path>         作業ディレクトリ(既定 $TMPDIR/tpcdaq_soak_<pid>)\n\
           --bin-dir <path>         tpcdaq の Rust バイナリの置き場(既定 = 本体と同じ dir)\n\
         \n\
         env(必須): TPCDAQ_ROOT_SINK_BIN / TPCDAQ_ECC_BRIDGE_BIN / TPCDAQ_FAKE_ECC_BIN /\n\
         TPCDAQ_REAL_GRAW / TPCDAQ_REAL_GEOMETRY_MINI";

    #[derive(Debug, Clone, PartialEq)]
    pub enum Mode {
        Soak,
        Burst,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub struct Args {
        pub mode: Mode,
        /// 走行時間(mode に応じて --duration-h / --burst-min から導く)。
        pub duration_s: f64,
        /// 0.0 = ペーシングなし(全速)。
        pub rate_mbps: f64,
        pub run_seconds: f64,
        /// 走行時間とは別に run 本数を打ち切る(スモーク用。`None` = 時間だけで決める)。
        pub max_runs: Option<u64>,
        pub rss_limit_mib: u64,
        pub metrics_interval_s: u64,
        pub min_free_gib: u64,
        pub keep_outputs: bool,
        pub out_dir: Option<PathBuf>,
        pub bin_dir: Option<PathBuf>,
    }

    fn positive(flag: &str, value: &str) -> Result<f64, String> {
        let parsed: f64 = value
            .parse()
            .map_err(|_| format!("{flag} value is not a number: {value}"))?;
        if !(parsed.is_finite() && parsed > 0.0) {
            return Err(format!(
                "{flag} must be a positive finite number, got {value}"
            ));
        }
        Ok(parsed)
    }

    fn positive_u64(flag: &str, value: &str) -> Result<u64, String> {
        let parsed: u64 = value
            .parse()
            .map_err(|_| format!("{flag} value is not an integer: {value}"))?;
        if parsed == 0 {
            return Err(format!("{flag} must be greater than 0"));
        }
        Ok(parsed)
    }

    pub fn parse(raw: &[String]) -> Result<Args, String> {
        let mut mode: Option<Mode> = None;
        let mut duration_h: Option<f64> = None;
        let mut burst_min: Option<f64> = None;
        let mut rate_mbps: Option<f64> = None;
        let mut run_minutes: f64 = 10.0;
        let mut max_runs: Option<u64> = None;
        let mut rss_limit_mib: u64 = 4096;
        let mut metrics_interval_s: u64 = 60;
        let mut min_free_gib: u64 = 20;
        let mut keep_outputs = false;
        let mut out_dir: Option<PathBuf> = None;
        let mut bin_dir: Option<PathBuf> = None;

        let mut iter = raw.iter();
        while let Some(arg) = iter.next() {
            let mut value = || -> Result<String, String> {
                iter.next()
                    .cloned()
                    .ok_or_else(|| format!("{arg} requires a value"))
            };
            match arg.as_str() {
                "--mode" => {
                    mode = Some(match value()?.as_str() {
                        "soak" => Mode::Soak,
                        "burst" => Mode::Burst,
                        other => return Err(format!("--mode must be soak|burst, got {other}")),
                    })
                }
                "--duration-h" => duration_h = Some(positive("--duration-h", &value()?)?),
                "--burst-min" => burst_min = Some(positive("--burst-min", &value()?)?),
                "--rate-mbps" => {
                    // 0 = 全速(burst の既定)。負・非有限は拒否。
                    let text = value()?;
                    let parsed: f64 = text
                        .parse()
                        .map_err(|_| format!("--rate-mbps value is not a number: {text}"))?;
                    if !parsed.is_finite() || parsed < 0.0 {
                        return Err(format!("--rate-mbps must be >= 0 and finite, got {text}"));
                    }
                    rate_mbps = Some(parsed);
                }
                "--run-minutes" => run_minutes = positive("--run-minutes", &value()?)?,
                "--runs" => max_runs = Some(positive_u64("--runs", &value()?)?),
                "--rss-limit-mib" => rss_limit_mib = positive_u64("--rss-limit-mib", &value()?)?,
                "--metrics-interval-s" => {
                    metrics_interval_s = positive_u64("--metrics-interval-s", &value()?)?
                }
                "--min-free-gib" => min_free_gib = positive_u64("--min-free-gib", &value()?)?,
                "--keep-outputs" => keep_outputs = true,
                "--out-dir" => out_dir = Some(PathBuf::from(value()?)),
                "--bin-dir" => bin_dir = Some(PathBuf::from(value()?)),
                other => return Err(format!("unknown option: {other}")),
            }
        }

        let mode = mode.ok_or_else(|| "--mode is required".to_string())?;
        let duration_s = match mode {
            Mode::Soak => duration_h.unwrap_or(12.0) * 3600.0,
            Mode::Burst => burst_min.unwrap_or(10.0) * 60.0,
        };
        let rate_mbps = rate_mbps.unwrap_or(match mode {
            // SPEC §12 末尾「mini 100 Hz 相当 ≈ 28 MB/s = 224 Mbps」。
            Mode::Soak => 224.0,
            // SPEC §12-6「ペーシングなし全速」。
            Mode::Burst => 0.0,
        });

        Ok(Args {
            mode,
            duration_s,
            rate_mbps,
            run_seconds: run_minutes * 60.0,
            max_runs,
            rss_limit_mib,
            metrics_interval_s,
            min_free_gib,
            keep_outputs,
            out_dir,
            bin_dir,
        })
    }
}

// =====================================================================
// 入力(env と バイナリの place)
// =====================================================================

struct Inputs {
    root_sink: PathBuf,
    ecc_bridge: PathBuf,
    fake_ecc: PathBuf,
    graw: PathBuf,
    geometry: PathBuf,
    bin_dir: PathBuf,
}

fn inputs(cfg: &args::Args) -> Result<Inputs, String> {
    let mut missing = Vec::new();
    let mut get = |name: &str| -> PathBuf {
        match std::env::var_os(name) {
            Some(v) => PathBuf::from(v),
            None => {
                missing.push(name.to_string());
                PathBuf::new()
            }
        }
    };
    let root_sink = get("TPCDAQ_ROOT_SINK_BIN");
    let ecc_bridge = get("TPCDAQ_ECC_BRIDGE_BIN");
    let fake_ecc = get("TPCDAQ_FAKE_ECC_BIN");
    let graw = get("TPCDAQ_REAL_GRAW");
    let geometry = get("TPCDAQ_REAL_GEOMETRY_MINI");
    if !missing.is_empty() {
        return Err(format!("未設定の env = {}", missing.join(", ")));
    }

    let bin_dir = match &cfg.bin_dir {
        Some(dir) => dir.clone(),
        None => std::env::current_exe()
            .map_err(|e| format!("current_exe: {e}"))?
            .parent()
            .ok_or_else(|| "current_exe has no parent directory".to_string())?
            .to_path_buf(),
    };

    let inputs = Inputs {
        root_sink,
        ecc_bridge,
        fake_ecc,
        graw,
        geometry,
        bin_dir,
    };
    for (what, path) in [
        ("TPCDAQ_ROOT_SINK_BIN", &inputs.root_sink),
        ("TPCDAQ_ECC_BRIDGE_BIN", &inputs.ecc_bridge),
        ("TPCDAQ_FAKE_ECC_BIN", &inputs.fake_ecc),
        ("TPCDAQ_REAL_GRAW", &inputs.graw),
        ("TPCDAQ_REAL_GEOMETRY_MINI", &inputs.geometry),
    ] {
        if !path.is_file() {
            return Err(format!("{what} が指すファイルが無い: {}", path.display()));
        }
    }
    for name in RUST_BINS {
        let path = inputs.bin_dir.join(name);
        if !path.is_file() {
            return Err(format!(
                "{} が無い(--bin-dir を指定するか cargo build --release --bins)",
                path.display()
            ));
        }
    }
    Ok(inputs)
}

const RUST_BINS: [&str; 5] = [
    "controller",
    "receiver",
    "decoder",
    "graw_writer",
    "monitor",
];

// =====================================================================
// 子プロセス
// =====================================================================

/// 起動した子 1 つ。`log` はその子の stdout+stderr を落とした先。
struct Proc {
    name: String,
    child: Child,
    pid: u32,
    log: PathBuf,
}

impl Proc {
    fn spawn(name: &str, log_dir: &Path, mut command: Command) -> Result<Proc, String> {
        let log = log_dir.join(format!("{name}.log"));
        let out = File::create(&log).map_err(|e| format!("create {}: {e}", log.display()))?;
        let err = out
            .try_clone()
            .map_err(|e| format!("clone log handle: {e}"))?;
        let child = command
            .stdout(Stdio::from(out))
            .stderr(Stdio::from(err))
            .spawn()
            .map_err(|e| format!("spawn {name}: {e}"))?;
        let pid = child.id();
        Ok(Proc {
            name: name.to_string(),
            child,
            pid,
            log,
        })
    }

    fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// ログに `needle` が現れるまで待つ(起動完了の自己申告を読む — 041 デモと同じ流儀)。
    fn wait_for_log(&mut self, needle: &str, timeout: Duration) -> Result<String, String> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(line) = log_line_containing(&self.log, needle) {
                return Ok(line);
            }
            if !self.alive() {
                return Err(format!(
                    "{} が {:?} を出す前に終了した(log = {})",
                    self.name,
                    needle,
                    self.log.display()
                ));
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "{} が {timeout:?} 以内に {:?} を出さなかった(log = {})",
                    self.name,
                    needle,
                    self.log.display()
                ));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn signal(&self, name: &str) {
        let _ = Command::new("kill")
            .arg(format!("-{name}"))
            .arg(self.pid.to_string())
            .status();
    }

    fn stop(&mut self, sig: &str, timeout: Duration) {
        if !self.alive() {
            return;
        }
        self.signal(sig);
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if !self.alive() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn log_line_containing(path: &Path, needle: &str) -> Option<String> {
    let file = File::open(path).ok()?;
    BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .find(|line| line.contains(needle))
}

fn log_lines_containing(path: &Path, needle: &str) -> Vec<String> {
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter(|line| line.contains(needle))
        .collect()
}

// =====================================================================
// 手書き HTTP(依存を増やさない — E2E と同じ流儀)
// =====================================================================

fn http(
    method: &str,
    addr: SocketAddr,
    path: &str,
    body: Option<&str>,
) -> Result<(u16, Value), String> {
    let payload = body.unwrap_or("");
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{payload}",
        payload.len()
    );
    let mut stream =
        TcpStream::connect(addr).map_err(|e| format!("connect to the controller {addr}: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(300)))
        .map_err(|e| format!("set_read_timeout: {e}"))?;
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("write {method} {path}: {e}"))?;
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|e| format!("read {method} {path}: {e}"))?;
    let text = String::from_utf8_lossy(&raw).to_string();
    let status: u16 = text
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("no status line in {text:?}"))?;
    let body = text
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or_default();
    Ok((status, serde_json::from_str(&body).unwrap_or(Value::Null)))
}

fn post(addr: SocketAddr, path: &str, body: String) -> Result<Value, String> {
    let (status, value) = http("POST", addr, path, Some(&body))?;
    if status != 200 {
        return Err(format!("POST {path} -> {status}: {value}"));
    }
    Ok(value)
}

// =====================================================================
// WS probe(モニタ経路に本物のクライアントを 1 本ぶら下げる)
// =====================================================================
//
// tokio-tungstenite は dev-dependency なので bin からは使えない(TODO/031 受け入れ:
// 「新依存なし」)。ここで要るのは **繋いで読み続けるだけ**のクライアントなので、
// RFC 6455 のうち「ハンドシェイク + サーバ→クライアントのフレーム読み」だけを実装する
// (クライアント→サーバは 1 通も送らない = マスク処理が要らない。既定購読のままでよい)。
mod ws {
    use super::*;

    /// monitor が 1 Hz で配る `status` JSON の最新値。
    pub struct Latest {
        pub status: Mutex<Option<Value>>,
        pub text_messages: AtomicU64,
        pub binary_messages: AtomicU64,
        pub connected: AtomicBool,
    }

    impl Latest {
        pub fn new() -> Arc<Latest> {
            Arc::new(Latest {
                status: Mutex::new(None),
                text_messages: AtomicU64::new(0),
                binary_messages: AtomicU64::new(0),
                connected: AtomicBool::new(false),
            })
        }
    }

    /// `ws://addr/ws` へ繋いで、止められるまで読み続けるスレッドを起こす。
    pub fn spawn(addr: SocketAddr, latest: Arc<Latest>, stop: Arc<AtomicBool>) {
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match connect(addr) {
                    Ok(stream) => {
                        latest.connected.store(true, Ordering::Relaxed);
                        let _ = pump(stream, &latest, &stop);
                        latest.connected.store(false, Ordering::Relaxed);
                    }
                    Err(e) => eprintln!("soak_harness: WS probe connect failed: {e}"),
                }
                // 落ちたら黙って諦めない(silent failure 禁止)。少し待って張り直す。
                if !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_secs(2));
                }
            }
        });
    }

    fn connect(addr: SocketAddr) -> Result<TcpStream, String> {
        let mut stream = TcpStream::connect(addr).map_err(|e| format!("connect {addr}: {e}"))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .map_err(|e| format!("set_read_timeout: {e}"))?;
        // Sec-WebSocket-Key は「16 B を base64 した任意の値」。ここでは固定値でよい
        // (乱数性はプロキシ経由のキャッシュ避けのためのもので、認証ではない)。
        let request = format!(
            "GET /ws HTTP/1.1\r\nHost: {addr}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Key: c29ha19oYXJuZXNzX3Byb2I=\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );
        stream
            .write_all(request.as_bytes())
            .map_err(|e| format!("write handshake: {e}"))?;

        // 応答ヘッダを \r\n\r\n まで 1 バイトずつ読む(本文の頭を食わないため)。
        let mut header = Vec::new();
        let mut byte = [0u8; 1];
        while !header.ends_with(b"\r\n\r\n") {
            let n = stream
                .read(&mut byte)
                .map_err(|e| format!("read handshake: {e}"))?;
            if n == 0 {
                return Err("server closed during the handshake".to_string());
            }
            header.push(byte[0]);
            if header.len() > 8192 {
                return Err("handshake response is too long".to_string());
            }
        }
        let text = String::from_utf8_lossy(&header).to_string();
        if !text.starts_with("HTTP/1.1 101") {
            return Err(format!(
                "WS upgrade refused: {}",
                text.lines().next().unwrap_or("")
            ));
        }
        Ok(stream)
    }

    /// サーバ→クライアントのフレームを読み続ける(マスクなし。opcode 1 = text だけ解釈)。
    fn pump(mut stream: TcpStream, latest: &Latest, stop: &AtomicBool) -> Result<(), String> {
        let mut payload = Vec::new();
        while !stop.load(Ordering::Relaxed) {
            let mut head = [0u8; 2];
            if let Err(e) = stream.read_exact(&mut head) {
                return Err(format!("read frame header: {e}"));
            }
            let opcode = head[0] & 0x0f;
            let masked = head[1] & 0x80 != 0;
            let len = match head[1] & 0x7f {
                126 => {
                    let mut b = [0u8; 2];
                    stream.read_exact(&mut b).map_err(|e| e.to_string())?;
                    u16::from_be_bytes(b) as usize
                }
                127 => {
                    let mut b = [0u8; 8];
                    stream.read_exact(&mut b).map_err(|e| e.to_string())?;
                    u64::from_be_bytes(b) as usize
                }
                short => short as usize,
            };
            if masked {
                let mut mask = [0u8; 4];
                stream.read_exact(&mut mask).map_err(|e| e.to_string())?;
            }
            payload.resize(len, 0);
            stream
                .read_exact(&mut payload)
                .map_err(|e| format!("read frame payload ({len} B): {e}"))?;
            match opcode {
                0x1 => {
                    latest.text_messages.fetch_add(1, Ordering::Relaxed);
                    if let Ok(value) = serde_json::from_slice::<Value>(&payload) {
                        if value["type"] == "status" {
                            if let Ok(mut slot) = latest.status.lock() {
                                *slot = Some(value);
                            }
                        }
                    }
                }
                0x2 => {
                    latest.binary_messages.fetch_add(1, Ordering::Relaxed);
                }
                0x8 => return Ok(()), // close
                _ => {}
            }
        }
        Ok(())
    }
}

// =====================================================================
// OS 由来のサンプル(RSS / fd / 空きディスク)
// =====================================================================

/// `ps` 1 回で全プロセスの RSS[KiB] を採る(pid -> KiB)。
fn rss_kib(pids: &[u32]) -> BTreeMap<u32, u64> {
    let mut out = BTreeMap::new();
    if pids.is_empty() {
        return out;
    }
    let list = pids
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let Ok(result) = Command::new("ps")
        .args(["-o", "pid=,rss=", "-p", &list])
        .output()
    else {
        return out;
    };
    for line in String::from_utf8_lossy(&result.stdout).lines() {
        let mut it = line.split_whitespace();
        if let (Some(pid), Some(rss)) = (it.next(), it.next()) {
            if let (Ok(pid), Ok(rss)) = (pid.parse::<u32>(), rss.parse::<u64>()) {
                out.insert(pid, rss);
            }
        }
    }
    out
}

/// `lsof` 1 回で全プロセスの open fd 数を採る(pid -> 本数)。
///
/// `-F pf` は「p<pid>」に続いて「f<fd>」を並べる機械可読形式。lsof が無い環境では
/// 空の map を返す(その列は空欄になる —— silent に 0 と嘘をつかない)。
fn open_fds(pids: &[u32]) -> BTreeMap<u32, u64> {
    let mut out = BTreeMap::new();
    if pids.is_empty() {
        return out;
    }
    let list = pids
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let Ok(result) = Command::new("lsof")
        .args(["-p", &list, "-F", "pf"])
        .output()
    else {
        return out;
    };
    let mut current: Option<u32> = None;
    for line in String::from_utf8_lossy(&result.stdout).lines() {
        let Some((tag, rest)) = line.split_at_checked(1) else {
            continue;
        };
        match tag {
            "p" => current = rest.parse::<u32>().ok(),
            "f" => {
                if let Some(pid) = current {
                    *out.entry(pid).or_insert(0) += 1;
                }
            }
            _ => {}
        }
    }
    out
}

/// `df -k` の Available[KiB] → GiB。
fn free_gib(path: &Path) -> f64 {
    let Ok(result) = Command::new("df").arg("-k").arg(path).output() else {
        return f64::NAN;
    };
    let text = String::from_utf8_lossy(&result.stdout);
    let Some(line) = text.lines().nth(1) else {
        return f64::NAN;
    };
    line.split_whitespace()
        .nth(3)
        .and_then(|s| s.parse::<f64>().ok())
        .map(|kib| kib / 1024.0 / 1024.0)
        .unwrap_or(f64::NAN)
}

fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// `suffix` で終わる名前のファイルを除いた総バイト数(ROOT を数えないため)。
fn dir_bytes_excluding(dir: &Path, suffix: &str) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .filter_map(|e| e.ok())
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_bytes_excluding(&e.path(), suffix),
            Ok(_) if e.file_name().to_string_lossy().ends_with(suffix) => 0,
            Ok(_) => e.metadata().map(|m| m.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}

// =====================================================================
// CSV(判定の一次データ)
// =====================================================================

/// CSV の列名。**プロセス毎の RSS/fd を先に、次に全ロスレスカウンタ**。
fn csv_header(procs: &[String]) -> String {
    let mut cols = vec![
        "ts_unix".to_string(),
        "elapsed_s".to_string(),
        "run_index".to_string(),
        "run_number".to_string(),
    ];
    for p in procs {
        cols.push(format!("rss_kib_{p}"));
    }
    for p in procs {
        cols.push(format!("fd_{p}"));
    }
    cols.extend(COUNTER_COLUMNS.iter().map(|c| c.0.to_string()));
    cols.push("free_gib".to_string());
    cols.join(",")
}

/// (列名, 出どころ). 出どころ = "<source>.<key>"。
/// source: recv / dec / gw = controller /api/status の components、rs / mon = monitor WS status。
const COUNTER_COLUMNS: [(&str, &str); 26] = [
    // receiver(SPEC §1.4: 保存系の入り口。overflow_frames が「落とした」の唯一の印)
    ("recv_bytes", "recv.bytes"),
    ("recv_frames", "recv.frames"),
    ("recv_overflow_frames", "recv.overflow_frames"),
    ("recv_framer_resets", "recv.framer_resets"),
    ("recv_messages_abandoned", "recv.messages_abandoned"),
    ("recv_encode_errors", "recv.encode_errors"),
    ("recv_send_errors", "recv.send_errors"),
    ("recv_extra_connections", "recv.extra_connections"),
    ("recv_empty_connections", "recv.empty_connections"),
    // decoder
    ("dec_frames_in", "dec.frames_in"),
    ("dec_fragments_out", "dec.fragments_out"),
    ("dec_malformed", "dec.malformed"),
    ("dec_seq_gaps", "dec.seq_gaps"),
    ("dec_run_mismatches", "dec.run_mismatches"),
    ("dec_batches_abandoned", "dec.batches_abandoned"),
    ("dec_eos_abandoned", "dec.eos_abandoned"),
    ("dec_cobo_mismatch", "dec.cobo_mismatch"),
    // graw-writer
    ("gw_seq_gaps", "gw.seq_gaps"),
    ("gw_run_mismatches", "gw.run_mismatches"),
    ("gw_write_errors", "gw.write_errors"),
    ("gw_unexpected_sources", "gw.unexpected_sources"),
    ("gw_decode_errors", "gw.decode_errors"),
    // root-sink(WS status = SPEC §5.3 そのまま)
    ("rs_events_built", "rs.events_built"),
    ("rs_events_incomplete", "rs.events_incomplete"),
    ("rs_late_fragments", "rs.late_fragments"),
    ("rs_pending_events", "rs.pending_events"),
];

/// モニタ系 drop(**不合格にはしないが必ず記録する** —— CLAUDE.md の絶対ルール)。
const MONITOR_COLUMNS: [(&str, &str); 4] = [
    ("mon_publish_drops", "rs.publish_drops"),
    ("mon_gaps", "mon.monitorGaps"),
    ("mon_ws_dropped", "mon.wsDropped"),
    ("mon_clients", "mon.clients"),
];

/// ロスレス系で **0 でなければ即不合格**の列(SPEC §12-5「全カウンタ 0」)。
const MUST_BE_ZERO: [&str; 16] = [
    "recv_overflow_frames",
    "recv_framer_resets",
    "recv_messages_abandoned",
    "recv_encode_errors",
    "recv_send_errors",
    "recv_extra_connections",
    "dec_malformed",
    "dec_seq_gaps",
    "dec_run_mismatches",
    "dec_batches_abandoned",
    "dec_eos_abandoned",
    "dec_cobo_mismatch",
    "gw_seq_gaps",
    "gw_run_mismatches",
    "gw_write_errors",
    "gw_unexpected_sources",
];

/// 1 サンプル分の値(列名 -> 数値)。
type Sample = BTreeMap<String, f64>;

// =====================================================================
// 本体
// =====================================================================

struct Stack {
    procs: Vec<Proc>,
    rest: SocketAddr,
    data_root: PathBuf,
    log_dir: PathBuf,
}

impl Stack {
    fn names(&self) -> Vec<String> {
        self.procs.iter().map(|p| p.name.clone()).collect()
    }

    fn pids(&self) -> Vec<u32> {
        self.procs.iter().map(|p| p.pid).collect()
    }

    fn get(&mut self, name: &str) -> Option<&mut Proc> {
        self.procs.iter_mut().find(|p| p.name == name)
    }

    fn dead(&mut self) -> Option<String> {
        for proc in &mut self.procs {
            if !proc.alive() {
                return Some(format!(
                    "{} が走行中に落ちた(log = {})",
                    proc.name,
                    proc.log.display()
                ));
            }
        }
        None
    }
}

fn run(cfg: &args::Args) -> Result<String, String> {
    let inputs = inputs(cfg)?;
    let out_dir = match &cfg.out_dir {
        Some(dir) => dir.clone(),
        None => std::env::temp_dir().join(format!("tpcdaq_soak_{}", std::process::id())),
    };
    std::fs::create_dir_all(&out_dir).map_err(|e| format!("create {}: {e}", out_dir.display()))?;
    let data_root = out_dir.join("data");
    let log_dir = out_dir.join("logs");
    std::fs::create_dir_all(&data_root).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&log_dir).map_err(|e| e.to_string())?;

    // --- 起動時のディスク見積もり(silent 失敗禁止)---------------------------
    let free = free_gib(&data_root);
    if cfg.rate_mbps > 0.0 {
        let bytes_per_run = cfg.rate_mbps / 8.0 * 1e6 * cfg.run_seconds;
        let need_gib = bytes_per_run * (1.0 + ROOT_TO_GRAW_RATIO) * 2.0 / 1024.0 / 1024.0 / 1024.0;
        if free.is_finite() && free < need_gib {
            return Err(format!(
                "空きディスクが 2 run 分に足りない: 空き {free:.1} GiB < 必要 {need_gib:.1} GiB\n\
                 (1 run = {:.1} 分 @ {} Mbps、ROOT は入力の {ROOT_TO_GRAW_RATIO}× で見積もり)",
                cfg.run_seconds / 60.0,
                cfg.rate_mbps
            ));
        }
    }
    if free.is_finite() && free < cfg.min_free_gib as f64 {
        return Err(format!(
            "空きディスク {free:.1} GiB が床 {} GiB を下回っている",
            cfg.min_free_gib
        ));
    }
    println!(
        "soak_harness: mode={:?} 走行 {:.3} h / 1 run {:.3} min / rate {} / out {}",
        cfg.mode,
        cfg.duration_s / 3600.0,
        cfg.run_seconds / 60.0,
        if cfg.rate_mbps > 0.0 {
            format!("{} Mbps", cfg.rate_mbps)
        } else {
            "全速(ペーシングなし)".to_string()
        },
        out_dir.display()
    );

    let mut stack = start_stack(cfg, &inputs, &out_dir, &data_root, &log_dir)?;

    // --- WS probe(モニタ経路に本物のクライアントを 1 本)----------------------
    let stop = Arc::new(AtomicBool::new(false));
    let latest = ws::Latest::new();
    ws::spawn(
        monitor_ws_addr(&mut stack)?,
        Arc::clone(&latest),
        Arc::clone(&stop),
    );
    std::thread::sleep(Duration::from_millis(500));

    // --- サンプラ(CSV へ 1 行 / interval)------------------------------------
    let csv_path = out_dir.join("metrics.csv");
    let names = stack.names();
    let pids = stack.pids();
    let run_index = Arc::new(AtomicU64::new(0));
    let run_number = Arc::new(AtomicU64::new(0));
    let sampler = spawn_sampler(
        csv_path.clone(),
        names.clone(),
        pids,
        stack.rest,
        Arc::clone(&latest),
        Arc::clone(&run_index),
        Arc::clone(&run_number),
        Arc::clone(&stop),
        cfg.metrics_interval_s,
        data_root.clone(),
        cfg.min_free_gib,
    )?;

    // --- SIGINT = graceful(TODO/053-D。一晩走行の運用要件)---------------------
    // 「今の run を**完走**してから report を書いて exit 0」。走行中の run を切ると
    // framer が途中で切れて「全カウンタ 0」が壊れる = 証拠が濁る。
    let interrupted = Arc::new(AtomicBool::new(false));
    install_sigint(Arc::clone(&interrupted));

    // --- run 反復 -------------------------------------------------------------
    let mut results: Vec<RunResult> = Vec::new();
    let outcome = run_loop(
        cfg,
        &inputs,
        &mut stack,
        &latest,
        &run_index,
        &run_number,
        Arc::clone(&stop),
        Arc::clone(&interrupted),
        &mut results,
    );

    stop.store(true, Ordering::Relaxed);
    let _ = sampler.join();

    // --- 畳む(root_sink の終了 JSON と monitor の終了ログを回収する)----------
    let shutdown = shutdown_stack(&mut stack);
    // **失敗しても、そこまでに合格した run はレポートに出す**(証拠第一)。
    let laps_total = results.iter().map(|r| r.laps).sum::<u64>();
    let report = report(
        cfg,
        &csv_path,
        &names,
        &results,
        &shutdown,
        laps_total,
        interrupted.load(Ordering::Relaxed),
    );
    std::fs::write(out_dir.join("report.txt"), &report).map_err(|e| e.to_string())?;

    match outcome {
        Ok(()) => {
            // 全カウンタの最終確認は終了 JSON でもう一度(run 毎の照合と二重に張る)。
            verify_final(&shutdown, laps_total)?;
            Ok(report)
        }
        Err(msg) => {
            eprintln!("{report}");
            Err(msg)
        }
    }
}

// ---------------------------------------------------------------------
// スタック起動
// ---------------------------------------------------------------------

fn start_stack(
    cfg: &args::Args,
    inputs: &Inputs,
    out_dir: &Path,
    data_root: &Path,
    log_dir: &Path,
) -> Result<Stack, String> {
    let config_path = out_dir.join("soak.toml");
    std::fs::write(&config_path, config_toml(inputs, data_root))
        .map_err(|e| format!("write {}: {e}", config_path.display()))?;

    let mut procs: Vec<Proc> = Vec::new();

    // 1. root-sink(下流から。monitor の SUB は slow joiner なので PUB を先に上げる)
    let mut cmd = Command::new(&inputs.root_sink);
    cmd.args([
        "--bind",
        "tcp://*:47003",
        "--pub",
        "tcp://*:47004",
        "--geometry",
        &inputs.geometry.to_string_lossy(),
        "--output-root",
        &data_root.to_string_lossy(),
        "--expect",
        "0:0",
    ]);
    let mut root_sink = Proc::spawn("root_sink", log_dir, cmd)?;
    root_sink.wait_for_log("monitor PUB bind", Duration::from_secs(30))?;
    procs.push(root_sink);

    // 2. monitor(WS)
    let mut cmd = Command::new(inputs.bin_dir.join("monitor"));
    cmd.arg("--config").arg(&config_path);
    let mut monitor = Proc::spawn("monitor", log_dir, cmd)?;
    monitor.wait_for_log("monitor WS listening", Duration::from_secs(30))?;
    procs.push(monitor);
    std::thread::sleep(Duration::from_millis(400)); // SUB の slow-joiner マージン

    // 3. graw-writer / decoder / receiver
    for (name, bin, extra) in [
        ("graw_writer", "graw_writer", Vec::new()),
        ("decoder", "decoder", Vec::new()),
        ("receiver0", "receiver", vec!["--cobo-id", "0"]),
    ] {
        let mut cmd = Command::new(inputs.bin_dir.join(bin));
        cmd.arg("--config").arg(&config_path).args(&extra);
        let mut proc = Proc::spawn(name, log_dir, cmd)?;
        proc.wait_for_log("command socket listening", Duration::from_secs(30))?;
        procs.push(proc);
    }

    // 4. fake-ECC → ecc-bridge(制御プレーンは実配線 —— 030 裁定①)
    let mut cmd = Command::new(&inputs.fake_ecc);
    cmd.args(["--port", "0", "--no-data-link"]);
    let mut fake_ecc = Proc::spawn("fake_ecc", log_dir, cmd)?;
    let proxy_line = fake_ecc.wait_for_log("PROXY ", Duration::from_secs(30))?;
    let proxy = proxy_line
        .split_once("PROXY ")
        .map(|(_, rest)| rest.trim().to_string())
        .ok_or_else(|| format!("fake_ecc の PROXY 行が読めない: {proxy_line:?}"))?;
    procs.push(fake_ecc);

    let mut cmd = Command::new(&inputs.ecc_bridge);
    cmd.args(["--bind", "tcp://*:47200", "--ecc-proxy", &proxy]);
    let mut bridge = Proc::spawn("ecc_bridge", log_dir, cmd)?;
    bridge.wait_for_log("BIND ", Duration::from_secs(30))?;
    procs.push(bridge);

    // 5. controller(REST)
    let mut cmd = Command::new(inputs.bin_dir.join("controller"));
    cmd.arg("--config").arg(&config_path);
    let mut controller = Proc::spawn("controller", log_dir, cmd)?;
    controller.wait_for_log("controller REST listening", Duration::from_secs(30))?;
    procs.push(controller);

    let rest: SocketAddr = REST_LISTEN
        .parse()
        .map_err(|e| format!("REST listen address: {e}"))?;
    let _ = cfg;
    Ok(Stack {
        procs,
        rest,
        data_root: data_root.to_path_buf(),
        log_dir: log_dir.to_path_buf(),
    })
}

/// SPEC §3.2 の既定ポートをそのまま使う(耐久走行は専有マシン前提。E2E のような
/// 動的ポート化は `--config` 起動では効かない —— コマンド REP が定数だから)。
const REST_LISTEN: &str = "127.0.0.1:18080";
const WS_LISTEN: &str = "127.0.0.1:19000";
const COBO_LISTEN: &str = "127.0.0.1:46005";

fn config_toml(inputs: &Inputs, data_root: &Path) -> String {
    format!(
        r#"# soak_harness が生成(手で直さない)。SPEC §3.1。
[system]
experiment = "mini_eTPC (soak)"
output_root = "{output_root}"
geometry = "{geometry}"

[[cobo]]
id = 0
listen = "{cobo_listen}"
data_sender_id = "CoBo[0]"

[decoder]
workers = 1

[root_sink]
snapshot_hz = 1.0
event_publish_hz = 20.0
build_timeout_ms = 1000

[monitor]
ws_listen = "{ws_listen}"

[controller]
rest_listen = "{rest_listen}"
passphrase = "{passphrase}"
ecc_proxy = "unused-by-the-controller"
config_id = "soak"
router_ip = "127.0.0.1"
"#,
        output_root = data_root.display(),
        geometry = inputs.geometry.display(),
        cobo_listen = COBO_LISTEN,
        ws_listen = WS_LISTEN,
        rest_listen = REST_LISTEN,
        passphrase = PASSPHRASE,
    )
}

fn monitor_ws_addr(stack: &mut Stack) -> Result<SocketAddr, String> {
    let _ = stack;
    WS_LISTEN
        .parse()
        .map_err(|e| format!("monitor WS address: {e}"))
}

// ---------------------------------------------------------------------
// サンプラ
// ---------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn spawn_sampler(
    csv_path: PathBuf,
    names: Vec<String>,
    pids: Vec<u32>,
    rest: SocketAddr,
    latest: Arc<ws::Latest>,
    run_index: Arc<AtomicU64>,
    run_number: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    interval_s: u64,
    data_root: PathBuf,
    min_free_gib: u64,
) -> Result<std::thread::JoinHandle<()>, String> {
    let mut header = csv_header(&names);
    for (name, _) in MONITOR_COLUMNS {
        let _ = write!(header, ",{name}");
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&csv_path)
        .map_err(|e| format!("open {}: {e}", csv_path.display()))?;
    writeln!(file, "{header}").map_err(|e| e.to_string())?;
    file.flush().map_err(|e| e.to_string())?;

    let started = Instant::now();
    Ok(std::thread::spawn(move || {
        let mut next = Instant::now();
        while !stop.load(Ordering::Relaxed) {
            if Instant::now() >= next {
                next += Duration::from_secs(interval_s);
                let row = sample_row(
                    started,
                    &names,
                    &pids,
                    rest,
                    &latest,
                    run_index.load(Ordering::Relaxed),
                    run_number.load(Ordering::Relaxed),
                    &data_root,
                );
                if writeln!(file, "{row}").is_err() || file.flush().is_err() {
                    eprintln!("soak_harness: metrics CSV への追記に失敗した");
                    return;
                }
                // 走行中のディスク床(silent にディスクを食い潰さない)。
                // 現 run はフレーム境界を切らずに走り切らせ、次の run へ進ませない。
                let free = free_gib(&data_root);
                if free.is_finite() && free < min_free_gib as f64 {
                    eprintln!(
                        "soak_harness: 空きディスク {free:.1} GiB が床 {min_free_gib} GiB を \
                         割った —— 現 run を最後に停止する"
                    );
                    stop.store(true, Ordering::Relaxed);
                }
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }))
}

#[allow(clippy::too_many_arguments)]
fn sample_row(
    started: Instant,
    names: &[String],
    pids: &[u32],
    rest: SocketAddr,
    latest: &ws::Latest,
    run_index: u64,
    run_number: u64,
    data_root: &Path,
) -> String {
    let rss = rss_kib(pids);
    let fds = open_fds(pids);
    let status = http("GET", rest, "/api/status", None)
        .map(|(_, v)| v)
        .unwrap_or(Value::Null);
    let ws_status = latest.status.lock().ok().and_then(|s| s.clone());

    let mut cells = vec![
        format!("{:.3}", unix_now()),
        format!("{:.1}", started.elapsed().as_secs_f64()),
        run_index.to_string(),
        run_number.to_string(),
    ];
    for pid in pids {
        cells.push(rss.get(pid).map(|v| v.to_string()).unwrap_or_default());
    }
    for pid in pids {
        cells.push(fds.get(pid).map(|v| v.to_string()).unwrap_or_default());
    }
    for (_, source) in COUNTER_COLUMNS {
        cells.push(counter_cell(source, &status, ws_status.as_ref()));
    }
    cells.push(format!("{:.2}", free_gib(data_root)));
    for (_, source) in MONITOR_COLUMNS {
        cells.push(counter_cell(source, &status, ws_status.as_ref()));
    }
    let _ = names;
    cells.join(",")
}

/// `"<source>.<key>"` を実際の JSON から引く。取れなければ**空欄**にする
/// (0 と嘘をつかない —— silent failure 禁止)。
fn counter_cell(source: &str, status: &Value, ws_status: Option<&Value>) -> String {
    let Some((which, key)) = source.split_once('.') else {
        return String::new();
    };
    let value = match which {
        "recv" => component_metric(status, "receiver0", key),
        "dec" => component_metric(status, "decoder", key),
        "gw" => component_metric(status, "graw-writer", key),
        "rs" | "mon" => ws_status.and_then(|s| s.get(key).and_then(Value::as_u64)),
        _ => None,
    };
    value.map(|v| v.to_string()).unwrap_or_default()
}

/// 3 コンポーネントすべてが metrics を返している `/api/status` を取る。
///
/// run 停止の直後は graw-writer が 60 MB 級の flush+fsync に入っていることがあり、
/// controller の `status_timeout` を割って **その 1 個だけ metrics が欠けた** 応答が返る。
/// 欠けたまま「カウンタ 0」と判定するのは silent failure なので、揃うまで撃ち直す。
fn status_with_all_metrics(rest: SocketAddr, timeout: Duration) -> Result<Value, String> {
    let deadline = Instant::now() + timeout;
    let mut last = Value::Null;
    loop {
        if let Ok((_, status)) = http("GET", rest, "/api/status", None) {
            let complete = ["receiver0", "decoder", "graw-writer"].iter().all(|name| {
                component_metric(&status, name, "heartbeats_in").is_some()
                    || component_metric(&status, name, "frames").is_some()
            });
            if complete {
                return Ok(status);
            }
            last = status;
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "/api/status が {timeout:?} 以内に 3 コンポーネント分の metrics を揃えなかった: {last}"
            ));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn component_metric(status: &Value, name: &str, key: &str) -> Option<u64> {
    status["components"]
        .as_array()?
        .iter()
        .find(|c| c["name"] == name)?["metrics"]
        .get(key)?
        .as_u64()
}

// ---------------------------------------------------------------------
// run 反復
// ---------------------------------------------------------------------

struct RunResult {
    run: u32,
    laps: u64,
    replayed_bytes: u64,
    graw_bytes: u64,
    entries: u64,
    seconds: f64,
}

/// SIGINT を「現 run を完走してから畳む」合図に変える(TODO/053-D)。
///
/// tokio は既に依存にあり `signal` feature も入っている —— 新依存も `unsafe` も足さずに
/// シグナルを取れる唯一の手段がこれ(std にシグナル API は無い)。ハーネス本体は
/// 同期コードなので、**専用スレッドの current-thread ランタイム**で 1 回だけ待つ。
///
/// **SIGTERM / SIGKILL は従来どおり即死**(暴走時の逃げ道を塞がない)。
fn install_sigint(flag: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                // 黙って「Ctrl-C が効かない」状態にしない(CLAUDE.md)。
                eprintln!("soak_harness: SIGINT ハンドラを張れなかった({e})—— Ctrl-C は即死のまま");
                return;
            }
        };
        rt.block_on(async {
            if tokio::signal::ctrl_c().await.is_ok() {
                flag.store(true, Ordering::Relaxed);
                eprintln!(
                    "soak_harness: SIGINT —— 現 run を完走してから report を書いて終了する\
                     (即座に止めるなら SIGTERM)"
                );
            }
        });
    });
}

#[allow(clippy::too_many_arguments)]
fn run_loop(
    cfg: &args::Args,
    inputs: &Inputs,
    stack: &mut Stack,
    latest: &ws::Latest,
    run_index: &AtomicU64,
    run_number: &AtomicU64,
    stop: Arc<AtomicBool>,
    interrupted: Arc<AtomicBool>,
    results: &mut Vec<RunResult>,
) -> Result<(), String> {
    let token = post(
        stack.rest,
        "/api/control/acquire",
        format!(r#"{{"operator":"{OPERATOR}","passphrase":"{PASSPHRASE}"}}"#),
    )?["token"]
        .as_str()
        .ok_or_else(|| "acquire が token を返さない".to_string())?
        .to_string();

    let started = Instant::now();
    let mut index = 0u64;

    while started.elapsed().as_secs_f64() < cfg.duration_s && !interrupted.load(Ordering::Relaxed) {
        index += 1;
        run_index.store(index, Ordering::Relaxed);
        // 残り時間が 1 run より短いなら、その分だけ回して打ち切る(尻切れを作らない)。
        let remaining = cfg.duration_s - started.elapsed().as_secs_f64();
        let run_seconds = cfg.run_seconds.min(remaining.max(1.0));

        let result = one_run(cfg, inputs, stack, &token, run_seconds, run_number)?;
        eprintln!(
            "soak_harness: run {} 合格(laps={} / graw={} B / entries={} / {:.1} s / 経過 {:.2} h)",
            result.run,
            result.laps,
            result.graw_bytes,
            result.entries,
            result.seconds,
            started.elapsed().as_secs_f64() / 3600.0
        );

        // 走行中の健全性(プロセス死・RSS 上限・ディスク床)。
        if let Some(dead) = stack.dead() {
            return Err(dead);
        }
        check_rss(stack, cfg.rss_limit_mib)?;
        let free = free_gib(&stack.data_root);
        if free.is_finite() && free < cfg.min_free_gib as f64 {
            return Err(format!(
                "空きディスク {free:.1} GiB が床 {} GiB を割った(run {} の直後)",
                cfg.min_free_gib, result.run
            ));
        }

        if !cfg.keep_outputs {
            let dir = stack.data_root.join(format!("run{:04}", result.run));
            std::fs::remove_dir_all(&dir).map_err(|e| format!("remove {}: {e}", dir.display()))?;
        }
        results.push(result);
        if interrupted.load(Ordering::Relaxed) {
            eprintln!("soak_harness: SIGINT —— run {index} を完走したのでここで畳む");
            break;
        }
        if stop.load(Ordering::Relaxed) || cfg.max_runs.is_some_and(|n| index >= n) {
            break;
        }
    }
    // WS probe が本当に繋がって読んでいたか(モニタ経路を測った証拠)。
    if latest.text_messages.load(Ordering::Relaxed) == 0 {
        return Err("WS probe が 1 通も status を受けていない(モニタ経路が死んでいる)".to_string());
    }
    Ok(())
}

fn one_run(
    cfg: &args::Args,
    inputs: &Inputs,
    stack: &mut Stack,
    token: &str,
    run_seconds: f64,
    run_number: &AtomicU64,
) -> Result<RunResult, String> {
    let begin = Instant::now();
    let body = post(
        stack.rest,
        "/api/run/start",
        format!(r#"{{"token":"{token}","comment":"soak"}}"#),
    )?;
    let run = body["run"]
        .as_u64()
        .ok_or_else(|| format!("run/start に run 番号が無い: {body}"))? as u32;
    run_number.store(run as u64, Ordering::Relaxed);

    // receiver が Arm で実際に bind したデータポート(固定値を書かない)。
    let (_, status) = http("GET", stack.rest, "/api/status", None)?;
    let address = status["components"]
        .as_array()
        .and_then(|cs| cs.iter().find(|c| c["name"] == "receiver0"))
        .and_then(|c| c["metrics"]["bind_address"].as_str())
        .ok_or_else(|| format!("receiver0 の bind_address が無い(Arm 済みか): {status}"))?
        .to_string();

    // --- graw_replay(lap 毎に eventIdx を進める = 周回で duplicate にならない)---
    let mut cmd = Command::new(inputs.bin_dir.join("graw_replay"));
    cmd.arg(&address)
        .arg(&inputs.graw)
        .arg("--laps-until-s")
        .arg(format!("{run_seconds:.3}"));
    if cfg.rate_mbps > 0.0 {
        cmd.arg("--rate-mbps").arg(cfg.rate_mbps.to_string());
    }
    let replay = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("spawn graw_replay: {e}"))?;
    if !replay.status.success() {
        return Err(format!("graw_replay が失敗した: {:?}", replay.status));
    }
    // "replayed N bytes to <addr>" の N が **周回込みの実送出量**。
    let replayed_bytes: u64 = String::from_utf8_lossy(&replay.stdout)
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| {
            format!(
                "graw_replay の stdout が読めない: {:?}",
                String::from_utf8_lossy(&replay.stdout)
            )
        })?;
    if !replayed_bytes.is_multiple_of(GRAW_BYTES_PER_LAP) {
        return Err(format!(
            "graw_replay が lap 境界で止まっていない: {replayed_bytes} B は \
             {GRAW_BYTES_PER_LAP} B の整数倍でない"
        ));
    }
    let laps = replayed_bytes / GRAW_BYTES_PER_LAP;
    if laps == 0 {
        return Err("1 周も送れていない(--run-minutes が短すぎる)".to_string());
    }

    // --- **停止の前に**全コンポーネントのカウンタを採る(030 E2E-C と同じ手順)---
    //
    // controller の run/stop は最後にコンポーネントを Idle へ戻す。graw-writer は
    // そこで RunWriter ごと畳むので、**停止後の GetStatus には自分のカウンタが載らない**
    // (metrics は `bind_address` だけになる — 2026-08-15 実測)。ロスレス照合の材料は
    // ここで確保しておく。
    let pre_stop = status_with_all_metrics(stack.rest, Duration::from_secs(30))?;

    // --- run 停止(graw_replay が閉じた = 自然 EOF。stop は後片付け)---
    let stop_body = post(
        stack.rest,
        "/api/run/stop",
        format!(r#"{{"token":"{token}"}}"#),
    )?;
    if stop_body["ok"] != Value::Bool(true) || stop_body["reason"] != Value::String("normal".into())
    {
        return Err(format!("run {run} の停止が正常でない: {stop_body}"));
    }

    verify_run(
        stack,
        run,
        laps,
        replayed_bytes,
        begin.elapsed().as_secs_f64(),
        &pre_stop,
    )
}

/// run 1 本分の照合(**合格したものだけ消す**)。
fn verify_run(
    stack: &mut Stack,
    run: u32,
    laps: u64,
    replayed_bytes: u64,
    seconds: f64,
    pre_stop: &Value,
) -> Result<RunResult, String> {
    let run_dir = stack.data_root.join(format!("run{run:04}"));
    let root_file = run_dir.join(format!("run{run:04}.root"));
    let monitor_file = run_dir.join(format!("run{run:04}_monitor.root"));

    // root-sink の run クローズと ROOT の finalize は EOS の後(非同期)。
    // **monitor.root が最後**に書かれる(SPEC §12-9 の R10 もこれを終点にしている)ので、
    // それを待ってから ROOT 側を数える。
    let sink_log = stack
        .get("root_sink")
        .ok_or_else(|| "root_sink が居ない".to_string())?
        .log
        .clone();
    if !wait_for_file(&monitor_file, Duration::from_secs(600)) {
        return Err(format!(
            "{} が出来なかった(log = {})",
            monitor_file.display(),
            sink_log.display()
        ));
    }
    if !root_file.is_file() {
        return Err(format!("{} が無い", root_file.display()));
    }

    // **ROOT は 1 GiB でパート分割される**(`run0001.root` / `run0001_0001.root` / …
    // —— 2026-08-15 実測。root_recorder のローテーション)。entries はパートの総和で見る。
    let prefix = run_dir.join(format!("run{run:04}")).display().to_string();
    let entries: u64 = log_lines_containing(&sink_log, "finalized ")
        .iter()
        .filter(|line| line.contains(&prefix))
        .map(|line| {
            line.split_once('(')
                .and_then(|(_, rest)| rest.split_whitespace().next())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0)
        })
        .sum();
    if entries != laps * EVENTS_PER_LAP {
        return Err(format!(
            "run {run}: ROOT entries = {entries}(期待 {} = {laps} laps × {EVENTS_PER_LAP})",
            laps * EVENTS_PER_LAP
        ));
    }

    // ROOT(パート分割 + monitor.root)を除いた全ファイル = 生 graw + ctrl/。
    let graw_bytes = dir_bytes_excluding(&run_dir, ".root");
    if graw_bytes != replayed_bytes {
        return Err(format!(
            "run {run}: 生 graw 出力 {graw_bytes} B ≠ 送出 {replayed_bytes} B \
             ({laps} laps × {GRAW_BYTES_PER_LAP})"
        ));
    }

    // 全ロスレスカウンタ 0(停止前に採ったスナップショット = その run の最終値)。
    let status = pre_stop;
    for name in MUST_BE_ZERO {
        let source = COUNTER_COLUMNS
            .iter()
            .find(|(col, _)| *col == name)
            .map(|(_, src)| *src)
            .ok_or_else(|| format!("列 {name} の出どころが未定義"))?;
        let cell = counter_cell(source, status, None);
        if cell.is_empty() {
            return Err(format!(
                "run {run}: カウンタ {name} が読めない(コンポーネント応答が欠けた)"
            ));
        }
        if cell != "0" {
            return Err(format!(
                "run {run}: ロスレスカウンタ {name} = {cell}(0 でなければ不合格)"
            ));
        }
    }

    // root-sink の FATAL は 1 行でも出ていたら不合格。
    let fatals = log_lines_containing(&sink_log, "root_sink: FATAL");
    if !fatals.is_empty() {
        return Err(format!("run {run}: root_sink FATAL: {fatals:?}"));
    }

    Ok(RunResult {
        run,
        laps,
        replayed_bytes,
        graw_bytes,
        entries,
        seconds,
    })
}

fn wait_for_file(path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.is_file() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    path.is_file()
}

fn check_rss(stack: &Stack, limit_mib: u64) -> Result<(), String> {
    let rss = rss_kib(&stack.pids());
    for proc in &stack.procs {
        if let Some(kib) = rss.get(&proc.pid) {
            if kib / 1024 > limit_mib {
                return Err(format!(
                    "{} の RSS が {} MiB(上限 {limit_mib} MiB)を超えた",
                    proc.name,
                    kib / 1024
                ));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// 停止と終了データの回収
// ---------------------------------------------------------------------

struct Shutdown {
    /// root_sink の終了 JSON(SPEC §6 の全カウンタ)。
    sink_counts: Value,
    /// monitor の "monitor stopped" 行(monitor_gaps / ws_dropped / clients_dropped_slow)。
    monitor_line: String,
}

fn shutdown_stack(stack: &mut Stack) -> Shutdown {
    // 上流から順に(receiver → decoder → graw-writer)畳み、root_sink は最後。
    for name in ["controller", "receiver0", "decoder", "graw_writer"] {
        if let Some(proc) = stack.get(name) {
            proc.stop("INT", Duration::from_secs(30));
        }
    }
    if let Some(proc) = stack.get("root_sink") {
        proc.stop("TERM", Duration::from_secs(120));
    }
    for name in ["monitor", "ecc_bridge", "fake_ecc"] {
        if let Some(proc) = stack.get(name) {
            proc.stop("INT", Duration::from_secs(30));
        }
    }

    let sink_log = stack.log_dir.join("root_sink.log");
    // 終了 JSON は stdout の最終行(ログには stderr と混ざるので `{"batches":` で拾う)。
    let sink_counts = log_lines_containing(&sink_log, "{\"batches\":")
        .last()
        .and_then(|line| serde_json::from_str(line).ok())
        .unwrap_or(Value::Null);
    let monitor_line = log_line_containing(&stack.log_dir.join("monitor.log"), "monitor stopped")
        .unwrap_or_default();
    Shutdown {
        sink_counts,
        monitor_line,
    }
}

/// 終了 JSON 側の最終確認(run 毎の照合と二重に張る — SPEC §12-5「全カウンタ 0」)。
fn verify_final(shutdown: &Shutdown, laps_total: u64) -> Result<(), String> {
    let counts = &shutdown.sink_counts;
    if counts.is_null() {
        return Err("root_sink の終了 JSON を回収できなかった".to_string());
    }
    for key in [
        "events_incomplete",
        "late_fragments",
        "unexpected_fragments",
        "duplicate_fragments",
        "unexpected_sources",
        "run_number_mismatch",
        "stale_eos",
        "unknown",
        "duplicate_event_ids",
        "items_out_of_range",
        "charge_keys_out_of_range",
        "frames_outside_geometry",
    ] {
        let value = counts[key]
            .as_u64()
            .ok_or_else(|| format!("終了 JSON に {key} が無い: {counts}"))?;
        if value != 0 {
            return Err(format!(
                "root_sink 終了 JSON の {key} = {value}(0 でなければ不合格)"
            ));
        }
    }
    if counts["fatal"] != Value::String(String::new()) {
        return Err(format!(
            "root_sink が fatal で終わった: {}",
            counts["fatal"]
        ));
    }
    let want = laps_total * EVENTS_PER_LAP;
    for key in ["events_complete", "entries_written"] {
        let got = counts[key].as_u64().unwrap_or(u64::MAX);
        if got != want {
            return Err(format!("root_sink 終了 JSON の {key} = {got}(期待 {want})"));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// レポート(判定は CSV から機械的に)
// ---------------------------------------------------------------------

/// CSV を読み戻して列毎の系列にする。
fn read_csv(path: &Path) -> (Vec<String>, Vec<Sample>, Vec<f64>) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return (Vec::new(), Vec::new(), Vec::new());
    };
    let mut lines = text.lines();
    let Some(header) = lines.next() else {
        return (Vec::new(), Vec::new(), Vec::new());
    };
    let columns: Vec<String> = header.split(',').map(|s| s.to_string()).collect();
    let mut samples = Vec::new();
    let mut elapsed = Vec::new();
    for line in lines {
        let cells: Vec<&str> = line.split(',').collect();
        if cells.len() != columns.len() {
            continue;
        }
        let mut sample = Sample::new();
        for (column, cell) in columns.iter().zip(&cells) {
            if let Ok(value) = cell.parse::<f64>() {
                sample.insert(column.clone(), value);
            }
        }
        elapsed.push(sample.get("elapsed_s").copied().unwrap_or(f64::NAN));
        samples.push(sample);
    }
    (columns, samples, elapsed)
}

fn series(samples: &[Sample], column: &str) -> Vec<f64> {
    samples
        .iter()
        .filter_map(|s| s.get(column).copied())
        .collect()
}

/// 最小二乗の傾き(単位 = 値/時間)。点が 2 未満なら NaN。
fn slope_per_hour(x_s: &[f64], y: &[f64]) -> f64 {
    let n = x_s.len().min(y.len());
    if n < 2 {
        return f64::NAN;
    }
    let xs: Vec<f64> = x_s[..n].iter().map(|s| s / 3600.0).collect();
    let mean_x = xs.iter().sum::<f64>() / n as f64;
    let mean_y = y[..n].iter().sum::<f64>() / n as f64;
    let mut num = 0.0;
    let mut den = 0.0;
    for i in 0..n {
        num += (xs[i] - mean_x) * (y[i] - mean_y);
        den += (xs[i] - mean_x).powi(2);
    }
    if den == 0.0 {
        f64::NAN
    } else {
        num / den
    }
}

fn mean_window(x_s: &[f64], y: &[f64], from: f64, to: f64) -> f64 {
    let mut sum = 0.0;
    let mut n = 0usize;
    for i in 0..x_s.len().min(y.len()) {
        if x_s[i] >= from && x_s[i] <= to {
            sum += y[i];
            n += 1;
        }
    }
    if n == 0 {
        f64::NAN
    } else {
        sum / n as f64
    }
}

/// SPEC §12-5 v1.15 の RSS 単調性判定。
///
/// 「走行後半の半分」で **後半の最初の窓の平均 ×1.05 ≥ 最後の窓の平均** を要求する
/// (v1.11 の「後半 12 h の最終 1 h 平均 ≤ 後半開始 1 h 平均 +5%」を、走行長 H に対して
/// 後半 = [H/2, H]、窓幅 = H/12 と一般化したもの。12 h 走なら窓 = 1 h で原式に一致する)。
struct RssVerdict {
    process: String,
    first_window: f64,
    last_window: f64,
    slope_kib_per_h: f64,
    ok: bool,
}

fn rss_verdicts(
    names: &[String],
    samples: &[Sample],
    elapsed: &[f64],
    duration_s: f64,
) -> Vec<RssVerdict> {
    let window = duration_s / 12.0;
    let half = duration_s / 2.0;
    names
        .iter()
        .map(|name| {
            let column = format!("rss_kib_{name}");
            let y = series(samples, &column);
            let x: Vec<f64> = elapsed[..elapsed.len().min(y.len())].to_vec();
            let first_window = mean_window(&x, &y, half, half + window);
            let last_window = mean_window(&x, &y, duration_s - window, f64::INFINITY);
            let ok = !(first_window.is_finite() && last_window.is_finite())
                || last_window <= first_window * 1.05;
            RssVerdict {
                process: name.clone(),
                first_window,
                last_window,
                slope_kib_per_h: slope_per_hour(&x, &y),
                ok,
            }
        })
        .collect()
}

fn report(
    cfg: &args::Args,
    csv_path: &Path,
    names: &[String],
    runs: &[RunResult],
    shutdown: &Shutdown,
    laps_total: u64,
    interrupted: bool,
) -> String {
    let (columns, samples, elapsed) = read_csv(csv_path);
    let mut out = String::new();
    let _ = writeln!(out, "===== soak_harness report =====");
    if interrupted {
        // 「予定時間まで走った」のか「SIGINT で切り上げた」のかがレポートだけで分かること。
        let _ = writeln!(
            out,
            "**SIGINT で打ち切り**(現 run を完走してから畳んだ —— 走行時間は予定より短い)"
        );
    }
    let _ = writeln!(
        out,
        "mode={:?} 走行予定={:.3} h / 1 run={:.3} min / rate={} Mbps(0=全速)",
        cfg.mode,
        cfg.duration_s / 3600.0,
        cfg.run_seconds / 60.0,
        cfg.rate_mbps
    );
    let _ = writeln!(out, "metrics CSV = {}", csv_path.display());
    let _ = writeln!(
        out,
        "サンプル数 = {} / 列数 = {}",
        samples.len(),
        columns.len()
    );

    let total_bytes: u64 = runs.iter().map(|r| r.replayed_bytes).sum();
    let total_seconds: f64 = runs.iter().map(|r| r.seconds).sum();
    let _ = writeln!(
        out,
        "\n-- run --\nrun 数 = {} / 総 laps = {laps_total} / 総送出 = {total_bytes} B \
         ({:.2} GiB) / 総 run 時間 = {:.1} s",
        runs.len(),
        total_bytes as f64 / 1024.0 / 1024.0 / 1024.0,
        total_seconds
    );
    if total_seconds > 0.0 {
        let mbps = total_bytes as f64 * 8.0 / total_seconds / 1e6;
        let _ = writeln!(
            out,
            "達成スループット = {:.1} Mbps({:.1} MB/s、100 Hz 相当 224 Mbps の {:.2}×)",
            mbps,
            total_bytes as f64 / total_seconds / 1e6,
            mbps / 224.0
        );
    }
    for r in runs {
        let _ = writeln!(
            out,
            "  run {:04}: laps={} bytes={} entries={} {:.1} s",
            r.run, r.laps, r.graw_bytes, r.entries, r.seconds
        );
    }

    let _ = writeln!(out, "\n-- 全系列(始値 / 終値 / 傾き per h)--");
    for column in &columns {
        if column == "ts_unix" || column == "elapsed_s" {
            continue;
        }
        let y = series(&samples, column);
        if y.is_empty() {
            let _ = writeln!(out, "  {column:<28} (データなし)");
            continue;
        }
        let first = y.first().copied().unwrap_or(f64::NAN);
        let last = y.last().copied().unwrap_or(f64::NAN);
        let _ = writeln!(
            out,
            "  {column:<28} {first:>14.1} -> {last:>14.1}   slope/h = {:>12.3}",
            slope_per_hour(&elapsed, &y)
        );
    }

    // 判定に使う H は **実測の走行長**(--runs で打ち切ったときに予定値だと窓が空になる)。
    let span = elapsed.last().copied().unwrap_or(cfg.duration_s);
    let _ = writeln!(
        out,
        "\n-- RSS 単調性(SPEC §12-5 v1.15: 後半 [H/2,H] の先頭窓 ×1.05 ≥ 末尾窓、窓 = H/12。\
         H = 実測 {span:.0} s)--"
    );
    for v in rss_verdicts(names, &samples, &elapsed, span) {
        let _ = writeln!(
            out,
            "  {:<12} 後半先頭窓 {:>10.0} KiB -> 末尾窓 {:>10.0} KiB  slope {:>10.1} KiB/h  {}",
            v.process,
            v.first_window,
            v.last_window,
            v.slope_kib_per_h,
            if v.ok {
                "OK"
            } else {
                "上昇トレンド(要調査)"
            }
        );
    }

    let _ = writeln!(out, "\n-- モニタ系 drop(落としてよいが silent にしない)--");
    for (name, _) in MONITOR_COLUMNS {
        let y = series(&samples, name);
        let _ = writeln!(
            out,
            "  {name:<20} 最終値 = {}",
            y.last()
                .map(|v| format!("{v:.0}"))
                .unwrap_or("(なし)".into())
        );
    }
    let _ = writeln!(out, "  monitor 終了ログ = {}", shutdown.monitor_line);

    let _ = writeln!(out, "\n-- root_sink 終了 JSON --\n{}", shutdown.sink_counts);
    out
}
