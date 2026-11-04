# 188 — controller の run 中監視 + 保存系 Error の自動異常停止(SPEC §1.3「run 中の監視」/ §1.4-3)

**Status: TODO(発注書 — Fable 2026-11-04 起票。11/9 前に要る。204 の後に着手、Opus implementer)**(設計 = TODO/181 §4-4 / §5-1「§4-4」(Fable レビュー済み 2026-10-24)、仕様 = SPEC v1.53 §1.3「run 中の監視(v1.53、TODO/188)」の段落 + §1.4-3。文言は既に SPEC にある — 実装だけ)

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
