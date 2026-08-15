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
use tower_http::services::ServeDir;
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
    pub eos_timeout: Duration,
    pub eos_poll: Duration,
    pub command_timeout: Duration,
    pub ecc_timeout: Duration,
    pub status_timeout: Duration,
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
            eos_timeout: Duration::from_secs(config.controller.eos_timeout_s),
            eos_poll: DEFAULT_EOS_POLL,
            command_timeout: DEFAULT_COMMAND_TIMEOUT,
            ecc_timeout: DEFAULT_ECC_TIMEOUT,
            status_timeout: DEFAULT_STATUS_TIMEOUT,
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
// ecc-bridge の応答(SPEC §8.2)
// ---------------------------------------------------------------------

/// ecc-bridge の 3 フィールド応答。`state` は `Off/Idle/Described/Prepared/Ready/Running/
/// Paused/Unknown`(ECC 不達は `Unknown`)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EccReply {
    pub ok: bool,
    pub state: String,
    #[serde(default)]
    pub error: String,
}

// ---------------------------------------------------------------------
// Transport(外への口 = テストの差し替え点)
// ---------------------------------------------------------------------

/// controller が外へ触る唯一の口。[`Sequencer`] はこれ以外の IO を知らない。
pub trait Transport {
    /// コンポーネントへ 1 コマンド(JSON REQ/REP、SPEC §2.6)。`Err` は**不達**
    /// (`success: false` の応答は `Ok` で返る — 断り方の区別は呼び手の仕事)。
    fn command(&mut self, endpoint: &str, command: &Command) -> Result<CommandResponse, String>;

    /// ecc-bridge へ 1 リクエスト(JSON REQ/REP、SPEC §8.2)。
    fn ecc(&mut self, request: &Value) -> Result<EccReply, String>;

    /// EOS 待ちなどの休止。モックは仮想時間を進めるだけでよい。
    fn sleep(&mut self, duration: Duration);
}

/// 実物の ZMQ Transport。ソケットは**呼び出し毎に作って捨てる**
/// ([`crate::command::request`] の doc コメント参照 — REQ の状態機械を持ち越さないため)。
#[derive(Clone)]
pub struct ZmqTransport {
    context: zmq::Context,
    ecc_endpoint: String,
    command_timeout: Duration,
    ecc_timeout: Duration,
}

impl ZmqTransport {
    pub fn new(ecc_endpoint: String, command_timeout: Duration, ecc_timeout: Duration) -> Self {
        Self {
            context: zmq::Context::new(),
            ecc_endpoint,
            command_timeout,
            ecc_timeout,
        }
    }

    /// タイムアウトだけ差し替えた複製(status ポーリング用)。
    fn with_command_timeout(&self, timeout: Duration) -> Self {
        Self {
            context: self.context.clone(),
            ecc_endpoint: self.ecc_endpoint.clone(),
            command_timeout: timeout,
            ecc_timeout: timeout,
        }
    }
}

impl Transport for ZmqTransport {
    fn command(&mut self, endpoint: &str, command: &Command) -> Result<CommandResponse, String> {
        crate::command::request(&self.context, endpoint, command, self.command_timeout)
            .map_err(|e| e.to_string())
    }

    fn ecc(&mut self, request: &Value) -> Result<EccReply, String> {
        let timeout_ms = i32::try_from(self.ecc_timeout.as_millis()).unwrap_or(i32::MAX);
        let endpoint = self.ecc_endpoint.clone();
        let fail = |what: &str, e: zmq::Error| format!("ecc-bridge {endpoint}: {what}: {e}");

        let socket = self
            .context
            .socket(zmq::REQ)
            .map_err(|e| fail("cannot create REQ socket", e))?;
        socket
            .set_rcvtimeo(timeout_ms)
            .map_err(|e| fail("cannot set rcvtimeo", e))?;
        socket
            .set_sndtimeo(timeout_ms)
            .map_err(|e| fail("cannot set sndtimeo", e))?;
        socket
            .set_linger(0)
            .map_err(|e| fail("cannot set linger", e))?;
        socket
            .connect(&endpoint)
            .map_err(|e| fail("cannot connect", e))?;

        let payload = serde_json::to_vec(request)
            .map_err(|e| format!("ecc-bridge {endpoint}: cannot encode request: {e}"))?;
        socket
            .send(payload, 0)
            .map_err(|e| fail("send failed", e))?;
        let reply = socket
            .recv_bytes(0)
            .map_err(|e| fail(&format!("no reply within {timeout_ms} ms"), e))?;
        serde_json::from_slice(&reply).map_err(|e| {
            format!(
                "ecc-bridge {endpoint}: reply is not {{ok,state,error}}: {e} ({})",
                String::from_utf8_lossy(&reply)
            )
        })
    }

    fn sleep(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

// ---------------------------------------------------------------------
// シーケンサ(SPEC §1.3 の純ロジック)
// ---------------------------------------------------------------------

/// run 開始の成功報告。
#[derive(Debug, Clone, PartialEq)]
pub struct StartReport {
    pub run: u32,
    /// Arm 応答から回収した (cobo_id, router_ip, router_port)。DataLinkSet の材料。
    pub links: Vec<RouterLink>,
    /// `Arm` を撃ち直したコンポーネントの実績(034)。**1 回で通ったものは載らない**ので、
    /// 空でない = bind の解放待ちが実際に起きた、という運用の可視化になる。
    pub arm_retries: Vec<ArmRetry>,
    /// 歩き戻しの**着地点**(034 で入り 036 で意味を精密化)。ここが `"Idle"`(ECC が
    /// 未初期化なら `"Off"`)でなければ run は始まらないので、成功報告では常にそのどちらか。
    /// 034 からのキー名を保つ(実機の audit を見る手順を変えないため)。
    pub ecc_state_after_reset: String,
    /// 歩き戻しの**各段**(036)。`"<action>-><その直後に ECC が申告した状態>"` の列で、
    /// 先頭は必ず `status->…`(= 歩く前に見えた状態)。
    /// 例: `["status->Ready", "breakup->Prepared", "reset->Described", "reset->Idle"]`。
    /// 1 要素だけなら歩く必要が無かった。**ログブックの audit に残す**(silent にしない)。
    pub ecc_walk_back: Vec<String>,
}

/// `Arm` のリトライ実績(034 = 論点 B「リトライしたら回数と所要を必ず出す」)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmRetry {
    pub component: String,
    /// 総試行回数(最後の 1 回を含む)。1 は載らない(= リトライしていない)。
    pub attempts: u32,
    /// 最初の試行から決着までの実所要。
    pub waited_ms: u64,
}

impl ArmRetry {
    fn to_json(&self) -> Value {
        json!({
            "component": self.component,
            "attempts": self.attempts,
            "waited_ms": self.waited_ms,
        })
    }
}

/// receiver が実際に bind した口(SPEC §8.2「router_port は receiver が実際に bind した
/// ポートを controller が Arm 応答から取って渡す」)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouterLink {
    pub cobo_id: u32,
    pub router_ip: String,
    pub router_port: u16,
}

/// run 開始の失敗報告(監査ログにそのまま載る)。
#[derive(Debug, Clone, PartialEq)]
pub struct StartFailure {
    /// `Configure` / `Arm` / `Start` / `ecc` / `geometry` のどこで落ちたか。
    pub stage: &'static str,
    pub detail: String,
    /// 巻き戻しで送ったコマンドのうち失敗したものの記録(silent にしない)。
    pub rollback_notes: Vec<String>,
}

impl std::fmt::Display for StartFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} stage failed: {}", self.stage, self.detail)?;
        if !self.rollback_notes.is_empty() {
            write!(f, " (rollback: {})", self.rollback_notes.join("; "))?;
        }
        Ok(())
    }
}

/// 停止の種類(SPEC §1.3: 正常停止と中止で EOS の待ち方だけが違う)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopMode {
    /// 正常停止。EOS の自然到達を `eos_timeout` まで待ってから強制する。
    Normal,
    /// 中止。待たずに即・強制 EOS(v1.6)。文字列は `run_stop.reason` の `abort:` の後ろ。
    Abort(String),
}

/// run 停止の報告(そのまま `run_stop` レコードになる)。
#[derive(Debug, Clone, PartialEq)]
pub struct StopReport {
    pub ok: bool,
    /// `"normal"` / `"abort:..."` / `"error:eos-timeout"`(SPEC §9.2)。
    pub reason: String,
    pub counters: Counters,
    pub files: Vec<OutputFile>,
    /// receiver へ `Stop` を送って EOS を強制したか(SPEC §1.3 の 5 秒分岐)。
    pub forced_eos: bool,
    /// EOS がチェーンを流れ切ったことを観測できたか。
    pub eos_closed: bool,
    /// 途中で起きた異常の記録(不達・不許可遷移など。silent にしない)。
    pub notes: Vec<String>,
}

/// §1.3 の run シーケンスそのもの。IO は [`Transport`] 越しだけ。
pub struct Sequencer<'a> {
    params: &'a ControllerParams,
    transport: &'a mut dyn Transport,
}

impl<'a> Sequencer<'a> {
    pub fn new(params: &'a ControllerParams, transport: &'a mut dyn Transport) -> Self {
        Self { params, transport }
    }

    // -----------------------------------------------------------------
    // 開始(SPEC §1.3)
    // -----------------------------------------------------------------

    /// run 開始シーケンス。
    ///
    /// **ecc を `Idle` へ歩き戻す(034 → 036)** → `Configure`(下流から)→ `Arm`(receiver の
    /// 実 bind ポートを回収)→ `Start{run}` → ecc `describe` → `prepare` →
    /// `configure`(DataLinkSet)→ `start`。
    /// **どの段の失敗も巻き戻す**(以降を実行せず、実行済みコンポーネントを畳む)。
    pub fn start_run(
        &mut self,
        run: u32,
        comment: &str,
        run_start_ts: &str,
    ) -> Result<StartReport, StartFailure> {
        let params = self.params;
        let mut touched = vec![false; params.components.len()];

        // 0. ECC を Idle へ**歩き戻す**(034 = 論点 A のユーザー裁定「毎 run 完全にリセット
        //    して一からやり直す」= ワルシャワ大学の作法。036 で実 ECC の意味論に合わせた)。
        //
        //    コンポーネントに 1 コマンドも触る前にここを済ませる —— 前の run が Running の
        //    まま残っていたとき、新しい listen が古い CoBo ストリームを拾わないため。
        let mut ecc_walk_back = Vec::new();
        let ecc_state_after_reset = match self.walk_ecc_back_to_idle(&mut ecc_walk_back) {
            Ok(state) => state,
            Err(detail) => return Err(self.rollback("ecc", detail, &touched)),
        };

        // 1. Configure(下流から: graw-writer → decoder → receivers)
        //    SPEC §6.4 v1.8: 正式な run 開始 TS は controller が 1 つだけ決めてここで配る。
        let run_config = RunConfig {
            run_number: run,
            comment: comment.to_string(),
            config: json!({ "run_start_ts": run_start_ts }),
        };
        for (i, component) in params.components.iter().enumerate() {
            if let Err(detail) = self.apply(component, &Command::Configure(run_config.clone())) {
                return Err(self.rollback("Configure", detail, &touched));
            }
            touched[i] = true;
        }

        // 2. Arm — receiver はここで bind + listen(listen-before-start)。実ポートを回収する。
        //    bind は前の run のソケット解放待ちで一時的に失敗しうるので粘る(034)。
        let mut links = Vec::new();
        let mut arm_retries = Vec::new();
        for (i, component) in params.components.iter().enumerate() {
            let response = match self.arm_with_retry(component, &mut arm_retries) {
                Ok(response) => response,
                Err(detail) => return Err(self.rollback("Arm", detail, &touched)),
            };
            touched[i] = true;
            if let ComponentKind::Receiver { cobo_id } = component.kind {
                match self.router_link(cobo_id, &response) {
                    Ok(link) => links.push(link),
                    Err(detail) => {
                        return Err(self.rollback(
                            "Arm",
                            format!("{}: {detail}", component.name),
                            &touched,
                        ))
                    }
                }
            }
        }

        // 3. Start{run}
        for (i, component) in params.components.iter().enumerate() {
            if let Err(detail) = self.apply(component, &Command::Start { run_number: run }) {
                return Err(self.rollback("Start", detail, &touched));
            }
            touched[i] = true;
        }

        // 4. ecc-bridge 経由(listen-before-start: ここに来た時点で全 receiver は listen 済み)
        // 相ごとに別の ConfigId を使う実運用があるので、**その相の id** を渡す(SPEC §8.2 v1.13)。
        let requests = [
            json!({"action": "describe", "config_id": params.config_ids.describe}),
            json!({"action": "prepare", "config_id": params.config_ids.prepare}),
            self.configure_request(&links),
            json!({"action": "start"}),
        ];
        for request in &requests {
            if let Err(detail) = self.apply_ecc(request) {
                return Err(self.rollback("ecc", detail, &touched));
            }
        }

        Ok(StartReport {
            run,
            links,
            arm_retries,
            ecc_state_after_reset,
            ecc_walk_back,
        })
    }

    /// ECC を **`Idle` まで歩き戻す**(036、SPEC §1.3 v1.12 の ⚠ 注記)。
    ///
    /// 実 ECC の `reset` は `EV_UNDO` = **1 段戻すだけ**で、`Active`(= Ready/Running/Paused)
    /// からは遷移が定義されておらず**無音で無視**される。よって「どこからでも reset 1 発で
    /// Idle」は成立しない —— **現在の状態を見て必要な段数だけ**歩く:
    ///
    /// ```text
    /// Running/Paused --stop--> Ready --breakup--> Prepared --reset--> Described --reset--> Idle
    /// ```
    ///
    /// 出典(`reference/20190315_patched`): `GetBench/src/get/rc/BackEnd.cpp:924`
    /// (`reset()` = `engine.step(EV_UNDO)` のみ)/ 同 `:250-270`(`EV_UNDO` は
    /// `Described→Idle` と `Prepared→Described` の 2 本だけ、`Active→Prepared` は `EV_BREAK`)/
    /// `StateMachine/src/dhsm/Engine.cpp:334-352`(未定義遷移は例外を投げず `Ignored` = 無音)/
    /// 同 `BackEnd.cpp:955-962`(`configure` は `ST_PREPARED` ガードで黙ってスキップ)。
    ///
    /// **`Idle` に着けなければ run を始めない**。歩けない状態(`Unknown` 等)も、
    /// 撃ったのに状態が動かなかった段(= 実 ECC の無音無視)も `Err` にする ——
    /// 黙って先へ進むと `configure` がスキップされ、**CoBo がデータリンクを張り直さないまま**
    /// run が始まる(2 本目以降が成立しない)。
    ///
    /// `Off`(ECC 未初期化 = 我々の起動直後)はこれ以上戻れない底なので、`Idle` と同じく
    /// 「何も出さない」で通す(`describe` は `Off / Idle / Described` から許される)。
    ///
    /// 歩いた跡は `trail` に `"<action>-><直後に ECC が申告した状態>"` で積む(audit 行き)。
    fn walk_ecc_back_to_idle(&mut self, trail: &mut Vec<String>) -> Result<String, String> {
        let mut state = self.apply_ecc(&json!({"action": "status"}))?;
        trail.push(format!("status->{state}"));

        let mut steps = 0usize;
        while state != "Idle" && state != "Off" {
            let action = match state.as_str() {
                "Running" | "Paused" => "stop",
                "Ready" => "breakup",
                "Prepared" | "Described" => "reset",
                _ => {
                    return Err(format!(
                        "ecc is in state {state} — refusing to start a run \
                         (cannot walk this state back to Idle)"
                    ))
                }
            };
            if steps == ECC_WALK_BACK_MAX_STEPS {
                return Err(format!(
                    "ecc did not reach Idle within {ECC_WALK_BACK_MAX_STEPS} steps \
                     (stuck in {state}; walked {trail:?}) — refusing to start a run"
                ));
            }
            let landed = self.apply_ecc(&json!({ "action": action }))?;
            trail.push(format!("{action}->{landed}"));
            steps += 1;
            if landed == state {
                // 実 ECC は未定義遷移をエラーにしない(`Ignored`)。「ok なのに状態が
                // 動いていない」が唯一の手がかりなので、ここで止める(silent 禁止)。
                return Err(format!(
                    "ecc {action} left the ECC in {state} — the real ECC ignores undefined \
                     transitions in silence; refusing to start a run"
                ));
            }
            state = landed;
        }
        info!(state, ?trail, "ecc walked back before the run");
        Ok(state)
    }

    /// `Arm` を「bind が空くまで」指数バックオフで粘って送る(034 = 論点 B)。
    ///
    /// 直前の run の `Reset` で閉じた PULL / listen ソケットは libzmq の `close` が非同期な
    /// ぶんだけ残るので、直後の `Arm` は `Address already in use` で弾かれる(030 実測:
    /// 3 回に 2 回、3 試行 0.118–0.121 s で成功)。ここで [`ARM_MAX_ATTEMPTS`] 回まで
    /// [`ARM_RETRY_BACKOFF`] から倍々に待って撃ち直す。
    ///
    /// **粘った事実は必ず残す**(silent 禁止): 成功しても [`ArmRetry`] を積んで audit へ、
    /// 失敗したら「何回・何 ms 粘ったか」を失敗理由の文字列に載せる。
    ///
    /// 断り方(不達 / 不許可遷移 / bind 失敗)で場合分けはしない —— 相手の文字列で分岐すると
    /// 文言が変わった日に黙って壊れる。恒久的な失敗でも高々 620 ms 遅れて同じ理由で落ちる。
    fn arm_with_retry(
        &mut self,
        component: &ComponentEndpoint,
        retries: &mut Vec<ArmRetry>,
    ) -> Result<CommandResponse, String> {
        let began = Instant::now();
        let mut backoff = ARM_RETRY_BACKOFF;
        let mut last = String::new();
        for attempt in 1..=ARM_MAX_ATTEMPTS {
            match self.apply(component, &Command::Arm) {
                Ok(response) => {
                    if attempt > 1 {
                        let retry = ArmRetry {
                            component: component.name.clone(),
                            attempts: attempt,
                            waited_ms: began.elapsed().as_millis() as u64,
                        };
                        warn!(
                            component = component.name,
                            attempts = retry.attempts,
                            waited_ms = retry.waited_ms,
                            "Arm succeeded only after retrying (socket close is asynchronous)"
                        );
                        retries.push(retry);
                    }
                    return Ok(response);
                }
                Err(detail) => {
                    last = detail;
                    if attempt < ARM_MAX_ATTEMPTS {
                        warn!(
                            component = component.name,
                            attempt,
                            backoff_ms = backoff.as_millis() as u64,
                            "Arm failed — retrying: {last}"
                        );
                        self.transport.sleep(backoff);
                        backoff *= 2;
                    }
                }
            }
        }
        let waited_ms = began.elapsed().as_millis() as u64;
        retries.push(ArmRetry {
            component: component.name.clone(),
            attempts: ARM_MAX_ATTEMPTS,
            waited_ms,
        });
        Err(format!(
            "{last} (gave up after {ARM_MAX_ATTEMPTS} attempts over {waited_ms} ms)"
        ))
    }

    /// 実行済みコンポーネントだけを**上流から**畳む(SPEC §1.3「半端な Running を残さない」)。
    ///
    /// `Stop` → `Reset` の順に送る。`Stop` は `Running` からしか合法でない(§1.3 遷移表)ため、
    /// `Configured` / `Armed` で止まったコンポーネントは `Stop` が断られる —— それでも
    /// `Reset` が必ず `Idle` へ戻す。断られた事実は `rollback_notes` に残す(silent にしない)。
    fn rollback(&mut self, stage: &'static str, detail: String, touched: &[bool]) -> StartFailure {
        let params = self.params;
        let mut rollback_notes = Vec::new();
        error!(stage, %detail, "run start failed — rolling back");
        for (i, component) in params.components.iter().enumerate().rev() {
            if !touched[i] {
                continue;
            }
            for command in [Command::Stop, Command::Reset] {
                if let Err(note) = self.apply(component, &command) {
                    rollback_notes.push(note);
                }
            }
        }
        StartFailure {
            stage,
            detail,
            rollback_notes,
        }
    }

    /// Arm 応答から DataLinkSet 1 本分を組む(SPEC §8.2)。
    fn router_link(&self, cobo_id: u32, response: &CommandResponse) -> Result<RouterLink, String> {
        let metrics = response.metrics.as_ref();
        let port = metrics
            .and_then(|m| m.get("router_port"))
            .and_then(Value::as_u64)
            .and_then(|p| u16::try_from(p).ok())
            .ok_or_else(|| "Arm response carries no router_port".to_string())?;
        let bind_address = metrics
            .and_then(|m| m.get("bind_address"))
            .and_then(Value::as_str);
        Ok(RouterLink {
            cobo_id,
            router_ip: self.router_ip(cobo_id, bind_address),
            router_port: port,
        })
    }

    /// DataLinkSet の `router_ip`。設定が最優先、無ければ receiver の実 bind アドレスから。
    /// `0.0.0.0` のような不定アドレスは CoBo から見て接続先にならないので落として警告する。
    fn router_ip(&self, cobo_id: u32, bind_address: Option<&str>) -> String {
        if let Some(ip) = &self.params.router_ip {
            return ip.clone();
        }
        let host = bind_address
            .and_then(|address| address.rsplit_once(':'))
            .map(|(host, _)| host.trim_matches(|c| c == '[' || c == ']'));
        match host {
            Some(host) if !host.is_empty() && host != "0.0.0.0" && host != "*" && host != "::" => {
                host.to_string()
            }
            _ => {
                warn!(
                    cobo_id,
                    bind_address = bind_address.unwrap_or("<none>"),
                    "receiver binds an unspecified address — using 127.0.0.1 as router_ip \
                     (set [controller] router_ip for a real CoBo network)"
                );
                "127.0.0.1".to_string()
            }
        }
    }

    fn configure_request(&self, links: &[RouterLink]) -> Value {
        let items: Vec<Value> = links
            .iter()
            .map(|link| {
                json!({
                    "sender": self.params.data_sender_id(link.cobo_id),
                    "router_ip": link.router_ip,
                    "router_port": link.router_port,
                    // 実機の罠: flowType は大文字 TCP(SPEC §8.2)。
                    "type": "TCP",
                })
            })
            .collect();
        json!({
            "action": "configure",
            "config_id": self.params.config_ids.configure,
            "links": items,
        })
    }

    // -----------------------------------------------------------------
    // 停止・中止(SPEC §1.3 / v1.6)
    // -----------------------------------------------------------------

    /// run 停止シーケンス。**中止でも必ず EOS で閉じてから**コンポーネントを畳む(v1.6)。
    pub fn stop_run(&mut self, mode: StopMode) -> StopReport {
        let params = self.params;
        let mut notes = Vec::new();

        // 1. ecc stop(CoBo が送信を止める)。中止では不達でも続行する。
        //    **データリンクはここでは閉じない**(SPEC §1.3 v1.12 の事実修正、TODO/032):
        //    実 GET の `daqStop` は TCP を close せず、close は `breakup`(`daqDisconnect`)か
        //    次の `configure` の張り直しのときである。よって実機の通常経路では
        //    **ecc stop 後に EOF は届かない** —— EOS は下の receiver `Stop` で注入する。
        //    (コメントのみ 036 で訂正。停止側の挙動そのものは TODO/033 の担当。)
        let ecc_stop_failed = match self.apply_ecc(&json!({"action": "stop"})) {
            Ok(_) => false,
            Err(detail) => {
                warn!(%detail, "ecc stop failed — continuing to close the run with EOS");
                notes.push(detail);
                true
            }
        };

        // 2. EOS 伝播待ち。中止は待たずに即・強制する(v1.6)。
        let mut eos_closed = match mode {
            StopMode::Normal => self.wait_for_eos(&mut notes),
            StopMode::Abort(_) => false,
        };
        let forced_eos = !eos_closed;
        if forced_eos {
            // 強制 EOS = receiver へ Stop(drain タスクが未送 EOS を出してから畳む)。
            info!("EOS did not arrive — forcing it via receiver Stop (SPEC §1.3)");
            for component in params.components.iter() {
                if matches!(component.kind, ComponentKind::Receiver { .. }) {
                    if let Err(note) = self.apply(component, &Command::Stop) {
                        notes.push(note);
                    }
                }
            }
            eos_closed = self.wait_for_eos(&mut notes);
        }

        // 3. run_stop の材料を集める(Stop / Reset の前 —— graw-writer は Reset で
        //    RunWriter を手放し、ファイル実績が metrics から消える)。
        let statuses = self.collect_status(&mut notes);
        let counters = counters_from(params, &statuses);
        let files = files_from(&statuses);

        // 4. コンポーネントを畳む(上流から)。ここは run クローズ後なので Reset してよい。
        //    強制 EOS で既に Stop 済みの receiver へは Stop を撃ち直さない。
        for (i, component) in params.components.iter().enumerate().rev() {
            let already_stopped =
                forced_eos && matches!(component.kind, ComponentKind::Receiver { .. });
            if !already_stopped {
                if let Err(note) = self.apply(component, &Command::Stop) {
                    notes.push(note);
                }
            }
            // Stop 後は Configured(または Error)なので、次の run のために Idle へ戻す。
            let state = statuses.get(i).and_then(|s| s.state);
            if state == Some(ComponentState::Error) {
                warn!(
                    component = component.name,
                    "component is in Error — resetting after run close"
                );
            }
            if let Err(note) = self.apply(component, &Command::Reset) {
                notes.push(note);
            }
        }

        let reason = if let StopMode::Abort(why) = &mode {
            format!("abort:{why}")
        } else if !eos_closed {
            "error:eos-timeout".to_string()
        } else if ecc_stop_failed {
            "abort:ecc-stop-failed".to_string()
        } else {
            "normal".to_string()
        };
        StopReport {
            ok: reason == "normal",
            reason,
            counters,
            files,
            forced_eos,
            eos_closed,
            notes,
        }
    }

    /// EOS がチェーンを流れ切る(= root-sink が run をクローズできる)まで待つ。
    /// root-sink は REP を持たないので、**graw-writer / decoder の `GetStatus`** で代理観測する
    /// (SPEC §1.3 の「GetStatus と時間で監視」)。
    fn wait_for_eos(&mut self, notes: &mut Vec<String>) -> bool {
        let params = self.params;
        let mut waited = Duration::ZERO;
        loop {
            if self.eos_complete(notes) {
                return true;
            }
            if waited >= params.eos_timeout {
                let note = format!(
                    "EOS did not propagate within {} ms",
                    params.eos_timeout.as_millis()
                );
                warn!("{note}");
                push_once(notes, note);
                return false;
            }
            self.transport.sleep(params.eos_poll);
            waited += params.eos_poll;
        }
    }

    /// EOS 到達の判定(2 点とも満たすこと):
    /// - decoder: `eos_in` が上流 CoBo 数に達した(全 receiver の EOS を受け取った)
    /// - graw-writer: `files_open == 0`(全期待ソースの EOS 受領で全ファイルを閉じた、SPEC §7)
    fn eos_complete(&mut self, notes: &mut Vec<String>) -> bool {
        let params = self.params;
        let expected_eos = params.cobos.len() as u64;
        for component in params.components.iter() {
            let key = match component.kind {
                ComponentKind::Decoder => "eos_in",
                ComponentKind::GrawWriter => "files_open",
                ComponentKind::Receiver { .. } => continue,
            };
            let response = match self
                .transport
                .command(&component.endpoint, &Command::GetStatus)
            {
                Ok(response) => response,
                Err(e) => {
                    push_once(notes, format!("{}: GetStatus failed: {e}", component.name));
                    return false;
                }
            };
            let value = response
                .metrics
                .as_ref()
                .and_then(|m| m.get(key))
                .and_then(Value::as_u64);
            let done = match (component.kind, value) {
                (ComponentKind::Decoder, Some(eos_in)) => eos_in >= expected_eos,
                (ComponentKind::GrawWriter, Some(open)) => open == 0,
                (_, None) => {
                    push_once(
                        notes,
                        format!("{}: GetStatus metrics has no {key}", component.name),
                    );
                    false
                }
                _ => false,
            };
            if !done {
                return false;
            }
        }
        true
    }

    /// 全コンポーネントの `GetStatus`(`components` と同じ順)。
    fn collect_status(&mut self, notes: &mut Vec<String>) -> Vec<ComponentStatus> {
        let params = self.params;
        let mut out = Vec::with_capacity(params.components.len());
        for component in params.components.iter() {
            let status = match self
                .transport
                .command(&component.endpoint, &Command::GetStatus)
            {
                Ok(response) => ComponentStatus::from_response(component, &response),
                Err(e) => {
                    push_once(notes, format!("{}: GetStatus failed: {e}", component.name));
                    ComponentStatus::unreachable(component, e)
                }
            };
            out.push(status);
        }
        out
    }

    // -----------------------------------------------------------------
    // 下請け
    // -----------------------------------------------------------------

    /// コマンドを 1 件送り、**不達も「断られた」も `Err`** にまとめる(呼び手はどちらでも
    /// シーケンスを止めるので区別しない。文字列にはどちらか判る形で残す)。
    fn apply(
        &mut self,
        component: &ComponentEndpoint,
        command: &Command,
    ) -> Result<CommandResponse, String> {
        match self.transport.command(&component.endpoint, command) {
            Err(e) => Err(format!("{}: {command} unreachable: {e}", component.name)),
            Ok(response) if !response.success => Err(format!(
                "{}: {command} refused in state {}: {}",
                component.name, response.state, response.message
            )),
            Ok(response) => {
                info!(component = component.name, %command, state = %response.state, "command applied");
                Ok(response)
            }
        }
    }

    /// ecc-bridge へ 1 リクエスト。成功したら **ECC が申告した状態**を返す(034: reset が
    /// 本当に Idle まで戻ったかを呼び手が見られるようにするため)。
    fn apply_ecc(&mut self, request: &Value) -> Result<String, String> {
        let action = request
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("<none>")
            .to_string();
        match self.transport.ecc(request) {
            Err(e) => Err(format!("ecc {action} unreachable: {e}")),
            Ok(reply) if !reply.ok => Err(format!(
                "ecc {action} failed in state {}: {}",
                reply.state, reply.error
            )),
            Ok(reply) => {
                info!(action, state = reply.state, "ecc command applied");
                Ok(reply.state)
            }
        }
    }
}

/// 同じ文言を何度も積まない(EOS 待ちは毎周期同じ失敗を出しうる)。
fn push_once(notes: &mut Vec<String>, note: String) {
    if !notes.contains(&note) {
        notes.push(note);
    }
}

// ---------------------------------------------------------------------
// コンポーネント状態のスナップショット
// ---------------------------------------------------------------------

/// `GET /api/status` と run_stop の材料。
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentStatus {
    pub name: String,
    pub kind: ComponentKind,
    pub endpoint: String,
    pub reachable: bool,
    pub state: Option<ComponentState>,
    pub run_number: Option<u32>,
    pub metrics: Option<Value>,
    pub error: Option<String>,
}

impl ComponentStatus {
    fn from_response(component: &ComponentEndpoint, response: &CommandResponse) -> Self {
        Self {
            name: component.name.clone(),
            kind: component.kind,
            endpoint: component.endpoint.clone(),
            reachable: true,
            state: Some(response.state),
            run_number: response.run_number,
            metrics: response.metrics.clone(),
            error: (!response.success).then(|| response.message.clone()),
        }
    }

    fn unreachable(component: &ComponentEndpoint, error: String) -> Self {
        Self {
            name: component.name.clone(),
            kind: component.kind,
            endpoint: component.endpoint.clone(),
            reachable: false,
            state: None,
            run_number: None,
            metrics: None,
            error: Some(error),
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "endpoint": self.endpoint,
            "reachable": self.reachable,
            "state": self.state.map(|s| s.to_string()),
            "run_number": self.run_number,
            "metrics": self.metrics,
            "error": self.error,
        })
    }

    fn metric_u64(&self, key: &str) -> Option<u64> {
        self.metrics
            .as_ref()
            .and_then(|m| m.get(key))
            .and_then(Value::as_u64)
    }
}

/// run_stop の `counters`(SPEC §9.2)を GetStatus 実測から充填する。
///
/// **取れる分だけ入れる**: `events_built` / `events_incomplete` / `late_fragments` は
/// root-sink だけが持つ値で、root-sink は REP を持たない(SPEC §1.3)ので常に `None`。
/// `overflow_frames` / `malformed` は receiver / decoder の GetStatus から実測できるので
/// `Some`(025 で `Counters` が `Option<u64>` 化された — SPEC v1.10 §9.2、016 逸脱③の確定処置。
/// 016 は非 Option で「取得不能」も `0` 埋めしていた)。
fn counters_from(params: &ControllerParams, statuses: &[ComponentStatus]) -> Counters {
    let mut frames = BTreeMap::new();
    let mut overflow_frames = 0u64;
    let mut malformed = 0u64;
    for status in statuses {
        match status.kind {
            ComponentKind::Receiver { cobo_id } => {
                frames.insert(
                    cobo_id.to_string(),
                    status.metric_u64("frames").unwrap_or(0),
                );
                overflow_frames += status.metric_u64("overflow_frames").unwrap_or(0);
            }
            ComponentKind::Decoder => {
                malformed += status.metric_u64("malformed").unwrap_or(0);
            }
            ComponentKind::GrawWriter => {}
        }
    }
    // 設定にある CoBo が status に居ない(不達)場合も鍵は出す(欠落を隠さない)。
    for cobo in &params.cobos {
        frames.entry(cobo.id.to_string()).or_insert(0);
    }
    Counters {
        events_built: None,
        events_incomplete: None,
        late_fragments: None,
        frames,
        overflow_frames: Some(overflow_frames),
        malformed: Some(malformed),
    }
}

/// run_stop の `files`(SPEC §9.2)。graw-writer の metrics の実績をそのまま写す。
fn files_from(statuses: &[ComponentStatus]) -> Vec<OutputFile> {
    let mut files = Vec::new();
    for status in statuses {
        if status.kind != ComponentKind::GrawWriter {
            continue;
        }
        let entries = status
            .metrics
            .as_ref()
            .and_then(|m| m.get("files"))
            .and_then(Value::as_array);
        for entry in entries.into_iter().flatten() {
            let path = entry.get("path").and_then(Value::as_str);
            let bytes = entry.get("bytes").and_then(Value::as_u64);
            if let (Some(path), Some(bytes)) = (path, bytes) {
                files.push(OutputFile {
                    path: path.to_string(),
                    bytes,
                });
            }
        }
    }
    files
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
}

/// ログ投稿(§2.3 LogPost)の受理・拒否カウンタ。silent にしないための可視化フック。
#[derive(Debug, Default)]
struct LogPostCounters {
    accepted: AtomicU64,
    rejected: AtomicU64,
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
        })
    }

    pub fn params(&self) -> &ControllerParams {
        &self.params
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
// run_start の材料(ジオメトリ)
// ---------------------------------------------------------------------

/// `run_start.geometry`(SPEC §9.2)と `expected_fragments` をジオメトリファイルから作る。
///
/// `expected_fragments` = ジオメトリが申告している (cobo, asad) 在庫。root-sink の
/// EventBuilder が使う期待集合と同じ定義(`tools/root_sink/eb_core.hpp`)。
/// [`crate::geometry::Geometry::asad_inventory`](025)を直接呼ぶ — `lookup` を経由しないので
/// `unmapped_hit_count` を汚さない(旧実装は同じ理由で `dump_tsv` の先頭 2 列をパースして
/// いたが、025 で専用アクセサに置き換えた)。
fn geometry_record(path: &Path) -> Result<(GeometryRecord, Vec<(u32, u32)>), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let sha256 = format!("{:x}", hasher.finalize());

    let text = String::from_utf8_lossy(&bytes);
    let geometry = crate::geometry::parse(&text);
    let fragments = expected_fragments(&geometry);
    if fragments.is_empty() {
        warn!(path = %path.display(), "geometry declares no (cobo, asad) — expected_fragments is empty");
    }
    Ok((
        GeometryRecord {
            path: path.display().to_string(),
            sha256,
        },
        fragments,
    ))
}

/// 025: [`crate::geometry::Geometry::asad_inventory`] へ委譲するだけの薄いラッパ
/// (呼び出し側 [`geometry_record`] のドキュメントを参照)。
fn expected_fragments(geometry: &Geometry) -> Vec<(u32, u32)> {
    geometry.asad_inventory()
}

/// logbook `run_start` の `(config_id, config_ids)`(SPEC §9.2 v1.13)。
///
/// - 3 相同値: `config_id` = その値、`config_ids` は**出さない**
///   (省略 ≠ 空値 —— 025 の counters と同じ nullable 規律)。
/// - 非同値: `config_id` = **configure 相**の id(run のデータ取得条件を代表する相)、
///   `config_ids` に 3 相全部。
fn run_start_config_fields(ids: &ConfigIds) -> (String, Option<ConfigIds>) {
    match ids.same_value() {
        Some(id) => (id.to_string(), None),
        None => (ids.configure.clone(), Some(ids.clone())),
    }
}

/// 正式な run 開始 TS(SPEC §6.4 v1.8)。graw-writer のファイル名 TS と同じ書式にして、
/// 将来そのまま採用できるようにしておく(`src/graw_writer.rs` の `local_timestamp`)。
fn run_start_timestamp() -> String {
    chrono::Local::now()
        .format("%Y-%m-%dT%H:%M:%S%.3f")
        .to_string()
}

// ---------------------------------------------------------------------
// REST ハンドラ(SPEC §8.1)
// ---------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct AcquireRequest {
    operator: String,
    passphrase: String,
}

#[derive(Debug, Deserialize)]
struct TokenRequest {
    token: String,
}

#[derive(Debug, Deserialize)]
struct RunStartRequest {
    token: String,
    #[serde(default)]
    comment: String,
}

/// `POST /api/run/next`(SPEC v1.10 §8.1)。`next_run` はまず `Value` のまま受けて
/// 手でバリデーションする — `u32`/`u64` で直接受けると、型不一致(文字列・小数・負値)を
/// axum の `Json` extractor が本ハンドラより先に弾いてしまい、発注書どおりの
/// 「それ以外は 400」を一律に返せない(extractor 側の rejection は 422 になりうる)。
#[derive(Debug, Deserialize)]
struct RunNextRequest {
    token: String,
    next_run: Value,
}

#[derive(Debug, Deserialize)]
struct CommentRequest {
    author: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct SinceQuery {
    #[serde(default)]
    since_seq: u64,
}

type Reply = (StatusCode, Json<Value>);

fn ok_json(body: Value) -> Reply {
    (StatusCode::OK, Json(body))
}

fn err_json(code: StatusCode, message: impl Into<String>) -> Reply {
    (code, Json(json!({"error": message.into()})))
}

/// 全コンポーネント + ecc + 自身の相(SPEC §8.1)。閲覧系なので認証は要らない。
async fn get_status(State(controller): State<Arc<Controller>>) -> Reply {
    let reply = tokio::task::spawn_blocking(move || {
        let mut transport = controller
            .transport
            .with_command_timeout(controller.params.status_timeout);
        let mut notes = Vec::new();
        let components: Vec<Value> = {
            let mut sequencer = Sequencer::new(&controller.params, &mut transport);
            sequencer
                .collect_status(&mut notes)
                .iter()
                .map(ComponentStatus::to_json)
                .collect()
        };
        let ecc = match transport.ecc(&json!({"action": "status"})) {
            Ok(reply) => json!({"ok": reply.ok, "state": reply.state, "error": reply.error}),
            Err(e) => json!({"ok": false, "state": "Unknown", "error": e}),
        };
        let run_state = lock(&controller.run);
        let operator = lock(&controller.session)
            .as_ref()
            .map(|s| s.operator.clone());
        json!({
            "phase": run_state.phase.to_string(),
            "run": run_state.active.as_ref().map(|active| json!({
                "run": active.run,
                "operator": active.operator,
                "comment": active.comment,
                "started_ts": active.started_ts,
                "elapsed_s": active.started_at.elapsed().as_secs_f64(),
            })),
            "operator": operator,
            "components": components,
            "ecc": ecc,
            "log_posts": {
                "accepted": controller.log_posts.accepted.load(Ordering::Relaxed),
                "rejected": controller.log_posts.rejected.load(Ordering::Relaxed),
            },
            "notes": notes,
        })
    })
    .await;
    match reply {
        Ok(body) => ok_json(body),
        Err(e) => err_json(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("status task: {e}"),
        ),
    }
}

/// 操作権の取得(常に横取り可、横取りは監査へ)。
async fn post_acquire(
    State(controller): State<Arc<Controller>>,
    Json(request): Json<AcquireRequest>,
) -> Reply {
    if request.passphrase != controller.params.passphrase {
        controller.audit(
            "control/acquire",
            json!({"operator": request.operator}),
            &request.operator,
            false,
            Some("wrong passphrase".to_string()),
        );
        warn!(
            operator = request.operator,
            "control/acquire refused — wrong passphrase"
        );
        return err_json(StatusCode::FORBIDDEN, "wrong passphrase");
    }
    let (token, preempted) = controller.acquire(&request.operator);
    controller.audit(
        "control/acquire",
        json!({"operator": request.operator, "preempted": preempted}),
        &request.operator,
        true,
        None,
    );
    if let Some(previous) = &preempted {
        warn!(
            operator = request.operator,
            previous, "control token taken over"
        );
    }
    ok_json(json!({"token": token, "preempted": preempted}))
}

async fn post_release(
    State(controller): State<Arc<Controller>>,
    Json(request): Json<TokenRequest>,
) -> Reply {
    let operator = match controller.authorize(&request.token) {
        Ok(operator) => operator,
        Err(why) => return unauthorized(&controller, "control/release", json!({}), why),
    };
    *lock(&controller.session) = None;
    controller.audit("control/release", json!({}), &operator, true, None);
    ok_json(json!({"released": true}))
}

/// token 無し / 不一致は 401 相当 + 監査(SPEC §8.1)。
fn unauthorized(controller: &Controller, action: &str, params: Value, why: &str) -> Reply {
    controller.audit(
        action,
        params,
        "<unauthorized>",
        false,
        Some(why.to_string()),
    );
    warn!(action, why, "rejected an unauthorized request");
    err_json(StatusCode::UNAUTHORIZED, why)
}

/// run 開始(§1.3 のシーケンス)。
async fn post_run_start(
    State(controller): State<Arc<Controller>>,
    Json(request): Json<RunStartRequest>,
) -> Reply {
    let operator = match controller.authorize(&request.token) {
        Ok(operator) => operator,
        Err(why) => {
            return unauthorized(
                &controller,
                "run/start",
                json!({"comment": request.comment}),
                why,
            )
        }
    };

    // 相の占有(Idle からしか始められない = 二重 start を弾く)。
    {
        let mut run_state = lock(&controller.run);
        if run_state.phase != Phase::Idle {
            let why = format!("controller is {} — not Idle", run_state.phase);
            drop(run_state);
            controller.audit(
                "run/start",
                json!({"comment": request.comment}),
                &operator,
                false,
                Some(why.clone()),
            );
            return err_json(StatusCode::CONFLICT, why);
        }
        run_state.phase = Phase::Starting;
    }

    // ブロッキングタスクが panic しても相を Starting のまま固めない(制御不能にしない)。
    let recovery = Arc::clone(&controller);
    let reply = tokio::task::spawn_blocking(move || {
        let outcome = start_run_blocking(&controller, &operator, &request.comment);
        let mut run_state = lock(&controller.run);
        match outcome {
            Ok((run, ts)) => {
                run_state.phase = Phase::Running;
                run_state.active = Some(ActiveRun {
                    run,
                    operator,
                    comment: request.comment,
                    started_ts: ts,
                    started_at: Instant::now(),
                });
                ok_json(json!({"run": run, "phase": "Running"}))
            }
            Err(detail) => {
                run_state.phase = Phase::Idle;
                run_state.active = None;
                err_json(StatusCode::INTERNAL_SERVER_ERROR, detail)
            }
        }
    })
    .await;
    match reply {
        Ok(reply) => reply,
        Err(e) => {
            let mut run_state = lock(&recovery.run);
            run_state.phase = Phase::Idle;
            run_state.active = None;
            error!(error = %e, "run start task died — back to Idle");
            err_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("run start task: {e}"),
            )
        }
    }
}

/// run 開始の本体(ブロッキング)。成功したら (run 番号, 開始 TS)。
fn start_run_blocking(
    controller: &Controller,
    operator: &str,
    comment: &str,
) -> Result<(u32, String), String> {
    // ジオメトリはコンポーネントに触る前に読む(run_start レコードに必須 — SPEC §9.2)。
    let geometry = match geometry_record(&controller.params.geometry_path) {
        Ok(geometry) => geometry,
        Err(detail) => {
            controller.audit(
                "run/start",
                json!({"comment": comment}),
                operator,
                false,
                Some(detail.clone()),
            );
            return Err(detail);
        }
    };

    // run 番号採番(015、controller 単一書き手)。
    let run = match crate::state::take_next_run(&controller.state_path) {
        Ok(run) => run,
        Err(e) => {
            let detail = format!("cannot take the next run number: {e}");
            controller.audit(
                "run/start",
                json!({"comment": comment}),
                operator,
                false,
                Some(detail.clone()),
            );
            return Err(detail);
        }
    };

    let run_start_ts = run_start_timestamp();
    let mut transport = controller.transport.clone();
    let result = {
        let mut sequencer = Sequencer::new(&controller.params, &mut transport);
        sequencer.start_run(run, comment, &run_start_ts)
    };

    match result {
        Ok(report) => {
            let (geometry, expected_fragments) = geometry;
            let (config_id, config_ids) = run_start_config_fields(&controller.params.config_ids);
            // ここから先(run_start をログブックへ書いた後)は **絶対に採番を巻き戻さない** ——
            // 記録に残った run 番号を次の run が撃ち直すことになる(034 = 論点 C)。
            controller.append(LogbookRecord::RunStart {
                actor: ACTOR.to_string(),
                run,
                config_id,
                config_ids,
                geometry,
                cobos: controller.params.cobo_listens(),
                operator: operator.to_string(),
                comment: comment.to_string(),
                expected_fragments,
            });
            controller.audit(
                "run/start",
                json!({"run": run, "comment": comment, "links": report.links.iter()
                    .map(|l| json!({"cobo": l.cobo_id, "router_ip": l.router_ip, "router_port": l.router_port}))
                    .collect::<Vec<_>>(),
                    // 034: Arm を撃ち直したなら回数と所要を必ず残す(silent 禁止)。
                    "arm_retries": report.arm_retries.iter().map(ArmRetry::to_json).collect::<Vec<_>>(),
                    // 034: 歩き戻しの着地点(`Idle` / `Off` 以外では run は始まらない)。
                    "ecc_state_after_reset": report.ecc_state_after_reset,
                    // 036: **歩き戻しの各段**。実機で歩き戻しが効いているかはここで機械確認する
                    // (2 本目以降が `["status->Ready","breakup->Prepared","reset->Described",
                    //  "reset->Idle"]` になっていること)。
                    "ecc_walk_back": report.ecc_walk_back}),
                operator,
                true,
                None,
            );
            info!(run, ts = run_start_ts, "run started");
            Ok((run, run_start_ts))
        }
        Err(failure) => {
            let detail = failure.to_string();
            // 034 = 論点 C: **run_start をログブックへ書く前**に落ちたので、採番を返す。
            // controller は `next_run` の単一書き手で、この時点の相は `Starting`
            // (`run/next` も次の `run/start` も弾かれる)ため、割り込みが無い。
            let rewound = match crate::state::set_next_run(&controller.state_path, run) {
                Ok(()) => {
                    info!(run, "run start failed — next_run rewound to this number");
                    true
                }
                Err(e) => {
                    error!(run, error = %e, "cannot rewind next_run — the number is spent");
                    false
                }
            };
            controller.audit(
                "run/start",
                json!({
                    "run": run,
                    "comment": comment,
                    "stage": failure.stage,
                    "next_run_rewound": rewound,
                }),
                operator,
                false,
                Some(detail.clone()),
            );
            Err(detail)
        }
    }
}

/// run 番号の手動設定(SPEC v1.10 §8.1)。次の `run/start` から有効になる。
///
/// 検証順序は既存の `post_ecc`(未知 action → 400)と揃えた: authorize(401)→ 入力の形
/// (400)→ 相(Phase が Idle か、409)→ 成功。
async fn post_run_next(
    State(controller): State<Arc<Controller>>,
    Json(request): Json<RunNextRequest>,
) -> Reply {
    let operator = match controller.authorize(&request.token) {
        Ok(operator) => operator,
        Err(why) => return unauthorized(&controller, "run/next", json!({}), why),
    };

    // 正整数のみ(0・負・非数はすべて 400)。`Value::as_u64` は非負整数の JSON 数値だけ
    // `Some` を返す(小数・負値・文字列・真偽値・null はすべて `None`)。
    let next_run = match request
        .next_run
        .as_u64()
        .filter(|&n| n > 0 && n <= u32::MAX as u64)
    {
        Some(n) => n as u32,
        None => {
            let why = format!(
                "next_run must be a positive integer, got {}",
                request.next_run
            );
            controller.audit(
                "run/next",
                json!({"next_run": request.next_run}),
                &operator,
                false,
                Some(why.clone()),
            );
            return err_json(StatusCode::BAD_REQUEST, why);
        }
    };

    {
        let run_state = lock(&controller.run);
        if run_state.phase != Phase::Idle {
            let why = format!(
                "controller is {} — cannot set next_run while a run is active",
                run_state.phase
            );
            drop(run_state);
            controller.audit(
                "run/next",
                json!({"next_run": next_run}),
                &operator,
                false,
                Some(why.clone()),
            );
            return err_json(StatusCode::CONFLICT, why);
        }
    }

    match crate::state::set_next_run(&controller.state_path, next_run) {
        Ok(()) => {
            controller.audit(
                "run/next",
                json!({"next_run": next_run}),
                &operator,
                true,
                None,
            );
            info!(next_run, "next run number set manually");
            ok_json(json!({"next_run": next_run}))
        }
        Err(e) => {
            let why = format!("cannot persist next_run: {e}");
            controller.audit(
                "run/next",
                json!({"next_run": next_run}),
                &operator,
                false,
                Some(why.clone()),
            );
            err_json(StatusCode::INTERNAL_SERVER_ERROR, why)
        }
    }
}

/// run 停止(§1.3)。コンポーネントが Error に落ちていれば中止経路(v1.6)へ切り替える。
async fn post_run_stop(
    State(controller): State<Arc<Controller>>,
    Json(request): Json<TokenRequest>,
) -> Reply {
    let operator = match controller.authorize(&request.token) {
        Ok(operator) => operator,
        Err(why) => return unauthorized(&controller, "run/stop", json!({}), why),
    };

    let active = {
        let mut run_state = lock(&controller.run);
        if run_state.phase != Phase::Running {
            let why = format!("controller is {} — no run to stop", run_state.phase);
            drop(run_state);
            controller.audit("run/stop", json!({}), &operator, false, Some(why.clone()));
            return err_json(StatusCode::CONFLICT, why);
        }
        run_state.phase = Phase::Stopping;
        run_state.active.clone()
    };

    // 停止タスクが panic しても相を Stopping のまま固めない(次の run を出せなくしない)。
    let recovery = Arc::clone(&controller);
    let reply = tokio::task::spawn_blocking(move || {
        let body = stop_run_blocking(&controller, &operator, active);
        let mut run_state = lock(&controller.run);
        run_state.phase = Phase::Idle;
        run_state.active = None;
        ok_json(body)
    })
    .await;
    match reply {
        Ok(reply) => reply,
        Err(e) => {
            let mut run_state = lock(&recovery.run);
            run_state.phase = Phase::Idle;
            run_state.active = None;
            error!(error = %e, "run stop task died — back to Idle");
            err_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("run stop task: {e}"),
            )
        }
    }
}

fn stop_run_blocking(controller: &Controller, operator: &str, active: Option<ActiveRun>) -> Value {
    let mut transport = controller.transport.clone();

    // 中止判定(SPEC §1.4-3): 走行中のコンポーネントが Error なら異常停止として扱う。
    let mode = {
        let mut notes = Vec::new();
        let mut sequencer = Sequencer::new(&controller.params, &mut transport);
        let statuses = sequencer.collect_status(&mut notes);
        let broken: Vec<&str> = statuses
            .iter()
            .filter(|s| s.state == Some(ComponentState::Error) || !s.reachable)
            .map(|s| s.name.as_str())
            .collect();
        if broken.is_empty() {
            StopMode::Normal
        } else {
            warn!(components = ?broken, "stopping as an abort — component(s) in Error/unreachable");
            StopMode::Abort(format!("component-error:{}", broken.join(",")))
        }
    };

    let report = {
        let mut sequencer = Sequencer::new(&controller.params, &mut transport);
        sequencer.stop_run(mode)
    };

    let run = active.as_ref().map(|a| a.run).unwrap_or(0);
    let duration_s = active
        .as_ref()
        .map(|a| a.started_at.elapsed().as_secs_f64())
        .unwrap_or(0.0);
    controller.append(LogbookRecord::RunStop {
        actor: ACTOR.to_string(),
        run,
        duration_s,
        ok: report.ok,
        reason: report.reason.clone(),
        counters: report.counters.clone(),
        files: report.files.clone(),
    });
    controller.audit(
        "run/stop",
        json!({
            "run": run,
            "reason": report.reason,
            "forced_eos": report.forced_eos,
            "eos_closed": report.eos_closed,
        }),
        operator,
        report.ok,
        (!report.notes.is_empty()).then(|| report.notes.join("; ")),
    );
    info!(
        run,
        reason = report.reason,
        forced_eos = report.forced_eos,
        "run stopped"
    );
    json!({
        "run": run,
        "ok": report.ok,
        "reason": report.reason,
        "forced_eos": report.forced_eos,
        "eos_closed": report.eos_closed,
        "notes": report.notes,
    })
}

/// ECC の段階操作(R6: GET controller と同じ操作感)。
async fn post_ecc(
    State(controller): State<Arc<Controller>>,
    UrlPath(action): UrlPath<String>,
    Json(request): Json<TokenRequest>,
) -> Reply {
    const ACTIONS: [&str; 7] = [
        "describe",
        "prepare",
        "configure",
        "start",
        "stop",
        "breakup",
        "reset",
    ];
    let endpoint = format!("ecc/{action}");
    let operator = match controller.authorize(&request.token) {
        Ok(operator) => operator,
        Err(why) => return unauthorized(&controller, &endpoint, json!({}), why),
    };
    if !ACTIONS.contains(&action.as_str()) {
        let why = format!("unknown ecc action: {action}");
        controller.audit(&endpoint, json!({}), &operator, false, Some(why.clone()));
        return err_json(StatusCode::BAD_REQUEST, why);
    }

    let reply = tokio::task::spawn_blocking(move || {
        let mut transport = controller.transport.clone();
        let request_body = match action.as_str() {
            // DataLinkSet は receiver が**実際に bind した**ポートで組む(SPEC §8.2)。
            "configure" => match configure_request_now(&controller, &mut transport) {
                Ok(body) => body,
                Err(why) => {
                    controller.audit(&endpoint, json!({}), &operator, false, Some(why.clone()));
                    return err_json(StatusCode::CONFLICT, why);
                }
            },
            // describe / prepare は**その相の** ConfigId を載せる(SPEC §8.2 v1.13)。
            // start / stop / breakup / reset は id を使わない = 載せない。
            other => match controller.params.config_ids.for_action(other) {
                Some(config_id) => json!({"action": action, "config_id": config_id}),
                None => json!({"action": action}),
            },
        };
        match transport.ecc(&request_body) {
            Ok(reply) => {
                controller.audit(
                    &endpoint,
                    request_body.clone(),
                    &operator,
                    reply.ok,
                    (!reply.ok).then(|| reply.error.clone()),
                );
                ok_json(json!({"ok": reply.ok, "state": reply.state, "error": reply.error}))
            }
            Err(e) => {
                controller.audit(&endpoint, request_body, &operator, false, Some(e.clone()));
                err_json(StatusCode::BAD_GATEWAY, e)
            }
        }
    })
    .await;
    match reply {
        Ok(reply) => reply,
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, format!("ecc task: {e}")),
    }
}

/// いまの receiver の bind 実績から DataLinkSet を組む(段階操作の `configure` 用)。
fn configure_request_now(
    controller: &Controller,
    transport: &mut ZmqTransport,
) -> Result<Value, String> {
    let mut sequencer = Sequencer::new(&controller.params, transport);
    let mut links = Vec::new();
    for component in controller.params.components.iter() {
        let ComponentKind::Receiver { cobo_id } = component.kind else {
            continue;
        };
        let response = sequencer.apply(component, &Command::GetStatus)?;
        let link = sequencer
            .router_link(cobo_id, &response)
            .map_err(|e| format!("{}: {e} (Arm the receivers first)", component.name))?;
        links.push(link);
    }
    Ok(sequencer.configure_request(&links))
}

/// ログブックの読み出し(閲覧系 = 認証なし、SPEC §8.1 の二層)。
async fn get_logbook(
    State(controller): State<Arc<Controller>>,
    Query(query): Query<SinceQuery>,
) -> Reply {
    let path = controller.logbook_path.clone();
    let reply =
        tokio::task::spawn_blocking(move || crate::logbook::read_since(&path, query.since_seq))
            .await;
    match reply {
        Ok(Ok(result)) => ok_json(json!({
            "records": result.records,
            "tail_corrupt": result.tail_corrupt,
        })),
        Ok(Err(e)) => err_json(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        Err(e) => err_json(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("logbook task: {e}"),
        ),
    }
}

/// コメント投稿(R11)。**状態を変えない**ので token は要求しない(記録は `author` が持つ)。
async fn post_comment(
    State(controller): State<Arc<Controller>>,
    Json(request): Json<CommentRequest>,
) -> Reply {
    controller.append(LogbookRecord::Comment {
        actor: "ui".to_string(),
        author: request.author,
        text: request.text,
    });
    ok_json(json!({"appended": true}))
}

// ---------------------------------------------------------------------
// ログ投稿 PULL(SPEC §2.3 LogPost / §3.2)
// ---------------------------------------------------------------------

/// LogPost を受けてログブックへ流すスレッド。
///
/// ペイロードは「JSONL 1 レコード分の JSON 文字列」(§2.3)。`ts` / `seq` は controller が
/// 付ける(§9.1)ので、投稿側は付けない。読めなかった投稿は捨てるが**数えて警告する**。
fn spawn_log_post_thread(
    bind: String,
    logbook: Arc<Mutex<LogbookWriter>>,
    counters: Arc<LogPostCounters>,
    stop: Arc<AtomicBool>,
) -> Result<(std::thread::JoinHandle<()>, String), ControllerError> {
    let context = zmq::Context::new();
    let socket = context
        .socket(zmq::PULL)
        .map_err(|source| ControllerError::LogPull {
            bind: bind.clone(),
            source,
        })?;
    crate::zmq_helper::apply_pull_hwm(&socket).map_err(|source| ControllerError::LogPull {
        bind: bind.clone(),
        source,
    })?;
    socket
        .set_rcvtimeo(LOG_PULL_POLL_MS)
        .map_err(|source| ControllerError::LogPull {
            bind: bind.clone(),
            source,
        })?;
    socket
        .bind(&bind)
        .map_err(|source| ControllerError::LogPull {
            bind: bind.clone(),
            source,
        })?;
    let endpoint = match socket.get_last_endpoint() {
        Ok(Ok(endpoint)) => endpoint,
        _ => bind.clone(),
    };
    info!(%endpoint, "log post PULL bound");

    let handle = std::thread::Builder::new()
        .name("controller-logpost".to_string())
        .spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let message = match socket.recv_bytes(0) {
                    Ok(message) => message,
                    Err(zmq::Error::EAGAIN) => continue,
                    Err(e) => {
                        error!(error = %e, "log post recv failed");
                        continue;
                    }
                };
                match serde_json::from_slice::<LogbookRecord>(&message) {
                    Ok(record) => {
                        let mut writer = lock(&logbook);
                        match writer.append(record) {
                            Ok(_) => {
                                counters.accepted.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(e) => {
                                counters.rejected.fetch_add(1, Ordering::Relaxed);
                                error!(error = %e, "cannot append a log post");
                            }
                        }
                    }
                    Err(e) => {
                        counters.rejected.fetch_add(1, Ordering::Relaxed);
                        warn!(error = %e, payload = %String::from_utf8_lossy(&message),
                              "log post is not a logbook record — dropped");
                    }
                }
            }
            info!("log post task stopped");
        })
        .map_err(|source| ControllerError::Io {
            path: PathBuf::from("controller-logpost"),
            source,
        })?;
    Ok((handle, endpoint))
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
        .route("/api/logbook", get(get_logbook))
        .route("/api/logbook/comment", post(post_comment));
    if let Some(dir) = ui_dir {
        // UI は後波(P3)。ここは枠だけ — 設定に ui_dir が無ければ配信しない。
        info!(dir = %dir.display(), "serving the web UI from disk");
        app = app.fallback_service(ServeDir::new(dir));
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

// ---------------------------------------------------------------------
// テスト(仕様書 — 先に書く)
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    // 025: `expected_fragments` の TSV パース削除後は本体(非テストコード)で
    // `BTreeSet` を使わなくなったので、テスト専用にここで import する。
    use std::collections::BTreeSet;

    // -----------------------------------------------------------------
    // モック Transport(コマンド列を記録する)
    // -----------------------------------------------------------------

    /// 1 回の外向き操作の記録。`component` はコンポーネント名 or `"ecc"`。
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Call {
        component: String,
        action: String,
    }

    impl Call {
        fn new(component: &str, action: &str) -> Self {
            Self {
                component: component.to_string(),
                action: action.to_string(),
            }
        }
    }

    /// スクリプト可能なモック。
    ///
    /// - `fail`: (コンポーネント名, コマンド名) → その 1 件を `success: false` で断る
    /// - `metrics`: コンポーネント名 → GetStatus / Arm 応答に載せる metrics
    struct MockTransport {
        calls: Vec<Call>,
        names: BTreeMap<String, String>, // endpoint → name
        fail: Vec<(String, String)>,
        /// コンポーネント名 → 残り「Arm を断る回数」(034: bind 解放待ちの模擬)。
        refuse_arm: BTreeMap<String, u32>,
        metrics: BTreeMap<String, Value>,
        /// ECC が受けた**リクエスト本文**そのまま(action 名だけでは `config_id` を見られない)。
        ecc_requests: Vec<Value>,
        ecc_fail: Vec<String>,
        /// この ECC が今いる状態(036)。[`ecc_transition`] で**実 ECC と同じように**動く。
        ecc_state: String,
        /// 「受けたのに無音で何もしない」action(036)。実 ECC の未定義遷移は例外も出さず
        /// `Ignored`(`StateMachine/src/dhsm/Engine.cpp:334-352`)なので、歩き戻しが
        /// 空振りする経路を**わざと**作れるようにしておく。
        ecc_ignores: Vec<String>,
        /// EOS 完了を「N 回目の GetStatus 以降」にする(時間経過の模擬)。
        eos_after_polls: Option<usize>,
        eos_polls: usize,
        slept: Duration,
    }

    impl MockTransport {
        fn new(params: &ControllerParams) -> Self {
            let mut names = BTreeMap::new();
            let mut metrics = BTreeMap::new();
            for (i, component) in params.components.iter().enumerate() {
                names.insert(component.endpoint.clone(), component.name.clone());
                let value = match component.kind {
                    // 非対称な値(取り違え検出用)。
                    ComponentKind::Receiver { cobo_id } => json!({
                        "router_port": 46100 + cobo_id,
                        "bind_address": format!("0.0.0.0:{}", 46100 + cobo_id),
                        "frames": 3000 + i,
                        "overflow_frames": 0,
                    }),
                    ComponentKind::Decoder => json!({"eos_in": 0, "malformed": 7}),
                    ComponentKind::GrawWriter => json!({
                        "files_open": 1,
                        "files": [{"path": "run0001/CoBo0_AsAd0_ts_0000.graw", "bytes": 30_108_672u64}],
                    }),
                };
                metrics.insert(component.name.clone(), value);
            }
            Self {
                calls: Vec::new(),
                names,
                fail: Vec::new(),
                refuse_arm: BTreeMap::new(),
                metrics,
                ecc_requests: Vec::new(),
                ecc_fail: Vec::new(),
                ecc_state: "Idle".to_string(),
                ecc_ignores: Vec::new(),
                eos_after_polls: None,
                eos_polls: 0,
                slept: Duration::ZERO,
            }
        }

        /// EOS は最初から流れ切っている(正常停止で強制 EOS に落ちない)。
        fn with_eos_closed(mut self) -> Self {
            self.eos_after_polls = Some(0);
            self
        }

        fn fail_on(mut self, component: &str, command: &str) -> Self {
            self.fail.push((component.to_string(), command.to_string()));
            self
        }

        /// 034: 最初の `times` 回だけ `Arm` を断る(libzmq の close 非同期による
        /// `Address already in use` が数十 ms で解ける様子の模擬)。
        fn refuse_arm_times(mut self, component: &str, times: u32) -> Self {
            self.refuse_arm.insert(component.to_string(), times);
            self
        }

        fn fail_ecc(mut self, action: &str) -> Self {
            self.ecc_fail.push(action.to_string());
            self
        }

        /// 036: ECC がこの状態に居るところから始める(前の run の `ecc stop` 後は `Ready`)。
        fn ecc_in(mut self, state: &str) -> Self {
            self.ecc_state = state.to_string();
            self
        }

        /// 036: この action を**受けるが無音で何もしない** ECC にする(実 ECC の `Ignored`)。
        fn ecc_ignores(mut self, action: &str) -> Self {
            self.ecc_ignores.push(action.to_string());
            self
        }

        fn name_of(&self, endpoint: &str) -> String {
            self.names
                .get(endpoint)
                .cloned()
                .unwrap_or_else(|| endpoint.to_string())
        }

        /// あるコンポーネントが受けた**状態を変えるコマンド**の列(`GetStatus` は
        /// 観測であって操作ではないので除く)。
        fn actions(&self, component: &str) -> Vec<String> {
            self.calls
                .iter()
                .filter(|c| c.component == component && c.action != "GetStatus")
                .map(|c| c.action.clone())
                .collect()
        }

        /// GetStatus を除いた呼び出し列(順序照合を読みやすくするため)。
        fn commands(&self) -> Vec<Call> {
            self.calls
                .iter()
                .filter(|c| c.action != "GetStatus")
                .cloned()
                .collect()
        }

        fn index_of(&self, component: &str, action: &str) -> Option<usize> {
            self.calls
                .iter()
                .position(|c| c.component == component && c.action == action)
        }

        /// ECC が受けたリクエストのうち、その action の 1 通目(042: 相ごとの `config_id`)。
        fn ecc_request(&self, action: &str) -> Option<&Value> {
            self.ecc_requests
                .iter()
                .find(|r| r.get("action").and_then(Value::as_str) == Some(action))
        }
    }

    /// **実 ECC の状態機械そのもの**(036)。`None` = その状態に遷移が無い =
    /// 実 ECC は例外も出さず**無音で無視**する(`StateMachine/src/dhsm/Engine.cpp:334-352`)。
    ///
    /// 出典は `reference/20190315_patched/GetBench/src/get/rc/BackEnd.cpp:250-270`
    /// (SM の全遷移)/ `:924`(`reset()` = `EV_UNDO` だけ)/ `:955-962`(`configure` の
    /// `ST_PREPARED` ガード)。`tools/ecc_bridge/ecc_core.hpp` の表と**同じ意味論**にしてある。
    ///
    /// **このダブルを甘くしてはならない**: 「reset はどこからでも Idle」という誤った
    /// テストダブルが 034 の誤実装を green で通した(036 の起票理由そのもの)。
    fn ecc_transition(state: &str, action: &str) -> Option<&'static str> {
        match (action, state) {
            ("describe", "Off" | "Idle" | "Described") => Some("Described"),
            ("prepare", "Described" | "Prepared") => Some("Prepared"),
            // ST_PREPARED ガード: Prepared 以外からの configure は**黙ってスキップ**される。
            ("configure", "Prepared") => Some("Ready"),
            ("start", "Ready") => Some("Running"),
            ("stop", "Running" | "Paused") => Some("Ready"),
            // EV_BREAK: Active(= Ready/Running/Paused)→ Prepared。
            ("breakup", "Ready" | "Running" | "Paused") => Some("Prepared"),
            // EV_UNDO は **1 段戻すだけ**。Active からは遷移が無い。
            ("reset", "Described") => Some("Idle"),
            ("reset", "Prepared") => Some("Described"),
            // status は観測。それ以外はすべて無音(状態は動かない)。
            _ => None,
        }
    }

    /// コマンドの短い名前(記録用)。
    fn action_name(command: &Command) -> String {
        match command {
            Command::Configure(_) => "Configure".to_string(),
            Command::Arm => "Arm".to_string(),
            Command::Start { .. } => "Start".to_string(),
            Command::Stop => "Stop".to_string(),
            Command::Reset => "Reset".to_string(),
            Command::GetStatus => "GetStatus".to_string(),
        }
    }

    impl Transport for MockTransport {
        fn command(
            &mut self,
            endpoint: &str,
            command: &Command,
        ) -> Result<CommandResponse, String> {
            let name = self.name_of(endpoint);
            let action = action_name(command);
            self.calls.push(Call::new(&name, &action));

            let mut metrics = self.metrics.get(&name).cloned().unwrap_or(Value::Null);
            if matches!(command, Command::GetStatus) {
                // EOS の進み方を模擬する: 規定回数の GetStatus を過ぎたら「閉じた」形にする。
                self.eos_polls += 1;
                let closed = self
                    .eos_after_polls
                    .is_some_and(|after| self.eos_polls > after);
                if closed {
                    if let Some(map) = metrics.as_object_mut() {
                        if map.contains_key("eos_in") {
                            map.insert("eos_in".to_string(), json!(9));
                        }
                        if map.contains_key("files_open") {
                            map.insert("files_open".to_string(), json!(0));
                        }
                    }
                }
            }

            let mut refused = self.fail.iter().any(|(c, a)| c == &name && a == &action);
            if action == "Arm" {
                if let Some(left) = self.refuse_arm.get_mut(&name) {
                    if *left > 0 {
                        *left -= 1;
                        refused = true;
                    }
                }
            }
            let state = ComponentState::Running;
            let response = if refused {
                CommandResponse::error(state, format!("mock refuses {action}"))
            } else {
                CommandResponse::success(state, format!("{action} ok"))
            };
            Ok(response.with_metrics(metrics))
        }

        fn ecc(&mut self, request: &Value) -> Result<EccReply, String> {
            let action = request
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("<none>")
                .to_string();
            self.calls.push(Call::new("ecc", &action));
            self.ecc_requests.push(request.clone());
            if self.ecc_fail.contains(&action) {
                return Ok(EccReply {
                    ok: false,
                    state: self.ecc_state.clone(),
                    error: format!("mock refuses {action}"),
                });
            }
            if !self.ecc_ignores.contains(&action) {
                if let Some(next) = ecc_transition(&self.ecc_state, &action) {
                    self.ecc_state = next.to_string();
                }
            }
            // 実 ECC は未定義遷移でもエラーを返さない(無音)ので **ok は常に true**。
            Ok(EccReply {
                ok: true,
                state: self.ecc_state.clone(),
                error: String::new(),
            })
        }

        fn sleep(&mut self, duration: Duration) {
            self.slept += duration; // 仮想時間: 実際には眠らない
        }
    }

    // -----------------------------------------------------------------
    // テスト用パラメタ(1 CoBo と 2 CoBo)
    // -----------------------------------------------------------------

    fn params_with(cobo_ids: &[u32]) -> ControllerParams {
        let mut components = vec![
            ComponentEndpoint {
                name: "graw-writer".to_string(),
                endpoint: "inproc://gw".to_string(),
                kind: ComponentKind::GrawWriter,
            },
            ComponentEndpoint {
                name: "decoder".to_string(),
                endpoint: "inproc://dec".to_string(),
                kind: ComponentKind::Decoder,
            },
        ];
        let mut cobos = Vec::new();
        for &id in cobo_ids {
            components.push(ComponentEndpoint {
                name: format!("receiver{id}"),
                endpoint: format!("inproc://rx{id}"),
                kind: ComponentKind::Receiver { cobo_id: id },
            });
            cobos.push(CoboSpec {
                id,
                listen: format!("0.0.0.0:{}", 46005 + id),
                data_sender_id: format!("CoBo[{id}]"),
            });
        }
        ControllerParams {
            rest_listen: "127.0.0.1:0".to_string(),
            passphrase: "secret".to_string(),
            log_pull_bind: "inproc://logpost".to_string(),
            ui_dir: None,
            config_ids: ConfigIds::same("mock"),
            output_root: PathBuf::from("/tmp/tpcdaq-mock"),
            geometry_path: PathBuf::from("/tmp/tpcdaq-mock/geometry.dat"),
            cobos,
            components,
            ecc_endpoint: "inproc://ecc".to_string(),
            router_ip: None,
            eos_timeout: Duration::from_secs(5),
            eos_poll: Duration::from_millis(200),
            command_timeout: Duration::from_secs(1),
            ecc_timeout: Duration::from_secs(1),
            status_timeout: Duration::from_secs(1),
        }
    }

    // -----------------------------------------------------------------
    // 開始シーケンス(SPEC §1.3)
    // -----------------------------------------------------------------

    /// 順序の正本: **ecc を Idle へ歩き戻す(034 → 036)**→ Configure(下流から)→ Arm →
    /// Start → ecc 4 段。
    ///
    /// 先頭の段は 034(論点 A = ユーザー裁定「毎 run 完全にリセットして一からやり直す」)で
    /// 入り、036 で**現在状態を見て必要な段数だけ歩く**形になった。ここは既に `Idle` に
    /// 居るので `status` 1 発だけで済む(= 1 本目の実機の姿)。
    #[test]
    fn start_follows_the_spec_order_and_collects_router_ports() {
        let params = params_with(&[0, 1]);
        let mut mock = MockTransport::new(&params).ecc_in("Idle");
        let report = Sequencer::new(&params, &mut mock)
            .start_run(57, "gas test", "2026-08-14T10:00:00.000")
            .unwrap();

        let expected = vec![
            Call::new("ecc", "status"),
            Call::new("graw-writer", "Configure"),
            Call::new("decoder", "Configure"),
            Call::new("receiver0", "Configure"),
            Call::new("receiver1", "Configure"),
            Call::new("graw-writer", "Arm"),
            Call::new("decoder", "Arm"),
            Call::new("receiver0", "Arm"),
            Call::new("receiver1", "Arm"),
            Call::new("graw-writer", "Start"),
            Call::new("decoder", "Start"),
            Call::new("receiver0", "Start"),
            Call::new("receiver1", "Start"),
            Call::new("ecc", "describe"),
            Call::new("ecc", "prepare"),
            Call::new("ecc", "configure"),
            Call::new("ecc", "start"),
        ];
        assert_eq!(mock.commands(), expected);

        // Arm 応答から回収した実ポート(モックは 46100 + cobo_id を返す)。
        assert_eq!(
            report.links,
            vec![
                RouterLink {
                    cobo_id: 0,
                    router_ip: "127.0.0.1".to_string(), // 0.0.0.0 は落として警告(不定アドレス)
                    router_port: 46100,
                },
                RouterLink {
                    cobo_id: 1,
                    router_ip: "127.0.0.1".to_string(),
                    router_port: 46101,
                },
            ]
        );
        assert!(
            report.arm_retries.is_empty(),
            "1 発で通ったならリトライ記録は空: {:?}",
            report.arm_retries
        );
    }

    // -----------------------------------------------------------------
    // 034 — 連続 run の運用性
    // -----------------------------------------------------------------

    /// **論点 A**(ユーザー裁定、036 で歩き戻しに更新): run 開始は必ず ECC を `Idle` へ
    /// 戻すところから始まる —— **コンポーネントに 1 コマンドも触る前**に片付ける
    /// (= 前の run が残した `Ready` から `describe` を撃って順序違反で死ぬ経路を塞ぐ。
    /// 新しい listen が前の run の CoBo ストリームを拾う経路も同時に塞がる)。
    #[test]
    fn the_start_sequence_walks_the_ecc_back_to_idle_before_touching_any_component() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params).ecc_in("Ready");
        Sequencer::new(&params, &mut mock)
            .start_run(57, "", "2026-08-14T10:00:00.000")
            .unwrap();

        let calls = mock.commands();
        let first_component = calls
            .iter()
            .position(|c| c.component != "ecc")
            .expect("コンポーネントに 1 つもコマンドが出ていない");
        assert_eq!(
            &calls[..first_component],
            &[
                Call::new("ecc", "status"),
                Call::new("ecc", "breakup"),
                Call::new("ecc", "reset"),
                Call::new("ecc", "reset"),
            ],
            "コンポーネントに触る前に ECC が Idle まで片付いていない: {calls:?}"
        );
    }

    // -----------------------------------------------------------------
    // 036 — ECC の歩き戻し(実 ECC の `EV_UNDO` 意味論)
    // -----------------------------------------------------------------

    /// **036 の本体**: 前の run の `ecc stop` が置いた `Ready` からは
    /// **`breakup` → `reset` → `reset`** の 3 手で `Idle` まで歩き戻す。
    ///
    /// 手計算の出典 = 実 ECC の SM(`GetBench/src/get/rc/BackEnd.cpp:250-270`):
    /// `EV_BREAK` が `Active(Ready) → Prepared`、`EV_UNDO` が `Prepared → Described` と
    /// `Described → Idle` の**各 1 段**。`reset` を 3 発撃つのではなく `breakup` から
    /// 始まるのは、`Active` からの `EV_UNDO` が**存在しない**(= 無音で無視される)ため。
    #[test]
    fn a_ready_ecc_is_walked_back_to_idle_before_the_run_starts() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params).ecc_in("Ready");
        let report = Sequencer::new(&params, &mut mock)
            .start_run(57, "", "2026-08-14T10:00:00.000")
            .unwrap();

        // 発行されたコマンド列(コンポーネントに触る前の 4 手)。
        assert_eq!(
            &mock.commands()[..4],
            &[
                Call::new("ecc", "status"),
                Call::new("ecc", "breakup"),
                Call::new("ecc", "reset"),
                Call::new("ecc", "reset"),
            ]
        );
        // 歩いた跡は audit に残る(silent 禁止 — 実機で効いているかはこれで見る)。
        assert_eq!(
            report.ecc_walk_back,
            vec![
                "status->Ready",
                "breakup->Prepared",
                "reset->Described",
                "reset->Idle",
            ]
        );
        assert_eq!(report.ecc_state_after_reset, "Idle");
    }

    /// `Idle` なら **1 コマンドも出さない**(状態を見てから歩くので、要らない段は歩かない)。
    /// `status` は観測であって操作ではない。
    #[test]
    fn an_idle_ecc_is_left_alone() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params).ecc_in("Idle");
        let report = Sequencer::new(&params, &mut mock)
            .start_run(57, "", "2026-08-14T10:00:00.000")
            .unwrap();

        assert_eq!(
            mock.commands().first(),
            Some(&Call::new("ecc", "status")),
            "run 開始の第 1 手は ECC の状態を**見る**こと: {:?}",
            mock.commands()
        );
        for action in ["breakup", "reset", "stop"] {
            assert!(
                !mock.commands().contains(&Call::new("ecc", action)),
                "Idle なのに {action} を撃った: {:?}",
                mock.commands()
            );
        }
        assert_eq!(report.ecc_walk_back, vec!["status->Idle"]);
        assert_eq!(report.ecc_state_after_reset, "Idle");
    }

    /// 前の run が `Running` のまま残っていたら **`stop` から**歩き戻す(4 手)。
    /// controller が死んだ後の復旧(E2E-G の状況)がこれ。
    #[test]
    fn a_running_ecc_is_stopped_first_and_then_walked_back_to_idle() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params).ecc_in("Running");
        let report = Sequencer::new(&params, &mut mock)
            .start_run(57, "", "2026-08-14T10:00:00.000")
            .unwrap();

        assert_eq!(
            &mock.commands()[..5],
            &[
                Call::new("ecc", "status"),
                Call::new("ecc", "stop"),
                Call::new("ecc", "breakup"),
                Call::new("ecc", "reset"),
                Call::new("ecc", "reset"),
            ]
        );
        assert_eq!(
            report.ecc_walk_back,
            vec![
                "status->Running",
                "stop->Ready",
                "breakup->Prepared",
                "reset->Described",
                "reset->Idle",
            ]
        );
    }

    /// **判断できない状態では run を始めない**(`Unknown` = ECC 不達 or 未知の綴り)。
    /// コンポーネントには 1 コマンドも出さない。
    #[test]
    fn an_ecc_in_an_unknown_state_refuses_to_start_the_run() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params).ecc_in("Unknown");
        let failure = Sequencer::new(&params, &mut mock)
            .start_run(57, "", "2026-08-14T10:00:00.000")
            .unwrap_err();

        assert_eq!(failure.stage, "ecc");
        assert!(failure.detail.contains("Unknown"), "{}", failure.detail);
        assert_eq!(mock.commands(), vec![Call::new("ecc", "status")]);
    }

    /// **無音で無視された段を見逃さない**(036 の肝): 実 ECC は未定義遷移をエラーにせず
    /// `Ignored` で返す(`StateMachine/src/dhsm/Engine.cpp:334-352`)。つまり
    /// 「ok なのに状態が動いていない」が唯一の手がかりであり、そこで **run を止める**。
    /// 黙って先へ進むと `configure` がスキップされた ECC の上に run を建ててしまう。
    #[test]
    fn an_ecc_that_ignores_a_walk_back_step_in_silence_refuses_to_start_the_run() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params)
            .ecc_in("Ready")
            .ecc_ignores("breakup");
        let failure = Sequencer::new(&params, &mut mock)
            .start_run(57, "", "2026-08-14T10:00:00.000")
            .unwrap_err();

        assert_eq!(failure.stage, "ecc");
        assert!(failure.detail.contains("breakup"), "{}", failure.detail);
        assert!(failure.detail.contains("Ready"), "{}", failure.detail);
        // 空振りした 1 手で止まる(残りを撃ち続けない)。
        assert_eq!(
            mock.commands(),
            vec![Call::new("ecc", "status"), Call::new("ecc", "breakup")]
        );
    }

    /// ECC の状態が**取れなければ** run を始めない(見えない ECC の上に run を建てない)。
    /// コンポーネントには 1 コマンドも出ない。
    #[test]
    fn a_failing_ecc_status_stops_the_run_before_any_component_is_configured() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params).fail_ecc("status");
        let failure = Sequencer::new(&params, &mut mock)
            .start_run(57, "", "2026-08-14T10:00:00.000")
            .unwrap_err();

        assert_eq!(failure.stage, "ecc");
        assert!(failure.detail.contains("status"), "{}", failure.detail);
        assert_eq!(mock.commands(), vec![Call::new("ecc", "status")]);
    }

    /// 歩き戻しの途中の失敗も **run を始めない**(半端な ECC の上に run を建てない)。
    #[test]
    fn a_failing_ecc_reset_stops_the_run_before_any_component_is_configured() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params)
            .ecc_in("Ready")
            .fail_ecc("reset");
        let failure = Sequencer::new(&params, &mut mock)
            .start_run(57, "", "2026-08-14T10:00:00.000")
            .unwrap_err();

        assert_eq!(failure.stage, "ecc");
        assert!(failure.detail.contains("reset"), "{}", failure.detail);
        assert_eq!(
            mock.commands(),
            vec![
                Call::new("ecc", "status"),
                Call::new("ecc", "breakup"),
                Call::new("ecc", "reset"),
            ]
        );
    }

    /// **論点 B**: Arm の bind 失敗は指数バックオフでリトライし、**回数と所要を必ず残す**
    /// (silent 禁止)。
    ///
    /// 手計算の出典: バックオフは [`ARM_RETRY_BACKOFF`] = 20 ms から倍々。
    /// 3 回目で通るなら眠るのは 1 回目の後(20 ms)と 2 回目の後(40 ms)= **60 ms**。
    /// (030 実測の「2 本目は 3 回・0.118–0.121 s」がまさにこの形)。
    #[test]
    fn arm_retries_with_exponential_backoff_and_records_the_attempts() {
        let params = params_with(&[0]);
        // decoder の Arm を 2 回だけ断る(3 回目で通る)。非対称に 1 台だけ詰まらせる。
        let mut mock = MockTransport::new(&params).refuse_arm_times("decoder", 2);
        let report = Sequencer::new(&params, &mut mock)
            .start_run(57, "", "2026-08-14T10:00:00.000")
            .unwrap();

        assert_eq!(report.arm_retries.len(), 1, "{:?}", report.arm_retries);
        assert_eq!(report.arm_retries[0].component, "decoder");
        assert_eq!(report.arm_retries[0].attempts, 3);
        assert_eq!(
            mock.slept,
            Duration::from_millis(60),
            "20 ms + 40 ms の指数バックオフ"
        );
        // 詰まった相手だけ Arm を撃ち直す(通った相手は 1 回のまま)。
        assert_eq!(
            mock.actions("decoder"),
            vec!["Configure", "Arm", "Arm", "Arm", "Start"]
        );
        assert_eq!(
            mock.actions("graw-writer"),
            vec!["Configure", "Arm", "Start"]
        );
    }

    /// リトライ上限を使い切ったら **どれだけ粘ったかを言って** 失敗する(黙って諦めない)。
    ///
    /// 手計算の出典: [`ARM_MAX_ATTEMPTS`] = 6 回 = リトライ 5 回。眠る合計は
    /// 20+40+80+160+320 = **620 ms**(最後の試行の後は眠らない)。
    #[test]
    fn arm_gives_up_after_the_retry_budget_and_says_how_hard_it_tried() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params).fail_on("decoder", "Arm");
        let failure = Sequencer::new(&params, &mut mock)
            .start_run(57, "", "2026-08-14T10:00:00.000")
            .unwrap_err();

        assert_eq!(failure.stage, "Arm");
        assert!(
            failure.detail.contains("6 attempts"),
            "何回粘ったか言っていない: {}",
            failure.detail
        );
        assert_eq!(mock.slept, Duration::from_millis(620));
        assert_eq!(
            mock.actions("decoder")
                .iter()
                .filter(|a| *a == "Arm")
                .count(),
            ARM_MAX_ATTEMPTS as usize
        );
    }

    // -----------------------------------------------------------------
    // ConfigId の 3 相化(SPEC §3.1 / §8.2 / §9.2 v1.13、TODO/042)
    // -----------------------------------------------------------------

    /// 3 相に**非対称な** id を持つパラメタ(相の取り違え検出用)。
    fn per_phase_ids() -> ConfigIds {
        ConfigIds {
            describe: "zCobo-ZC706".to_string(),
            prepare: "prep-xcfg".to_string(),
            configure: "pulser".to_string(),
        }
    }

    /// run シーケンスの describe / prepare / configure は**それぞれ自分の相の id** を運ぶ。
    /// id を使わない `start` には `config_id` を載せない(SPEC §8.2)。
    #[test]
    fn the_run_sequence_sends_the_id_of_each_phase() {
        let params = ControllerParams {
            config_ids: per_phase_ids(),
            ..params_with(&[0])
        };
        let mut mock = MockTransport::new(&params);
        Sequencer::new(&params, &mut mock)
            .start_run(57, "", "2026-08-14T10:00:00.000")
            .unwrap();

        assert_eq!(
            mock.ecc_request("describe").unwrap()["config_id"],
            json!("zCobo-ZC706")
        );
        assert_eq!(
            mock.ecc_request("prepare").unwrap()["config_id"],
            json!("prep-xcfg")
        );
        assert_eq!(
            mock.ecc_request("configure").unwrap()["config_id"],
            json!("pulser")
        );
        assert_eq!(mock.ecc_request("start").unwrap().get("config_id"), None);
    }

    /// DataLinkSet(`configure`)に載るのは **configure 相**の id。
    #[test]
    fn configure_request_uses_the_configure_phase_id() {
        let params = ControllerParams {
            config_ids: per_phase_ids(),
            ..params_with(&[0])
        };
        let mut mock = MockTransport::new(&params);
        let sequencer = Sequencer::new(&params, &mut mock);
        let links = vec![RouterLink {
            cobo_id: 0,
            router_ip: "10.0.0.1".to_string(),
            router_port: 46005,
        }];
        assert_eq!(
            sequencer.configure_request(&links)["config_id"],
            json!("pulser")
        );
    }

    /// logbook `run_start`(SPEC §9.2 v1.13): 同値なら `config_id` はその値・`config_ids` は
    /// **出さない**。非同値なら `config_id` は configure 相・`config_ids` に 3 相全部。
    #[test]
    fn run_start_config_fields_follow_the_nullable_rule() {
        assert_eq!(
            run_start_config_fields(&ConfigIds::same("default")),
            ("default".to_string(), None)
        );
        assert_eq!(
            run_start_config_fields(&per_phase_ids()),
            ("pulser".to_string(), Some(per_phase_ids()))
        );
    }

    /// DataLinkSet の形(SPEC §8.2 の実機の罠: `CoBo[k]` 形式・大文字 `TCP`)。
    #[test]
    fn configure_request_carries_the_real_wire_quirks() {
        let params = params_with(&[0, 1]);
        let mut mock = MockTransport::new(&params);
        let sequencer = Sequencer::new(&params, &mut mock);
        let links = vec![
            RouterLink {
                cobo_id: 0,
                router_ip: "10.0.0.1".to_string(),
                router_port: 46005,
            },
            RouterLink {
                cobo_id: 1,
                router_ip: "10.0.0.1".to_string(),
                router_port: 46006,
            },
        ];
        assert_eq!(
            sequencer.configure_request(&links),
            json!({
                "action": "configure",
                "config_id": "mock",
                "links": [
                    {"sender": "CoBo[0]", "router_ip": "10.0.0.1", "router_port": 46005, "type": "TCP"},
                    {"sender": "CoBo[1]", "router_ip": "10.0.0.1", "router_port": 46006, "type": "TCP"},
                ],
            })
        );
    }

    /// **巻き戻し**: Configure が receiver0 で失敗 → 実行済み(graw-writer, decoder)だけ畳む。
    /// 失敗した receiver0 と、触っていない receiver1 には Stop/Reset を送らない。
    #[test]
    fn configure_failure_rolls_back_only_the_components_already_commanded() {
        let params = params_with(&[0, 1]);
        let mut mock = MockTransport::new(&params).fail_on("receiver0", "Configure");
        let failure = Sequencer::new(&params, &mut mock)
            .start_run(57, "", "2026-08-14T10:00:00.000")
            .unwrap_err();

        assert_eq!(failure.stage, "Configure");
        assert_eq!(
            mock.commands(),
            vec![
                // 034/036: run 開始は必ず ECC を Idle へ戻すところから(論点 A)。
                // このモックの ECC は既に `Idle` なので `status` で確かめるだけ。失敗しても
                // ECC は Idle = **一番安全な状態**のままなので、ここを巻き戻す必要はない。
                Call::new("ecc", "status"),
                Call::new("graw-writer", "Configure"),
                Call::new("decoder", "Configure"),
                Call::new("receiver0", "Configure"), // ここで失敗
                // 巻き戻しは上流から(= 開始順の逆)
                Call::new("decoder", "Stop"),
                Call::new("decoder", "Reset"),
                Call::new("graw-writer", "Stop"),
                Call::new("graw-writer", "Reset"),
            ]
        );
        assert!(
            mock.actions("receiver1").is_empty(),
            "触っていない相手を畳まない"
        );
    }

    /// **巻き戻し**(発注書の指定シナリオ): Arm が receiver1 で失敗 → 以降を実行せず
    /// (Start も ecc も出さない)、Configure 済みの 4 台を上流から畳む。
    #[test]
    fn arm_failure_stops_every_component_it_already_commanded_and_never_starts() {
        let params = params_with(&[0, 1]);
        let mut mock = MockTransport::new(&params).fail_on("receiver1", "Arm");
        let failure = Sequencer::new(&params, &mut mock)
            .start_run(57, "", "2026-08-14T10:00:00.000")
            .unwrap_err();

        assert_eq!(failure.stage, "Arm");
        let calls = mock.commands();
        assert!(
            !calls.iter().any(|c| c.action == "Start"),
            "巻き戻し中に Start を出さない: {calls:?}"
        );
        // 034/036: 先頭の歩き戻し(ここは既に `Idle` なので `status` だけ)は通る。
        // **ECC を走らせる段**(describe 以降)へ進まないことが、この主張の中身である。
        assert!(
            !calls
                .iter()
                .any(|c| c.component == "ecc" && c.action != "status"),
            "ECC を走らせる段へ進まない: {calls:?}"
        );
        // 実行済み 4 台すべてが Stop → Reset を受ける(Stop は Armed からは断られるので
        // Reset が Idle まで戻す = 半端な状態を残さない)。
        for name in ["receiver1", "receiver0", "decoder", "graw-writer"] {
            let actions = mock.actions(name);
            assert!(
                actions.ends_with(&["Stop".to_string(), "Reset".to_string()]),
                "{name} の巻き戻しが Stop → Reset で終わっていない: {actions:?}"
            );
        }
    }

    /// ecc 段の失敗も巻き戻す(全コンポーネントが Running のまま残らない)。
    #[test]
    fn ecc_failure_rolls_back_every_started_component() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params).fail_ecc("start");
        let failure = Sequencer::new(&params, &mut mock)
            .start_run(57, "", "2026-08-14T10:00:00.000")
            .unwrap_err();

        assert_eq!(failure.stage, "ecc");
        for name in ["graw-writer", "decoder", "receiver0"] {
            let actions = mock.actions(name);
            assert_eq!(
                actions,
                vec!["Configure", "Arm", "Start", "Stop", "Reset"],
                "{name}"
            );
        }
    }

    /// receiver が router_port を返さなければ Arm 段の失敗として巻き戻す
    /// (DataLinkSet を推測で組まない = silent failure を作らない)。
    #[test]
    fn a_receiver_without_a_router_port_fails_the_arm_stage() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params);
        mock.metrics
            .insert("receiver0".to_string(), json!({"frames": 0}));
        let failure = Sequencer::new(&params, &mut mock)
            .start_run(57, "", "2026-08-14T10:00:00.000")
            .unwrap_err();
        assert_eq!(failure.stage, "Arm");
        assert!(failure.detail.contains("router_port"), "{}", failure.detail);
    }

    // -----------------------------------------------------------------
    // 停止シーケンス(SPEC §1.3)
    // -----------------------------------------------------------------

    /// 正常停止: EOS が自然に流れ切れば receiver へ強制 Stop を撃たない…
    /// ではなく、**畳む段**の Stop は撃つ。強制 EOS(待たずの Stop)は起きない。
    #[test]
    fn normal_stop_waits_for_eos_and_does_not_force_it() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params).with_eos_closed();
        let report = Sequencer::new(&params, &mut mock).stop_run(StopMode::Normal);

        assert!(report.eos_closed);
        assert!(!report.forced_eos, "EOS が来ているのに強制してはいけない");
        assert_eq!(report.reason, "normal");
        assert!(report.ok);
        assert_eq!(mock.slept, Duration::ZERO, "待たずに閉じたなら眠らない");

        // ecc stop が最初、コンポーネントは上流から Stop → Reset。
        assert_eq!(
            mock.commands(),
            vec![
                Call::new("ecc", "stop"),
                Call::new("receiver0", "Stop"),
                Call::new("receiver0", "Reset"),
                Call::new("decoder", "Stop"),
                Call::new("decoder", "Reset"),
                Call::new("graw-writer", "Stop"),
                Call::new("graw-writer", "Reset"),
            ]
        );
    }

    /// **強制 EOS 分岐**: EOS が来ないまま `eos_timeout` を超えたら receiver へ Stop を撃つ。
    /// 撃った receiver へは畳む段で Stop を撃ち直さない(Reset だけ)。
    #[test]
    fn stop_forces_eos_through_the_receivers_after_the_timeout() {
        let params = params_with(&[0]);
        // 1 回の EOS 判定 = graw-writer への GetStatus 1 回(files_open != 0 でそこで打ち切る)。
        // 最初の待ちは「即時判定 1 回 + 200 ms 毎に 25 回(= eos_timeout 5 s)」= 26 回で
        // 諦める。27 回目(= 強制 EOS 直後の判定)で閉じる形にすると、眠った合計時間は
        // ちょうど eos_timeout になる。
        let mut mock = MockTransport::new(&params);
        mock.eos_after_polls = Some(26);
        let report = Sequencer::new(&params, &mut mock).stop_run(StopMode::Normal);

        assert!(report.forced_eos, "5 秒不達なら強制 EOS(SPEC §1.3)");
        assert!(report.eos_closed, "強制 EOS の後は閉じる");
        assert_eq!(
            mock.slept, params.eos_timeout,
            "諦めるまでちょうど eos_timeout 待つ"
        );

        let commands = mock.commands();
        assert_eq!(commands[0], Call::new("ecc", "stop"));
        assert_eq!(commands[1], Call::new("receiver0", "Stop"), "強制 EOS");
        // 畳む段では receiver へ Stop を撃ち直さない。
        assert_eq!(
            mock.actions("receiver0"),
            vec!["Stop", "Reset"],
            "強制 EOS の Stop を二重に撃たない"
        );
    }

    /// **中止経路**(SPEC §1.3 v1.6): 待たずに強制 EOS。**run がクローズする前に
    /// decoder を Reset しない**。run_stop は ok=false / reason="abort:..."。
    #[test]
    fn abort_closes_with_eos_before_resetting_the_decoder() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params).with_eos_closed();
        let report =
            Sequencer::new(&params, &mut mock).stop_run(StopMode::Abort("overflow".to_string()));

        assert!(!report.ok);
        assert_eq!(report.reason, "abort:overflow");
        assert!(report.forced_eos, "中止は待たずに強制 EOS(v1.6)");

        // 「EOS が流れ切ったと観測した」時点 = 強制 Stop の直後に来る GetStatus。
        let forced_stop = mock.index_of("receiver0", "Stop").unwrap();
        let decoder_reset = mock.index_of("decoder", "Reset").unwrap();
        let eos_observed = mock
            .calls
            .iter()
            .enumerate()
            .position(|(i, c)| {
                i > forced_stop && c.component == "decoder" && c.action == "GetStatus"
            })
            .unwrap();
        assert!(
            eos_observed < decoder_reset,
            "run クローズの確認より前に decoder を Reset している: {:?}",
            mock.calls
        );
        // decoder の Stop も EOS 確認の後(= 送出を打ち切らせない)。
        let decoder_stop = mock.index_of("decoder", "Stop").unwrap();
        assert!(eos_observed < decoder_stop);
        // 中止でも ecc stop は最初に試す(不達でも続行する)。
        assert_eq!(mock.commands()[0], Call::new("ecc", "stop"));
    }

    /// 中止は ecc が応答しなくても最後まで進む(SPEC §1.3 v1.6「不達でも続行」)。
    #[test]
    fn abort_continues_even_when_ecc_stop_fails() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params)
            .with_eos_closed()
            .fail_ecc("stop");
        let report =
            Sequencer::new(&params, &mut mock).stop_run(StopMode::Abort("ecc-dead".to_string()));

        assert_eq!(report.reason, "abort:ecc-dead");
        assert!(
            !report.notes.is_empty(),
            "ecc stop の失敗を silent にしない"
        );
        for name in ["receiver0", "decoder", "graw-writer"] {
            assert_eq!(mock.actions(name), vec!["Stop", "Reset"], "{name}");
        }
    }

    /// 正常停止のつもりで ecc stop が通らなければ中止扱いに落ちる(ok=false)。
    #[test]
    fn a_normal_stop_whose_ecc_stop_fails_is_reported_as_an_abort() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params)
            .with_eos_closed()
            .fail_ecc("stop");
        let report = Sequencer::new(&params, &mut mock).stop_run(StopMode::Normal);
        assert!(!report.ok);
        assert_eq!(report.reason, "abort:ecc-stop-failed");
    }

    /// EOS が強制しても流れ切らなければ `error:eos-timeout`(silent にしない)。
    #[test]
    fn stop_reports_an_error_when_eos_never_propagates() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params); // eos_after_polls = None = 永久に閉じない
        let report = Sequencer::new(&params, &mut mock).stop_run(StopMode::Normal);
        assert!(!report.ok);
        assert_eq!(report.reason, "error:eos-timeout");
        assert!(!report.eos_closed);
        assert!(report
            .notes
            .iter()
            .any(|n| n.contains("EOS did not propagate")));
    }

    // -----------------------------------------------------------------
    // run_stop の材料(SPEC §9.2)
    // -----------------------------------------------------------------

    /// counters / files を GetStatus 実測から充填する。root-sink 由来の 3 項目は
    /// 取得手段が無いので `None`(025 で `Counters` が `Option<u64>` 化された —
    /// SPEC v1.10 §9.2。016 時点は非 Option で `0` 埋めだった)。
    #[test]
    fn stop_fills_counters_and_files_from_get_status() {
        let params = params_with(&[0, 1]);
        let mut mock = MockTransport::new(&params).with_eos_closed();
        // 非対称な実測値(取り違え検出用)。
        mock.metrics.insert(
            "receiver0".to_string(),
            json!({"frames": 3852, "overflow_frames": 3, "router_port": 46100}),
        );
        mock.metrics.insert(
            "receiver1".to_string(),
            json!({"frames": 3849, "overflow_frames": 5, "router_port": 46101}),
        );
        mock.metrics
            .insert("decoder".to_string(), json!({"eos_in": 9, "malformed": 4}));
        mock.metrics.insert(
            "graw-writer".to_string(),
            json!({
                "files_open": 0,
                "files": [
                    {"path": "run0057/CoBo0_AsAd0_ts_0000.graw", "bytes": 30_108_672u64},
                    {"path": "run0057/CoBo0_AsAd1_ts_0000.graw", "bytes": 30_108_100u64},
                ],
            }),
        );

        let report = Sequencer::new(&params, &mut mock).stop_run(StopMode::Normal);

        assert_eq!(report.counters.frames.get("0"), Some(&3852));
        assert_eq!(report.counters.frames.get("1"), Some(&3849));
        assert_eq!(report.counters.overflow_frames, Some(8)); // 3 + 5
        assert_eq!(report.counters.malformed, Some(4));
        // root-sink 由来(REP が無い)は取得不能 = null(0 と混同しない — 025, SPEC v1.10 §9.2)。
        assert_eq!(report.counters.events_built, None);
        assert_eq!(report.counters.events_incomplete, None);
        assert_eq!(report.counters.late_fragments, None);
        assert_eq!(
            report.files,
            vec![
                OutputFile {
                    path: "run0057/CoBo0_AsAd0_ts_0000.graw".to_string(),
                    bytes: 30_108_672,
                },
                OutputFile {
                    path: "run0057/CoBo0_AsAd1_ts_0000.graw".to_string(),
                    bytes: 30_108_100,
                },
            ]
        );
    }

    /// 設定にある CoBo が不達でも `frames` の鍵は出す(欠落を隠さない)。
    #[test]
    fn counters_keep_a_key_for_every_configured_cobo() {
        let params = params_with(&[0, 1]);
        let statuses = vec![];
        let counters = counters_from(&params, &statuses);
        assert_eq!(counters.frames.get("0"), Some(&0));
        assert_eq!(counters.frames.get("1"), Some(&0));
    }

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

    // -----------------------------------------------------------------
    // ジオメトリ由来の run_start 材料(SPEC §9.2)
    // -----------------------------------------------------------------

    /// `expected_fragments` = ジオメトリが申告する (cobo, asad) 在庫。
    /// sha256 は空でない 64 桁の 16 進(ファイルの実ハッシュ)。
    #[test]
    fn geometry_record_reports_the_sha256_and_the_expected_fragments() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/geometry_2cobo_fake.dat");
        let (record, fragments) = geometry_record(&path).unwrap();
        assert_eq!(record.path, path.display().to_string());
        assert_eq!(record.sha256.len(), 64);
        assert!(record.sha256.chars().all(|c| c.is_ascii_hexdigit()));
        // 合成 2-CoBo フィクスチャの申告どおり: CoBo 0 は AsAd 1 枚、CoBo 1 は 2 枚
        // (`grep -v '^#' tests/fixtures/geometry_2cobo_fake.dat | cut -f4,5 | sort -u` の実測)。
        assert_eq!(fragments, vec![(0, 0), (1, 0), (1, 1)]);
    }

    /// 同じファイルなら同じハッシュ、1 バイト違えば違うハッシュ(取り違え検出)。
    #[test]
    fn geometry_sha256_changes_with_the_file_content() {
        let dir = std::env::temp_dir().join(format!("tpcdaq-geo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.dat");
        let b = dir.join("b.dat");
        std::fs::write(&a, "0\t0\t0\t0\tU\t1\t2\n").unwrap();
        std::fs::write(&b, "0\t0\t0\t1\tU\t1\t2\n").unwrap();
        let sha_a = geometry_record(&a).unwrap().0.sha256;
        let sha_b = geometry_record(&b).unwrap().0.sha256;
        assert_eq!(sha_a, geometry_record(&a).unwrap().0.sha256);
        assert_ne!(sha_a, sha_b);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------
    // 操作権(SPEC §8.1)
    // -----------------------------------------------------------------

    fn test_controller(tag: &str) -> Controller {
        let mut params = params_with(&[0]);
        params.output_root =
            std::env::temp_dir().join(format!("tpcdaq-controller-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&params.output_root);
        Controller::new(params).unwrap()
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
}
