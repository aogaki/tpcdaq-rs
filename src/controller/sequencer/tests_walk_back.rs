#![allow(clippy::unwrap_used)]
use super::*;
use crate::controller::test_support::*;

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
            "eos: not awaited — the chain is broken (not every component is Running)".to_string(),
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
            c.action == "GetStatus" && (c.component == "decoder" || c.component == "graw-writer")
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
            "eos: not awaited — the chain is broken (not every component is Running)".to_string(),
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
                acquisition_restarts: None,
                frames_before_restart: None,
                frames_type1: None,
                frames_type2: None,
                compact_size_mismatch: None,
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
