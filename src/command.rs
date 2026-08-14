//! コマンドチャネル(JSON REQ/REP、ComponentState 遷移)。SPEC §2.6。
//!
//! 制御面は JSON(serde_json)/ REQ/REP。データ面の MessagePack([`crate::msg`])とは
//! 明確に分ける(SPEC §2.1、delila-rs と同一)。
//!
//! # 状態機械(SPEC §1.3 — delila-rs をそのまま採用)
//!
//! ```text
//!   Idle --Configure--> Configured --Arm--> Armed --Start--> Running
//!                            ^                                  |
//!                            +--------------- Stop -------------+
//!   Configured / Armed / Running --Reset--> Idle
//!   Configured / Armed / Running --(異常)--> Error --Reset--> Idle
//! ```

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tmq::AsZmqSocket;
use tokio::sync::{broadcast, oneshot};
use tracing::{error, info, warn};

// ---------------------------------------------------------------------
// 状態機械(SPEC §1.3)
// ---------------------------------------------------------------------

/// コンポーネントの状態(SPEC §1.3)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ComponentState {
    /// 初期状態。設定を持たない。
    #[default]
    Idle,
    /// 設定を読み込み検証済み。
    Configured,
    /// 資源を確保済み(receiver はここで bind + listen = listen-before-start)。
    Armed,
    /// 取得・処理中。
    Running,
    /// 異常。`Reset` でのみ脱出できる。
    Error,
}

impl ComponentState {
    /// `target` への遷移が合法か(SPEC §1.3 の遷移表)。
    ///
    /// 自己遷移は常に不許可。Error へは活動中の状態からのみ入り、Error からは Idle だけへ出る。
    pub fn can_transition_to(&self, target: ComponentState) -> bool {
        use ComponentState::{Armed, Configured, Error, Idle, Running};
        matches!(
            (self, target),
            // 正常系
            (Idle, Configured)      // Configure
                | (Configured, Armed)   // Arm
                | (Armed, Running)      // Start
                | (Running, Configured) // Stop
                // Reset(活動中および Error から)
                | (Configured, Idle)
                | (Armed, Idle)
                | (Running, Idle)
                | (Error, Idle)
                // 異常発生
                | (Configured, Error)
                | (Armed, Error)
                | (Running, Error)
        )
    }
}

impl std::fmt::Display for ComponentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            ComponentState::Idle => "Idle",
            ComponentState::Configured => "Configured",
            ComponentState::Armed => "Armed",
            ComponentState::Running => "Running",
            ComponentState::Error => "Error",
        };
        f.write_str(name)
    }
}

// ---------------------------------------------------------------------
// コマンドとレスポンス(SPEC §2.6)
// ---------------------------------------------------------------------

/// `Configure` のペイロード。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunConfig {
    pub run_number: u32,
    /// run コメント(logbook 用)。
    #[serde(default)]
    pub comment: String,
    /// コンポーネント固有の設定断片。中身の解釈は受け手に任せる。
    #[serde(default)]
    pub config: serde_json::Value,
}

/// controller → コンポーネントのコマンド(SPEC §2.6。serde enum の JSON 表現)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Command {
    /// `{"Configure": {"run_number": N, "comment": "...", "config": {...}}}`
    Configure(RunConfig),
    /// `"Arm"`
    Arm,
    /// `{"Start": {"run_number": N}}`
    Start { run_number: u32 },
    /// `"Stop"`
    Stop,
    /// `"Reset"`
    Reset,
    /// `"GetStatus"` — 状態を変えない問い合わせ。
    GetStatus,
}

impl Command {
    /// このコマンドが目指す状態。`GetStatus` は遷移しないので `None`。
    ///
    /// 遷移が合法かどうかは [`ComponentState::can_transition_to`] が決める。
    pub fn target_state(&self) -> Option<ComponentState> {
        match self {
            Command::Configure(_) => Some(ComponentState::Configured),
            Command::Arm => Some(ComponentState::Armed),
            Command::Start { .. } => Some(ComponentState::Running),
            Command::Stop => Some(ComponentState::Configured),
            Command::Reset => Some(ComponentState::Idle),
            Command::GetStatus => None,
        }
    }

    /// JSON へ符号化する。
    pub fn to_json(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    /// JSON から復号する。未知のコマンドは既定値に落とさずエラーにする。
    pub fn from_json(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

impl std::fmt::Display for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Command::Configure(cfg) => write!(f, "Configure(run={})", cfg.run_number),
            Command::Arm => f.write_str("Arm"),
            Command::Start { run_number } => write!(f, "Start(run={run_number})"),
            Command::Stop => f.write_str("Stop"),
            Command::Reset => f.write_str("Reset"),
            Command::GetStatus => f.write_str("GetStatus"),
        }
    }
}

/// コンポーネント → controller の応答(SPEC §2.6)。
///
/// `run_number` と `metrics` は「値 or null」で常に鍵が出る(SPEC の形をそのまま守る)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandResponse {
    pub success: bool,
    pub state: ComponentState,
    pub message: String,
    pub run_number: Option<u32>,
    /// run_stop レコードの材料(frames/bytes/drops 等)。
    pub metrics: Option<serde_json::Value>,
}

impl CommandResponse {
    /// 成功応答。
    pub fn success(state: ComponentState, message: impl Into<String>) -> Self {
        Self {
            success: true,
            state,
            message: message.into(),
            run_number: None,
            metrics: None,
        }
    }

    /// 失敗応答。`state` には**変わらなかった現在の状態**を入れる。
    pub fn error(state: ComponentState, message: impl Into<String>) -> Self {
        Self {
            success: false,
            state,
            message: message.into(),
            run_number: None,
            metrics: None,
        }
    }

    /// run 番号を添える。
    pub fn with_run_number(mut self, run_number: u32) -> Self {
        self.run_number = Some(run_number);
        self
    }

    /// メトリクスを添える。
    pub fn with_metrics(mut self, metrics: serde_json::Value) -> Self {
        self.metrics = Some(metrics);
        self
    }

    /// JSON へ符号化する。
    pub fn to_json(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    /// JSON から復号する。
    pub fn from_json(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

// ---------------------------------------------------------------------
// REP コマンドタスク(SPEC §2.6)
// ---------------------------------------------------------------------

/// REP ソケットを bind し、JSON コマンドを 1 件ずつ `handler` に渡して応答を返す。
///
/// # 落ちないこと(delila-rs TODO 58 の教訓)
///
/// bind・受信・送信のどの失敗でも**タスクを終わらせず、壊れたソケットを捨てて再 bind** する。
/// 一過性のソケットエラーでコマンド経路が死ぬと、コンポーネントは走り続けているのに
/// 制御不能(kill しか手がない)になる。タスクを終わらせるのは `shutdown` だけ。
/// 失敗は握り潰さず `tracing` に出す(CLAUDE.md「silent failure を作らない」)。
/// subscriber の初期化は各 bin の仕事なので、ライブラリ側は記録するだけで出力先を決めない。
///
/// # 引数
///
/// - `address`: bind 先(例 `tcp://*:47101`)。テストでは `tcp://127.0.0.1:0` を使う。
/// - `component`: ログに出す名前。
/// - `shutdown`: 受信(または送信側の drop)でタスクを終了する。
/// - `bound_endpoint`: 最初の bind に成功した時点の実エンドポイントを 1 度だけ通知する。
///   `tcp://127.0.0.1:0`(動的ポート)を使うテスト用。production では `None` でよい。
/// - `handler`: コマンド 1 件を処理して応答を返す。状態はハンドラ側が持つ(`FnMut`)。
pub async fn run_command_task<H>(
    address: String,
    component: &'static str,
    mut shutdown: broadcast::Receiver<()>,
    mut bound_endpoint: Option<oneshot::Sender<String>>,
    mut handler: H,
) where
    H: FnMut(Command) -> CommandResponse,
{
    let context = tmq::Context::new();
    // コマンドが読めなかったときの応答に載せる状態。ハンドラの返す state で更新する。
    let mut last_state = ComponentState::Idle;

    'bind: loop {
        let socket = match tmq::reply(&context).bind(&address) {
            Ok(socket) => socket,
            Err(e) => {
                error!(component, %address, error = %e, "bind failed — retrying in 1 s");
                tokio::select! {
                    _ = shutdown.recv() => break 'bind,
                    _ = tokio::time::sleep(Duration::from_secs(1)) => continue 'bind,
                }
            }
        };

        if let Some(tx) = bound_endpoint.take() {
            match socket.get_socket().get_last_endpoint() {
                Ok(Ok(endpoint)) => {
                    let _ = tx.send(endpoint);
                }
                Ok(Err(raw)) => error!(component, "last_endpoint is not utf-8: {raw:?}"),
                Err(e) => error!(component, error = %e, "cannot read last_endpoint"),
            }
        }
        info!(component, %address, "command socket listening");

        let mut receiver = socket;
        loop {
            tokio::select! {
                biased;

                _ = shutdown.recv() => break 'bind,

                received = receiver.recv() => {
                    let (mut multipart, sender) = match received {
                        Ok(pair) => pair,
                        Err(e) => {
                            error!(component, error = %e, "command recv failed — rebinding");
                            continue 'bind;
                        }
                    };

                    let response = match multipart.pop_front() {
                        Some(frame) => match Command::from_json(&frame) {
                            Ok(cmd) => handler(cmd),
                            Err(e) => {
                                warn!(component, error = %e, "invalid command");
                                CommandResponse::error(last_state, format!("invalid command: {e}"))
                            }
                        },
                        None => {
                            warn!(component, "empty command message");
                            CommandResponse::error(last_state, "empty command message")
                        }
                    };
                    last_state = response.state;

                    let bytes = match response.to_json() {
                        Ok(bytes) => bytes,
                        Err(e) => {
                            error!(component, error = %e, "cannot encode response — rebinding");
                            continue 'bind;
                        }
                    };
                    match sender.send(vec![bytes].into()).await {
                        Ok(next) => receiver = next,
                        Err(e) => {
                            error!(component, error = %e, "command send failed — rebinding");
                            continue 'bind;
                        }
                    }
                }
            }
        }
    }

    info!(component, "command task stopped");
}

// ---------------------------------------------------------------------
// REQ クライアント(controller 側、016 で追加。SPEC §2.6「タイムアウト付き」)
// ---------------------------------------------------------------------

/// [`request`] の失敗。**どの段で失敗したかを型で残す**(呼び手が audit / ログに
/// そのまま書けるように — silent failure を作らない、CLAUDE.md)。
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("cannot create/connect REQ socket to {endpoint}: {source}")]
    Socket {
        endpoint: String,
        #[source]
        source: zmq::Error,
    },

    #[error("cannot encode command: {0}")]
    Encode(#[from] serde_json::Error),

    #[error("send to {endpoint} failed: {source}")]
    Send {
        endpoint: String,
        #[source]
        source: zmq::Error,
    },

    /// タイムアウト(`EAGAIN`)もここに来る。呼び手はこれを「不達」として扱う。
    #[error("no reply from {endpoint} within {timeout_ms} ms: {source}")]
    Recv {
        endpoint: String,
        timeout_ms: i32,
        #[source]
        source: zmq::Error,
    },

    #[error("reply from {endpoint} is not a CommandResponse: {source}")]
    Decode {
        endpoint: String,
        #[source]
        source: serde_json::Error,
    },
}

/// コマンド 1 件を REQ/REP で送り、応答を待つ(**同期・ブロッキング**)。
///
/// # なぜ毎回ソケットを作るか
///
/// REQ ソケットは send → recv の厳密な交替を強制する状態機械を持ち、**タイムアウトで
/// recv を諦めた時点でその状態機械が壊れる**(次の send が `EFSM` で失敗する)。
/// 使い捨てにすれば「1 回の往復 = 1 個のソケット」で状態を持ち越さず、再接続の
/// リトライロジックも要らない(KISS)。呼ばれるのは run シーケンスと status ポーリング
/// (高々数 Hz)だけで、ホットパスではない。
///
/// `timeout` は送受信の両方に効く。相手が居ない・遅い場合は [`ClientError::Recv`]
/// (`EAGAIN`)になり、**永久に待たない**。
pub fn request(
    context: &zmq::Context,
    endpoint: &str,
    command: &Command,
    timeout: Duration,
) -> Result<CommandResponse, ClientError> {
    let timeout_ms = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
    let socket_err = |source: zmq::Error| ClientError::Socket {
        endpoint: endpoint.to_string(),
        source,
    };

    let socket = context.socket(zmq::REQ).map_err(socket_err)?;
    socket.set_rcvtimeo(timeout_ms).map_err(socket_err)?;
    socket.set_sndtimeo(timeout_ms).map_err(socket_err)?;
    // 応答を待たずに捨てるとき、未送分を抱えたまま context が閉じないようにする。
    socket.set_linger(0).map_err(socket_err)?;
    socket.connect(endpoint).map_err(socket_err)?;

    let payload = command.to_json()?;
    socket
        .send(payload, 0)
        .map_err(|source| ClientError::Send {
            endpoint: endpoint.to_string(),
            source,
        })?;
    let reply = socket.recv_bytes(0).map_err(|source| ClientError::Recv {
        endpoint: endpoint.to_string(),
        timeout_ms,
        source,
    })?;
    CommandResponse::from_json(&reply).map_err(|source| ClientError::Decode {
        endpoint: endpoint.to_string(),
        source,
    })
}

// ---------------------------------------------------------------------
// テスト(仕様書 — 先に書く)
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    const ALL_STATES: [ComponentState; 5] = [
        ComponentState::Idle,
        ComponentState::Configured,
        ComponentState::Armed,
        ComponentState::Running,
        ComponentState::Error,
    ];

    /// 遷移表(SPEC §1.3 = delila-rs `common/command.rs` をそのまま採用)。
    /// 行 = from、列 = to、列順は [Idle, Configured, Armed, Running, Error]。
    ///
    ///   Idle       -Configure→ Configured
    ///   Configured -Arm→       Armed
    ///   Armed      -Start→     Running
    ///   Running    -Stop→      Configured
    ///   Configured/Armed/Running -Reset→ Idle、Error -Reset→ Idle
    ///   Configured/Armed/Running -(異常)→ Error
    const LEGAL: [[bool; 5]; 5] = [
        // to:      Idle,  Configured, Armed, Running, Error
        /* Idle */
        [false, true, false, false, false],
        /* Configured */ [true, false, true, false, true],
        /* Armed */ [true, false, false, true, true],
        /* Running */ [true, true, false, false, true],
        /* Error */ [true, false, false, false, false],
    ];

    #[test]
    fn every_state_pair_matches_the_transition_table() {
        for (i, from) in ALL_STATES.iter().enumerate() {
            for (j, to) in ALL_STATES.iter().enumerate() {
                assert_eq!(
                    from.can_transition_to(*to),
                    LEGAL[i][j],
                    "transition {from} -> {to}"
                );
            }
        }
    }

    #[test]
    fn no_state_transitions_to_itself() {
        for state in ALL_STATES {
            assert!(!state.can_transition_to(state), "{state} -> {state}");
        }
    }

    #[test]
    fn error_is_only_escapable_by_reset_to_idle() {
        for to in ALL_STATES {
            assert_eq!(
                ComponentState::Error.can_transition_to(to),
                to == ComponentState::Idle
            );
        }
    }

    #[test]
    fn commands_map_to_their_target_states() {
        let cfg = Command::Configure(RunConfig {
            run_number: 17,
            comment: "beam on".to_string(),
            config: json!({"workers": 4}),
        });
        assert_eq!(cfg.target_state(), Some(ComponentState::Configured));
        assert_eq!(Command::Arm.target_state(), Some(ComponentState::Armed));
        assert_eq!(
            Command::Start { run_number: 17 }.target_state(),
            Some(ComponentState::Running)
        );
        assert_eq!(
            Command::Stop.target_state(),
            Some(ComponentState::Configured)
        );
        assert_eq!(Command::Reset.target_state(), Some(ComponentState::Idle));
        // GetStatus は問い合わせのみ = 遷移しない
        assert_eq!(Command::GetStatus.target_state(), None);
    }

    /// 全順路 Idle → Configured → Armed → Running → Configured → Idle が
    /// コマンド列だけで通ること(echo デモの純ロジック版)。
    #[test]
    fn the_full_run_sequence_is_legal() {
        let sequence = [
            Command::Configure(RunConfig {
                run_number: 17,
                comment: String::new(),
                config: serde_json::Value::Null,
            }),
            Command::Arm,
            Command::Start { run_number: 17 },
            Command::Stop,
            Command::Reset,
        ];
        let mut state = ComponentState::Idle;
        for cmd in &sequence {
            let target = cmd.target_state().unwrap();
            assert!(state.can_transition_to(target), "{state} -> {target}");
            state = target;
        }
        assert_eq!(state, ComponentState::Idle);
    }

    // -----------------------------------------------------------------
    // JSON 表現(SPEC §2.6)
    // -----------------------------------------------------------------

    #[test]
    fn configure_serializes_as_a_tagged_object() {
        let cmd = Command::Configure(RunConfig {
            run_number: 17,
            comment: "beam on".to_string(),
            config: json!({"workers": 4}),
        });
        let v: serde_json::Value = serde_json::from_slice(&cmd.to_json().unwrap()).unwrap();
        assert_eq!(
            v,
            json!({"Configure": {"run_number": 17, "comment": "beam on", "config": {"workers": 4}}})
        );
    }

    #[test]
    fn unit_commands_serialize_as_bare_strings() {
        for (cmd, text) in [
            (Command::Arm, "\"Arm\""),
            (Command::Stop, "\"Stop\""),
            (Command::Reset, "\"Reset\""),
            (Command::GetStatus, "\"GetStatus\""),
        ] {
            assert_eq!(String::from_utf8(cmd.to_json().unwrap()).unwrap(), text);
        }
    }

    #[test]
    fn start_carries_the_run_number() {
        let v: serde_json::Value =
            serde_json::from_slice(&Command::Start { run_number: 42 }.to_json().unwrap()).unwrap();
        assert_eq!(v, json!({"Start": {"run_number": 42}}));
    }

    #[test]
    fn commands_roundtrip_through_json() {
        let commands = [
            Command::Configure(RunConfig {
                run_number: 3,
                comment: "cosmics".to_string(),
                config: json!({"decoder": {"workers": 2}}),
            }),
            Command::Arm,
            Command::Start { run_number: 3 },
            Command::Stop,
            Command::Reset,
            Command::GetStatus,
        ];
        for cmd in &commands {
            let bytes = cmd.to_json().unwrap();
            let back = Command::from_json(&bytes).unwrap();
            assert_eq!(format!("{back}"), format!("{cmd}"));
            assert_eq!(back.target_state(), cmd.target_state());
        }
    }

    /// `comment` / `config` は省略可(controller が最小コマンドを送れる)。
    #[test]
    fn configure_accepts_a_minimal_payload() {
        let cmd = Command::from_json(br#"{"Configure": {"run_number": 9}}"#).unwrap();
        match cmd {
            Command::Configure(cfg) => {
                assert_eq!(cfg.run_number, 9);
                assert_eq!(cfg.comment, "");
                assert_eq!(cfg.config, serde_json::Value::Null);
            }
            other => panic!("expected Configure, got {other}"),
        }
    }

    #[test]
    fn malformed_json_is_an_error_not_a_default_command() {
        assert!(Command::from_json(b"not json at all").is_err());
        assert!(Command::from_json(br#"{"Frobnicate": {}}"#).is_err());
    }

    /// SPEC §2.6 のレスポンス形。`run_number` / `metrics` は「N|null」= 常に鍵が出る。
    #[test]
    fn command_response_has_the_spec_shape() {
        let resp = CommandResponse::success(ComponentState::Running, "started");
        let v: serde_json::Value = serde_json::from_slice(&resp.to_json().unwrap()).unwrap();
        assert_eq!(
            v,
            json!({
                "success": true,
                "state": "Running",
                "message": "started",
                "run_number": null,
                "metrics": null,
            })
        );
    }

    #[test]
    fn command_response_carries_run_number_and_metrics() {
        let resp = CommandResponse::success(ComponentState::Configured, "stopped")
            .with_run_number(17)
            .with_metrics(json!({"frames": 108, "bytes": 15_040_512u64, "drops": 0}));
        let v: serde_json::Value = serde_json::from_slice(&resp.to_json().unwrap()).unwrap();
        assert_eq!(v["run_number"], json!(17));
        assert_eq!(v["metrics"]["frames"], json!(108));
        assert!(v["success"].as_bool().unwrap());
    }

    #[test]
    fn error_response_reports_the_unchanged_state() {
        let resp = CommandResponse::error(ComponentState::Configured, "illegal transition");
        let v: serde_json::Value = serde_json::from_slice(&resp.to_json().unwrap()).unwrap();
        assert_eq!(v["success"], json!(false));
        assert_eq!(v["state"], json!("Configured"));
        let back = CommandResponse::from_json(&resp.to_json().unwrap()).unwrap();
        assert!(!back.success);
        assert_eq!(back.state, ComponentState::Configured);
    }

    #[test]
    fn component_state_displays_as_its_json_name() {
        for state in ALL_STATES {
            let text = serde_json::to_string(&state).unwrap();
            assert_eq!(text, format!("\"{state}\""));
        }
    }

    // -----------------------------------------------------------------
    // REQ クライアント(016 で追加)
    // -----------------------------------------------------------------

    /// 実 ZMQ(port 0)で `run_command_task` と 1 往復し、応答がそのまま戻ること。
    #[tokio::test]
    async fn request_round_trips_against_a_real_command_task() {
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
        let (bound_tx, bound_rx) = oneshot::channel();
        let task = tokio::spawn(run_command_task(
            "tcp://127.0.0.1:0".to_string(),
            "test",
            shutdown_rx,
            Some(bound_tx),
            |cmd| match cmd {
                // 非対称な応答(取り違え検出用)。
                Command::Arm => CommandResponse::success(ComponentState::Armed, "armed")
                    .with_run_number(57)
                    .with_metrics(json!({"router_port": 46123})),
                other => CommandResponse::error(ComponentState::Idle, format!("no: {other}")),
            },
        ));
        let endpoint = bound_rx.await.unwrap();

        // `request` はブロッキング(controller も spawn_blocking から呼ぶ)。ここで直に
        // 呼ぶと REP タスクと同じ executor スレッドを塞いで自分で自分を待つことになる。
        let (armed, refused) = tokio::task::spawn_blocking(move || {
            let context = zmq::Context::new();
            let timeout = Duration::from_millis(2000);
            let armed = request(&context, &endpoint, &Command::Arm, timeout);
            let refused = request(&context, &endpoint, &Command::Stop, timeout);
            (armed, refused)
        })
        .await
        .unwrap();

        let armed = armed.unwrap();
        assert!(armed.success);
        assert_eq!(armed.state, ComponentState::Armed);
        assert_eq!(armed.run_number, Some(57));
        assert_eq!(armed.metrics.unwrap()["router_port"], json!(46123));

        // 断られた応答も `Ok`(不達との区別は呼び手の仕事)。
        assert!(!refused.unwrap().success);

        let _ = shutdown_tx.send(());
        let _ = task.await;
    }

    /// 相手が居なければ**永久に待たず** `Recv`(タイムアウト)で戻る(SPEC §2.6)。
    #[tokio::test]
    async fn request_times_out_instead_of_hanging_when_nobody_answers() {
        // 誰も bind していないアドレス(確保して即座に手放す)。
        let dead = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            drop(listener);
            format!("tcp://{addr}")
        };
        let context = zmq::Context::new();
        let started = std::time::Instant::now();
        let error = request(
            &context,
            &dead,
            &Command::GetStatus,
            Duration::from_millis(200),
        )
        .unwrap_err();
        assert!(matches!(error, ClientError::Recv { .. }), "{error}");
        assert!(started.elapsed() < Duration::from_secs(5), "戻りが遅すぎる");
    }
}
