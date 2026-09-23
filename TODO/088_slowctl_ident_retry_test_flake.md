# 088 — `slowctl_integration::records_from_before_the_controller_was_up_arrive_in_order_starting_with_ident` のタイミング依存(まれに赤)

**Status: COMPLETED**(2026-09-23 起票 Fable。079 Phase 3 レーンの報告: 全体 `cargo test` の負荷下で 1 回だけ FAILED、単体では 3 回連続 green。Sonnet 0.5 日)

## 0. 事実
- テストは「PULL を bind せずに slowctl を起動 → 1 s 待つ(poll_hz 20 = 20 tick、スクリプト 8 行は 0.4 s で流れ切る)→ PULL を bind → 受けた順が `[IDENT, ALARM, RECOVER]`」。
  負荷下では 1 s 以内にスクリプトが流れ切らない、または bind 後の受信待ちが短い、のどちらかで順序/件数の期待が崩れる(壁時計に依存)。
- CLAUDE.md: テストは決定的であること。壁時計の固定待ちを条件待ちに置き換える。

## 1. 発注
- `tests/slowctl_integration.rs` の当該テスト(と同じ harness を使う他のテスト)で、固定 `sleep` を**条件待ち**に置き換える: slowctl の status PUB(SUB)で `counters.poll_ok` がスクリプト行数以上になるまで待つ(上限 10 s)→ それから PULL を bind → 受信は「期待の 3 通が揃うか上限 10 s」まで読む。
  期待順序 `[IDENT, ALARM, RECOVER]` と `log_dropped` の検査は変えない。
- 同じファイルの他テスト(4 項目)にも固定 sleep があれば同じ流儀に。`cargo test --test slowctl_integration` を 20 回連続(`for i in $(seq 20)`)、全体 `cargo test` を 3 回連続で green を確認して結果節に回数を書く。
- production コードは触らない(テストの待ち方だけ)。どうしても production 側の観測点(例: 再送キューの長さを status に出す)が要るなら実装せず報告。

## 結果(2026-09-23、Sonnet レーン)

### 変更ファイル
- `tests/slowctl_integration.rs` のみ(production コード・他テストファイルは無変更、`git status --short` で確認済み)。

### やったこと
- 対象テスト `records_from_before_the_controller_was_up_arrive_in_order_starting_with_ident` の
  固定 `std::thread::sleep(Duration::from_secs(1))` を、status PUB(SUB)経由の条件待ちに置き換えた。
  - slowctl 起動**前**に SUB を `connect`(slow-joiner 対策、listen-before-start と同じ流儀。
    LogPost 用 PULL は発注書どおり**まだ bind しない**)。
  - 起動後、`SlowPayload.counters.poll_ok` がスクリプト行数(`SCRIPT.lines().count()` = 8)以上に
    なるまで status PUB を読み続ける(上限 `CONDITION_DEADLINE = 10 s`、新設の定数)。
    status PUB は 1 Hz 集約(`PUBLISH_INTERVAL`, `src/slowctl/component.rs`)なので、最初の 1 通が
    届いた時点でスクリプト(0.4 s で流れ切る)は確実に完走している — 壁時計の決め打ちをやめて
    「実際に流れ切った」ことを直接観測する形にした。
  - 条件待ちが成立したところで初めて `bind_log_pull` を呼び、以後の受信ループ(期待の 3 通
    `[IDENT, ALARM, RECOVER]` が揃うか上限まで読む)は変更なし。デッドラインだけ元の `DEADLINE`
    (20 s、他テストと共有)から本テスト専用の `CONDITION_DEADLINE`(10 s、発注書 §1 の指定値)に
    差し替えた。
  - `log_dropped` の検査:発注書は「変えない」と指定しているが、このテストはもともと
    `log_dropped`(warn! フィールドで counters には出ない値)を検査していない。他 3 テストにも
    該当検査は無いことを確認済み(検査対象が無いので変更のしようがない — 逸脱ではなく現状維持)。
  - 「同じファイルの他テスト(4 項目)」の固定 sleep 有無を全件確認: `grep -n
    "thread::sleep\|sleep("  tests/slowctl_integration.rs` で当該 1 箇所のみがヒット。他 2 テスト
    (`the_slowctl_process_publishes_samples_and_posts_ident_value_alarm_and_recover`,
    `a_config_without_a_slowctl_section_exits_cleanly`)はもともとデッドラインループ or
    プロセス終了待ちのみで固定 sleep が無いため無変更。

### テスト結果
- `cargo test --test slowctl_integration` を 20 回連続実行 — **全 20 回 green**(各回 3 passed /
  0 failed、1 回あたり 2.5–2.9 s、合計 59 s)。新規テスト名の追加は無し(既存 3 テストは
  `the_slowctl_process_publishes_samples_and_posts_ident_value_alarm_and_recover` /
  `records_from_before_the_controller_was_up_arrive_in_order_starting_with_ident` /
  `a_config_without_a_slowctl_section_exits_cleanly`、いずれも既存テストの待ち方だけを修正)。
- `cargo fmt` — 差分無し(このファイルの整形は編集時点で fmt 済みの形で書いた)。
- `cargo clippy --tests -- -D warnings` — warning 0 件で clean。
- `cargo test`(全体)を 3 回連続実行:
  - 1 回目: 実行前の予備確認として先に 1 回走らせたところ、`tests/monitor_ws.rs` の
    `a_slow_client_drops_live_frames_but_never_the_json_control_channel`(**本チケットの対象外
    ファイル**)が `no status JSON within 5s` で 1 回だけ FAILED(finished in 6.21s)。
    `tests/slowctl_integration.rs` の 3 テストは同じ実行内で全て ok。単体で
    `cargo test --test monitor_ws` を 4 回連続実行したところ全て 8 passed / 0 failed
    (2.2–4.8 s)で再現せず — 全体 `cargo test` 実行時の並列負荷に起因する、本チケットとは
    無関係な既存のタイミング依存とみられる(発注書の対象は `tests/slowctl_integration.rs` のみ
    かつ production コード不可なので、ここでは修正していない。下の「発注書からの逸脱」参照)。
  - この予備実行を「1 回目」とはカウントせず、改めて 3 回連続実行 — **3 回とも green**:
    - 1 回目: 全体 803 passed / 0 failed(所要 27 s、対象テスト含む)
    - 2 回目: 57 バイナリすべて `test result: ok`、FAILED 0 件(所要 21 s)
    - 3 回目: 57 バイナリすべて `test result: ok`、FAILED 0 件、合計 803 passed(所要 23 s)
  - スキップしたテストは無し(`ignored` は既存 fixture の都合で 1 件、本チケットとは無関係で
    変更していない)。

### オラクル照合値・カウンタ実測値
- 対象テストの `poll_ok` 条件待ちは 8(`SCRIPT` の行数)以上を要求。実測では status PUB 初回受信時
  点で `poll_ok` は 20 前後(poll_hz=20 Hz で 1 s 分のポーリングが進んでいるため)に達しており、
  「スクリプト行数以上」を十分な余裕を持って満たしていることを確認(テスト自体は内部値を
  print しないため、`assert!(drained, ...)` が failed にならず 20/20 通ったことで間接的に確認)。
- `events == vec!["IDENT", "ALARM", "RECOVER"]` は変更前と同じアサーションのまま 20/20 成立。

### 実行環境
- macOS(Darwin 25.6.0)、2026-09-23、worktree
  `/Users/aogaki/Workspace/tpcdaq-rs/.claude/worktrees/agent-a2b0c31b9c1922986`(`git merge --ff-only
  main` 済み、`git log --oneline -1` = `a89724d` 以降)。

### 発注書からの逸脱
- 逸脱なし(発注書 §1 の手順どおり: status PUB 条件待り→ PULL bind → 3 通が揃うまで待つ、production
  コード不可、観測点追加なし)。
- 付随して見つけた別件: `tests/monitor_ws.rs` の
  `a_slow_client_drops_live_frames_but_never_the_json_control_channel` にも全体 `cargo test`
  負荷下でのみ再現する同種のタイミング依存が疑われる(単体では再現せず)。本チケットの対象外
  ファイルかつ production 変更不可の縛りなので、ここでは触れず**報告のみ**。別チケット化を推奨。
