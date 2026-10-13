#![allow(clippy::unwrap_used)]
use super::*;
use crate::controller::test_support::*;

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
        .position(|(i, c)| i > forced_stop && c.component == "decoder" && c.action == "GetStatus")
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
