# 188 — controller の run 中監視 + 保存系 Error の自動異常停止(SPEC §1.3「run 中の監視」/ §1.4-3)

**Status: IMPLEMENTED — Fable レビュー中・マージ待ち・実機の受け入れ待ち(2026-11-04、ブランチ `worktree-agent-af34f5c9c61d9815f`)**(発注書 — Fable 2026-11-04 起票。11/9 前に要る)(設計 = TODO/181 §4-4 / §5-1「§4-4」(Fable レビュー済み 2026-10-24)、仕様 = SPEC v1.53 §1.3「run 中の監視(v1.53、TODO/188)」の段落 + §1.4-3。文言は既に SPEC にある — 実装だけ)

## 0. 背景(1 行ずつ)
- run 197: receiver の内部キューが 17:35:11 に溢れた(= graw の喪失)が、receiver が Error になるのは**コマンドを受けたとき**(`receiver.rs` `handle()` 冒頭の `latch_overload`)で、controller は run 中に誰にも `GetStatus` を送らない → 17:40:49 にユーザーが画面を開くまで 5.5 分、誰も気づかず graw を 19,855 失った。
- SPEC §1.4-3「controller は run 中の定期監視(既定 2 s)でこれを検知して run を異常停止する」は**未実装**(停止時の中止判定 `rest.rs::stop_run_blocking` だけ)。
- 11/9 からの高レート運転(1 GbE 上限 182 Hz ≈ 50 MB/s ≈ 180 GB/h を冗長性なしの ZFS に書く)では、ディスクの詰まり・満杯で graw-writer が止まる = receiver の溢れ = graw の喪失を**誰も止めない**。これを ≤ 2 s で止めるのが本チケット。
- 204(停止の第二段が root_sink の close まで待つ・graw-writer が開いている間は切らない)の**後**に着手: 自動異常停止は `stop_run_blocking` を共用するので 204 の停止がそのまま効く。同じ `src/controller/` を触るので直列。

## 1. 変更(決定済み — 設計の分岐は無い。出会ったら実装せず報告)
### A. 監視タスク(`src/controller/watchdog.rs`、新規)
- controller の phase が `Running` または `Stopping` の間、**2 s ごと**に全コンポーネント(receivers / decoder / graw-writer)へ `GetStatus`(`status_timeout`、`ZmqTransport::command` — 呼び出しごとに REQ を作るので制御コマンドとソケットは衝突しない。REP は逐次なので decoder の Reset の join 中などは 1 回不達に見えることがある)。
- 結果(各コンポーネントの state / reachable / 主要カウンタ)は `Controller` の共有の最新値に置く(`Arc<Mutex<…>>`)。`GET /api/status` は今どおり自分で `GetStatus` を打ってよい(変えない)。将来 UI の 2 s 更新に使えるが本チケットでは使わない。
- 専用 OS スレッドか tokio タスク(blocking な REQ を `spawn_blocking` で)— 実装の判断。controller の終了で止まる(スレッド / タスクを残さない)。
### B. 自動異常停止(phase == Running のときだけ)
- 引き金 = **receiver または graw-writer(保存系)が `Error`**。
  - **引き金にしない**: decoder の Error、`decoder_link_dropped_frames` の増加(§1.4-7 — 画面に出すだけ、187)、**1 回の不達**(`reachable == false` 1 回。連続不達でも止めない — 不達は停止で `abort:component-error` になる既存の経路に任せる)、Stopping / Idle / Starting 中の Error(Stopping 中は観測のみ)。
- 動作: `controller.run` の mutex 下で `phase: Running → Stopping` を **CAS**(`rest.rs` の `POST /api/run/stop` と同じロック区間。取り合って負けた側 = 既に Stopping なら何もしない。REST 側は既存どおり 409)。`stopping_since` も同じロック区間で立てる(185 の申し送り — 立てないと `/api/status.stopping.waited_s` が null)。
- 停止本体 = `stop_run_blocking` を**共用**(新しい停止経路を作らない)。`StopMode::Abort("overload:<component名>")` → run_stop の `reason = "abort:overload:receiver0"` 等、`operator = "watchdog"`。audit に 1 行(`action = "run/stop"`、`operator = "watchdog"`、`params` に `trigger: {component, state, overflow_frames}`)。
- `stop_run_blocking` の中止判定(「Error / 不達のコンポーネントがあれば abort」)はそのまま働く — reason は watchdog が渡した `abort:overload:…` を優先する(二重に合成しない)。
### C. 設定
- 周期は定数 2 s(SPEC §1.3 の既定)。**新しい TOML キーは作らない**。

## 2. テスト(受け入れ — TDD: 先に赤。名前は固定)
`tests/controller_integration.rs`(本物の controller + モックのコンポーネント REP、既存の流儀)か `src/controller/` の単体(MockTransport)— どちらで書くかは実装の判断だが、①と④は controller の phase / ロックを通す統合で:
1. `a_receiver_in_error_is_aborted_by_the_watchdog_within_two_polls` — Running 中に receiver が Error を報告 → ≤ 2 回目のポーリング(≈ 4 s)で run_stop `reason == "abort:overload:receiver0"`、audit の `operator == "watchdog"`、phase は Stopping → Idle。graw-writer の Error でも同じ(`abort:overload:graw-writer`)。
2. `a_decoder_in_error_does_not_trigger_the_watchdog` — decoder Error が続いても run は止まらない(≥ 3 ポーリング待って phase == Running)。
3. `a_single_unreachable_component_does_not_trigger_the_watchdog` — 1 回の不達(以後回復)で止まらない。
4. `a_concurrent_manual_stop_and_the_watchdog_stop_the_run_exactly_once` — receiver Error と同時に `POST /api/run/stop` → 停止は 1 回(run_stop レコード 1 本、ecc stop 1 回)、負けた側は 409 か何もしない。
5. `the_watchdog_does_not_fire_outside_running` — Idle / Starting / Stopping 中の Error で停止しない(Stopping 中の観測は続く)。
6. `the_watchdog_sets_stopping_since_so_the_status_shows_waited_s` — watchdog 経由の停止でも `/api/status.stopping.waited_s` が数値。
- 全体: `cargo fmt && cargo clippy --tests -- -D warnings && cargo test`。既存の停止・歩き戻し・REST テストは無改変 green。

## 3. 受け入れ(実機 ELITPC — 主対話がユーザーの OK を得て、物理 run の無いときに)
- 試験 run(周期トリガ 10 ms、pevent = false、60 s)の最中に graw_writer を `kill -STOP` → ≈ 2 s 分の受信で receiver の内部キュー(1,000)が溢れる → **≤ 4 s 以内に run_stop `abort:overload:receiver0`(operator watchdog)が書かれ、phase Stopping**。204 の守り(graw-writer が到達可能で `files_open > 0` なら切らない)で停止は待つ — graw-writer は SIGSTOP 中は不達なので、**≈ 10 s 後に `kill -CONT`** → EOS が流れて graw が閉じる → `forced_eos:true`、`eos_closed:true`、`overflow_frames > 0`、`lost_receiver_to_graw == 0`(恒等式 = 溢れた分は receiver の `frames` に含まれ graw に無い、差は overflow で説明される)、graw ファイルは閉じている。SPEC §12-14 の末尾の文の実機確認。
- 対照: 同じ試験を SIGSTOP 無しで → `ok:true` / `normal`、watchdog の audit 行は無い。

## 4. 範囲・禁止
- 触ってよい: 新規 `src/controller/watchdog.rs`、`src/controller/{mod.rs(起動・終了), rest.rs(phase CAS の共有・stop_run_blocking の operator / mode 引数), sequencer.rs(StopMode に overload の表現が要るなら最小), status.rs}`、`tests/controller_integration.rs`。worktree で作業、着手時に `git merge --ff-only main`(204 のマージ後)。
- **触らない**: `src/receiver.rs` / `src/decoder.rs` / `src/graw_writer.rs`(`latch_overload` の癖はこの監視で実害が消える — 変えない)、`tools/root_sink/`、`docs/`、`TODO/`(本チケットの `## 結果` 以外)、`ui/`(187)。新しい TOML キー・REST エンドポイントを作らない。decoder の Error で止めない。
- モデル: **Opus**(ロックの取り合い・停止の共用・スレッドの寿命)。

## 5. 報告(必須)
worktree のブランチとコミット、`git diff --stat`、テスト 1–6 の名前・場所・PASS / FAIL、赤の証拠、cargo のテスト数、発注書からの逸脱と理由、**「実行できなかった操作」の節(無ければ「なし」)**、残プロセスが無いこと。

## 結果(2026-11-04 — 実装・テスト済み。Fable レビュー、マージ、ELITPC の受け入れは未)
- **実装**: implementer(Opus)、worktree `.claude/worktrees/agent-af34f5c9c61d9815f`、ブランチ `worktree-agent-af34f5c9c61d9815f`。並び = `dc8d957`(204)→ **`6319dea`**(188 本体)→ `953a8d9`(204 の追補 `90af662` のマージ、衝突なし)。188 だけの差分(`git diff --stat 90af662..HEAD`)= 5 ファイル + 976 / − 30: `src/controller/watchdog.rs`(新規 348)、`src/controller/{mod.rs, rest.rs, status.rs}`、`tests/controller_integration.rs`。receiver / decoder / graw_writer / sequencer は無変更(主対話が diff で確認)。主対話が 6 本のテスト名(各 1 件)を照合。
- **中身**: `WATCHDOG_PERIOD = 2 s` の専用 OS スレッド。Idle / Starting は問い合わせない、Stopping は全コンポーネントへ GetStatus(観測のみ)、Running は receiver / graw-writer の Error を引き金に `Controller::begin_stopping(Some(run))`(CAS + `stopping_since` を同じロック区間、REST の stop と共有。同じ run かも確かめる)→ 停止の本体は別スレッド `watchdog-stop` で `stop_run_blocking(.., "watchdog", active, Some(&trigger))`(mode `Abort("overload:<名前>")`、audit `params.trigger = {component, state, overflow_frames}`)。drop ガードで Idle に戻す。終了時は watchdog が停止を join、`run_controller` が watchdog を join。到達性 / Error の変化だけ warn / info 1 行。
- **テスト**(TDD、赤 = `poll_once` のスタブで lib 1 FAILED・統合 0 / 5 passed): 1 `a_receiver_in_error_is_aborted_by_the_watchdog_within_two_polls`(統合、receiver0 と graw-writer の両方、< 4 s)・2 `a_decoder_in_error_does_not_trigger_the_watchdog`(統合、3 ポーリング)・3 `a_single_unreachable_component_does_not_trigger_the_watchdog`(統合、GetStatus を 2.5 s 寝かせる)・4 `a_concurrent_manual_stop_and_the_watchdog_stop_the_run_exactly_once`(統合、3 ラウンド、本物の取り合いで手動 200 が 4 回・409 が 2 回)・5 `the_watchdog_does_not_fire_outside_running`(単体 MockTransport — 統合のカウントでは Stopping の観測を区別できないため、逸脱)・6 `the_watchdog_sets_stopping_since_so_the_status_shows_waited_s`(統合)— すべて PASS。
- **cargo**(`90af662` のマージ後、2026-11-04、macOS): fmt OK、clippy 警告 0、`cargo test --no-fail-fast` **1,155 passed / 0 failed / 1 ignored**(lib 811、controller_integration 48)。env ゲート: controller_integration 48/48、p3_e2e 5/5、p3_error_paths 2/3(既知の F2 = [206](206_p3_f2_stale_exit6_expectation.md))。
- **逸脱・判断(申告)**: テスト 5 は単体、停止は別スレッド、CAS で同じ run を確認、同時 Error は components の順で最初(graw-writer が receiver より先 = ディスクで止まった graw-writer の名前が reason に出る)、変化だけのログ、共有の最新値は `Controller` の Mutex フィールド、テストのフェイクにつまみ。
- **実行できなかった操作**: heredoc での追記が worktree 隔離の検査で拒否 → Edit で同じ内容(実害なし)。残プロセスなし。
- **未**: Fable のフェーズ境界レビュー(2026-11-04 依頼中)→ main にマージ → ELITPC 反映 → 受け入れ(§3: graw_writer の SIGSTOP → ≤ 4 s で `abort:overload:receiver0`、SIGCONT で閉じる + 対照)。

## Fable のフェーズ境界レビュー(2026-11-04、`953a8d9`)
- **判定: このままマージ可**。Fable が worktree で `cargo test --test controller_integration watchdog` 5/5・`--lib watchdog` 1/1 を実走。CAS(`begin_stopping` が唯一の Running → Stopping の入口、run 番号も照合)・停止の共用・`watchdog-stop` のスレッドの寿命・ロック順序・REQ の作り方・周期(累積ドリフトなし)・REST の 409 の意味の保存・audit / reason / ログ・テスト 4 の取り合い(揺らがない)はすべて問題なし。
- **マージ前に入れる(同じブランチの上に 1 コミット — 2026-11-04 発注)**: 1 = controller の終了中に watchdog の停止が走ると、root-sink の SUB スレッド(204)が共有の `stop_flag` で先に死に、`eos_closed:false` を偽に記録する → SUB 用の停止フラグを分け、watchdog の join の後に立てる。3 = Stopping 中の不達(decoder の Reset の join ≤ 2.1 s > status_timeout 1 s)は info に。
- **188 の外(別票)** → [209](209_graw_writer_status_during_disk_stall.md): graw-writer の GetStatus は RunWriter のロックを blocking で取るので、**実ディスクの詰まり中は不達に見え、204 の守り(到達可能 ∧ files_open > 0)が Off** → 停止は静止 ≥ eos_timeout で切れ、receiver の graw リンクの送出待ちを `messages_abandoned` に数えて捨てる(= ユーザー裁定 B-4「ディスクが固まったら graw-writer を落とすまで待つ」が実際には効かない)。指摘 4(不達の相手に毎周 1 s)は多 CoBo のときの留意だけ。
- **受け入れ §3 の読み方(Fable の机上計算、既定値 status 1 s / command 5 s / eos_timeout 5 s / quiesce 500 ms)**: SIGSTOP 中の graw-writer は不達 → 守りは Off。**SIGCONT が abort の audit 行から ≈ 20 s 以内(第一段の中)なら `eos_closed:true`**、遅れると打ち切り + `messages_abandoned > 0`(試験 run の graw の一部を失う)。→ 実機では abort の audit 行を見てから ≈ 10 s で SIGCONT。ELITPC の daq.toml に時間のキーは無い = 既定値(2026-11-04 確認)。
- SPEC §1.3「run 中の監視」の文言修正案(Fable A–D): 取り合いは CAS 1 箇所(REST が負けたら 409、watchdog が負けたら何もしない、run 番号も照合)、不達は 1 回でも連続でも引き金にしない、同時 Error は components の順で最初、停止の本体は別スレッドで Stopping 中も観測が続く、§9.2 audit の `params.trigger` — マージ時に主対話が docs へ。運用注記: systemd の `TimeoutStopSec` が 204 の待ちより短いと SIGKILL → 次の起動の歩き戻しに落ちる(RUNBOOK に 1 行)。
