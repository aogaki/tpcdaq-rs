#![allow(clippy::unwrap_used)]
use super::*;
use crate::controller::test_support::*;

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
    let old: EccReply = serde_json::from_str(r#"{"ok":true,"state":"Idle","error":""}"#).unwrap();
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
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ecc_transitions.txt");
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
