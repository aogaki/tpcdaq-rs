//! slowctl の全サンプルを InfluxDB v2 write API へ書く(TODO/122)。
//!
//! # 形
//!
//! ```text
//! device_loop ──offer(try_send)──▶ 有界チャネル ──▶ writer スレッド 1 本 ──POST──▶ InfluxDB
//!                                                     └ 再送バッファ(上限 buffer_max_points)
//! ```
//!
//! **モニタ系**(CLAUDE.md): 落としてよいが必ず数える。読み出しは決して止めない ——
//! 読み出しスレッドが触るのは [`InfluxSink::offer`] の `try_send` だけで、HTTP は writer
//! スレッドの中に閉じている。
//!
//! # 点(point)
//!
//! counters と `buffer_max_points` の「点」は **`Sample` 1 個**(= line protocol の field 1 個。
//! `alarm` field は数えない)。発注書の見積もり「≈ 18 値/s、100,000 点 ≈ 1.5 h」と同じ数え方。
//!
//! # 落とす所(すべて `influx_points_dropped` に数える)
//!
//! 1. チャネル満杯(writer が POST で塞がっている間に溢れた)—— [`InfluxSink::offer`]
//! 2. 再送バッファが上限を超えた —— **古い方から**
//! 3. NaN / inf —— line protocol に書けない(1 つ混ざるとバッチ全体が 400 で弾かれる)
//! 4. 停止時に書けずに残った分

use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tracing::{error, warn};

use super::{Sample, SlowCounters};
use crate::config::InfluxConfig;

/// 読み出しスレッド → writer のチャネル段数(1 段 = 1 poll ぶんの `Vec<Sample>`)。
///
/// writer は POST の最中(最長 `timeout_ms`)以外は常に吸い出すので、1 Hz 級の poll には
/// 桁で足りる(slowctl 本体の `EVENT_QUEUE` と同じ段数)。
const INFLUX_QUEUE: usize = 256;

/// 1 回の POST に載せる上限(点)。これだけ溜まれば `batch_ms` を待たずに POST する。
/// InfluxDB の推奨バッチ(5,000 行)に合わせた(1 行 ≥ 1 点なので行数はそれ以下)。
const BATCH_MAX_POINTS: usize = 5_000;

/// writer がチャネルを見る刻み(`batch_ms` の判定もこの粒度)。
const RECV_TICK: Duration = Duration::from_millis(50);

/// 失敗後の再試行間隔(指数バックオフ)の下限・上限。
const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(60);

/// 4xx / 5xx の本文(InfluxDB の `{"code","message"}`)を last_error に残す上限(バイト)。
const ERROR_BODY_LIMIT: u64 = 1024;

/// node の無いサンプル(機器全体の値)の `ch` タグ(発注書 §2)。
const BOARD_TAG: &str = "board";

/// line protocol で逃がす文字(InfluxDB v2 の line protocol 仕様)。
/// measurement はカンマと空白、tag / field の key はそれに等号が加わる。
const MEASUREMENT_SPECIAL: &[char] = &[',', ' '];
const KEY_SPECIAL: &[char] = &[',', '=', ' '];

// ---------------------------------------------------------------------
// パラメタ
// ---------------------------------------------------------------------

/// writer の起動パラメタ。**トークンを持つので `Debug` を付けない**(ログに漏らさない)。
pub struct InfluxParams {
    pub url: String,
    pub org: String,
    pub bucket: String,
    pub token: String,
    pub batch: Duration,
    pub buffer_max_points: usize,
    pub timeout: Duration,
}

impl InfluxParams {
    /// `[slowctl.influx]` から作る。**トークンを読むのはここだけ**(slowctl の起動時)。
    /// 読めなければ `Err` = slowctl の起動失敗。config の検証では読まない
    /// (controller / monitor を巻き込まない —— `InfluxConfig` の doc)。
    pub fn from_config(config: &InfluxConfig) -> Result<Self, String> {
        Ok(Self {
            url: config.url.clone(),
            org: config.org.clone(),
            bucket: config.bucket.clone(),
            token: read_token(&config.token_file)?,
            batch: Duration::from_millis(config.batch_ms),
            buffer_max_points: config.buffer_max_points,
            timeout: Duration::from_millis(config.timeout_ms),
        })
    }
}

/// InfluxDB のトークンファイルを読む(発注書 §4: 中身は 1 行、前後の空白は除く)。
///
/// 読めない・空なら `Err`(メッセージにパスを含める —— どのファイルを直せばよいか分かるように)。
pub fn read_token(path: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read token_file {}: {e}", path.display()))?;
    let token = text.trim();
    if token.is_empty() {
        return Err(format!("token_file {} is empty", path.display()));
    }
    Ok(token.to_string())
}

// ---------------------------------------------------------------------
// カウンタ(SPEC §2.3 の counters の influx_*)
// ---------------------------------------------------------------------

/// writer の累積カウンタ。読み出しスレッド(dropped)・writer・本体(PUB)が共有する。
#[derive(Debug, Default)]
pub struct InfluxCounters {
    points_written: AtomicU64,
    points_dropped: AtomicU64,
    write_errors: AtomicU64,
    /// 最後に失敗した POST の内容(成功しても消さない —— 「最後のエラー」の記録)。
    last_error: Mutex<Option<String>>,
}

impl InfluxCounters {
    /// status PUB の counters に influx_* を書き写す。
    pub fn write_into(&self, out: &mut SlowCounters) {
        out.influx_points_written = self.points_written.load(Ordering::Relaxed);
        out.influx_points_dropped = self.points_dropped.load(Ordering::Relaxed);
        out.influx_write_errors = self.write_errors.load(Ordering::Relaxed);
        out.influx_last_error = lock(&self.last_error).clone();
    }

    /// 落とした点を足し、足した後の累計を返す(warn に載せる)。
    fn add_dropped(&self, points: usize) -> u64 {
        let points = to_u64(points);
        self.points_dropped.fetch_add(points, Ordering::Relaxed) + points
    }

    fn add_written(&self, points: usize) {
        self.points_written
            .fetch_add(to_u64(points), Ordering::Relaxed);
    }

    fn record_error(&self, error: &WriteError) {
        self.write_errors.fetch_add(1, Ordering::Relaxed);
        *lock(&self.last_error) = Some(error.to_string());
    }
}

fn to_u64(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// 毒されたロックでも中身は読める(カウンタの表示を panic 1 つで止めない)。
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// ---------------------------------------------------------------------
// 読み出しスレッド側の口
// ---------------------------------------------------------------------

/// 読み出しスレッドが持つ投函口(複製して各デバイススレッドに配る)。
#[derive(Debug, Clone)]
pub struct InfluxSink {
    tx: SyncSender<Vec<Sample>>,
    counters: Arc<InfluxCounters>,
}

impl InfluxSink {
    /// 1 poll ぶんを writer に渡す。**決してブロックしない**(`try_send`)——
    /// 満杯なら落として数える(発注書 §3、モニタ系の規約)。
    pub fn offer(&self, samples: &[Sample]) {
        if samples.is_empty() {
            return;
        }
        match self.tx.try_send(samples.to_vec()) {
            Ok(()) => {}
            Err(TrySendError::Full(lost)) => {
                let total = self.counters.add_dropped(lost.len());
                warn!(
                    points = lost.len(),
                    influx_points_dropped = total,
                    "slowctl: the InfluxDB queue is full — dropped this poll"
                );
            }
            Err(TrySendError::Disconnected(lost)) => {
                let total = self.counters.add_dropped(lost.len());
                warn!(
                    points = lost.len(),
                    influx_points_dropped = total,
                    "slowctl: the InfluxDB writer is gone — dropped this poll"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------
// line protocol(純関数)
// ---------------------------------------------------------------------

/// 行のまとめ単位 (device, node, ts_ms)。
type LineKey<'a> = (&'a str, Option<u8>, u64);

/// サンプル列を line protocol にする(発注書 §2)。
///
/// * 同じ (device, node, ts_ms) を 1 行にまとめる。行は**初出順**、field は入力順
///   (出力が決定的 = 手組みのオラクルで固定できる)。
/// * `measurement` = device、tag `ch` = node(無ければ `board`)、field = param ごとの
///   float と `alarm`(行のどれか 1 つでも alarm なら true)、timestamp = ts_ms(precision=ms)。
/// * 単位は書かない(param ごとに固定 —— SPEC に表で持つ)。
/// * NaN / inf は書けないので、**呼び手が除いておくこと**(writer は受け取った時点で数えて落とす)。
pub fn line_protocol(samples: &[Sample]) -> String {
    let mut order: Vec<LineKey<'_>> = Vec::new();
    let mut lines: HashMap<LineKey<'_>, Vec<&Sample>> = HashMap::new();
    for sample in samples {
        let key = (sample.device.as_str(), sample.node, sample.ts_ms);
        lines
            .entry(key)
            .or_insert_with(|| {
                order.push(key);
                Vec::new()
            })
            .push(sample);
    }

    let mut out = String::new();
    for key in order {
        if let Some(group) = lines.get(&key) {
            push_line(&mut out, key, group);
        }
    }
    out
}

fn push_line(out: &mut String, (device, node, ts_ms): LineKey<'_>, group: &[&Sample]) {
    // String への write! は失敗しない(fmt::Error を返すのは書き先の側だけ)。
    push_escaped(out, device, MEASUREMENT_SPECIAL);
    out.push_str(",ch=");
    match node {
        Some(node) => {
            let _ = write!(out, "{node}");
        }
        None => out.push_str(BOARD_TAG),
    }
    out.push(' ');
    for sample in group {
        push_escaped(out, &sample.param, KEY_SPECIAL);
        // f64 の Display = 往復できる最短表記(33.0 → "33")。接尾辞なしの数値は float。
        let _ = write!(out, "={},", sample.value);
    }
    let alarm = group.iter().any(|s| s.alarm);
    let _ = writeln!(out, "alarm={alarm} {ts_ms}");
}

fn push_escaped(out: &mut String, text: &str, special: &[char]) {
    for c in text.chars() {
        if special.contains(&c) {
            out.push('\\');
        }
        out.push(c);
    }
}

// ---------------------------------------------------------------------
// writer スレッド
// ---------------------------------------------------------------------

/// 起動済みの writer。[`InfluxWriter::sink`] を読み出しスレッドに配り、本体は
/// [`InfluxWriter::counters`] を status PUB に載せる。
pub struct InfluxWriter {
    sink: InfluxSink,
    handle: JoinHandle<()>,
}

impl InfluxWriter {
    pub fn spawn(params: InfluxParams) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::sync_channel(INFLUX_QUEUE);
        let counters = Arc::new(InfluxCounters::default());
        let handle = std::thread::Builder::new()
            .name("slowctl-influx".to_string())
            .spawn({
                let counters = Arc::clone(&counters);
                move || writer_loop(params, rx, counters)
            })?;
        Ok(Self {
            sink: InfluxSink { tx, counters },
            handle,
        })
    }

    pub fn sink(&self) -> InfluxSink {
        self.sink.clone()
    }

    pub fn counters(&self) -> &Arc<InfluxCounters> {
        &self.sink.counters
    }

    /// 停止: 溜まっている分を最後に書けるだけ書いて、writer スレッドを畳む。
    ///
    /// **sink の複製が全部落ちてから呼ぶ**(デバイススレッドを join した後)。複製が
    /// 残っているとチャネルが切れず、ここから戻らない。
    pub fn finish(self) {
        drop(self.sink);
        if self.handle.join().is_err() {
            error!("slowctl: the InfluxDB writer thread panicked");
        }
    }
}

/// writer の本体ループ。全 sink が落ちる(チャネルが切れる)まで回る。
fn writer_loop(params: InfluxParams, rx: Receiver<Vec<Sample>>, counters: Arc<InfluxCounters>) {
    let client = Client::new(&params);
    let mut buffer = Buffer {
        points: VecDeque::new(),
        max: params.buffer_max_points,
        overflow_warned: false,
    };
    let mut backoff = BACKOFF_MIN;
    // 失敗後、次に試してよい時刻(None = 直前は成功 or まだ書いていない)。
    let mut retry_at: Option<Instant> = None;
    let mut last_post = Instant::now();

    while receive(&rx, &mut buffer, &counters) {
        if buffer.points.is_empty() {
            continue;
        }
        let now = Instant::now();
        let due = buffer.points.len() >= BATCH_MAX_POINTS
            || now.duration_since(last_post) >= params.batch;
        let backing_off = retry_at.is_some_and(|at| now < at);
        if !due || backing_off {
            continue;
        }
        last_post = now;
        match write_batch(&client, &mut buffer, &counters) {
            Ok(()) => {
                backoff = BACKOFF_MIN;
                retry_at = None;
            }
            Err(e) => {
                counters.record_error(&e);
                report_failure(&e, backoff, buffer.points.len());
                retry_at = Some(now + backoff);
                backoff = (backoff * 2).min(BACKOFF_MAX);
            }
        }
    }

    // 停止: 残りを書けるだけ書く(最初の失敗で諦める —— 停止を長引かせない)。
    while !buffer.points.is_empty() {
        if let Err(e) = write_batch(&client, &mut buffer, &counters) {
            counters.record_error(&e);
            break;
        }
    }
    if !buffer.points.is_empty() {
        let total = counters.add_dropped(buffer.points.len());
        warn!(
            points = buffer.points.len(),
            influx_points_dropped = total,
            "slowctl: stopping with points not written to InfluxDB — dropped"
        );
    }
}

/// チャネルを吸い出してバッファへ。`false` = 全 sink が落ちた(停止)。
fn receive(rx: &Receiver<Vec<Sample>>, buffer: &mut Buffer, counters: &InfluxCounters) -> bool {
    match rx.recv_timeout(RECV_TICK) {
        Ok(samples) => buffer.admit(samples, counters),
        Err(RecvTimeoutError::Timeout) => return true,
        Err(RecvTimeoutError::Disconnected) => return false,
    }
    loop {
        match rx.try_recv() {
            Ok(samples) => buffer.admit(samples, counters),
            Err(TryRecvError::Empty) => return true,
            Err(TryRecvError::Disconnected) => return false,
        }
    }
}

/// バッファ先頭から最大 [`BATCH_MAX_POINTS`] 点を 1 回 POST する。成功したらその分を
/// バッファから外して数える。失敗したらバッファは触らない(= バッチは先頭に戻ったまま)。
fn write_batch(
    client: &Client,
    buffer: &mut Buffer,
    counters: &InfluxCounters,
) -> Result<(), WriteError> {
    let n = buffer.points.len().min(BATCH_MAX_POINTS);
    let body = line_protocol(&buffer.points.make_contiguous()[..n]);
    client.post(&body)?;
    buffer.points.drain(..n);
    buffer.overflow_warned = false;
    counters.add_written(n);
    Ok(())
}

/// 失敗を可視化する。4xx は設定ミス(url / org / bucket / token)で、再試行しても
/// 直らない —— `error!` にする(発注書 §3)。5xx・接続不可・タイムアウトは `warn!`。
fn report_failure(e: &WriteError, retry_in: Duration, buffered: usize) {
    if e.is_client_error() {
        error!(error = %e, retry_in_s = retry_in.as_secs(), buffered,
               "slowctl: InfluxDB rejected the write — check url / org / bucket / token \
                (retrying later)");
    } else {
        warn!(error = %e, retry_in_s = retry_in.as_secs(), buffered,
              "slowctl: cannot write to InfluxDB — retrying");
    }
}

/// 再送バッファ(FIFO、上限 `max` 点)。
struct Buffer {
    points: VecDeque<Sample>,
    max: usize,
    /// 溢れの warn を出したか(書けるまで 1 回だけ —— 長い停電中に 1 poll ごとに出さない)。
    overflow_warned: bool,
}

impl Buffer {
    /// 1 poll ぶんを末尾に積む。NaN / inf と、上限を超えた**古い方**は落として数える。
    fn admit(&mut self, samples: Vec<Sample>, counters: &InfluxCounters) {
        for sample in samples {
            if !sample.value.is_finite() {
                let total = counters.add_dropped(1);
                warn!(
                    key = sample.key(),
                    value = sample.value,
                    influx_points_dropped = total,
                    "slowctl: NaN / inf cannot be written to InfluxDB — dropped"
                );
                continue;
            }
            self.points.push_back(sample);
        }
        let excess = self.points.len().saturating_sub(self.max);
        if excess > 0 {
            self.points.drain(..excess);
            let total = counters.add_dropped(excess);
            if !self.overflow_warned {
                self.overflow_warned = true;
                warn!(
                    limit = self.max,
                    influx_points_dropped = total,
                    "slowctl: the InfluxDB retry buffer is full — dropping the oldest points \
                       (said once until a write succeeds; the count keeps going up)"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------
// HTTP(ureq、ブロッキング)
// ---------------------------------------------------------------------

/// 1 回の POST の失敗。
#[derive(Debug, thiserror::Error)]
enum WriteError {
    /// InfluxDB が 4xx / 5xx を返した(`message` は InfluxDB の本文)。
    #[error("HTTP {code}: {message}")]
    Status { code: u16, message: String },
    /// 接続不可・タイムアウトなど。
    #[error("{0}")]
    Transport(#[from] ureq::Error),
}

impl WriteError {
    fn is_client_error(&self) -> bool {
        matches!(self, WriteError::Status { code, .. } if (400..500).contains(code))
    }
}

/// v2 write API の口(発注書 §1)。
struct Client {
    agent: ureq::Agent,
    write_url: String,
    org: String,
    bucket: String,
    authorization: String,
}

impl Client {
    fn new(params: &InfluxParams) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(params.timeout))
            // 4xx / 5xx を Err にせず status を見る(InfluxDB の言い分を last_error に残す)。
            .http_status_as_error(false)
            // 書き先は localhost。ureq は既定で HTTP_PROXY 等の環境変数を読むので、
            // それに横取りさせない。
            .proxy(None)
            .build()
            .into();
        Self {
            agent,
            write_url: format!("{}/api/v2/write", params.url.trim_end_matches('/')),
            org: params.org.clone(),
            bucket: params.bucket.clone(),
            authorization: format!("Token {}", params.token),
        }
    }

    /// `POST {url}/api/v2/write?org=…&bucket=…&precision=ms`(本文 = line protocol)。
    fn post(&self, body: &str) -> Result<(), WriteError> {
        let mut response = self
            .agent
            .post(&self.write_url)
            .query("org", &self.org)
            .query("bucket", &self.bucket)
            .query("precision", "ms")
            .header("Authorization", &self.authorization)
            .header("Content-Type", "text/plain; charset=utf-8")
            .send(body)?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let message = response
            .body_mut()
            .with_config()
            .limit(ERROR_BODY_LIMIT)
            .read_to_string()
            .unwrap_or_else(|e| format!("(cannot read the response body: {e})"));
        Err(WriteError::Status {
            code: status.as_u16(),
            message: message.trim().to_string(),
        })
    }
}

// ---------------------------------------------------------------------
// テスト
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::time::Instant;

    // -----------------------------------------------------------------
    // line protocol(純関数)
    // -----------------------------------------------------------------

    fn sample(device: &str, node: Option<u8>, param: &str, value: f64, ts_ms: u64) -> Sample {
        Sample {
            device: device.to_string(),
            node,
            param: param.to_string(),
            value,
            unit: "x".to_string(), // 単位は列に持たない(発注書 §2)—— 何であっても出力に出ない
            ts_ms,
            alarm: false,
        }
    }

    fn alarmed(mut sample: Sample) -> Sample {
        sample.alarm = true;
        sample
    }

    /// 時刻は任意(HV は 5 s、LV は 1 s 周期なので別々の poll = 別の ts にしてある)。
    const HV_TS: u64 = 1_760_259_600_000;
    const LV_TS: u64 = 1_760_259_601_000;

    /// 発注書 §2 / テスト節: ELITPC の実機値(TODO/118・hivolta_driver.rs のゴールデン)で、
    /// 行の形を一字一句固定する。
    ///
    /// * hv ch0 = VMON 341.06 / IMON 9.593 / STATUS 33 / VSET 350 / ISET 15。
    ///   33 = 0b10_0001 = ON(bit 0)+ UNV(bit 5)→ ドライバが 5 サンプル全部に alarm を立てる。
    /// * hv ch1 = 250.24 / 8.868 / 1(ON のみ)/ 250 / 15 → alarm なし。
    /// * hv ボード(node 無し)= BDALARM 0 / BDILK 0(実機は `00000` / `NO`)→ `ch=board`。
    /// * lv ch1 = 3.6 V / 0.997 A / OUTP 1(TODO/118 の実機)、VSET 3.6 / ISET 1.2 は例示値。
    /// * lv ch2 = OFF(0 / 0 / 0)、VSET 5 / ISET 0.1 は例示値。
    ///
    /// f64 は Rust の `Display`(往復できる最短表記)で出す: 33.0 → `33`、350.0 → `350`。
    /// line protocol では接尾辞なしの数値は float なので、型は全部 float のまま揃う。
    #[test]
    fn real_hv_and_lv_samples_become_one_line_per_channel() {
        let mut samples = Vec::new();
        for (param, value) in [
            ("vmon", 341.06),
            ("imon", 9.593),
            ("status", 33.0),
            ("vset", 350.0),
            ("iset", 15.0),
        ] {
            samples.push(alarmed(sample("hv", Some(0), param, value, HV_TS)));
        }
        for (param, value) in [
            ("vmon", 250.24),
            ("imon", 8.868),
            ("status", 1.0),
            ("vset", 250.0),
            ("iset", 15.0),
        ] {
            samples.push(sample("hv", Some(1), param, value, HV_TS));
        }
        samples.push(sample("hv", None, "bdalarm", 0.0, HV_TS));
        samples.push(sample("hv", None, "bdilk", 0.0, HV_TS));
        for (node, volt, curr, outp, vset, iset) in
            [(1, 3.6, 0.997, 1.0, 3.6, 1.2), (2, 0.0, 0.0, 0.0, 5.0, 0.1)]
        {
            for (param, value) in [
                ("volt", volt),
                ("curr", curr),
                ("outp", outp),
                ("vset", vset),
                ("iset", iset),
            ] {
                samples.push(sample("lv", Some(node), param, value, LV_TS));
            }
        }

        assert_eq!(
            line_protocol(&samples),
            "hv,ch=0 vmon=341.06,imon=9.593,status=33,vset=350,iset=15,alarm=true 1760259600000\n\
             hv,ch=1 vmon=250.24,imon=8.868,status=1,vset=250,iset=15,alarm=false 1760259600000\n\
             hv,ch=board bdalarm=0,bdilk=0,alarm=false 1760259600000\n\
             lv,ch=1 volt=3.6,curr=0.997,outp=1,vset=3.6,iset=1.2,alarm=false 1760259601000\n\
             lv,ch=2 volt=0,curr=0,outp=0,vset=5,iset=0.1,alarm=false 1760259601000\n"
        );
    }

    /// line protocol のエスケープ規則(InfluxDB v2 の line protocol 仕様):
    /// measurement はカンマと空白、field key はカンマ・等号・空白をバックスラッシュで逃がす。
    /// measurement の等号は特殊文字ではないので**逃がさない**(逃がすと `\=` が名前に残る)。
    #[test]
    fn device_and_param_names_are_escaped_by_the_line_protocol_rules() {
        let samples = [
            sample("gas line,1=a", None, "flow rate", 1.5, 7),
            sample("gas line,1=a", None, "p,mbar=abs", 1013.25, 7),
        ];
        assert_eq!(
            line_protocol(&samples),
            "gas\\ line\\,1=a,ch=board flow\\ rate=1.5,p\\,mbar\\=abs=1013.25,alarm=false 7\n"
        );
    }

    /// 同じ (device, node, ts_ms) だけが 1 行にまとまる。並びが入り組んでいても初出順に出し、
    /// 同じ ch でも ts が違えば別の行。`alarm` はその行のどれか 1 つでも立っていれば true。
    #[test]
    fn only_samples_with_the_same_device_node_and_ts_share_a_line() {
        let samples = [
            sample("hv", Some(0), "vmon", 100.5, 1_000),
            sample("hv", Some(1), "vmon", 200.25, 1_000),
            alarmed(sample("hv", Some(0), "imon", 3.75, 1_000)), // ch0 の行の 1 つだけ alarm
            sample("hv", Some(0), "vmon", 101.5, 2_000),         // 同じ ch0 でも別の ts
            sample("lv", Some(0), "volt", 3.5, 1_000),           // 同じ ts・同じ node でも別の機器
        ];
        assert_eq!(
            line_protocol(&samples),
            "hv,ch=0 vmon=100.5,imon=3.75,alarm=true 1000\n\
             hv,ch=1 vmon=200.25,alarm=false 1000\n\
             hv,ch=0 vmon=101.5,alarm=false 2000\n\
             lv,ch=0 volt=3.5,alarm=false 1000\n"
        );
    }

    // -----------------------------------------------------------------
    // トークンファイル(slowctl の起動時だけ読む)
    // -----------------------------------------------------------------

    /// 発注書 §4: 中身は 1 行、前後の空白は除く。無い・空ならパス入りの `Err`。
    #[test]
    fn the_token_file_is_one_trimmed_line_and_missing_or_blank_is_an_error() {
        let dir = std::env::temp_dir().join(format!("tpcdaq-influx-token-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let good = dir.join("good.token");
        std::fs::write(&good, "  write-only-token-xyz \n").unwrap();
        assert_eq!(read_token(&good).unwrap(), "write-only-token-xyz");

        let blank = dir.join("blank.token");
        std::fs::write(&blank, " \n\n").unwrap();
        let message = read_token(&blank).unwrap_err();
        assert!(message.contains("blank.token"), "message = {message:?}");

        let missing = dir.join("missing.token");
        let message = read_token(&missing).unwrap_err();
        assert!(
            message.contains("token_file") && message.contains("missing.token"),
            "message = {message:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------
    // writer(偽の InfluxDB = TcpListener で受けて記録する)
    // -----------------------------------------------------------------

    /// 偽 InfluxDB が受けた要求 1 本。
    #[derive(Debug)]
    struct Recorded {
        request_line: String,
        /// (小文字の名前, 値)
        headers: Vec<(String, String)>,
        body: String,
    }

    impl Recorded {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.as_str())
        }
    }

    /// InfluxDB が 4xx/5xx で返す JSON(形は v2 の実物に合わせた)。
    fn error_body(status: u16) -> &'static str {
        match status {
            401 => r#"{"code":"unauthorized","message":"unauthorized access"}"#,
            _ => r#"{"code":"internal error","message":"fake failure"}"#,
        }
    }

    /// `statuses[i]` を i 本目の要求に返す(尽きたら 204 = InfluxDB の書き込み成功)。
    /// 1 接続 1 要求(`Connection: close`)。
    fn fake_influx(listener: TcpListener, statuses: Vec<u16>) -> mpsc::Receiver<Recorded> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for (i, stream) in listener.incoming().enumerate() {
                let Ok(stream) = stream else { return };
                let Some(recorded) = read_request(&stream) else {
                    continue;
                };
                let status = statuses.get(i).copied().unwrap_or(204);
                let body = if status == 204 {
                    ""
                } else {
                    error_body(status)
                };
                let _ = write!(
                    &stream,
                    "HTTP/1.1 {status} Fake\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                if tx.send(recorded).is_err() {
                    return;
                }
            }
        });
        rx
    }

    fn read_request(stream: &TcpStream) -> Option<Recorded> {
        let mut reader = BufReader::new(stream);
        let mut request_line = String::new();
        reader.read_line(&mut request_line).ok()?;
        let mut headers = Vec::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).ok()?;
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            let (name, value) = line.split_once(':')?;
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
        let length: usize = headers
            .iter()
            .find(|(k, _)| k == "content-length")?
            .1
            .parse()
            .ok()?;
        let mut body = vec![0; length];
        reader.read_exact(&mut body).ok()?;
        Some(Recorded {
            request_line: request_line.trim_end().to_string(),
            headers,
            body: String::from_utf8(body).ok()?,
        })
    }

    /// 誰も listen していないアドレス(bind して即 drop —— 既存テストの流儀)。
    fn dead_addr() -> SocketAddr {
        let probe = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = probe.local_addr().unwrap();
        drop(probe);
        addr
    }

    fn params(addr: SocketAddr, buffer_max_points: usize) -> InfluxParams {
        InfluxParams {
            url: format!("http://{addr}"),
            org: "elinp".to_string(),
            bucket: "slowctl".to_string(),
            token: "tok-122".to_string(),
            batch: Duration::from_millis(20), // テストを待たせない(本番の既定は 1000 ms)
            buffer_max_points,
            timeout: Duration::from_secs(2),
        }
    }

    fn snapshot(counters: &InfluxCounters) -> SlowCounters {
        let mut out = SlowCounters::default();
        counters.write_into(&mut out);
        out
    }

    /// `done` が真になるまで待つ(期限切れはテスト失敗)。
    fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// 発注書 §1: `POST {url}/api/v2/write?org=…&bucket=…&precision=ms`、
    /// `Authorization: Token …`、本文は line protocol。
    #[test]
    fn the_writer_posts_line_protocol_to_the_v2_write_endpoint_with_the_token() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = fake_influx(listener, Vec::new());
        let writer = InfluxWriter::spawn(params(addr, 1000)).unwrap();
        let counters = Arc::clone(writer.counters());

        writer.sink().offer(&[
            alarmed(sample("hv", Some(0), "vmon", 341.06, HV_TS)),
            alarmed(sample("hv", Some(0), "imon", 9.593, HV_TS)),
        ]);

        let request = requests.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(
            request.request_line,
            "POST /api/v2/write?org=elinp&bucket=slowctl&precision=ms HTTP/1.1"
        );
        assert_eq!(request.header("authorization"), Some("Token tok-122"));
        assert_eq!(
            request.body,
            "hv,ch=0 vmon=341.06,imon=9.593,alarm=true 1760259600000\n"
        );
        // 2 点(= Sample 2 個)書けた。
        wait_for("points_written = 2", || {
            snapshot(&counters).influx_points_written == 2
        });
        writer.finish();
        let c = snapshot(&counters);
        assert_eq!(
            (
                c.influx_points_dropped,
                c.influx_write_errors,
                c.influx_last_error
            ),
            (0, 0, None)
        );
    }

    /// 発注書 §3: 失敗(500)したバッチは再送バッファに戻り、バックオフ後に**同じ本文**で
    /// 再送される。成功したら points_written はそのバッチの点数ちょうど(二重に数えない)。
    #[test]
    fn a_failed_batch_is_retried_and_counted_once_when_it_finally_succeeds() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = fake_influx(listener, vec![500]);
        let writer = InfluxWriter::spawn(params(addr, 1000)).unwrap();
        let counters = Arc::clone(writer.counters());

        writer.sink().offer(&[
            sample("lv", Some(1), "volt", 3.6, LV_TS),
            sample("lv", Some(1), "curr", 0.997, LV_TS),
            sample("lv", Some(2), "volt", 0.0, LV_TS),
        ]);

        let first = requests.recv_timeout(Duration::from_secs(10)).unwrap();
        let second = requests.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(first.body, second.body, "同じバッチを再送する");
        assert_eq!(
            second.body,
            "lv,ch=1 volt=3.6,curr=0.997,alarm=false 1760259601000\n\
             lv,ch=2 volt=0,alarm=false 1760259601000\n"
        );
        wait_for("points_written = 3", || {
            snapshot(&counters).influx_points_written == 3
        });
        writer.finish();
        let c = snapshot(&counters);
        assert_eq!(c.influx_points_written, 3, "失敗した 1 回目は数えない");
        assert_eq!(c.influx_write_errors, 1);
        assert_eq!(c.influx_points_dropped, 0);
        let last = c.influx_last_error.unwrap();
        assert!(last.contains("500"), "last_error = {last:?}");
    }

    /// 発注書 §3: 4xx(認証エラー等)も数えて last_error に InfluxDB の言い分を残し、
    /// 間隔を空けて再試行する(設定を直せば slowctl を再起動せずに書ける)。
    #[test]
    fn a_4xx_is_recorded_with_the_server_message_and_retried_later() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = fake_influx(listener, vec![401]);
        let writer = InfluxWriter::spawn(params(addr, 1000)).unwrap();
        let counters = Arc::clone(writer.counters());

        writer
            .sink()
            .offer(&[sample("hv", None, "bdilk", 0.0, HV_TS)]);

        requests.recv_timeout(Duration::from_secs(10)).unwrap();
        wait_for("write_errors = 1", || {
            snapshot(&counters).influx_write_errors == 1
        });
        let last = snapshot(&counters).influx_last_error.unwrap();
        assert!(
            last.contains("401") && last.contains("unauthorized access"),
            "last_error = {last:?}"
        );
        // 再試行(今度は 204)で書ける。
        requests.recv_timeout(Duration::from_secs(10)).unwrap();
        wait_for("points_written = 1", || {
            snapshot(&counters).influx_points_written == 1
        });
        writer.finish();
    }

    /// 発注書 §3 / テスト節: サーバ不在の間は再送バッファに溜まり、上限を超えたら
    /// **古い方から**落として数える。サーバが現れたら、残った新しい方だけが届く。
    ///
    /// 21 点(7 点 × 3 poll)を上限 10 点のバッファへ → 落ちるのは 21 − 10 = 11 点
    /// (i = 0..=10)、届くのは i = 11..=20 の 10 点。
    #[test]
    fn without_a_server_the_buffer_keeps_the_newest_points_and_counts_the_oldest_as_dropped() {
        let addr = dead_addr();
        let writer = InfluxWriter::spawn(params(addr, 10)).unwrap();
        let counters = Arc::clone(writer.counters());
        let sink = writer.sink();

        // i 番目の点: 値 i + 0.5、ts = 1000·(i + 1)(全部別の ts = 1 点 1 行)。
        let point = |i: u64| sample("hv", Some(0), "vmon", i as f64 + 0.5, 1_000 * (i + 1));
        for poll in 0..3u64 {
            let samples: Vec<Sample> = (0..7).map(|k| point(poll * 7 + k)).collect();
            sink.offer(&samples);
        }
        wait_for("points_dropped = 11", || {
            snapshot(&counters).influx_points_dropped == 11
        });
        // 溢れは受け取った時点で起きるので、最初の POST(batch 20 ms 後)より先に来うる。
        wait_for("接続不可が write_errors に出る", || {
            snapshot(&counters).influx_write_errors >= 1
        });
        let c = snapshot(&counters);
        assert_eq!(c.influx_points_written, 0);
        assert!(c.influx_last_error.is_some());

        // InfluxDB が上がる。
        let listener = TcpListener::bind(addr).unwrap();
        let requests = fake_influx(listener, Vec::new());
        let request = requests.recv_timeout(Duration::from_secs(10)).unwrap();
        let expected: String = (11..=20u64)
            .map(|i| {
                format!(
                    "hv,ch=0 vmon={},alarm=false {}\n",
                    i as f64 + 0.5,
                    1_000 * (i + 1)
                )
            })
            .collect();
        assert_eq!(request.body, expected);
        wait_for("points_written = 10", || {
            snapshot(&counters).influx_points_written == 10
        });
        drop(sink);
        writer.finish();
        assert_eq!(snapshot(&counters).influx_points_dropped, 11);
    }

    /// 発注書 §3: チャネルが満杯なら `try_send` で落として数える(読み出しは止めない)。
    /// writer が居なくなった後(停止中)に来た分も同じく数える。
    #[test]
    fn a_full_channel_drops_the_offer_and_counts_its_points() {
        let (tx, rx) = mpsc::sync_channel(1);
        let counters = Arc::new(InfluxCounters::default());
        let sink = InfluxSink {
            tx,
            counters: Arc::clone(&counters),
        };

        sink.offer(&[
            sample("hv", Some(0), "vmon", 1.5, 1),
            sample("hv", Some(0), "imon", 2.5, 1),
        ]); // 段数 1 に入る
        sink.offer(&[
            sample("hv", Some(1), "vmon", 3.5, 1),
            sample("hv", Some(1), "imon", 4.5, 1),
            sample("hv", Some(1), "status", 1.0, 1),
        ]); // 満杯 → この 3 点が落ちる
        assert_eq!(snapshot(&counters).influx_points_dropped, 3);
        assert_eq!(rx.recv().unwrap().len(), 2, "先に入った 2 点はそのまま");

        drop(rx);
        sink.offer(&[sample("lv", Some(1), "volt", 3.6, 2)]);
        assert_eq!(snapshot(&counters).influx_points_dropped, 4);
    }

    /// NaN / inf は line protocol に書けない(1 つ混ざるとバッチ全体が 400 で弾かれ、
    /// 後続の正常値まで巻き添えで溜まり続ける)。その点だけ落として数える。
    #[test]
    fn non_finite_values_are_dropped_and_counted_instead_of_poisoning_the_batch() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = fake_influx(listener, Vec::new());
        let writer = InfluxWriter::spawn(params(addr, 1000)).unwrap();
        let counters = Arc::clone(writer.counters());

        writer.sink().offer(&[
            sample("hv", Some(3), "vmon", f64::NAN, HV_TS),
            sample("hv", Some(3), "imon", 9.042, HV_TS),
            sample("hv", Some(3), "vset", f64::INFINITY, HV_TS),
        ]);

        let request = requests.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(
            request.body,
            "hv,ch=3 imon=9.042,alarm=false 1760259600000\n"
        );
        wait_for("points_written = 1", || {
            snapshot(&counters).influx_points_written == 1
        });
        writer.finish();
        assert_eq!(snapshot(&counters).influx_points_dropped, 2);
    }

    /// 停止時(全 sink が落ちた)に溜まっている分は、最後に 1 回書いてから終わる。
    /// batch を長く(10 s)して周期 POST を起こさず、finish だけで届くことを見る。
    #[test]
    fn finishing_the_writer_flushes_what_is_still_buffered() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = fake_influx(listener, Vec::new());
        let mut long_batch = params(addr, 1000);
        long_batch.batch = Duration::from_secs(10);
        let writer = InfluxWriter::spawn(long_batch).unwrap();
        let counters = Arc::clone(writer.counters());

        writer
            .sink()
            .offer(&[sample("lv", Some(2), "outp", 0.0, LV_TS)]);
        writer.finish();

        let request = requests.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(request.body, "lv,ch=2 outp=0,alarm=false 1760259601000\n");
        assert_eq!(snapshot(&counters).influx_points_written, 1);
    }
}
