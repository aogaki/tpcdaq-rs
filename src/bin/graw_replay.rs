//! graw_replay — 記録済み `.graw` ファイルのバイト列をそのまま TCP で receiver へ送出する
//! リプレイツール(TODO/005)。検出器が無くても、記録済み .graw を tpcdaq の受信ポートへ
//! 再送して受信〜デコードを検証できる(C++ 版 `tools/graw_replay.cpp` の後継。
//! `--rate-mbps` によるペーシングと `--loop` を追加。SPEC §12 末尾)。
//!
//! 使い方: `graw_replay <host:port> <file.graw>... [--rate-mbps <f64>] [--loop] [--chunk-bytes <n=65536>]`
//!
//! - ファイル 1 つ: **バイトそのまま**チャンク送出(従来経路 — §12-2 のバイト一致検証が
//!   このツールの上に立っているので、再フレーミングを挟まない)。
//! - ファイル複数(TODO/021): per-AsAd に分かれた実ファイル群(例: ELITPC の
//!   `CoBo0_AsAd{0..3}_*.graw` 4 本組)を **eventIdx 昇順にインターリーブ**して 1 本の
//!   TCP ストリームで送る — 実機ワイヤ(DataRouter が受けた形)の再現。同一 eventIdx 内は
//!   引数で渡した順(= AsAd 昇順で渡せば AsAd 昇順)。ファイル逐次のリプレイでは
//!   イベントビルダのタイムアウトを正しく通せない(全イベントが数秒割れる)。
//!   eventIdx を持たない制御フレーム(frameType ∉ {1,2})は遭遇時にそのまま流す。
//! - `--loop` なし: 送り切ったら接続を閉じて終了(受信側は EOF = run 境界)。
//! - `--loop` あり: 送り切ったら先頭へ戻って送り続ける(Ctrl-C か受信側切断で停止)。
//! - 接続失敗・送出中の切断は明確なエラーメッセージ + 非 0 exit(silent failure を作らない)。

use std::fmt;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use tpcdaq::decode::peek_event_idx;
use tpcdaq::framer::Framer;

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
/// bin 内ユニットテストの対象は `args::parse` と `pace` の純粋関数側に絞る(KISS。
/// マージ経路の順序・内容は tests/graw_replay_tool.rs の実 TCP 統合テストで機械照合)。
fn run(cfg: &args::Args) -> Result<u64, ReplayError> {
    let mut stream = TcpStream::connect(&cfg.endpoint).map_err(|source| ReplayError::Connect {
        endpoint: cfg.endpoint.clone(),
        source,
    })?;

    let rate_bytes_per_sec = cfg
        .rate_mbps
        .map(pace::mbps_to_bytes_per_sec)
        .unwrap_or(0.0);
    let start = Instant::now();

    if cfg.laps.is_some() || cfg.laps_until_s.is_some() {
        run_laps(cfg, &mut stream, rate_bytes_per_sec, start)
    } else if cfg.files.len() == 1 {
        run_single(cfg, &mut stream, rate_bytes_per_sec, start)
    } else {
        run_merged(cfg, &mut stream, rate_bytes_per_sec, start)
    }
}

/// 単一ファイル: **バイトそのまま**チャンク送出(従来経路、再フレーミングなし)。
fn run_single(
    cfg: &args::Args,
    stream: &mut TcpStream,
    rate_bytes_per_sec: f64,
    start: Instant,
) -> Result<u64, ReplayError> {
    let path = &cfg.files[0];
    let mut file = File::open(path).map_err(|source| ReplayError::FileOpen {
        path: path.clone(),
        source,
    })?;

    let mut buf = vec![0u8; cfg.chunk_bytes];
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

/// 複数ファイル: eventIdx 昇順の k-way マージ(TODO/021)。ストリーミング(各ファイル
/// 先頭の 1 フレームだけをメモリに持つ)なので 1 GiB × 4 でも常駐は数 MB。
fn run_merged(
    cfg: &args::Args,
    stream: &mut TcpStream,
    rate_bytes_per_sec: f64,
    start: Instant,
) -> Result<u64, ReplayError> {
    let mut total: u64 = 0;
    loop {
        total += merge::replay_once(
            &cfg.files,
            cfg.chunk_bytes,
            &mut |frame: &[u8]| stream.write_all(frame).map_err(ReplayError::Send),
            &mut |sent_total: u64| {
                if rate_bytes_per_sec > 0.0 {
                    let wait =
                        pace::sleep_for(total + sent_total, rate_bytes_per_sec, start.elapsed());
                    if wait > Duration::ZERO {
                        std::thread::sleep(wait);
                    }
                }
            },
        )?;
        if !cfg.loop_replay {
            break;
        }
    }
    Ok(total)
}

/// lap モード(TODO/031): `--laps` / `--laps-until-s` のいずれかが与えられたときの経路。
/// 単一ファイルでも `merge::replay_once` のフレーム単位経路を通す(lap 0 は無変更のまま
/// 送出するので、フレーム整列済みの実 .graw なら出力バイト列は入力ファイルと完全一致する)。
///
/// lap 0 で data フレーム(`peek_event_idx` が `Some` を返すフレーム)の eventIdx 最大値
/// (`max_idx`)を実測し、lap L(L ≥ 1)ではヘッダ 22..26 へ `+ L * (max_idx + 1)` を
/// 加算して送出する。制御フレームは全 lap で無変更のまま送出する。
fn run_laps(
    cfg: &args::Args,
    stream: &mut TcpStream,
    rate_bytes_per_sec: f64,
    start: Instant,
) -> Result<u64, ReplayError> {
    let mut total: u64 = 0;
    let mut lap: u32 = 0;
    let mut max_idx: u32 = 0;

    loop {
        let is_first_lap = lap == 0;
        let mut lap_max_idx: u32 = 0;

        let sent = merge::replay_once(
            &cfg.files,
            cfg.chunk_bytes,
            &mut |frame: &[u8]| -> Result<(), ReplayError> {
                let Some(event_idx) = peek_event_idx(frame) else {
                    // 制御フレーム: 全 lap で無変更のまま送出。
                    return stream.write_all(frame).map_err(ReplayError::Send);
                };
                if is_first_lap {
                    // lap 0 は無変更で送出しつつ、後続 lap のオフセット計算用に最大値を実測する。
                    lap_max_idx = lap_max_idx.max(event_idx);
                    return stream.write_all(frame).map_err(ReplayError::Send);
                }

                let offset = u64::from(lap) * (u64::from(max_idx) + 1);
                let new_idx = (u64::from(event_idx) + offset) as u32;
                let little = frame[0] & 0x80 != 0;
                let mut patched = frame.to_vec();
                if little {
                    patched[22..26].copy_from_slice(&new_idx.to_le_bytes());
                } else {
                    patched[22..26].copy_from_slice(&new_idx.to_be_bytes());
                }
                stream.write_all(&patched).map_err(ReplayError::Send)
            },
            &mut |sent_total: u64| {
                if rate_bytes_per_sec > 0.0 {
                    let wait =
                        pace::sleep_for(total + sent_total, rate_bytes_per_sec, start.elapsed());
                    if wait > Duration::ZERO {
                        std::thread::sleep(wait);
                    }
                }
            },
        )?;

        total += sent;
        if is_first_lap {
            max_idx = lap_max_idx;
        }
        lap += 1;

        let laps_done = cfg.laps.is_some_and(|n| lap >= n);
        let time_done = cfg
            .laps_until_s
            .is_some_and(|s| start.elapsed().as_secs_f64() >= s);
        if laps_done || time_done {
            break;
        }
    }

    Ok(total)
}

/// eventIdx マージの本体(TODO/021)。
mod merge {
    use super::*;

    /// 1 ソース = 1 ファイル + Framer。`next_frame` はフレーム 1 本を所有 Vec で返す。
    struct FrameSource {
        path: PathBuf,
        file: File,
        framer: Framer,
        buf: Vec<u8>,
        eof: bool,
    }

    impl FrameSource {
        fn open(path: &PathBuf, chunk_bytes: usize) -> Result<Self, ReplayError> {
            let file = File::open(path).map_err(|source| ReplayError::FileOpen {
                path: path.clone(),
                source,
            })?;
            Ok(Self {
                path: path.clone(),
                file,
                framer: Framer::new(),
                buf: vec![0u8; chunk_bytes],
                eof: false,
            })
        }

        fn next_frame(&mut self) -> Result<Option<Vec<u8>>, ReplayError> {
            loop {
                if let Some(frame) = self.framer.next() {
                    return Ok(Some(frame.to_vec()));
                }
                if self.eof {
                    // フレームにならない末尾の切れ端は黙って捨てない(CLAUDE.md)。
                    let leftover = self.framer.buffered();
                    if leftover > 0 {
                        eprintln!(
                            "graw_replay: {}: {leftover} trailing bytes do not form a \
                             complete frame — not replayed",
                            self.path.display()
                        );
                    }
                    return Ok(None);
                }
                let n = self.file.read(&mut self.buf).map_err(ReplayError::Read)?;
                if n == 0 {
                    self.eof = true;
                    continue;
                }
                self.framer.push(&self.buf[..n]);
            }
        }
    }

    /// 各ソース先頭の 1 フレーム。`event = None` は制御フレーム(即時送出)。
    struct Head {
        frame: Vec<u8>,
        event: Option<u32>,
    }

    /// 全ファイルを 1 周インターリーブ送出して総バイト数を返す。
    /// `emit` = フレーム送出、`paced` = 送出後のペーシング(累計バイトを渡す)。
    pub fn replay_once(
        files: &[PathBuf],
        chunk_bytes: usize,
        emit: &mut dyn FnMut(&[u8]) -> Result<(), ReplayError>,
        paced: &mut dyn FnMut(u64),
    ) -> Result<u64, ReplayError> {
        let mut sources: Vec<FrameSource> = files
            .iter()
            .map(|p| FrameSource::open(p, chunk_bytes))
            .collect::<Result<_, _>>()?;
        let mut heads: Vec<Option<Head>> = Vec::with_capacity(sources.len());
        for src in &mut sources {
            heads.push(src.next_frame()?.map(|frame| Head {
                event: peek_event_idx(&frame),
                frame,
            }));
        }

        let mut total: u64 = 0;
        loop {
            // 制御フレーム(eventIdx なし)が先頭に居るソースは最優先でそのまま流す
            // (ファイル内の順序だけを保てばよい — 実機でも run 先頭に来るもの)。
            let pick = heads
                .iter()
                .position(|h| matches!(h, Some(Head { event: None, .. })))
                .or_else(|| {
                    // データフレームは (eventIdx, 引数順) の最小を選ぶ。
                    heads
                        .iter()
                        .enumerate()
                        .filter_map(|(i, h)| match h {
                            Some(Head {
                                event: Some(ev), ..
                            }) => Some((*ev, i)),
                            _ => None,
                        })
                        .min()
                        .map(|(_, i)| i)
                });
            let Some(i) = pick else {
                break; // 全ソース枯渇
            };

            let head = heads[i].take().expect("picked head exists");
            emit(&head.frame)?;
            total += head.frame.len() as u64;
            paced(total);

            heads[i] = sources[i].next_frame()?.map(|frame| Head {
                event: peek_event_idx(&frame),
                frame,
            });
        }
        Ok(total)
    }
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
        "usage: graw_replay <host:port> <file.graw>... [--rate-mbps <f64>] [--loop] \
         [--chunk-bytes <n=65536>] [--laps <n>] [--laps-until-s <f64>]\n\
         (複数ファイル = eventIdx 昇順インターリーブ送出。AsAd 昇順で渡すこと。\n\
         --laps/--laps-until-s は eventIdx を周回ごとに +lap*(max_idx+1) して送り直す \
         lap モード。--loop とは併用不可)";

    const DEFAULT_CHUNK_BYTES: usize = 65536;

    #[derive(Debug, Clone, PartialEq)]
    pub struct Args {
        pub endpoint: String,
        /// 1 つ = バイトそのまま送出(従来)。複数 = eventIdx マージ送出(TODO/021)。
        pub files: Vec<PathBuf>,
        pub rate_mbps: Option<f64>,
        pub loop_replay: bool,
        pub chunk_bytes: usize,
        /// TODO/031: N 周(lap 0..N-1)で終了する lap モード。
        pub laps: Option<u32>,
        /// TODO/031: S 秒経過後、現 lap を完走してから終了する lap モード。
        pub laps_until_s: Option<f64>,
    }

    /// `raw`(プログラム名を除いた argv)を解釈する。エラーメッセージは利用者向けの文字列。
    pub fn parse(raw: &[String]) -> Result<Args, String> {
        let mut positional: Vec<&String> = Vec::new();
        let mut rate_mbps: Option<f64> = None;
        let mut loop_replay = false;
        let mut chunk_bytes = DEFAULT_CHUNK_BYTES;
        let mut laps: Option<u32> = None;
        let mut laps_until_s: Option<f64> = None;

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
                "--laps" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "--laps requires a value".to_string())?;
                    let parsed: u32 = value
                        .parse()
                        .map_err(|_| format!("--laps value is not an integer: {value}"))?;
                    if parsed == 0 {
                        return Err("--laps must be greater than 0".to_string());
                    }
                    laps = Some(parsed);
                }
                "--laps-until-s" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "--laps-until-s requires a value".to_string())?;
                    let parsed: f64 = value
                        .parse()
                        .map_err(|_| format!("--laps-until-s value is not a number: {value}"))?;
                    if !(parsed.is_finite() && parsed > 0.0) {
                        return Err(format!(
                            "--laps-until-s must be a positive finite number, got {value}"
                        ));
                    }
                    laps_until_s = Some(parsed);
                }
                other if other.starts_with("--") => {
                    return Err(format!("unknown option: {other}"));
                }
                _ => positional.push(arg),
            }
        }

        if positional.len() < 2 {
            return Err(format!(
                "expected at least 2 positional arguments (host:port, file.graw...), got {}",
                positional.len()
            ));
        }

        if loop_replay && (laps.is_some() || laps_until_s.is_some()) {
            return Err(
                "--loop cannot be combined with --laps/--laps-until-s (conflicting semantics)"
                    .to_string(),
            );
        }

        Ok(Args {
            endpoint: positional[0].clone(),
            files: positional[1..].iter().map(PathBuf::from).collect(),
            rate_mbps,
            loop_replay,
            chunk_bytes,
            laps,
            laps_until_s,
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
                files: vec![PathBuf::from("run.graw")],
                rate_mbps: None,
                loop_replay: false,
                chunk_bytes: 65536,
                laps: None,
                laps_until_s: None,
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
                files: vec![PathBuf::from("run.graw")],
                rate_mbps: Some(28.0),
                loop_replay: true,
                chunk_bytes: 4096,
                laps: None,
                laps_until_s: None,
            }
        );
    }

    /// TODO/031: `--laps` は N 周を指す非ゼロの整数。単位や lap の意味は README/USAGE 参照。
    #[test]
    fn parse_accepts_laps_option() {
        let got = args::parse(&strs(&["h:1", "f", "--laps", "3"])).unwrap();
        assert_eq!(got.laps, Some(3));
        assert_eq!(got.laps_until_s, None);
    }

    /// TODO/031: `--laps-until-s` は正の秒数。
    #[test]
    fn parse_accepts_laps_until_s_option() {
        let got = args::parse(&strs(&["h:1", "f", "--laps-until-s", "1.5"])).unwrap();
        assert_eq!(got.laps_until_s, Some(1.5));
        assert_eq!(got.laps, None);
    }

    /// TODO/031: `--laps 0` は「0 周」という意味を持たないので引数エラー。
    #[test]
    fn parse_rejects_zero_laps() {
        assert!(args::parse(&strs(&["h:1", "f", "--laps", "0"])).is_err());
    }

    /// TODO/031: `--laps-until-s` は 0 以下・非有限を拒否する。
    #[test]
    fn parse_rejects_non_positive_laps_until_s() {
        assert!(args::parse(&strs(&["h:1", "f", "--laps-until-s", "0"])).is_err());
        assert!(args::parse(&strs(&["h:1", "f", "--laps-until-s", "-1.0"])).is_err());
        assert!(args::parse(&strs(&["h:1", "f", "--laps-until-s", "nan"])).is_err());
        assert!(args::parse(&strs(&["h:1", "f", "--laps-until-s", "inf"])).is_err());
    }

    /// TODO/031: `--loop` と lap オプションの併用は意味が衝突するので引数エラー。
    #[test]
    fn parse_rejects_loop_combined_with_laps() {
        assert!(args::parse(&strs(&["h:1", "f", "--loop", "--laps", "3"])).is_err());
        assert!(args::parse(&strs(&["h:1", "f", "--loop", "--laps-until-s", "1.0"])).is_err());
    }

    /// TODO/031: 値なしの lap オプションは他の値必須オプション同様に引数エラー。
    #[test]
    fn parse_rejects_dangling_laps_flags_without_value() {
        assert!(args::parse(&strs(&["h:1", "f", "--laps"])).is_err());
        assert!(args::parse(&strs(&["h:1", "f", "--laps-until-s"])).is_err());
    }

    /// TODO/021: 複数ファイル = マージ送出。引数順(= AsAd 昇順で渡す)が保持されること。
    #[test]
    fn parse_accepts_multiple_files_in_argument_order() {
        let got = args::parse(&strs(&["h:1", "a0.graw", "a1.graw", "a2.graw", "a3.graw"])).unwrap();
        assert_eq!(
            got.files,
            vec![
                PathBuf::from("a0.graw"),
                PathBuf::from("a1.graw"),
                PathBuf::from("a2.graw"),
                PathBuf::from("a3.graw"),
            ]
        );
    }

    #[test]
    fn parse_rejects_wrong_positional_count() {
        assert!(args::parse(&strs(&["only-one-arg"])).is_err());
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
