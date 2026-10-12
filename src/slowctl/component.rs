//! slowctl コンポーネント本体(TODO/079 §2.6-4、SPEC §1.1 / §2.3 / §3.1 / §9.2)。
//!
//! **監視・記録・アラームのみ**(SPEC §11 の安全境界)。作動 API は無い。
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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;
use tracing::{error, info, warn};

use crate::config::{Config, InfluxConfig, SlowDeviceConfig, SLOWCTL_SOURCE_ID};
use crate::logbook::{LogbookRecord, SLOW_ACTOR};

use super::bronkhorst::open_serial;
use super::influx::{InfluxCounters, InfluxParams, InfluxSink, InfluxWriter};
use super::{
    now_ms, AlarmEvent, AlarmRule, Driver, DriverError, Ident, LogPolicy, Sample, SlowCounters,
    SlowPayload, SLOW_KIND,
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
            DriverError::Protocol(_) => self.parse_err.fetch_add(1, Ordering::Relaxed),
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

/// 1 デバイスぶんのブロッキングループ(専用 std スレッド)。
///
/// `influx` があれば、poll ごとのサンプルを InfluxDB writer にも渡す(TODO/122)。渡し方は
/// `try_send` だけ —— **読み出しは決して止めない**(満杯なら落として数える)。
fn device_loop(
    device: SlowDeviceConfig,
    poll_interval: Duration,
    tx: SyncSender<DeviceEvent>,
    counters: Arc<Counters>,
    stop: Arc<AtomicBool>,
    influx: Option<InfluxSink>,
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
                sleep_until_stop(backoff, &stop);
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
        ) {
            return; // 停止合図(またはチャネル切断)
        }
        sleep_until_stop(backoff, &stop);
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

/// 1 本のリンクで ident → poll ループを回す。戻り値 `true` = もう開き直さない(停止)。
fn run_one_session(
    mut driver: Box<dyn Driver + Send>,
    device: &SlowDeviceConfig,
    poll_interval: Duration,
    tx: &SyncSender<DeviceEvent>,
    counters: &Arc<Counters>,
    stop: &Arc<AtomicBool>,
    influx: Option<&InfluxSink>,
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
        sleep_until_stop(poll_interval.saturating_sub(started.elapsed()), stop);
    }
    true
}

/// 停止合図を見ながら寝る(SIGINT からの応答が `duration` ぶん遅れないように刻む)。
fn sleep_until_stop(duration: Duration, stop: &Arc<AtomicBool>) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        std::thread::sleep(DRAIN_TIMEOUT.min(deadline.saturating_duration_since(Instant::now())));
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

    let (tx, rx) = mpsc::sync_channel(EVENT_QUEUE);
    let mut counters = Vec::with_capacity(params.devices.len());
    let mut threads = Vec::with_capacity(params.devices.len());
    for device in &params.devices {
        let device_counters = Arc::new(Counters::default());
        counters.push(Arc::clone(&device_counters));
        let handle = std::thread::Builder::new()
            .name(format!("slowctl-{}", device.name))
            .spawn({
                let device = device.clone();
                let tx = tx.clone();
                let stop = Arc::clone(&stop);
                let poll_interval = params.device_poll_interval(&device);
                let influx = influx.as_ref().map(InfluxWriter::sink);
                move || device_loop(device, poll_interval, tx, device_counters, stop, influx)
            })
            .map_err(|source| SlowctlError::Spawn {
                device: device.name.clone(),
                source,
            })?;
        threads.push(handle);
    }
    drop(tx); // 本体は受けるだけ(全スレッドが終われば recv が切れる)

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
}
