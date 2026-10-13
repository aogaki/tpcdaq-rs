#![allow(clippy::unwrap_used)]
use super::*;
use std::collections::BTreeSet;

// -----------------------------------------------------------------
// モック Transport(コマンド列を記録する)
// -----------------------------------------------------------------

/// 1 回の外向き操作の記録。`component` はコンポーネント名 or `"ecc"`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Call {
    pub(super) component: String,
    pub(super) action: String,
}

impl Call {
    pub(super) fn new(component: &str, action: &str) -> Self {
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
pub(super) struct MockTransport {
    pub(super) calls: Vec<Call>,
    pub(super) names: BTreeMap<String, String>, // endpoint → name
    pub(super) fail: Vec<(String, String)>,
    /// コンポーネント名 → 残り「Arm を断る回数」(034: bind 解放待ちの模擬)。
    pub(super) refuse_arm: BTreeMap<String, u32>,
    pub(super) metrics: BTreeMap<String, Value>,
    /// ECC が受けた**リクエスト本文**そのまま(action 名だけでは `config_id` を見られない)。
    pub(super) ecc_requests: Vec<Value>,
    pub(super) ecc_fail: Vec<String>,
    /// この ECC が今いる状態(036)。[`ecc_transition`] で**実 ECC と同じように**動く。
    pub(super) ecc_state: String,
    /// この ECC が抱えている error フラグ(043)。**state とは別の軸**。
    /// clear 規則も実機どおり: 遷移が渡ったときだけ `NO_ERR` に戻り、遷移が無い
    /// (`Ignored`)ときは触られない(`tools/ecc_bridge/ecc_core.hpp` の規則ブロック、
    /// 出典 `GetBench/src/get/rc/BackEnd.cpp:249-290` / `:1146-1149`)。
    pub(super) ecc_error: String,
    /// 「受けたのに無音で何もしない」action(036)。実 ECC の未定義遷移は例外も出さず
    /// `Ignored`(`StateMachine/src/dhsm/Engine.cpp:334-352`)なので、歩き戻しが
    /// 空振りする経路を**わざと**作れるようにしておく。
    pub(super) ecc_ignores: Vec<String>,
    /// EOS 完了を「N 回目の GetStatus 以降」にする(時間経過の模擬)。
    /// **数えるのは decoder / graw-writer への GetStatus だけ**(receiver への
    /// 静止観測は EOS の進み方とは無関係なので数に入れない — 033-E)。
    pub(super) eos_after_polls: Option<usize>,
    pub(super) eos_polls: usize,
    /// receiver 名(静止観測の相手を kind で判別するため)。
    pub(super) receivers: BTreeSet<String>,
    /// receiver 名 → 受信バイト数の進み方(033-E)。GetStatus 1 回につきこの分だけ増える。
    /// 未登録 = `0` = 静止(実機の stop 後の姿)。
    pub(super) bytes_steps: BTreeMap<String, u64>,
    /// receiver 名 → GetStatus 回数(`bytes` の値を作るのに使う)。
    pub(super) receiver_polls: BTreeMap<String, u64>,
    /// この名前のコンポーネントは**不達**(REQ タイムアウト相当の `Err`)。
    pub(super) unreachable: Vec<String>,
    /// コンポーネント名 → 今の状態(113)。未登録 = `Idle`。断られなかった状態遷移
    /// コマンドはその行き先へ動かす(合法性は見ない — それは統合テストの本物の状態機械の役)。
    pub(super) states: BTreeMap<String, ComponentState>,
    /// コンポーネント名 → 応答に載せる `run_number`(115)。未登録 = `null`
    /// (実コンポーネントは run を持っていないとき鍵を null にする)。
    pub(super) run_numbers: BTreeMap<String, u32>,
    pub(super) slept: Duration,
}

impl MockTransport {
    pub(super) fn new(params: &ControllerParams) -> Self {
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
    pub(super) fn component_in(mut self, component: &str, state: ComponentState) -> Self {
        self.states.insert(component.to_string(), state);
        self
    }

    /// 115: このコンポーネントの応答に `run_number = run` を載せる(前の run の番号の出典)。
    pub(super) fn reports_run(mut self, component: &str, run: u32) -> Self {
        self.run_numbers.insert(component.to_string(), run);
        self
    }

    /// EOS は最初から流れ切っている(正常停止で強制 EOS に落ちない)。
    pub(super) fn with_eos_closed(mut self) -> Self {
        self.eos_after_polls = Some(0);
        self
    }

    /// 033-E: 全 receiver がまだ受信し続けている(GetStatus 1 回につき `step` バイト増える)。
    /// 静止しないので第一段はハード上限 `eos_timeout` まで待つ。
    pub(super) fn receivers_still_receiving(mut self, step: u64) -> Self {
        for name in self.receivers.iter() {
            self.bytes_steps.insert(name.clone(), step);
        }
        self
    }

    /// 033-E: **この receiver だけ**受信を続けている(他は静止)。
    pub(super) fn receiver_still_receiving(mut self, component: &str, step: u64) -> Self {
        self.bytes_steps.insert(component.to_string(), step);
        self
    }

    /// 033-E: この receiver の metrics から `bytes` を落とす(静止を判定する材料が無い形)。
    pub(super) fn without_receiver_bytes(mut self, component: &str) -> Self {
        if let Some(value) = self.metrics.get_mut(component) {
            if let Some(map) = value.as_object_mut() {
                map.remove("bytes");
            }
        }
        self
    }

    /// このコンポーネントは**不達**(接続拒否も REQ タイムアウトも controller から見れば
    /// 同じ `Err` —— 033 論点 3)。
    pub(super) fn unreachable(mut self, component: &str) -> Self {
        self.unreachable.push(component.to_string());
        self
    }

    pub(super) fn fail_on(mut self, component: &str, command: &str) -> Self {
        self.fail.push((component.to_string(), command.to_string()));
        self
    }

    /// 034: 最初の `times` 回だけ `Arm` を断る(libzmq の close 非同期による
    /// `Address already in use` が数十 ms で解ける様子の模擬)。
    pub(super) fn refuse_arm_times(mut self, component: &str, times: u32) -> Self {
        self.refuse_arm.insert(component.to_string(), times);
        self
    }

    pub(super) fn fail_ecc(mut self, action: &str) -> Self {
        self.ecc_fail.push(action.to_string());
        self
    }

    /// 036: ECC がこの状態に居るところから始める(前の run の `ecc stop` 後は `Ready`)。
    pub(super) fn ecc_in(mut self, state: &str) -> Self {
        self.ecc_state = state.to_string();
        self
    }

    /// 036: この action を**受けるが無音で何もしない** ECC にする(実 ECC の `Ignored`)。
    pub(super) fn ecc_ignores(mut self, action: &str) -> Self {
        self.ecc_ignores.push(action.to_string());
        self
    }

    /// 043: ECC が既に error フラグを抱えているところから始める
    /// (例 041 D-1 の「describe に失敗した直後」= `Idle` + `WHEN_DESCRIBE`)。
    pub(super) fn ecc_error_flag(mut self, flag: &str) -> Self {
        self.ecc_error = flag.to_string();
        self
    }

    pub(super) fn name_of(&self, endpoint: &str) -> String {
        self.names
            .get(endpoint)
            .cloned()
            .unwrap_or_else(|| endpoint.to_string())
    }

    /// あるコンポーネントが受けた**状態を変えるコマンド**の列(`GetStatus` は
    /// 観測であって操作ではないので除く)。
    pub(super) fn actions(&self, component: &str) -> Vec<String> {
        self.calls
            .iter()
            .filter(|c| c.component == component && c.action != "GetStatus")
            .map(|c| c.action.clone())
            .collect()
    }

    /// GetStatus を除いた呼び出し列(順序照合を読みやすくするため)。
    pub(super) fn commands(&self) -> Vec<Call> {
        self.calls
            .iter()
            .filter(|c| c.action != "GetStatus")
            .cloned()
            .collect()
    }

    pub(super) fn index_of(&self, component: &str, action: &str) -> Option<usize> {
        self.calls
            .iter()
            .position(|c| c.component == component && c.action == action)
    }

    /// ECC が受けたリクエストのうち、その action の 1 通目(042: 相ごとの `config_id`)。
    pub(super) fn ecc_request(&self, action: &str) -> Option<&Value> {
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
pub(super) fn ecc_transition(state: &str, action: &str) -> Option<&'static str> {
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
pub(super) fn action_name(command: &Command) -> String {
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
    fn command(&mut self, endpoint: &str, command: &Command) -> Result<CommandResponse, String> {
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

pub(super) fn params_with(cobo_ids: &[u32]) -> ControllerParams {
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

pub(super) fn test_controller(tag: &str) -> Controller {
    let mut params = params_with(&[0]);
    params.output_root =
        std::env::temp_dir().join(format!("tpcdaq-controller-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&params.output_root);
    Controller::new(params).unwrap()
}
