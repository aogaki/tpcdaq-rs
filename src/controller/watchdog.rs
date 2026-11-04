//! run 中の監視と保存系 Error の自動異常停止(SPEC §1.3「run 中の監視」v1.53 / §1.4-3、TODO/188)。
//!
//! # なぜ要るか
//!
//! receiver の内部キューの溢れ(= graw の喪失)は、receiver が**コマンドを受けたとき**に Error へ
//! 遷移する(`receiver.rs` `latch_overload`)。controller が run 中に誰にも問い合わせなければ、溢れは
//! 誰かが画面を開くまで表に出ない(run 197: 5.5 分放置、graw 19,855 フレーム喪失)。
//!
//! # 形
//!
//! 専用 OS スレッド 1 本([`spawn_watchdog_thread`])が [`WATCHDOG_PERIOD`] ごとに phase を見て、
//! `Running` / `Stopping` なら全コンポーネントへ `GetStatus`(`status_timeout`。REQ は呼び出しごとに
//! 作るので制御コマンドとソケットは衝突しない)。結果は `Controller` の最新値に置く。
//! **phase == Running のときだけ**、保存系(receiver / graw-writer)の Error を引き金に
//! `Running → Stopping` を CAS する([`Controller::begin_stopping`] = `POST /api/run/stop` と同じ
//! ロック区間。負けたら何もしない)。勝てば停止の本体([`stop_run_blocking`] を共用、operator
//! `watchdog`、reason `abort:overload:<component>`)を**別スレッド**で走らせる —— 停止の待ち
//! (分単位になりうる、204)の間も監視は止めない(Stopping 中は観測のみ)。
//!
//! 引き金にしないもの: decoder の Error(§1.4-7 —— 派生系は画面に出すだけ)、不達(1 回でも連続でも
//! —— REP は逐次で一時不達はふつうに起き、不達は停止の中止判定 `abort:component-error` の既存の
//! 経路に任せる)、Running 以外の相の Error。

use super::*;

/// 監視の周期(SPEC §1.3 の既定 2 s。TOML キーは作らない — 188 §1-C)。
pub const WATCHDOG_PERIOD: Duration = Duration::from_secs(2);

/// 停止合図を見る目覚まし(問い合わせの合間の眠りの刻み)。
const WATCHDOG_WAKE: Duration = Duration::from_millis(100);

/// 自動異常停止の audit `run/stop` の `operator`(SPEC §1.3)。
const WATCHDOG_OPERATOR: &str = "watchdog";

/// 直近の問い合わせの結果(`Controller` の共有の最新値)。将来 UI の 2 s 更新に使える ——
/// 188 では書くだけ(`GET /api/status` は今どおり自分で `GetStatus` を打つ)。
#[derive(Debug, Clone)]
pub struct WatchdogSnapshot {
    /// 問い合わせを終えた時刻。
    pub polled_at: Instant,
    /// 全コンポーネントの state / reachable / metrics(`components` と同じ順)。
    pub statuses: Vec<ComponentStatus>,
}

/// 自動異常停止の引き金(audit `run/stop` の `params.trigger`)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OverloadTrigger {
    /// Error を報告した保存系のコンポーネント(`run_stop.reason = "abort:overload:<これ>"`)。
    pub(super) component: String,
    /// その `overflow_frames`(receiver だけが持つ。graw-writer は `None` = null)。
    pub(super) overflow_frames: Option<u64>,
}

impl OverloadTrigger {
    /// 停止の種類(`run_stop.reason = "abort:overload:<component>"`)。
    pub(super) fn stop_mode(&self) -> StopMode {
        StopMode::Abort(format!("overload:{}", self.component))
    }

    /// audit `run/stop.params.trigger` = `{component, state, overflow_frames}`。
    pub(super) fn to_json(&self) -> Value {
        json!({
            "component": self.component,
            "state": ComponentState::Error.to_string(),
            "overflow_frames": self.overflow_frames,
        })
    }
}

/// 1 回の問い合わせの結果。
#[derive(Debug)]
pub(super) struct Poll {
    pub(super) statuses: Vec<ComponentStatus>,
    /// 自動異常停止の引き金(phase == Running のときだけ立ちうる)。
    pub(super) trigger: Option<OverloadTrigger>,
}

/// 1 回の問い合わせ(IO は [`Transport`] 越しだけ —— 相ごとの振る舞いをモックで照合できる)。
///
/// * `Idle` / `Starting` … 何も問い合わせない(`None`)。
/// * `Stopping` … 全コンポーネントへ `GetStatus`(観測のみ。引き金は立てない)。
/// * `Running` … 全コンポーネントへ `GetStatus`、保存系の Error があれば引き金。
pub(super) fn poll_once(
    params: &ControllerParams,
    transport: &mut dyn Transport,
    phase: Phase,
) -> Option<Poll> {
    if !matches!(phase, Phase::Running | Phase::Stopping) {
        return None;
    }
    // 不達の note はここでは捨てる —— 変化は [`log_changes`] が 1 行ずつ出す。
    let mut notes = Vec::new();
    let statuses = Sequencer::new(params, transport).collect_status(&mut notes);
    let trigger = if phase == Phase::Running {
        overload_trigger(&statuses)
    } else {
        None
    };
    Some(Poll { statuses, trigger })
}

/// 保存系(receiver / graw-writer)で Error を報告したもののうち `components` の順で最初の 1 つ
/// (graw-writer が receiver より先 —— ディスクで止まった graw-writer が receiver の溢れの原因)。
/// decoder の Error と不達(`state == None`)は引き金にしない。
fn overload_trigger(statuses: &[ComponentStatus]) -> Option<OverloadTrigger> {
    statuses
        .iter()
        .filter(|s| {
            matches!(
                s.kind,
                ComponentKind::Receiver { .. } | ComponentKind::GrawWriter
            )
        })
        .find(|s| s.state == Some(ComponentState::Error))
        .map(|s| OverloadTrigger {
            component: s.name.clone(),
            overflow_frames: s.metric_u64("overflow_frames"),
        })
}

/// 監視スレッドを起こす。`stop` が立ったら(controller の終了)走っている自動停止を待って終わる
/// (スレッドを残さない)。
pub(super) fn spawn_watchdog_thread(
    controller: Arc<Controller>,
    stop: Arc<AtomicBool>,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("watchdog".to_string())
        .spawn(move || watch(&controller, &stop))
}

fn watch(controller: &Arc<Controller>, stop: &AtomicBool) {
    info!(
        period_ms = WATCHDOG_PERIOD.as_millis() as u64,
        "watchdog started (SPEC §1.3)"
    );
    // 走っている自動停止(高々 1 本 —— CAS が 2 本目を作らせない)。
    let mut stopping: Option<std::thread::JoinHandle<()>> = None;
    let mut next_poll = Instant::now() + WATCHDOG_PERIOD;
    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        if now < next_poll {
            std::thread::sleep((next_poll - now).min(WATCHDOG_WAKE));
            continue;
        }
        // 次の問い合わせはこの問い合わせの開始から 1 周期後(長引いたらすぐ次)。
        next_poll = now + WATCHDOG_PERIOD;
        tick(controller, &mut stopping);
    }
    if let Some(handle) = stopping.take() {
        join_stop(handle);
    }
    info!("watchdog stopped");
}

/// 1 周期分: phase を見て問い合わせ、最新値を置き、引き金が立てば自動停止を始める。
fn tick(controller: &Arc<Controller>, stopping: &mut Option<std::thread::JoinHandle<()>>) {
    let (phase, run) = {
        let run_state = lock(&controller.run);
        (
            run_state.phase,
            run_state.active.as_ref().map(|active| active.run),
        )
    };
    let mut transport = controller
        .transport
        .with_command_timeout(controller.params.status_timeout);
    let Some(Poll { statuses, trigger }) = poll_once(&controller.params, &mut transport, phase)
    else {
        return;
    };
    {
        let mut latest = lock(&controller.watchdog);
        log_changes(phase, latest.as_ref(), &statuses);
        *latest = Some(WatchdogSnapshot {
            polled_at: Instant::now(),
            statuses,
        });
    }
    let Some(trigger) = trigger else {
        return;
    };

    // CAS: 問い合わせた時の run がまだ Running のときだけ(手動の停止が先なら何もしない)。
    let active = match controller.begin_stopping(run) {
        Ok(active) => active,
        Err(now) => {
            info!(
                component = %trigger.component,
                phase = %now,
                "watchdog: a storage component is in Error but the run is no longer Running — \
                 nothing to do"
            );
            return;
        }
    };
    warn!(
        component = %trigger.component,
        overflow_frames = ?trigger.overflow_frames,
        run = ?run,
        "watchdog: a storage component reports Error while Running — aborting the run \
         (SPEC §1.4-3)"
    );
    // 前の自動停止は phase を Idle に戻してから終わる(この run が始まれたなら終わっている)。
    if let Some(previous) = stopping.take() {
        join_stop(previous);
    }
    let spawned = {
        let controller = Arc::clone(controller);
        let (active, trigger) = (active.clone(), trigger.clone());
        std::thread::Builder::new()
            .name("watchdog-stop".to_string())
            .spawn(move || stop_for_overload(&controller, active, &trigger))
    };
    match spawned {
        Ok(handle) => *stopping = Some(handle),
        Err(e) => {
            error!(
                error = %e,
                "cannot spawn the watchdog stop thread — stopping inline (no polling until the \
                 run is closed)"
            );
            stop_for_overload(controller, active, &trigger);
        }
    }
}

/// 自動異常停止の本体: 停止は REST と同じ [`stop_run_blocking`](operator `watchdog`)。
/// 終わったら(panic しても)phase を Idle へ戻す。
fn stop_for_overload(
    controller: &Controller,
    active: Option<ActiveRun>,
    trigger: &OverloadTrigger,
) {
    let _back_to_idle = BackToIdle(controller);
    stop_run_blocking(controller, WATCHDOG_OPERATOR, active, Some(trigger));
}

/// drop で phase を Idle に戻す —— 停止の本体が panic しても Stopping のまま固めない
/// (REST の停止が `spawn_blocking` の `JoinError` で戻すのと同じ守り)。
struct BackToIdle<'a>(&'a Controller);

impl Drop for BackToIdle<'_> {
    fn drop(&mut self) {
        self.0.end_stopping();
    }
}

fn join_stop(handle: std::thread::JoinHandle<()>) {
    if handle.join().is_err() {
        error!("watchdog stop thread panicked — the phase was returned to Idle");
    }
}

/// 前回の問い合わせから到達性か Error が変わったコンポーネントを 1 行ずつ出す(silent にしない ——
/// 引き金にしない変化 = 不達・decoder の Error もここで見える)。前回が無ければ「健全」から比べる。
///
/// **Stopping 中の不達は info**(Running 中は warn): 停止は decoder の Reset の join(≤ 2.1 s、
/// SPEC §1.3 v1.40)で REP が答えず、`status_timeout` では 1 回不達に見えるのが常態 —— 停止の
/// たびに warn を出すと雑音になる(188 レビュー指摘 3)。
fn log_changes(phase: Phase, previous: Option<&WatchdogSnapshot>, statuses: &[ComponentStatus]) {
    let health = |s: &ComponentStatus| (s.reachable, s.state == Some(ComponentState::Error));
    for status in statuses {
        let before = previous
            .and_then(|p| p.statuses.iter().find(|s| s.name == status.name))
            .map_or((true, false), health);
        let now = health(status);
        if now == before {
            continue;
        }
        match now {
            (false, _) if phase == Phase::Stopping => info!(
                component = %status.name,
                error = ?status.error,
                "watchdog: unreachable while stopping (expected during a Reset join — not a \
                 trigger, SPEC §1.3)"
            ),
            (false, _) => warn!(
                component = %status.name,
                error = ?status.error,
                "watchdog: unreachable (not a trigger — SPEC §1.3)"
            ),
            (true, true) => warn!(component = %status.name, "watchdog: reports Error"),
            (true, false) => info!(
                component = %status.name,
                "watchdog: reachable and not in Error"
            ),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::controller::test_support::*;

    /// **188 テスト 5**: 自動異常停止は phase == Running のときだけ(SPEC §1.3「run 中の監視」)。
    /// Idle / Starting では問い合わせすらせず、Stopping では全コンポーネントへ問い合わせる
    /// (= **観測は続く**)が引き金にしない。4 相とも receiver0 が Error(溢れ 17 フレーム)を
    /// 報告するモックで、相ごとに「GetStatus を撃った相手」と「引き金」を照合する。
    #[test]
    fn the_watchdog_does_not_fire_outside_running() {
        let params = params_with(&[0]);
        for (phase, observes, fires) in [
            (Phase::Idle, false, false),
            (Phase::Starting, false, false),
            (Phase::Stopping, true, false),
            (Phase::Running, true, true),
        ] {
            let mut mock =
                MockTransport::new(&params).component_in("receiver0", ComponentState::Error);
            mock.set_metric("receiver0", "overflow_frames", json!(17));

            let poll = poll_once(&params, &mut mock, phase);

            let asked: Vec<(&str, &str)> = mock
                .calls
                .iter()
                .map(|c| (c.component.as_str(), c.action.as_str()))
                .collect();
            if observes {
                assert_eq!(
                    asked,
                    [
                        ("graw-writer", "GetStatus"),
                        ("decoder", "GetStatus"),
                        ("receiver0", "GetStatus"),
                    ],
                    "{phase}: 全コンポーネントへ GetStatus だけ"
                );
                let poll = poll.as_ref().unwrap_or_else(|| panic!("{phase}: 観測する"));
                assert_eq!(poll.statuses.len(), 3, "{phase}");
                assert_eq!(
                    poll.statuses[2].state,
                    Some(ComponentState::Error),
                    "{phase}"
                );
            } else {
                assert!(asked.is_empty(), "{phase}: 問い合わせない: {asked:?}");
                assert!(poll.is_none(), "{phase}");
            }
            let trigger = poll.and_then(|poll| poll.trigger);
            if fires {
                assert_eq!(
                    trigger,
                    Some(OverloadTrigger {
                        component: "receiver0".to_string(),
                        overflow_frames: Some(17),
                    }),
                    "{phase}"
                );
            } else {
                assert_eq!(trigger, None, "{phase}: 引き金にしない");
            }
        }
    }
}
