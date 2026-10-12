//! 設定モジュール(SPEC §3)。
//!
//! TOML 設定ファイルを読み、既定ポート・source_id 規約(SPEC §3.2)を適用したうえで
//! 検証し、[`Config`] を返す。パース・検証のどちらが失敗しても `Err` を返す
//! (半端な既定値のまま走らない — SPEC §3.2)。

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------
// 既定ポート・ID 規約(SPEC §3.2)
// ---------------------------------------------------------------------

/// CoBo データ着信ポートの基準値。実際の既定は `COBO_LISTEN_PORT_BASE + cobo_id`(SPEC §3.2)。
pub const COBO_LISTEN_PORT_BASE: u32 = 46005;

/// monitor WS の既定 listen アドレス(SPEC §3.2)。
pub const DEFAULT_MONITOR_WS_LISTEN: &str = "0.0.0.0:9000";

/// monitor SUB の既定接続先(SPEC §3.2「root-sink PUB bind = tcp://\*:47004」の受け側)。
pub const DEFAULT_MONITOR_SUB_ENDPOINT: &str = "tcp://127.0.0.1:47004";

/// monitor の live 送信キュー(0x02/0x03/0x10/0x11)の既定段数(TODO/026)。
///
/// live は **drop-oldest + `ws_dropped` 計数**(SPEC §10.3)。段数は「遅いクライアントが
/// どれだけ遅れてよいか」であって、ロスレス契約とは無関係(モニタ系)。
pub const DEFAULT_MONITOR_LIVE_QUEUE: usize = 64;

/// controller REST の既定 listen アドレス(SPEC §3.2)。
pub const DEFAULT_CONTROLLER_REST_LISTEN: &str = "0.0.0.0:8080";

/// controller のログ投稿 PULL bind の既定(SPEC §3.2「controller ログ投稿 PULL bind = 47005」)。
pub const DEFAULT_CONTROLLER_LOG_PULL_BIND: &str = "tcp://*:47005";

/// EOS 伝播待ちの既定タイムアウト(秒、SPEC §1.3。**ハード上限**)。
pub const DEFAULT_CONTROLLER_EOS_TIMEOUT_S: u64 = 5;

/// 受信静止(quiesce)判定の既定(ms、SPEC §1.3 v1.12 / TODO/033 論点 2′)。
///
/// 実機の `ecc stop` はデータリンクを close しないので EOF は来ない ——
/// 「EOF を `eos_timeout` 待つ」段は**毎停止まるごと空振り**する。代わりに
/// 「全 receiver の受信バイト数がこの時間だけ不変」= 在飛データを飲み切った、と読んで
/// 強制 EOS へ進む。LAN の在飛は ms 級なので 500 ms は 10 倍以上のマージン。
pub const DEFAULT_CONTROLLER_EOS_QUIESCE_MS: u64 = 500;

/// decoder の Batch source_id(SPEC §3.2 の source_id 表)。
pub const DECODER_SOURCE_ID: u32 = 100;

/// slowctl の Batch source_id(SPEC §3.2 の source_id 表 v1.31、旧 psu)。
pub const SLOWCTL_SOURCE_ID: u32 = 200;

/// slowctl の status PUB bind の既定(SPEC §3.2 v1.31)。
pub const DEFAULT_SLOWCTL_STATUS_PUB: &str = "tcp://*:47006";

/// slowctl → controller の LogPost PUSH 接続先の既定
/// ([`DEFAULT_CONTROLLER_LOG_PULL_BIND`] の connect 側表記)。
pub const DEFAULT_CONTROLLER_LOG_PUSH_ENDPOINT: &str = "tcp://127.0.0.1:47005";

/// 全デバイス共通の読み出し周期の既定(Hz、SPEC §3.1)。
pub const DEFAULT_SLOWCTL_POLL_HZ: f64 = 1.0;

/// 値が動かなくても logbook `slow` VALUE を書く間隔の既定(秒、SPEC §3.1 / §9.2)。
pub const DEFAULT_SLOWCTL_LOG_EVERY_S: u64 = 60;

/// TOML で受け付けるデバイス種別(SPEC §3.1 の列挙そのもの)。
pub const SLOWCTL_DEVICE_TYPES: &[&str] = &[
    "bronkhorst_flowbus",
    "arduino_env",
    "hivolta",
    "hmp2020",
    "fake",
];

/// シリアルポートを `addr`(TCP)ではなく `port` で指定する種別。
const SLOWCTL_SERIAL_TYPES: &[&str] = &["bronkhorst_flowbus", "arduino_env", "hmp2020", "fake"];

/// 取り違えに強いポート名か(TODO/079 §2.1: udev の by-id、無ければ by-path)。
///
/// **判定だけ**を返す純関数。偽でも起動は止めない(Mac 開発機に by-id は無い)—— 警告の
/// 出し所は [`resolve`]。
pub fn is_stable_serial_port(port: &str) -> bool {
    port.starts_with("/dev/serial/by-id/") || port.starts_with("/dev/serial/by-path/")
}

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

/// controller → graw-writer のコマンド REQ 接続先(SPEC §3.2「graw-writer = 47100」)。
/// bind 側 [`GRAW_WRITER_COMMAND_LISTEN`] とはポートは同じでアドレス表記だけが違う。
pub const DEFAULT_GRAW_WRITER_COMMAND_ENDPOINT: &str = "tcp://127.0.0.1:47100";

/// controller → decoder のコマンド REQ 接続先(SPEC §3.2「decoder = 47101」)。
pub const DEFAULT_DECODER_COMMAND_ENDPOINT: &str = "tcp://127.0.0.1:47101";

/// controller → ecc-bridge の REQ 接続先(SPEC §3.2「ecc-bridge REP = tcp://*:47200」)。
pub const DEFAULT_ECC_COMMAND_ENDPOINT: &str = "tcp://127.0.0.1:47200";

/// controller → receiver k のコマンド REQ 接続先の既定(SPEC §3.2「receiver k = 47110+k」)。
pub fn default_receiver_command_endpoint(cobo_id: u32) -> String {
    format!("tcp://127.0.0.1:{}", RECEIVER_COMMAND_PORT_BASE + cobo_id)
}

/// decoder の PUSH 送信タイムアウトの既定(ミリ秒、TODO/009 の停止設計)。
///
/// 通常運転ではタイムアウトしても諦めずに再試行する(ロスレス = 下流の背圧で待つ)。
/// **Reset コマンド処理中に限り**、このタイムアウトで送出待ちを打ち切れる
/// (破棄は `eos_abandoned` / `batches_abandoned` として可視化する)。
pub const DEFAULT_DECODER_SEND_TIMEOUT_MS: i32 = 1000;

/// receiver の PUSH 送信タイムアウトの既定(ミリ秒、TODO/023-2 = P2 レビュー R-P2-3)。
///
/// **decoder と同じ出所・同じ既定**([`DEFAULT_DECODER_SEND_TIMEOUT_MS`])。receiver も
/// decoder と同じく「タイムアウトしても通常は諦めず再試行、`Reset` / 畳み込み中に限り
/// 打ち切って `messages_abandoned` として可視化」という停止設計を採るので、打ち切りの
/// 粒度を 2 コンポーネントで揃える(`[receiver]` に TOML キーは足さない — decoder と同じ流儀)。
pub const DEFAULT_RECEIVER_SEND_TIMEOUT_MS: i32 = DEFAULT_DECODER_SEND_TIMEOUT_MS;

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

    #[error("invalid [monitor] setting: {0}")]
    InvalidMonitor(String),

    #[error("invalid [controller] setting: {0}")]
    InvalidController(String),

    #[error("invalid [slowctl] setting: {0}")]
    InvalidSlowctl(String),
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
    /// `[slowctl]`(SPEC §3.1 v1.31)。**セクションごと省略可** —— 省略 = slowctl を走らせない
    /// (既存の実運用 TOML が無変更で読める)。
    pub slowctl: Option<SlowctlConfig>,
}

/// `[slowctl]`(SPEC §3.1 v1.31、TODO/079)。**監視・記録・アラームのみ** ——
/// setpoint を書くキーは存在しない(SPEC §11 の安全境界)。
#[derive(Debug, Clone, PartialEq)]
pub struct SlowctlConfig {
    /// status PUB の bind(既定 [`DEFAULT_SLOWCTL_STATUS_PUB`])。
    pub status_pub: String,
    /// controller のログ投稿 PULL への PUSH 接続先(既定
    /// [`DEFAULT_CONTROLLER_LOG_PUSH_ENDPOINT`])。SPEC §3.1 の例には無いが、
    /// ポートを**コードに焼き込まない**ため設定で上書きできる形にしてある。
    pub log_push: String,
    /// 全デバイス共通の読み出し周期(Hz)。
    pub poll_hz: f64,
    /// 値が動かなくても VALUE を書く間隔(秒、SPEC §9.2)。
    pub log_every_s: u64,
    /// `[[slowctl.device]]`。
    pub device: Vec<SlowDeviceConfig>,
    /// `[slowctl.influx]`(TODO/122)。**省略可** —— 省略 = InfluxDB に書かない(従来と同一)。
    pub influx: Option<InfluxConfig>,
}

/// `[slowctl.influx]` の `batch_ms` の既定(TODO/122 §4)。
pub const DEFAULT_INFLUX_BATCH_MS: u64 = 1000;

/// `[slowctl.influx]` の `buffer_max_points` の既定(TODO/122 §4)。
///
/// 1 点 = `Sample` 1 個。HV 42 値 / 5 s + LV 10 値 / 1 s ≈ 18 点/s なので ≈ 1.5 h 分。
pub const DEFAULT_INFLUX_BUFFER_MAX_POINTS: usize = 100_000;

/// `[slowctl.influx]` の `timeout_ms` の既定(TODO/122 §4)。
pub const DEFAULT_INFLUX_TIMEOUT_MS: u64 = 2000;

/// `[slowctl.influx]`(TODO/122 §4)。slowctl の全サンプルを InfluxDB v2 write API へ書く。
///
/// **トークンそのものは持たない**(TOML にも `Debug` 出力にも載せない)。持つのはファイルの
/// パスだけで、**中身を読むのは slowctl の起動時だけ**(`slowctl::influx::read_token`)。
/// config の検証では読まない —— controller / monitor も同じ TOML を読むので、監視系の
/// トークンの不備で DAQ 全体を起動不能にしない(never-stop、保存と監視の分離)。
#[derive(Debug, Clone, PartialEq)]
pub struct InfluxConfig {
    /// 例 `http://127.0.0.1:8086`(`/api/v2/write` はコードが足す)。
    pub url: String,
    pub org: String,
    pub bucket: String,
    /// トークンを 1 行だけ書いたファイル(前後の空白は除く)。
    pub token_file: PathBuf,
    /// POST の間隔(ミリ秒)。
    pub batch_ms: u64,
    /// 再送バッファの上限(点 = `Sample` 1 個)。超えたら古い方から捨てて数える。
    pub buffer_max_points: usize,
    /// 1 回の POST にかける上限(ミリ秒)。
    pub timeout_ms: u64,
}

/// `[[slowctl.device]]` 1 台分(SPEC §3.1)。
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlowDeviceConfig {
    /// `bronkhorst_flowbus` / `arduino_env` / `hivolta` / `hmp2020` / `fake`。
    #[serde(rename = "type")]
    pub device_type: String,
    /// logbook / status の `device`。
    pub name: String,
    /// シリアル系の口(`/dev/serial/by-id/…`。`fake` はスクリプトファイルのパス)。
    #[serde(default)]
    pub port: Option<String>,
    /// TCP 系(HiVolta)の口。
    #[serde(default)]
    pub addr: Option<String>,
    /// FLOW-BUS のノード指定(`"scan"` か明示リスト)。
    #[serde(default)]
    pub nodes: crate::slowctl::NodesSpec,
    /// 空でなければ open 時に機器の serial と照合する(TODO/079 §2.1)。
    #[serde(default)]
    pub expect_serial: String,
    /// `[lo, hi]` 逸脱の監視規則。
    #[serde(default)]
    pub alarms: Vec<crate::slowctl::AlarmRule>,
    /// この機器だけの読み出し周期(秒、TODO/119)。省略 = `1 / poll_hz`。
    #[serde(default)]
    pub poll_s: Option<f64>,
    /// hivolta のみ: ch ごとに alarm から外す STATUS フラグ(`UNV` / `OVV` だけ許可、TODO/121 §D)。
    #[serde(default)]
    pub status_ignore: Vec<crate::slowctl::hivolta::StatusIgnore>,
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
    /// raw ツリー出力(`run{run:04}_raw.root`)を有効にするか(SPEC §3.1 / §6.7 v1.28)。
    /// 省略可、既定 false(root-sink 自体は §6.7 でオプション出力として定義。
    /// TOML CLI 起動器はまだ無く、このキーは仕様どおりの受け口 — 075 B-1)。
    #[serde(default)]
    pub raw_tree: bool,
}

/// monitor 固有の設定(`[monitor]`)。すべて省略可で、SPEC §3.2 の既定が入る(026 で追記)。
#[derive(Debug, Clone, PartialEq)]
pub struct MonitorConfig {
    /// WS の listen アドレス(既定 [`DEFAULT_MONITOR_WS_LISTEN`])。
    pub ws_listen: String,
    /// root-sink のモニタ PUB への SUB 接続先(既定 [`DEFAULT_MONITOR_SUB_ENDPOINT`])。
    pub sub_endpoint: String,
    /// 表示変換に使うジオメトリ。省略時は `[system] geometry` をそのまま使う
    /// (monitor だけ別のジオメトリを見たいときのための上書き口)。
    pub geometry: PathBuf,
    /// live 送信キューの段数(既定 [`DEFAULT_MONITOR_LIVE_QUEUE`])。
    pub live_queue: usize,
    /// slowctl の status PUB への SUB 接続先(SPEC §3.1 v1.31)。
    /// **省略 = 購読しない**(WS の `status` JSON に `slow` は出ない = 従来と同一)。
    pub slow_sub: Option<String>,
}

/// ECC の ConfigId(SPEC §3.1 / §8.2 v1.13)。
///
/// 実 ECC の ConfigId は **describe / prepare / configure の 3 組**で、実運用は相ごとに
/// 別名を使う(実例: `describe = "zCobo-ZC706"` / `configure = "pulser"` —— TODO/038 の実測)。
/// 設定の `config_id = "x"`(文字列)は **3 相とも `x`** の略記で、ここへ展開される
/// ([`ConfigIds::same`])。ecc-bridge の JSON は元よりアクション毎 `config_id` なので、
/// controller が [`ConfigIds::for_action`] でその相の id を選んで渡すだけでよい。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigIds {
    pub describe: String,
    pub prepare: String,
    pub configure: String,
}

impl ConfigIds {
    /// 3 相同値(= 設定の文字列形)。
    pub fn same(id: impl Into<String>) -> Self {
        let id = id.into();
        Self {
            describe: id.clone(),
            prepare: id.clone(),
            configure: id,
        }
    }

    /// 3 相が同値ならその値、非同値なら `None`。
    ///
    /// logbook `run_start` が `config_ids` を出すかどうかの分岐条件そのもの(SPEC §9.2 v1.13)。
    pub fn same_value(&self) -> Option<&str> {
        (self.describe == self.prepare && self.prepare == self.configure)
            .then_some(self.describe.as_str())
    }

    /// ecc-bridge のアクション名 → その相の id。`config_id` を使わないアクション
    /// (`start` / `stop` / `breakup` / `reset` / `status`)は `None`(SPEC §8.2)。
    pub fn for_action(&self, action: &str) -> Option<&str> {
        match action {
            "describe" => Some(&self.describe),
            "prepare" => Some(&self.prepare),
            "configure" => Some(&self.configure),
            _ => None,
        }
    }
}

/// 文字列との比較は **3 相すべてがその文字列**のときだけ真(略記の設定と等しい、の意)。
/// 単一 id 時代の呼び出し・テストがそのまま読める。
impl PartialEq<&str> for ConfigIds {
    fn eq(&self, other: &&str) -> bool {
        self.same_value() == Some(*other)
    }
}

/// `rest_listen` を省略すると `DEFAULT_CONTROLLER_REST_LISTEN`(SPEC §3.2)が既定として入る。
///
/// 016 で追記したキー(SPEC §8.1 の controller)はすべて省略可。コンポーネントのコマンド
/// エンドポイント表は SPEC §3.2 の既定(47100 / 47101 / 47110+k / 47200)を入れたうえで、
/// 設定で上書きできる。
#[derive(Debug, Clone, PartialEq)]
pub struct ControllerConfig {
    pub rest_listen: String,
    pub passphrase: String,
    /// ecc-bridge が使う **Ice** プロキシ文字列(controller は使わず ecc-bridge へ渡す設定)。
    pub ecc_proxy: String,
    /// ECC の ConfigId(3 相)。TOML では文字列(3 相同値の略記)かテーブルで書ける。
    pub config_id: ConfigIds,
    /// ログ投稿 PULL の bind(SPEC §2.3 LogPost / §3.2)。
    pub log_pull_bind: String,
    /// EOS 伝播待ちのタイムアウト(秒、SPEC §1.3)。**両段のハード上限**。
    pub eos_timeout_s: u64,
    /// 停止第一段の受信静止(quiesce)判定時間(ms、SPEC §1.3 v1.12)。省略可(既定 500)。
    pub eos_quiesce_ms: u64,
    /// Web UI 静的ファイルの根。`None` = 配信しない(SPEC §8.1、UI は後波)。
    pub ui_dir: Option<PathBuf>,
    /// controller → graw-writer のコマンド REQ 接続先。
    pub graw_writer_command: String,
    /// controller → decoder のコマンド REQ 接続先。
    pub decoder_command: String,
    /// controller → receiver のコマンド REQ 接続先。**`[[cobo]]` と同じ順・同じ長さ**。
    pub receiver_commands: Vec<String>,
    /// controller → ecc-bridge の REQ 接続先。
    pub ecc_command: String,
    /// DataLinkSet の `router_ip`(SPEC §8.2)。`None` なら receiver の実 bind アドレスから
    /// 導く(`0.0.0.0` のような不定アドレスなら `127.0.0.1` に落として警告する)。
    pub router_ip: Option<String>,
    /// 078 レーン B: ECC の config repo(生成 xcfg の出力先。パラメータ JSON の `template` も
    /// この下から読む)。省略時は params 系 REST(`/api/params*`)が 503 を返す
    /// (黙って無効にしない — CLAUDE.md)。`router_ip` と同じ「Option のまま持ち回り、
    /// 既定は呼び出し側で足す」流儀(ここは既定が無い)。
    pub ecc_config_dir: Option<PathBuf>,
    /// 078 レーン B: パラメータ JSON(current.json / `<id>.json`)の保存先。省略時の既定
    /// `<output_root>/params` は `router_ip` と同じく呼び出し側([`crate::controller::Controller`])
    /// で解決する(ここでは解決しない — output_root は同じ `Config` の別セクションにある)。
    pub params_dir: Option<PathBuf>,
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
    #[serde(default)]
    slowctl: Option<RawSlowctl>,
}

/// `[slowctl]` の TOML そのまま(079 Phase 2)。
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct RawSlowctl {
    status_pub: Option<String>,
    log_push: Option<String>,
    poll_hz: Option<f64>,
    log_every_s: Option<u64>,
    device: Vec<SlowDeviceConfig>,
    influx: Option<RawInflux>,
}

/// `[slowctl.influx]` の TOML そのまま(TODO/122)。url / org / bucket / token_file は必須。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawInflux {
    url: String,
    org: String,
    bucket: String,
    token_file: PathBuf,
    #[serde(default)]
    batch_ms: Option<u64>,
    #[serde(default)]
    buffer_max_points: Option<usize>,
    #[serde(default)]
    timeout_ms: Option<u64>,
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
    // ---- 026 で追記(すべて省略可)----
    #[serde(default)]
    sub_endpoint: Option<String>,
    #[serde(default)]
    geometry: Option<PathBuf>,
    #[serde(default)]
    live_queue: Option<usize>,
    // ---- 079 Phase 2 で追記(省略可)----
    #[serde(default)]
    slow_sub: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawController {
    #[serde(default)]
    rest_listen: Option<String>,
    passphrase: String,
    ecc_proxy: String,
    config_id: RawConfigId,
    // ---- 016 で追記(すべて省略可)----
    #[serde(default)]
    log_pull_bind: Option<String>,
    #[serde(default)]
    eos_timeout_s: Option<u64>,
    // ---- 033 で追記(省略可)----
    #[serde(default)]
    eos_quiesce_ms: Option<u64>,
    #[serde(default)]
    ui_dir: Option<PathBuf>,
    #[serde(default)]
    graw_writer_command: Option<String>,
    #[serde(default)]
    decoder_command: Option<String>,
    #[serde(default)]
    receiver_commands: Option<Vec<String>>,
    #[serde(default)]
    ecc_command: Option<String>,
    #[serde(default)]
    router_ip: Option<String>,
    // ---- 078 レーン B で追記(省略可、既存 TOML は無変更で読める)----
    #[serde(default)]
    ecc_config_dir: Option<PathBuf>,
    #[serde(default)]
    params_dir: Option<PathBuf>,
}

/// `[controller] config_id` の 2 通りの書き方(SPEC §3.1 v1.13)。
///
/// ```toml
/// config_id = "default"                                    # 3 相同値の略記
/// config_id = { describe = "zCobo-ZC706", prepare = "pulser", configure = "pulser" }
/// ```
///
/// テーブル形で相が欠けていれば `Err`(untagged なのでメッセージは
/// 「どの variant にも合わない」止まりだが、**黙って空 id にはしない**)。
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawConfigId {
    Same(String),
    PerPhase(ConfigIds),
}

impl From<RawConfigId> for ConfigIds {
    fn from(raw: RawConfigId) -> Self {
        match raw {
            RawConfigId::Same(id) => ConfigIds::same(id),
            RawConfigId::PerPhase(ids) => ids,
        }
    }
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
    let cobo: Vec<CoboConfig> = raw
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
        sub_endpoint: raw
            .monitor
            .sub_endpoint
            .unwrap_or_else(|| DEFAULT_MONITOR_SUB_ENDPOINT.to_string()),
        geometry: raw
            .monitor
            .geometry
            .unwrap_or_else(|| raw.system.geometry.clone()),
        live_queue: raw.monitor.live_queue.unwrap_or(DEFAULT_MONITOR_LIVE_QUEUE),
        slow_sub: raw.monitor.slow_sub,
    };

    let slowctl = raw.slowctl.map(|raw| {
        for device in &raw.device {
            if let Some(port) = &device.port {
                if SLOWCTL_SERIAL_TYPES.contains(&device.device_type.as_str())
                    && device.device_type != "fake"
                    && !is_stable_serial_port(port)
                {
                    // 拒否はしない(開発機に by-id は無い)。ただし黙認もしない — CLAUDE.md。
                    tracing::warn!(
                        device = device.name,
                        port,
                        "slowctl: port is not a /dev/serial/by-id or by-path name — \
                         USB の列挙順が変われば別の機器を読む(TODO/079 §2.1)"
                    );
                }
            }
        }
        SlowctlConfig {
            status_pub: raw
                .status_pub
                .unwrap_or_else(|| DEFAULT_SLOWCTL_STATUS_PUB.to_string()),
            log_push: raw
                .log_push
                .unwrap_or_else(|| DEFAULT_CONTROLLER_LOG_PUSH_ENDPOINT.to_string()),
            poll_hz: raw.poll_hz.unwrap_or(DEFAULT_SLOWCTL_POLL_HZ),
            log_every_s: raw.log_every_s.unwrap_or(DEFAULT_SLOWCTL_LOG_EVERY_S),
            device: raw.device,
            influx: raw.influx.map(|influx| InfluxConfig {
                url: influx.url,
                org: influx.org,
                bucket: influx.bucket,
                token_file: influx.token_file,
                batch_ms: influx.batch_ms.unwrap_or(DEFAULT_INFLUX_BATCH_MS),
                buffer_max_points: influx
                    .buffer_max_points
                    .unwrap_or(DEFAULT_INFLUX_BUFFER_MAX_POINTS),
                timeout_ms: influx.timeout_ms.unwrap_or(DEFAULT_INFLUX_TIMEOUT_MS),
            }),
        }
    });

    let controller = ControllerConfig {
        rest_listen: raw
            .controller
            .rest_listen
            .unwrap_or_else(|| DEFAULT_CONTROLLER_REST_LISTEN.to_string()),
        passphrase: raw.controller.passphrase,
        ecc_proxy: raw.controller.ecc_proxy,
        config_id: raw.controller.config_id.into(),
        log_pull_bind: raw
            .controller
            .log_pull_bind
            .unwrap_or_else(|| DEFAULT_CONTROLLER_LOG_PULL_BIND.to_string()),
        eos_timeout_s: raw
            .controller
            .eos_timeout_s
            .unwrap_or(DEFAULT_CONTROLLER_EOS_TIMEOUT_S),
        eos_quiesce_ms: raw
            .controller
            .eos_quiesce_ms
            .unwrap_or(DEFAULT_CONTROLLER_EOS_QUIESCE_MS),
        ui_dir: raw.controller.ui_dir,
        graw_writer_command: raw
            .controller
            .graw_writer_command
            .unwrap_or_else(|| DEFAULT_GRAW_WRITER_COMMAND_ENDPOINT.to_string()),
        decoder_command: raw
            .controller
            .decoder_command
            .unwrap_or_else(|| DEFAULT_DECODER_COMMAND_ENDPOINT.to_string()),
        receiver_commands: raw.controller.receiver_commands.unwrap_or_else(|| {
            cobo.iter()
                .map(|c| default_receiver_command_endpoint(c.id))
                .collect()
        }),
        ecc_command: raw
            .controller
            .ecc_command
            .unwrap_or_else(|| DEFAULT_ECC_COMMAND_ENDPOINT.to_string()),
        router_ip: raw.controller.router_ip,
        ecc_config_dir: raw.controller.ecc_config_dir,
        params_dir: raw.controller.params_dir,
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
        slowctl,
    }
}

fn validate(config: &Config) -> Result<(), ConfigError> {
    validate_cobo_ids(&config.cobo)?;
    validate_cobo_listen_unique(&config.cobo)?;
    validate_geometry_path(&config.system.geometry)?;
    validate_receiver(&config.receiver)?;
    validate_monitor(&config.monitor)?;
    validate_controller(&config.controller, config.cobo.len())?;
    if let Some(slowctl) = &config.slowctl {
        validate_slowctl(slowctl)?;
    }
    Ok(())
}

/// `[slowctl]` を検証する(SPEC §3.1 v1.31)。
///
/// 「静かに何も読まない slowctl」を作らないことが目的: 知らない `type`、口の無いデバイス、
/// 0 以下の周期はすべて起動失敗にする(SPEC §3.2「半端な既定値で走らない」)。
fn validate_slowctl(slowctl: &SlowctlConfig) -> Result<(), ConfigError> {
    // NaN も含めて「正の有限値でない」を弾く(黙って走らせない)。
    if !slowctl.poll_hz.is_finite() || slowctl.poll_hz <= 0.0 {
        return Err(ConfigError::InvalidSlowctl(format!(
            "poll_hz must be greater than 0, got {}",
            slowctl.poll_hz
        )));
    }
    let mut seen: HashSet<&str> = HashSet::new();
    for device in &slowctl.device {
        if !SLOWCTL_DEVICE_TYPES.contains(&device.device_type.as_str()) {
            return Err(ConfigError::InvalidSlowctl(format!(
                "unknown device type {:?} (known: {})",
                device.device_type,
                SLOWCTL_DEVICE_TYPES.join(", ")
            )));
        }
        if !seen.insert(device.name.as_str()) {
            return Err(ConfigError::InvalidSlowctl(format!(
                "duplicate device name {:?} — logbook / status のキーが衝突する",
                device.name
            )));
        }
        let serial = SLOWCTL_SERIAL_TYPES.contains(&device.device_type.as_str());
        // TODO/089 レビュー(2026-09-23): `hivolta` は TCP(`addr`)と USB VCP(`port`)の
        // どちらでも話せる(component.rs::open_driver は `addr` があれば優先、無ければ
        // `port` へフォールバック)。他の型の「serial なら port 必須 / それ以外は addr 必須」
        // という二分法は崩さず、`hivolta` だけ「どちらか 1 つ以上」の OR 検証にする。
        if device.device_type == "hivolta" {
            if device.addr.is_none() && device.port.is_none() {
                return Err(ConfigError::InvalidSlowctl(format!(
                    "device {:?} of type {:?} needs an `addr` (TCP :1470) or `port` (USB VCP)",
                    device.name, device.device_type
                )));
            }
        } else if serial && device.port.is_none() {
            return Err(ConfigError::InvalidSlowctl(format!(
                "device {:?} of type {:?} needs a `port`",
                device.name, device.device_type
            )));
        } else if !serial && device.addr.is_none() {
            return Err(ConfigError::InvalidSlowctl(format!(
                "device {:?} of type {:?} needs an `addr`",
                device.name, device.device_type
            )));
        }
        crate::slowctl::hivolta::ignore_masks(&device.status_ignore)
            .map_err(|e| ConfigError::InvalidSlowctl(format!("device {:?}: {e}", device.name)))?;
        if let Some(poll_s) = device.poll_s {
            if !poll_s.is_finite() || poll_s <= 0.0 {
                return Err(ConfigError::InvalidSlowctl(format!(
                    "device {:?}: poll_s must be greater than 0, got {poll_s}",
                    device.name
                )));
            }
        }
        device
            .nodes
            .resolve()
            .map_err(|e| ConfigError::InvalidSlowctl(format!("device {:?}: {e}", device.name)))?;
        for alarm in &device.alarms {
            if alarm.lo > alarm.hi {
                return Err(ConfigError::InvalidSlowctl(format!(
                    "device {:?} alarm on {:?}: lo {} is above hi {}",
                    device.name, alarm.param, alarm.lo, alarm.hi
                )));
            }
        }
    }
    if let Some(influx) = &slowctl.influx {
        validate_influx(influx)?;
    }
    Ok(())
}

/// `[slowctl.influx]` を検証する(TODO/122 §4)。
///
/// 書けない設定のまま走って「InfluxDB に何も入っていなかった」と後で気づく状態を作らない:
/// url の形、0 や空の値は起動失敗にする。
///
/// **トークンファイルの中身はここでは読まない**(見るのはパスが空でないことまで)。
/// 読めないことは slowctl の起動だけが失敗として出す —— [`InfluxConfig`] の doc。
fn validate_influx(influx: &InfluxConfig) -> Result<(), ConfigError> {
    let invalid =
        |message: String| ConfigError::InvalidSlowctl(format!("[slowctl.influx] {message}"));
    // HTTP クライアント(ureq)は TLS の feature を切ってある。https を通すと実行時に
    // 必ず失敗し続けるので、ここで止める。
    if influx.url.starts_with("https://") {
        return Err(invalid(format!(
            "url {:?}: TLS は未対応、localhost の http を使う (TLS is not built in; \
             use plain http to a local InfluxDB)",
            influx.url
        )));
    }
    if !influx.url.starts_with("http://") {
        return Err(invalid(format!(
            "url must start with http://, got {:?}",
            influx.url
        )));
    }
    if influx.token_file.as_os_str().is_empty() {
        return Err(invalid("token_file must not be empty".to_string()));
    }
    if influx.org.is_empty() || influx.bucket.is_empty() {
        return Err(invalid("org and bucket must not be empty".to_string()));
    }
    if influx.batch_ms == 0 || influx.buffer_max_points == 0 || influx.timeout_ms == 0 {
        return Err(invalid(format!(
            "batch_ms / buffer_max_points / timeout_ms must be greater than 0, got {} / {} / {}",
            influx.batch_ms, influx.buffer_max_points, influx.timeout_ms
        )));
    }
    Ok(())
}

/// `[monitor]` の 026 追記分を検証する(SPEC §5.4 / §10)。
///
/// `live_queue = 0` は「live メッセージを 1 通も保持しない」= 全部落とす設定で、
/// モニタとして意味を成さない。ジオメトリは表示変換(UVW グリッド)の前提なので、
/// 上書きした場合も `[system] geometry` と同じ強さで存在を確認する。
fn validate_monitor(monitor: &MonitorConfig) -> Result<(), ConfigError> {
    if monitor.live_queue == 0 {
        return Err(ConfigError::InvalidMonitor(
            "live_queue must be greater than 0".to_string(),
        ));
    }
    validate_geometry_path(&monitor.geometry)
}

/// `[controller]` の 016 追記分を検証する(SPEC §8.1)。
///
/// `receiver_commands` を手書きで上書きしたとき本数がずれていると、controller は
/// 一部の receiver に永久に到達できないまま起動してしまう(= 静かな配線ミス)。
/// `eos_timeout_s = 0` は「EOS を一切待たずに必ず強制 EOS」になり、SPEC §1.3 の
/// 停止シーケンスが意味を失うので拒否する。`eos_quiesce_ms = 0` も同様に
/// 「受信静止を一切待たずに即・強制 EOS」= 033 裁定が不採用とした挙動なので拒否する。
fn validate_controller(
    controller: &ControllerConfig,
    cobo_count: usize,
) -> Result<(), ConfigError> {
    if controller.receiver_commands.len() != cobo_count {
        return Err(ConfigError::InvalidController(format!(
            "receiver_commands has {} entries but there are {cobo_count} [[cobo]] blocks",
            controller.receiver_commands.len()
        )));
    }
    if controller.eos_timeout_s == 0 {
        return Err(ConfigError::InvalidController(
            "eos_timeout_s must be greater than 0".to_string(),
        ));
    }
    if controller.eos_quiesce_ms == 0 {
        return Err(ConfigError::InvalidController(
            "eos_quiesce_ms must be greater than 0".to_string(),
        ));
    }
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
ecc_proxy = "Ecc:tcp -h 127.0.0.1 -p 46002"
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
ecc_proxy = "Ecc:tcp -h 127.0.0.1 -p 46002"
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
ecc_proxy = "Ecc:tcp -h 127.0.0.1 -p 46002"
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
        assert_eq!(config.controller.ecc_proxy, "Ecc:tcp -h 127.0.0.1 -p 46002");
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
ecc_proxy = "Ecc:tcp -h 127.0.0.1 -p 46002"
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

    // --- 075 で足した `[root_sink] raw_tree`(SPEC §3.1 / §6.7 v1.28)---

    /// minimal_toml の `[root_sink]` は既存 3 キーのみ(raw_tree は省略) → 既定 false。
    #[test]
    fn root_sink_raw_tree_defaults_to_false_when_omitted() {
        let geometry = make_temp_geometry_file();
        let config = parse(&minimal_toml(&geometry, "")).unwrap();

        assert!(!config.root_sink.raw_tree);

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    #[test]
    fn root_sink_raw_tree_true_is_read() {
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
raw_tree = true

[monitor]
ws_listen = "0.0.0.0:9000"

[controller]
rest_listen = "0.0.0.0:8080"
passphrase = "change-me"
ecc_proxy = "Ecc:tcp -h 127.0.0.1 -p 46002"
config_id = "default"
"#,
            geometry = geometry.display()
        );

        let config = parse(&toml_str).unwrap();

        assert!(config.root_sink.raw_tree);

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
ecc_proxy = "Ecc:tcp -h 127.0.0.1 -p 46002"
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
ecc_proxy = "Ecc:tcp -h 127.0.0.1 -p 46002"
config_id = "default"
"#,
            geometry = geometry.display()
        );

        let config = parse(&toml_str).unwrap();

        assert_eq!(config.monitor.ws_listen, DEFAULT_MONITOR_WS_LISTEN);

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    // --- 026 で足した `[monitor]` キー(SPEC §3.2 の既定 + 上書き)---

    #[test]
    fn monitor_keys_default_to_the_spec_ports_when_omitted() {
        let geometry = make_temp_geometry_file();
        let config = parse(&minimal_toml(&geometry, "")).unwrap();

        // SPEC §3.2: root-sink PUB bind = 47004 / monitor WS = 9000。
        assert_eq!(config.monitor.sub_endpoint, DEFAULT_MONITOR_SUB_ENDPOINT);
        assert_eq!(config.monitor.sub_endpoint, "tcp://127.0.0.1:47004");
        assert_eq!(config.monitor.ws_listen, "0.0.0.0:9000");
        assert_eq!(config.monitor.live_queue, DEFAULT_MONITOR_LIVE_QUEUE);
        assert_eq!(config.monitor.live_queue, 64);
        // `[monitor] geometry` 省略時は `[system] geometry` を使う。
        assert_eq!(config.monitor.geometry, geometry);

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    #[test]
    fn monitor_keys_can_be_overridden() {
        let geometry = make_temp_geometry_file();
        let other = geometry.parent().unwrap().join("monitor_geometry.dat");
        std::fs::write(&other, b"# monitor-side geometry override\n").unwrap();
        let toml_str = minimal_toml(&geometry, "").replace(
            "[monitor]\nws_listen = \"0.0.0.0:9000\"",
            &format!(
                "[monitor]\nws_listen = \"127.0.0.1:19000\"\n\
                 sub_endpoint = \"tcp://127.0.0.1:57004\"\n\
                 live_queue = 7\n\
                 geometry = \"{}\"",
                other.display()
            ),
        );

        let config = parse(&toml_str).unwrap();

        assert_eq!(config.monitor.ws_listen, "127.0.0.1:19000");
        assert_eq!(config.monitor.sub_endpoint, "tcp://127.0.0.1:57004");
        assert_eq!(config.monitor.live_queue, 7);
        assert_eq!(config.monitor.geometry, other);
        // `[system] geometry` 側は動かない。
        assert_eq!(config.system.geometry, geometry);

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    /// live_queue = 0 は「有界キュー段数 0」= 全部落とす設定。半端な既定値で走らない
    /// (SPEC §3.2)ので起動失敗にする。
    #[test]
    fn monitor_live_queue_zero_is_rejected() {
        let geometry = make_temp_geometry_file();
        let toml_str = minimal_toml(&geometry, "").replace(
            "[monitor]\nws_listen = \"0.0.0.0:9000\"",
            "[monitor]\nws_listen = \"0.0.0.0:9000\"\nlive_queue = 0",
        );

        let err = parse(&toml_str).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidMonitor(_)), "got {err:?}");

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    /// 存在しない `[monitor] geometry` は起動失敗(ジオメトリ無しでは表示変換ができない)。
    #[test]
    fn monitor_geometry_must_exist() {
        let geometry = make_temp_geometry_file();
        let toml_str = minimal_toml(&geometry, "").replace(
            "[monitor]\nws_listen = \"0.0.0.0:9000\"",
            "[monitor]\nws_listen = \"0.0.0.0:9000\"\ngeometry = \"/nonexistent/monitor.dat\"",
        );

        let err = parse(&toml_str).unwrap_err();
        assert!(
            matches!(err, ConfigError::GeometryNotFound(ref p) if p == Path::new("/nonexistent/monitor.dat")),
            "got {err:?}"
        );

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
ecc_proxy = "Ecc:tcp -h 127.0.0.1 -p 46002"
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
        // SPEC §3.2 source_id 表: receiver = cobo_id、decoder = 100、slowctl = 200(旧 psu)。
        // receiver は CoboConfig::id をそのまま使うので専用定数はない。
        assert_eq!(DECODER_SOURCE_ID, 100);
        assert_eq!(SLOWCTL_SOURCE_ID, 200);
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
ecc_proxy = "Ecc:tcp -h 127.0.0.1 -p 46002"
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
ecc_proxy = "Ecc:tcp -h 127.0.0.1 -p 46002"
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
ecc_proxy = "Ecc:tcp -h 127.0.0.1 -p 46002"
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
ecc_proxy = "Ecc:tcp -h 127.0.0.1 -p 46002"
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
ecc_proxy = "Ecc:tcp -h 127.0.0.1 -p 46002"
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
ecc_proxy = "Ecc:tcp -h 127.0.0.1 -p 46002"
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

    // -----------------------------------------------------------------
    // 016 で足した `[controller]` キー(SPEC §8.1 / §3.2)
    // -----------------------------------------------------------------

    /// `[controller]` セクションだけを差し替えた最小構成の TOML(`[[cobo]]` は 2 台)。
    fn toml_with_controller_section(geometry: &Path, controller_section: &str) -> String {
        format!(
            r#"
[system]
experiment = "ELITPC"
output_root = "/data/tpcdaq"
geometry = "{geometry}"

[[cobo]]
id = 0
data_sender_id = "CoBo[0]"

[[cobo]]
id = 1
data_sender_id = "CoBo[1]"

[decoder]
workers = 1

[root_sink]
snapshot_hz = 1.0
event_publish_hz = 20.0
build_timeout_ms = 1000

[monitor]
ws_listen = "0.0.0.0:9000"

{controller_section}
"#,
            geometry = geometry.display()
        )
    }

    /// 省略時は SPEC §3.2 の既定表がそのまま入る(receiver は `[[cobo]]` の id で 47110+k)。
    #[test]
    fn controller_016_keys_default_to_the_spec_port_table() {
        let geometry = make_temp_geometry_file();
        let toml_str = toml_with_controller_section(
            &geometry,
            "[controller]\npassphrase = \"change-me\"\n\
             ecc_proxy = \"Ecc:tcp -h 127.0.0.1 -p 46002\"\nconfig_id = \"default\"\n",
        );
        let config = parse(&toml_str).unwrap();

        assert_eq!(config.controller.log_pull_bind, "tcp://*:47005");
        assert_eq!(config.controller.eos_timeout_s, 5);
        // 033-E: 新設キー。省略時は 500 ms(SPEC §1.3 v1.12)。
        assert_eq!(config.controller.eos_quiesce_ms, 500);
        assert_eq!(config.controller.ui_dir, None);
        assert_eq!(
            config.controller.graw_writer_command,
            "tcp://127.0.0.1:47100"
        );
        assert_eq!(config.controller.decoder_command, "tcp://127.0.0.1:47101");
        assert_eq!(
            config.controller.receiver_commands,
            vec!["tcp://127.0.0.1:47110", "tcp://127.0.0.1:47111"]
        );
        assert_eq!(config.controller.ecc_command, "tcp://127.0.0.1:47200");
        assert_eq!(config.controller.router_ip, None);
        // 078 レーン B: 新設キー。省略時は両方 `None`(`router_ip` と同じ「既定は無い/
        // 呼び出し側で足す」流儀 —— controller.rs の `Controller::params_dir` 参照)。
        assert_eq!(config.controller.ecc_config_dir, None);
        assert_eq!(config.controller.params_dir, None);

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    /// すべて設定で上書きできる(非対称な値で取り違えを検出する)。
    #[test]
    fn controller_016_keys_are_overridable() {
        let geometry = make_temp_geometry_file();
        let toml_str = toml_with_controller_section(
            &geometry,
            r#"[controller]
rest_listen = "0.0.0.0:8090"
passphrase = "elitpc-pass"
ecc_proxy = "Ecc:tcp -h 10.0.0.2 -p 46002"
config_id = "elitpc"
log_pull_bind = "tcp://*:47105"
eos_timeout_s = 9
eos_quiesce_ms = 750
ui_dir = "/srv/tpcdaq-ui"
graw_writer_command = "tcp://10.0.0.3:47100"
decoder_command = "tcp://10.0.0.4:47101"
receiver_commands = ["tcp://10.0.0.5:47110", "tcp://10.0.0.6:47111"]
ecc_command = "tcp://10.0.0.7:47200"
router_ip = "10.0.0.1"
ecc_config_dir = "/srv/tpcdaq-ecc-config"
params_dir = "/srv/tpcdaq-params"
"#,
        );
        let config = parse(&toml_str).unwrap();

        assert_eq!(config.controller.log_pull_bind, "tcp://*:47105");
        assert_eq!(config.controller.eos_timeout_s, 9);
        // 非対称値(eos_timeout_s = 9 s と取り違えたら落ちる)。
        assert_eq!(config.controller.eos_quiesce_ms, 750);
        assert_eq!(
            config.controller.ui_dir,
            Some(PathBuf::from("/srv/tpcdaq-ui"))
        );
        assert_eq!(
            config.controller.graw_writer_command,
            "tcp://10.0.0.3:47100"
        );
        assert_eq!(config.controller.decoder_command, "tcp://10.0.0.4:47101");
        assert_eq!(
            config.controller.receiver_commands,
            vec!["tcp://10.0.0.5:47110", "tcp://10.0.0.6:47111"]
        );
        assert_eq!(config.controller.ecc_command, "tcp://10.0.0.7:47200");
        assert_eq!(config.controller.router_ip.as_deref(), Some("10.0.0.1"));
        assert_eq!(
            config.controller.ecc_config_dir,
            Some(PathBuf::from("/srv/tpcdaq-ecc-config"))
        );
        assert_eq!(
            config.controller.params_dir,
            Some(PathBuf::from("/srv/tpcdaq-params"))
        );

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    /// 本数がずれた `receiver_commands` は起動失敗(黙って一部の receiver を見失わない)。
    #[test]
    fn controller_receiver_commands_must_match_the_cobo_count() {
        let geometry = make_temp_geometry_file();
        let toml_str = toml_with_controller_section(
            &geometry,
            "[controller]\npassphrase = \"p\"\necc_proxy = \"x\"\nconfig_id = \"c\"\n\
             receiver_commands = [\"tcp://127.0.0.1:47110\"]\n",
        );
        let err = parse(&toml_str).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidController(_)), "{err}");

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    /// `eos_timeout_s = 0` は「常に強制 EOS」になり停止シーケンスが意味を失う。
    #[test]
    fn controller_eos_timeout_zero_is_rejected() {
        let geometry = make_temp_geometry_file();
        let toml_str = toml_with_controller_section(
            &geometry,
            "[controller]\npassphrase = \"p\"\necc_proxy = \"x\"\nconfig_id = \"c\"\n\
             eos_timeout_s = 0\n",
        );
        let err = parse(&toml_str).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidController(_)), "{err}");

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    /// `eos_quiesce_ms = 0` は「静止を一切待たずに即・強制 EOS」= 033 裁定が明示的に
    /// 不採用とした挙動。`eos_timeout_s = 0` と同じく起動時に拒否する(TODO/046-C)。
    #[test]
    fn controller_eos_quiesce_zero_is_rejected() {
        let geometry = make_temp_geometry_file();
        let toml_str = toml_with_controller_section(
            &geometry,
            "[controller]\npassphrase = \"p\"\necc_proxy = \"x\"\nconfig_id = \"c\"\n\
             eos_quiesce_ms = 0\n",
        );
        let err = parse(&toml_str).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidController(_)), "{err}");

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    #[test]
    fn controller_unknown_field_is_err() {
        let geometry = make_temp_geometry_file();
        let toml_str = toml_with_controller_section(
            &geometry,
            "[controller]\npassphrase = \"p\"\necc_proxy = \"x\"\nconfig_id = \"c\"\n\
             eos_timeout_ms = 5000\n",
        );
        let err = parse(&toml_str).unwrap_err();
        assert!(matches!(err, ConfigError::Parse(_)), "{err}");

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    // -----------------------------------------------------------------
    // ConfigId の 3 相化(SPEC §3.1 v1.13、TODO/042)
    // -----------------------------------------------------------------

    /// 文字列形は「3 相同値」の略記 —— 既存設定の意味は変わらない(後方互換の本体)。
    #[test]
    fn config_id_string_expands_to_the_same_id_in_all_three_phases() {
        let geometry = make_temp_geometry_file();
        let toml_str = toml_with_controller_section(
            &geometry,
            "[controller]\npassphrase = \"p\"\necc_proxy = \"x\"\nconfig_id = \"default\"\n",
        );
        let config = parse(&toml_str).unwrap();

        assert_eq!(config.controller.config_id, ConfigIds::same("default"));
        assert_eq!(config.controller.config_id.same_value(), Some("default"));

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    /// テーブル形は相ごとに別の id を持てる(実運用例: `describe = zCobo-ZC706`、
    /// `configure = pulser` —— TODO/038 レーン A の実測)。**3 値すべて非対称**にして
    /// 相の取り違えを検出する。
    #[test]
    fn config_id_table_sets_each_phase_independently() {
        let geometry = make_temp_geometry_file();
        let toml_str = toml_with_controller_section(
            &geometry,
            "[controller]\npassphrase = \"p\"\necc_proxy = \"x\"\n\
             config_id = { describe = \"zCobo-ZC706\", prepare = \"prep-xcfg\", \
             configure = \"pulser\" }\n",
        );
        let config = parse(&toml_str).unwrap();
        let ids = &config.controller.config_id;

        assert_eq!(ids.describe, "zCobo-ZC706");
        assert_eq!(ids.prepare, "prep-xcfg");
        assert_eq!(ids.configure, "pulser");
        // 非同値 = 略記に畳めない(logbook `config_ids` を出す分岐条件、SPEC §9.2)。
        assert_eq!(ids.same_value(), None);
        // アクション名 → 相の対応(SPEC §8.2)。id を使わないアクションは None。
        assert_eq!(ids.for_action("describe"), Some("zCobo-ZC706"));
        assert_eq!(ids.for_action("prepare"), Some("prep-xcfg"));
        assert_eq!(ids.for_action("configure"), Some("pulser"));
        assert_eq!(ids.for_action("start"), None);

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    /// 相が欠けたテーブルは起動失敗。空 id を黙って ECC へ投げない
    /// (CLAUDE.md「silent failure を作らない」)。
    #[test]
    fn config_id_table_missing_a_phase_is_rejected() {
        let geometry = make_temp_geometry_file();
        let toml_str = toml_with_controller_section(
            &geometry,
            "[controller]\npassphrase = \"p\"\necc_proxy = \"x\"\n\
             config_id = { describe = \"a\", configure = \"b\" }\n",
        );
        let err = parse(&toml_str).unwrap_err();
        assert!(matches!(err, ConfigError::Parse(_)), "{err}");

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    /// 相が同値なら文字列形とテーブル形は**同じ設定**(表現だけの違い)。
    #[test]
    fn config_id_table_with_equal_phases_is_the_string_form() {
        let geometry = make_temp_geometry_file();
        let toml_str = toml_with_controller_section(
            &geometry,
            "[controller]\npassphrase = \"p\"\necc_proxy = \"x\"\n\
             config_id = { describe = \"pulser\", prepare = \"pulser\", \
             configure = \"pulser\" }\n",
        );
        let config = parse(&toml_str).unwrap();

        assert_eq!(config.controller.config_id, ConfigIds::same("pulser"));
        assert_eq!(config.controller.config_id.same_value(), Some("pulser"));
        // 既存テストと同じ「文字列との比較」がそのまま通る(後方互換の道具)。
        assert_eq!(config.controller.config_id, "pulser");

        let _ = std::fs::remove_dir_all(geometry.parent().unwrap());
    }

    /// SPEC §3.2「controller ログ投稿 PULL bind = 47005 / ecc-bridge REP = 47200 /
    /// コンポーネント REP = graw-writer 47100・decoder 47101・receiver k = 47110+k」。
    #[test]
    fn controller_endpoint_defaults_match_spec_3_2() {
        assert_eq!(DEFAULT_CONTROLLER_LOG_PULL_BIND, "tcp://*:47005");
        assert_eq!(DEFAULT_ECC_COMMAND_ENDPOINT, "tcp://127.0.0.1:47200");
        assert_eq!(
            DEFAULT_GRAW_WRITER_COMMAND_ENDPOINT,
            "tcp://127.0.0.1:47100"
        );
        assert_eq!(DEFAULT_DECODER_COMMAND_ENDPOINT, "tcp://127.0.0.1:47101");
        assert_eq!(
            default_receiver_command_endpoint(0),
            "tcp://127.0.0.1:47110"
        );
        assert_eq!(
            default_receiver_command_endpoint(3),
            "tcp://127.0.0.1:47113"
        );
        // bind 側(コンポーネントが開く口)と connect 側(controller が叩く口)でポート一致。
        assert!(GRAW_WRITER_COMMAND_LISTEN.ends_with(":47100"));
        assert!(DECODER_COMMAND_LISTEN.ends_with(":47101"));
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

    // -----------------------------------------------------------------
    // `[slowctl]` / `[monitor] slow_sub`(SPEC §3.1 v1.31、TODO/079 Phase 2)
    // -----------------------------------------------------------------

    /// **既存 TOML が無変更で読める**(セクションごと省略可)—— 079 Phase 2 の絶対条件。
    #[test]
    fn a_config_without_a_slowctl_section_still_parses_and_has_no_slow_wiring() {
        let geometry = make_temp_geometry_file();
        let config = parse(&minimal_toml(&geometry, "")).unwrap();
        assert!(
            config.slowctl.is_none(),
            "[slowctl] 省略 = slowctl を走らせない"
        );
        assert!(
            config.monitor.slow_sub.is_none(),
            "slow_sub 省略 = 購読しない(status JSON は従来と同一)"
        );
    }

    /// SPEC §3.1 の `[slowctl]` 例がそのまま読める。
    #[test]
    fn the_spec_3_1_slowctl_example_parses_into_one_bronkhorst_device() {
        let geometry = make_temp_geometry_file();
        let extra = r#"
[slowctl]
status_pub = "tcp://*:47006"
poll_hz = 1.0
log_every_s = 60
[[slowctl.device]]
type = "bronkhorst_flowbus"
name = "gas"
port = "/dev/serial/by-id/usb-FTDI_xxx-if00-port0"
nodes = "scan"
expect_serial = ""
alarms = [{ node = 4, param = "fmeasure", lo = 90.0, hi = 110.0 }]
"#;
        let config = parse(&minimal_toml(&geometry, extra)).unwrap();
        let slowctl = config.slowctl.unwrap();
        assert_eq!(slowctl.status_pub, "tcp://*:47006");
        assert_f64_eq(slowctl.poll_hz, 1.0);
        assert_eq!(slowctl.log_every_s, 60);
        // LogPost の宛先は省略 = controller の既定 PULL(SPEC §3.2 の 47005)。
        assert_eq!(slowctl.log_push, DEFAULT_CONTROLLER_LOG_PUSH_ENDPOINT);

        assert_eq!(slowctl.device.len(), 1);
        let device = &slowctl.device[0];
        assert_eq!(device.device_type, "bronkhorst_flowbus");
        assert_eq!(device.name, "gas");
        assert_eq!(
            device.port.as_deref(),
            Some("/dev/serial/by-id/usb-FTDI_xxx-if00-port0")
        );
        assert_eq!(device.nodes.resolve().unwrap()[0], 128);
        assert_eq!(device.expect_serial, "");
        assert_eq!(device.alarms.len(), 1);
        assert_eq!(device.alarms[0].node, Some(4));
        assert_eq!(device.alarms[0].param, "fmeasure");
        assert_f64_eq(device.alarms[0].lo, 90.0);
        assert_f64_eq(device.alarms[0].hi, 110.0);
    }

    /// `[monitor] slow_sub` は SPEC §3.2 の 47006 を指す任意キー。
    #[test]
    fn the_monitor_can_be_pointed_at_the_slowctl_pub() {
        let geometry = make_temp_geometry_file();
        let toml = minimal_toml(&geometry, "").replace(
            "[monitor]\nws_listen",
            "[monitor]\nslow_sub = \"tcp://127.0.0.1:47006\"\nws_listen",
        );
        let config = parse(&toml).unwrap();
        assert_eq!(
            config.monitor.slow_sub.as_deref(),
            Some("tcp://127.0.0.1:47006")
        );
    }

    /// TODO/079 §2.1: 取り違え防止のため `port` は by-id / by-path が望ましい。
    /// **拒否はしない**(Mac 開発機に by-id は無い)—— warn の判定条件だけを固定する。
    #[test]
    fn only_by_id_and_by_path_count_as_a_stable_serial_port_name() {
        assert!(is_stable_serial_port(
            "/dev/serial/by-id/usb-FTDI_xxx-if00-port0"
        ));
        assert!(is_stable_serial_port(
            "/dev/serial/by-path/pci-0000:00-usb-0:1"
        ));
        assert!(!is_stable_serial_port("/dev/ttyUSB0"));
        assert!(!is_stable_serial_port("/dev/tty.usbserial-A5XK3RJT"));
    }

    /// 不安定なポート名でも**起動できる**(警告だけ)。
    #[test]
    fn an_unstable_port_name_is_a_warning_not_a_startup_failure() {
        let geometry = make_temp_geometry_file();
        let extra = r#"
[slowctl]
status_pub = "tcp://*:47006"
[[slowctl.device]]
type = "bronkhorst_flowbus"
name = "gas"
port = "/dev/ttyUSB0"
"#;
        let config = parse(&minimal_toml(&geometry, extra)).unwrap();
        let slowctl = config.slowctl.unwrap();
        // 省略したキーには既定が入る(SPEC §3.1)。
        assert_f64_eq(slowctl.poll_hz, DEFAULT_SLOWCTL_POLL_HZ);
        assert_eq!(slowctl.log_every_s, DEFAULT_SLOWCTL_LOG_EVERY_S);
        assert_eq!(
            slowctl.device[0].nodes,
            crate::slowctl::NodesSpec::default()
        );
        assert!(slowctl.device[0].alarms.is_empty());
    }

    /// `deny_unknown_fields` は `[slowctl]` にも効く(綴り間違いを黙って無視しない)。
    #[test]
    fn an_unknown_slowctl_key_is_rejected() {
        let geometry = make_temp_geometry_file();
        let extra = r#"
[slowctl]
status_pub = "tcp://*:47006"
pool_hz = 1.0
[[slowctl.device]]
type = "fake"
name = "gas"
port = "script.txt"
"#;
        assert!(matches!(
            parse(&minimal_toml(&geometry, extra)),
            Err(ConfigError::Parse(_))
        ));

        let bad_device = r#"
[slowctl]
[[slowctl.device]]
type = "fake"
name = "gas"
prot = "script.txt"
"#;
        assert!(matches!(
            parse(&minimal_toml(&geometry, bad_device)),
            Err(ConfigError::Parse(_))
        ));
    }

    /// 知らない `type` は起動失敗(半端な既定で走らない — SPEC §3.2)。
    #[test]
    fn an_unknown_device_type_is_a_startup_failure() {
        let geometry = make_temp_geometry_file();
        let extra = r#"
[slowctl]
[[slowctl.device]]
type = "bronkhorst_flowbuss"
name = "gas"
port = "/dev/ttyUSB0"
"#;
        assert!(matches!(
            parse(&minimal_toml(&geometry, extra)),
            Err(ConfigError::InvalidSlowctl(_))
        ));
    }

    /// シリアル系デバイスに `port` が無いのも起動失敗(静かに何も読まない状態を作らない)。
    #[test]
    fn a_serial_device_without_a_port_is_a_startup_failure() {
        let geometry = make_temp_geometry_file();
        let extra = r#"
[slowctl]
[[slowctl.device]]
type = "bronkhorst_flowbus"
name = "gas"
"#;
        assert!(matches!(
            parse(&minimal_toml(&geometry, extra)),
            Err(ConfigError::InvalidSlowctl(_))
        ));
    }

    /// TODO/121 §D: `status_ignore` は UNV / OVV だけ許可。それ以外は起動失敗。
    #[test]
    fn status_ignore_accepts_only_unv_and_ovv() {
        let geometry = make_temp_geometry_file();
        let with = |flags: &str| {
            let extra = format!(
                r#"
[slowctl]
[[slowctl.device]]
type = "hivolta"
name = "hv"
addr = "192.0.2.1:1470"
status_ignore = [{{ ch = 0, flags = [{flags}] }}]
"#
            );
            parse(&minimal_toml(&geometry, &extra))
        };
        let config = with(r#""UNV", "OVV""#).unwrap();
        assert_eq!(config.slowctl.unwrap().device[0].status_ignore.len(), 1);
        for bad in [r#""TRIP""#, r#""unv""#, r#""UNV", "OVC""#] {
            assert!(
                matches!(with(bad), Err(ConfigError::InvalidSlowctl(_))),
                "{bad} must be rejected"
            );
        }
        let bad_ch = r#"
[slowctl]
[[slowctl.device]]
type = "hivolta"
name = "hv"
addr = "192.0.2.1:1470"
status_ignore = [{ ch = 8, flags = ["UNV"] }]
"#;
        assert!(matches!(
            parse(&minimal_toml(&geometry, bad_ch)),
            Err(ConfigError::InvalidSlowctl(_))
        ));
    }

    /// TODO/119: `poll_s` は省略可、指定すれば保持される。0・負・NaN は起動失敗。
    #[test]
    fn a_device_poll_s_is_optional_and_must_be_positive() {
        let geometry = make_temp_geometry_file();
        let with = |poll: &str| {
            let extra = format!(
                "[slowctl]\n[[slowctl.device]]\ntype = \"hivolta\"\nname = \"hv\"\n\
                 addr = \"127.0.0.1:1470\"\n{poll}\n"
            );
            parse(&minimal_toml(&geometry, &extra))
        };
        let config = with("").unwrap();
        assert_eq!(config.slowctl.unwrap().device[0].poll_s, None);
        let config = with("poll_s = 5.0").unwrap();
        assert_eq!(config.slowctl.unwrap().device[0].poll_s, Some(5.0));
        for bad in ["poll_s = 0.0", "poll_s = -1.0", "poll_s = nan"] {
            assert!(
                matches!(with(bad), Err(ConfigError::InvalidSlowctl(_))),
                "{bad}"
            );
        }
    }

    /// TODO/089 レビュー: `hivolta` は `addr` だけでも起動できる(TCP 優先経路)。
    #[test]
    fn a_hivolta_device_with_only_an_addr_is_accepted() {
        let geometry = make_temp_geometry_file();
        let extra = r#"
[slowctl]
[[slowctl.device]]
type = "hivolta"
name = "hv"
addr = "192.168.1.50:1470"
"#;
        let config = parse(&minimal_toml(&geometry, extra)).unwrap();
        let device = &config.slowctl.unwrap().device[0];
        assert_eq!(device.addr.as_deref(), Some("192.168.1.50:1470"));
        assert_eq!(device.port, None);
    }

    /// `hivolta` は `port` だけでも起動できる(USB VCP フォールバック経路)。
    #[test]
    fn a_hivolta_device_with_only_a_port_is_accepted() {
        let geometry = make_temp_geometry_file();
        let extra = r#"
[slowctl]
[[slowctl.device]]
type = "hivolta"
name = "hv"
port = "/dev/serial/by-id/usb-CAEN_xxx-if00-port0"
"#;
        let config = parse(&minimal_toml(&geometry, extra)).unwrap();
        let device = &config.slowctl.unwrap().device[0];
        assert_eq!(device.addr, None);
        assert_eq!(
            device.port.as_deref(),
            Some("/dev/serial/by-id/usb-CAEN_xxx-if00-port0")
        );
    }

    /// `hivolta` に `addr` も `port` も無ければ起動失敗(黙って何も読まないデバイスを作らない)。
    #[test]
    fn a_hivolta_device_without_addr_or_port_is_a_startup_failure() {
        let geometry = make_temp_geometry_file();
        let extra = r#"
[slowctl]
[[slowctl.device]]
type = "hivolta"
name = "hv"
"#;
        match parse(&minimal_toml(&geometry, extra)) {
            Err(ConfigError::InvalidSlowctl(message)) => {
                assert!(message.contains("addr"), "message = {message:?}");
                assert!(message.contains("port"), "message = {message:?}");
            }
            other => panic!("expected InvalidSlowctl, got {other:?}"),
        }
    }

    /// リポに置いた見本 TOML が**実際に読める**こと(腐った見本を配らない)。
    /// ジオメトリだけは実ファイルが要るので一時ファイルに差し替える。
    #[test]
    fn the_example_config_in_the_repository_parses() {
        let geometry = make_temp_geometry_file();
        let text = std::fs::read_to_string("config/slowctl_example.toml")
            .expect("config/slowctl_example.toml");
        let text = text.replace(
            "geometry = \"config/geometry_mini_eTPC.dat\"",
            &format!("geometry = \"{}\"", geometry.display()),
        );
        let config = parse(&text).unwrap();
        assert_eq!(
            config.monitor.slow_sub.as_deref(),
            Some("tcp://127.0.0.1:47006")
        );
        let slowctl = config.slowctl.unwrap();
        assert_eq!(slowctl.status_pub, DEFAULT_SLOWCTL_STATUS_PUB);
        assert_eq!(slowctl.device[0].device_type, "bronkhorst_flowbus");
        assert_eq!(slowctl.device[0].alarms.len(), 1);
    }

    // -----------------------------------------------------------------
    // `[slowctl.influx]`(TODO/122)
    // -----------------------------------------------------------------

    /// どこにも無いトークンファイル。**config はトークンを読まない**(読むのは slowctl の
    /// 起動時だけ —— 監視系の不備で controller / monitor を起動不能にしない)ので、
    /// 設定のテストはこのパスで足りる。
    const ABSENT_TOKEN_FILE: &str = "/nonexistent/tpcdaq-122/influx.token";

    /// `[slowctl]` + hv 1 台 + `influx_body`(`[slowctl.influx]` の中身)の TOML。
    fn slowctl_with_influx(geometry: &Path, influx_body: &str) -> String {
        let extra = format!(
            "[slowctl]\n[[slowctl.device]]\ntype = \"hivolta\"\nname = \"hv\"\n\
             addr = \"127.0.0.1:1470\"\n[slowctl.influx]\n{influx_body}\n"
        );
        minimal_toml(geometry, &extra)
    }

    /// url / org / bucket / token_file と、`extra`(任意キー)からなる `[slowctl.influx]` の中身。
    fn influx_body(url: &str, org: &str, bucket: &str, token_file: &str, extra: &str) -> String {
        format!(
            "url = \"{url}\"\norg = \"{org}\"\nbucket = \"{bucket}\"\n\
             token_file = \"{token_file}\"\n{extra}"
        )
    }

    /// `[slowctl.influx]` が無ければ書かない(= 今と同じ)。既存の `[slowctl]` は無変更で読める。
    #[test]
    fn a_slowctl_without_an_influx_section_writes_nothing_to_influxdb() {
        let geometry = make_temp_geometry_file();
        let extra = "[slowctl]\n[[slowctl.device]]\ntype = \"hivolta\"\nname = \"hv\"\n\
                     addr = \"127.0.0.1:1470\"\n";
        let config = parse(&minimal_toml(&geometry, extra)).unwrap();
        assert_eq!(config.slowctl.unwrap().influx, None);
    }

    /// 必須 4 キーだけ書けば、残り 3 キーには発注書 §4 の既定が入る。
    #[test]
    fn an_influx_section_with_only_the_required_keys_gets_the_defaults() {
        let geometry = make_temp_geometry_file();
        let body = influx_body(
            "http://127.0.0.1:8086",
            "elinp",
            "slowctl",
            "/opt/tpcdaq/influx.token",
            "",
        );
        let config = parse(&slowctl_with_influx(&geometry, &body)).unwrap();
        let influx = config.slowctl.unwrap().influx.unwrap();
        assert_eq!(influx.url, "http://127.0.0.1:8086");
        assert_eq!(influx.org, "elinp");
        assert_eq!(influx.bucket, "slowctl");
        assert_eq!(influx.token_file, PathBuf::from("/opt/tpcdaq/influx.token"));
        // 発注書 §4 の表: batch_ms 1000 / buffer_max_points 100000 / timeout_ms 2000。
        assert_eq!(influx.batch_ms, 1000);
        assert_eq!(influx.buffer_max_points, 100_000);
        assert_eq!(influx.timeout_ms, 2000);
    }

    /// 任意キーを書けばその値が入る。
    #[test]
    fn the_optional_influx_keys_override_the_defaults() {
        let geometry = make_temp_geometry_file();
        let body = influx_body(
            "http://10.0.0.5:8086/",
            "o",
            "b",
            ABSENT_TOKEN_FILE,
            "batch_ms = 250\nbuffer_max_points = 7\ntimeout_ms = 900",
        );
        let influx = parse(&slowctl_with_influx(&geometry, &body))
            .unwrap()
            .slowctl
            .unwrap()
            .influx
            .unwrap();
        assert_eq!(influx.url, "http://10.0.0.5:8086/");
        assert_eq!(influx.batch_ms, 250);
        assert_eq!(influx.buffer_max_points, 7);
        assert_eq!(influx.timeout_ms, 900);
    }

    /// 必須キー(url / org / bucket / token_file)が 1 つでも欠ければ起動失敗。
    #[test]
    fn an_influx_section_missing_a_required_key_is_rejected() {
        let geometry = make_temp_geometry_file();
        let all = [
            "url = \"http://127.0.0.1:8086\"".to_string(),
            "org = \"elinp\"".to_string(),
            "bucket = \"slowctl\"".to_string(),
            format!("token_file = \"{ABSENT_TOKEN_FILE}\""),
        ];
        for missing in 0..all.len() {
            let body: Vec<&str> = all
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != missing)
                .map(|(_, line)| line.as_str())
                .collect();
            let result = parse(&slowctl_with_influx(&geometry, &body.join("\n")));
            assert!(
                matches!(result, Err(ConfigError::Parse(_))),
                "missing {:?} must be rejected, got {result:?}",
                all[missing]
            );
        }
        // 綴り間違いも黙って無視しない(deny_unknown_fields)。
        let typo = format!("{}\nbatch_msec = 10", all.join("\n"));
        assert!(matches!(
            parse(&slowctl_with_influx(&geometry, &typo)),
            Err(ConfigError::Parse(_))
        ));
    }

    /// **トークンファイルが無くても `parse` は通る**(レビュー 2026-10-12 (a))。
    ///
    /// controller / monitor も同じ daq.toml を読むので、監視系のトークンの不備で DAQ 全体を
    /// 起動不能にしない(never-stop、保存と監視の分離)。読めないことは slowctl の起動だけが
    /// 失敗として出す(`tests/slowctl_integration.rs`)。config が見るのは「キーがあって空でない」まで。
    #[test]
    fn a_missing_token_file_does_not_stop_the_config_from_parsing() {
        let geometry = make_temp_geometry_file();
        let body = influx_body("http://127.0.0.1:8086", "o", "b", ABSENT_TOKEN_FILE, "");
        let config = parse(&slowctl_with_influx(&geometry, &body)).unwrap();
        assert_eq!(
            config.slowctl.unwrap().influx.unwrap().token_file,
            PathBuf::from(ABSENT_TOKEN_FILE)
        );

        // 空のパスは書き損じ —— これは config で弾く。
        let empty = influx_body("http://127.0.0.1:8086", "o", "b", "", "");
        match parse(&slowctl_with_influx(&geometry, &empty)) {
            Err(ConfigError::InvalidSlowctl(message)) => {
                assert!(message.contains("token_file"), "message = {message:?}");
            }
            other => panic!("expected InvalidSlowctl, got {other:?}"),
        }
    }

    /// url は `http://` で始まること。
    #[test]
    fn an_influx_url_without_an_http_scheme_is_rejected() {
        let geometry = make_temp_geometry_file();
        for bad in ["127.0.0.1:8086", "tcp://127.0.0.1:8086", "", "HTTP//x"] {
            let body = influx_body(bad, "o", "b", ABSENT_TOKEN_FILE, "");
            match parse(&slowctl_with_influx(&geometry, &body)) {
                Err(ConfigError::InvalidSlowctl(message)) => {
                    assert!(message.contains("url"), "{bad:?}: message = {message:?}");
                }
                other => panic!("{bad:?} must be rejected, got {other:?}"),
            }
        }
    }

    /// `https://` は検証で拒否する(レビュー 2026-10-12 (b))。HTTP クライアントは TLS を
    /// 持たない(ureq の default-features を切った)ので、通すと実行時に必ず失敗し続ける。
    #[test]
    fn an_https_influx_url_is_rejected_because_tls_is_not_built_in() {
        let geometry = make_temp_geometry_file();
        let body = influx_body("https://127.0.0.1:8086", "o", "b", ABSENT_TOKEN_FILE, "");
        match parse(&slowctl_with_influx(&geometry, &body)) {
            Err(ConfigError::InvalidSlowctl(message)) => {
                assert!(
                    message.contains("TLS は未対応、localhost の http を使う"),
                    "message = {message:?}"
                );
            }
            other => panic!("https must be rejected, got {other:?}"),
        }
    }

    /// 0 の batch_ms / buffer_max_points / timeout_ms と空の org / bucket は起動失敗
    /// (半端な値で走らない — SPEC §3.2)。
    #[test]
    fn zero_or_empty_influx_settings_are_rejected() {
        let geometry = make_temp_geometry_file();
        let base = |org: &str, bucket: &str, extra: &str| {
            influx_body(
                "http://127.0.0.1:8086",
                org,
                bucket,
                ABSENT_TOKEN_FILE,
                extra,
            )
        };
        for body in [
            base("o", "b", "batch_ms = 0"),
            base("o", "b", "buffer_max_points = 0"),
            base("o", "b", "timeout_ms = 0"),
            base("", "b", ""),
            base("o", "", ""),
        ] {
            assert!(
                matches!(
                    parse(&slowctl_with_influx(&geometry, &body)),
                    Err(ConfigError::InvalidSlowctl(_))
                ),
                "{body}"
            );
        }
    }

    /// SPEC §3.2: slowctl の Batch source_id は 200(旧 psu)。
    #[test]
    fn the_slowctl_source_id_is_two_hundred() {
        assert_eq!(SLOWCTL_SOURCE_ID, 200);
        assert_eq!(DEFAULT_SLOWCTL_STATUS_PUB, "tcp://*:47006");
        assert_eq!(
            DEFAULT_CONTROLLER_LOG_PUSH_ENDPOINT,
            "tcp://127.0.0.1:47005"
        );
    }
}
