//! 設定モジュール(SPEC §3)。
//!
//! TOML 設定ファイルを読み、既定ポート・source_id 規約(SPEC §3.2)を適用したうえで
//! 検証し、[`Config`] を返す。パース・検証のどちらが失敗しても `Err` を返す
//! (半端な既定値のまま走らない — SPEC §3.2)。

use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------
// 既定ポート・ID 規約(SPEC §3.2)
// ---------------------------------------------------------------------

/// CoBo データ着信ポートの基準値。実際の既定は `COBO_LISTEN_PORT_BASE + cobo_id`(SPEC §3.2)。
pub const COBO_LISTEN_PORT_BASE: u32 = 46005;

/// monitor WS の既定 listen アドレス(SPEC §3.2)。
pub const DEFAULT_MONITOR_WS_LISTEN: &str = "0.0.0.0:9000";

/// controller REST の既定 listen アドレス(SPEC §3.2)。
pub const DEFAULT_CONTROLLER_REST_LISTEN: &str = "0.0.0.0:8080";

/// decoder の Batch source_id(SPEC §3.2 の source_id 表)。
pub const DECODER_SOURCE_ID: u32 = 100;

/// psu の Batch source_id(SPEC §3.2 の source_id 表、P6)。
pub const PSU_SOURCE_ID: u32 = 200;

/// コンポーネント REP ポートの receiver 区画(SPEC §3.2: `receiver k = 47110 + k`)。
pub const RECEIVER_COMMAND_PORT_BASE: u32 = 47110;

/// receiver → graw-writer の既定 PUSH 接続先(SPEC §3.2 の `graw-writer PULL bind = 47001`)。
pub const DEFAULT_GRAW_WRITER_ENDPOINT: &str = "tcp://127.0.0.1:47001";

/// receiver → decoder の既定 PUSH 接続先(SPEC §3.2 の `decoder PULL bind = 47002`)。
pub const DEFAULT_DECODER_ENDPOINT: &str = "tcp://127.0.0.1:47002";

/// graw-writer 自身の PULL bind の既定アドレス(SPEC §3.2「graw-writer PULL bind = tcp://*:47001」)。
/// receiver 側の接続先定数 [`DEFAULT_GRAW_WRITER_ENDPOINT`] とはポートは同じだが bind/connect の
/// アドレス表記が違う(bind は `*`、connect は `127.0.0.1`)ので別定数にしてある。
pub const DEFAULT_GRAW_WRITER_PULL_BIND: &str = "tcp://*:47001";

/// graw-writer のファイルローテーション閾値の既定(SPEC §7「既定 1 GiB」)。
pub const DEFAULT_GRAW_WRITER_MAX_FILE_BYTES: u64 = 1024 * 1024 * 1024;

/// graw-writer の flush 周期の既定(ミリ秒、SPEC §7「flush は 1 秒毎」)。
pub const DEFAULT_GRAW_WRITER_FLUSH_INTERVAL_MS: u64 = 1000;

/// graw-writer のコマンド REP bind アドレス(SPEC §3.2「コンポーネント REP = 47100 + 連番」)。
/// receiver 群が `47110+k` を予約しているので、単一コンポーネントである graw-writer は
/// その手前の連番の先頭 `47100` を使う。
pub const GRAW_WRITER_COMMAND_LISTEN: &str = "tcp://*:47100";

/// decoder 自身の PULL bind の既定アドレス(SPEC §3.2「decoder PULL bind = tcp://*:47002」)。
/// receiver 側の接続先定数 [`DEFAULT_DECODER_ENDPOINT`] とはポートは同じだが bind/connect の
/// アドレス表記が違う(bind は `*`、connect は `127.0.0.1`)ので別定数にしてある。
pub const DEFAULT_DECODER_PULL_BIND: &str = "tcp://*:47002";

/// decoder → root-sink の既定 PUSH 接続先(SPEC §3.2 の `root-sink PULL bind = 47003`)。
pub const DEFAULT_ROOT_SINK_ENDPOINT: &str = "tcp://127.0.0.1:47003";

/// decoder のコマンド REP bind アドレス(SPEC §3.2 v1.2「decoder = 47101」)。
pub const DECODER_COMMAND_LISTEN: &str = "tcp://*:47101";

/// decoder の PUSH 送信タイムアウトの既定(ミリ秒、TODO/009 の停止設計)。
///
/// 通常運転ではタイムアウトしても諦めずに再試行する(ロスレス = 下流の背圧で待つ)。
/// **Reset コマンド処理中に限り**、このタイムアウトで送出待ちを打ち切れる
/// (破棄は `eos_abandoned` / `batches_abandoned` として可視化する)。
pub const DEFAULT_DECODER_SEND_TIMEOUT_MS: i32 = 1000;

/// バッチを閉じるバイト数の既定値(SPEC §2.3「8 MiB 到達 or 10 ms 経過」)。
pub const DEFAULT_BATCH_MAX_BYTES: usize = 8 * 1024 * 1024;

/// バッチを閉じる経過時間の既定値(ミリ秒、SPEC §2.3)。
pub const DEFAULT_BATCH_MAX_MS: u64 = 10;

/// receiver 内部キュー(drain → 送信タスク)の既定段数(フレーム数、SPEC §1.4-2)。
///
/// 目標 100 Hz(= 1 フレーム/トリガ/CoBo)で 2 秒分 = 200 フレームが下限。512 段は
/// 約 5 秒分の余裕にあたる(実 .graw の 1 フレーム 278,784 B で約 143 MB が上限)。
pub const DEFAULT_QUEUE_FRAMES: usize = 512;

/// アイドル時 Heartbeat の既定周期(ミリ秒、SPEC §2.2「アイドル時 1 Hz」)。
pub const DEFAULT_HEARTBEAT_MS: u64 = 1000;

// receiver の source_id は `cobo_id` そのもの(SPEC §3.2)なので、専用の定数や
// フィールドは持たない。`CoboConfig::id` をそのまま Batch の source_id として使う。

// ---------------------------------------------------------------------
// エラー
// ---------------------------------------------------------------------

/// 設定の読込・検証エラー。パース/検証いずれの失敗も起動失敗として扱う(SPEC §3.2)。
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config file {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("TOML parse error: {0}")]
    Parse(#[from] toml::de::Error),

    #[error("duplicate cobo id: {0}")]
    DuplicateCoboId(u32),

    #[error("duplicate cobo listen address: {0}")]
    DuplicateListen(String),

    #[error("geometry file not found: {}", .0.display())]
    GeometryNotFound(PathBuf),

    #[error("invalid [receiver] setting: {0}")]
    InvalidReceiver(String),
}

// ---------------------------------------------------------------------
// 解決・検証済み設定(公開 API)
// ---------------------------------------------------------------------

/// 解決・検証済みの設定全体。既定値はすべて適用済みで、そのまま各コンポーネントに渡せる。
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub system: SystemConfig,
    pub cobo: Vec<CoboConfig>,
    pub receiver: ReceiverConfig,
    pub graw_writer: GrawWriterConfig,
    pub decoder: DecoderConfig,
    pub root_sink: RootSinkConfig,
    pub monitor: MonitorConfig,
    pub controller: ControllerConfig,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemConfig {
    pub experiment: String,
    pub output_root: PathBuf,
    pub geometry: PathBuf,
}

/// 1 CoBo 分の設定(`[[cobo]]`)。`listen` を省略すると
/// `COBO_LISTEN_PORT_BASE + id`(SPEC §3.2)が既定として入る。
#[derive(Debug, Clone, PartialEq)]
pub struct CoboConfig {
    pub id: u32,
    pub listen: String,
    pub data_sender_id: String,
}

/// receiver 固有の設定(`[receiver]`)。セクションごと省略可で、すべて既定値が入る。
///
/// 全 CoBo で共通の値だけを置く(CoBo 毎に違うのは `[[cobo]]` 側の `listen` / `id`)。
#[derive(Debug, Clone, PartialEq)]
pub struct ReceiverConfig {
    /// バッチを閉じるバイト数(SPEC §2.3)。
    pub batch_max_bytes: usize,
    /// バッチを閉じる経過時間(ミリ秒、SPEC §2.3)。
    pub batch_max_ms: u64,
    /// drain → 送信タスク間の有界キュー段数(SPEC §1.4-1/2)。
    pub queue_frames: usize,
    /// アイドル時 Heartbeat の周期(ミリ秒、SPEC §2.2)。
    pub heartbeat_ms: u64,
    /// PUSH ソケットの HWM(SPEC §1.4-2。無制限 = 0 は禁止)。
    pub hwm: i32,
    /// graw-writer の PULL bind への接続先。
    pub graw_writer_endpoint: String,
    /// decoder の PULL bind への接続先。
    pub decoder_endpoint: String,
}

/// graw-writer 固有の設定(`[graw_writer]`)。省略可、すべて既定値が入る(SPEC §7)。
#[derive(Debug, Clone, PartialEq)]
pub struct GrawWriterConfig {
    /// PULL bind アドレス(SPEC §3.2)。
    pub pull_bind: String,
    /// ファイルローテーションの閾値(バイト、SPEC §7 既定 1 GiB)。
    pub max_file_bytes: u64,
    /// flush 周期(ミリ秒、SPEC §7 既定 1000)。
    pub flush_interval_ms: u64,
}

/// decoder 固有の設定(`[decoder]`)。`workers` 以外は省略可で既定値が入る
/// (SPEC §2.3 / §3.2、009 で追記)。
#[derive(Debug, Clone, PartialEq)]
pub struct DecoderConfig {
    /// 内部ワーカー数。009 時点では**受理するだけで未使用**(> 1 なら info ログ)。
    /// 並列化は実測してから決める(SPEC §13 の流儀)。
    pub workers: u32,
    /// PULL bind アドレス(SPEC §3.2)。
    pub pull_bind: String,
    /// root-sink の PULL bind への接続先(SPEC §3.2)。
    pub push_connect: String,
    /// 出力バッチを閉じるバイト数(SPEC §2.3)。
    pub batch_max_bytes: usize,
    /// 出力バッチを閉じる経過時間(ミリ秒、SPEC §2.3)。
    pub batch_max_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RootSinkConfig {
    pub snapshot_hz: f64,
    pub event_publish_hz: f64,
    pub build_timeout_ms: u64,
}

/// `ws_listen` を省略すると `DEFAULT_MONITOR_WS_LISTEN`(SPEC §3.2)が既定として入る。
#[derive(Debug, Clone, PartialEq)]
pub struct MonitorConfig {
    pub ws_listen: String,
}

/// `rest_listen` を省略すると `DEFAULT_CONTROLLER_REST_LISTEN`(SPEC §3.2)が既定として入る。
#[derive(Debug, Clone, PartialEq)]
pub struct ControllerConfig {
    pub rest_listen: String,
    pub passphrase: String,
    pub ecc_proxy: String,
    pub config_id: String,
}

// ---------------------------------------------------------------------
// TOML そのままの中間表現(既定値解決前)
// ---------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    system: SystemConfig,
    cobo: Vec<RawCobo>,
    #[serde(default)]
    receiver: RawReceiver,
    #[serde(default)]
    graw_writer: RawGrawWriter,
    decoder: RawDecoder,
    root_sink: RootSinkConfig,
    monitor: RawMonitor,
    controller: RawController,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCobo {
    id: u32,
    #[serde(default)]
    listen: Option<String>,
    data_sender_id: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct RawReceiver {
    batch_max_bytes: Option<usize>,
    batch_max_ms: Option<u64>,
    queue_frames: Option<usize>,
    heartbeat_ms: Option<u64>,
    hwm: Option<i32>,
    graw_writer_endpoint: Option<String>,
    decoder_endpoint: Option<String>,
}

/// `[decoder]` の TOML そのまま(009)。`workers` は 001 からの既存キーなので必須のまま、
/// 009 で足したキーだけが省略可。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDecoder {
    workers: u32,
    #[serde(default)]
    pull_bind: Option<String>,
    #[serde(default)]
    push_connect: Option<String>,
    #[serde(default)]
    batch_max_bytes: Option<usize>,
    #[serde(default)]
    batch_max_ms: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct RawGrawWriter {
    pull_bind: Option<String>,
    max_file_bytes: Option<u64>,
    flush_interval_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMonitor {
    #[serde(default)]
    ws_listen: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawController {
    #[serde(default)]
    rest_listen: Option<String>,
    passphrase: String,
    ecc_proxy: String,
    config_id: String,
}

// ---------------------------------------------------------------------
// 読込・解決・検証
// ---------------------------------------------------------------------

/// TOML 文字列から設定を読み、既定値を適用し、検証する。
///
/// パース・検証いずれかが失敗すれば `Err`(半端な既定値のまま走らない — SPEC §3.2)。
pub fn parse(toml_str: &str) -> Result<Config, ConfigError> {
    let raw: RawConfig = toml::from_str(toml_str)?;
    let config = resolve(raw);
    validate(&config)?;
    Ok(config)
}

/// ファイルパスから設定を読む(`parse` のファイル版)。
pub fn load(path: impl AsRef<Path>) -> Result<Config, ConfigError> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse(&text)
}

fn resolve(raw: RawConfig) -> Config {
    let cobo = raw
        .cobo
        .into_iter()
        .map(|c| {
            let listen = c
                .listen
                .unwrap_or_else(|| format!("0.0.0.0:{}", COBO_LISTEN_PORT_BASE + c.id));
            CoboConfig {
                id: c.id,
                listen,
                data_sender_id: c.data_sender_id,
            }
        })
        .collect();

    let receiver = ReceiverConfig {
        batch_max_bytes: raw
            .receiver
            .batch_max_bytes
            .unwrap_or(DEFAULT_BATCH_MAX_BYTES),
        batch_max_ms: raw.receiver.batch_max_ms.unwrap_or(DEFAULT_BATCH_MAX_MS),
        queue_frames: raw.receiver.queue_frames.unwrap_or(DEFAULT_QUEUE_FRAMES),
        heartbeat_ms: raw.receiver.heartbeat_ms.unwrap_or(DEFAULT_HEARTBEAT_MS),
        hwm: raw.receiver.hwm.unwrap_or(crate::zmq_helper::DEFAULT_HWM),
        graw_writer_endpoint: raw
            .receiver
            .graw_writer_endpoint
            .unwrap_or_else(|| DEFAULT_GRAW_WRITER_ENDPOINT.to_string()),
        decoder_endpoint: raw
            .receiver
            .decoder_endpoint
            .unwrap_or_else(|| DEFAULT_DECODER_ENDPOINT.to_string()),
    };

    let graw_writer = GrawWriterConfig {
        pull_bind: raw
            .graw_writer
            .pull_bind
            .unwrap_or_else(|| DEFAULT_GRAW_WRITER_PULL_BIND.to_string()),
        max_file_bytes: raw
            .graw_writer
            .max_file_bytes
            .unwrap_or(DEFAULT_GRAW_WRITER_MAX_FILE_BYTES),
        flush_interval_ms: raw
            .graw_writer
            .flush_interval_ms
            .unwrap_or(DEFAULT_GRAW_WRITER_FLUSH_INTERVAL_MS),
    };

    let decoder = DecoderConfig {
        workers: raw.decoder.workers,
        pull_bind: raw
            .decoder
            .pull_bind
            .unwrap_or_else(|| DEFAULT_DECODER_PULL_BIND.to_string()),
        push_connect: raw
            .decoder
            .push_connect
            .unwrap_or_else(|| DEFAULT_ROOT_SINK_ENDPOINT.to_string()),
        batch_max_bytes: raw
            .decoder
            .batch_max_bytes
            .unwrap_or(DEFAULT_BATCH_MAX_BYTES),
        batch_max_ms: raw.decoder.batch_max_ms.unwrap_or(DEFAULT_BATCH_MAX_MS),
    };

    let monitor = MonitorConfig {
        ws_listen: raw
            .monitor
            .ws_listen
            .unwrap_or_else(|| DEFAULT_MONITOR_WS_LISTEN.to_string()),
    };

    let controller = ControllerConfig {
        rest_listen: raw
            .controller
            .rest_listen
            .unwrap_or_else(|| DEFAULT_CONTROLLER_REST_LISTEN.to_string()),
        passphrase: raw.controller.passphrase,
        ecc_proxy: raw.controller.ecc_proxy,
        config_id: raw.controller.config_id,
    };

    Config {
        system: raw.system,
        cobo,
        receiver,
        graw_writer,
        decoder,
        root_sink: raw.root_sink,
        monitor,
        controller,
    }
}

fn validate(config: &Config) -> Result<(), ConfigError> {
    validate_cobo_ids(&config.cobo)?;
    validate_cobo_listen_unique(&config.cobo)?;
    validate_geometry_path(&config.system.geometry)?;
    validate_receiver(&config.receiver)?;
    Ok(())
}

/// `[receiver]` の数値が意味を成すことを確認する。
///
/// 0 段のキュー・0 バイトのバッチ・HWM 0(= 無制限、SPEC §1.4-2 で禁止)は、
/// そのまま起動すると「静かに全フレームを落とす」「メモリが無制限に伸びる」形の
/// 事故になるので、起動失敗にする。
fn validate_receiver(receiver: &ReceiverConfig) -> Result<(), ConfigError> {
    let positive = [
        ("batch_max_bytes", receiver.batch_max_bytes as u64),
        ("batch_max_ms", receiver.batch_max_ms),
        ("queue_frames", receiver.queue_frames as u64),
        ("heartbeat_ms", receiver.heartbeat_ms),
    ];
    for (field, value) in positive {
        if value == 0 {
            return Err(ConfigError::InvalidReceiver(format!(
                "{field} must be greater than 0"
            )));
        }
    }
    if receiver.hwm <= 0 {
        // SPEC §1.4-2: HWM=0 は無制限バッファ = メモリ暴走。
        return Err(ConfigError::InvalidReceiver(format!(
            "hwm must be greater than 0 (0 means unlimited buffering), got {}",
            receiver.hwm
        )));
    }
    Ok(())
}

fn validate_cobo_ids(cobos: &[CoboConfig]) -> Result<(), ConfigError> {
    let mut seen = HashSet::new();
    for c in cobos {
        if !seen.insert(c.id) {
            return Err(ConfigError::DuplicateCoboId(c.id));
        }
    }
    Ok(())
}

fn validate_cobo_listen_unique(cobos: &[CoboConfig]) -> Result<(), ConfigError> {
    let mut seen: HashSet<&str> = HashSet::new();
    for c in cobos {
        if !seen.insert(c.listen.as_str()) {
            return Err(ConfigError::DuplicateListen(c.listen.clone()));
        }
    }
    Ok(())
}

/// geometry パスの存在検証だけを切り出した関数。
///
/// こうしておくことで、ユニットテストは一時ディレクトリ + 実ファイルを使って
/// このチェックだけを直接呼べる(発注書 001 の指示どおり)。
fn validate_geometry_path(path: &Path) -> Result<(), ConfigError> {
    if path.exists() {
        Ok(())
    } else {
        Err(ConfigError::GeometryNotFound(path.to_path_buf()))
    }
}

// ---------------------------------------------------------------------
// テスト
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// テスト用に一意な一時ディレクトリを作り、その中にダミーの geometry ファイルを置く。
    /// 返すのはそのファイルへのパス(存在確認テストが読む「実在するファイル」)。
    fn make_temp_geometry_file() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("tpcdaq-config-test-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        let geometry = dir.join("geometry_mini_eTPC.dat");
        std::fs::write(&geometry, b"# dummy geometry fixture for config tests\n").unwrap();
        geometry
    }

    /// 006 で足した `[receiver]` のテスト用に、それ以外は最小構成の TOML を組む。
    /// `extra` にセクション断片(空文字列可)を差し込む。
    fn minimal_toml(geometry: &Path, extra: &str) -> String {
        format!(
            r#"
[system]
experiment = "mini_eTPC"
output_root = "/data/tpcdaq"
geometry = "{geometry}"

[[cobo]]
id = 0
listen = "0.0.0.0:46005"
data_sender_id = "CoBo[0]"
{extra}
[decoder]
workers = 4

[root_sink]
snapshot_hz = 1.0
event_publish_hz = 20.0
build_timeout_ms = 1000

[monitor]
ws_listen = "0.0.0.0:9000"

[controller]
rest_listen = "0.0.0.0:8080"
passphrase = "change-me"
ecc_proxy = "GetEcc:tcp -h 127.0.0.1 -p 46002"
config_id = "default"
"#,
            geometry = geometry.display()
        )
    }

    /// 009 で足した `[decoder]` キーのテスト用に、`[decoder]` セクションだけを差し替えた
    /// 最小構成の TOML を組む(`minimal_toml` は `[decoder]` を固定で埋め込むので、
    /// そちらの `extra` では `[decoder]` を上書きできない)。
    fn toml_with_decoder_section(geometry: &Path, decoder_section: &str) -> String {
        format!(
            r#"
[system]
experiment = "mini_eTPC"
output_root = "/data/tpcdaq"
geometry = "{geometry}"

[[cobo]]
id = 0
listen = "0.0.0.0:46005"
data_sender_id = "CoBo[0]"
{decoder_section}
[root_sink]
snapshot_hz = 1.0
event_publish_hz = 20.0
build_timeout_ms = 1000

[monitor]
ws_listen = "0.0.0.0:9000"

[controller]
rest_listen = "0.0.0.0:8080"
passphrase = "change-me"
ecc_proxy = "GetEcc:tcp -h 127.0.0.1 -p 46002"
config_id = "default"
"#,
            geometry = geometry.display()
        )
    }

    /// 浮動小数点は TOML リテラル → f64 の単純往復であり計算を挟まないので、
    /// 等値比較で十分(clippy::float_cmp を避けるため差分での比較にする)。
    fn assert_f64_eq(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-12,
            "expected {expected}, got {actual}"
        );
    }

    // --- SPEC §3.1 の mini TOML そのもの(1 CoBo) ---

    #[test]
    fn mini_1_cobo_toml_parses_with_expected_values() {
        let geometry = make_temp_geometry_file();
        let toml_str = format!(
            r#"
[system]
experiment = "mini_eTPC"
output_root = "/data/tpcdaq"
geometry = "{geometry}"

[[cobo]]
id = 0
listen = "0.0.0.0:46005"
data_sender_id = "CoBo[0]"

[decoder]
workers = 4

[root_sink]
snapshot_hz = 1.0
event_publish_hz = 20.0
build_timeout_ms = 1000

[monitor]
ws_listen = "0.0.0.0:9000"

[controller]
rest_listen = "0.0.0.0:8080"
passphrase = "change-me"
ecc_proxy = "GetEcc:tcp -h 127.0.0.1 -p 46002"
config_id = "default"
"#,
            geometry = geometry.display()
        );

        let config = parse(&toml_str).unwrap();

        assert_eq!(config.system.experiment, "mini_eTPC");
        assert_eq!(config.system.output_root, PathBuf::from("/data/tpcdaq"));
        assert_eq!(config.system.geometry, geometry);

        assert_eq!(config.cobo.len(), 1);
        assert_eq!(config.cobo[0].id, 0);
        assert_eq!(config.cobo[0].listen, "0.0.0.0:46005");
        assert_eq!(config.cobo[0].data_sender_id, "CoBo[0]");

        assert_eq!(config.decoder.workers, 4);

        assert_f64_eq(config.root_sink.snapshot_hz, 1.0);
        assert_f64_eq(config.root_sink.event_publish_hz, 20.0);
        assert_eq!(config.root_sink.build_timeout_ms, 1000);

        assert_eq!(config.monitor.ws_listen, "0.0.0.0:9000");

        assert_eq!(config.controller.rest_listen, "0.0.0.0:8080");
        assert_eq!(config.controller.passphrase, "change-me");
        assert_eq!(
            config.controller.ecc_proxy,
            "GetEcc:tcp -h 127.0.0.1 -p 46002"
        );
        assert_eq!(config.controller.config_id, "default");

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    // --- ELITPC 相当(2 CoBo、値は mini とすべて非対称にして取り違えを検出しやすくする) ---

    #[test]
    fn elitpc_2_cobo_toml_parses_with_expected_values() {
        let geometry = make_temp_geometry_file();
        let toml_str = format!(
            r#"
[system]
experiment = "ELITPC"
output_root = "/data/tpcdaq-elitpc"
geometry = "{geometry}"

[[cobo]]
id = 0
listen = "0.0.0.0:46005"
data_sender_id = "CoBo[0]"

[[cobo]]
id = 1
listen = "0.0.0.0:46006"
data_sender_id = "CoBo[1]"

[decoder]
workers = 8

[root_sink]
snapshot_hz = 2.0
event_publish_hz = 10.0
build_timeout_ms = 500

[monitor]
ws_listen = "0.0.0.0:9001"

[controller]
rest_listen = "0.0.0.0:8090"
passphrase = "elitpc-pass"
ecc_proxy = "GetEcc:tcp -h 127.0.0.1 -p 46002"
config_id = "elitpc"
"#,
            geometry = geometry.display()
        );

        let config = parse(&toml_str).unwrap();

        assert_eq!(config.cobo.len(), 2);
        assert_eq!(config.cobo[0].id, 0);
        assert_eq!(config.cobo[0].listen, "0.0.0.0:46005");
        assert_eq!(config.cobo[0].data_sender_id, "CoBo[0]");
        assert_eq!(config.cobo[1].id, 1);
        assert_eq!(config.cobo[1].listen, "0.0.0.0:46006");
        assert_eq!(config.cobo[1].data_sender_id, "CoBo[1]");

        assert_eq!(config.decoder.workers, 8);
        assert_f64_eq(config.root_sink.snapshot_hz, 2.0);
        assert_f64_eq(config.root_sink.event_publish_hz, 10.0);
        assert_eq!(config.root_sink.build_timeout_ms, 500);
        assert_eq!(config.monitor.ws_listen, "0.0.0.0:9001");
        assert_eq!(config.controller.rest_listen, "0.0.0.0:8090");
        assert_eq!(config.controller.config_id, "elitpc");

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    // --- 既定値(SPEC §3.2)の充足 ---

    #[test]
    fn cobo_listen_defaults_to_port_base_plus_id_when_omitted() {
        let geometry = make_temp_geometry_file();
        // id = 7(非対称な値。0/1 だと式を書かなくても偶然一致しうるため避ける)。
        // 46005 (COBO_LISTEN_PORT_BASE) + 7 = 46012 — SPEC §3.2 の式の手計算。
        let toml_str = format!(
            r#"
[system]
experiment = "mini_eTPC"
output_root = "/data/tpcdaq"
geometry = "{geometry}"

[[cobo]]
id = 7
data_sender_id = "CoBo[7]"

[decoder]
workers = 1

[root_sink]
snapshot_hz = 1.0
event_publish_hz = 20.0
build_timeout_ms = 1000

[monitor]
ws_listen = "0.0.0.0:9000"

[controller]
rest_listen = "0.0.0.0:8080"
passphrase = "change-me"
ecc_proxy = "GetEcc:tcp -h 127.0.0.1 -p 46002"
config_id = "default"
"#,
            geometry = geometry.display()
        );

        let config = parse(&toml_str).unwrap();

        assert_eq!(config.cobo[0].listen, "0.0.0.0:46012");

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    #[test]
    fn monitor_ws_listen_defaults_when_omitted() {
        let geometry = make_temp_geometry_file();
        let toml_str = format!(
            r#"
[system]
experiment = "mini_eTPC"
output_root = "/data/tpcdaq"
geometry = "{geometry}"

[[cobo]]
id = 0
listen = "0.0.0.0:46005"
data_sender_id = "CoBo[0]"

[decoder]
workers = 4

[root_sink]
snapshot_hz = 1.0
event_publish_hz = 20.0
build_timeout_ms = 1000

[monitor]

[controller]
rest_listen = "0.0.0.0:8080"
passphrase = "change-me"
ecc_proxy = "GetEcc:tcp -h 127.0.0.1 -p 46002"
config_id = "default"
"#,
            geometry = geometry.display()
        );

        let config = parse(&toml_str).unwrap();

        assert_eq!(config.monitor.ws_listen, DEFAULT_MONITOR_WS_LISTEN);

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    #[test]
    fn controller_rest_listen_defaults_when_omitted() {
        let geometry = make_temp_geometry_file();
        let toml_str = format!(
            r#"
[system]
experiment = "mini_eTPC"
output_root = "/data/tpcdaq"
geometry = "{geometry}"

[[cobo]]
id = 0
listen = "0.0.0.0:46005"
data_sender_id = "CoBo[0]"

[decoder]
workers = 4

[root_sink]
snapshot_hz = 1.0
event_publish_hz = 20.0
build_timeout_ms = 1000

[monitor]
ws_listen = "0.0.0.0:9000"

[controller]
passphrase = "change-me"
ecc_proxy = "GetEcc:tcp -h 127.0.0.1 -p 46002"
config_id = "default"
"#,
            geometry = geometry.display()
        );

        let config = parse(&toml_str).unwrap();

        assert_eq!(
            config.controller.rest_listen,
            DEFAULT_CONTROLLER_REST_LISTEN
        );

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    #[test]
    fn source_id_constants_match_spec_3_2() {
        // SPEC §3.2 source_id 表: receiver = cobo_id、decoder = 100、psu = 200。
        // receiver は CoboConfig::id をそのまま使うので専用定数はない。
        assert_eq!(DECODER_SOURCE_ID, 100);
        assert_eq!(PSU_SOURCE_ID, 200);
    }

    // --- 不正系 ---

    #[test]
    fn duplicate_cobo_id_is_err() {
        let geometry = make_temp_geometry_file();
        // 同一 id = 3 を持つ 2 ブロック(listen は異なる — id 重複だけを検出できることの確認)。
        let toml_str = format!(
            r#"
[system]
experiment = "mini_eTPC"
output_root = "/data/tpcdaq"
geometry = "{geometry}"

[[cobo]]
id = 3
listen = "0.0.0.0:46008"
data_sender_id = "CoBo[3]"

[[cobo]]
id = 3
listen = "0.0.0.0:46009"
data_sender_id = "CoBo[3b]"

[decoder]
workers = 4

[root_sink]
snapshot_hz = 1.0
event_publish_hz = 20.0
build_timeout_ms = 1000

[monitor]
ws_listen = "0.0.0.0:9000"

[controller]
rest_listen = "0.0.0.0:8080"
passphrase = "change-me"
ecc_proxy = "GetEcc:tcp -h 127.0.0.1 -p 46002"
config_id = "default"
"#,
            geometry = geometry.display()
        );

        let err = parse(&toml_str).unwrap_err();
        assert!(matches!(err, ConfigError::DuplicateCoboId(3)));

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    #[test]
    fn duplicate_cobo_listen_is_err() {
        let geometry = make_temp_geometry_file();
        // id は異なる(0/1)が listen アドレスが衝突する(実機のポート配線ミスを模す)。
        let toml_str = format!(
            r#"
[system]
experiment = "mini_eTPC"
output_root = "/data/tpcdaq"
geometry = "{geometry}"

[[cobo]]
id = 0
listen = "0.0.0.0:47100"
data_sender_id = "CoBo[0]"

[[cobo]]
id = 1
listen = "0.0.0.0:47100"
data_sender_id = "CoBo[1]"

[decoder]
workers = 4

[root_sink]
snapshot_hz = 1.0
event_publish_hz = 20.0
build_timeout_ms = 1000

[monitor]
ws_listen = "0.0.0.0:9000"

[controller]
rest_listen = "0.0.0.0:8080"
passphrase = "change-me"
ecc_proxy = "GetEcc:tcp -h 127.0.0.1 -p 46002"
config_id = "default"
"#,
            geometry = geometry.display()
        );

        let err = parse(&toml_str).unwrap_err();
        assert!(matches!(err, ConfigError::DuplicateListen(ref s) if s == "0.0.0.0:47100"));

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    #[test]
    fn unknown_field_in_system_is_err() {
        let geometry = make_temp_geometry_file();
        let toml_str = format!(
            r#"
[system]
experiment = "mini_eTPC"
output_root = "/data/tpcdaq"
geometry = "{geometry}"
typo_field = "oops"

[[cobo]]
id = 0
listen = "0.0.0.0:46005"
data_sender_id = "CoBo[0]"

[decoder]
workers = 4

[root_sink]
snapshot_hz = 1.0
event_publish_hz = 20.0
build_timeout_ms = 1000

[monitor]
ws_listen = "0.0.0.0:9000"

[controller]
rest_listen = "0.0.0.0:8080"
passphrase = "change-me"
ecc_proxy = "GetEcc:tcp -h 127.0.0.1 -p 46002"
config_id = "default"
"#,
            geometry = geometry.display()
        );

        let err = parse(&toml_str).unwrap_err();
        assert!(matches!(err, ConfigError::Parse(_)));

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    #[test]
    fn missing_required_key_experiment_is_err() {
        let geometry = make_temp_geometry_file();
        // [system] から `experiment` を欠落させる。
        let toml_str = format!(
            r#"
[system]
output_root = "/data/tpcdaq"
geometry = "{geometry}"

[[cobo]]
id = 0
listen = "0.0.0.0:46005"
data_sender_id = "CoBo[0]"

[decoder]
workers = 4

[root_sink]
snapshot_hz = 1.0
event_publish_hz = 20.0
build_timeout_ms = 1000

[monitor]
ws_listen = "0.0.0.0:9000"

[controller]
rest_listen = "0.0.0.0:8080"
passphrase = "change-me"
ecc_proxy = "GetEcc:tcp -h 127.0.0.1 -p 46002"
config_id = "default"
"#,
            geometry = geometry.display()
        );

        let err = parse(&toml_str).unwrap_err();
        assert!(matches!(err, ConfigError::Parse(_)));

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    #[test]
    fn geometry_path_missing_is_err() {
        let dir = std::env::temp_dir().join(format!(
            "tpcdaq-config-test-missing-geometry-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let missing_geometry = dir.join("does-not-exist.dat");

        let toml_str = format!(
            r#"
[system]
experiment = "mini_eTPC"
output_root = "/data/tpcdaq"
geometry = "{geometry}"

[[cobo]]
id = 0
listen = "0.0.0.0:46005"
data_sender_id = "CoBo[0]"

[decoder]
workers = 4

[root_sink]
snapshot_hz = 1.0
event_publish_hz = 20.0
build_timeout_ms = 1000

[monitor]
ws_listen = "0.0.0.0:9000"

[controller]
rest_listen = "0.0.0.0:8080"
passphrase = "change-me"
ecc_proxy = "GetEcc:tcp -h 127.0.0.1 -p 46002"
config_id = "default"
"#,
            geometry = missing_geometry.display()
        );

        let err = parse(&toml_str).unwrap_err();
        assert!(matches!(err, ConfigError::GeometryNotFound(ref p) if p == &missing_geometry));
    }

    // --- geometry パス存在チェック単体(発注書 001: 検証関数を分離してテストできること) ---

    #[test]
    fn validate_geometry_path_ok_for_existing_file() {
        let geometry = make_temp_geometry_file();
        assert!(validate_geometry_path(&geometry).is_ok());
        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    #[test]
    fn validate_geometry_path_err_for_missing_file() {
        let dir = std::env::temp_dir().join(format!(
            "tpcdaq-config-test-validate-missing-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let missing = dir.join("nope.dat");

        let err = validate_geometry_path(&missing).unwrap_err();
        assert!(matches!(err, ConfigError::GeometryNotFound(ref p) if p == &missing));
    }

    // --- load()(ファイル版)---

    #[test]
    fn load_reads_file_and_parses() {
        let geometry = make_temp_geometry_file();
        let config_path = geometry.parent().unwrap().join("config.toml");
        let toml_str = format!(
            r#"
[system]
experiment = "mini_eTPC"
output_root = "/data/tpcdaq"
geometry = "{geometry}"

[[cobo]]
id = 0
listen = "0.0.0.0:46005"
data_sender_id = "CoBo[0]"

[decoder]
workers = 4

[root_sink]
snapshot_hz = 1.0
event_publish_hz = 20.0
build_timeout_ms = 1000

[monitor]
ws_listen = "0.0.0.0:9000"

[controller]
rest_listen = "0.0.0.0:8080"
passphrase = "change-me"
ecc_proxy = "GetEcc:tcp -h 127.0.0.1 -p 46002"
config_id = "default"
"#,
            geometry = geometry.display()
        );
        std::fs::write(&config_path, toml_str).unwrap();

        let config = load(&config_path).unwrap();
        assert_eq!(config.system.experiment, "mini_eTPC");

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    // --- [receiver] セクション(006 で追加。SPEC §1.4 / §2.3 / §3.2) ---

    /// `[receiver]` は丸ごと省略できて、SPEC の既定値が入る。
    /// (既存の設定ファイルが 006 の追加で壊れないことの担保でもある)
    #[test]
    fn receiver_section_defaults_when_omitted() {
        let geometry = make_temp_geometry_file();
        let config = parse(&minimal_toml(&geometry, "")).unwrap();

        // SPEC §2.3「8 MiB 到達 or 10 ms 経過」/ §2.2「アイドル時 1 Hz」/ §1.4-2(HWM 1000)
        assert_eq!(config.receiver.batch_max_bytes, 8 * 1024 * 1024);
        assert_eq!(config.receiver.batch_max_ms, 10);
        assert_eq!(config.receiver.queue_frames, DEFAULT_QUEUE_FRAMES);
        assert_eq!(config.receiver.heartbeat_ms, 1000);
        assert_eq!(config.receiver.hwm, crate::zmq_helper::DEFAULT_HWM);
        // SPEC §3.2: graw-writer PULL = 47001、decoder PULL = 47002
        assert_eq!(
            config.receiver.graw_writer_endpoint,
            "tcp://127.0.0.1:47001"
        );
        assert_eq!(config.receiver.decoder_endpoint, "tcp://127.0.0.1:47002");

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    /// 明示値はすべて素通しされる(既定値と重ならない非対称な値を選ぶ)。
    #[test]
    fn receiver_section_values_override_the_defaults() {
        let geometry = make_temp_geometry_file();
        let section = r#"
[receiver]
batch_max_bytes = 262144
batch_max_ms = 3
queue_frames = 17
heartbeat_ms = 250
hwm = 64
graw_writer_endpoint = "tcp://10.0.0.5:47901"
decoder_endpoint = "tcp://10.0.0.6:47902"
"#;
        let config = parse(&minimal_toml(&geometry, section)).unwrap();

        assert_eq!(config.receiver.batch_max_bytes, 262_144);
        assert_eq!(config.receiver.batch_max_ms, 3);
        assert_eq!(config.receiver.queue_frames, 17);
        assert_eq!(config.receiver.heartbeat_ms, 250);
        assert_eq!(config.receiver.hwm, 64);
        assert_eq!(config.receiver.graw_writer_endpoint, "tcp://10.0.0.5:47901");
        assert_eq!(config.receiver.decoder_endpoint, "tcp://10.0.0.6:47902");

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    /// 0 段のキュー = 全フレーム破棄、HWM 0 = 無制限バッファ(SPEC §1.4-2 で禁止)。
    /// どちらも「起動してから静かに壊れる」ので起動失敗にする。
    #[test]
    fn receiver_degenerate_values_are_err() {
        let geometry = make_temp_geometry_file();
        for (section, needle) in [
            ("[receiver]\nqueue_frames = 0\n", "queue_frames"),
            ("[receiver]\nbatch_max_bytes = 0\n", "batch_max_bytes"),
            ("[receiver]\nbatch_max_ms = 0\n", "batch_max_ms"),
            ("[receiver]\nheartbeat_ms = 0\n", "heartbeat_ms"),
            ("[receiver]\nhwm = 0\n", "hwm"),
        ] {
            let err = parse(&minimal_toml(&geometry, section)).unwrap_err();
            match err {
                ConfigError::InvalidReceiver(msg) => {
                    assert!(msg.contains(needle), "message should name {needle}: {msg}")
                }
                other => panic!("expected InvalidReceiver for {section:?}, got {other}"),
            }
        }

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    #[test]
    fn receiver_unknown_field_is_err() {
        let geometry = make_temp_geometry_file();
        let err = parse(&minimal_toml(
            &geometry,
            "[receiver]\nbatch_max_kib = 8192\n",
        ))
        .unwrap_err();
        assert!(matches!(err, ConfigError::Parse(_)));

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    /// SPEC §3.2「コンポーネント REP = 47100 + 連番(receiver k = 47110 + k)」。
    #[test]
    fn receiver_command_port_base_matches_spec_3_2() {
        assert_eq!(RECEIVER_COMMAND_PORT_BASE, 47110);
        // 手計算: CoBo 1 の REP ポート = 47110 + 1 = 47111
        assert_eq!(RECEIVER_COMMAND_PORT_BASE + 1, 47111);
    }

    // --- [graw_writer] セクション(007 で追加。SPEC §3.2 / §7) ---

    /// `[graw_writer]` は丸ごと省略できて、SPEC §7 の既定値が入る。
    #[test]
    fn graw_writer_section_defaults_when_omitted() {
        let geometry = make_temp_geometry_file();
        let config = parse(&minimal_toml(&geometry, "")).unwrap();

        assert_eq!(config.graw_writer.pull_bind, "tcp://*:47001");
        assert_eq!(
            config.graw_writer.max_file_bytes,
            DEFAULT_GRAW_WRITER_MAX_FILE_BYTES
        );
        assert_eq!(config.graw_writer.max_file_bytes, 1024 * 1024 * 1024);
        assert_eq!(config.graw_writer.flush_interval_ms, 1000);

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    /// 明示値はすべて素通しされる(既定値と重ならない非対称な値)。
    #[test]
    fn graw_writer_section_values_override_the_defaults() {
        let geometry = make_temp_geometry_file();
        let section = r#"
[graw_writer]
pull_bind = "tcp://*:57001"
max_file_bytes = 123456789
flush_interval_ms = 250
"#;
        let config = parse(&minimal_toml(&geometry, section)).unwrap();

        assert_eq!(config.graw_writer.pull_bind, "tcp://*:57001");
        assert_eq!(config.graw_writer.max_file_bytes, 123_456_789);
        assert_eq!(config.graw_writer.flush_interval_ms, 250);

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    #[test]
    fn graw_writer_unknown_field_is_err() {
        let geometry = make_temp_geometry_file();
        let err = parse(&minimal_toml(
            &geometry,
            "[graw_writer]\nmax_file_kib = 1024\n",
        ))
        .unwrap_err();
        assert!(matches!(err, ConfigError::Parse(_)));

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    /// SPEC §3.2「コンポーネント REP = 47100 + 連番」。graw-writer は receiver 群(47110+k)の
    /// 手前 = 連番の先頭 47100。
    #[test]
    fn graw_writer_command_listen_matches_spec_3_2() {
        assert_eq!(GRAW_WRITER_COMMAND_LISTEN, "tcp://*:47100");
    }

    // --- [decoder] セクション(009 で追記。SPEC §2.3 / §3.1 / §3.2) ---

    /// `[decoder]` の既存キー(`workers`)だけを書いた最小形。009 で足したキーは
    /// すべて省略でき、SPEC §2.3/§3.2 の既定値が入る。
    #[test]
    fn decoder_section_defaults_when_the_new_keys_are_omitted() {
        let geometry = make_temp_geometry_file();
        // minimal_toml は `[decoder] workers = 4` を含む(= 新キーはすべて省略された形)。
        let config = parse(&minimal_toml(&geometry, "")).unwrap();

        assert_eq!(config.decoder.workers, 4, "既存キーは無改変");
        assert_eq!(config.decoder.pull_bind, "tcp://*:47002");
        assert_eq!(config.decoder.push_connect, "tcp://127.0.0.1:47003");
        assert_eq!(config.decoder.batch_max_bytes, DEFAULT_BATCH_MAX_BYTES);
        // 手計算: SPEC §2.3「8 MiB 到達 or 10 ms 経過」→ 8 * 1024 * 1024 = 8,388,608
        assert_eq!(config.decoder.batch_max_bytes, 8_388_608);
        assert_eq!(config.decoder.batch_max_ms, 10);

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    /// 明示値はすべて素通しされる(既定値と重ならない非対称な値)。
    #[test]
    fn decoder_section_values_override_the_defaults() {
        let geometry = make_temp_geometry_file();
        let toml_str = toml_with_decoder_section(
            &geometry,
            r#"
[decoder]
workers = 3
pull_bind = "tcp://*:57002"
push_connect = "tcp://10.0.0.7:57003"
batch_max_bytes = 1234567
batch_max_ms = 25
"#,
        );
        let config = parse(&toml_str).unwrap();

        assert_eq!(config.decoder.workers, 3);
        assert_eq!(config.decoder.pull_bind, "tcp://*:57002");
        assert_eq!(config.decoder.push_connect, "tcp://10.0.0.7:57003");
        assert_eq!(config.decoder.batch_max_bytes, 1_234_567);
        assert_eq!(config.decoder.batch_max_ms, 25);

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    #[test]
    fn decoder_unknown_field_is_err() {
        let geometry = make_temp_geometry_file();
        let toml_str =
            toml_with_decoder_section(&geometry, "[decoder]\nworkers = 1\nbatch_max_kib = 8192\n");
        let err = parse(&toml_str).unwrap_err();
        assert!(matches!(err, ConfigError::Parse(_)));

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    /// SPEC §3.2 v1.2「コンポーネント REP: decoder = 47101」/「decoder PULL bind = 47002」/
    /// 「root-sink PULL bind = 47003」。
    #[test]
    fn decoder_endpoints_match_spec_3_2() {
        assert_eq!(DECODER_COMMAND_LISTEN, "tcp://*:47101");
        assert_eq!(DEFAULT_DECODER_PULL_BIND, "tcp://*:47002");
        assert_eq!(DEFAULT_ROOT_SINK_ENDPOINT, "tcp://127.0.0.1:47003");
        // decoder の PULL bind と、receiver 側が connect する先はポートが一致すること。
        assert_eq!(DEFAULT_DECODER_ENDPOINT, "tcp://127.0.0.1:47002");
    }

    #[test]
    fn load_missing_file_returns_io_err() {
        let missing = std::env::temp_dir().join(format!(
            "tpcdaq-config-test-missing-config-{}.toml",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&missing);

        let err = load(&missing).unwrap_err();
        assert!(matches!(err, ConfigError::Io { .. }));
    }
}
