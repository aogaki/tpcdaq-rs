//! controller — REST + run シーケンス + ログブック統合(SPEC §8.1 / §1.3 / §2.6 / §9)。
//!
//! # 立ち位置
//!
//! controller は §1.3 の状態機械の**外**にいるオーケストレータで、自分自身は
//! `Idle / Starting / Running / Stopping`([`Phase`])の 4 相しか持たない。各コンポーネントの
//! 状態機械を動かすのは JSON REQ/REP(§2.6)、ECC を動かすのは ecc-bridge の JSON REQ/REP
//! (§8.2)、外の世界への口は REST(§8.1)だけである。
//!
//! # 三層構造(テスト容易性のための切り方)
//!
//! 1. [`Transport`] — 「外に何かを送る」1 枚の trait(コンポーネントへのコマンド / ecc-bridge /
//!    待ち時間)。実物は [`ZmqTransport`]、テストはコマンド列を記録するモック。
//! 2. [`Sequencer`] — §1.3 の run 開始 / 停止 / 中止シーケンスの**純ロジック**。IO は
//!    [`Transport`] 越しだけ、時刻も `Transport::sleep` 経由なので、モックで全分岐を機械照合できる。
//! 3. [`Controller`] + axum — REST・操作権・監査・ログブック書き込みの配線。
//!
//! # 連続 run(034 — ユーザー裁定「毎 run 完全にリセットして一からやり直す」/ 036)
//!
//! run 開始は **必ず ECC を `Idle` へ戻すところから**始まる。前の run の `ecc stop` は ECC を
//! `Ready` に置くが、`describe` は `Off/Idle/Described` からしか許されないので、戻さないと
//! 2 本目が必ず順序違反で落ちる。
//!
//! ただし実 ECC の `reset` は `EV_UNDO` = **1 段戻すだけ**で、`Active`(Ready/Running/Paused)
//! からは遷移が無く**無音で無視**される(`GetBench/src/get/rc/BackEnd.cpp:924` / `:250-270`、
//! `StateMachine/src/dhsm/Engine.cpp:334-352`)。よって `reset` を 1 発撃つのではなく、
//! **現在の状態を見て必要な段数だけ歩き戻す**(036、SPEC §1.3 v1.12):
//! `Running/Paused --stop--> Ready --breakup--> Prepared --reset--> Described --reset--> Idle`。
//! `Idle` に着けなければ **run を始めない**([`Sequencer::walk_ecc_back_to_idle`])。
//!
//! コンポーネントも同じく Idle まで歩き戻してから始める(113、SPEC §1.3 v1.37): 停止の
//! 畳む段の `Reset` が届かないと、そのコンポーネントは `Configured` のまま残り、次の
//! `Configure` が断られ続ける。`GetStatus` で Idle でないものにだけ `Reset` を撃ち、跡を
//! audit `run/start.component_walk_back` に残す([`Sequencer::walk_components_back_to_idle`])。
//! 全員が `Running`(run 中に controller だけが再起動した姿)なら、`Reset` の前に停止経路の
//! 2 段そのもの(自然 EOS か受信静止 → 閉じていなければ receiver `Stop` → EOS 待ち → 下流
//! `Stop`)で前の run を閉じる(114 —— `Reset` は送出待ちを打ち切るので、送出待ちがあると
//! EOS ごと打ち切られる。競争に頼らない)。
//!
//! あわせて、`Arm` は前の run が閉じたソケットの解放待ちで一時的に bind に失敗しうるので
//! 指数バックオフで粘り(粘った実績は audit へ)、run 開始が失敗したときは
//! **`run_start` をログブックへ書く前に限り** 採番を巻き戻す。
//!
//! # 巻き戻し(SPEC §1.3「半端な Running を残さない」)
//!
//! 開始シーケンスのどの段で失敗しても、**実行済みのコンポーネントだけ**を上流から畳む。
//! `Stop` は `Running` からしか合法でない(§1.3 の遷移表: `Armed`/`Configured` からの `Stop` は
//! 不許可)ので、`Stop` に続けて `Reset` を送り、必ず `Idle` まで戻す。ecc start 前に失敗した
//! 場合はまだ CoBo からデータが 1 バイトも流れていない(= 閉じるべき run が存在しない)ので、
//! この `Reset` は「run クローズ前に Reset しない」規則(v1.6)に抵触しない。
//!
//! # 中止(SPEC §1.3 v1.6)
//!
//! 中止も必ず EOS で閉じる: `ecc stop`(不達でも続行)→ receiver へ `Stop`(= 強制 EOS)→
//! EOS がチェーンを流れて run がクローズしたことを graw-writer / decoder の `GetStatus` で
//! 確認 → **その後で**各コンポーネントの `Stop` / `Reset`。run クローズ前に decoder を
//! `Reset` しない(送出打ち切りの seq ギャップが root-sink を fatal 死させるため)。

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use axum::extract::{Path as UrlPath, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::{broadcast, oneshot};
use tower_http::services::{ServeDir, ServeFile};
use tracing::{error, info, warn};

use crate::command::{Command, CommandResponse, ComponentState, RunConfig};
use crate::config::{Config, ConfigIds};
use crate::geometry::Geometry;
use crate::logbook::{
    CoboListen, Counters, Geometry as GeometryRecord, LogbookRecord, LogbookWriter, OutputFile,
};

// ---------------------------------------------------------------------
// 定数
// ---------------------------------------------------------------------

/// コンポーネントへの 1 コマンドの待ち時間(SPEC §2.6「タイムアウト付き」)。
/// 相手が居ない・固まっているときに run シーケンスが永久に止まらないための上限。
pub const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

/// ecc-bridge への 1 リクエストの待ち時間。実機の `describe` / `configure` は
/// FPGA 設定を含むため秒単位で掛かる(GET の運用実績)。コンポーネントより長く取る。
pub const DEFAULT_ECC_TIMEOUT: Duration = Duration::from_secs(60);

/// `GET /api/status` の 1 コンポーネントあたりの待ち時間。
/// 死んだコンポーネントが 1 個あるだけで UI が固まらないよう、シーケンス用より短くする。
pub const DEFAULT_STATUS_TIMEOUT: Duration = Duration::from_secs(1);

/// EOS 伝播の観測間隔(SPEC §1.3 の EOS 待ち)。
pub const DEFAULT_EOS_POLL: Duration = Duration::from_millis(200);

/// `Arm` リトライの初回バックオフ(034 = 論点 B)。以降は試行ごとに倍。
///
/// decoder / graw-writer の PULL は固定ポート bind なのに、前の run の `Reset` で閉じた
/// ソケットは **libzmq の close が非同期**なので即座には空かない。直後の `Arm` は
/// `Address already in use` で弾かれる(030 実測: 3 回に 2 回、3 試行 = **0.118–0.121 s** で成功)。
pub const ARM_RETRY_BACKOFF: Duration = Duration::from_millis(20);

/// `Arm` の最大試行回数(初回 + リトライ 5 回)。
///
/// 眠る合計 = 20+40+80+160+320 = **620 ms**(最後の試行の後は眠らない)。030 の実測所要
/// 0.12 s 級に対して約 5 倍の余裕で、「1 秒級で足りる」という見立ての範囲に収まる。
/// **上限を無くさない**のは、bind 失敗が本当に恒久的(他プロセスがポートを握っている)な
/// ときに run 開始が永久に返らないのを避けるため。
pub const ARM_MAX_ATTEMPTS: u32 = 6;

/// ECC を `Idle` へ歩き戻すのに要する**最大段数**(036)。
///
/// 実 ECC の SM で `Idle` から一番遠いのは `Running` / `Paused` で、そこからは
/// `stop`(→ Ready)→ `breakup`(→ Prepared)→ `reset`(→ Described)→ `reset`(→ Idle)の
/// **4 段**。上限を置くのは、状態が期待どおり動かないときに run 開始が永久に回らないため。
const ECC_WALK_BACK_MAX_STEPS: usize = 4;

/// ログ投稿 PULL の recv タイムアウト(ms)。停止合図を見るための目覚まし。
const LOG_PULL_POLL_MS: i32 = 200;

/// ログブックの `actor`(SPEC §9.1「書き手は controller ただ一人」)。
const ACTOR: &str = "controller";

// ---------------------------------------------------------------------
// 起動パラメタ
// ---------------------------------------------------------------------

/// DataLinkSet と run_start に必要な CoBo 1 台分の情報。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoboSpec {
    pub id: u32,
    /// 設定上の listen(run_start レコードに載る。実際の bind ポートは Arm 応答から取る)。
    pub listen: String,
    /// ECC の NodeId 表記そのまま(`CoBo[0]`、SPEC §8.2 の実機の罠)。
    pub data_sender_id: String,
}

/// コンポーネントの種別。receiver だけが CoBo に紐づく。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentKind {
    GrawWriter,
    Decoder,
    Receiver { cobo_id: u32 },
}

/// 1 コンポーネントのコマンド REP 接続先(SPEC §3.2 の表)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentEndpoint {
    /// ログ・status・監査に出す名前(`graw-writer` / `decoder` / `receiver0`)。
    pub name: String,
    pub endpoint: String,
    pub kind: ComponentKind,
}

/// controller 1 プロセス分の起動パラメタ(= 設定の解決結果)。
///
/// `components` は**下流から上流の順**(graw-writer → decoder → receivers)で持つ。
/// 開始シーケンスはこの順、停止シーケンスは逆順に進む(SPEC §1.3)。
/// monitor は P3 後波なのでまだ並ばない。
#[derive(Debug, Clone, PartialEq)]
pub struct ControllerParams {
    pub rest_listen: String,
    pub passphrase: String,
    pub log_pull_bind: String,
    pub ui_dir: Option<PathBuf>,
    /// ECC の ConfigId(describe / prepare / configure の 3 相、SPEC §8.2 v1.13)。
    /// 設定が文字列形なら 3 相同値が入っている。
    pub config_ids: ConfigIds,
    pub output_root: PathBuf,
    pub geometry_path: PathBuf,
    pub cobos: Vec<CoboSpec>,
    pub components: Vec<ComponentEndpoint>,
    pub ecc_endpoint: String,
    /// DataLinkSet の `router_ip`。`None` なら receiver の実 bind アドレスから導く。
    pub router_ip: Option<String>,
    /// 078 レーン B §10: ECC の config repo(生成 xcfg の出力先 + テンプレートの読み先)。
    /// `None` なら params 系 REST は 503(§3.1 と同じ意味、既定は無い)。
    pub ecc_config_dir: Option<PathBuf>,
    /// 078 レーン B §10: パラメータ JSON の保存先。`None` なら
    /// `<output_root>/params`([`Controller::params_dir`]で解決)。
    pub params_dir: Option<PathBuf>,
    /// EOS 待ちの**ハード上限**(両段。SPEC §1.3)。
    pub eos_timeout: Duration,
    /// 停止第一段の受信静止(quiesce)判定時間(SPEC §1.3 v1.12、TODO/033-E)。
    /// 全 receiver の受信バイト数がこの時間だけ不変なら在飛データを飲み切ったとみなす。
    pub eos_quiesce: Duration,
    pub eos_poll: Duration,
    pub command_timeout: Duration,
    pub ecc_timeout: Duration,
    pub status_timeout: Duration,
    /// 124(SPEC §3.1 v1.42): slowctl の power REP への REQ 接続先。`None` なら
    /// `POST /api/slow/power` は 503(opt-in — 作動の経路を既定で開かない)。
    pub slow_cmd: Option<String>,
    /// 124: `[[slowctl.device]]` の (name, type)、設定の順。power の宛先と 400 の判定に使う
    /// (power を持たない型も載る —— 「その機器に電源スイッチは無い」と言い分けるため)。
    pub slow_devices: Vec<(String, String)>,
}

impl ControllerParams {
    /// 設定から起動パラメタを解決する(SPEC §3.1 `[controller]` + §3.2 の既定ポート表)。
    pub fn from_config(config: &Config) -> Self {
        let mut components = vec![
            ComponentEndpoint {
                name: "graw-writer".to_string(),
                endpoint: config.controller.graw_writer_command.clone(),
                kind: ComponentKind::GrawWriter,
            },
            ComponentEndpoint {
                name: "decoder".to_string(),
                endpoint: config.controller.decoder_command.clone(),
                kind: ComponentKind::Decoder,
            },
        ];
        // `receiver_commands` は検証済みで `[[cobo]]` と同じ長さ・同じ順(config.rs)。
        for (cobo, endpoint) in config
            .cobo
            .iter()
            .zip(config.controller.receiver_commands.iter())
        {
            components.push(ComponentEndpoint {
                name: format!("receiver{}", cobo.id),
                endpoint: endpoint.clone(),
                kind: ComponentKind::Receiver { cobo_id: cobo.id },
            });
        }

        Self {
            rest_listen: config.controller.rest_listen.clone(),
            passphrase: config.controller.passphrase.clone(),
            log_pull_bind: config.controller.log_pull_bind.clone(),
            ui_dir: config.controller.ui_dir.clone(),
            config_ids: config.controller.config_id.clone(),
            output_root: config.system.output_root.clone(),
            geometry_path: config.system.geometry.clone(),
            cobos: config
                .cobo
                .iter()
                .map(|c| CoboSpec {
                    id: c.id,
                    listen: c.listen.clone(),
                    data_sender_id: c.data_sender_id.clone(),
                })
                .collect(),
            components,
            ecc_endpoint: config.controller.ecc_command.clone(),
            router_ip: config.controller.router_ip.clone(),
            ecc_config_dir: config.controller.ecc_config_dir.clone(),
            params_dir: config.controller.params_dir.clone(),
            eos_timeout: Duration::from_secs(config.controller.eos_timeout_s),
            eos_quiesce: Duration::from_millis(config.controller.eos_quiesce_ms),
            eos_poll: DEFAULT_EOS_POLL,
            command_timeout: DEFAULT_COMMAND_TIMEOUT,
            ecc_timeout: DEFAULT_ECC_TIMEOUT,
            status_timeout: DEFAULT_STATUS_TIMEOUT,
            slow_cmd: config.controller.slow_cmd.clone(),
            slow_devices: config
                .slowctl
                .iter()
                .flat_map(|slowctl| slowctl.device.iter())
                .map(|device| (device.name.clone(), device.device_type.clone()))
                .collect(),
        }
    }

    /// ログブック `run_start.cobos`(SPEC §9.2)。
    fn cobo_listens(&self) -> Vec<CoboListen> {
        self.cobos
            .iter()
            .map(|c| CoboListen {
                id: c.id,
                listen: c.listen.clone(),
            })
            .collect()
    }

    fn data_sender_id(&self, cobo_id: u32) -> String {
        match self.cobos.iter().find(|c| c.id == cobo_id) {
            Some(cobo) => cobo.data_sender_id.clone(),
            None => {
                // components と cobos は同じ設定から作るので起きないが、起きたら黙らない。
                warn!(
                    cobo_id,
                    "no [[cobo]] block for this receiver — falling back to CoBo[k]"
                );
                format!("CoBo[{cobo_id}]")
            }
        }
    }
}

// ---------------------------------------------------------------------
// 操作権(SPEC §8.1)
// ---------------------------------------------------------------------

/// 操作権を握っている 1 クライアント。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Session {
    token: String,
    operator: String,
}

/// トークンを 1 つ発行する。
///
/// **これは認証ではない**(SPEC §3.1「事故防止であり認証ではない」)。二重操作の事故を
/// 防ぐための識別子なので、暗号論的乱数は使わず単調な素材(起動からの ns + 連番 + pid)で
/// 十分とする(依存を増やさない)。
fn new_token() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}-{:x}-{n:x}", std::process::id(), nanos)
}

// ---------------------------------------------------------------------
// controller 自身の run 状態
// ---------------------------------------------------------------------

/// controller の 4 相(§1.3 の状態機械の**外**にあるオーケストレータの相)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
    #[default]
    Idle,
    Starting,
    Running,
    Stopping,
}

impl std::fmt::Display for Phase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Phase::Idle => "Idle",
            Phase::Starting => "Starting",
            Phase::Running => "Running",
            Phase::Stopping => "Stopping",
        })
    }
}

#[derive(Debug, Clone)]
struct ActiveRun {
    run: u32,
    operator: String,
    comment: String,
    started_ts: String,
    started_at: Instant,
}

#[derive(Debug, Default)]
struct RunState {
    phase: Phase,
    active: Option<ActiveRun>,
    /// 124(Fable B-2): slowctl へ送った power コマンドが返るまでの**占有の印**。
    /// run/start の `Starting` と同じ手口 —— 短いロックで印を立て、REQ(最長 10 s)は
    /// ロックの外で待つ。立っている間は run/start と 2 本目の power を 409 で弾く。
    power_inflight: Option<PowerInflight>,
}

/// 実行中の power コマンド(409 の文言で「誰の何が飛んでいるか」を言うため)。
#[derive(Debug)]
struct PowerInflight {
    device: String,
    ch: u8,
    on: bool,
    operator: String,
}

/// ログ投稿(§2.3 LogPost)の受理・拒否カウンタ。silent にしないための可視化フック。
#[derive(Debug, Default)]
struct LogPostCounters {
    accepted: AtomicU64,
    rejected: AtomicU64,
    /// 直近値だけ更新してログブックに書かなかった slow VALUE の件数
    /// (TODO/121 §B —— 書かないことを silent にしない)。
    slow_values_not_logged: AtomicU64,
}

// ---------------------------------------------------------------------
// Controller(REST の状態)
// ---------------------------------------------------------------------

/// axum の共有状態。REST ハンドラはここを通してしか外に触らない。
pub struct Controller {
    params: ControllerParams,
    transport: ZmqTransport,
    /// SPEC §9.1「書き手は controller ただ一人」—— REST も LogPost スレッドもここを通る。
    logbook: Arc<Mutex<LogbookWriter>>,
    logbook_path: PathBuf,
    state_path: PathBuf,
    session: Mutex<Option<Session>>,
    run: Mutex<RunState>,
    log_posts: Arc<LogPostCounters>,
    /// slowctl の直近値(SPEC §9.2 v1.31: `run_start` / `run_stop` の `slow`)。
    /// キーは `"<device>/<node>/<param>"`(node 無しは `"-"`)。LogPost スレッドが書き、
    /// run の開閉で読む。**run 平均は作らない**(発注書 §2.6-7)。
    slow_latest: Arc<Mutex<SlowLatest>>,
}

/// `run_start.slow` / `run_stop.slow` の中身(SPEC §9.2 v1.31)。
pub type SlowLatest = BTreeMap<String, SlowLatestValue>;

/// その 1 要素。`ts` は controller がログブック行に打つのと同じ書式(SPEC §9.1)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlowLatestValue {
    pub value: f64,
    pub unit: String,
    pub ts: String,
}

/// `slow` レコード 1 本から直近値マップを更新する(純関数 —— テストが直接叩ける)。
///
/// 対象は **VALUE / ALARM / RECOVER** だけ(SPEC §9.2)。IDENT は文字列、ERROR は
/// カウンタなので「直近値」ではない。`values` の形が違う行は**黙って捨てずに** `false` を
/// 返して呼び手が数えられるようにする。
pub fn apply_slow_record(latest: &mut SlowLatest, record: &LogbookRecord, ts: &str) -> bool {
    let LogbookRecord::Slow {
        device,
        node,
        event,
        values,
        ..
    } = record
    else {
        return false;
    };
    if !matches!(event.as_str(), "VALUE" | "ALARM" | "RECOVER") {
        return false;
    }
    let Some(map) = values.as_object() else {
        return false;
    };
    let mut applied = false;
    for (param, entry) in map {
        let (Some(value), Some(unit)) = (
            entry.get("value").and_then(serde_json::Value::as_f64),
            entry.get("unit").and_then(serde_json::Value::as_str),
        ) else {
            continue;
        };
        latest.insert(
            crate::slowctl::latest_key(device, *node, param),
            SlowLatestValue {
                value,
                unit: unit.to_string(),
                ts: ts.to_string(),
            },
        );
        applied = true;
    }
    applied
}

/// 毒された Mutex でも controller を止めない(中身は controller 自身が作ったものだけで、
/// panic による不整合を持ち込む相手が居ない)。`.unwrap()` を使わないための共通口でもある。
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Controller {
    /// ログブックを開いて controller を組み立てる(`output_root` は無ければ作る)。
    pub fn new(params: ControllerParams) -> Result<Self, ControllerError> {
        std::fs::create_dir_all(&params.output_root).map_err(|source| ControllerError::Io {
            path: params.output_root.clone(),
            source,
        })?;
        let logbook_path = params.output_root.join("logbook.jsonl");
        let state_path = params.output_root.join("tpcdaq_state.json");
        let logbook = LogbookWriter::open(&logbook_path)?;
        let transport = ZmqTransport::new(
            params.ecc_endpoint.clone(),
            params.command_timeout,
            params.ecc_timeout,
        );
        Ok(Self {
            params,
            transport,
            logbook: Arc::new(Mutex::new(logbook)),
            logbook_path,
            state_path,
            session: Mutex::new(None),
            run: Mutex::new(RunState::default()),
            log_posts: Arc::new(LogPostCounters::default()),
            slow_latest: Arc::new(Mutex::new(SlowLatest::new())),
        })
    }

    pub fn params(&self) -> &ControllerParams {
        &self.params
    }

    /// `run_start` / `run_stop` に載せる slowctl の直近値(SPEC §9.2 v1.31)。
    ///
    /// 1 本も受け取っていなければ `None` = **フィールドごと省略**(slowctl が動いて
    /// いないことと「全部 0」を混同しない —— SPEC の nullable 規律)。
    fn slow_snapshot(&self) -> Option<Value> {
        let latest = lock(&self.slow_latest);
        if latest.is_empty() {
            return None;
        }
        serde_json::to_value(&*latest).ok()
    }

    // ---- ログブック ----

    /// 1 レコード追記。失敗しても run を止めない(が、必ずログに出す)。
    fn append(&self, record: LogbookRecord) {
        let mut writer = lock(&self.logbook);
        if let Err(e) = writer.append(record) {
            error!(error = %e, "cannot append to the logbook");
        }
    }

    /// 監査レコード(SPEC §8.1「状態変更系はすべて token 必須 + 監査ログ」)。
    fn audit(&self, action: &str, params: Value, operator: &str, ok: bool, error: Option<String>) {
        self.append(LogbookRecord::Audit {
            actor: ACTOR.to_string(),
            action: action.to_string(),
            params,
            operator: operator.to_string(),
            ok,
            error,
        });
    }

    // ---- 操作権 ----

    /// 常に横取りできる(C++ 版 CommandRouter 方式、SPEC §8.1)。奪われた側の名前を返す。
    fn acquire(&self, operator: &str) -> (String, Option<String>) {
        let token = new_token();
        let mut session = lock(&self.session);
        let preempted = session.as_ref().map(|s| s.operator.clone());
        *session = Some(Session {
            token: token.clone(),
            operator: operator.to_string(),
        });
        (token, preempted)
    }

    /// token を検証し、操作者名を返す。合わなければ `Err`(REST では 401)。
    fn authorize(&self, token: &str) -> Result<String, &'static str> {
        let session = lock(&self.session);
        match session.as_ref() {
            None => Err("no operator holds the control token"),
            Some(s) if s.token != token => Err("stale or unknown token"),
            Some(s) => Ok(s.operator.clone()),
        }
    }

    // ---- パラメータ JSON(078 レーン B §10)----

    /// `params_dir` の既定(省略 = `<output_root>/params`)。`router_ip` と同じ
    /// 「`Option` のまま持ち回り、既定は呼び出し側で足す」流儀(config.rs は解決しない
    /// —— `output_root` は `[system]` セクションの値なので、`[controller]` だけでは解決できない)。
    fn params_dir(&self) -> PathBuf {
        self.params
            .params_dir
            .clone()
            .unwrap_or_else(|| self.params.output_root.join("params"))
    }

    /// 現在保存されている params(`<params_dir>/current.json`)。無ければ `Ok(None)`
    /// (= まだ PUT されていない = TOML の config_id をそのまま使う、078 §10-3)。
    /// 読めるが検証に落ちる(壊れた保存)なら `Err`(silent にしない — CLAUDE.md)。
    fn current_params(&self) -> Result<Option<CurrentParams>, String> {
        let path = self.params_dir().join("current.json");
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
        };
        let params = crate::params::Params::from_json_str(&text)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Some(CurrentParams {
            sha256: sha256_hex(&text),
            text,
            params,
        }))
    }

    /// `ecc_config_dir` の下からテンプレートを読んで render する。`ecc_config_dir` の
    /// 有無(= 503 かどうか)は呼び手が確認済みという前提(ここでは判定しない —— エラー
    /// 文字列で分岐させないため。078 §10 の「503 とテンプレート不備の 400」は呼び手が
    /// 別々のタイミングでチェックすることで作り分ける)。
    fn render_with_template_dir(
        &self,
        ecc_config_dir: &Path,
        params: &crate::params::Params,
    ) -> Result<crate::params::Rendered, String> {
        let template_path = ecc_config_dir.join(&params.template);
        let template_xml = std::fs::read_to_string(&template_path)
            .map_err(|e| format!("cannot read template {}: {e}", template_path.display()))?;
        crate::params::render(params, &template_xml).map_err(|e| e.to_string())
    }
}

/// [`Controller::current_params`] の戻り値(078 レーン B §10)。
struct CurrentParams {
    params: crate::params::Params,
    /// 受け取った JSON テキストの sha256(フル、`Params::json_hash8` と同じ元テキストから
    /// 計算 —— 先頭 8 桁が id の suffix と一致する)。
    sha256: String,
    /// 保存されている JSON テキストそのもの(GET `/api/params` の `params` フィールドを
    /// 素通しで返すのに使う —— `Params` は往復用の全フィールドを公開していないため)。
    text: String,
}

/// テキストの sha256(16 進、小文字)。`geometry_record` と同じ手順。
fn sha256_hex(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// controller の起動時エラー。
#[derive(Debug, thiserror::Error)]
pub enum ControllerError {
    #[error("io error on {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("logbook error: {0}")]
    Logbook(#[from] crate::logbook::LogbookError),

    #[error("cannot bind REST listener on {listen}: {source}")]
    Bind {
        listen: String,
        #[source]
        source: std::io::Error,
    },

    #[error("cannot bind the log post PULL socket on {bind}: {source}")]
    LogPull {
        bind: String,
        #[source]
        source: zmq::Error,
    },

    #[error("REST server failed: {0}")]
    Serve(#[source] std::io::Error),
}

// ---------------------------------------------------------------------
// エントリポイント
// ---------------------------------------------------------------------

/// REST ルータを組む(SPEC §8.1 のエンドポイント + `ui_dir` の静的配信)。
pub fn router(controller: Arc<Controller>) -> Router {
    let ui_dir = controller.params.ui_dir.clone();
    let mut app = Router::new()
        .route("/api/status", get(get_status))
        .route("/api/control/acquire", post(post_acquire))
        .route("/api/control/release", post(post_release))
        .route("/api/run/start", post(post_run_start))
        .route("/api/run/stop", post(post_run_stop))
        .route("/api/run/next", post(post_run_next))
        .route("/api/ecc/{action}", post(post_ecc))
        .route("/api/params", get(get_params).put(put_params))
        .route("/api/params/render", get(get_params_render))
        .route("/api/params/templates", get(get_params_templates))
        .route("/api/params/defaults", get(get_params_defaults))
        .route("/api/slow/power", post(post_slow_power))
        .route("/api/logbook", get(get_logbook))
        .route("/api/logbook/comment", post(post_comment));
    if let Some(dir) = ui_dir {
        // UI は後波(P3)。ここは枠だけ — 設定に ui_dir が無ければ配信しない。
        info!(dir = %dir.display(), "serving the web UI from disk");
        // 052: クライアントルート(`/run` 等)への直リンク・リロードが 404 にならないよう、
        // 実在しないパスは index.html へ fallback する(tower-http の SPA 定型)。
        // `/api/...` と WS は上の `.route(...)` で先に確定するので、この fallback は
        // 静的側(未マッチのパス)にしか効かない。
        let index = dir.join("index.html");
        app = app.fallback_service(ServeDir::new(dir).fallback(ServeFile::new(index)));
    }
    app.with_state(controller)
}

/// 実際に bind した口(動的ポートを使うテスト用の自己申告)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundEndpoints {
    pub rest: SocketAddr,
    pub log_pull: String,
}

/// controller を起動し、`shutdown` が来るまで REST を提供する。
///
/// `bound` は REST とログ投稿 PULL の実 bind 先を 1 度だけ通知する(ポート 0 / `*` を使う
/// テスト用。production は `None` でよい)。
pub async fn run_controller(
    params: ControllerParams,
    mut shutdown: broadcast::Receiver<()>,
    bound: Option<oneshot::Sender<BoundEndpoints>>,
) -> Result<(), ControllerError> {
    let rest_listen = params.rest_listen.clone();
    let log_pull_bind = params.log_pull_bind.clone();
    let controller = Arc::new(Controller::new(params)?);

    let stop_flag = Arc::new(AtomicBool::new(false));
    let (log_thread, log_pull) = spawn_log_post_thread(
        log_pull_bind,
        Arc::clone(&controller.logbook),
        Arc::clone(&controller.log_posts),
        Arc::clone(&controller.slow_latest),
        Arc::clone(&stop_flag),
    )?;

    let listener = tokio::net::TcpListener::bind(&rest_listen)
        .await
        .map_err(|source| ControllerError::Bind {
            listen: rest_listen.clone(),
            source,
        })?;
    let local = listener
        .local_addr()
        .map_err(|source| ControllerError::Bind {
            listen: rest_listen.clone(),
            source,
        })?;
    info!(%local, "controller REST listening");
    if let Some(tx) = bound {
        let _ = tx.send(BoundEndpoints {
            rest: local,
            log_pull,
        });
    }

    let result = axum::serve(listener, router(Arc::clone(&controller)))
        .with_graceful_shutdown(async move {
            let _ = shutdown.recv().await;
        })
        .await
        .map_err(ControllerError::Serve);

    stop_flag.store(true, Ordering::Relaxed);
    if log_thread.join().is_err() {
        error!("log post thread panicked");
    }
    info!("controller stopped");
    result
}

mod logpost;
mod params_api;
mod power;
mod rest;
mod sequencer;
mod status;
mod transport;

use self::logpost::*;
use self::params_api::*;
use self::power::*;
use self::rest::*;
pub use self::sequencer::*;
pub use self::status::*;
pub use self::transport::*;
#[cfg(test)]
mod test_support;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::controller::test_support::*;
    use std::collections::BTreeSet;
    // -----------------------------------------------------------------
    // router_ip の解決
    // -----------------------------------------------------------------

    #[test]
    fn router_ip_prefers_the_config_then_the_actual_bind_address() {
        let mut params = params_with(&[0]);
        let mut mock = MockTransport::new(&params);

        // 設定が最優先。
        params.router_ip = Some("192.168.1.10".to_string());
        {
            let sequencer = Sequencer::new(&params, &mut mock);
            assert_eq!(
                sequencer.router_ip(0, Some("0.0.0.0:46005")),
                "192.168.1.10"
            );
        }

        // 設定が無ければ実 bind アドレスの host 部。
        params.router_ip = None;
        let sequencer = Sequencer::new(&params, &mut mock);
        assert_eq!(sequencer.router_ip(0, Some("10.0.0.7:46005")), "10.0.0.7");
        // 不定アドレスは CoBo から見て接続先にならないので落とす。
        assert_eq!(sequencer.router_ip(0, Some("0.0.0.0:46005")), "127.0.0.1");
        assert_eq!(sequencer.router_ip(0, None), "127.0.0.1");
    }

    #[test]
    fn acquire_always_preempts_and_names_the_previous_holder() {
        let controller = test_controller("preempt");
        let (token_a, preempted) = controller.acquire("aogaki");
        assert_eq!(preempted, None);
        assert_eq!(controller.authorize(&token_a).unwrap(), "aogaki");

        let (token_b, preempted) = controller.acquire("student");
        assert_eq!(preempted.as_deref(), Some("aogaki"));
        // 横取りされた側の token はもう効かない(操作権は常に 1 つ)。
        assert!(controller.authorize(&token_a).is_err());
        assert_eq!(controller.authorize(&token_b).unwrap(), "student");

        let _ = std::fs::remove_dir_all(&controller.params.output_root);
    }

    #[test]
    fn tokens_are_unique_per_acquire() {
        let controller = test_controller("token-unique");
        let mut seen = BTreeSet::new();
        for _ in 0..100 {
            assert!(
                seen.insert(controller.acquire("aogaki").0),
                "token が重複した"
            );
        }
        let _ = std::fs::remove_dir_all(&controller.params.output_root);
    }

    #[test]
    fn authorize_rejects_an_empty_or_unknown_token() {
        let controller = test_controller("token-reject");
        assert!(
            controller.authorize("").is_err(),
            "誰も握っていなければ拒否"
        );
        controller.acquire("aogaki");
        assert!(controller.authorize("bogus").is_err());
        let _ = std::fs::remove_dir_all(&controller.params.output_root);
    }

    // -----------------------------------------------------------------
    // 設定からのパラメタ解決(SPEC §3.2 の既定表)
    // -----------------------------------------------------------------

    #[test]
    fn params_from_config_uses_the_spec_default_endpoint_table() {
        let dir = std::env::temp_dir().join(format!("tpcdaq-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let geometry = dir.join("geometry.dat");
        std::fs::write(&geometry, "0\t0\t0\t0\tU\t1\t2\n").unwrap();
        let toml = format!(
            r#"
[system]
experiment = "mini_eTPC"
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

[controller]
passphrase = "change-me"
ecc_proxy = "Ecc:tcp -h 127.0.0.1 -p 46002"
config_id = "default"
"#,
            geometry = geometry.display()
        );
        let config = crate::config::parse(&toml).unwrap();
        let params = ControllerParams::from_config(&config);

        assert_eq!(
            params
                .components
                .iter()
                .map(|c| (c.name.as_str(), c.endpoint.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("graw-writer", "tcp://127.0.0.1:47100"),
                ("decoder", "tcp://127.0.0.1:47101"),
                ("receiver0", "tcp://127.0.0.1:47110"),
                ("receiver1", "tcp://127.0.0.1:47111"),
            ]
        );
        assert_eq!(params.ecc_endpoint, "tcp://127.0.0.1:47200");
        assert_eq!(params.log_pull_bind, "tcp://*:47005");
        assert_eq!(params.eos_timeout, Duration::from_secs(5));
        assert_eq!(params.rest_listen, "0.0.0.0:8080");
        assert_eq!(params.ui_dir, None);
        assert_eq!(params.cobos[1].data_sender_id, "CoBo[1]");
        // 124: `slow_cmd` は opt-in(省略 = None = power は 503)、`[slowctl]` が無ければ機器表も空。
        assert_eq!(params.slow_cmd, None);
        assert!(params.slow_devices.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 124(Fable B-1): `slow_cmd` は `[controller]` から、機器表は `[[slowctl.device]]` の
    /// (name, type) から**設定の順に**作る(power 以外の型も載せる — 400 の判定に使う)。
    #[test]
    fn params_from_config_take_slow_cmd_and_the_slowctl_device_table() {
        let dir = std::env::temp_dir().join(format!("tpcdaq-cfg-slow-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let geometry = dir.join("geometry.dat");
        std::fs::write(&geometry, "0\t0\t0\t0\tU\t1\t2\n").unwrap();
        let toml = format!(
            r#"
[system]
experiment = "mini_eTPC"
output_root = "/data/tpcdaq"
geometry = "{geometry}"

[[cobo]]
id = 0
data_sender_id = "CoBo[0]"

[decoder]
workers = 1

[root_sink]
snapshot_hz = 1.0
event_publish_hz = 20.0
build_timeout_ms = 1000

[monitor]
ws_listen = "0.0.0.0:9000"

[slowctl]
[[slowctl.device]]
type = "hmp2020"
name = "lv"
port = "/dev/serial/by-id/usb-HAMEG-if00-port0"
[[slowctl.device]]
type = "bronkhorst_flowbus"
name = "gas"
port = "/dev/serial/by-id/usb-FTDI-if00-port0"
[[slowctl.device]]
type = "hivolta"
name = "hv"
addr = "192.168.1.20:1470"

[controller]
passphrase = "change-me"
ecc_proxy = "Ecc:tcp -h 127.0.0.1 -p 46002"
config_id = "default"
slow_cmd = "tcp://127.0.0.1:47017"
"#,
            geometry = geometry.display()
        );
        let config = crate::config::parse(&toml).unwrap();
        let params = ControllerParams::from_config(&config);

        assert_eq!(params.slow_cmd.as_deref(), Some("tcp://127.0.0.1:47017"));
        assert_eq!(
            params.slow_devices,
            vec![
                ("lv".to_string(), "hmp2020".to_string()),
                ("gas".to_string(), "bronkhorst_flowbus".to_string()),
                ("hv".to_string(), "hivolta".to_string()),
            ]
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// run 開始 TS は graw-writer のファイル名 TS と同じ書式(SPEC §6.4 v1.8)。
    #[test]
    fn the_run_start_timestamp_has_the_graw_writer_shape() {
        let ts = run_start_timestamp();
        // "YYYY-MM-DDTHH:MM:SS.mmm" = 23 バイト固定。
        assert_eq!(ts.len(), 23, "{ts}");
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[10..11], "T");
        assert_eq!(&ts[19..20], ".");
    }

    // -----------------------------------------------------------------
    // SPA index.html fallback(052 — `/run` 等の直リンク・リロードが 404 にならない)
    // -----------------------------------------------------------------

    /// テスト用 `ui_dir`: index.html(SPA シェル)と実在の静的ファイルを 1 本置く。
    /// 中身は用途ごとに別の文字列にして、どの応答が返ったかを文字列比較だけで判別する。
    fn spa_ui_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tpcdaq-spa-ui-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("index.html"), "<html>052 SPA shell</html>").unwrap();
        std::fs::write(dir.join("app.js"), "console.log('052 real static file');").unwrap();
        dir
    }

    /// `params`(`ui_dir` 込み)で実際に REST を listen する controller を立てる。
    /// 到達不能コンポーネント / ecc-bridge への `GetStatus` を速く諦めさせるため、
    /// タイムアウト系は短く上書きしておく(呼び手が `params_with` の既定 1 s のままにしない)。
    async fn spawn_spa_test_server(mut params: ControllerParams, tag: &str) -> SocketAddr {
        params.rest_listen = "127.0.0.1:0".to_string();
        params.output_root =
            std::env::temp_dir().join(format!("tpcdaq-spa-out-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&params.output_root);
        params.command_timeout = Duration::from_millis(50);
        params.ecc_timeout = Duration::from_millis(50);
        params.status_timeout = Duration::from_millis(50);
        let controller = Arc::new(Controller::new(params).unwrap());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router(controller)).await;
        });
        addr
    }

    /// 1 リクエスト = 1 接続の手書き HTTP GET。ボディはそのまま返す
    /// (index.html の中身をそのまま照合したいので JSON にはパースしない —
    /// `tests/controller_integration.rs` の `http` ヘルパーとの違いはそこだけ)。
    fn spa_http_get(addr: SocketAddr, path: &str) -> (u16, String) {
        use std::io::{Read, Write};
        use std::net::TcpStream;

        let request =
            format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).unwrap();
        let text = String::from_utf8_lossy(&raw).to_string();
        let status: u16 = text
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| panic!("no status line in {text:?}"));
        let body = text
            .split_once("\r\n\r\n")
            .map(|(_, body)| body.to_string())
            .unwrap_or_default();
        (status, body)
    }

    /// 発注書 052 の完了条件①: 実在しないパス(クライアントルート直リンクの模擬)への
    /// GET が 200 + index.html の内容で返る。
    #[tokio::test(flavor = "multi_thread")]
    async fn unknown_path_falls_back_to_index_html() {
        let dir = spa_ui_dir("unknown-path");
        let mut params = params_with(&[]);
        params.ui_dir = Some(dir.clone());
        let addr = spawn_spa_test_server(params, "unknown-path").await;

        let (status, body) = spa_http_get(addr, "/run");
        assert_eq!(status, 200);
        assert_eq!(body, "<html>052 SPA shell</html>");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 発注書 052 の完了条件②: `/api/...` 経路は fallback に食われず従来どおり動く
    /// (index.html の中身ではなく、本来の JSON ハンドラの応答が返る)。
    #[tokio::test(flavor = "multi_thread")]
    async fn spa_fallback_does_not_shadow_api_routes() {
        let dir = spa_ui_dir("api-untouched");
        let mut params = params_with(&[]);
        params.ui_dir = Some(dir.clone());
        let addr = spawn_spa_test_server(params, "api-untouched").await;

        let (status, body) = spa_http_get(addr, "/api/status");
        assert_eq!(status, 200);
        assert_ne!(body, "<html>052 SPA shell</html>");
        assert!(
            body.contains("\"phase\""),
            "/api/status は本来の JSON のはず: {body}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 発注書 052 の完了条件③: 実在する静的ファイルは従来どおり自分自身の内容で返る
    /// (index.html へ吸われない)。
    #[tokio::test(flavor = "multi_thread")]
    async fn existing_static_file_is_served_unchanged() {
        let dir = spa_ui_dir("real-file");
        let mut params = params_with(&[]);
        params.ui_dir = Some(dir.clone());
        let addr = spawn_spa_test_server(params, "real-file").await;

        let (status, body) = spa_http_get(addr, "/app.js");
        assert_eq!(status, 200);
        assert_eq!(body, "console.log('052 real static file');");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
