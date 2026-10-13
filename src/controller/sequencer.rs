use super::*;

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
    pub(super) fn to_json(&self) -> Value {
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
    pub(super) fn router_link(
        &self,
        cobo_id: u32,
        response: &CommandResponse,
    ) -> Result<RouterLink, String> {
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
    pub(super) fn router_ip(&self, cobo_id: u32, bind_address: Option<&str>) -> String {
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

    pub(super) fn configure_request(&self, links: &[RouterLink]) -> Value {
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
    pub(super) fn collect_status(&mut self, notes: &mut Vec<String>) -> Vec<ComponentStatus> {
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
    pub(super) fn apply(
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
pub(super) fn previous_run_number(reported: &[(&str, u32)]) -> Result<u32, String> {
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
pub(super) fn push_once(notes: &mut Vec<String>, note: String) {
    if !notes.contains(&note) {
        notes.push(note);
    }
}

#[cfg(test)]
mod tests_start;
#[cfg(test)]
mod tests_stop;
#[cfg(test)]
mod tests_walk_back;
