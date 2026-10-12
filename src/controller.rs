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
// ecc-bridge の応答(SPEC §8.2)
// ---------------------------------------------------------------------

/// ecc-bridge の 4 フィールド応答。`state` は `Off/Idle/Described/Prepared/Ready/Running/
/// Paused/Unknown`(ECC 不達は `Unknown`)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EccReply {
    pub ok: bool,
    pub state: String,
    /// **輸送・ブリッジ層**のエラー文字列(Ice 例外 / ECC の CmdException 本文 /
    /// リクエスト不正)。043 で意味も値も変えていない。
    #[serde(default)]
    pub error: String,
    /// **GET の error フラグ**(`NO_ERR`/`WHEN_DESCRIBE`/…、SPEC §8.2 v1.14 = TODO/043)。
    ///
    /// `state` とは**別の軸**である —— 実 ECC は describe に失敗しても state は `Idle` の
    /// ままで、フラグにだけ `WHEN_DESCRIBE` を抱える(041 D-1 実測。仕組みは
    /// `tools/ecc_bridge/ecc_core.hpp` の set/clear 規則ブロック)。controller はこれを
    /// **解釈せず素通しする**(表示は P4)。
    ///
    /// `#[serde(default)]`: このフィールドを知らない古い ecc-bridge が相手でも
    /// 応答の parse は落とさない(その場合は空文字 = 「載っていなかった」)。
    #[serde(default)]
    pub ecc_error: String,
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
    /// コンポーネントの歩き戻し(113)。Idle でなかったものだけ
    /// `"<名前>: <見えた状態>-><Reset 応答の状態>"` で積む(例 `"receiver0: Configured->Idle"`)。
    /// Running が居たときは先頭に EOS の扱いが 1 行載る(114): `"eos: closed"` /
    /// `"eos: did not propagate within <ms> ms"`(全員 Running)/ `"eos: not awaited — …"`
    /// (一部だけ Running)。空 = 全員 Idle だった(正常な姿)。**audit に残す**。
    pub component_walk_back: Vec<String>,
    /// 歩き戻しで閉じた前の run(115)。Running が 1 つも居なければ `None`。
    pub walked_back_run: Option<WalkedBackRun>,
}

/// 歩き戻しで閉じた前の run(115)= その `run_stop` の材料。書くのは呼び手
/// ([`start_run_blocking`] が `RunStart` の**前**に)。
#[derive(Debug, Clone, PartialEq)]
pub struct WalkedBackRun {
    /// 前の run の番号。状態調べの `GetStatus` で `run_number` を申告した全員が一致したときだけ
    /// `Ok`。食い違う/誰も申告しない → `Err(理由)` —— run_stop は書かない(推測で台帳を汚さない)。
    pub run: Result<u32, String>,
    /// EOS を receiver `Stop` で注入したか(run_stop の同名の項)。
    pub forced_eos: bool,
    /// EOS がチェーンを流れ切ったと観測できたか(一部だけ Running なら待たないので常に `false`)。
    pub eos_closed: bool,
    /// **Stop の後・Reset の前**に採った実績(graw-writer は Reset でファイル実績を手放す)。
    pub counters: Counters,
    pub files: Vec<OutputFile>,
    /// 材料集めの異常(`GetStatus` 不達など。silent にしない)。
    pub notes: Vec<String>,
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
    /// `walk-back`(113)/ `Configure` / `Arm` / `Start` / `ecc` / `geometry` のどこで落ちたか。
    pub stage: &'static str,
    pub detail: String,
    /// 巻き戻しで送ったコマンドのうち失敗したものの記録(silent にしない)。
    pub rollback_notes: Vec<String>,
    /// 歩き戻しで閉じた前の run(115)。start が後で落ちても前の run は閉じているので、
    /// 呼び手はこれで run_stop を書く。`Box` は `Result` の `Err` を小さく保つため
    /// (clippy::result_large_err)。
    pub walked_back_run: Option<Box<WalkedBackRun>>,
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
    /// 歩き戻しで閉じた前の run(115)。歩き戻しが置き、`start_run` の成功報告か
    /// [`Self::rollback`] の失敗報告が持ち出す(どの段で落ちても失われない)。
    walked_back_run: Option<WalkedBackRun>,
}

impl<'a> Sequencer<'a> {
    pub fn new(params: &'a ControllerParams, transport: &'a mut dyn Transport) -> Self {
        Self {
            params,
            transport,
            walked_back_run: None,
        }
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

        // 0'. コンポーネントも Idle へ歩き戻す(113)。前の run の停止で Reset が届かなかった
        //     コンポーネントは Configured 等のまま残り、下の Configure が断られ続ける。
        //     ここで Reset した相手は Idle に戻っているので `touched` には数えない。
        let mut component_walk_back = Vec::new();
        if let Err(detail) = self.walk_components_back_to_idle(&mut component_walk_back) {
            return Err(self.rollback("walk-back", detail, &touched));
        }

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
            component_walk_back,
            walked_back_run: self.walked_back_run.take(),
        })
    }

    /// 各コンポーネントを **`Idle` まで歩き戻す**(113 / 114、SPEC §1.3 v1.39)。
    ///
    /// まず全員の状態を**上流から** `GetStatus` で調べ(不達ならどれにも触る前に `Err`)、
    /// Idle でないものだけを戻す。§1.3 の遷移表で `Reset` は Configured / Armed / Running /
    /// Error のどこからでも Idle へ 1 手で戻すが、`Running` には先に `Stop`(EOS を流す)——
    /// `Reset` は送出待ちを打ち切るので、送出待ちがあると EOS ごと打ち切られる(v1.6 の
    /// 「run を閉じる前に Reset しない」と同じ理由。decoder なら EOS バリアの喪失)。
    ///
    /// * **全員 `Running`**(チェーンが無傷 = 典型は run 中に controller だけが落ちて再起動
    ///   した姿、114): **停止経路の 2 段そのもの**で前の run を閉じる ——
    ///   [`Self::wait_for_eos_or_quiesce`](自然 EOS か受信静止の早い方。`receiver::drain_task`
    ///   は Stop で即 break してカーネル受信バッファの未読分を捨てるので、静止を確かめてから
    ///   Stop する)→ 閉じていなければ全 receiver へ `Stop`(強制 EOS)→ [`Self::wait_for_eos`]
    ///   (上限 `eos_timeout`)→ 残り(強制 EOS を撃たなかったなら receiver も)へ `Stop`。
    ///   decoder の `Stop` は自分の EOS を出さない(全 receiver の EOS を受けたときだけ
    ///   出る)ので、この順でないと EOS は root-sink まで届かない。待ちの結果は跡の先頭に
    ///   `"eos: closed"` / `"eos: did not propagate within <ms> ms"` で 1 行残し、**どちらでも
    ///   start は止めない**(届かなければ後の `Reset` が打ち切り、それは各コンポーネントの
    ///   metrics で見える)。ECC は呼び手がこれより先に歩き戻しているので zCoBo は既に止まって
    ///   いる(停止経路の ecc stop → receiver Stop と同じ順)。
    /// * **一部だけ `Running`**(チェーンがどこかで既に Reset 済み): EOS は端まで届かず観測も
    ///   できないので待たない。跡の先頭に `"eos: not awaited — …"` を 1 行残し、`Running` の
    ///   ものは 113 と同じく `Stop` → `Reset`(Stop は Reset より先に全部済ませる)。
    ///
    /// Running が 1 つでも居たら、**Stop の後・Reset の前**に `GetStatus` で前の run の
    /// run_stop の材料を採り([`WalkedBackRun`]、115)、`self.walked_back_run` に置く。
    /// run 番号は状態調べの `run_number` の一致からだけ決める([`previous_run_number`])。
    ///
    /// 最後に Idle でない全員へ上流から `Reset`。コンポーネントごとの跡は
    /// `"<名前>: <状態>[-><Stop の着地>]-><Reset の着地>"`
    /// (例 `"receiver0: Configured->Idle"`、`"decoder: Running->Configured->Idle"`)。
    ///
    /// `Stop` / `Reset` が断られる/届かないコンポーネントがあれば `Err`(どれが、何の状態で
    /// 残ったかを文言に載せる)。`Stop` が通らなければ `Reset` へは進まない —— Idle でない
    /// まま Configure へ進んでも断られるだけなので、ここで止めて理由を明示する。
    fn walk_components_back_to_idle(&mut self, trail: &mut Vec<String>) -> Result<(), String> {
        let params = self.params;

        // 1. 状態調べ(上流から)。`steps` は (相手, 歩き戻し前の状態, 今の状態, 跡の 1 行) を
        //    上流から並べる。何をするかは**歩き戻し前の状態**で決め、失敗の文言は**今の状態**
        //    (直前の手が着地した状態)で名乗る。
        //    前の run の番号の申告(115)も `(名前, run_number)` で上流から控える。
        let mut steps = Vec::with_capacity(params.components.len());
        let mut reported_runs = Vec::new();
        for component in params.components.iter().rev() {
            let response = self.apply(component, &Command::GetStatus)?;
            let state = response.state;
            if let Some(run) = response.run_number {
                reported_runs.push((component.name.as_str(), run));
            }
            steps.push((
                component,
                state,
                state,
                format!("{}: {state}", component.name),
            ));
        }
        let running = steps
            .iter()
            .filter(|(_, before, _, _)| *before == ComponentState::Running)
            .count();
        let chain_intact = running > 0 && running == steps.len();
        // run_stop(115)に載る EOS の顛末。
        let mut forced_eos = false;
        let mut eos_closed = false;

        // 2. チェーンが無傷なら前の run を EOS で閉じる(停止経路の 2 段そのもの、114 追補)。
        //    第一段 = 自然 EOS か受信静止の早い方まで待つ —— `receiver::drain_task` は Stop で
        //    即 break し、カーネル受信バッファの未読分を数えずに捨てるので、静止を確かめてから
        //    Stop する。閉じていなければ全 receiver へ Stop(強制 EOS)→ 第二段 = EOS 待ち。
        if chain_intact {
            let mut notes = Vec::new();
            let mut closed = self.wait_for_eos_or_quiesce(&mut notes);
            forced_eos = !closed;
            if forced_eos {
                for (component, _, now, step) in steps.iter_mut() {
                    if matches!(component.kind, ComponentKind::Receiver { .. }) {
                        self.walk_back_step(component, now, &Command::Stop, step)?;
                    }
                }
                closed = self.wait_for_eos(&mut notes);
            }
            eos_closed = closed;
            if closed {
                info!(
                    forced_eos,
                    ?notes,
                    "previous run closed with EOS before the walk-back"
                );
                trail.push("eos: closed".to_string());
            } else {
                let line = format!(
                    "eos: did not propagate within {} ms",
                    params.eos_timeout.as_millis()
                );
                warn!(
                    ?notes,
                    "{line} — resetting anyway; Reset abandons whatever is still waiting to be \
                     sent, EOS included (visible as messages_abandoned / eos_abandoned)"
                );
                trail.push(line);
            }
            // 停止経路の畳む段と同じ: 強制 EOS で既に Stop 済みの receiver へは撃ち直さない。
            for (component, _, now, step) in steps.iter_mut() {
                let already_stopped =
                    forced_eos && matches!(component.kind, ComponentKind::Receiver { .. });
                if !already_stopped {
                    self.walk_back_step(component, now, &Command::Stop, step)?;
                }
            }
        } else if running > 0 {
            let line = "eos: not awaited — the chain is broken (not every component is Running)";
            warn!("{line}; Running components are stopped and reset one by one");
            trail.push(line.to_string());
            // Running のものを上流から Stop(Reset より先に全部 —— 下の材料集めのため)。
            // receiver の Stop は EOS を注入する(待たないので閉じたかは観測しない)。
            for (component, before, now, step) in steps.iter_mut() {
                if *before == ComponentState::Running {
                    self.walk_back_step(component, now, &Command::Stop, step)?;
                    forced_eos |= matches!(component.kind, ComponentKind::Receiver { .. });
                }
            }
        }

        // 2'. 前の run の run_stop の材料(115)。Stop の後・Reset の前 —— graw-writer は
        //     Reset でファイル実績を手放す(停止経路の第 3 段と同じ理由)。
        if running > 0 {
            let mut notes = Vec::new();
            let statuses = self.collect_status(&mut notes);
            let run = previous_run_number(&reported_runs);
            if let Err(why) = &run {
                warn!("{why}");
            }
            self.walked_back_run = Some(WalkedBackRun {
                run,
                forced_eos,
                eos_closed,
                counters: counters_from(params, &statuses),
                files: files_from(&statuses),
                notes,
            });
        }

        // 3. Idle でない全員を上流から Reset。
        for (component, before, mut now, mut step) in steps {
            if before == ComponentState::Idle {
                continue;
            }
            self.walk_back_step(component, &mut now, &Command::Reset, &mut step)?;
            warn!(
                component = component.name,
                walked = step,
                "component was not Idle at run start — walked back"
            );
            trail.push(step);
        }
        Ok(())
    }

    /// 歩き戻しの 1 手(`Stop` / `Reset`)。通ったら `now` を着地した状態に進め、跡 `step` に
    /// `"->状態"` で足す。断られた/届かなければ、どれが・**今**何の状態で残ったか(`now` =
    /// 直前の手の着地。例: Stop の後の Reset が断られたら Configured)を文言に載せて `Err`。
    fn walk_back_step(
        &mut self,
        component: &ComponentEndpoint,
        now: &mut ComponentState,
        command: &Command,
        step: &mut String,
    ) -> Result<(), String> {
        let left = *now;
        let landed = self.apply(component, command).map_err(|detail| {
            format!(
                "{detail} — {} was left {left}; refusing to start a run",
                component.name
            )
        })?;
        *now = landed.state;
        step.push_str(&format!("->{}", landed.state));
        Ok(())
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
            walked_back_run: self.walked_back_run.take().map(Box::new),
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

        // 2. EOS 伝播待ち(第一段)。中止は待たずに即・強制する(v1.6)。
        //    正常停止は「自然 EOS の完了」**または**「全 receiver の受信静止」の早い方まで
        //    (SPEC §1.3 v1.12 / TODO/033-E)。実機では EOF が構造的に来ないので、
        //    静止検出が無いとここは毎停止 eos_timeout をまるごと空振りする。
        let mut eos_closed = match mode {
            StopMode::Normal => self.wait_for_eos_or_quiesce(&mut notes),
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

    /// 停止**第一段**の待ち(SPEC §1.3 v1.12、TODO/033-E)。
    ///
    /// 「自然 EOS の完了」**または**「全 receiver の受信静止」の早い方で返る:
    ///
    /// * `true` … 自然 EOS がチェーンを流れ切った(EOF 由来 = リプレイ・breakup 先行)。
    ///   呼び手は強制 EOS へ進まない。
    /// * `false` … 静止を検出した(実機の通常経路)か、ハード上限 `eos_timeout` に達した。
    ///   呼び手は receiver `Stop` で EOS を注入する。
    ///
    /// 静止 = 全 receiver の受信バイト数が `eos_quiesce` のあいだ 1 バイトも増えないこと。
    /// `ecc stop` の flush 済み在飛データを**飲み切ってから**畳むための待ちであり、
    /// 「即・強制 EOS」にしない理由そのもの(ロスレス規約の一部)。
    fn wait_for_eos_or_quiesce(&mut self, notes: &mut Vec<String>) -> bool {
        let params = self.params;
        let mut waited = Duration::ZERO;
        // 直前サンプルと、それが何も動かないまま続いた時間。
        let mut previous: Option<Vec<Option<u64>>> = None;
        let mut still = Duration::ZERO;
        // 一度不達だった receiver(以後は問い合わせない —— 下記 `receiver_bytes` 参照)。
        let mut gone: Vec<String> = Vec::new();
        loop {
            if self.eos_complete(notes) {
                return true;
            }
            let sample = self.receiver_bytes(&mut gone, notes);
            match (&previous, &sample) {
                // 1 サンプル分(= eos_poll)だけ何も動かなかった。
                (Some(before), Some(now)) if before == now => still += params.eos_poll,
                // 変化した / 判定できない / 最初のサンプル → 静止の計測をやり直す。
                _ => still = Duration::ZERO,
            }
            previous = sample;
            if still >= params.eos_quiesce {
                info!(
                    quiesce_ms = params.eos_quiesce.as_millis(),
                    waited_ms = waited.as_millis(),
                    "receivers are quiet — draining is done, forcing EOS now (SPEC §1.3 v1.12)"
                );
                return false;
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

    /// 全 receiver の受信バイト数(`components` の receiver だけ、順序どおり)。
    ///
    /// * 要素 `None` … その receiver が**不達**。drain できない相手を待っても無意味なので
    ///   「静止」として扱う(不達そのものは第二段の `Stop` 不達で可視になる)。
    /// * 戻り値 `None` … 到達できたのに `bytes` が読めない receiver が居る。静止を
    ///   判定する材料が無いので**静止とみなさず**ハード上限に委ねる(尻尾を落とさない側に倒す)。
    ///   欠落は note に出す(silent failure を作らない)。
    ///
    /// `gone` は「一度不達だった receiver」の名前。**2 度目からは問い合わせない** ——
    /// 不達の REQ は毎回 `command_timeout`(既定 5 s)を丸ごと燃やすので、poll 毎に
    /// 撃ち直すと静止判定そのものが分単位に化ける(033-D の F1 で実測 55 s)。
    /// 静止扱いは変わらないので判定結果には影響しない。
    fn receiver_bytes(
        &mut self,
        gone: &mut Vec<String>,
        notes: &mut Vec<String>,
    ) -> Option<Vec<Option<u64>>> {
        let params = self.params;
        let mut out = Vec::new();
        for component in params.components.iter() {
            if !matches!(component.kind, ComponentKind::Receiver { .. }) {
                continue;
            }
            if gone.iter().any(|name| name == &component.name) {
                out.push(None);
                continue;
            }
            match self
                .transport
                .command(&component.endpoint, &Command::GetStatus)
            {
                Err(e) => {
                    push_once(notes, format!("{}: GetStatus failed: {e}", component.name));
                    gone.push(component.name.clone());
                    out.push(None);
                }
                Ok(response) => {
                    let bytes = response
                        .metrics
                        .as_ref()
                        .and_then(|m| m.get("bytes"))
                        .and_then(Value::as_u64);
                    match bytes {
                        Some(bytes) => out.push(Some(bytes)),
                        None => {
                            push_once(
                                notes,
                                format!("{}: GetStatus metrics has no bytes", component.name),
                            );
                            return None;
                        }
                    }
                }
            }
        }
        Some(out)
    }

    /// EOS がチェーンを流れ切る(= root-sink が run をクローズできる)まで待つ。
    /// root-sink は REP を持たないので、**graw-writer / decoder の `GetStatus`** で代理観測する
    /// (SPEC §1.3 の「GetStatus と時間で監視」)。
    ///
    /// 強制 EOS の**後**の第二段はこちら —— 静止で切り上げてよいのは第一段だけで、
    /// ここは「EOS が実際に流れ切ったか」しか答えにならない(SPEC §1.3 v1.12 終端条項)。
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

    /// EOS 到達の判定(3 点とも満たすこと):
    /// - decoder: `eos_in` が上流 CoBo 数に達した(全 receiver の EOS を受け取った)
    /// - decoder: `eos_out >= 1`(**自分の EOS を送り終えた** = TODO/033-C)
    /// - graw-writer: `files_open == 0`(全期待ソースの EOS 受領で全ファイルを閉じた、SPEC §7)
    ///
    /// 3 点目が要る理由: root-sink が run を閉じられるのは **decoder の EOS が届いたとき**
    /// であり、`eos_in` は「decoder が受けた」までしか言っていない。root-sink が停滞して
    /// PUSH の HWM が埋まっていると decoder は EOS を送れずに居るので、2 点判定では
    /// 「EOS 未達なのに reason=normal」になり得た(033 併発見)。
    fn eos_complete(&mut self, notes: &mut Vec<String>) -> bool {
        let params = self.params;
        let expected_eos = params.cobos.len() as u64;
        for component in params.components.iter() {
            // receiver は EOS の**作り手**なので観測点にしない(SPEC §1.3)。
            if matches!(component.kind, ComponentKind::Receiver { .. }) {
                continue;
            }
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
            let read = |key: &'static str, notes: &mut Vec<String>| -> Option<u64> {
                let value = response
                    .metrics
                    .as_ref()
                    .and_then(|m| m.get(key))
                    .and_then(Value::as_u64);
                if value.is_none() {
                    push_once(
                        notes,
                        format!("{}: GetStatus metrics has no {key}", component.name),
                    );
                }
                value
            };
            let done = match component.kind {
                ComponentKind::Decoder => {
                    let eos_in = read("eos_in", notes);
                    let eos_out = read("eos_out", notes);
                    match (eos_in, eos_out) {
                        (Some(eos_in), Some(eos_out)) => eos_in >= expected_eos && eos_out >= 1,
                        _ => false,
                    }
                }
                ComponentKind::GrawWriter => matches!(read("files_open", notes), Some(0)),
                ComponentKind::Receiver { .. } => continue,
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
        // 所要を必ず残す(057: 実 ECC の configure は初回に分単位かかりうる。elapsed が
        // 無いと「1 本が長い」のか「本数の合計」なのか事後に切り分けられない)。
        let started = std::time::Instant::now();
        let outcome = self.transport.ecc(request);
        let elapsed_ms = started.elapsed().as_millis() as u64;
        match outcome {
            Err(e) => Err(format!("ecc {action} unreachable: {e}")),
            Ok(reply) if !reply.ok => Err(format!(
                "ecc {action} failed in state {}: {}",
                reply.state, reply.error
            )),
            Ok(reply) => {
                info!(
                    action,
                    state = reply.state,
                    elapsed_ms,
                    "ecc command applied"
                );
                Ok(reply.state)
            }
        }
    }
}

/// 歩き戻しで閉じた前の run の番号(115)。`reported` は状態調べで `run_number` を申告した
/// コンポーネント(上流から `(名前, 番号)`)で、**全員が一致したときだけ** `Ok`。
/// 食い違う/誰も申告しない → `Err(理由)`。状態ファイルの `next_run - 1` は `run/next` で
/// 手動変更され得るので使わない(推測で台帳を汚さない)。
fn previous_run_number(reported: &[(&str, u32)]) -> Result<u32, String> {
    let Some(&(_, first)) = reported.first() else {
        return Err(
            "no component reported a run_number — the previous run is unknown; \
             run_stop not written"
                .to_string(),
        );
    };
    if reported.iter().all(|&(_, run)| run == first) {
        return Ok(first);
    }
    let listed: Vec<String> = reported
        .iter()
        .map(|(name, run)| format!("{name}={run}"))
        .collect();
    Err(format!(
        "components disagree on the previous run number ({}) — run_stop not written",
        listed.join(", ")
    ))
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
        // `ecc_error` は **GET の error フラグ**をそのまま載せるだけ(SPEC §8.2 v1.14 =
        // TODO/043)。controller は解釈も加工もしない —— 表示の判断は UI(P4)の仕事。
        // ecc-bridge に届かなかったときは `state` と同じく `Unknown`: 「聞けていない」を
        // 「ECC がエラー無しと申告した(NO_ERR)」に化けさせない。
        let ecc = match transport.ecc(&json!({"action": "status"})) {
            Ok(reply) => json!({
                "ok": reply.ok,
                "state": reply.state,
                "error": reply.error,
                "ecc_error": reply.ecc_error,
            }),
            Err(e) => json!({"ok": false, "state": "Unknown", "error": e, "ecc_error": "Unknown"}),
        };
        // 078 レーン B §10-4: `params <id> · exceptions N` の元(UI はレーン C)。
        // status は必ず 200 を返す既存の姿勢を崩さない —— params 側の不整合は `notes` へ
        // 積んで `params: null` にする(このリクエスト自体は失敗させない)。
        let params_status = params_status_json(&controller, &mut notes);
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
            "params": params_status,
            "log_posts": {
                "accepted": controller.log_posts.accepted.load(Ordering::Relaxed),
                "rejected": controller.log_posts.rejected.load(Ordering::Relaxed),
                "slow_values_not_logged": controller
                    .log_posts
                    .slow_values_not_logged
                    .load(Ordering::Relaxed),
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

/// `GET /api/status` の `params: {id, exceptions} | null`(078 レーン B §10-4)。
/// current.json が無い、ecc_config_dir 未設定、render 失敗のいずれも `null` + `notes` へ
/// 一言(silent にしない)。status 応答そのものは落とさない。
fn params_status_json(controller: &Controller, notes: &mut Vec<String>) -> Value {
    let current = match controller.current_params() {
        Ok(Some(current)) => current,
        Ok(None) => return Value::Null,
        Err(e) => {
            notes.push(format!("params: {e}"));
            return Value::Null;
        }
    };
    let Some(ecc_config_dir) = &controller.params.ecc_config_dir else {
        notes.push("params: ecc_config_dir is not configured".to_string());
        return Value::Null;
    };
    match controller.render_with_template_dir(ecc_config_dir, &current.params) {
        Ok(rendered) => json!({
            "id": rendered.config_id,
            "exceptions": current.params.exceptions.len(),
        }),
        Err(e) => {
            notes.push(format!("params: {e}"));
            Value::Null
        }
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
    // 124(Fable B-2): power コマンドの実行中も弾く(HV ON の検査の後に run が始まって、
    // ランプアップ中のデータを取らないように)。同じロック区間で判定する。
    {
        let mut run_state = lock(&controller.run);
        let refused = if run_state.phase != Phase::Idle {
            Some(format!("controller is {} — not Idle", run_state.phase))
        } else if run_state.power_inflight.is_some() {
            Some(RUN_START_POWER_IN_FLIGHT.to_string())
        } else {
            None
        };
        if let Some(why) = refused {
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

    // 078 レーン B §10-3: current.json があれば prepare/configure の config_id を
    // その `<id>` に差し替える(describe は TOML のまま)。無ければ従来どおり TOML の id
    // (= `params_snapshot` が `None`)。**現在の params が壊れている / render できないのに
    // 黙って TOML の id へ後退はしない** —— run のデータ取得条件が記録と食い違うほうが
    // 静かなフォールバックより危険(CLAUDE.md「silent failure を作らない」)。
    let params_snapshot = match run_start_params_snapshot(controller) {
        Ok(snapshot) => snapshot,
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
    let params_id = params_snapshot
        .as_ref()
        .map(|(_, rendered)| rendered.config_id.clone());
    let params_field = params_snapshot.as_ref().map(|(current, rendered)| {
        json!({
            "id": rendered.config_id,
            "sha256": current.sha256,
            "exceptions": current.params.exceptions.len(),
        })
    });

    let mut params_for_run = controller.params.clone();
    if let Some(id) = &params_id {
        params_for_run.config_ids = ConfigIds {
            describe: params_for_run.config_ids.describe.clone(),
            prepare: id.clone(),
            configure: id.clone(),
        };
    }

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
        let mut sequencer = Sequencer::new(&params_for_run, &mut transport);
        sequencer.start_run(run, comment, &run_start_ts)
    };

    // 115: 歩き戻しで閉じた前の run の run_stop を、新しい run の RunStart より**前に**書く
    // (start が後で落ちても前の run は閉じているので書く)。要約は audit へ(null = 閉じた run 無し)。
    let walked_back_run = match &result {
        Ok(report) => report.walked_back_run.as_ref(),
        Err(failure) => failure.walked_back_run.as_deref(),
    };
    let previous_run_stop =
        walked_back_run.map(|closed| append_walked_back_run_stop(controller, closed));

    match result {
        Ok(report) => {
            let (geometry, expected_fragments) = geometry;
            let (config_id, config_ids) = run_start_config_fields(&params_for_run.config_ids);
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
                slow: controller.slow_snapshot(),
                params: params_field,
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
                    "ecc_walk_back": report.ecc_walk_back,
                    // 113: Idle でなかったコンポーネントを Reset で歩き戻した跡(空 = 全員 Idle)。
                    // 空でない = 前の run の停止で Reset が届かなかった、という運用の可視化。
                    "component_walk_back": report.component_walk_back,
                    // 115: 歩き戻しで閉じた前の run の run_stop の要約(`{"run", "notes"}`、
                    // run が null = 番号が決まらず書かなかった)。null = 閉じた run 無し。
                    "previous_run_stop": previous_run_stop,
                    // 078 レーン B §10-3: この run が使った params の `<id>`(無ければ null =
                    // TOML の config_id だけで走った)。
                    "params_id": params_id}),
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
                    "previous_run_stop": previous_run_stop,
                    "params_id": params_id,
                }),
                operator,
                false,
                Some(detail.clone()),
            );
            Err(detail)
        }
    }
}

/// 歩き戻しで閉じた前の run の `run_stop` を書く(115、Fable 裁定)。**新しい run の
/// `RunStart` より前に**呼ぶ(start が後で落ちても前の run は閉じているので書く)。
///
/// * 番号が決まらなければ(`closed.run` が `Err`)書かない —— 理由は要約の notes へ。
/// * `reason: "abort:controller-restart"`、`ok: false`。EOS の顛末と counters / files は
///   歩き戻しの材料そのまま。
/// * `duration_s` はログブックの**同じ番号の** `run_start.ts` から。無ければ `null`。
/// * ログブックの**最後の** `run_start` は照合だけに使い、番号が違えば note を残す。
///
/// 戻り値は audit `run/start.previous_run_stop` に載せる要約 `{"run": N | null, "notes": [...]}`
/// (書かなかった理由・照合の食い違いを silent にしない)。
fn append_walked_back_run_stop(controller: &Controller, closed: &WalkedBackRun) -> Value {
    let mut notes = closed.notes.clone();
    let run = match &closed.run {
        Ok(run) => *run,
        Err(why) => {
            notes.push(why.clone());
            return json!({"run": null, "notes": notes});
        }
    };

    let records = match crate::logbook::read_since(&controller.logbook_path, 0) {
        Ok(result) => result.records,
        Err(e) => {
            notes.push(format!("cannot read the logbook: {e}"));
            Vec::new()
        }
    };
    let starts: Vec<(u32, &str)> = records
        .iter()
        .filter_map(|line| match &line.record {
            LogbookRecord::RunStart { run, .. } => Some((*run, line.ts.as_str())),
            _ => None,
        })
        .collect();
    if let Some(&(last, _)) = starts.last() {
        if last != run {
            notes.push(format!(
                "the last run_start in the logbook is run {last}, not {run}"
            ));
        }
    }
    let duration_s = match starts.iter().rev().find(|(r, _)| *r == run) {
        None => {
            notes.push(format!(
                "no run_start for run {run} in the logbook — duration_s is null"
            ));
            None
        }
        Some((_, ts)) => {
            let seconds = seconds_since(ts, chrono::Local::now().fixed_offset());
            if seconds.is_none() {
                notes.push(format!(
                    "run_start.ts {ts:?} of run {run} is not RFC3339 — duration_s is null"
                ));
            }
            seconds
        }
    };

    warn!(
        run,
        eos_closed = closed.eos_closed,
        ?notes,
        "writing run_stop for the run the walk-back closed (the controller restarted mid-run)"
    );
    controller.append(LogbookRecord::RunStop {
        actor: ACTOR.to_string(),
        run,
        duration_s,
        ok: false,
        reason: "abort:controller-restart".to_string(),
        forced_eos: closed.forced_eos,
        eos_closed: closed.eos_closed,
        counters: closed.counters.clone(),
        files: closed.files.clone(),
        slow: controller.slow_snapshot(),
    });
    json!({"run": run, "notes": notes})
}

/// RFC3339 の `ts` から `now` までの秒(オフセットが違っても同じ瞬間で比べる)。
/// 読めなければ `None`。
fn seconds_since(ts: &str, now: chrono::DateTime<chrono::FixedOffset>) -> Option<f64> {
    let started = chrono::DateTime::parse_from_rfc3339(ts).ok()?;
    Some((now - started).num_milliseconds() as f64 / 1000.0)
}

/// [`start_run_blocking`] の params 部分だけを切り出したもの(078 レーン B §10-3)。
/// current.json が無ければ `Ok(None)`(= 従来どおり TOML の config_id)。
/// current.json はあるが検証/render に失敗したら `Err`(黙って TOML へ後退しない)。
#[allow(clippy::type_complexity)]
fn run_start_params_snapshot(
    controller: &Controller,
) -> Result<Option<(CurrentParams, crate::params::Rendered)>, String> {
    let Some(current) = controller.current_params()? else {
        return Ok(None);
    };
    let ecc_config_dir = controller.params.ecc_config_dir.as_ref().ok_or_else(|| {
        "params/current.json exists but ecc_config_dir is not configured — \
         refusing to start a run with a stale config_id"
            .to_string()
    })?;
    let rendered = controller.render_with_template_dir(ecc_config_dir, &current.params)?;
    Ok(Some((current, rendered)))
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
        duration_s: Some(duration_s),
        ok: report.ok,
        reason: report.reason.clone(),
        // 033-A: 停止の顛末は **run_stop(唯一の run 台帳)** にも載せる。ここに無いと
        // audit 行と突き合わせないと「どう閉じたか / 閉じ損ねたか」が読めない
        // (SPEC §9.2 v1.12。041 でこの不履行が実 run で確認された)。
        forced_eos: report.forced_eos,
        eos_closed: report.eos_closed,
        counters: report.counters.clone(),
        files: report.files.clone(),
        slow: controller.slow_snapshot(),
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
// パラメータ JSON REST(078 レーン B §10、SPEC §3.3)
// ---------------------------------------------------------------------

/// GET/PUT `/api/params` と PUT の 400/503 応答が共有する「ecc_config_dir 未設定 = 503」。
/// **黙って無効にしない**(CLAUDE.md)—— 発注書 §10 のとおり `{"error": "..."}` は固定文言。
const ECC_CONFIG_DIR_NOT_CONFIGURED: &str = "ecc_config_dir is not configured";

/// GET/PUT `/api/params` の共通の応答本体(078 §10「GET と同じ形を返す」)。
fn params_body(
    ecc_config_dir: &Path,
    current: &CurrentParams,
    rendered: &crate::params::Rendered,
) -> Value {
    let configure_path = ecc_config_dir.join(&rendered.configure_file_name);
    let prepare_path = ecc_config_dir.join(&rendered.prepare_file_name);
    let params_value: Value = serde_json::from_str(&current.text).unwrap_or(Value::Null);
    json!({
        "id": rendered.config_id,
        "sha256": current.sha256,
        "exceptions": current.params.exceptions.len(),
        "params": params_value,
        "files": {
            "configure": configure_path.display().to_string(),
            "prepare": prepare_path.display().to_string(),
        },
    })
}

/// `GET /api/params`(閲覧系 = 認証なし)。current.json が無ければ 404、
/// ecc_config_dir が無ければ 503(078 §10)。
async fn get_params(State(controller): State<Arc<Controller>>) -> Reply {
    let reply = tokio::task::spawn_blocking(move || {
        let Some(ecc_config_dir) = controller.params.ecc_config_dir.clone() else {
            return err_json(
                StatusCode::SERVICE_UNAVAILABLE,
                ECC_CONFIG_DIR_NOT_CONFIGURED,
            );
        };
        let current = match controller.current_params() {
            Ok(Some(current)) => current,
            Ok(None) => return err_json(StatusCode::NOT_FOUND, "no params configured"),
            Err(e) => return err_json(StatusCode::INTERNAL_SERVER_ERROR, e),
        };
        match controller.render_with_template_dir(&ecc_config_dir, &current.params) {
            // GET は「保存済みの current.json をそのまま映す」だけの閲覧系なので、
            // render に失敗するのは current.json の PUT 後にテンプレートが消えた等の
            // サーバ側の不整合 —— 400(このリクエストの入力の問題)ではなく 500。
            Ok(rendered) => ok_json(params_body(&ecc_config_dir, &current, &rendered)),
            Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, e),
        }
    })
    .await;
    match reply {
        Ok(reply) => reply,
        Err(e) => err_json(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("params task: {e}"),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct ParamsPutRequest {
    token: String,
    params: Value,
}

/// `PUT /api/params { token, params }`(078 §10)。検証順序: authorize(401)→
/// ecc_config_dir の有無(503)→ JSON のスキーマ検証(400)→ 相が Idle か(409)→
/// テンプレート読込 + render(400)→ 書き込み。**同じ JSON → 同じファイル**
/// (既存の configure/prepare/`<id>.json` は上書きしない)。
async fn put_params(
    State(controller): State<Arc<Controller>>,
    Json(request): Json<ParamsPutRequest>,
) -> Reply {
    let operator = match controller.authorize(&request.token) {
        Ok(operator) => operator,
        Err(why) => return unauthorized(&controller, "params/put", json!({}), why),
    };

    let Some(ecc_config_dir) = controller.params.ecc_config_dir.clone() else {
        controller.audit(
            "params/put",
            json!({}),
            &operator,
            false,
            Some(ECC_CONFIG_DIR_NOT_CONFIGURED.to_string()),
        );
        return err_json(
            StatusCode::SERVICE_UNAVAILABLE,
            ECC_CONFIG_DIR_NOT_CONFIGURED,
        );
    };

    // axum の `Json` extractor は本文を `Value` として parse 済みなので、受け取ったバイト列
    // そのものはもう無い。`Value` を再直列化した**この文字列**を正本として扱う
    // (`serde_json::Map` は `preserve_order` 機能を有効にしていないので `BTreeMap` 裏付け =
    // キーは常に辞書順。空白の揺れは吸収され、同じ内容の JSON は常に同じバイト列になる ——
    // これは §3-3「同じ JSON → 同じファイル」を REST 越しでも保つのに必要)。
    let text = request.params.to_string();
    let params = match crate::params::Params::from_json_str(&text) {
        Ok(params) => params,
        Err(e) => {
            let why = e.to_string();
            controller.audit("params/put", json!({}), &operator, false, Some(why.clone()));
            return err_json(StatusCode::BAD_REQUEST, why);
        }
    };

    let reply = tokio::task::spawn_blocking(move || {
        // 113 §2-3: **相の確認と書き込みを同じロック区間に**置く。確認の後でロックを手放すと、
        // その隙に別クライアントの `run/start` が相を Starting にして current.json を読み、
        // 書きかけ / 直後に差し替わる params で run が始まりうる。ロックを握っている間は
        // start が相を取れないので、「PUT が先に済む」か「PUT が 409」のどちらかに決まる。
        // audit はロックを手放してから(logbook のロックと入れ子にしない)。
        let saved = {
            let run_state = lock(&controller.run);
            if run_state.phase != Phase::Idle {
                Err((
                    StatusCode::CONFLICT,
                    format!(
                        "controller is {} — not Idle (次 run から効くという半端は作らない)",
                        run_state.phase
                    ),
                ))
            } else {
                save_params(&controller, &ecc_config_dir, &params, &text)
            }
        };
        let rendered = match saved {
            Ok(rendered) => rendered,
            Err((code, why)) => {
                controller.audit("params/put", json!({}), &operator, false, Some(why.clone()));
                return err_json(code, why);
            }
        };

        let current = CurrentParams {
            sha256: sha256_hex(&text),
            text,
            params,
        };
        let exceptions = current.params.exceptions.len();
        controller.audit(
            "params/put",
            json!({"id": rendered.config_id, "exceptions": exceptions}),
            &operator,
            true,
            None,
        );
        info!(id = rendered.config_id, exceptions, "params saved");
        ok_json(params_body(&ecc_config_dir, &current, &rendered))
    })
    .await;
    match reply {
        Ok(reply) => reply,
        Err(e) => err_json(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("params/put task: {e}"),
        ),
    }
}

/// [`put_params`] の render + 書き込み(呼び手が `controller.run` のロックを握った状態で呼ぶ)。
/// 失敗は (HTTP コード, 理由): テンプレート/render の不備は 400、書けないのは 500。
fn save_params(
    controller: &Controller,
    ecc_config_dir: &Path,
    params: &crate::params::Params,
    text: &str,
) -> Result<crate::params::Rendered, (StatusCode, String)> {
    let rendered = controller
        .render_with_template_dir(ecc_config_dir, params)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let cannot_write = |path: &Path, e: std::io::Error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("cannot write {}: {e}", path.display()),
        )
    };

    // §3-3: 生成物は content-addressed(`<id>` = stem + JSON の sha256 先頭 8 桁)なので、
    // 既存なら書かない(同じ JSON の再 PUT でタイムスタンプが動かない)。
    for file_name in [&rendered.configure_file_name, &rendered.prepare_file_name] {
        let path = ecc_config_dir.join(file_name);
        if !path.exists() {
            std::fs::write(&path, &rendered.xml).map_err(|e| cannot_write(&path, e))?;
        }
    }

    let params_dir = controller.params_dir();
    std::fs::create_dir_all(&params_dir).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("cannot create {}: {e}", params_dir.display()),
        )
    })?;
    // `current.json` は「今の設定」なので常に上書きする(§3-3 の content-addressed 規則は
    // configure/prepare/`<id>.json` だけの話 —— これは固定名で「最新」を指す)。
    // 113 §2-3: 一時ファイルに書いて rename で差し替える(同じファイルシステム内の rename は
    // アトミック)—— 読み手(run/start・GET /api/params・status)が書きかけを見ない。
    let current_path = params_dir.join("current.json");
    let tmp_path = params_dir.join("current.json.tmp");
    std::fs::write(&tmp_path, text).map_err(|e| cannot_write(&tmp_path, e))?;
    std::fs::rename(&tmp_path, &current_path).map_err(|e| cannot_write(&current_path, e))?;

    let id_path = params_dir.join(format!("{}.json", rendered.config_id));
    if !id_path.exists() {
        std::fs::write(&id_path, text).map_err(|e| cannot_write(&id_path, e))?;
    }
    Ok(rendered)
}

#[derive(Debug, Deserialize)]
struct ParamsRenderQuery {
    template: Option<String>,
}

/// `GET /api/params/render?template=<name>`(dry run、閲覧系 = 認証なし)。
/// current.json を(省略時はその `template`、指定時はそのテンプレート名で)render して
/// **書かずに**返す(UI の diff 表示用、078 §10)。
async fn get_params_render(
    State(controller): State<Arc<Controller>>,
    Query(query): Query<ParamsRenderQuery>,
) -> Reply {
    let reply = tokio::task::spawn_blocking(move || {
        let Some(ecc_config_dir) = controller.params.ecc_config_dir.clone() else {
            return err_json(
                StatusCode::SERVICE_UNAVAILABLE,
                ECC_CONFIG_DIR_NOT_CONFIGURED,
            );
        };
        let current = match controller.current_params() {
            Ok(Some(current)) => current,
            Ok(None) => return err_json(StatusCode::NOT_FOUND, "no params configured"),
            Err(e) => return err_json(StatusCode::INTERNAL_SERVER_ERROR, e),
        };
        let mut params = current.params.clone();
        if let Some(template) = &query.template {
            params.template = template.clone();
        }
        match controller.render_with_template_dir(&ecc_config_dir, &params) {
            Ok(rendered) => ok_json(json!({
                "id": rendered.config_id,
                "exceptions": params.exceptions.len(),
                "configure_xcfg": rendered.xml,
            })),
            // dry run の入力(current.json + クエリの template)が原因で失敗しているので 400。
            Err(e) => err_json(StatusCode::BAD_REQUEST, e),
        }
    })
    .await;
    match reply {
        Ok(reply) => reply,
        Err(e) => err_json(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("params/render task: {e}"),
        ),
    }
}

/// 生成物 `<stem>__<hash8>.xcfg`(078 §3-3)か。`__` の後ろが 16 進 8 桁のときだけ。
fn is_generated_xcfg(name: &str) -> bool {
    let stem = name.strip_suffix(".xcfg").unwrap_or(name);
    match stem.rsplit_once("__") {
        Some((_, hash)) => hash.len() == 8 && hash.chars().all(|c| c.is_ascii_hexdigit()),
        None => false,
    }
}

/// `GET /api/params/templates`(閲覧系 = 認証なし、TODO/111 §5.0)。
/// `ecc_config_dir` 直下の `configure-*.xcfg`(生成物を除く)のファイル名を辞書順。
async fn get_params_templates(State(controller): State<Arc<Controller>>) -> Reply {
    let reply = tokio::task::spawn_blocking(move || {
        let Some(ecc_config_dir) = controller.params.ecc_config_dir.clone() else {
            return err_json(
                StatusCode::SERVICE_UNAVAILABLE,
                ECC_CONFIG_DIR_NOT_CONFIGURED,
            );
        };
        let entries = match std::fs::read_dir(&ecc_config_dir) {
            Ok(entries) => entries,
            Err(e) => {
                return err_json(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("cannot read {}: {e}", ecc_config_dir.display()),
                )
            }
        };
        let mut templates: Vec<String> = Vec::new();
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    return err_json(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("cannot read {}: {e}", ecc_config_dir.display()),
                    )
                }
            };
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("configure-")
                && name.ends_with(".xcfg")
                && !is_generated_xcfg(&name)
                && entry.path().is_file()
            {
                templates.push(name);
            }
        }
        templates.sort();
        ok_json(json!({"templates": templates}))
    })
    .await;
    match reply {
        Ok(reply) => reply,
        Err(e) => err_json(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("params/templates task: {e}"),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct ParamsDefaultsQuery {
    template: Option<String>,
}

/// `GET /api/params/defaults?template=<name>`(閲覧系 = 認証なし、TODO/111 §5.0)。
/// テンプレート xcfg からノブの値を抽出して返す(UI の「Load defaults from template」)。
/// 400 = 名前が不正 / `ecc_config_dir` の外 / xcfg として読めない、404 = 無い、503 = 未設定。
async fn get_params_defaults(
    State(controller): State<Arc<Controller>>,
    Query(query): Query<ParamsDefaultsQuery>,
) -> Reply {
    let reply = tokio::task::spawn_blocking(move || {
        let Some(ecc_config_dir) = controller.params.ecc_config_dir.clone() else {
            return err_json(
                StatusCode::SERVICE_UNAVAILABLE,
                ECC_CONFIG_DIR_NOT_CONFIGURED,
            );
        };
        let name = query.template.unwrap_or_default();
        if name.is_empty() {
            return err_json(StatusCode::BAD_REQUEST, "template is required");
        }
        if name.contains('/') || name.contains('\\') || name.contains("..") {
            return err_json(
                StatusCode::BAD_REQUEST,
                format!("template {name:?} must be a file name in ecc_config_dir"),
            );
        }
        if !name.ends_with(".xcfg") {
            return err_json(
                StatusCode::BAD_REQUEST,
                format!("template {name:?} is not an .xcfg file"),
            );
        }
        let path = ecc_config_dir.join(&name);
        if !path.exists() {
            return err_json(
                StatusCode::NOT_FOUND,
                format!("template {name} not found in {}", ecc_config_dir.display()),
            );
        }
        // シンボリックリンク等でディレクトリの外へ出るものは拒否する(canonicalize してから比べる)。
        let inside = match (path.canonicalize(), ecc_config_dir.canonicalize()) {
            (Ok(file), Ok(dir)) => file.starts_with(dir),
            (Err(e), _) | (_, Err(e)) => {
                return err_json(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("cannot resolve {}: {e}", path.display()),
                )
            }
        };
        if !inside {
            return err_json(
                StatusCode::BAD_REQUEST,
                format!("template {name} resolves outside ecc_config_dir"),
            );
        }
        let xml = match std::fs::read_to_string(&path) {
            Ok(xml) => xml,
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                return err_json(
                    StatusCode::BAD_REQUEST,
                    format!("template {name} is not UTF-8 text: {e}"),
                )
            }
            Err(e) => {
                return err_json(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("cannot read {}: {e}", path.display()),
                )
            }
        };
        match crate::params::extract(&name, &xml) {
            Ok(extracted) => ok_json(extracted.to_json()),
            Err(e) => err_json(StatusCode::BAD_REQUEST, e.to_string()),
        }
    })
    .await;
    match reply {
        Ok(reply) => reply,
        Err(e) => err_json(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("params/defaults task: {e}"),
        ),
    }
}

// ---------------------------------------------------------------------
// HV / LV の ch ごとの ON/OFF(TODO/124 レーン B、SPEC §8.1 / §9.2 / §11 v1.42)
// ---------------------------------------------------------------------

/// slowctl の power REP への REQ の待ち時間(124 §2)。slowctl 側の上限(機器での実行 5 s)
/// より長く取り、slowctl が自分の `"timeout"` を返す余地を残す。
const SLOW_CMD_TIMEOUT: Duration = Duration::from_secs(10);

/// audit の `action`(SPEC §9.2 v1.42)。
const SLOW_POWER_ACTION: &str = "slow/power";
const SLOW_CMD_NOT_CONFIGURED: &str = "slow_cmd is not configured";
const HV_ON_REFUSED: &str = "HV ON is allowed only while no run is active";
const LV_OFF_REFUSED: &str = "LV OFF is refused while a run is active — stop the run first";
const POWER_IN_FLIGHT: &str = "another power command is in flight";
const RUN_START_POWER_IN_FLIGHT: &str =
    "slow-control power command in flight — retry in a few seconds";
/// 504 の文言に必ず入れる(Fable B-3)。送った後に返事が無い = 機器が動いたかは分からない。
const STATE_UNKNOWN: &str = "state unknown — check the table";

/// 規則(SPEC §11 v1.42、ユーザー裁定 2026-10-12): HV ON(ランプアップ中の偽事象)と
/// LV OFF(AsAd の電源を落とす)は相 `Idle` のときだけ。HV OFF と LV ON はいつでも可。
/// Starting / Stopping も Running と同じく「run が動いている」側に数える(Fable B-2)。
fn power_rule(device_type: &str, on: bool, phase: Phase) -> Result<(), &'static str> {
    if phase == Phase::Idle {
        return Ok(());
    }
    match (device_type, on) {
        ("hivolta", true) => Err(HV_ON_REFUSED),
        ("hmp2020", false) => Err(LV_OFF_REFUSED),
        _ => Ok(()),
    }
}

fn on_off(on: bool) -> &'static str {
    if on {
        "ON"
    } else {
        "OFF"
    }
}

/// `POST /api/slow/power {token, device, ch, on}`(SPEC §8.1 v1.42)。
///
/// device / ch / on は `Value` のまま受けて手で検証する(`RunNextRequest` と同じ理由 ——
/// 型違いを `Json` extractor が先に弾くと、400 と audit の 1 行(Fable B-4)を出せない)。
/// token の省略も同じく 401 + audit にする。
#[derive(Debug, Deserialize)]
struct SlowPowerRequest {
    #[serde(default)]
    token: String,
    #[serde(default)]
    device: Value,
    #[serde(default)]
    ch: Value,
    #[serde(default)]
    on: Value,
}

/// 検証済みの power コマンド。
#[derive(Debug)]
struct PowerCommand {
    device: String,
    device_type: String,
    ch: u8,
    on: bool,
}

/// 400 の判定(124 §2): device が `[[slowctl.device]]` に在り、type が hivolta / hmp2020、
/// ch がその型の範囲(hivolta 0–7、hmp2020 1–2)、on が真偽値。
fn validate_power_request(
    devices: &[(String, String)],
    request: &SlowPowerRequest,
) -> Result<PowerCommand, String> {
    let Some(device) = request.device.as_str() else {
        return Err(format!(
            "device must be a slow-control device name, got {}",
            request.device
        ));
    };
    let Some((_, device_type)) = devices.iter().find(|(name, _)| name == device) else {
        return Err(format!(
            "unknown slow-control device {device:?} (not in [[slowctl.device]])"
        ));
    };
    let channels = match device_type.as_str() {
        "hivolta" => 0..=7u8,
        "hmp2020" => 1..=2u8,
        other => {
            return Err(format!(
                "device {device:?} is a {other} — only hivolta / hmp2020 have a power switch"
            ))
        }
    };
    let Some(ch) = request
        .ch
        .as_u64()
        .and_then(|ch| u8::try_from(ch).ok())
        .filter(|ch| channels.contains(ch))
    else {
        return Err(format!(
            "ch must be an integer in {}–{} for {device_type}, got {}",
            channels.start(),
            channels.end(),
            request.ch
        ));
    };
    let Some(on) = request.on.as_bool() else {
        return Err(format!("on must be true or false, got {}", request.on));
    };
    Ok(PowerCommand {
        device: device.to_string(),
        device_type: device_type.clone(),
        ch,
        on,
    })
}

/// audit `slow/power` の `params`(SPEC §9.2 v1.42: `{device, ch, on, phase, before, after}`)。
/// device / ch / on は**受け取ったまま**(400 の行にも何が来たかを残す)。
fn power_audit_params(
    request: &SlowPowerRequest,
    phase: Phase,
    before: &Value,
    after: &Value,
) -> Value {
    json!({
        "device": request.device,
        "ch": request.ch,
        "on": request.on,
        "phase": phase.to_string(),
        "before": before,
        "after": after,
    })
}

/// slowctl に届かなかった / 返事が読めなかった理由。
#[derive(Debug)]
enum SlowCmdFailure {
    /// 送ったが期限内に返事が無い(audit の error は `"timeout"` —— Fable B-3)。
    Timeout,
    /// ソケット・接続・送信の失敗。
    Unreachable(String),
    /// 返事が JSON でない。
    BadReply(String),
}

/// slowctl へ power の REQ を 1 往復(124 §1 の要求形)。ソケットは呼び出し毎に作って捨てる
/// (`ZmqTransport::ecc` と同じ —— REQ の状態機械を持ち越さない)。linger 0。
fn slow_power_request(
    context: &zmq::Context,
    endpoint: &str,
    command: &PowerCommand,
    timeout: Duration,
) -> Result<Value, SlowCmdFailure> {
    let timeout_ms = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
    let fail = |what: &str, e: zmq::Error| {
        SlowCmdFailure::Unreachable(format!("slowctl {endpoint}: {what}: {e}"))
    };
    let socket = context
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
        .connect(endpoint)
        .map_err(|e| fail("cannot connect", e))?;

    let payload = json!({
        "op": "power",
        "device": command.device,
        "ch": command.ch,
        "on": command.on,
    });
    socket
        .send(payload.to_string().as_bytes(), 0)
        .map_err(|e| fail("send failed", e))?;
    let reply = match socket.recv_bytes(0) {
        Ok(reply) => reply,
        Err(zmq::Error::EAGAIN) => return Err(SlowCmdFailure::Timeout),
        Err(e) => return Err(fail("recv failed", e)),
    };
    serde_json::from_slice(&reply).map_err(|e| {
        SlowCmdFailure::BadReply(format!(
            "slowctl reply is not JSON: {e} ({})",
            String::from_utf8_lossy(&reply)
        ))
    })
}

/// HV / LV の ch ごとの ON/OFF(SPEC §8.1 / §11 v1.42、TODO/124 + Fable B 1〜4)。
///
/// 判定の順は `put_params` と揃えた: authorize(401)→ `slow_cmd` の有無(503)→ 入力(400)→
/// 短いロックの中で「印 → 規則」(409)→ 印を立ててロックを放し、REQ はブロッキングタスクで
/// (200 / 502 / 504)。**どの経路でも audit `slow/power` を 1 行**書く(Fable B-4)。
async fn post_slow_power(
    State(controller): State<Arc<Controller>>,
    Json(request): Json<SlowPowerRequest>,
) -> Reply {
    let phase_now = lock(&controller.run).phase;
    let operator = match controller.authorize(&request.token) {
        Ok(operator) => operator,
        Err(why) => {
            let params = power_audit_params(&request, phase_now, &Value::Null, &Value::Null);
            return unauthorized(&controller, SLOW_POWER_ACTION, params, why);
        }
    };
    let refuse = |code: StatusCode, phase: Phase, why: String| {
        let params = power_audit_params(&request, phase, &Value::Null, &Value::Null);
        controller.audit(
            SLOW_POWER_ACTION,
            params,
            &operator,
            false,
            Some(why.clone()),
        );
        warn!(%code, device = %request.device, ch = %request.ch, on = %request.on, %phase, why,
              "slow/power refused");
        err_json(code, why)
    };

    let Some(endpoint) = controller.params.slow_cmd.clone() else {
        return refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            phase_now,
            SLOW_CMD_NOT_CONFIGURED.to_string(),
        );
    };
    let command = match validate_power_request(&controller.params.slow_devices, &request) {
        Ok(command) => command,
        Err(why) => return refuse(StatusCode::BAD_REQUEST, phase_now, why),
    };

    // Fable B-2: 「同じロック区間で検査と送信」は取らない(std Mutex を REQ の 10 s のあいだ
    // 握れない)。run/start の `Starting` と同じく、短いロックで判定して印を立てる。
    // audit はロックを手放してから(logbook のロックと入れ子にしない)。
    let (phase, refused) = {
        let mut run_state = lock(&controller.run);
        let phase = run_state.phase;
        let refused = match &run_state.power_inflight {
            Some(inflight) => Some(format!(
                "{POWER_IN_FLIGHT} ({} ch {} {} by {})",
                inflight.device,
                inflight.ch,
                on_off(inflight.on),
                inflight.operator
            )),
            None => power_rule(&command.device_type, command.on, phase)
                .err()
                .map(str::to_string),
        };
        if refused.is_none() {
            run_state.power_inflight = Some(PowerInflight {
                device: command.device.clone(),
                ch: command.ch,
                on: command.on,
                operator: operator.clone(),
            });
        }
        (phase, refused)
    };
    if let Some(why) = refused {
        return refuse(StatusCode::CONFLICT, phase, why);
    }
    info!(
        device = command.device,
        ch = command.ch,
        on = command.on,
        %phase,
        operator,
        "slow/power: sending to slowctl"
    );

    // ブロッキングタスクが panic しても印を立てたままにしない(run/start の復旧と同じ形)。
    let recovery = Arc::clone(&controller);
    let recovery_params = power_audit_params(&request, phase, &Value::Null, &Value::Null);
    let recovery_operator = operator.clone();
    let reply = tokio::task::spawn_blocking(move || {
        let outcome = slow_power_request(
            &controller.transport.context,
            &endpoint,
            &command,
            SLOW_CMD_TIMEOUT,
        );
        lock(&controller.run).power_inflight = None;
        finish_slow_power(&controller, &request, phase, &operator, &command, outcome)
    })
    .await;
    match reply {
        Ok(reply) => reply,
        Err(e) => {
            lock(&recovery.run).power_inflight = None;
            let why = format!("slow/power task: {e}");
            recovery.audit(
                SLOW_POWER_ACTION,
                recovery_params,
                &recovery_operator,
                false,
                Some(why.clone()),
            );
            error!(error = %e, "slow/power task died — power_inflight cleared");
            err_json(StatusCode::INTERNAL_SERVER_ERROR, why)
        }
    }
}

/// slowctl の返事(または不達)を REST の応答 + audit 1 行にする(124 §2、Fable B-3)。
///
/// * `ok:true` → 200、slowctl の応答**そのまま**。
/// * `ok:false` + `error` → 502、slowctl の応答そのまま(error 文言・before を含む)。
/// * 形の崩れた返事 → 502(何が来たかを文言に入れる)。
/// * 返事が無い / 届かない → 504、文言に「state unknown — check the table」。
fn finish_slow_power(
    controller: &Controller,
    request: &SlowPowerRequest,
    phase: Phase,
    operator: &str,
    command: &PowerCommand,
    outcome: Result<Value, SlowCmdFailure>,
) -> Reply {
    let (code, body, before, after, error) = match outcome {
        Ok(reply) => {
            let before = reply.get("before").cloned().unwrap_or(Value::Null);
            let after = reply.get("after").cloned().unwrap_or(Value::Null);
            let ok = reply.get("ok").and_then(Value::as_bool);
            let error = reply
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_string);
            match (ok, error) {
                (Some(true), _) => (StatusCode::OK, reply, before, after, None),
                (Some(false), Some(error)) => {
                    (StatusCode::BAD_GATEWAY, reply, before, after, Some(error))
                }
                _ => {
                    let why = format!("slowctl reply is not {{ok, error, ...}}: {reply}");
                    let body = json!({ "error": why });
                    (StatusCode::BAD_GATEWAY, body, before, after, Some(why))
                }
            }
        }
        Err(SlowCmdFailure::BadReply(why)) => (
            StatusCode::BAD_GATEWAY,
            json!({ "error": why }),
            Value::Null,
            Value::Null,
            Some(why),
        ),
        Err(SlowCmdFailure::Timeout) => (
            StatusCode::GATEWAY_TIMEOUT,
            json!({ "error": format!(
                "slowctl did not reply within {} s — {STATE_UNKNOWN}",
                SLOW_CMD_TIMEOUT.as_secs()
            ) }),
            Value::Null,
            Value::Null,
            Some("timeout".to_string()),
        ),
        Err(SlowCmdFailure::Unreachable(why)) => (
            StatusCode::GATEWAY_TIMEOUT,
            json!({ "error": format!("{why} — {STATE_UNKNOWN}") }),
            Value::Null,
            Value::Null,
            Some(why),
        ),
    };

    let ok = code == StatusCode::OK;
    controller.audit(
        SLOW_POWER_ACTION,
        power_audit_params(request, phase, &before, &after),
        operator,
        ok,
        error.clone(),
    );
    if ok {
        info!(device = command.device, ch = command.ch, on = command.on,
              %before, %after, "slow/power done");
    } else {
        warn!(%code, device = command.device, ch = command.ch, on = command.on,
              %before, %after, error = error.as_deref().unwrap_or(""), "slow/power failed");
    }
    (code, Json(body))
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
    slow_latest: Arc<Mutex<SlowLatest>>,
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
                        // SPEC §9.2 v1.31: slowctl の VALUE/ALARM/RECOVER は run_start /
                        // run_stop に載せる直近値でもある。ログブックへ書く前に拾う
                        // (書き込みが失敗しても「今の圧力」は失わない)。
                        if matches!(record, LogbookRecord::Slow { .. }) {
                            let ts = crate::logbook::now_rfc3339_millis();
                            apply_slow_record(&mut lock(&slow_latest), &record, &ts);
                        }
                        // TODO/121 §B: 定期値 VALUE は直近値の更新だけ(時系列は別の保存先)。
                        // IDENT / ALARM / RECOVER / ERROR はまれで安全上の意味があるので書く。
                        if matches!(&record, LogbookRecord::Slow { event, .. } if event == "VALUE")
                        {
                            counters
                                .slow_values_not_logged
                                .fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
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
        /// この ECC が抱えている error フラグ(043)。**state とは別の軸**。
        /// clear 規則も実機どおり: 遷移が渡ったときだけ `NO_ERR` に戻り、遷移が無い
        /// (`Ignored`)ときは触られない(`tools/ecc_bridge/ecc_core.hpp` の規則ブロック、
        /// 出典 `GetBench/src/get/rc/BackEnd.cpp:249-290` / `:1146-1149`)。
        ecc_error: String,
        /// 「受けたのに無音で何もしない」action(036)。実 ECC の未定義遷移は例外も出さず
        /// `Ignored`(`StateMachine/src/dhsm/Engine.cpp:334-352`)なので、歩き戻しが
        /// 空振りする経路を**わざと**作れるようにしておく。
        ecc_ignores: Vec<String>,
        /// EOS 完了を「N 回目の GetStatus 以降」にする(時間経過の模擬)。
        /// **数えるのは decoder / graw-writer への GetStatus だけ**(receiver への
        /// 静止観測は EOS の進み方とは無関係なので数に入れない — 033-E)。
        eos_after_polls: Option<usize>,
        eos_polls: usize,
        /// receiver 名(静止観測の相手を kind で判別するため)。
        receivers: BTreeSet<String>,
        /// receiver 名 → 受信バイト数の進み方(033-E)。GetStatus 1 回につきこの分だけ増える。
        /// 未登録 = `0` = 静止(実機の stop 後の姿)。
        bytes_steps: BTreeMap<String, u64>,
        /// receiver 名 → GetStatus 回数(`bytes` の値を作るのに使う)。
        receiver_polls: BTreeMap<String, u64>,
        /// この名前のコンポーネントは**不達**(REQ タイムアウト相当の `Err`)。
        unreachable: Vec<String>,
        /// コンポーネント名 → 今の状態(113)。未登録 = `Idle`。断られなかった状態遷移
        /// コマンドはその行き先へ動かす(合法性は見ない — それは統合テストの本物の状態機械の役)。
        states: BTreeMap<String, ComponentState>,
        /// コンポーネント名 → 応答に載せる `run_number`(115)。未登録 = `null`
        /// (実コンポーネントは run を持っていないとき鍵を null にする)。
        run_numbers: BTreeMap<String, u32>,
        slept: Duration,
    }

    impl MockTransport {
        fn new(params: &ControllerParams) -> Self {
            let mut names = BTreeMap::new();
            let mut metrics = BTreeMap::new();
            let mut receivers = BTreeSet::new();
            for (i, component) in params.components.iter().enumerate() {
                names.insert(component.endpoint.clone(), component.name.clone());
                let value = match component.kind {
                    // 非対称な値(取り違え検出用)。
                    ComponentKind::Receiver { cobo_id } => {
                        receivers.insert(component.name.clone());
                        json!({
                            "router_port": 46100 + cobo_id,
                            "bind_address": format!("0.0.0.0:{}", 46100 + cobo_id),
                            "frames": 3000 + i,
                            "overflow_frames": 0,
                            // 033-E: 実 receiver の metrics と同じ形(receiver.rs `json()`)。
                            "bytes": 0,
                        })
                    }
                    // 033-C: `eos_out` も **decoder の実 metrics と同じ形**で持つ
                    // (最後のホップの観測点。無ければ controller は判定を保留する)。
                    ComponentKind::Decoder => json!({"eos_in": 0, "eos_out": 0, "malformed": 7}),
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
                // 実 ECC の初期値も NO_ERR(`BackEnd::BackEnd()`、BackEnd.cpp:132)。
                ecc_error: "NO_ERR".to_string(),
                ecc_ignores: Vec::new(),
                eos_after_polls: None,
                eos_polls: 0,
                receivers,
                bytes_steps: BTreeMap::new(),
                receiver_polls: BTreeMap::new(),
                unreachable: Vec::new(),
                states: BTreeMap::new(),
                run_numbers: BTreeMap::new(),
                slept: Duration::ZERO,
            }
        }

        /// 113: このコンポーネントが `state` に居るところから始める
        /// (例: 前の run の停止で Reset が届かず `Configured` のまま残った)。
        fn component_in(mut self, component: &str, state: ComponentState) -> Self {
            self.states.insert(component.to_string(), state);
            self
        }

        /// 115: このコンポーネントの応答に `run_number = run` を載せる(前の run の番号の出典)。
        fn reports_run(mut self, component: &str, run: u32) -> Self {
            self.run_numbers.insert(component.to_string(), run);
            self
        }

        /// EOS は最初から流れ切っている(正常停止で強制 EOS に落ちない)。
        fn with_eos_closed(mut self) -> Self {
            self.eos_after_polls = Some(0);
            self
        }

        /// 033-E: 全 receiver がまだ受信し続けている(GetStatus 1 回につき `step` バイト増える)。
        /// 静止しないので第一段はハード上限 `eos_timeout` まで待つ。
        fn receivers_still_receiving(mut self, step: u64) -> Self {
            for name in self.receivers.iter() {
                self.bytes_steps.insert(name.clone(), step);
            }
            self
        }

        /// 033-E: **この receiver だけ**受信を続けている(他は静止)。
        fn receiver_still_receiving(mut self, component: &str, step: u64) -> Self {
            self.bytes_steps.insert(component.to_string(), step);
            self
        }

        /// 033-E: この receiver の metrics から `bytes` を落とす(静止を判定する材料が無い形)。
        fn without_receiver_bytes(mut self, component: &str) -> Self {
            if let Some(value) = self.metrics.get_mut(component) {
                if let Some(map) = value.as_object_mut() {
                    map.remove("bytes");
                }
            }
            self
        }

        /// このコンポーネントは**不達**(接続拒否も REQ タイムアウトも controller から見れば
        /// 同じ `Err` —— 033 論点 3)。
        fn unreachable(mut self, component: &str) -> Self {
            self.unreachable.push(component.to_string());
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

        /// 043: ECC が既に error フラグを抱えているところから始める
        /// (例 041 D-1 の「describe に失敗した直後」= `Idle` + `WHEN_DESCRIBE`)。
        fn ecc_error_flag(mut self, flag: &str) -> Self {
            self.ecc_error = flag.to_string();
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
            if self.unreachable.contains(&name) {
                return Err(format!("mock: {name} is unreachable"));
            }

            let mut metrics = self.metrics.get(&name).cloned().unwrap_or(Value::Null);
            if matches!(command, Command::GetStatus) && self.receivers.contains(&name) {
                // 033-E: 受信バイト数の進み方(step == 0 = 静止)。
                // **EOS の進み方(`eos_polls`)には数えない** —— 静止観測は別軸。
                let polls = self.receiver_polls.entry(name.clone()).or_insert(0);
                *polls += 1;
                let polls = *polls;
                let step = self.bytes_steps.get(&name).copied().unwrap_or(0);
                if let Some(map) = metrics.as_object_mut() {
                    if map.contains_key("bytes") {
                        map.insert("bytes".to_string(), json!(step * polls));
                    }
                }
            } else if matches!(command, Command::GetStatus) {
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
                        if map.contains_key("eos_out") {
                            map.insert("eos_out".to_string(), json!(1));
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
            if !refused {
                if let Some(target) = command.target_state() {
                    self.states.insert(name.clone(), target);
                }
            }
            let state = self
                .states
                .get(&name)
                .copied()
                .unwrap_or(ComponentState::Idle);
            let mut response = if refused {
                CommandResponse::error(state, format!("mock refuses {action}"))
            } else {
                CommandResponse::success(state, format!("{action} ok"))
            };
            if let Some(&run) = self.run_numbers.get(&name) {
                response = response.with_run_number(run);
            }
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
                // 断られた = 遷移が渡っていない → error フラグには**触らない**(043)。
                return Ok(EccReply {
                    ok: false,
                    state: self.ecc_state.clone(),
                    error: format!("mock refuses {action}"),
                    ecc_error: self.ecc_error.clone(),
                });
            }
            if !self.ecc_ignores.contains(&action) {
                if let Some(next) = ecc_transition(&self.ecc_state, &action) {
                    self.ecc_state = next.to_string();
                    // 043: `resetErrorFlag` は**全遷移の 1 番目のアクション**
                    // (BackEnd.cpp:249-290 / :1146-1149)。遷移が渡ったときだけ消える ——
                    // 遷移が無い(`Ignored`)ときはアクションが 1 つも走らないので残る。
                    self.ecc_error = "NO_ERR".to_string();
                }
            }
            // 実 ECC は未定義遷移でもエラーを返さない(無音)ので **ok は常に true**。
            Ok(EccReply {
                ok: true,
                state: self.ecc_state.clone(),
                error: String::new(),
                ecc_error: self.ecc_error.clone(),
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
            ecc_config_dir: None,
            params_dir: None,
            eos_timeout: Duration::from_secs(5),
            eos_quiesce: Duration::from_millis(500),
            eos_poll: Duration::from_millis(200),
            command_timeout: Duration::from_secs(1),
            ecc_timeout: Duration::from_secs(1),
            status_timeout: Duration::from_secs(1),
            slow_cmd: None,
            slow_devices: Vec::new(),
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

    // -----------------------------------------------------------------
    // ECC の error フラグ(043、SPEC §8.2 v1.14)
    // -----------------------------------------------------------------

    /// ecc-bridge の応答は `ecc_error` を運ぶ。**古いブリッジ(このキーを出さない)相手でも
    /// parse を落とさない**(`#[serde(default)]`)—— 応答が読めなくなると状態そのものが
    /// 見えなくなり、フラグを可視化するはずの 043 が逆に運用を止めてしまう。
    #[test]
    fn an_ecc_reply_carries_the_error_flag_and_tolerates_an_older_bridge() {
        // 041 D-1 の実測そのもの: state は正常(Idle)なのにフラグだけ立っている。
        let reply: EccReply = serde_json::from_str(
            r#"{"ok":true,"state":"Idle","error":"","ecc_error":"WHEN_DESCRIBE"}"#,
        )
        .unwrap();
        assert!(reply.ok);
        assert_eq!(reply.state, "Idle");
        assert_eq!(reply.error, "");
        assert_eq!(reply.ecc_error, "WHEN_DESCRIBE");

        // 輸送層の `error` と GET の error フラグは**別の軸**(片方が他方を上書きしない)。
        let both: EccReply = serde_json::from_str(
            r#"{"ok":false,"state":"Ready","error":"Could not establish data link. refused",
                "ecc_error":"WHEN_START"}"#,
        )
        .unwrap();
        assert_eq!(both.error, "Could not establish data link. refused");
        assert_eq!(both.ecc_error, "WHEN_START");

        // `ecc_error` を知らない ecc-bridge = 空文字(「載っていなかった」)。
        let old: EccReply =
            serde_json::from_str(r#"{"ok":true,"state":"Idle","error":""}"#).unwrap();
        assert_eq!(old.ecc_error, "");
    }

    /// **フラグの clear 規則**(043)を [`MockTransport`] が実機どおりに持つこと。
    ///
    /// 出典 `GetBench/src/get/rc/BackEnd.cpp:249-290`(`resetErrorFlag` が全遷移の
    /// **1 番目のアクション**)/ `:1146-1149`。遷移が**無い**状態で叩くと dhsm は
    /// `Ignored` を返してアクションを 1 つも実行しない(`dhsm/Engine.cpp:338-353`)ので
    /// **フラグは残る**。ダブルをここで甘くする(status や無音 action で消す)と、
    /// 041 D-1 の「見えないまま残るフラグ」を再現できなくなる。
    #[test]
    fn the_ecc_error_flag_survives_a_silent_action_and_clears_only_on_a_transition() {
        let params = params_with(&[0]);
        // 前の describe が失敗した直後の ECC(state は Idle のまま、フラグだけ立っている)。
        let mut mock = MockTransport::new(&params)
            .ecc_in("Idle")
            .ecc_error_flag("WHEN_DESCRIBE");

        // status は観測 —— 何度読んでも消えない。
        for _ in 0..2 {
            let reply = mock.ecc(&json!({"action": "status"})).unwrap();
            assert_eq!(reply.state, "Idle");
            assert_eq!(reply.ecc_error, "WHEN_DESCRIBE");
        }

        // `Idle` からの reset は EV_UNDO が無い = 無音(`Ignored`)。状態もフラグも動かない。
        let reply = mock.ecc(&json!({"action": "reset"})).unwrap();
        assert!(reply.ok, "実 ECC は無音なので ok は true のまま");
        assert_eq!(reply.state, "Idle");
        assert_eq!(reply.ecc_error, "WHEN_DESCRIBE", "遷移が無ければ消えない");

        // describe は `Idle` から遷移が**ある** → 渡り始めた時点で `NO_ERR` に消える。
        let reply = mock.ecc(&json!({"action": "describe"})).unwrap();
        assert_eq!(reply.state, "Described");
        assert_eq!(reply.ecc_error, "NO_ERR");
    }

    // -----------------------------------------------------------------
    // 共有 golden 遷移表とのパリティ(049)
    // -----------------------------------------------------------------

    /// ECC の状態機械は **意図的に 2 実装**ある(036 の事故に由来する負荷分散された防御。
    /// 044 で「統一しない」と裁定済み)。残るリスクは**将来の片側だけの修正によるドリフト**
    /// なので、`tests/fixtures/ecc_transitions.txt` の golden 表を **C++ 側
    /// (`tools/ecc_bridge/test_ecc_bridge.cpp` の同名テスト)と同じ 1 ファイルから**読み、
    /// 全 64 行(8 state × 8 action)を照合する。
    ///
    /// 表の各列の意味と、この Rust 実装への対応はファイル冒頭のヘッダに書いてある。
    /// **表と実装が食い違ったら、表を直す前に一次資料**(`reference/20190315_patched/
    /// GetBench/src/get/rc/BackEnd.cpp`)**を見ること** —— 意味論の正本は
    /// `tools/ecc_bridge/ecc_core.hpp` の 2 つのコメントブロック(048-C)。
    #[test]
    fn the_ecc_state_machine_matches_the_shared_golden_table() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/ecc_transitions.txt");
        let text = std::fs::read_to_string(&path).unwrap();
        let params = params_with(&[0]);
        // 「フラグに触らない」を検出するための番兵。`NO_ERR` と区別でき、かつ**実在の値**
        // (041 D-1 で実測した「describe に失敗した直後」の姿)を使う。
        const SENTINEL: &str = "WHEN_DESCRIBE";

        let mut rows = 0;
        for (index, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let cols: Vec<&str> = line.split_whitespace().collect();
            let at = format!("{}:{}", path.display(), index + 1);
            assert_eq!(cols.len(), 4, "{at}: 4 列でない: {raw}");
            let (state, action, outcome, flag) = (cols[0], cols[1], cols[2], cols[3]);
            let at = format!("{at} [{state} {action}]");

            // outcome → 「遷移が渡ったか」と「渡った先」。`Ignored` / `Denied` / `Observed`
            // は Rust 側ではすべて「遷移が無い」= `None` に畳まれる(表のヘッダ参照)。
            let applied = outcome.strip_prefix("Applied:");
            assert!(
                applied.is_some() || matches!(outcome, "Ignored" | "Denied" | "Observed"),
                "{at}: 未知の outcome: {outcome}"
            );

            // (1) 遷移表そのもの。
            assert_eq!(ecc_transition(state, action), applied, "{at}: 遷移表");

            // (2) 状態と error フラグの**実際の見え方**(`MockTransport::ecc` の応答)。
            let mut mock = MockTransport::new(&params)
                .ecc_in(state)
                .ecc_error_flag(SENTINEL);
            let reply = mock.ecc(&json!({"action": action})).unwrap();
            assert_eq!(reply.state, applied.unwrap_or(state), "{at}: 次状態");
            let want_flag = match flag {
                "clear" => "NO_ERR",
                "keep" => SENTINEL,
                other => panic!("{at}: 未知の error_flag: {other}"),
            };
            assert_eq!(reply.ecc_error, want_flag, "{at}: error フラグ");
            rows += 1;
        }
        // 表が空 / 半分しか読めていないのに green になるのを防ぐ(8 state × 8 action)。
        assert_eq!(rows, 64, "golden 表の行数");
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

    /// SPEC §9.2 v1.31: `slow` の VALUE/ALARM/RECOVER だけが直近値になる。
    /// キーは `"<device>/<node>/<param>"`、node 無しは `"-"`。
    #[test]
    fn only_measured_slow_events_become_the_run_record_snapshot() {
        let mut latest = SlowLatest::new();
        let ts = "2026-09-22T10:00:00.000+03:00";

        let value = LogbookRecord::Slow {
            actor: "slowctl".to_string(),
            device: "gas".to_string(),
            node: Some(4),
            event: "VALUE".to_string(),
            values: json!({"fmeasure": {"value": 130.5, "unit": "mbar"}}),
        };
        assert!(apply_slow_record(&mut latest, &value, ts));
        assert_eq!(latest["gas/4/fmeasure"].value, 130.5);
        assert_eq!(latest["gas/4/fmeasure"].unit, "mbar");
        assert_eq!(latest["gas/4/fmeasure"].ts, ts);

        // node 無しの機器(Arduino 等)は `-`。
        let nodeless = LogbookRecord::Slow {
            actor: "slowctl".to_string(),
            device: "env".to_string(),
            node: None,
            event: "ALARM".to_string(),
            values: json!({"t_c": {"value": 31.0, "unit": "degC"}}),
        };
        assert!(apply_slow_record(&mut latest, &nodeless, ts));
        assert_eq!(latest["env/-/t_c"].value, 31.0);

        // IDENT(文字列)と ERROR(カウンタ)は直近値ではない —— 取り込まない。
        let ident = LogbookRecord::Slow {
            actor: "slowctl".to_string(),
            device: "gas".to_string(),
            node: Some(4),
            event: "IDENT".to_string(),
            values: json!({"model": "F-201CV"}),
        };
        assert!(!apply_slow_record(&mut latest, &ident, ts));
        let error = LogbookRecord::Slow {
            actor: "slowctl".to_string(),
            device: "gas".to_string(),
            node: None,
            event: "ERROR".to_string(),
            values: json!({"poll_err": 3}),
        };
        assert!(!apply_slow_record(&mut latest, &error, ts));

        // slow 以外のレコードも無視(誤って run 記録に混ぜない)。
        assert!(!apply_slow_record(&mut latest, &sample_comment(), ts));
        assert_eq!(latest.len(), 2);

        // 新しい VALUE は上書きする(直近値であって履歴ではない)。
        let newer = LogbookRecord::Slow {
            actor: "slowctl".to_string(),
            device: "gas".to_string(),
            node: Some(4),
            event: "VALUE".to_string(),
            values: json!({"fmeasure": {"value": 131.0, "unit": "mbar"}}),
        };
        assert!(apply_slow_record(&mut latest, &newer, "later"));
        assert_eq!(latest["gas/4/fmeasure"].value, 131.0);
        assert_eq!(latest["gas/4/fmeasure"].ts, "later");
    }

    fn sample_comment() -> LogbookRecord {
        LogbookRecord::Comment {
            actor: "ui".to_string(),
            author: "aogaki".to_string(),
            text: "beam tuned".to_string(),
        }
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

    // -----------------------------------------------------------------
    // 113 — コンポーネントの歩き戻し(停止時の Reset 不達からの回復)
    // -----------------------------------------------------------------

    /// **113 の本体**: 前の run の停止で Reset が届かず `Configured` のまま残った receiver0 を、
    /// ECC の歩き戻しの後・Configure の前に `Reset` で `Idle` へ戻す。Idle のものには触らない。
    /// 歩いた跡は `"<名前>: <見えた状態>-><Reset 応答の状態>"` で残る(audit 行き)。
    #[test]
    fn a_component_left_configured_is_reset_before_any_configure() {
        let params = params_with(&[0, 1]);
        let mut mock = MockTransport::new(&params)
            .component_in("receiver0", ComponentState::Configured)
            .component_in("decoder", ComponentState::Error);
        let report = Sequencer::new(&params, &mut mock)
            .start_run(57, "", "2026-10-11T10:00:00.000")
            .unwrap();

        assert_eq!(
            &mock.commands()[..4],
            &[
                Call::new("ecc", "status"),
                // 停止・巻き戻しと同じく上流から(receiver1, receiver0, decoder, graw-writer)。
                // Idle の receiver1 / graw-writer には Reset を撃たない。Running ではないので
                // Stop も撃たない。
                Call::new("receiver0", "Reset"),
                Call::new("decoder", "Reset"),
                Call::new("graw-writer", "Configure"),
            ]
        );
        assert_eq!(
            report.component_walk_back,
            vec![
                "receiver0: Configured->Idle".to_string(),
                "decoder: Error->Idle".to_string(),
            ]
        );
        assert_eq!(
            mock.actions("graw-writer"),
            vec!["Configure", "Arm", "Start"]
        );
        assert_eq!(mock.actions("receiver1"), vec!["Configure", "Arm", "Start"]);
        // Running が居ない = 前の run は通常の停止で既に閉じている(run_stop も書かれている)
        // ので、歩き戻しは run_stop の材料を作らない(115)。
        assert_eq!(report.walked_back_run, None);
    }

    /// Running のまま残ったコンポーネントは **Stop(EOS を流す)→ Reset** の順で戻し、
    /// Configure はその後(v1.6 の「run を閉じる前に Reset しない」と同じ理由)。
    /// 跡は Stop の着地(§1.3 遷移表: Running --Stop--> Configured)も挟む。
    /// ここは一部だけ Running(receiver0 は Armed)なので EOS は待たず、跡の先頭に
    /// その旨が載る(114)。Running の Stop は**どの Reset よりも先**(115: 前の run の
    /// run_stop の材料を Stop の後・Reset の前に採るため)。
    #[test]
    fn a_running_component_is_stopped_before_the_reset_and_configured_after() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params)
            .component_in("decoder", ComponentState::Running)
            .component_in("receiver0", ComponentState::Armed);
        let report = Sequencer::new(&params, &mut mock)
            .start_run(57, "", "2026-10-11T10:00:00.000")
            .unwrap();

        assert_eq!(
            &mock.commands()[..5],
            &[
                Call::new("ecc", "status"),
                Call::new("decoder", "Stop"),
                Call::new("receiver0", "Reset"), // Armed → Reset のみ
                Call::new("decoder", "Reset"),
                Call::new("graw-writer", "Configure"),
            ]
        );
        assert_eq!(
            report.component_walk_back,
            vec![
                "eos: not awaited — the chain is broken (not every component is Running)"
                    .to_string(),
                "receiver0: Armed->Idle".to_string(),
                "decoder: Running->Configured->Idle".to_string(),
            ]
        );
        assert_eq!(
            mock.actions("decoder"),
            vec!["Stop", "Reset", "Configure", "Arm", "Start"]
        );
    }

    /// Running の相手が Stop を断ったら Reset へは進まず、stage `walk-back` で止まる。
    #[test]
    fn a_running_component_that_refuses_the_stop_is_not_reset_and_the_run_does_not_start() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params)
            .component_in("decoder", ComponentState::Running)
            .fail_on("decoder", "Stop");
        let failure = Sequencer::new(&params, &mut mock)
            .start_run(57, "", "2026-10-11T10:00:00.000")
            .unwrap_err();

        assert_eq!(failure.stage, "walk-back");
        assert!(failure.detail.contains("decoder"), "{}", failure.detail);
        assert!(failure.detail.contains("Running"), "{}", failure.detail);
        assert_eq!(mock.actions("decoder"), vec!["Stop"]);
        assert!(!mock.commands().iter().any(|c| c.action == "Configure"));
    }

    /// 全員 Idle なら歩き戻しの跡は空(1 本目・正常な連続 run の姿)。閉じる run も無い
    /// (115: run_stop は書かない)。
    #[test]
    fn idle_components_leave_an_empty_walk_back_trail() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params);
        let report = Sequencer::new(&params, &mut mock)
            .start_run(57, "", "2026-10-11T10:00:00.000")
            .unwrap();
        assert!(report.component_walk_back.is_empty());
        assert!(!mock.commands().iter().any(|c| c.action == "Reset"));
        assert_eq!(report.walked_back_run, None);
    }

    /// 歩き戻しの `Reset` が断られたら run を始めない。**どれが・何の状態で**残ったかを
    /// 文言に載せ、どのコンポーネントにも Configure を出さない。
    #[test]
    fn a_component_that_refuses_the_walk_back_reset_stops_the_run_before_any_configure() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params)
            .component_in("receiver0", ComponentState::Armed)
            .fail_on("receiver0", "Reset");
        let failure = Sequencer::new(&params, &mut mock)
            .start_run(57, "", "2026-10-11T10:00:00.000")
            .unwrap_err();

        assert_eq!(failure.stage, "walk-back");
        assert!(failure.detail.contains("receiver0"), "{}", failure.detail);
        assert!(failure.detail.contains("Armed"), "{}", failure.detail);
        assert!(
            !mock.commands().iter().any(|c| c.action == "Configure"),
            "{:?}",
            mock.commands()
        );
    }

    // -----------------------------------------------------------------
    // 114 — 歩き戻しで前の run を EOS で閉じる(チェーンが無傷のときだけ)
    // -----------------------------------------------------------------

    /// **114 の本体**: run 中に controller だけが落ちて再起動した姿(相 Idle、ECC も
    /// コンポーネントも全員 Running)。チェーンが無傷なので、**停止経路の 2 段そのもの**で
    /// 前の run を閉じてから歩き戻す(114 追補): 第一段 = 自然 EOS か受信静止の早い方まで待つ
    /// (`receiver::drain_task` は Stop で即 break してカーネル受信バッファの未読分を捨てるので、
    /// 静止を確かめてから Stop する)→ 閉じていなければ全 receiver へ `Stop`(強制 EOS)→
    /// 第二段 = EOS 待ち → decoder `Stop` → graw-writer `Stop` → 上流から `Reset` × 全員 →
    /// Configure。ECC はそれより先に歩き戻る(= zCoBo は既に止まっている)。
    ///
    /// ここは「第一段は静止で切り上がる(EOS は来ない)→ 強制 EOS → 第二段で閉じる」の形。
    /// 手計算(eos_poll = 200 ms、eos_quiesce = 500 ms、受信バイト数は不変、
    /// `eos_after_polls = Some(7)`。MockTransport が EOS 側に数えるのは decoder / graw-writer
    /// への GetStatus だけ):
    /// - 状態調べ: decoder = 1・graw-writer = 2(receiver は数えない)。
    /// - 第一段: 1 周 = graw-writer への判定 1 回(files_open = 1 で打ち切るので decoder へは
    ///   行かない)+ receiver 2 台の bytes。graw-writer = 3, 4, 5, 6 の 4 周で静止 0 → 200 →
    ///   400 → 600 ms ≥ 500 ms → 切り上げ。眠るのは 3 回 = 600 ms。receiver はそれぞれ
    ///   状態調べ 1 + 第一段 4 = **5 回**問い合わされてから Stop される。
    /// - 第二段: graw-writer = 7(≤ 7 = 未着)→ 200 ms → graw-writer = 8・decoder = 9
    ///   (> 7 = 到着)。眠るのは 1 回 = 200 ms。
    /// - 合計 800 ms。receiver0 の Stop と decoder の Stop のあいだは graw-writer × 2 + decoder × 1。
    #[test]
    fn a_fully_running_chain_is_closed_with_eos_before_the_walk_back_resets() {
        let params = params_with(&[0, 1]);
        let mut mock = MockTransport::new(&params)
            .ecc_in("Running")
            .component_in("graw-writer", ComponentState::Running)
            .component_in("decoder", ComponentState::Running)
            .component_in("receiver0", ComponentState::Running)
            .component_in("receiver1", ComponentState::Running);
        mock.eos_after_polls = Some(7);
        let report = Sequencer::new(&params, &mut mock)
            .start_run(59, "", "2026-10-12T08:43:00.000")
            .unwrap();

        let first_receiver_stop = mock.index_of("receiver1", "Stop").unwrap();
        for receiver in ["receiver0", "receiver1"] {
            let polls = mock.calls[..first_receiver_stop]
                .iter()
                .filter(|c| c.component == receiver && c.action == "GetStatus")
                .count();
            assert_eq!(
                polls, 5,
                "{receiver}: 静止を確かめずに Stop している: {:?}",
                mock.calls
            );
        }

        assert_eq!(
            &mock.commands()[..14],
            &[
                Call::new("ecc", "status"),
                Call::new("ecc", "stop"),
                Call::new("ecc", "breakup"),
                Call::new("ecc", "reset"),
                Call::new("ecc", "reset"),
                // 強制 EOS = 全 receiver へ Stop(上流から)。
                Call::new("receiver1", "Stop"),
                Call::new("receiver0", "Stop"),
                // EOS が流れ切ってから下流を Stop(decoder の Stop は自分の EOS を出さない)。
                Call::new("decoder", "Stop"),
                Call::new("graw-writer", "Stop"),
                Call::new("receiver1", "Reset"),
                Call::new("receiver0", "Reset"),
                Call::new("decoder", "Reset"),
                Call::new("graw-writer", "Reset"),
                Call::new("graw-writer", "Configure"),
            ]
        );
        let last_receiver_stop = mock.index_of("receiver0", "Stop").unwrap();
        let decoder_stop = mock.index_of("decoder", "Stop").unwrap();
        assert_eq!(
            &mock.calls[last_receiver_stop + 1..decoder_stop],
            &[
                Call::new("graw-writer", "GetStatus"),
                Call::new("graw-writer", "GetStatus"),
                Call::new("decoder", "GetStatus"),
            ],
            "第二段の EOS 待ちが receiver Stop と decoder Stop のあいだに無い"
        );
        assert_eq!(mock.slept, Duration::from_millis(800));
        assert_eq!(
            report.component_walk_back,
            vec![
                "eos: closed".to_string(),
                "receiver1: Running->Configured->Idle".to_string(),
                "receiver0: Running->Configured->Idle".to_string(),
                "decoder: Running->Configured->Idle".to_string(),
                "graw-writer: Running->Configured->Idle".to_string(),
            ]
        );
    }

    /// 全員 Running で、**第一段で自然 EOS が成立**する形(実機では ECC の歩き戻しの breakup で
    /// CoBo がリンクを close し、receiver が EOF から EOS を出す)。強制 EOS は撃たない ——
    /// receiver の `Stop` は EOS を見届けた**後**に、停止経路の畳む段と同じく下流と一緒に撃つ
    /// (跡は全員 `Running->Configured->Idle` のまま)。
    ///
    /// 手計算(`with_eos_closed` = 最初の判定から閉じている): 状態調べ(receiver0 → decoder →
    /// graw-writer)の直後、第一段の 1 回目の判定(graw-writer → decoder)で閉じる。眠らない。
    #[test]
    fn a_fully_running_chain_whose_eos_arrives_on_its_own_is_not_forced() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params)
            .with_eos_closed()
            .component_in("graw-writer", ComponentState::Running)
            .component_in("decoder", ComponentState::Running)
            .component_in("receiver0", ComponentState::Running);
        let report = Sequencer::new(&params, &mut mock)
            .start_run(59, "", "2026-10-12T08:43:00.000")
            .unwrap();

        assert_eq!(
            &mock.calls[..16],
            &[
                Call::new("ecc", "status"),
                // 状態調べ(上流から)。
                Call::new("receiver0", "GetStatus"),
                Call::new("decoder", "GetStatus"),
                Call::new("graw-writer", "GetStatus"),
                // 第一段: 自然 EOS が成立(receiver の静止観測には進まない)。
                Call::new("graw-writer", "GetStatus"),
                Call::new("decoder", "GetStatus"),
                // 強制 EOS は無し。畳む段と同じく上流から Stop、その後 Reset。
                Call::new("receiver0", "Stop"),
                Call::new("decoder", "Stop"),
                Call::new("graw-writer", "Stop"),
                // 115: 前の run の run_stop の材料(Stop の後・Reset の前、collect_status の順)。
                Call::new("graw-writer", "GetStatus"),
                Call::new("decoder", "GetStatus"),
                Call::new("receiver0", "GetStatus"),
                Call::new("receiver0", "Reset"),
                Call::new("decoder", "Reset"),
                Call::new("graw-writer", "Reset"),
                Call::new("graw-writer", "Configure"),
            ]
        );
        assert_eq!(mock.slept, Duration::ZERO);
        assert_eq!(
            report.component_walk_back,
            vec![
                "eos: closed".to_string(),
                "receiver0: Running->Configured->Idle".to_string(),
                "decoder: Running->Configured->Idle".to_string(),
                "graw-writer: Running->Configured->Idle".to_string(),
            ]
        );
    }

    /// 全員 Running だが EOS が `eos_timeout` 内に届かない(例: 下流が詰まって decoder が
    /// 自分の EOS を送れない)。跡に `eos: did not propagate within 5000 ms` を残し、それでも
    /// start は通す —— その後の Reset が送出待ちを EOS ごと打ち切り、打ち切りは各コンポーネントの
    /// metrics(`messages_abandoned` / `eos_abandoned`)で見える。待ち方は停止経路と同じ 2 段:
    /// 第一段は受信静止(バイト数不変)で 600 ms(eos_poll 200 ms × 3 回 ≥ eos_quiesce 500 ms)、
    /// 強制 EOS の後の第二段はハード上限 eos_timeout = 5 000 ms(200 ms × 25 回)まるごと。
    /// 合計 5 600 ms。
    #[test]
    fn a_running_chain_whose_eos_never_arrives_is_reset_anyway_and_the_run_starts() {
        let params = params_with(&[0]);
        // eos_after_polls = None = 永久に閉じない。
        let mut mock = MockTransport::new(&params)
            .component_in("graw-writer", ComponentState::Running)
            .component_in("decoder", ComponentState::Running)
            .component_in("receiver0", ComponentState::Running);
        let report = Sequencer::new(&params, &mut mock)
            .start_run(59, "", "2026-10-12T08:43:00.000")
            .unwrap();

        assert_eq!(mock.slept, Duration::from_millis(5_600));
        assert_eq!(
            report.component_walk_back,
            vec![
                "eos: did not propagate within 5000 ms".to_string(),
                "receiver0: Running->Configured->Idle".to_string(),
                "decoder: Running->Configured->Idle".to_string(),
                "graw-writer: Running->Configured->Idle".to_string(),
            ]
        );
        for name in ["receiver0", "decoder", "graw-writer"] {
            assert_eq!(
                mock.actions(name),
                vec!["Stop", "Reset", "Configure", "Arm", "Start"],
                "{name}"
            );
        }
    }

    /// 全員 Running の経路で receiver / 下流の `Stop` が通り EOS も閉じた**後**に receiver0 の
    /// `Reset` が断られたら、文言は**その時点の**状態(Stop の着地 = Configured)を名乗る ——
    /// 歩き戻し前の Running ではない(オペレータが次に何を見るべきかを誤らせない)。
    /// Reset は上流から 1 つずつなので、receiver0 で止まり decoder / graw-writer には届かない。
    #[test]
    fn a_reset_refused_after_the_eos_close_names_the_state_the_component_was_left_in() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params)
            .with_eos_closed()
            .component_in("graw-writer", ComponentState::Running)
            .component_in("decoder", ComponentState::Running)
            .component_in("receiver0", ComponentState::Running)
            .reports_run("graw-writer", 58)
            .reports_run("decoder", 58)
            .reports_run("receiver0", 58)
            .fail_on("receiver0", "Reset");
        let failure = Sequencer::new(&params, &mut mock)
            .start_run(59, "", "2026-10-12T08:43:00.000")
            .unwrap_err();

        assert_eq!(failure.stage, "walk-back");
        assert!(
            failure.detail.contains("receiver0 was left Configured"),
            "{}",
            failure.detail
        );
        assert_eq!(mock.actions("receiver0"), vec!["Stop", "Reset"]);
        assert_eq!(mock.actions("decoder"), vec!["Stop"]);
        assert_eq!(mock.actions("graw-writer"), vec!["Stop"]);
        // 115: run 58 は Reset の前に EOS で閉じて材料も採れているので、歩き戻しが途中で
        // 落ちても失敗報告に載る(呼び手が run_stop を書く)。
        let closed = failure
            .walked_back_run
            .expect("closed run rides on the failure");
        assert_eq!(closed.run, Ok(58));
        assert!(closed.eos_closed);
    }

    /// 一部だけ Running(decoder だけ。receiver0 は前の停止で Reset 済み = Idle、graw-writer は
    /// Configured のまま)。チェーンが途中で切れているので EOS は端まで届かず観測もできない ——
    /// **待たない**。跡の先頭に「待たない」旨の 1 行を残し、Running の decoder は 113 と同じく
    /// Stop → Reset。decoder / graw-writer への GetStatus は状態調べと run_stop の材料集め
    /// (115)の 1 回ずつだけ。閉じ損ねた run 58 も run_stop の材料になる(`eos_closed: false`。
    /// receiver は Running でなかったので Stop していない = `forced_eos: false`)。
    #[test]
    fn a_partly_running_chain_is_walked_back_without_waiting_for_eos() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params)
            .component_in("decoder", ComponentState::Running)
            .component_in("graw-writer", ComponentState::Configured)
            .reports_run("decoder", 58)
            .reports_run("graw-writer", 58);
        let report = Sequencer::new(&params, &mut mock)
            .start_run(59, "", "2026-10-12T08:43:00.000")
            .unwrap();

        assert_eq!(
            &mock.commands()[..5],
            &[
                Call::new("ecc", "status"),
                Call::new("decoder", "Stop"),
                Call::new("decoder", "Reset"),
                Call::new("graw-writer", "Reset"),
                Call::new("graw-writer", "Configure"),
            ]
        );
        let first_configure = mock.index_of("graw-writer", "Configure").unwrap();
        let eos_side_polls = mock.calls[..first_configure]
            .iter()
            .filter(|c| {
                c.action == "GetStatus"
                    && (c.component == "decoder" || c.component == "graw-writer")
            })
            .count();
        assert_eq!(
            eos_side_polls, 4,
            "状態調べ・材料集め以外の GetStatus(= EOS 待ち)が出ている: {:?}",
            mock.calls
        );
        assert_eq!(mock.slept, Duration::ZERO);
        assert_eq!(
            report.component_walk_back,
            vec![
                "eos: not awaited — the chain is broken (not every component is Running)"
                    .to_string(),
                "decoder: Running->Configured->Idle".to_string(),
                "graw-writer: Configured->Idle".to_string(),
            ]
        );
        let closed = report.walked_back_run.unwrap();
        assert_eq!(closed.run, Ok(58));
        assert!(!closed.eos_closed);
        assert!(!closed.forced_eos);
    }

    // -----------------------------------------------------------------
    // 115 — 歩き戻しで閉じた前の run の run_stop の材料
    // -----------------------------------------------------------------

    /// **115 の本体(材料側)**: 全員 Running で run 58 を EOS で閉じたら、run_stop の材料を
    /// **Stop の後・Reset の前**に `GetStatus` で採って返す(graw-writer は Reset でファイル
    /// 実績を手放すため)。run 番号は `run_number` を申告した全員の一致(ここでは graw-writer は
    /// 申告しない = 数に入れない)。第一段で自然 EOS なので `forced_eos: false`。
    ///
    /// 値の出典(MockTransport の metrics): receiver0 の frames = 3000 + 並び順 2 = 3002、
    /// overflow_frames = 0、decoder の malformed = 7、graw-writer の files は 1 本 30 108 672 B。
    #[test]
    fn a_fully_running_chain_hands_its_closed_run_back_for_the_run_stop() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params)
            .with_eos_closed()
            .component_in("graw-writer", ComponentState::Running)
            .component_in("decoder", ComponentState::Running)
            .component_in("receiver0", ComponentState::Running)
            .reports_run("decoder", 58)
            .reports_run("receiver0", 58);
        let report = Sequencer::new(&params, &mut mock)
            .start_run(59, "", "2026-10-12T08:43:00.000")
            .unwrap();

        let last_stop = mock.index_of("graw-writer", "Stop").unwrap();
        let first_reset = mock.index_of("receiver0", "Reset").unwrap();
        assert_eq!(
            &mock.calls[last_stop + 1..first_reset],
            &[
                Call::new("graw-writer", "GetStatus"),
                Call::new("decoder", "GetStatus"),
                Call::new("receiver0", "GetStatus"),
            ],
            "材料集めが Stop の後・Reset の前に無い"
        );
        let mut frames = BTreeMap::new();
        frames.insert("0".to_string(), 3002);
        assert_eq!(
            report.walked_back_run,
            Some(WalkedBackRun {
                run: Ok(58),
                forced_eos: false,
                eos_closed: true,
                counters: Counters {
                    events_built: None,
                    events_incomplete: None,
                    late_fragments: None,
                    frames,
                    overflow_frames: Some(0),
                    malformed: Some(7),
                },
                files: vec![OutputFile {
                    path: "run0001/CoBo0_AsAd0_ts_0000.graw".to_string(),
                    bytes: 30_108_672,
                }],
                notes: Vec::new(),
            })
        );
    }

    /// run 番号の申告が食い違ったら番号を決めない —— 推測で台帳を汚さない。理由は
    /// 申告をそのまま並べて残す(呼び手は run_stop を書かずに audit の note へ回す)。
    #[test]
    fn a_run_number_the_components_disagree_on_is_not_used() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params)
            .with_eos_closed()
            .component_in("graw-writer", ComponentState::Running)
            .component_in("decoder", ComponentState::Running)
            .component_in("receiver0", ComponentState::Running)
            .reports_run("graw-writer", 58)
            .reports_run("decoder", 57)
            .reports_run("receiver0", 58);
        let report = Sequencer::new(&params, &mut mock)
            .start_run(59, "", "2026-10-12T08:43:00.000")
            .unwrap();

        let closed = report.walked_back_run.unwrap();
        assert_eq!(
            closed.run,
            Err("components disagree on the previous run number \
                 (receiver0=58, decoder=57, graw-writer=58) — run_stop not written"
                .to_string())
        );
    }

    /// 一部だけ Running でも receiver が Running なら、その `Stop` が EOS を注入する
    /// (= `forced_eos: true`)。待たないので `eos_closed: false`。
    #[test]
    fn a_running_receiver_in_a_broken_chain_counts_as_a_forced_eos() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params)
            .component_in("graw-writer", ComponentState::Running)
            .component_in("receiver0", ComponentState::Running)
            .reports_run("graw-writer", 61)
            .reports_run("receiver0", 61);
        let report = Sequencer::new(&params, &mut mock)
            .start_run(62, "", "2026-10-12T08:43:00.000")
            .unwrap();

        let closed = report.walked_back_run.unwrap();
        assert_eq!(closed.run, Ok(61));
        assert!(closed.forced_eos);
        assert!(!closed.eos_closed);
    }

    /// 歩き戻しの**後**で start が落ちても(ここでは graw-writer が Configure を断る)、
    /// 前の run は閉じているので材料は失敗報告に載る(呼び手が run_stop を書く)。
    #[test]
    fn a_start_that_fails_after_the_walk_back_still_hands_back_the_closed_run() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params)
            .with_eos_closed()
            .component_in("graw-writer", ComponentState::Running)
            .component_in("decoder", ComponentState::Running)
            .component_in("receiver0", ComponentState::Running)
            .reports_run("graw-writer", 58)
            .reports_run("decoder", 58)
            .reports_run("receiver0", 58)
            .fail_on("graw-writer", "Configure");
        let failure = Sequencer::new(&params, &mut mock)
            .start_run(59, "", "2026-10-12T08:43:00.000")
            .unwrap_err();

        assert_eq!(failure.stage, "Configure");
        let closed = failure.walked_back_run.unwrap();
        assert_eq!(closed.run, Ok(58));
        assert!(closed.eos_closed);
    }

    /// GetStatus が届かないコンポーネントも、状態が判らないまま Configure へ進まない。
    #[test]
    fn an_unreachable_component_stops_the_run_at_the_walk_back() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params).unreachable("decoder");
        let failure = Sequencer::new(&params, &mut mock)
            .start_run(57, "", "2026-10-11T10:00:00.000")
            .unwrap_err();

        assert_eq!(failure.stage, "walk-back");
        assert!(
            failure.detail.contains("decoder: GetStatus unreachable"),
            "{}",
            failure.detail
        );
        assert!(!mock.commands().iter().any(|c| c.action == "Configure"));
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

    /// **強制 EOS 分岐(ハード上限)**: EOS も来ず、receiver も**受信し続けている**
    /// (= 静止しない)ままなら、第一段は `eos_timeout` まで待ってから強制 EOS を撃つ。
    /// 撃った receiver へは畳む段で Stop を撃ち直さない(Reset だけ)。
    ///
    /// 033-E で前提を明示化: 受信が止まらないので **quiesce では切り上がらない**
    /// (静止で切り上がる姿は `stop_forces_eos_as_soon_as_the_receivers_go_quiet`)。
    #[test]
    fn stop_forces_eos_through_the_receivers_after_the_timeout() {
        let params = params_with(&[0]);
        // 1 回の EOS 判定 = graw-writer への GetStatus 1 回(files_open != 0 でそこで打ち切る)。
        // 最初の待ちは「即時判定 1 回 + 200 ms 毎に 25 回(= eos_timeout 5 s)」= 26 回で
        // 諦める。27 回目(= 強制 EOS 直後の判定)で閉じる形にすると、眠った合計時間は
        // ちょうど eos_timeout になる。
        // (receiver への静止観測はこの数に入らない —— MockTransport の `eos_polls` 参照。)
        let mut mock = MockTransport::new(&params).receivers_still_receiving(4096);
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

    // -----------------------------------------------------------------
    // 停止第一段の受信静止検出(SPEC §1.3 v1.12 / TODO/033-E)
    // -----------------------------------------------------------------

    /// **静止検出**: 実機の停止経路(EOF が来ない)で、receiver の受信バイト数が
    /// `eos_quiesce` = 500 ms のあいだ不変になったら、`eos_timeout` を待たずに強制 EOS へ。
    ///
    /// 手計算(eos_poll = 200 ms、eos_quiesce = 500 ms):
    /// 1 サンプル目は比較相手が無いので静止 0 ms → sleep(200)→ 2 サンプル目で 200 ms →
    /// sleep(200)→ 3 サンプル目で 400 ms → sleep(200)→ 4 サンプル目で 600 ms ≥ 500 ms。
    /// よって**眠るのは 3 回 = 600 ms**(従来の 5 000 ms から 1/8 以下)。
    #[test]
    fn stop_forces_eos_as_soon_as_the_receivers_go_quiet() {
        let params = params_with(&[0]);
        // bytes_step = 0(既定)= 受信が止まっている。EOS は自然には来ない。
        let mut mock = MockTransport::new(&params);
        mock.eos_after_polls = Some(4); // 強制 EOS の後だけ閉じる
        let report = Sequencer::new(&params, &mut mock).stop_run(StopMode::Normal);

        assert!(report.forced_eos, "静止したら強制 EOS へ進む");
        assert!(report.eos_closed, "強制 EOS の後は閉じる");
        assert_eq!(report.reason, "normal");
        assert_eq!(
            mock.slept,
            Duration::from_millis(600),
            "静止検出なのに eos_timeout を待っている"
        );
        assert!(
            mock.slept < params.eos_timeout,
            "ハード上限より早く畳めていない"
        );
        // 静止は「畳む口実」であって異常ではない —— note は 1 つも増えない。
        assert!(report.notes.is_empty(), "{:?}", report.notes);
    }

    /// 自然 EOS が静止より**先**に成立したら強制しない(早い方で進む、の片側)。
    #[test]
    fn a_natural_eos_wins_over_the_quiesce_detection() {
        let params = params_with(&[0]);
        // 受信は止まっている(静止条件は 600 ms で成立する)が、EOS は 1 回目で成立。
        let mut mock = MockTransport::new(&params).with_eos_closed();
        let report = Sequencer::new(&params, &mut mock).stop_run(StopMode::Normal);

        assert!(!report.forced_eos, "自然 EOS が先なら注入しない");
        assert_eq!(mock.slept, Duration::ZERO, "1 回も眠らない");
        assert_eq!(report.reason, "normal");
    }

    /// **不達の receiver は静止扱い**(drain できない相手を待っても無意味)。
    /// 不達そのものは第二段の `Stop` 不達として note に出る = silent にしない。
    #[test]
    fn an_unreachable_receiver_counts_as_quiet() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params).unreachable("receiver0");
        mock.eos_after_polls = Some(4);
        let report = Sequencer::new(&params, &mut mock).stop_run(StopMode::Normal);

        assert!(report.forced_eos);
        assert_eq!(
            mock.slept,
            Duration::from_millis(600),
            "不達を「まだ受信中」と読んでハード上限まで待っている"
        );
        assert!(
            report
                .notes
                .iter()
                .any(|n| n.contains("receiver0") && n.contains("unreachable")),
            "不達を silent にしている: {:?}",
            report.notes
        );
        // **不達へ撃ち直さない**: 静止判定の間に GetStatus を送るのは 1 回だけ
        // (不達の REQ は毎回 command_timeout を丸ごと燃やす —— F1 で 55 s を実測した)。
        // 残りは強制 EOS の Stop 1 回 + 畳む段の Stop / Reset。
        let receiver_status_polls = mock
            .calls
            .iter()
            .filter(|c| c.component == "receiver0" && c.action == "GetStatus")
            .count();
        assert_eq!(
            receiver_status_polls, 2,
            "静止判定で 1 回 + run_stop の材料集め(collect_status)で 1 回のはず: {:?}",
            mock.calls
        );
    }

    /// `bytes` を報告しない receiver が居たら**静止と判定しない**(尻尾を落とさない側に倒し、
    /// ハード上限に委ねる)。欠落は note に出す。
    #[test]
    fn a_receiver_without_a_bytes_metric_never_looks_quiet() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params).without_receiver_bytes("receiver0");
        mock.eos_after_polls = Some(26); // ハード上限の後に閉じる
        let report = Sequencer::new(&params, &mut mock).stop_run(StopMode::Normal);

        assert_eq!(
            mock.slept, params.eos_timeout,
            "判定材料が無いのに静止として切り上げている"
        );
        assert!(report.forced_eos);
        assert!(
            report
                .notes
                .iter()
                .any(|n| n.contains("GetStatus metrics has no bytes")),
            "欠落を silent にしている: {:?}",
            report.notes
        );
    }

    /// 2 CoBo: **1 台でも受信が続いていれば静止ではない**(全 receiver 不変が条件)。
    /// receiver0 は静止、receiver1 だけがまだ受信している —— 非対称な形。
    #[test]
    fn quiesce_needs_every_receiver_to_be_quiet() {
        let params = params_with(&[0, 1]);
        let mut mock = MockTransport::new(&params).receiver_still_receiving("receiver1", 1);
        mock.eos_after_polls = Some(26);
        let report = Sequencer::new(&params, &mut mock).stop_run(StopMode::Normal);

        assert_eq!(
            mock.slept, params.eos_timeout,
            "受信が続いているのに静止として切り上げている"
        );
        assert!(report.forced_eos);
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

    /// **033-C(併発見の穴)**: 上流 EOS は全部届き(`eos_in` 充足)graw-writer も
    /// 全ファイルを閉じたが、**decoder が自分の EOS を送れていない**(`eos_out == 0` ——
    /// root-sink 停滞 + PUSH HWM 満杯)状態。
    ///
    /// 2 点判定の頃はこれを「EOS 完了」と読んで **reason=normal** を書いてしまえた。
    /// 3 点目が入った今は完了と読まず、強制 EOS → それでも届かず `error:eos-timeout`。
    #[test]
    fn eos_is_not_complete_while_the_decoder_has_not_sent_its_own_eos() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params);
        // 非対称: eos_in は期待(1 CoBo)を**超えて**満たし、files_open も 0。
        // 足りないのは eos_out だけ。
        mock.metrics.insert(
            "decoder".to_string(),
            json!({"eos_in": 3, "eos_out": 0, "malformed": 0}),
        );
        mock.metrics.insert(
            "graw-writer".to_string(),
            json!({"files_open": 0, "files": []}),
        );

        let report = Sequencer::new(&params, &mut mock).stop_run(StopMode::Normal);

        assert!(
            !report.eos_closed,
            "decoder が EOS を送っていないのに閉じたことにしている"
        );
        assert!(report.forced_eos, "完了でないなら強制 EOS へ進む");
        assert_eq!(report.reason, "error:eos-timeout");
        assert!(!report.ok);
    }

    /// 同じ状況で `eos_out` が 1 になれば(= 最後のホップが通れば)正常停止として閉じる。
    /// 上の負性テストと対にして「3 点目だけが違い」であることを固定する。
    #[test]
    fn eos_is_complete_once_the_decoder_reports_its_own_eos_out() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params);
        mock.metrics.insert(
            "decoder".to_string(),
            json!({"eos_in": 3, "eos_out": 1, "malformed": 0}),
        );
        mock.metrics.insert(
            "graw-writer".to_string(),
            json!({"files_open": 0, "files": []}),
        );

        let report = Sequencer::new(&params, &mut mock).stop_run(StopMode::Normal);

        assert!(report.eos_closed);
        assert!(!report.forced_eos);
        assert_eq!(report.reason, "normal");
    }

    /// `eos_out` を持たない decoder(= 古い metrics)を「完了」と読まない。
    /// 欠落は **note に出す**(silent failure を作らない — CLAUDE.md)。
    #[test]
    fn a_decoder_without_eos_out_is_never_read_as_complete() {
        let params = params_with(&[0]);
        let mut mock = MockTransport::new(&params);
        mock.metrics
            .insert("decoder".to_string(), json!({"eos_in": 3}));
        mock.metrics.insert(
            "graw-writer".to_string(),
            json!({"files_open": 0, "files": []}),
        );

        let report = Sequencer::new(&params, &mut mock).stop_run(StopMode::Normal);

        assert!(!report.eos_closed);
        assert!(
            report
                .notes
                .iter()
                .any(|n| n.contains("GetStatus metrics has no eos_out")),
            "欠落を silent にしている: {:?}",
            report.notes
        );
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
        mock.metrics.insert(
            "decoder".to_string(),
            json!({"eos_in": 9, "eos_out": 1, "malformed": 4}),
        );
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
    // 115 — 歩き戻しで閉じた前の run の run_stop(台帳側)
    // -----------------------------------------------------------------

    /// 前の run の `run_start`(controller 自身が書いたものと同じ形)。
    fn run_start_of(run: u32) -> LogbookRecord {
        LogbookRecord::RunStart {
            actor: ACTOR.to_string(),
            run,
            config_id: "mock".to_string(),
            config_ids: None,
            geometry: GeometryRecord {
                path: "/tmp/tpcdaq-mock/geometry.dat".to_string(),
                sha256: "ab12".to_string(),
            },
            cobos: vec![CoboListen {
                id: 0,
                listen: "0.0.0.0:46005".to_string(),
            }],
            operator: "aogaki".to_string(),
            comment: "alpha".to_string(),
            expected_fragments: vec![(0, 0), (0, 1)],
            slow: None,
            params: None,
        }
    }

    /// 歩き戻しが返す材料(非対称な値: CoBo 0 = 68 フレーム、graw 2 本)。
    fn walked_back(run: Result<u32, String>) -> WalkedBackRun {
        let mut frames = BTreeMap::new();
        frames.insert("0".to_string(), 68);
        WalkedBackRun {
            run,
            forced_eos: true,
            eos_closed: true,
            counters: Counters {
                events_built: None,
                events_incomplete: None,
                late_fragments: None,
                frames,
                overflow_frames: Some(0),
                malformed: Some(2),
            },
            files: vec![
                OutputFile {
                    path: "run0058/CoBo0_AsAd0_ts_0000.graw".to_string(),
                    bytes: 6528,
                },
                OutputFile {
                    path: "run0058/CoBo0_AsAd1_ts_0000.graw".to_string(),
                    bytes: 6432,
                },
            ],
            notes: Vec::new(),
        }
    }

    fn logbook_records(controller: &Controller) -> Vec<LogbookRecord> {
        crate::logbook::read_since(&controller.logbook_path, 0)
            .unwrap()
            .records
            .into_iter()
            .map(|line| line.record)
            .collect()
    }

    /// `duration_s` は前の run の `run_start.ts` から測る。オフセットが違っても同じ瞬間で比べる。
    /// 手計算: 08:42:00.000+03:00 = 05:42:00.000Z、now = 05:42:15.250Z → 15.25 s。
    #[test]
    fn seconds_since_measures_across_utc_offsets() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-12T05:42:15.250+00:00").unwrap();
        assert_eq!(
            seconds_since("2026-10-12T08:42:00.000+03:00", now),
            Some(15.25)
        );
        assert_eq!(seconds_since("not a timestamp", now), None);
    }

    /// **115 の本体(台帳側)**: 番号が決まった前の run には run_stop を書く ——
    /// `reason: "abort:controller-restart"`、`ok: false`、EOS の顛末と counters / files は
    /// 歩き戻しの材料そのまま、`duration_s` はその run の run_start からの経過。
    /// 戻り値は audit `run/start.previous_run_stop` に載る要約。
    #[test]
    fn a_walked_back_run_gets_a_run_stop_marked_controller_restart() {
        let controller = test_controller("orphan-stop");
        controller.append(run_start_of(58));

        let summary = append_walked_back_run_stop(&controller, &walked_back(Ok(58)));

        assert_eq!(summary, json!({"run": 58, "notes": []}));
        let records = logbook_records(&controller);
        assert_eq!(records.len(), 2, "{records:?}");
        let LogbookRecord::RunStop {
            run,
            duration_s,
            ok,
            reason,
            forced_eos,
            eos_closed,
            counters,
            files,
            ..
        } = &records[1]
        else {
            panic!("run_stop が書かれていない: {records:?}");
        };
        assert_eq!(*run, 58);
        assert!(!ok);
        assert_eq!(reason, "abort:controller-restart");
        assert!(*forced_eos);
        assert!(*eos_closed);
        let expected = walked_back(Ok(58));
        assert_eq!(counters, &expected.counters);
        assert_eq!(files, &expected.files);
        // run_start を書いた直後なので経過は 0 s 以上・数秒未満。
        let duration = duration_s.expect("run_start があるので duration_s は数値");
        assert!((0.0..10.0).contains(&duration), "{duration}");
        let _ = std::fs::remove_dir_all(&controller.params.output_root);
    }

    /// 番号が決まらない(申告の食い違い)なら run_stop は書かない。理由は要約の notes へ
    /// (audit に残る = silent にしない)。
    #[test]
    fn a_walked_back_run_without_an_agreed_number_writes_no_run_stop() {
        let controller = test_controller("orphan-disagree");
        controller.append(run_start_of(58));
        let why = "components disagree on the previous run number \
                   (receiver0=58, decoder=57) — run_stop not written"
            .to_string();

        let summary = append_walked_back_run_stop(&controller, &walked_back(Err(why.clone())));

        assert_eq!(summary, json!({"run": null, "notes": [why]}));
        let records = logbook_records(&controller);
        assert!(
            !records
                .iter()
                .any(|r| matches!(r, LogbookRecord::RunStop { .. })),
            "{records:?}"
        );
        let _ = std::fs::remove_dir_all(&controller.params.output_root);
    }

    /// ログブックの最後の run_start は**照合だけ**に使う: 番号が違えば note を残し、
    /// 同じ番号の run_start が無ければ `duration_s` は null(この経路に限り nullable)。
    /// run_stop そのものはコンポーネントの申告した番号で書く。
    #[test]
    fn a_walked_back_run_missing_from_the_logbook_gets_a_null_duration_and_notes() {
        let controller = test_controller("orphan-no-start");
        controller.append(run_start_of(57));

        let summary = append_walked_back_run_stop(&controller, &walked_back(Ok(58)));

        assert_eq!(
            summary,
            json!({"run": 58, "notes": [
                "the last run_start in the logbook is run 57, not 58",
                "no run_start for run 58 in the logbook — duration_s is null",
            ]})
        );
        let records = logbook_records(&controller);
        let Some(LogbookRecord::RunStop {
            run, duration_s, ..
        }) = records.last()
        else {
            panic!("run_stop が書かれていない: {records:?}");
        };
        assert_eq!(*run, 58);
        assert_eq!(*duration_s, None);
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

    // -----------------------------------------------------------------
    // slow/power(TODO/124 レーン B、SPEC §11 v1.42)
    // -----------------------------------------------------------------

    /// 規則の全表(4 相 × HV/LV × ON/OFF = 16 通り): HV ON と LV OFF は Idle だけ、
    /// HV OFF と LV ON は常時(Starting / Stopping も Running と同じく「run が動いている」側)。
    #[test]
    fn the_power_rule_allows_hv_on_and_lv_off_only_while_idle() {
        for phase in [
            Phase::Idle,
            Phase::Starting,
            Phase::Running,
            Phase::Stopping,
        ] {
            let idle = phase == Phase::Idle;
            assert_eq!(
                power_rule("hivolta", true, phase).is_ok(),
                idle,
                "HV ON @ {phase}"
            );
            assert_eq!(
                power_rule("hmp2020", false, phase).is_ok(),
                idle,
                "LV OFF @ {phase}"
            );
            assert_eq!(
                power_rule("hivolta", false, phase),
                Ok(()),
                "HV OFF @ {phase}"
            );
            assert_eq!(
                power_rule("hmp2020", true, phase),
                Ok(()),
                "LV ON @ {phase}"
            );
        }
        assert_eq!(
            power_rule("hivolta", true, Phase::Starting),
            Err("HV ON is allowed only while no run is active")
        );
        assert_eq!(
            power_rule("hmp2020", false, Phase::Stopping),
            Err("LV OFF is refused while a run is active — stop the run first")
        );
    }

    /// Fable B テスト「HV ON は Starting / Stopping でも 409」。統合テストでは Starting /
    /// Stopping の窓を確実には作れないので、相を直接置いてハンドラを叩く。規則で断るので
    /// slowctl へは送らない(`slow_cmd` は届かない口のままでよい)。409 も audit に 1 行ずつ。
    #[tokio::test(flavor = "multi_thread")]
    async fn hv_on_and_lv_off_are_409_while_starting_or_stopping_and_each_is_audited() {
        let mut params = params_with(&[]);
        params.output_root =
            std::env::temp_dir().join(format!("tpcdaq-controller-power-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&params.output_root);
        params.slow_cmd = Some("tcp://127.0.0.1:1".to_string());
        params.slow_devices = vec![
            ("hv".to_string(), "hivolta".to_string()),
            ("lv".to_string(), "hmp2020".to_string()),
        ];
        let controller = Arc::new(Controller::new(params).unwrap());
        let (token, _) = controller.acquire("aogaki");

        for phase in [Phase::Starting, Phase::Stopping] {
            lock(&controller.run).phase = phase;
            for (device, ch, on, why) in [
                (
                    "hv",
                    3,
                    true,
                    "HV ON is allowed only while no run is active",
                ),
                (
                    "lv",
                    2,
                    false,
                    "LV OFF is refused while a run is active — stop the run first",
                ),
            ] {
                let request: SlowPowerRequest = serde_json::from_value(
                    json!({"token": token, "device": device, "ch": ch, "on": on}),
                )
                .unwrap();
                let (status, Json(body)) =
                    post_slow_power(State(Arc::clone(&controller)), Json(request)).await;
                assert_eq!(status, StatusCode::CONFLICT, "{device} {on} @ {phase}");
                assert_eq!(body["error"], json!(why));
            }
        }
        assert!(
            lock(&controller.run).power_inflight.is_none(),
            "規則で断ったら印は立てない"
        );

        let audits: Vec<Value> = crate::logbook::read_since(&controller.logbook_path, 0)
            .unwrap()
            .records
            .into_iter()
            .filter_map(|line| match line.record {
                LogbookRecord::Audit {
                    action,
                    params,
                    ok,
                    error,
                    ..
                } if action == "slow/power" => {
                    Some(json!({"params": params, "ok": ok, "error": error}))
                }
                _ => None,
            })
            .collect();
        // 手計算: 2 相 × 2 操作 = 4 行。
        assert_eq!(audits.len(), 4, "{audits:?}");
        assert_eq!(
            audits[0],
            json!({
                "params": {"device": "hv", "ch": 3, "on": true, "phase": "Starting",
                           "before": null, "after": null},
                "ok": false,
                "error": "HV ON is allowed only while no run is active",
            })
        );
        assert_eq!(audits[3]["params"]["phase"], json!("Stopping"));
        assert_eq!(audits[3]["params"]["device"], json!("lv"));

        let _ = std::fs::remove_dir_all(&controller.params.output_root);
    }
}
