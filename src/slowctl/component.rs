//! slowctl コンポーネント本体(TODO/079 §2.6-4、SPEC §1.1 / §2.3 / §3.1 / §9.2)。
//!
//! **監視・記録・アラーム**が本分(SPEC §11 の安全境界)。唯一の作動は HV / LV の ch ごとの
//! ON / OFF(v1.42、TODO/124)で、`[slowctl] cmd_bind` を書いたときだけ開く localhost の REP
//! (専用スレッド `slowctl-cmd`)から受ける。
//!
//! # power ON / OFF の流れ(TODO/124 §1 + Fable 1–5)
//!
//! REP スレッドは要求を**静的に**検証し(op・device 名・type が hivolta / hmp2020・ch の範囲)、
//! その機器の深さ 1 のキューへ `try_send`(満杯 = `device busy`)して、結果を最大 5 s 待つ
//! (超えたら `timeout — state unknown — check the table`)。REQ / REP なので**どの経路でも
//! 1 通だけ**返す。
//! 実行は**その機器の device_loop スレッド**(ポートの持ち主を 1 つに保つ): poll の合間の
//! sleep がキューの `recv_timeout` なので、コマンドは poll 周期を待たずに走り、走ったら残りの
//! sleep を捨ててすぐ poll する(表が古いまま残らない)。backoff 中のコマンドには即
//! `device not connected`。run の相による規則は controller が判断する(slowctl は run を知らない)。
//!
//! # 形(なぜ tokio を使わないか)
//!
//! デバイス IO は**ブロッキングなシリアル読み書き**で、周期は 1 Hz 級。tokio に載せても
//! 得るものが無く(`spawn_blocking` でスレッドに戻るだけ)、代わりに async 境界ぶんの
//! 複雑さが増える。よって **std スレッド + `mpsc` + 同期 ZMQ** で書く(KISS)。
//! tokio に触れるのは `src/bin/slowctl.rs` の SIGINT 待ちだけで、そこから
//! [`run_slowctl`] には `AtomicBool` 1 本しか渡らない。
//!
//! # 落としてよいもの / いけないもの
//!
//! * status PUB(§2.3)= **モニタ系**。SUB が居なければ落ちてよい —— 落ちた数は数える。
//! * LogPost(§9)= 記録系だが、**controller 不在で slowctl を止めてはいけない**(計測は続く)。
//!   保存系のロスレス契約(receiver → graw-writer / decoder)とは別の口である。
//!   ただし **`IDENT` / `ALARM` / `RECOVER` は一度きりの事実**なので落とさない ——
//!   送信に失敗したら [`RetryQueue`](上限 [`RETRY_QUEUE_LIMIT`])に入れ、周回の先頭で
//!   投函順に再送する。実運用の起動順は controller が最後(docs/NEW_DAQ_SETUP_ja.md)で、
//!   slowctl が先に立つと起動時の IDENT が必ず失敗するため、これが無いと機器の識別記録が
//!   実機で常に消える。`VALUE`(次が来る)と `ERROR`(次回集計に入る)は再送しない。
//!   捨てた分は `log_dropped` に数えて warn する(silent にしない — CLAUDE.md)。
//! * InfluxDB(TODO/122、`[slowctl.influx]` があるときだけ)= **モニタ系**。全サンプルを
//!   [`super::influx`] の writer スレッドへ `try_send` で渡す。落ちた分は
//!   `influx_points_dropped` に数え、読み出しスレッドは HTTP を待たない。

use std::collections::BTreeMap;
use std::ops::RangeInclusive;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::json;
use tracing::{error, info, warn};

use crate::config::{Config, InfluxConfig, SlowDeviceConfig, SLOWCTL_SOURCE_ID};
use crate::logbook::{LogbookRecord, SLOW_ACTOR};

use super::bronkhorst::open_serial;
use super::influx::{InfluxCounters, InfluxParams, InfluxSink, InfluxWriter};
use super::{
    now_ms, AlarmEvent, AlarmRule, Driver, DriverError, Ident, LogPolicy, PowerOutcome,
    PowerRefused, Sample, SlowCounters, SlowPayload, SLOW_KIND,
};

/// status PUB の周期(SPEC §2.3「1 Hz に集約、最新値のみ」)。
const PUBLISH_INTERVAL: Duration = Duration::from_secs(1);

/// ERROR レコードの集計周期(SPEC §9.2「ERROR は 1 分毎の集計、0 なら書かない」)。
const ERROR_REPORT_INTERVAL: Duration = Duration::from_secs(60);

/// 本体ループが停止合図を確認する周期。
const DRAIN_TIMEOUT: Duration = Duration::from_millis(50);

/// 連続でこの回数 `poll` が失敗したら、リンクを捨てて開き直す。
const MAX_CONSECUTIVE_POLL_ERRORS: u32 = 5;

/// 再接続バックオフの下限・上限(TODO/079 §2.6-4「指数バックオフ再接続(1 → 30 s)」)。
const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(30);

/// デバイススレッド → 本体のキュー段数(1 Hz 級なので浅くてよい)。
const EVENT_QUEUE: usize = 256;

/// 届かなかった一度きりのレコード(IDENT / ALARM / RECOVER)を抱える上限。
///
/// 実運用の起動順では controller が最後に上がる(docs/NEW_DAQ_SETUP_ja.md)ので、
/// slowctl が先に立つと起動時の IDENT が**必ず**投函に失敗する。捨てると機器の識別記録が
/// 二度と残らないので、届くまで抱えて再送する。抱え込みが無制限にならないよう上限を置き、
/// 超えたら最古から捨てて数える。64 = 機器数 × 事象数として実運用の桁に十分な余裕。
const RETRY_QUEUE_LIMIT: usize = 64;

/// LogPost 1 本にかける上限(ミリ秒)。
///
/// **0(ノンブロッキング)にしてはいけない**: ロスレス PUSH は `ZMQ_IMMEDIATE`
/// (zmq_helper)なので「接続確立済みの相手」にしか積まない。起動直後は `connect` が
/// 返っていても ZMTP ハンドシェイクが済んでおらず、最初の 1 本(= IDENT)が
/// **相手が居るのに落ちる**。かといって無限に待つと controller 不在で計測が止まる
/// (それは slowctl の仕事ではない)。ハンドシェイク(localhost で ms 級)には
/// 桁で足り、かつ controller が落ちていても 1 本あたりこの時間しか止まらない値にする。
const LOG_SEND_TIMEOUT_MS: i32 = 200;

/// power コマンドの結果を待つ上限(TODO/124 §1)。超えたら `ok:false` + [`POWER_TIMEOUT_ERROR`]。
/// controller の REQ(10 s)より短く、controller には必ずこちらの応答が先に届く。
const POWER_REPLY_TIMEOUT: Duration = Duration::from_secs(5);

/// 機器ごとの power コマンドのキューの深さ(TODO/124 Fable 2)。満杯なら即 `device busy`。
const POWER_QUEUE_DEPTH: usize = 1;

/// [`POWER_REPLY_TIMEOUT`] を超えたときの `error`(レーン B レビュー 2026-10-12)。機器スレッドが
/// 遅れて実行し終えているかもしれないので状態は不明 —— controller は 502 でこの文言をそのまま返し、
/// UI もそのまま出す。
const POWER_TIMEOUT_ERROR: &str = "timeout — state unknown — check the table";

// ---------------------------------------------------------------------
// エラー
// ---------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum SlowctlError {
    #[error("cannot bind the status PUB at {endpoint}: {source}")]
    Pub {
        endpoint: String,
        #[source]
        source: zmq::Error,
    },

    #[error("cannot connect the LogPost PUSH to {endpoint}: {source}")]
    LogPush {
        endpoint: String,
        #[source]
        source: zmq::Error,
    },

    /// `[slowctl] cmd_bind` の REP を開けない(TODO/124)。
    #[error("cannot bind the power on/off REP at {endpoint}: {source}")]
    Cmd {
        endpoint: String,
        #[source]
        source: zmq::Error,
    },

    #[error("cannot spawn the device thread for {device}: {source}")]
    Spawn {
        device: String,
        #[source]
        source: std::io::Error,
    },

    #[error("device {device}: {message}")]
    Device { device: String, message: String },

    /// `[slowctl.influx]` の writer を起動できない(トークンが読めない等、TODO/122)。
    #[error("InfluxDB writer: {0}")]
    Influx(String),
}

// ---------------------------------------------------------------------
// パラメタ
// ---------------------------------------------------------------------

/// 解決済みの起動パラメタ(SPEC §3.1 の `[slowctl]`)。
#[derive(Debug, Clone, PartialEq)]
pub struct SlowctlParams {
    pub status_pub: String,
    /// power ON/OFF の REP(TODO/124)。`None` = 開かない(opt-in、Fable 8)。
    pub cmd_bind: Option<String>,
    pub log_push: String,
    pub poll_hz: f64,
    pub log_every_s: u64,
    pub devices: Vec<SlowDeviceConfig>,
    /// `[slowctl.influx]`(TODO/122)。`None` = InfluxDB に書かない。
    pub influx: Option<InfluxConfig>,
}

impl SlowctlParams {
    /// `[slowctl]` が無い設定なら `None`(= slowctl を走らせない)。
    pub fn from_config(config: &Config) -> Option<Self> {
        let slowctl = config.slowctl.as_ref()?;
        Some(Self {
            status_pub: slowctl.status_pub.clone(),
            cmd_bind: slowctl.cmd_bind.clone(),
            log_push: slowctl.log_push.clone(),
            poll_hz: slowctl.poll_hz,
            log_every_s: slowctl.log_every_s,
            devices: slowctl.device.clone(),
            influx: slowctl.influx.clone(),
        })
    }

    /// 1 周期の長さ。`poll_hz` は設定検証で 0 より大きいことが保証されている。
    pub fn poll_interval(&self) -> Duration {
        Duration::from_secs_f64(1.0 / self.poll_hz)
    }

    /// 機器ごとの周期。`poll_s` があればそれ、無ければ [`Self::poll_interval`](TODO/119)。
    /// `poll_s` は設定検証で正の有限値が保証されている。
    pub fn device_poll_interval(&self, device: &SlowDeviceConfig) -> Duration {
        match device.poll_s {
            Some(poll_s) => Duration::from_secs_f64(poll_s),
            None => self.poll_interval(),
        }
    }
}

// ---------------------------------------------------------------------
// カウンタ(SPEC §2.3 の `counters`)
// ---------------------------------------------------------------------

/// デバイス 1 台ぶんの累積カウンタ。silent failure を作らないための可視化フック。
#[derive(Debug, Default)]
pub struct Counters {
    pub poll_ok: AtomicU64,
    pub poll_err: AtomicU64,
    pub parse_err: AtomicU64,
    pub reconnects: AtomicU64,
    pub identity_mismatch: AtomicU64,
}

impl Counters {
    pub fn snapshot(&self) -> SlowCounters {
        SlowCounters {
            poll_ok: self.poll_ok.load(Ordering::Relaxed),
            poll_err: self.poll_err.load(Ordering::Relaxed),
            parse_err: self.parse_err.load(Ordering::Relaxed),
            reconnects: self.reconnects.load(Ordering::Relaxed),
            identity_mismatch: self.identity_mismatch.load(Ordering::Relaxed),
            // influx_* は機器ごとではなく writer 1 本の持ち物(main_loop が上書きする)。
            ..SlowCounters::default()
        }
    }

    /// `DriverError` の種別をそのままカウンタに落とす(区別を潰さない)。
    fn record(&self, error: &DriverError) {
        match error {
            // Unsupported は set_power の既定でだけ出る(ident / poll は返さない)。万一来たら
            // 「機器とのやり取りが噛み合わない」側に数える。
            DriverError::Protocol(_) | DriverError::Unsupported => {
                self.parse_err.fetch_add(1, Ordering::Relaxed)
            }
            DriverError::Identity { .. } => self.identity_mismatch.fetch_add(1, Ordering::Relaxed),
            DriverError::Timeout | DriverError::Io(_) => {
                self.poll_err.fetch_add(1, Ordering::Relaxed)
            }
        };
    }
}

/// 全デバイスのカウンタを 1 つに畳む(PUB ペイロードは 1 組だけ持つ —— SPEC §2.3)。
pub fn sum_counters(counters: &[Arc<Counters>]) -> SlowCounters {
    counters
        .iter()
        .map(|c| c.snapshot())
        .fold(SlowCounters::default(), |mut acc, c| {
            acc.poll_ok += c.poll_ok;
            acc.poll_err += c.poll_err;
            acc.parse_err += c.parse_err;
            acc.reconnects += c.reconnects;
            acc.identity_mismatch += c.identity_mismatch;
            acc
        })
}

/// ERROR レコードの増分(SPEC §9.2「1 分毎の集計、0 なら書かない」)。
fn counter_delta(now: &SlowCounters, previous: &SlowCounters) -> Option<SlowCounters> {
    let delta = SlowCounters {
        poll_ok: 0, // ERROR レコードに成功数は載せない(SPEC §9.2 の 4 項目)
        poll_err: now.poll_err.saturating_sub(previous.poll_err),
        parse_err: now.parse_err.saturating_sub(previous.parse_err),
        reconnects: now.reconnects.saturating_sub(previous.reconnects),
        identity_mismatch: now
            .identity_mismatch
            .saturating_sub(previous.identity_mismatch),
        ..SlowCounters::default() // influx_* は ERROR レコードに載せない
    };
    let empty = delta.poll_err == 0
        && delta.parse_err == 0
        && delta.reconnects == 0
        && delta.identity_mismatch == 0;
    (!empty).then_some(delta)
}

// ---------------------------------------------------------------------
// デバイススレッド
// ---------------------------------------------------------------------

/// デバイススレッド → 本体。
#[derive(Debug, Clone, PartialEq)]
pub enum DeviceEvent {
    Ident(Vec<Ident>),
    Samples(Vec<Sample>),
}

/// TOML の `type` からドライバを作る。
///
/// **実装済み**: `bronkhorst_flowbus` / `arduino_env`(TODO/079)、`hivolta` / `hmp2020`
/// (TODO/089)、`fake`。知らない `type` はここで明示的に失敗する —— 黙って何も読まない
/// デバイスを作らない(CLAUDE.md)。
fn open_driver(device: &SlowDeviceConfig) -> Result<Box<dyn Driver + Send>, SlowctlError> {
    let missing_port = || SlowctlError::Device {
        device: device.name.clone(),
        message: "no `port` in the config (should have been caught by validation)".to_string(),
    };
    let to_device_error = |e: DriverError| SlowctlError::Device {
        device: device.name.clone(),
        message: e.to_string(),
    };
    match device.device_type.as_str() {
        "bronkhorst_flowbus" => {
            let port = device.port.as_deref().ok_or_else(missing_port)?;
            let driver = open_serial(
                &device.name,
                port,
                device.nodes.clone(),
                device.expect_serial.clone(),
            )
            .map_err(to_device_error)?;
            Ok(Box::new(driver))
        }
        "arduino_env" => {
            let port = device.port.as_deref().ok_or_else(missing_port)?;
            let driver =
                super::arduino::open_serial(&device.name, port).map_err(to_device_error)?;
            Ok(Box::new(driver))
        }
        "hivolta" => {
            let status_ignore =
                super::hivolta::ignore_masks(&device.status_ignore).map_err(|message| {
                    SlowctlError::Device {
                        device: device.name.clone(),
                        message,
                    }
                })?;
            for (ch, flags) in super::hivolta::ignore_notices(&status_ignore) {
                info!(
                    device = %device.name,
                    ch,
                    flags = ?flags,
                    "slowctl: alarm judgement ignores these STATUS flags (status_ignore)"
                );
            }
            // TODO/089 §1: `addr`(TCP)優先、無ければ `port`(serial 9600 8N1)。
            if let Some(addr) = device.addr.as_deref() {
                let driver = super::hivolta_driver::open_tcp(
                    &device.name,
                    addr,
                    device.expect_serial.clone(),
                    status_ignore,
                )
                .map_err(to_device_error)?;
                Ok(Box::new(driver))
            } else {
                let port = device.port.as_deref().ok_or_else(missing_port)?;
                // TODO/119 §5: ポートはここで開かない(ident / poll のたびに開閉する)。
                let driver = super::hivolta_driver::open_serial(
                    &device.name,
                    port,
                    device.expect_serial.clone(),
                    status_ignore,
                );
                Ok(Box::new(driver))
            }
        }
        "hmp2020" => {
            let port = device.port.as_deref().ok_or_else(missing_port)?;
            let driver =
                super::hmp_driver::open_serial(&device.name, port, device.expect_serial.clone())
                    .map_err(to_device_error)?;
            Ok(Box::new(driver))
        }
        #[cfg(any(test, feature = "slowctl-fake"))]
        "fake" => {
            let port = device.port.as_deref().ok_or_else(missing_port)?;
            let driver = super::fake_device::FakeDevice::from_script(
                &device.name,
                std::path::Path::new(port),
            )
            .map_err(|e| SlowctlError::Device {
                device: device.name.clone(),
                message: e.to_string(),
            })?;
            Ok(Box::new(driver))
        }
        other => Err(SlowctlError::Device {
            device: device.name.clone(),
            message: format!("device type {other:?} is not implemented yet (TODO/079 Phase 3+)"),
        }),
    }
}

/// REP スレッド → device_loop の power コマンド(TODO/124)。
pub struct PowerCommand {
    pub ch: u8,
    pub on: bool,
    /// これを過ぎたら**実行しない**(レビュー 2026-10-12)。REP が結果を待つ上限と同じ時刻
    /// (受けた時刻 + [`POWER_REPLY_TIMEOUT`])。過ぎた = REP は timeout を返し、controller は
    /// `power_inflight` を外している —— その後に SET を送ると run/start と競合する。
    pub deadline: Instant,
    /// 結果(REP 応答の JSON そのもの)の返し先。容量 1 の oneshot。REP が timeout 済みなら
    /// 受け手はもう居ない([`deliver_power_reply`] が warn する)。
    pub reply: SyncSender<serde_json::Value>,
}

/// REP スレッドから見た 1 台(TOML の `[[slowctl.device]]` 1 個 + その device_loop の口)。
pub struct PowerTarget {
    pub name: String,
    pub device_type: String,
    pub commands: SyncSender<PowerCommand>,
}

/// 1 デバイスぶんのブロッキングループ(専用 std スレッド)。
///
/// `influx` があれば、poll ごとのサンプルを InfluxDB writer にも渡す(TODO/122)。渡し方は
/// `try_send` だけ —— **読み出しは決して止めない**(満杯なら落として数える)。
///
/// `commands` は power ON/OFF(TODO/124)。セッション中なら実行し、backoff 中なら即
/// `device not connected` を返す(Fable 3)。REP が無い(`cmd_bind` 省略)ときは送り手が
/// 居ないだけで、sleep の挙動は従来と同じ。
fn device_loop(
    device: SlowDeviceConfig,
    poll_interval: Duration,
    tx: SyncSender<DeviceEvent>,
    counters: Arc<Counters>,
    stop: Arc<AtomicBool>,
    influx: Option<InfluxSink>,
    commands: Receiver<PowerCommand>,
) {
    let mut backoff = BACKOFF_MIN;
    let mut opened_once = false;

    while !stop.load(Ordering::Relaxed) {
        if opened_once {
            counters.reconnects.fetch_add(1, Ordering::Relaxed);
        }
        let driver = match open_driver(&device) {
            Ok(driver) => driver,
            Err(e) => {
                warn!(device = device.name, error = %e, backoff_s = backoff.as_secs(),
                      "slowctl: cannot open the device — retrying");
                opened_once = true;
                backoff_sleep(backoff, &stop, &commands, &device.name);
                backoff = (backoff * 2).min(BACKOFF_MAX);
                continue;
            }
        };
        opened_once = true;

        if run_one_session(
            driver,
            &device,
            poll_interval,
            &tx,
            &counters,
            &stop,
            influx.as_ref(),
            &commands,
        ) {
            return; // 停止合図(またはチャネル切断)
        }
        backoff_sleep(backoff, &stop, &commands, &device.name);
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

/// 1 本のリンクで ident → poll ループを回す。戻り値 `true` = もう開き直さない(停止)。
///
/// poll の合間の待ちで power コマンドを受けたら実行し、**残りの待ちを捨ててすぐ poll する**
/// (TODO/124 Fable 4 —— 表が古いまま残らない)。
#[allow(clippy::too_many_arguments)] // 既存 7 本 + power のキュー。束ねる型を新設するほどではない
fn run_one_session(
    mut driver: Box<dyn Driver + Send>,
    device: &SlowDeviceConfig,
    poll_interval: Duration,
    tx: &SyncSender<DeviceEvent>,
    counters: &Arc<Counters>,
    stop: &Arc<AtomicBool>,
    influx: Option<&InfluxSink>,
    commands: &Receiver<PowerCommand>,
) -> bool {
    match driver.ident() {
        Ok(idents) => {
            info!(
                device = device.name,
                nodes = idents.len(),
                "slowctl: device identified"
            );
            if tx.send(DeviceEvent::Ident(idents)).is_err() {
                return true;
            }
        }
        Err(e) => {
            counters.record(&e);
            warn!(device = device.name, error = %e, "slowctl: ident failed — reopening");
            return false;
        }
    }

    let mut consecutive_errors = 0u32;
    while !stop.load(Ordering::Relaxed) {
        let started = Instant::now();
        match driver.poll() {
            Ok(samples) => {
                counters.poll_ok.fetch_add(1, Ordering::Relaxed);
                consecutive_errors = 0;
                if let Some(influx) = influx {
                    influx.offer(&samples);
                }
                if tx.send(DeviceEvent::Samples(samples)).is_err() {
                    return true;
                }
            }
            Err(e) => {
                counters.record(&e);
                consecutive_errors += 1;
                warn!(device = device.name, error = %e, consecutive = consecutive_errors,
                      "slowctl: poll failed");
                if consecutive_errors >= MAX_CONSECUTIVE_POLL_ERRORS {
                    warn!(
                        device = device.name,
                        "slowctl: too many consecutive failures — reopening the link"
                    );
                    return false;
                }
            }
        }
        match wait_or_command(
            poll_interval.saturating_sub(started.elapsed()),
            stop,
            commands,
        ) {
            // 実行したら残りの待ちを捨てて、ループの先頭(= poll)へ。
            Wake::Command(command) => {
                run_power_command(driver.as_mut(), &device.name, command);
            }
            Wake::Elapsed | Wake::Stopped => {}
        }
    }
    true
}

/// [`wait_or_command`] が戻った理由。
enum Wake {
    Elapsed,
    Stopped,
    Command(PowerCommand),
}

/// 停止合図を見ながら `duration` だけ待つ。その間に power コマンドが来たら**すぐ**返す
/// (TODO/124 Fable 3: 旧 `sleep_until_stop` の `thread::sleep` を `recv_timeout` に置き換えた形)。
/// 刻みは [`DRAIN_TIMEOUT`](SIGINT からの応答が `duration` ぶん遅れないように)。
fn wait_or_command(
    duration: Duration,
    stop: &AtomicBool,
    commands: &Receiver<PowerCommand>,
) -> Wake {
    let deadline = Instant::now() + duration;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Wake::Stopped;
        }
        let Some(left) = deadline.checked_duration_since(Instant::now()) else {
            return Wake::Elapsed;
        };
        if left.is_zero() {
            return Wake::Elapsed;
        }
        let slice = DRAIN_TIMEOUT.min(left);
        match commands.recv_timeout(slice) {
            Ok(command) => return Wake::Command(command),
            Err(RecvTimeoutError::Timeout) => {}
            // REP が無い(`cmd_bind` 省略)か畳まれた: ただ寝る(従来の sleep と同じ)。
            Err(RecvTimeoutError::Disconnected) => std::thread::sleep(slice),
        }
    }
}

/// 再接続の backoff を待つ。その間に来たコマンドには即 `device not connected`(Fable 3)。
fn backoff_sleep(
    duration: Duration,
    stop: &AtomicBool,
    commands: &Receiver<PowerCommand>,
    device: &str,
) {
    let deadline = Instant::now() + duration;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match wait_or_command(left, stop, commands) {
            Wake::Command(command) => {
                info!(
                    device,
                    ch = command.ch,
                    on = command.on,
                    "slowctl: power command refused — device not connected (reconnect backoff)"
                );
                let PowerCommand { ch, on, reply, .. } = command;
                deliver_power_reply(reply, power_refused("device not connected"), device, ch, on);
            }
            Wake::Elapsed | Wake::Stopped => return,
        }
    }
}

/// power コマンド 1 件を実行して結果を返す(TODO/124)。毎回 info に device / ch / on /
/// before / after / 結果を残す(§1)。戻り値 = 返し先に届けられたか。
///
/// **実行を始める直前に期限を見る**(レビュー 2026-10-12): 長い poll や ident の間にキューで
/// 待っていたコマンドが期限を過ぎていれば、機器には何も送らずに捨てる(返し先はもう居ない)。
fn run_power_command(driver: &mut dyn Driver, device: &str, command: PowerCommand) -> bool {
    let PowerCommand {
        ch,
        on,
        deadline,
        reply,
    } = command;
    if Instant::now() >= deadline {
        warn!(
            device,
            ch, on, "slowctl: power command expired before execution — not sent"
        );
        return false;
    }
    let response = power_reply(device, ch, on, driver.set_power(ch, on));
    info!(
        device,
        ch,
        on,
        ok = %response["ok"],
        before = %response["before"],
        after = %response["after"],
        error = %response["error"],
        "slowctl: power command"
    );
    deliver_power_reply(reply, response, device, ch, on)
}

/// 結果を REP スレッドへ返す。受け手が既に居なければ(5 s の timeout 済み)、返し先が無くても
/// **warn で残す**(TODO/124 Fable 5 —— 機器の状態は変わっているかもしれない)。
fn deliver_power_reply(
    reply: SyncSender<serde_json::Value>,
    response: serde_json::Value,
    device: &str,
    ch: u8,
    on: bool,
) -> bool {
    let after = response["after"].clone();
    match reply.try_send(response) {
        Ok(()) => true,
        Err(_) => {
            warn!(
                device,
                ch,
                on,
                after = %after,
                "slowctl: power command finished after the REP gave up (timeout) — \
                 nobody to reply to; check the table"
            );
            false
        }
    }
}

// ---------------------------------------------------------------------
// power ON/OFF の REP(TODO/124)
// ---------------------------------------------------------------------

/// 失敗の応答 `{"ok":false,"error":…,"before":null}`(before を読む前の失敗)。
fn power_refused(error: &str) -> serde_json::Value {
    json!({"ok": false, "error": error, "before": null})
}

/// device_loop の実行結果 → REP 応答(発注書 §1 の契約)。
///
/// * 成功: `{"ok":true,"device","ch","on","before","after"}`
/// * 機器の拒否(DIS・LOCAL 等): `{"ok":false,"error","before":{…}}`
/// * SET 後の状態を読めた失敗(切り替わらない等): `{"ok":false,"error","before":{…},"after":{…}}`
///   (レビュー 2026-10-12)
/// * before を読む前の失敗: `{"ok":false,"error","before":null}`
fn power_reply(
    device: &str,
    ch: u8,
    on: bool,
    result: Result<PowerOutcome, DriverError>,
) -> serde_json::Value {
    match result {
        Ok(PowerOutcome {
            before,
            result: Ok(after),
        }) => json!({
            "ok": true, "device": device, "ch": ch, "on": on, "before": before, "after": after
        }),
        Ok(PowerOutcome {
            before,
            result: Err(PowerRefused { error, after: None }),
        }) => json!({"ok": false, "error": error, "before": before}),
        Ok(PowerOutcome {
            before,
            result:
                Err(PowerRefused {
                    error,
                    after: Some(after),
                }),
        }) => json!({"ok": false, "error": error, "before": before, "after": after}),
        Err(e) => power_refused(&e.to_string()),
    }
}

/// power を持つ型の ch の範囲(TODO/124 §1: hivolta 0–7、hmp2020 1–2)。ほかの型は `None`。
fn power_channels(device_type: &str) -> Option<RangeInclusive<u8>> {
    match device_type {
        "hivolta" => Some(0..=super::hivolta::NUM_CH - 1),
        "hmp2020" => Some(super::hmp::MIN_CH..=super::hmp::MAX_CH),
        _ => None,
    }
}

/// REP の 1 要求を処理して応答を返す(ZMQ を知らない —— 単体テストが直接叩ける)。
///
/// 要求: `{"op":"power","device":"hv","ch":3,"on":false}`。静的検証(op・device・type・ch)を
/// 通ったものだけをその機器のキューへ `try_send` し、結果を `reply_timeout` まで待つ。
/// 機器が識別済みか(セッション中か)は device_loop が判断する(backoff 中なら
/// `device not connected`)。
pub fn handle_power_request(
    request: &[u8],
    targets: &[PowerTarget],
    reply_timeout: Duration,
) -> serde_json::Value {
    #[derive(Deserialize)]
    struct Op {
        op: String,
    }
    #[derive(Deserialize)]
    struct Power {
        device: String,
        /// 範囲外(負・256 以上)も「範囲外」と言えるように幅を取って読む。
        ch: i64,
        on: bool,
    }

    let op: Op = match serde_json::from_slice(request) {
        Ok(op) => op,
        Err(e) => return power_refused(&format!("bad request: {e}")),
    };
    if op.op != "power" {
        return power_refused(&format!("unknown op {:?} (only \"power\")", op.op));
    }
    let power: Power = match serde_json::from_slice(request) {
        Ok(power) => power,
        Err(e) => return power_refused(&format!("bad request: {e}")),
    };
    let Some(target) = targets.iter().find(|t| t.name == power.device) else {
        return power_refused(&format!("unknown device {:?}", power.device));
    };
    let Some(channels) = power_channels(&target.device_type) else {
        return power_refused(&format!(
            "device {:?} (type {}) has no power on/off — only hivolta / hmp2020",
            target.name, target.device_type
        ));
    };
    let ch = match u8::try_from(power.ch) {
        Ok(ch) if channels.contains(&ch) => ch,
        _ => {
            return power_refused(&format!(
                "ch {} is out of range for {} ({}..={})",
                power.ch,
                target.device_type,
                channels.start(),
                channels.end()
            ))
        }
    };

    // REP が待つ上限とコマンドの期限は同じ時刻(過ぎたら device_loop は実行しない)。
    let deadline = Instant::now() + reply_timeout;
    let (reply, reply_rx) = mpsc::sync_channel(1);
    let command = PowerCommand {
        ch,
        on: power.on,
        deadline,
        reply,
    };
    match target.commands.try_send(command) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => return power_refused("device busy"),
        Err(TrySendError::Disconnected(_)) => return power_refused("device thread is not running"),
    }
    match reply_rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(response) => response,
        Err(RecvTimeoutError::Timeout) => {
            warn!(
                device = target.name,
                ch,
                on = power.on,
                timeout_s = reply_timeout.as_secs_f64(),
                "slowctl: power command timed out — state unknown"
            );
            power_refused(POWER_TIMEOUT_ERROR)
        }
        Err(RecvTimeoutError::Disconnected) => {
            power_refused("device thread stopped before replying")
        }
    }
}

/// `cmd_bind` の REP を開く(linger 0、`rcvtimeo` = [`DRAIN_TIMEOUT`] で停止合図を見る)。
fn bind_cmd_rep(context: &zmq::Context, endpoint: &str) -> Result<zmq::Socket, zmq::Error> {
    let socket = context.socket(zmq::REP)?;
    socket.set_linger(0)?;
    socket.set_rcvtimeo(i32::try_from(DRAIN_TIMEOUT.as_millis()).unwrap_or(i32::MAX))?;
    socket.bind(endpoint)?;
    Ok(socket)
}

/// REP スレッド `slowctl-cmd` の本体(TODO/124 Fable 1)。`stop` が立つまで回る。
///
/// REQ / REP なので受けた要求には**必ず 1 通**返す。recv / send が失敗したら(状態機械が
/// 壊れた = `EFSM` 等)ソケットを作り直す(開けるまで刻んで再試行、停止合図は見る)。
fn cmd_loop(
    context: &zmq::Context,
    endpoint: &str,
    mut socket: zmq::Socket,
    targets: &[PowerTarget],
    stop: &AtomicBool,
) {
    while !stop.load(Ordering::Relaxed) {
        let failure = match socket.recv_bytes(0) {
            Ok(request) => {
                let reply = handle_power_request(&request, targets, POWER_REPLY_TIMEOUT);
                info!(
                    request = %String::from_utf8_lossy(&request),
                    reply = %reply,
                    "slowctl: power request answered"
                );
                socket.send(reply.to_string().as_bytes(), 0).err()
            }
            Err(zmq::Error::EAGAIN) | Err(zmq::Error::EINTR) => None,
            Err(e) => Some(e),
        };
        let Some(e) = failure else {
            continue;
        };
        warn!(endpoint, error = %e, "slowctl: the power REP failed — rebuilding the socket");
        drop(socket);
        socket = loop {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            match bind_cmd_rep(context, endpoint) {
                Ok(socket) => break socket,
                Err(e) => {
                    warn!(endpoint, error = %e, "slowctl: cannot rebind the power REP — retrying");
                    std::thread::sleep(DRAIN_TIMEOUT);
                }
            }
        };
    }
}

// ---------------------------------------------------------------------
// 本体(集約 + status PUB + LogPost)
// ---------------------------------------------------------------------

/// 1 デバイスぶんのアラーム状態(規則ごとの「今 ALARM 中か」)。
struct DeviceAlarms {
    rules: Vec<AlarmRule>,
    firing: Vec<bool>,
}

/// 集約器 —— ZMQ を知らない(単体テストが直接叩ける)。
pub struct Aggregator {
    policy: LogPolicy,
    alarms: Vec<DeviceAlarms>,
    /// `Sample::key()` → 最新値(status PUB の「最新値のみ」)。
    latest: BTreeMap<String, Sample>,
    /// `Sample::key()` → 前回 **logbook に書いた** (値, ts_ms)。
    logged: BTreeMap<String, (f64, u64)>,
}

impl Aggregator {
    pub fn new(devices: &[SlowDeviceConfig], log_every_s: u64) -> Self {
        Self {
            policy: LogPolicy { log_every_s },
            alarms: devices
                .iter()
                .map(|d| DeviceAlarms {
                    rules: d.alarms.clone(),
                    firing: vec![false; d.alarms.len()],
                })
                .collect(),
            latest: BTreeMap::new(),
            logged: BTreeMap::new(),
        }
    }

    /// 1 デバイスぶんのサンプル群を取り込み、**書くべき logbook レコード**を返す。
    ///
    /// SPEC §9.2: ALARM / RECOVER は即時、VALUE は [`LogPolicy`]。
    pub fn ingest(&mut self, device_index: usize, samples: Vec<Sample>) -> Vec<LogbookRecord> {
        let mut records = Vec::new();
        for sample in samples {
            let key = sample.key();

            // 1) しきい値の出入り(規則ごと)。
            if let Some(alarms) = self.alarms.get_mut(device_index) {
                for (rule_index, rule) in alarms.rules.iter().enumerate() {
                    let was = alarms.firing.get(rule_index).copied().unwrap_or(false);
                    if let Some(event) = rule.evaluate(was, &sample) {
                        if let Some(slot) = alarms.firing.get_mut(rule_index) {
                            *slot = event == AlarmEvent::Alarm;
                        }
                        records.push(slow_record(&sample, event.as_str()));
                        // ALARM/RECOVER を書いた時点が「最後に書いた値」でもある。
                        self.logged
                            .insert(key.clone(), (sample.value, sample.ts_ms));
                    }
                }
            }

            // 2) 機器側 Alarm info(1/20)≠ 0 —— 規則が無くても記録に残す(SPEC §9.2)。
            //    立ち上がりだけ書く(1 Hz で連打しない)。
            let was_hw_alarm = self.latest.get(&key).map(|s| s.alarm).unwrap_or(false);
            if sample.alarm && !was_hw_alarm {
                records.push(slow_record(&sample, AlarmEvent::Alarm.as_str()));
                self.logged
                    .insert(key.clone(), (sample.value, sample.ts_ms));
            } else if !sample.alarm && was_hw_alarm {
                records.push(slow_record(&sample, AlarmEvent::Recover.as_str()));
                self.logged
                    .insert(key.clone(), (sample.value, sample.ts_ms));
            }

            // 3) VALUE(2 % か log_every_s)。
            let last = self.logged.get(&key).copied();
            if self.policy.should_log(last, sample.value, sample.ts_ms) {
                records.push(slow_record(&sample, "VALUE"));
                self.logged
                    .insert(key.clone(), (sample.value, sample.ts_ms));
            }

            self.latest.insert(key, sample);
        }
        records
    }

    /// SPEC §2.3 の PUB ペイロード(最新値のみ、キー順で決定的)。
    pub fn payload(&self, counters: SlowCounters) -> SlowPayload {
        SlowPayload {
            kind: SLOW_KIND.to_string(),
            ts_ms: now_ms(),
            samples: self.latest.values().cloned().collect(),
            counters,
        }
    }
}

/// VALUE / ALARM / RECOVER の 1 レコード(SPEC §9.2 の `{<param>: {value, unit}}`)。
fn slow_record(sample: &Sample, event: &str) -> LogbookRecord {
    LogbookRecord::Slow {
        actor: SLOW_ACTOR.to_string(),
        device: sample.device.clone(),
        node: sample.node,
        event: event.to_string(),
        values: {
            // `json!` のキーはリテラルに限られるので Map を直に組む。
            let mut map = serde_json::Map::new();
            map.insert(
                sample.param.clone(),
                json!({"value": sample.value, "unit": sample.unit}),
            );
            serde_json::Value::Object(map)
        },
    }
}

/// IDENT の 1 レコード(SPEC §9.2 の `{<field>: "<string>"}`)。
fn ident_record(ident: &Ident) -> LogbookRecord {
    LogbookRecord::Slow {
        actor: SLOW_ACTOR.to_string(),
        device: ident.device.clone(),
        node: ident.node,
        event: "IDENT".to_string(),
        values: serde_json::to_value(&ident.fields).unwrap_or(serde_json::Value::Null),
    }
}

/// ERROR の 1 レコード(SPEC §9.2 の 4 項目の増分)。
fn error_record(device: &str, delta: &SlowCounters) -> LogbookRecord {
    LogbookRecord::Slow {
        actor: SLOW_ACTOR.to_string(),
        device: device.to_string(),
        node: None,
        event: "ERROR".to_string(),
        values: json!({
            "poll_err": delta.poll_err,
            "parse_err": delta.parse_err,
            "reconnects": delta.reconnects,
            "identity_mismatch": delta.identity_mismatch,
        }),
    }
}

// ---------------------------------------------------------------------
// ワイヤ(SPEC §2.2 エンベロープ + §2.3 map 形式ペイロード)
// ---------------------------------------------------------------------

/// SPEC §2.2 のエンベロープ(`{"Data": [source_id, run, seq, created_ns, payload]}`)を
/// 手で組み、その 5 番目に **map 形式 msgpack のペイロード**を差す。
///
/// root-sink(C++ の `monitor_pub.hpp`)と**同じバイト構成**にするための手組み:
/// `rmp_serde` の 1 回の呼び出しでは「エンベロープは positional・ペイロードは named」を
/// 作り分けられない(シリアライザ全体の設定になる)ため。
pub fn encode_pub(
    source_id: u32,
    run_number: u32,
    sequence_number: u64,
    created_ns: u64,
    payload: &SlowPayload,
) -> Result<Vec<u8>, rmp_serde::encode::Error> {
    let body = rmp_serde::to_vec_named(payload)?;
    let mut out = vec![0x81]; // fixmap(1)
    out.extend_from_slice(&[0xa4, b'D', b'a', b't', b'a']); // "Data"
    out.push(0x95); // fixarray(5)
    for value in [
        u64::from(source_id),
        u64::from(run_number),
        sequence_number,
        created_ns,
    ] {
        out.push(0xcf); // uint64
        out.extend_from_slice(&value.to_be_bytes());
    }
    out.extend_from_slice(&body);
    Ok(out)
}

fn created_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

// ---------------------------------------------------------------------
// 実プロセス
// ---------------------------------------------------------------------

/// slowctl を走らせる(**同期** —— 呼び手が専用スレッド or `spawn_blocking` を用意する)。
///
/// `stop` が立つまで回り、立ったらデバイススレッドを畳んでソケットを閉じる。
pub fn run_slowctl(params: SlowctlParams, stop: Arc<AtomicBool>) -> Result<(), SlowctlError> {
    // TODO/122: InfluxDB のトークンを読むのは slowctl の起動時だけ(config の検証では読まない
    // —— controller / monitor を巻き込まない)。ソケットを開く前に読み、読めなければ何も
    // 開かずに起動失敗にする。
    let influx_params = params
        .influx
        .as_ref()
        .map(InfluxParams::from_config)
        .transpose()
        .map_err(SlowctlError::Influx)?;

    let context = zmq::Context::new();

    let publisher = context
        .socket(zmq::PUB)
        .and_then(|s| {
            crate::zmq_helper::apply_pub_hwm(&s)?;
            s.set_linger(0)?;
            s.bind(&params.status_pub)?;
            Ok(s)
        })
        .map_err(|source| SlowctlError::Pub {
            endpoint: params.status_pub.clone(),
            source,
        })?;
    let bound = publisher
        .get_last_endpoint()
        .ok()
        .and_then(Result::ok)
        .unwrap_or_else(|| params.status_pub.clone());
    info!(
        endpoint = bound,
        "slowctl: status PUB bound (SPEC §3.2 = 47006)"
    );

    // LogPost は記録系だが、controller 不在で計測を止めない —— 送信はノンブロッキング。
    let log_push = context
        .socket(zmq::PUSH)
        .and_then(|s| {
            crate::zmq_helper::apply_push_hwm(&s)?;
            s.set_sndtimeo(LOG_SEND_TIMEOUT_MS)?;
            s.set_linger(0)?;
            s.connect(&params.log_push)?;
            Ok(s)
        })
        .map_err(|source| SlowctlError::LogPush {
            endpoint: params.log_push.clone(),
            source,
        })?;
    info!(
        endpoint = params.log_push,
        "slowctl: LogPost PUSH connected"
    );

    // TODO/122: 全サンプルを InfluxDB へ(任意)。
    let influx = match influx_params {
        Some(influx_params) => {
            info!(
                url = influx_params.url,
                org = influx_params.org,
                bucket = influx_params.bucket,
                batch_ms = influx_params.batch.as_millis(),
                buffer_max_points = influx_params.buffer_max_points,
                "slowctl: writing every sample to InfluxDB"
            );
            let writer =
                InfluxWriter::spawn(influx_params).map_err(|source| SlowctlError::Spawn {
                    device: "influx".to_string(),
                    source,
                })?;
            Some(writer)
        }
        None => None,
    };

    // TODO/124: power ON/OFF の REP(opt-in)。bind の失敗は起動失敗(PUB と同じ扱い)。
    let cmd_socket = match &params.cmd_bind {
        Some(endpoint) => {
            let socket = bind_cmd_rep(&context, endpoint).map_err(|source| SlowctlError::Cmd {
                endpoint: endpoint.clone(),
                source,
            })?;
            info!(
                endpoint,
                "slowctl: power on/off REP bound (SPEC §3.2 = 47007, localhost only)"
            );
            Some((endpoint.clone(), socket))
        }
        None => {
            info!("slowctl: no cmd_bind — the power on/off REP stays closed");
            None
        }
    };

    let (tx, rx) = mpsc::sync_channel(EVENT_QUEUE);
    let mut counters = Vec::with_capacity(params.devices.len());
    let mut threads = Vec::with_capacity(params.devices.len());
    let mut targets = Vec::with_capacity(params.devices.len());
    for device in &params.devices {
        let device_counters = Arc::new(Counters::default());
        counters.push(Arc::clone(&device_counters));
        let (commands, command_rx) = mpsc::sync_channel(POWER_QUEUE_DEPTH);
        targets.push(PowerTarget {
            name: device.name.clone(),
            device_type: device.device_type.clone(),
            commands,
        });
        let handle = std::thread::Builder::new()
            .name(format!("slowctl-{}", device.name))
            .spawn({
                let device = device.clone();
                let tx = tx.clone();
                let stop = Arc::clone(&stop);
                let poll_interval = params.device_poll_interval(&device);
                let influx = influx.as_ref().map(InfluxWriter::sink);
                move || {
                    device_loop(
                        device,
                        poll_interval,
                        tx,
                        device_counters,
                        stop,
                        influx,
                        command_rx,
                    )
                }
            })
            .map_err(|source| SlowctlError::Spawn {
                device: device.name.clone(),
                source,
            })?;
        threads.push(handle);
    }
    drop(tx); // 本体は受けるだけ(全スレッドが終われば recv が切れる)

    // REP が無ければ送り手(targets)はここで落ちる —— device_loop はただ寝る。
    let cmd_thread = match cmd_socket {
        Some((endpoint, socket)) => {
            let handle = std::thread::Builder::new()
                .name("slowctl-cmd".to_string())
                .spawn({
                    let context = context.clone();
                    let stop = Arc::clone(&stop);
                    move || cmd_loop(&context, &endpoint, socket, &targets, &stop)
                })
                .map_err(|source| SlowctlError::Spawn {
                    device: "slowctl-cmd".to_string(),
                    source,
                })?;
            Some(handle)
        }
        None => {
            drop(targets);
            None
        }
    };

    main_loop(
        &params,
        &rx,
        &publisher,
        &log_push,
        &counters,
        influx.as_ref().map(|w| w.counters().as_ref()),
        &stop,
    );

    stop.store(true, Ordering::Relaxed);
    for handle in threads {
        if handle.join().is_err() {
            error!("slowctl: a device thread panicked");
        }
    }
    if let Some(handle) = cmd_thread {
        if handle.join().is_err() {
            error!("slowctl: the power REP thread panicked");
        }
    }
    // デバイススレッドの sink が全部落ちた後でないと writer は畳めない(InfluxWriter::finish)。
    if let Some(writer) = influx {
        writer.finish();
    }
    info!("slowctl: stopped");
    Ok(())
}

/// 本体ループ(ZMQ とタイマだけ。集約の判断は [`Aggregator`])。
fn main_loop(
    params: &SlowctlParams,
    rx: &Receiver<DeviceEvent>,
    publisher: &zmq::Socket,
    log_push: &zmq::Socket,
    counters: &[Arc<Counters>],
    influx: Option<&InfluxCounters>,
    stop: &Arc<AtomicBool>,
) {
    let mut aggregator = Aggregator::new(&params.devices, params.log_every_s);
    // デバイス名 → インデックス(サンプルからどの規則集合を使うかを引く)。
    let index_of: BTreeMap<&str, usize> = params
        .devices
        .iter()
        .enumerate()
        .map(|(i, d)| (d.name.as_str(), i))
        .collect();

    let mut poster = LogPoster::new(log_push);
    let mut sequence_number = 0u64;
    let mut last_publish = Instant::now();
    let mut last_error_report = Instant::now();
    let mut previous: Vec<SlowCounters> = vec![SlowCounters::default(); params.devices.len()];

    while !stop.load(Ordering::Relaxed) {
        // 周回の先頭で再送を流す —— controller が後から上がっても 1 通目は IDENT になる。
        poster.flush();

        match rx.recv_timeout(DRAIN_TIMEOUT) {
            Ok(DeviceEvent::Ident(idents)) => {
                for ident in &idents {
                    poster.post(ident_record(ident));
                }
            }
            Ok(DeviceEvent::Samples(samples)) => {
                let device_index = samples
                    .first()
                    .and_then(|s| index_of.get(s.device.as_str()).copied())
                    .unwrap_or(0);
                for record in aggregator.ingest(device_index, samples) {
                    poster.post(record);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                warn!("slowctl: every device thread is gone — stopping");
                break;
            }
        }

        if last_publish.elapsed() >= PUBLISH_INTERVAL {
            last_publish = Instant::now();
            let mut totals = sum_counters(counters);
            if let Some(influx) = influx {
                influx.write_into(&mut totals);
            }
            let payload = aggregator.payload(totals);
            match encode_pub(
                SLOWCTL_SOURCE_ID,
                // slowctl は run チェーンの外に居る(run 番号を知らない)。SPEC §2.2 の
                // エンベロープは run_number を要求するので 0 を載せる。
                0,
                sequence_number,
                created_ns(),
                &payload,
            ) {
                Ok(bytes) => {
                    sequence_number += 1;
                    if let Err(e) = publisher.send(&bytes[..], zmq::DONTWAIT) {
                        // モニタ系 = 落としてよいが silent にしない(CLAUDE.md)。
                        warn!(error = %e, "slowctl: status PUB send dropped");
                    }
                }
                Err(e) => error!(error = %e, "slowctl: cannot encode the status payload"),
            }
        }

        if last_error_report.elapsed() >= ERROR_REPORT_INTERVAL {
            last_error_report = Instant::now();
            for (index, device) in params.devices.iter().enumerate() {
                let Some(now) = counters.get(index).map(|c| c.snapshot()) else {
                    continue;
                };
                let Some(previous_slot) = previous.get_mut(index) else {
                    continue;
                };
                if let Some(delta) = counter_delta(&now, previous_slot) {
                    poster.post(error_record(&device.name, &delta));
                }
                *previous_slot = now;
            }
        }
    }
}

// ---------------------------------------------------------------------
// LogPost(再送キュー付き)
// ---------------------------------------------------------------------

/// 投函に失敗したとき**再送する**レコードか(SPEC §9.2 の `event` 別)。
///
/// * `IDENT` / `ALARM` / `RECOVER` = **一度きりの事実**。落とすと二度と記録に現れない。
/// * `VALUE` = 次の VALUE が来る。`ERROR` = 次回の 1 分集計に増分が含まれる。
///   どちらも溜め込む意味がないので再送しない(古い値で logbook を埋めない)。
fn is_retryable(record: &LogbookRecord) -> bool {
    matches!(record, LogbookRecord::Slow { event, .. }
        if matches!(event.as_str(), "IDENT" | "ALARM" | "RECOVER"))
}

/// 届かなかった一度きりのレコードを抱えておく有界キュー(FIFO)。
///
/// 上限を超えたら**最古から**捨てる —— 新しい ALARM のほうが運用上の価値が高く、
/// 無制限に抱えるのは CLAUDE.md が禁じるメモリ暴走そのものだから。
#[derive(Debug, Default)]
pub struct RetryQueue {
    queue: std::collections::VecDeque<LogbookRecord>,
}

impl RetryQueue {
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// 末尾に積む。上限超過で最古を捨てたら `true`(呼び手が数えて warn する)。
    pub fn push(&mut self, record: LogbookRecord) -> bool {
        let evicted = self.queue.len() >= RETRY_QUEUE_LIMIT;
        if evicted {
            self.queue.pop_front();
        }
        self.queue.push_back(record);
        evicted
    }

    fn pop_front(&mut self) -> Option<LogbookRecord> {
        self.queue.pop_front()
    }

    /// 送れなかった 1 本を**先頭へ**戻す(投函順を崩さない)。
    fn push_front(&mut self, record: LogbookRecord) {
        self.queue.push_front(record);
    }
}

/// LogPost の投函口。**落ちても計測は止めない**が、一度きりのレコードは落とさない。
///
/// 1 周回につき失敗する送信は高々 1 本(最初の失敗で `down` を立て、その周回は
/// 以後 send を試さない)。`LOG_SEND_TIMEOUT_MS` のブロックが積み上がらないようにするため。
struct LogPoster<'a> {
    socket: &'a zmq::Socket,
    retry: RetryQueue,
    /// この周回で既に送信が失敗したか(次の [`LogPoster::flush`] で降りる)。
    down: bool,
    /// 捨てた投稿の累計(キュー溢れ + 再送しない種別の送信失敗)。
    /// SPEC §2.3 の `counters` には載らない slowctl 自身の健全性。
    dropped: u64,
}

impl<'a> LogPoster<'a> {
    fn new(socket: &'a zmq::Socket) -> Self {
        Self {
            socket,
            retry: RetryQueue::default(),
            down: false,
            dropped: 0,
        }
    }

    /// 周回の先頭で呼ぶ: `down` を降ろし、溜まった再送を**投函順に**流す。
    ///
    /// 新しいレコードより先に流すので、controller が後から上がっても
    /// **1 通目は IDENT** になる(実運用の起動順 = controller が最後、docs/NEW_DAQ_SETUP_ja.md)。
    fn flush(&mut self) {
        self.down = false;
        // `front()` の借用を跨いで `try_send(&mut self)` を呼べないので、1 本ずつ取り出して
        // 失敗したら**先頭に戻す**(順序は保たれる)。
        while let Some(record) = self.retry.pop_front() {
            if !self.try_send(&record) {
                // 相手がまだ居ない。残りはこの周回では試さない(200 ms を積み上げない)。
                self.retry.push_front(record);
                return;
            }
        }
    }

    /// 1 本投函する。落ちたら種別に応じて再送キューへ回すか、数えて捨てる。
    fn post(&mut self, record: LogbookRecord) {
        if !self.down && self.try_send(&record) {
            return;
        }
        if is_retryable(&record) {
            if self.retry.push(record) {
                self.dropped += 1;
                warn!(
                    log_dropped = self.dropped,
                    limit = RETRY_QUEUE_LIMIT,
                    "slowctl: the LogPost retry queue is full — dropped the oldest record"
                );
            }
        } else {
            self.dropped += 1;
        }
    }

    /// 実際に送る。失敗したら `down` を立てて `false`。
    fn try_send(&mut self, record: &LogbookRecord) -> bool {
        let json = match serde_json::to_vec(record) {
            Ok(json) => json,
            Err(e) => {
                // 符号化できないものは何度試しても通らない。捨てる以外にない(が黙らない)。
                error!(error = %e, "slowctl: cannot encode a logbook record");
                return true;
            }
        };
        // `DONTWAIT` は使わない —— `LOG_SEND_TIMEOUT_MS` を効かせるため(理由はその定数の doc)。
        match self.socket.send(&json[..], 0) {
            Ok(()) => true,
            Err(e) => {
                if !self.down {
                    warn!(error = %e, queued = self.retry.len(), log_dropped = self.dropped,
                          "slowctl: LogPost did not go through (is the controller running?)");
                }
                self.down = true;
                false
            }
        }
    }
}

// ---------------------------------------------------------------------
// テスト
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::slowctl::NodesSpec;

    fn device(name: &str, alarms: Vec<AlarmRule>) -> SlowDeviceConfig {
        SlowDeviceConfig {
            device_type: "fake".to_string(),
            name: name.to_string(),
            port: Some("script.txt".to_string()),
            addr: None,
            nodes: NodesSpec::List(vec![3]),
            expect_serial: String::new(),
            alarms,
            poll_s: None,
            status_ignore: Vec::new(),
        }
    }

    /// TODO/119: 機器の `poll_s` が周期に反映され、無ければ全体の `poll_hz` 由来。
    #[test]
    fn a_device_poll_s_overrides_the_global_interval() {
        let mut slow = device("hv", Vec::new());
        let params = SlowctlParams {
            status_pub: String::new(),
            cmd_bind: None,
            log_push: String::new(),
            poll_hz: 2.0,
            log_every_s: 60,
            devices: vec![slow.clone()],
            influx: None,
        };
        assert_eq!(
            params.device_poll_interval(&slow),
            Duration::from_millis(500)
        );
        slow.poll_s = Some(5.0);
        assert_eq!(params.device_poll_interval(&slow), Duration::from_secs(5));
    }

    fn sample(value: f64, ts_ms: u64) -> Sample {
        Sample {
            device: "gas".to_string(),
            node: Some(3),
            param: "fmeasure".to_string(),
            value,
            unit: "mbar".to_string(),
            ts_ms,
            alarm: false,
        }
    }

    // -----------------------------------------------------------------
    // open_driver の経路(TODO/079 §2.7: `arduino_env` — 実機には触らない。
    // config パースの単体として `type` と `port` の検証まで)。
    // -----------------------------------------------------------------

    /// `type = "arduino_env"` は開こうとする(= ルーティングが効いている)。実機は無いので
    /// 「存在しないポートを開こうとして失敗する」ところまでで止める —— これで
    /// `open_driver` が `arduino::open_serial` に配線されていることが確認できる
    /// (未実装のままなら "not implemented yet" というメッセージになり、この assert は落ちる)。
    #[test]
    fn open_driver_routes_arduino_env_to_the_arduino_serial_opener() {
        let mut config = device("env", Vec::new());
        config.device_type = "arduino_env".to_string();
        config.port = Some("/dev/serial/by-id/does-not-exist-079-arduino-test".to_string());

        // `Box<dyn Driver>` は `Debug` を持たないので `unwrap_err()` は使えない。
        let error = match open_driver(&config) {
            Err(e) => e,
            Ok(_) => panic!("a nonexistent serial port should not open"),
        };
        let SlowctlError::Device { device, message } = error else {
            panic!("expected SlowctlError::Device");
        };
        assert_eq!(device, "env");
        assert!(
            message.contains("cannot open serial port"),
            "message = {message:?}(open_driver がルーティングされていない可能性)"
        );
        assert!(
            !message.contains("not implemented yet"),
            "arduino_env は Phase 3 で実装済みのはず: {message:?}"
        );
    }

    /// `port` が無いのは設定検証で弾かれるはずだが、`open_driver` 自体も
    /// bronkhorst と同じ防御を持つ(TODO/079 §2.6 の `missing_port` と同じ形)。
    #[test]
    fn open_driver_of_arduino_env_without_a_port_is_a_device_error() {
        let mut config = device("env", Vec::new());
        config.device_type = "arduino_env".to_string();
        config.port = None;

        assert!(matches!(
            open_driver(&config),
            Err(SlowctlError::Device { .. })
        ));
    }

    // -----------------------------------------------------------------
    // open_driver の経路(TODO/089): `hivolta` は `addr` があれば TCP を優先し、
    // `hmp2020` は `port`(serial)経路。実機は無いので「開こうとして失敗する」
    // ところまでで止める(arduino_env のテストの流儀そのもの)。
    // -----------------------------------------------------------------

    /// `addr` があれば TCP 接続を試みる(接続拒否まで確認 = ルーティングが効いている証拠)。
    #[test]
    fn open_driver_of_hivolta_with_an_addr_prefers_tcp_and_reports_connection_refused() {
        let mut config = device("hv", Vec::new());
        config.device_type = "hivolta".to_string();
        config.port = None;
        // 誰も listen していないポート(bind して即 drop —— tests/…/ecc_bridge.rs と同じ流儀)。
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let dead_port = probe.local_addr().unwrap().port();
        drop(probe);
        config.addr = Some(format!("127.0.0.1:{dead_port}"));

        let error = match open_driver(&config) {
            Err(e) => e,
            Ok(_) => panic!("nobody is listening on that port"),
        };
        let SlowctlError::Device { device, message } = error else {
            panic!("expected SlowctlError::Device");
        };
        assert_eq!(device, "hv");
        assert!(
            message.contains("cannot connect to hivolta"),
            "message = {message:?}(addr 優先で TCP を試みたことの確認)"
        );
        assert!(!message.contains("not implemented yet"));
    }

    /// `addr` が無ければ `port`(serial)経路にフォールバックする。
    #[test]
    fn open_driver_of_hivolta_without_an_addr_falls_back_to_serial() {
        let mut config = device("hv", Vec::new());
        config.device_type = "hivolta".to_string();
        config.addr = None;
        config.port = Some("/dev/serial/by-id/does-not-exist-089-hivolta-test".to_string());

        // TODO/119 §5: 開くのは ident / poll のとき(open_driver 自体は開かない)。
        let mut driver = open_driver(&config).unwrap();
        let message = driver.ident().unwrap_err().to_string();
        assert!(
            message.contains("cannot open serial port"),
            "message = {message:?}(addr が無ければ port へ落ちることの確認)"
        );
    }

    /// `addr` も `port` も無ければ設定検証で弾かれるはずだが、`open_driver` 自体も防御を持つ。
    #[test]
    fn open_driver_of_hivolta_without_addr_or_port_is_a_device_error() {
        let mut config = device("hv", Vec::new());
        config.device_type = "hivolta".to_string();
        config.addr = None;
        config.port = None;

        assert!(matches!(
            open_driver(&config),
            Err(SlowctlError::Device { .. })
        ));
    }

    /// `hmp2020` は `port`(serial)経路(arduino_env のテストの流儀)。
    #[test]
    fn open_driver_routes_hmp2020_to_the_hmp_serial_opener() {
        let mut config = device("hmp", Vec::new());
        config.device_type = "hmp2020".to_string();
        config.port = Some("/dev/serial/by-id/does-not-exist-089-hmp-test".to_string());

        let error = match open_driver(&config) {
            Err(e) => e,
            Ok(_) => panic!("a nonexistent serial port should not open"),
        };
        let SlowctlError::Device { device, message } = error else {
            panic!("expected SlowctlError::Device");
        };
        assert_eq!(device, "hmp");
        assert!(
            message.contains("cannot open serial port"),
            "message = {message:?}(open_driver がルーティングされていない可能性)"
        );
        assert!(!message.contains("not implemented yet"));
    }

    /// `port` が無いのは設定検証で弾かれるはずだが、`open_driver` 自体も防御を持つ。
    #[test]
    fn open_driver_of_hmp2020_without_a_port_is_a_device_error() {
        let mut config = device("hmp", Vec::new());
        config.device_type = "hmp2020".to_string();
        config.port = None;

        assert!(matches!(
            open_driver(&config),
            Err(SlowctlError::Device { .. })
        ));
    }

    fn events(records: &[LogbookRecord]) -> Vec<&str> {
        records
            .iter()
            .map(|r| match r {
                LogbookRecord::Slow { event, .. } => event.as_str(),
                _ => "?",
            })
            .collect()
    }

    /// しきい値を跨ぐと ALARM、戻ると RECOVER。間の同値サンプルでは何も書かない。
    #[test]
    fn crossing_a_threshold_writes_alarm_then_recover() {
        let rule = AlarmRule {
            node: None,
            param: "fmeasure".to_string(),
            lo: 90.0,
            hi: 110.0,
        };
        let devices = vec![device("gas", vec![rule])];
        let mut aggregator = Aggregator::new(&devices, 60);

        // 初回は VALUE(前回が無い)。
        assert_eq!(
            events(&aggregator.ingest(0, vec![sample(100.0, 0)])),
            ["VALUE"]
        );
        // 同じ値・1 s 後 = 何も書かない(2 % 未満・60 s 未満)。
        assert!(aggregator.ingest(0, vec![sample(100.0, 1_000)]).is_empty());
        // 逸脱 → ALARM(値も大きく動くので VALUE も続く)。
        assert_eq!(
            events(&aggregator.ingest(0, vec![sample(120.0, 2_000)])),
            ["ALARM"]
        );
        // 逸脱したまま = 再通知しない。
        assert!(aggregator.ingest(0, vec![sample(120.0, 3_000)]).is_empty());
        // 戻る → RECOVER。
        assert_eq!(
            events(&aggregator.ingest(0, vec![sample(100.0, 4_000)])),
            ["RECOVER"]
        );
    }

    /// 規則が無くても、機器側 Alarm info の**立ち上がり/立ち下がり**は記録に残る。
    #[test]
    fn a_hardware_alarm_bit_is_recorded_even_without_a_rule() {
        let devices = vec![device("gas", Vec::new())];
        let mut aggregator = Aggregator::new(&devices, 60);
        aggregator.ingest(0, vec![sample(100.0, 0)]);

        let mut hot = sample(100.0, 1_000);
        hot.alarm = true;
        assert_eq!(events(&aggregator.ingest(0, vec![hot.clone()])), ["ALARM"]);
        // 立ったままなら連打しない。
        let mut still_hot = hot.clone();
        still_hot.ts_ms = 2_000;
        assert!(aggregator.ingest(0, vec![still_hot]).is_empty());
        assert_eq!(
            events(&aggregator.ingest(0, vec![sample(100.0, 3_000)])),
            ["RECOVER"]
        );
    }

    /// PUB ペイロードは **最新値のみ**(同じキーは上書き)、キー順で決定的。
    #[test]
    fn the_payload_keeps_only_the_latest_value_per_key() {
        let devices = vec![device("gas", Vec::new())];
        let mut aggregator = Aggregator::new(&devices, 60);
        let mut temperature = sample(21.5, 0);
        temperature.param = "temperature".to_string();
        aggregator.ingest(0, vec![sample(100.0, 0), temperature]);
        aggregator.ingest(0, vec![sample(130.0, 1_000)]);

        let payload = aggregator.payload(SlowCounters::default());
        assert_eq!(payload.kind, SLOW_KIND);
        let keys: Vec<String> = payload.samples.iter().map(|s| s.key()).collect();
        assert_eq!(keys, vec!["gas/3/fmeasure", "gas/3/temperature"]);
        assert_eq!(payload.samples[0].value, 130.0, "最新値で上書き");
    }

    /// ERROR は増分で、増分が 0 なら書かない(SPEC §9.2)。
    #[test]
    fn the_error_record_carries_the_delta_and_is_skipped_when_empty() {
        let previous = SlowCounters {
            poll_ok: 10,
            poll_err: 1,
            parse_err: 0,
            reconnects: 0,
            identity_mismatch: 0,
            ..SlowCounters::default()
        };
        assert_eq!(
            counter_delta(&previous, &previous),
            None,
            "増分 0 なら書かない"
        );

        let now = SlowCounters {
            poll_ok: 20,
            poll_err: 4,
            parse_err: 2,
            reconnects: 1,
            identity_mismatch: 0,
            ..SlowCounters::default()
        };
        let delta = counter_delta(&now, &previous).unwrap();
        assert_eq!(delta.poll_err, 3);
        assert_eq!(delta.parse_err, 2);
        assert_eq!(delta.reconnects, 1);
        assert_eq!(delta.poll_ok, 0, "ERROR に成功数は載せない");

        let record = error_record("gas", &delta);
        assert_eq!(
            serde_json::to_string(&record).unwrap(),
            r#"{"type":"slow","actor":"slowctl","device":"gas","node":null,"event":"ERROR","values":{"identity_mismatch":0,"parse_err":2,"poll_err":3,"reconnects":1}}"#
        );
    }

    /// カウンタは種別を潰さずに振り分けられる(silent failure を作らない)。
    #[test]
    fn driver_errors_land_in_the_right_counter() {
        let counters = Counters::default();
        counters.record(&DriverError::Timeout);
        counters.record(&DriverError::Protocol("x".to_string()));
        counters.record(&DriverError::Identity {
            expected: "a".to_string(),
            got: "b".to_string(),
        });
        let snapshot = counters.snapshot();
        assert_eq!(snapshot.poll_err, 1);
        assert_eq!(snapshot.parse_err, 1);
        assert_eq!(snapshot.identity_mismatch, 1);
    }

    /// 発注書 §2.6-4: PUB は SPEC §2.2 のエンベロープ + §2.3 の map ペイロード。
    #[test]
    fn the_pub_wire_is_readable_by_the_monitor_decoder() {
        let devices = vec![device("gas", Vec::new())];
        let mut aggregator = Aggregator::new(&devices, 60);
        aggregator.ingest(0, vec![sample(130.5, 7)]);
        let payload = aggregator.payload(SlowCounters {
            poll_ok: 3,
            ..SlowCounters::default()
        });

        let bytes = encode_pub(
            SLOWCTL_SOURCE_ID,
            0,
            42,
            1_755_000_000_000_000_000,
            &payload,
        )
        .unwrap();
        let message = crate::monitor::decode_message(&bytes).unwrap();
        assert_eq!(message.source_id, 200, "SPEC §3.2: slowctl = 200");
        assert_eq!(message.sequence_number, 42);
        let slow = message.slow().unwrap();
        assert_eq!(slow.kind, SLOW_KIND);
        assert_eq!(slow.samples[0].value, 130.5);
        assert_eq!(slow.samples[0].unit, "mbar");
        assert_eq!(slow.counters.poll_ok, 3);
    }

    // -----------------------------------------------------------------
    // LogPost の再送(079 Phase 2 レビュー指摘)
    // -----------------------------------------------------------------

    fn slow_event(event: &str) -> LogbookRecord {
        LogbookRecord::Slow {
            actor: SLOW_ACTOR.to_string(),
            device: "gas".to_string(),
            node: Some(3),
            event: event.to_string(),
            values: json!({}),
        }
    }

    /// 一度きりの事実(IDENT / ALARM / RECOVER)だけ再送する。
    /// VALUE は次が来る、ERROR は次回集計に入るので、古い値で logbook を埋めない。
    #[test]
    fn only_one_off_records_are_worth_resending() {
        assert!(is_retryable(&slow_event("IDENT")));
        assert!(is_retryable(&slow_event("ALARM")));
        assert!(is_retryable(&slow_event("RECOVER")));
        assert!(!is_retryable(&slow_event("VALUE")));
        assert!(!is_retryable(&slow_event("ERROR")));
        // slow 以外(controller が書くもの)は slowctl の再送対象ではない。
        assert!(!is_retryable(&LogbookRecord::Comment {
            actor: "ui".to_string(),
            author: "aogaki".to_string(),
            text: "x".to_string(),
        }));
    }

    /// 再送キューは FIFO で、上限を超えたら**最古から**捨てて呼び手に知らせる。
    #[test]
    fn the_retry_queue_is_bounded_and_drops_the_oldest_first() {
        let mut queue = RetryQueue::default();
        assert!(queue.is_empty());

        // 上限ちょうどまでは 1 本も捨てない。
        for i in 0..RETRY_QUEUE_LIMIT {
            assert!(!queue.push(slow_event(&format!("IDENT{i}"))), "i = {i}");
        }
        assert_eq!(queue.len(), RETRY_QUEUE_LIMIT);

        // 上限 + 1 本目で最古(IDENT0)が落ちる。長さは上限のまま。
        assert!(queue.push(slow_event("ALARM")));
        assert_eq!(queue.len(), RETRY_QUEUE_LIMIT);
        let LogbookRecord::Slow { event, .. } = queue.pop_front().unwrap() else {
            panic!("retry queue holds slow records");
        };
        assert_eq!(event, "IDENT1", "先頭は IDENT0 ではなく IDENT1");

        // 送れなかった 1 本を戻すと順序が保たれる。
        queue.push_front(slow_event("IDENT1"));
        let LogbookRecord::Slow { event, .. } = queue.pop_front().unwrap() else {
            panic!("retry queue holds slow records");
        };
        assert_eq!(event, "IDENT1");
    }

    /// 相手が居ない間は溜め、相手が現れたら**投函順に**流れる(実配線で確かめる)。
    ///
    /// これが実運用の姿そのもの: 起動順で controller は最後に上がる
    /// (docs/NEW_DAQ_SETUP_ja.md)ので、slowctl の 1 通目 = IDENT は必ず 1 度失敗する。
    #[test]
    fn records_queued_while_the_controller_is_down_arrive_in_order_once_it_is_up() {
        let context = zmq::Context::new();
        let endpoint = {
            let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = probe.local_addr().unwrap().port();
            drop(probe);
            format!("tcp://127.0.0.1:{port}")
        };

        let push = context.socket(zmq::PUSH).unwrap();
        crate::zmq_helper::apply_push_hwm(&push).unwrap();
        push.set_sndtimeo(50).unwrap(); // テストを待たせない(本番は LOG_SEND_TIMEOUT_MS)
        push.set_linger(0).unwrap();
        push.connect(&endpoint).unwrap();

        let mut poster = LogPoster::new(&push);
        // 相手はまだ居ない: IDENT と ALARM は溜まり、VALUE は捨てられる。
        poster.flush();
        poster.post(slow_event("IDENT"));
        poster.post(slow_event("VALUE"));
        poster.post(slow_event("ALARM"));
        assert_eq!(poster.retry.len(), 2, "VALUE は溜めない");
        assert_eq!(poster.dropped, 1, "捨てた VALUE は数える");

        // controller が上がる。
        let pull = context.socket(zmq::PULL).unwrap();
        pull.set_rcvtimeo(2000).unwrap();
        pull.set_linger(0).unwrap();
        pull.bind(&endpoint).unwrap();

        // 何周か回れば接続が張られて流れ切る(1 周に 1 本しか失敗を試さない設計なので、
        // 接続が張られるまでの周回が要る)。
        for _ in 0..40 {
            poster.flush();
            if poster.retry.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        assert!(poster.retry.is_empty(), "再送が流れ切っていない");

        let mut events = Vec::new();
        for _ in 0..2 {
            let raw = pull.recv_bytes(0).expect("queued record");
            let record: LogbookRecord = serde_json::from_slice(&raw).unwrap();
            let LogbookRecord::Slow { event, .. } = record else {
                panic!("expected a slow record");
            };
            events.push(event);
        }
        assert_eq!(events, vec!["IDENT", "ALARM"], "投函順のまま届く");
        assert_eq!(poster.dropped, 1, "捨てたのは VALUE の 1 本だけ");
    }

    // -----------------------------------------------------------------
    // TODO/124: power ON/OFF の REP(要求の検証・キュー・timeout)
    // -----------------------------------------------------------------

    /// REP から見た 1 台と、テストが device_loop 役として受けるキュー。
    fn target(name: &str, device_type: &str) -> (PowerTarget, Receiver<PowerCommand>) {
        let (commands, rx) = mpsc::sync_channel(POWER_QUEUE_DEPTH);
        (
            PowerTarget {
                name: name.to_string(),
                device_type: device_type.to_string(),
                commands,
            },
            rx,
        )
    }

    const SHORT: Duration = Duration::from_millis(50);

    /// 正しい要求は機器のキューに入り、device_loop の返した応答がそのまま REP に返る。
    #[test]
    fn a_power_request_reaches_the_device_queue_and_its_reply_comes_back() {
        let (hv, hv_rx) = target("hv", "hivolta");
        let device = std::thread::spawn(move || {
            let command = hv_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!((command.ch, command.on), (3, false));
            command
                .reply
                .send(json!({"ok": true, "from": "device"}))
                .unwrap();
        });

        let reply = handle_power_request(
            br#"{"op":"power","device":"hv","ch":3,"on":false}"#,
            &[hv],
            Duration::from_secs(5),
        );

        assert_eq!(reply, json!({"ok": true, "from": "device"}));
        device.join().unwrap();
    }

    /// 不正な要求は**キューに入れずに** 1 通で返す(ok:false、before:null、理由つき)。
    #[test]
    fn bad_requests_are_refused_with_the_reason_and_never_queued() {
        let (hv, hv_rx) = target("hv", "hivolta");
        let (lv, lv_rx) = target("lv", "hmp2020");
        let (gas, gas_rx) = target("gas", "bronkhorst_flowbus");
        let targets = [hv, lv, gas];

        let cases: [(&[u8], &str); 11] = [
            (b"not json", "bad request"),
            (
                br#"{"op":"set","device":"hv","ch":3,"on":true}"#,
                "unknown op",
            ),
            (br#"{"device":"hv","ch":3,"on":true}"#, "bad request"),
            (br#"{"op":"power","device":"hv","ch":3}"#, "bad request"),
            (
                br#"{"op":"power","device":"nope","ch":3,"on":true}"#,
                "unknown device",
            ),
            (
                br#"{"op":"power","device":"gas","ch":3,"on":true}"#,
                "no power on/off",
            ),
            // hivolta = 0..=7(CH:8 は全 ch まとめての意味になる)。
            (
                br#"{"op":"power","device":"hv","ch":8,"on":true}"#,
                "out of range",
            ),
            (
                br#"{"op":"power","device":"hv","ch":-1,"on":true}"#,
                "out of range",
            ),
            (
                br#"{"op":"power","device":"hv","ch":300,"on":true}"#,
                "out of range",
            ),
            // hmp2020 = 1..=2。
            (
                br#"{"op":"power","device":"lv","ch":0,"on":true}"#,
                "out of range",
            ),
            (
                br#"{"op":"power","device":"lv","ch":3,"on":false}"#,
                "out of range",
            ),
        ];
        for (request, reason) in cases {
            let reply = handle_power_request(request, &targets, SHORT);
            let text = String::from_utf8_lossy(request);
            assert_eq!(reply["ok"], json!(false), "{text}");
            assert_eq!(reply["before"], json!(null), "{text}");
            let error = reply["error"].as_str().unwrap();
            assert!(error.contains(reason), "{text}: error = {error:?}");
        }
        assert!(hv_rx.try_recv().is_err(), "hv に何も入っていない");
        assert!(lv_rx.try_recv().is_err(), "lv に何も入っていない");
        assert!(gas_rx.try_recv().is_err(), "gas に何も入っていない");
    }

    /// 範囲の両端(hivolta 0 / 7、hmp2020 1 / 2)は通ってキューに入る。
    #[test]
    fn the_channel_range_ends_are_accepted() {
        let (hv, hv_rx) = target("hv", "hivolta");
        let (lv, lv_rx) = target("lv", "hmp2020");
        let targets = [hv, lv];
        for (request, rx, ch) in [
            (
                br#"{"op":"power","device":"hv","ch":0,"on":true}"#,
                &hv_rx,
                0u8,
            ),
            (
                br#"{"op":"power","device":"hv","ch":7,"on":true}"#,
                &hv_rx,
                7,
            ),
            (
                br#"{"op":"power","device":"lv","ch":1,"on":true}"#,
                &lv_rx,
                1,
            ),
            (
                br#"{"op":"power","device":"lv","ch":2,"on":true}"#,
                &lv_rx,
                2,
            ),
        ] {
            // 誰も答えないので timeout で返る —— キューに入ったことだけを見る。
            handle_power_request(request, &targets, SHORT);
            assert_eq!(rx.try_recv().unwrap().ch, ch);
        }
    }

    /// Fable 2: キューの深さは 1。先客が残っていれば待たずに `device busy`。
    #[test]
    fn a_full_queue_is_answered_device_busy_at_once() {
        let (hv, _hv_rx) = target("hv", "hivolta");
        let (stale, _stale_rx) = mpsc::sync_channel(1);
        hv.commands
            .try_send(PowerCommand {
                ch: 1,
                on: true,
                deadline: Instant::now() + Duration::from_secs(5),
                reply: stale,
            })
            .unwrap();

        let started = Instant::now();
        let reply = handle_power_request(
            br#"{"op":"power","device":"hv","ch":2,"on":false}"#,
            &[hv],
            Duration::from_secs(5),
        );
        assert_eq!(
            reply,
            json!({"ok": false, "error": "device busy", "before": null})
        );
        assert!(started.elapsed() < Duration::from_secs(1), "待たない");
    }

    /// §1: 上限を超えたら timeout。文言は「状態不明 — 表で確認」(レーン B レビュー: controller は
    /// 502 でそのまま返し、UI もそのまま出す)。Fable 5: 後から届いた結果は返し先が無い(= warn する側)。
    #[test]
    fn an_unanswered_command_times_out_and_its_late_result_has_nobody_to_go_to() {
        let (hv, hv_rx) = target("hv", "hivolta");
        let reply = handle_power_request(
            br#"{"op":"power","device":"hv","ch":4,"on":true}"#,
            &[hv],
            SHORT,
        );
        assert_eq!(
            reply,
            json!({"ok": false,
                   "error": "timeout — state unknown — check the table",
                   "before": null})
        );

        // device_loop 役が遅れて実行を終えた: 返し先はもう居ない → false(warn の経路)。
        let command = hv_rx.try_recv().unwrap();
        let late = json!({"ok": true, "after": {"status": 1, "flags": ["ON"]}});
        assert!(!deliver_power_reply(command.reply, late, "hv", 4, true));
    }

    /// device_loop が居ない(スレッドが終わった)なら即そう返す。
    #[test]
    fn a_device_without_its_thread_is_reported_not_running() {
        let (hv, hv_rx) = target("hv", "hivolta");
        drop(hv_rx);
        let reply = handle_power_request(
            br#"{"op":"power","device":"hv","ch":4,"on":true}"#,
            &[hv],
            Duration::from_secs(5),
        );
        assert_eq!(reply["ok"], json!(false));
        assert!(reply["error"].as_str().unwrap().contains("not running"));
    }

    /// 応答の形(発注書 §1 の契約): 成功 / 機器の拒否(before あり)/ 切り替わらない(before と
    /// 読めた after、レビュー 2026-10-12)/ before 前の失敗(null)。
    #[test]
    fn the_reply_shapes_follow_the_contract() {
        let before = json!({"status": 1, "flags": ["ON"]});
        let after = json!({"status": 0, "flags": []});
        assert_eq!(
            power_reply(
                "hv",
                3,
                false,
                Ok(PowerOutcome {
                    before: before.clone(),
                    result: Ok(after.clone()),
                })
            ),
            json!({"ok": true, "device": "hv", "ch": 3, "on": false,
                   "before": before, "after": after})
        );
        assert_eq!(
            power_reply(
                "hv",
                3,
                true,
                Ok(PowerOutcome {
                    before: json!({"status": 4096, "flags": ["DIS"]}),
                    result: Err(PowerRefused {
                        error: "front switch is OFF (DIS)".to_string(),
                        after: None,
                    }),
                })
            ),
            json!({"ok": false, "error": "front switch is OFF (DIS)",
                   "before": {"status": 4096, "flags": ["DIS"]}})
        );
        assert_eq!(
            power_reply(
                "lv",
                2,
                true,
                Ok(PowerOutcome {
                    before: json!({"outp": {"1": 1, "2": 0}}),
                    result: Err(PowerRefused {
                        error: "channel did not switch (outp=0)".to_string(),
                        after: Some(json!({"outp": {"1": 1, "2": 0}})),
                    }),
                })
            ),
            json!({"ok": false, "error": "channel did not switch (outp=0)",
                   "before": {"outp": {"1": 1, "2": 0}},
                   "after": {"outp": {"1": 1, "2": 0}}})
        );
        assert_eq!(
            power_reply("lv", 2, true, Err(DriverError::Timeout)),
            json!({"ok": false, "error": "device did not answer in time", "before": null})
        );
    }

    // -----------------------------------------------------------------
    // TODO/124: device_loop がコマンドを受ける(Fable 3 / 4)
    // -----------------------------------------------------------------

    /// poll と set_power の出入りを記録する偽ドライバ(どちらも `work` だけ時間を食う)。
    struct Recorder {
        log: Arc<std::sync::Mutex<Vec<&'static str>>>,
        work: Duration,
    }

    impl Driver for Recorder {
        fn ident(&mut self) -> Result<Vec<Ident>, DriverError> {
            Ok(Vec::new())
        }
        fn poll(&mut self) -> Result<Vec<Sample>, DriverError> {
            self.log.lock().unwrap().push("poll>");
            std::thread::sleep(self.work);
            self.log.lock().unwrap().push("poll<");
            Ok(vec![sample(1.0, 0)])
        }
        fn set_power(&mut self, _node: u8, on: bool) -> Result<PowerOutcome, DriverError> {
            self.log.lock().unwrap().push("power>");
            std::thread::sleep(self.work);
            self.log.lock().unwrap().push("power<");
            Ok(PowerOutcome {
                before: json!({"outp": {"1": 0, "2": 0}}),
                result: Ok(json!({"outp": {"1": 0, "2": u8::from(on)}})),
            })
        }
    }

    /// `Recorder` を 1 セッション回すスレッド。戻り値 = (イベント rx, コマンド tx, 停止, join, ログ)。
    #[allow(clippy::type_complexity)]
    fn spawn_session(
        poll_interval: Duration,
        work: Duration,
    ) -> (
        Receiver<DeviceEvent>,
        SyncSender<PowerCommand>,
        Arc<AtomicBool>,
        std::thread::JoinHandle<bool>,
        Arc<std::sync::Mutex<Vec<&'static str>>>,
    ) {
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let driver = Recorder {
            log: Arc::clone(&log),
            work,
        };
        let (tx, rx) = mpsc::sync_channel(EVENT_QUEUE);
        let (cmd_tx, cmd_rx) = mpsc::sync_channel(POWER_QUEUE_DEPTH);
        let stop = Arc::new(AtomicBool::new(false));
        let handle = std::thread::spawn({
            let stop = Arc::clone(&stop);
            move || {
                run_one_session(
                    Box::new(driver),
                    &device("lv", Vec::new()),
                    poll_interval,
                    &tx,
                    &Arc::new(Counters::default()),
                    &stop,
                    None,
                    &cmd_rx,
                )
            }
        });
        (rx, cmd_tx, stop, handle, log)
    }

    /// 次の `Samples`(= poll 1 回)を待つ。
    fn next_poll(rx: &Receiver<DeviceEvent>, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match rx.recv_timeout(left) {
                Ok(DeviceEvent::Samples(_)) => return true,
                Ok(DeviceEvent::Ident(_)) => continue,
                Err(_) => return false,
            }
        }
        false
    }

    fn send_command(
        commands: &SyncSender<PowerCommand>,
        ch: u8,
        on: bool,
    ) -> Receiver<serde_json::Value> {
        let (reply, reply_rx) = mpsc::sync_channel(1);
        let deadline = Instant::now() + Duration::from_secs(5);
        commands
            .try_send(PowerCommand {
                ch,
                on,
                deadline,
                reply,
            })
            .unwrap();
        reply_rx
    }

    /// ident と poll(全 ch 一括)に答える偽 DT1415ET(STATUS は内部の値 = 全 ch 0)。
    fn fake_dt1415et() -> crate::slowctl::fake_hv::FakeHv {
        use crate::slowctl::hivolta::{mon_bd, mon_ch, BdPar, ChPar, ALL_CH};
        let mut fake = crate::slowctl::fake_hv::FakeHv::new();
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
        fake
    }

    /// 送った行を全部記録し、poll の 1 往復目(全 ch 一括の VMON)で `stall` だけ固まる口
    /// (長い poll = 応答の遅い機器の体)。
    struct SlowWire {
        inner: crate::slowctl::fake_hv::FakeHv,
        sent: Arc<std::sync::Mutex<Vec<u8>>>,
        polling: Arc<AtomicBool>,
        stall: Duration,
    }

    impl std::io::Read for SlowWire {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.inner.read(buf)
        }
    }

    impl std::io::Write for SlowWire {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.sent.lock().unwrap().extend_from_slice(buf);
            if buf.starts_with(b"$CMD:MON,CH:8,PAR:VMON") {
                self.polling.store(true, Ordering::Relaxed);
                std::thread::sleep(self.stall);
            }
            self.inner.write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.inner.flush()
        }
    }

    /// レビュー 2026-10-12: 長い poll の最中に積まれたコマンドは、poll が明けた時点で期限
    /// (REP が待つ上限と同じ)を過ぎていれば**実行しない** —— 機器には SET が 1 本も届かず、
    /// 返し先にも何も返らない(REP は既に timeout を返している)。
    #[test]
    fn a_command_that_expired_during_a_long_poll_is_dropped_without_sending_set() {
        use crate::slowctl::hivolta_driver::HiVoltaDriver;
        let sent = Arc::new(std::sync::Mutex::new(Vec::new()));
        let polling = Arc::new(AtomicBool::new(false));
        let wire = SlowWire {
            inner: fake_dt1415et(),
            sent: Arc::clone(&sent),
            polling: Arc::clone(&polling),
            stall: Duration::from_millis(400),
        };
        let driver = HiVoltaDriver::new("hv", wire, String::new());
        let (tx, rx) = mpsc::sync_channel(EVENT_QUEUE);
        let (cmd_tx, cmd_rx) = mpsc::sync_channel(POWER_QUEUE_DEPTH);
        let stop = Arc::new(AtomicBool::new(false));
        let handle = std::thread::spawn({
            let stop = Arc::clone(&stop);
            move || {
                run_one_session(
                    Box::new(driver),
                    &device("hv", Vec::new()),
                    Duration::from_secs(10),
                    &tx,
                    &Arc::new(Counters::default()),
                    &stop,
                    None,
                    &cmd_rx,
                )
            }
        });

        // poll が固まっている最中に、期限 50 ms のコマンドを積む(poll は 400 ms 固まる)。
        let deadline = Instant::now() + Duration::from_secs(5);
        while !polling.load(Ordering::Relaxed) {
            assert!(Instant::now() < deadline, "poll が始まらない");
            std::thread::sleep(Duration::from_millis(5));
        }
        let (reply, reply_rx) = mpsc::sync_channel(1);
        cmd_tx
            .try_send(PowerCommand {
                ch: 3,
                on: true,
                deadline: Instant::now() + Duration::from_millis(50),
                reply,
            })
            .unwrap();

        // poll は最後まで終わる(サンプルが出る)。コマンドは捨てられ、返し先は何も受けない。
        assert!(next_poll(&rx, Duration::from_secs(5)), "poll が明ける");
        assert!(matches!(
            reply_rx.recv_timeout(Duration::from_secs(5)),
            Err(RecvTimeoutError::Disconnected)
        ));

        stop.store(true, Ordering::Relaxed);
        handle.join().unwrap();
        let wire_text = String::from_utf8(sent.lock().unwrap().clone()).unwrap();
        assert!(
            wire_text.contains("$CMD:MON,CH:8,PAR:VMON"),
            "poll は送っている"
        );
        assert!(
            !wire_text.contains("CMD:SET"),
            "SET は 1 本も送らない: {wire_text:?}"
        );
        assert!(
            !wire_text.contains("CH:3,PAR:STATUS"),
            "before の読み出しにも入らない"
        );
    }

    /// Fable 3 / 4: poll 周期(10 s)を待たずにコマンドを実行し、実行したらすぐ poll する。
    #[test]
    fn a_command_runs_without_waiting_for_the_poll_period_and_a_poll_follows_at_once() {
        let (rx, commands, stop, handle, log) =
            spawn_session(Duration::from_secs(10), Duration::ZERO);
        assert!(next_poll(&rx, Duration::from_secs(5)), "最初の poll");

        let started = Instant::now();
        let reply = send_command(&commands, 2, true)
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "poll 周期 10 s を待っていない: {:?}",
            started.elapsed()
        );
        assert_eq!(reply["ok"], json!(true));
        assert_eq!(reply["device"], json!("lv"));
        assert_eq!(reply["after"], json!({"outp": {"1": 0, "2": 1}}));

        assert!(
            next_poll(&rx, Duration::from_secs(2)),
            "コマンドの直後に poll(残りの sleep を捨てる)"
        );
        assert_eq!(
            *log.lock().unwrap(),
            ["poll>", "poll<", "power>", "power<", "poll>", "poll<"]
        );

        stop.store(true, Ordering::Relaxed);
        assert!(handle.join().unwrap(), "停止で抜ける");
    }

    /// poll とコマンドが交互に来ても**直列**(出入りが入れ子にならない)。
    #[test]
    fn polls_and_commands_never_overlap() {
        let (rx, commands, stop, handle, log) =
            spawn_session(Duration::from_millis(20), Duration::from_millis(5));
        assert!(next_poll(&rx, Duration::from_secs(5)));
        for i in 0..5u8 {
            let reply = send_command(&commands, 1 + i % 2, i % 2 == 0)
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            assert_eq!(reply["ok"], json!(true));
            std::thread::sleep(Duration::from_millis(7));
        }
        stop.store(true, Ordering::Relaxed);
        handle.join().unwrap();

        let log = log.lock().unwrap();
        assert_eq!(log.iter().filter(|e| **e == "power>").count(), 5);
        for pair in log.chunks(2) {
            assert!(
                matches!(pair, ["poll>", "poll<"] | ["power>", "power<"]),
                "入れ子になった: {log:?}"
            );
        }
    }

    /// Fable 3: backoff 中(開けない機器)のコマンドは、backoff の明けを待たずに即
    /// `device not connected`。
    #[test]
    fn a_command_during_the_backoff_is_answered_device_not_connected_at_once() {
        let mut config = device("lv", Vec::new());
        config.port = Some("/nonexistent/tpcdaq-124/script.txt".to_string());
        let (tx, _rx) = mpsc::sync_channel(EVENT_QUEUE);
        let (cmd_tx, cmd_rx) = mpsc::sync_channel(POWER_QUEUE_DEPTH);
        let stop = Arc::new(AtomicBool::new(false));
        let handle = std::thread::spawn({
            let stop = Arc::clone(&stop);
            move || {
                device_loop(
                    config,
                    Duration::from_secs(1),
                    tx,
                    Arc::new(Counters::default()),
                    stop,
                    None,
                    cmd_rx,
                )
            }
        });

        let started = Instant::now();
        let reply = send_command(&cmd_tx, 1, true)
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "backoff(1 s〜)の明けを待っていない: {:?}",
            started.elapsed()
        );
        assert_eq!(
            reply,
            json!({"ok": false, "error": "device not connected", "before": null})
        );

        stop.store(true, Ordering::Relaxed);
        handle.join().unwrap();
    }

    /// REP スレッドは**どの要求にも 1 通**返し(壊れた要求も)、停止合図で抜ける。
    #[test]
    fn the_rep_thread_answers_every_request_once_and_stops_on_the_flag() {
        let context = zmq::Context::new();
        let port = {
            let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = probe.local_addr().unwrap().port();
            drop(probe);
            port
        };
        let endpoint = format!("tcp://127.0.0.1:{port}");
        let socket = bind_cmd_rep(&context, &endpoint).unwrap();
        let (hv, hv_rx) = target("hv", "hivolta");
        drop(hv_rx); // device_loop は居ない
        let stop = Arc::new(AtomicBool::new(false));
        let handle = std::thread::spawn({
            let context = context.clone();
            let stop = Arc::clone(&stop);
            move || cmd_loop(&context, &endpoint, socket, &[hv], &stop)
        });

        let req = context.socket(zmq::REQ).unwrap();
        req.set_rcvtimeo(5_000).unwrap();
        req.set_linger(0).unwrap();
        req.connect(&format!("tcp://127.0.0.1:{port}")).unwrap();
        for (request, reason) in [
            (&b"{garbage"[..], "bad request"),
            (
                br#"{"op":"power","device":"hv","ch":1,"on":false}"#,
                "not running",
            ),
        ] {
            req.send(request, 0).unwrap();
            let reply: serde_json::Value =
                serde_json::from_slice(&req.recv_bytes(0).unwrap()).unwrap();
            assert_eq!(reply["ok"], json!(false));
            assert!(reply["error"].as_str().unwrap().contains(reason), "{reply}");
        }

        stop.store(true, Ordering::Relaxed);
        let started = Instant::now();
        handle.join().unwrap();
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
