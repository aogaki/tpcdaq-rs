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

/// **中止経路**(SPEC §1.3 v1.6): EOS で閉じてから畳む。**run がクローズする前に
/// decoder を Reset しない**。run_stop は ok=false / reason="abort:..."。
///
/// 185(SPEC §1.3 v1.53 ①)で前提を改訂: 中止も第一段(受信静止)を通る —— 「待たずに即・
/// 強制 EOS」は撤去した。ここは受信が止まっている(静止 600 ms)→ 強制 EOS → 第二段で
/// 閉じる形(`eos_after_polls = Some(4)` = 第一段の graw-writer 判定 4 回の後に閉じる。
/// 手計算は `stop_forces_eos_as_soon_as_the_receivers_go_quiet` と同じ)。
#[test]
fn abort_closes_with_eos_before_resetting_the_decoder() {
    let params = params_with(&[0]);
    let mut mock = MockTransport::new(&params);
    mock.eos_after_polls = Some(4);
    let report =
        Sequencer::new(&params, &mut mock).stop_run(StopMode::Abort("overflow".to_string()));

    assert!(!report.ok);
    assert_eq!(report.reason, "abort:overflow");
    assert!(report.forced_eos, "静止の後に強制 EOS(v1.53)");

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
// 185(SPEC §1.3 v1.53): 中止も第一段を通る / 第二段は「下流が進んでいる限り待つ」
// -----------------------------------------------------------------

/// **185 テスト 3 — 中止でも第一段(受信静止)を通る**。receiver がまだ受信している間は
/// 強制 EOS(receiver `Stop`)を撃たない —— 即座に撃つと `receiver::drain_task` が畳まれ、
/// カーネル受信バッファの未読分(= graw)が数えられずに消える(SPEC §1.3 v1.53 ①)。
///
/// 手計算(eos_poll 200 ms、eos_timeout 5 s、受信は止まらない): 第一段は静止しないので
/// ハード上限 = 判定 26 回(即時 1 + 200 ms × 25)= 5 000 ms。receiver0 は Stop の前に
/// 26 回問い合わされる。`eos_after_polls = Some(26)` = 強制 EOS 直後の判定で閉じる。
#[test]
fn an_abort_waits_for_the_receivers_to_go_quiet_before_forcing_eos() {
    let params = params_with(&[0]);
    let mut mock = MockTransport::new(&params).receivers_still_receiving(4096);
    mock.eos_after_polls = Some(26);
    let report = Sequencer::new(&params, &mut mock)
        .stop_run(StopMode::Abort("overload:receiver0".to_string()));

    assert_eq!(report.reason, "abort:overload:receiver0");
    assert!(!report.ok);
    let forced_stop = mock.index_of("receiver0", "Stop").unwrap();
    let polls_before_stop = mock.calls[..forced_stop]
        .iter()
        .filter(|c| c.component == "receiver0" && c.action == "GetStatus")
        .count();
    assert_eq!(
        polls_before_stop, 26,
        "中止で受信静止を確かめずに強制 EOS を撃っている: {:?}",
        mock.calls
    );
    assert_eq!(mock.slept, params.eos_timeout);
    assert!(report.forced_eos);
    assert!(report.eos_closed, "強制 EOS の後は閉じる");
}

/// **185 テスト 1(前半)— 強制 EOS の後、下流が書き出しを続けている間は待つ**。
/// decoder の `batches_out` が動き続ける間は `eos_timeout`(5 s)を超えて待ち、動かなく
/// なってから `eos_timeout` のあいだ静かなら打ち切る(上限のノブは無い)。
///
/// 手計算(eos_poll 200 ms、eos_timeout 5 s、EOS は来ない):
/// - 第一段: 受信は止まっている → 静止 600 ms で強制 EOS。
/// - 第二段: 観測サンプルは slept = 600, 800, …。`batches_out` は slept < 9 000 の
///   GetStatus で +1 なので 8 800 のサンプルまで動き、9 000 から不変。不変が 200 ms ずつ
///   積もって 8 800 + 5 000 = **13 800 ms** で打ち切り(第二段の待ちは 13 200 ms、最後に
///   動いたのは第二段の 8 200 ms 時点 = 打ち切りの 5.0 s 前)。
#[test]
fn after_the_forced_eos_the_stop_waits_as_long_as_the_decoder_keeps_sending() {
    let params = params_with(&[0]);
    let mut mock = MockTransport::new(&params).counter_moves_until(
        "decoder",
        "batches_out",
        Duration::from_millis(9_000),
    );
    let report = Sequencer::new(&params, &mut mock).stop_run(StopMode::Normal);

    assert!(report.forced_eos);
    assert!(!report.eos_closed);
    assert_eq!(report.reason, "error:eos-timeout");
    assert_eq!(
        mock.slept,
        Duration::from_millis(13_800),
        "動いている間は待ち、止まってから eos_timeout で打ち切る"
    );
    // 文言は今の流儀(`EOS did not propagate within <eos_timeout> ms` —— e2e の
    // `tests/p3_error_paths.rs` F1 もこの頭で照合する)に、待った総時間と最後に動いた時刻を足す。
    assert!(
        report.notes.iter().any(|n| n
            == "EOS did not propagate within 5000 ms of no downstream progress after the \
                forced EOS — waited 13200 ms; the observed counters last moved 5.0 s ago"),
        "打ち切りの note に最後に動いた時刻が無い: {:?}",
        report.notes
    );
}

/// **185 テスト 1(後半)— 途中で EOS が完了すれば即 true**。decoder はまだ動いて
/// いる(60 s まで)が、EOS は仮想時刻 8 000 ms で完了する → その判定で返る。
///
/// 手計算: 第一段 600 ms(静止)→ 強制 EOS → 第二段は slept = 600, 800, … の判定で、
/// 8 000 ms の判定で完了 → 眠らずに返る。合計 **8 000 ms**(> eos_timeout = 5 000 ms ——
/// 旧実装なら 5 600 ms で打ち切っていた)。
#[test]
fn a_forced_eos_that_completes_while_the_decoder_still_sends_returns_at_once() {
    let params = params_with(&[0]);
    let mut mock = MockTransport::new(&params).counter_moves_until(
        "decoder",
        "batches_out",
        Duration::from_secs(60),
    );
    mock.eos_after_slept = Some(Duration::from_millis(8_000));
    let report = Sequencer::new(&params, &mut mock).stop_run(StopMode::Normal);

    assert!(report.forced_eos);
    assert!(
        report.eos_closed,
        "動いている間に EOS が届いたのに打ち切っている"
    );
    assert_eq!(report.reason, "normal");
    assert_eq!(mock.slept, Duration::from_millis(8_000));
}

/// 観測カウンタは receiver の `batches`・graw-writer の `asad[].frames` 合計 + `ctrl_frames`・
/// decoder の `frames_in` / `batches_out` の**全部**(どれが動いても待つ)。ここは graw-writer
/// の `ctrl_frames` だけが 3 000 ms まで動く形(asad は不変の 68)。
///
/// 手計算: 第一段 600 ms → 第二段は 2 800 ms のサンプルまで動き、2 800 + 5 000 = 7 800 ms で
/// 打ち切り。
#[test]
fn the_graw_writer_frames_also_count_as_downstream_progress() {
    let params = params_with(&[0]);
    let mut mock = MockTransport::new(&params).counter_moves_until(
        "graw-writer",
        "ctrl_frames",
        Duration::from_millis(3_000),
    );
    mock.set_metric(
        "graw-writer",
        "asad",
        json!([{"cobo": 0, "asad": 0, "bytes": 6528, "frames": 68}]),
    );
    let report = Sequencer::new(&params, &mut mock).stop_run(StopMode::Normal);

    assert!(!report.eos_closed);
    assert_eq!(mock.slept, Duration::from_millis(7_800));
}

/// 観測カウンタの 1 つ 1 つ(SPEC §1.3 v1.53 の 4 系統)が、**単独で**動いても第二段を
/// 引き延ばす(どれかを観測から落とすと、その系統だけが書き出し中の停止は 5 s で切られる)。
///
/// 手計算はどの行も `the_graw_writer_frames_also_count_as_downstream_progress` と同じ:
/// 3 000 ms まで動く → 2 800 ms のサンプルが最後の動き → 2 800 + 5 000 = 7 800 ms で打ち切り
/// (第一段は `bytes` だけを見るので、receiver の `batches` が動いても 600 ms で静止する)。
#[test]
fn each_observed_counter_alone_keeps_the_second_stage_waiting() {
    let params = params_with(&[0]);
    for (component, key) in [
        ("receiver0", "batches"),
        ("decoder", "frames_in"),
        ("decoder", "batches_out"),
        ("graw-writer", "ctrl_frames"),
    ] {
        let mut mock = MockTransport::new(&params).counter_moves_until(
            component,
            key,
            Duration::from_millis(3_000),
        );
        // 動かない側の材料は全部揃えておく(欠けは「動いていない」扱いで判定には効かないが、
        // 1 系統だけが動く形をはっきりさせる)。
        mock.set_metric("receiver0", "batches", json!(1288));
        mock.set_metric("decoder", "frames_in", json!(3852));
        mock.set_metric("decoder", "batches_out", json!(1287));
        mock.set_metric("graw-writer", "ctrl_frames", json!(2));
        mock.set_metric(
            "graw-writer",
            "asad",
            json!([{"cobo": 0, "asad": 0, "bytes": 6528, "frames": 3850}]),
        );
        let report = Sequencer::new(&params, &mut mock).stop_run(StopMode::Normal);

        assert!(!report.eos_closed, "{component}.{key}");
        assert_eq!(
            mock.slept,
            Duration::from_millis(7_800),
            "{component}.{key} が動いているのに第二段を引き延ばしていない"
        );
    }
}

/// **185 テスト 2 — Heartbeat 系だけが動いても待ち続けない**(アイドルでも動くので、観測に
/// 入れると永久待ちになる)。decoder / graw-writer の `heartbeats_*` が 60 s まで動いても、
/// 第二段は `eos_timeout` で打ち切る: 第一段 600 ms + 第二段 5 000 ms = **5 600 ms**。
#[test]
fn heartbeats_alone_do_not_keep_the_stop_waiting() {
    let params = params_with(&[0]);
    let until = Duration::from_secs(60);
    let mut mock = MockTransport::new(&params)
        .counter_moves_until("decoder", "heartbeats_in", until)
        .counter_moves_until("decoder", "heartbeats_out", until)
        .counter_moves_until("decoder", "heartbeats_abandoned", until)
        .counter_moves_until("graw-writer", "heartbeats_in", until);
    let report = Sequencer::new(&params, &mut mock).stop_run(StopMode::Normal);

    assert!(!report.eos_closed);
    assert_eq!(report.reason, "error:eos-timeout");
    assert_eq!(
        mock.slept,
        Duration::from_millis(5_600),
        "Heartbeat の動きを下流の進捗と読んで待ち続けている"
    );
    assert!(
        report.notes.iter().any(|n| n
            == "EOS did not propagate within 5000 ms of no downstream progress after the \
                forced EOS — waited 5000 ms; the observed counters never moved"),
        "{:?}",
        report.notes
    );
}

// -----------------------------------------------------------------
// 185 テスト 4: run_stop.counters のリンクごとの送受と `root_complete`(SPEC §9.2 v1.53)
// -----------------------------------------------------------------

/// 欠けの無い 2 CoBo のチェーン(非対称な値)。手計算:
/// - receiver: frames 3 852 + 3 849 = 7 701、overflow 0、decoder リンクの落とし 0。
/// - graw-writer: asad 1 926 + 1 925 + 1 924 + 1 924 = 7 699、ctrl_frames 2 → 7 701。
/// - decoder: frames_in 7 701、fragments_out 7 699(ctrl 2 本はフラグメントにならない)、
///   abandoned 0 + 0。
fn complete_chain(params: &ControllerParams) -> MockTransport {
    let mut mock = MockTransport::new(params);
    for (name, frames, batches) in [("receiver0", 3852, 1288), ("receiver1", 3849, 1285)] {
        mock.set_metric(name, "frames", json!(frames));
        mock.set_metric(name, "overflow_frames", json!(0));
        mock.set_metric(name, "batches", json!(batches));
        mock.set_metric(name, "decoder_link_dropped_frames", json!(0));
        mock.set_metric(name, "decoder_link_dropped_batches", json!(0));
        mock.set_metric(name, "decoder_link_dropped_heartbeats", json!(0));
    }
    mock.set_metric("graw-writer", "asad", asad_frames(1924));
    mock.set_metric("graw-writer", "ctrl_frames", json!(2));
    mock.set_metric("decoder", "frames_in", json!(7701));
    mock.set_metric("decoder", "fragments_out", json!(7699));
    mock.set_metric("decoder", "batches_out", json!(2571));
    mock.set_metric("decoder", "batches_abandoned", json!(0));
    mock.set_metric("decoder", "eos_abandoned", json!(0));
    mock
}

/// graw-writer の `asad` 配列(cobo 1 / asad 1 のフレーム数だけを変えられる形)。
fn asad_frames(cobo1_asad1: u64) -> Value {
    json!([
        {"cobo": 0, "asad": 0, "bytes": 1, "frames": 1926},
        {"cobo": 0, "asad": 1, "bytes": 1, "frames": 1925},
        {"cobo": 1, "asad": 0, "bytes": 1, "frames": 1924},
        {"cobo": 1, "asad": 1, "bytes": 1, "frames": cobo1_asad1},
    ])
}

/// metrics からキーを 1 つ消す(古いコンポーネントの模擬)。
fn remove_metric(mock: &mut MockTransport, component: &str, key: &str) {
    if let Some(map) = mock
        .metrics
        .get_mut(component)
        .and_then(Value::as_object_mut)
    {
        map.remove(key);
    }
}

/// 新キーが全部載り、欠けが無ければ `lost_* == 0`・`root_complete == true`。
#[test]
fn run_stop_counters_carry_the_link_counts_and_a_complete_run_is_root_complete() {
    let params = params_with(&[0, 1]);
    let mut mock = complete_chain(&params).with_eos_closed();
    let report = Sequencer::new(&params, &mut mock).stop_run(StopMode::Normal);
    let c = &report.counters;

    assert!(report.eos_closed);
    assert_eq!(c.decoder_link_dropped_frames, Some(0));
    assert_eq!(c.decoder_link_dropped_batches, Some(0));
    assert_eq!(c.decoder_link_dropped_heartbeats, Some(0));
    assert_eq!(c.graw_frames, Some(7701));
    assert_eq!(c.decoder_frames_in, Some(7701));
    assert_eq!(c.decoder_fragments_out, Some(7699));
    assert_eq!(c.decoder_abandoned, Some(0));
    assert_eq!(c.lost_receiver_to_graw, Some(0));
    assert_eq!(c.lost_receiver_to_decoder, Some(0));
    assert_eq!(c.root_complete, Some(true));

    // ログブックに載る形(SPEC §9.2 v1.53 のキー名どおり)。
    let json = serde_json::to_value(c).unwrap();
    for key in [
        "decoder_link_dropped_frames",
        "decoder_link_dropped_batches",
        "decoder_link_dropped_heartbeats",
        "graw_frames",
        "decoder_frames_in",
        "decoder_fragments_out",
        "decoder_abandoned",
        "lost_receiver_to_graw",
        "lost_receiver_to_decoder",
    ] {
        assert!(json[key].is_number(), "{key}: {json}");
    }
    assert_eq!(json["root_complete"], json!(true));
}

/// `root_complete == false` の 4 ケース(SPEC §1.4-8)。各ケースの差の手計算は行ごと。
#[test]
fn root_complete_is_false_for_overflow_drop_missing_eos_and_abandon() {
    let params = params_with(&[0, 1]);

    // ① 溢れ(graw の喪失): receiver1 が 5 本溢れ、graw-writer はその 5 本を受けていない
    //    (asad (1,1) = 1 919 → 7 696)、decoder も 7 696 本。差 = (7 701 − 5) − 7 696 = 0。
    let mut mock = complete_chain(&params).with_eos_closed();
    mock.set_metric("receiver1", "overflow_frames", json!(5));
    mock.set_metric("graw-writer", "asad", asad_frames(1919));
    mock.set_metric("decoder", "frames_in", json!(7696));
    let c = Sequencer::new(&params, &mut mock)
        .stop_run(StopMode::Normal)
        .counters;
    assert_eq!(c.overflow_frames, Some(5));
    assert_eq!(c.graw_frames, Some(7696));
    assert_eq!(c.lost_receiver_to_graw, Some(0));
    assert_eq!(c.lost_receiver_to_decoder, Some(0));
    assert_eq!(c.root_complete, Some(false), "溢れ");

    // ② decoder リンクで数えて落とした: receiver0 が 120 本(40 バッチ)を落とし、decoder は
    //    7 701 − 120 = 7 581 本。差 = 7 701 − 0 − 120 − 7 581 = 0、graw は完全。
    let mut mock = complete_chain(&params).with_eos_closed();
    mock.set_metric("receiver0", "decoder_link_dropped_frames", json!(120));
    mock.set_metric("receiver0", "decoder_link_dropped_batches", json!(40));
    mock.set_metric("receiver1", "decoder_link_dropped_heartbeats", json!(3));
    mock.set_metric("decoder", "frames_in", json!(7581));
    let c = Sequencer::new(&params, &mut mock)
        .stop_run(StopMode::Normal)
        .counters;
    assert_eq!(c.decoder_link_dropped_frames, Some(120));
    assert_eq!(c.decoder_link_dropped_batches, Some(40));
    assert_eq!(c.decoder_link_dropped_heartbeats, Some(3));
    assert_eq!(c.lost_receiver_to_graw, Some(0));
    assert_eq!(c.lost_receiver_to_decoder, Some(0));
    assert_eq!(c.root_complete, Some(false), "decoder リンクの落とし");

    // ③ EOS 未達(強制しても流れ切らない): graw-writer は 11 本受けていない(asad (1,1) =
    //    1 913 → 7 690)、decoder は 51 本受けていない(7 650)= ZMQ のキューに居たまま。
    //    差 = 7 701 − 7 690 = 11 / 7 701 − 0 − 7 650 = 51。
    let mut mock = complete_chain(&params);
    mock.set_metric("graw-writer", "asad", asad_frames(1913));
    mock.set_metric("decoder", "frames_in", json!(7650));
    let report = Sequencer::new(&params, &mut mock).stop_run(StopMode::Normal);
    assert!(!report.eos_closed);
    assert_eq!(report.counters.graw_frames, Some(7690));
    assert_eq!(report.counters.lost_receiver_to_graw, Some(11));
    assert_eq!(report.counters.lost_receiver_to_decoder, Some(51));
    assert_eq!(report.counters.root_complete, Some(false), "EOS 未達");

    // ④ decoder が手放した(Reset の打ち切り): batches 1 + eos 1 = 2。
    let mut mock = complete_chain(&params).with_eos_closed();
    mock.set_metric("decoder", "batches_abandoned", json!(1));
    mock.set_metric("decoder", "eos_abandoned", json!(1));
    let c = Sequencer::new(&params, &mut mock)
        .stop_run(StopMode::Normal)
        .counters;
    assert_eq!(c.decoder_abandoned, Some(2));
    assert_eq!(c.root_complete, Some(false), "decoder の abandoned");
}

/// 値が取れないときは null(0 と区別)。
#[test]
fn link_counts_are_null_when_a_component_cannot_tell() {
    let params = params_with(&[0, 1]);
    let dropped_keys = [
        "decoder_link_dropped_frames",
        "decoder_link_dropped_batches",
        "decoder_link_dropped_heartbeats",
    ];

    // 184 より前の receiver(decoder リンクのカウンタが無い): 落としの 3 キー・decoder 側の
    // 差・root_complete が null。graw 側の差は材料が揃うので 0。
    let mut mock = complete_chain(&params).with_eos_closed();
    for name in ["receiver0", "receiver1"] {
        for key in dropped_keys {
            remove_metric(&mut mock, name, key);
        }
    }
    let c = Sequencer::new(&params, &mut mock)
        .stop_run(StopMode::Normal)
        .counters;
    assert_eq!(c.decoder_link_dropped_frames, None);
    assert_eq!(c.decoder_link_dropped_batches, None);
    assert_eq!(c.decoder_link_dropped_heartbeats, None);
    assert_eq!(c.lost_receiver_to_graw, Some(0));
    assert_eq!(c.lost_receiver_to_decoder, None);
    assert_eq!(c.root_complete, None);
    let json = serde_json::to_value(&c).unwrap();
    assert_eq!(json["decoder_link_dropped_frames"], Value::Null);
    assert_eq!(json["root_complete"], Value::Null);

    // 1 台だけ申告しない(receiver1 が古い)なら合計は出さない(半端な和を出さない)。
    let mut mock = complete_chain(&params).with_eos_closed();
    remove_metric(&mut mock, "receiver1", "decoder_link_dropped_frames");
    let c = Sequencer::new(&params, &mut mock)
        .stop_run(StopMode::Normal)
        .counters;
    assert_eq!(c.decoder_link_dropped_frames, None);
    assert_eq!(c.decoder_link_dropped_batches, Some(0));

    // graw-writer 不達: graw のフレーム数と graw 側の差が null。
    let mut mock = complete_chain(&params)
        .with_eos_closed()
        .unreachable("graw-writer");
    let c = Sequencer::new(&params, &mut mock)
        .stop_run(StopMode::Normal)
        .counters;
    assert_eq!(c.graw_frames, None);
    assert_eq!(c.lost_receiver_to_graw, None);
    assert_eq!(c.lost_receiver_to_decoder, Some(0));

    // decoder 不達: decoder の 3 キー・decoder 側の差・root_complete が null。
    let mut mock = complete_chain(&params)
        .with_eos_closed()
        .unreachable("decoder");
    let c = Sequencer::new(&params, &mut mock)
        .stop_run(StopMode::Normal)
        .counters;
    assert_eq!(c.decoder_frames_in, None);
    assert_eq!(c.decoder_fragments_out, None);
    assert_eq!(c.decoder_abandoned, None);
    assert_eq!(c.lost_receiver_to_decoder, None);
    assert_eq!(c.lost_receiver_to_graw, Some(0));
    assert_eq!(c.root_complete, None);
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

/// decoder の取得再開カウンタ(TODO/110)が `run_stop.counters` に載る。
/// `frames_before_restart` は上流ソース別の辞書(TODO/133)。値は非対称(restarts=1 /
/// source 0 = 19 / source 1 = 2)。metrics に無ければ `None`(0 と区別)。
#[test]
fn run_stop_counters_carry_the_decoder_restart_counters() {
    let params = params_with(&[0]);
    let mut mock = MockTransport::new(&params).with_eos_closed();
    mock.metrics.insert(
        "decoder".to_string(),
        json!({"eos_in": 1, "eos_out": 1, "malformed": 0,
               "acquisition_restarts": 1, "frames_before_restart": {"0": 19, "1": 2}}),
    );
    let report = Sequencer::new(&params, &mut mock).stop_run(StopMode::Normal);
    assert_eq!(report.counters.acquisition_restarts, Some(1));
    assert_eq!(
        report.counters.frames_before_restart,
        Some(FramesBeforeRestart::PerSource(BTreeMap::from([
            ("0".to_string(), 19),
            ("1".to_string(), 2),
        ])))
    );

    let json = serde_json::to_value(&report.counters).unwrap();
    assert_eq!(json["acquisition_restarts"], json!(1));
    assert_eq!(json["frames_before_restart"], json!({"0": 19, "1": 2}));

    // decoder が古くてキーを持たない → null(取得不能)。
    let mut mock = MockTransport::new(&params).with_eos_closed();
    mock.metrics
        .insert("decoder".to_string(), json!({"eos_in": 1, "malformed": 0}));
    let report = Sequencer::new(&params, &mut mock).stop_run(StopMode::Normal);
    assert_eq!(report.counters.acquisition_restarts, None);
    assert_eq!(report.counters.frames_before_restart, None);
}

/// frameType 別カウンタ + compact サイズ不一致(TODO/158)が `run_stop.counters` に載る。
/// 非対称な値 type1=7 / type2=3 / mismatch=2。decoder が持たなければ `None`(0 と区別)。
#[test]
fn run_stop_counters_carry_the_frame_type_counters() {
    let params = params_with(&[0]);
    let mut mock = MockTransport::new(&params).with_eos_closed();
    mock.metrics.insert(
        "decoder".to_string(),
        json!({"eos_in": 1, "eos_out": 1, "malformed": 0,
               "frames_type1": 7, "frames_type2": 3, "compact_size_mismatch": 2}),
    );
    let report = Sequencer::new(&params, &mut mock).stop_run(StopMode::Normal);
    assert_eq!(report.counters.frames_type1, Some(7));
    assert_eq!(report.counters.frames_type2, Some(3));
    assert_eq!(report.counters.compact_size_mismatch, Some(2));

    let json = serde_json::to_value(&report.counters).unwrap();
    assert_eq!(json["frames_type1"], json!(7));
    assert_eq!(json["frames_type2"], json!(3));
    assert_eq!(json["compact_size_mismatch"], json!(2));

    let mut mock = MockTransport::new(&params).with_eos_closed();
    mock.metrics
        .insert("decoder".to_string(), json!({"eos_in": 1, "malformed": 0}));
    let report = Sequencer::new(&params, &mut mock).stop_run(StopMode::Normal);
    assert_eq!(report.counters.frames_type1, None);
    assert_eq!(report.counters.compact_size_mismatch, None);
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
