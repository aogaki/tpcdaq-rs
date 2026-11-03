# 185 — controller の停止: 中止も静止を待つ、強制 EOS の後は「下流が進んでいる限り待つ」、run_stop にリンクごとの送受と `root_complete`

**Status: COMPLETED**(2026-11-03 main へマージ。実機の受け入れ = ELITPC 反映後、184 と一緒に — CURRENT)(2026-10-24 起票 Opus、実装 **Opus implementer**。設計 = TODO/181 §5-1「§4-2」(Fable レビュー済み)、仕様 = SPEC v1.53 §1.3(停止シーケンス 2・異常中止 ①・run 単位の照合)/ §1.4-8 / §9.2「v1.53 run_stop.counters の追加キー」)

## 0. 背景
- run 197: 強制 EOS の後の待ち(`wait_for_eos`、`eos_timeout` 固定 5 s)で打ち切り → Reset で途中の ≈ 1,097 フレームが数えられずに消えた。中止(`StopMode::Abort`)は第一段の静止検出を飛ばして即 Stop するので、receiver の drain が畳まれてカーネル受信バッファの未読分(= graw)も数えられずに消えうる。
- 184 で receiver は decoder リンクを数えて落とす(`decoder_link_dropped_frames` / `_batches` / `_heartbeats`)・Error 中も `Stop` を実行する(EOS を出せる)ようになる。**このカウンタ名は 184 の発注書で固定済み** — 本チケットはモックの GetStatus で先に作ってよい。

## 1. 変更(決定済み)
### A. `src/controller/sequencer.rs` `stop_run`
- **中止でも第一段(`wait_for_eos_or_quiesce`)を通す**: `StopMode::Abort(_) => false` の近道を撤去。中止は reason のラベルだけで、閉じ方は正常停止と同じ。
- **第二段(強制 EOS の後、`wait_for_eos`)を進捗待ちに**: `eos_complete` なら true。そうでなければ観測カウンタを `eos_poll` ごとに採り、**`eos_timeout` のあいだ 1 つも動かなければ**打ち切って false(今の「通算 `eos_timeout` で打ち切り」をやめる)。上限のノブは**置かない**。
  - 観測カウンタ = receiver の `batches`、graw-writer の `asad[].frames` の合計(+ `ctrl_frames`)、decoder の `frames_in` と `batches_out`。**Heartbeat 系(`heartbeats_*`)は入れない**(アイドルでも動き、永久待ちになる)。
  - 不達のコンポーネントは「動いていない」とみなす(今の `receiver_bytes` の不達の扱いと同じ流儀)。
  - 打ち切ったときの notes の文言は今の流儀(`"eos: did not propagate …"`)に「最後に動いたのは何秒前か」を足す。
### B. 停止中の見える化 — `/api/status`
- phase が `Stopping` の間、`/api/status` に **`stopping: {decoder_backlog, waited_s}`** を出す(`decoder_backlog` = decoder の `frames_in − fragments_out`、取れなければ null。`waited_s` = 停止開始からの秒)。Stopping 以外では出さない(キーごと省略)。
- 実装の形(sequencer → status への受け渡し)は工学判断でよいが、`controller.run` の mutex を停止の待ちの間ずっと握らないこと(status が止まる)。
- `POST /api/run/stop` は今どおり停止の完了まで応答しない(分単位になりうる。UI は 187 で phase を見る)。HTTP 側にタイムアウトが入っていないことを確かめる(axum / tower の設定)。
### C. run_stop.counters の追加(`src/controller/status.rs` `counters_from`、`Counters` の型、ログブックへの直列化)— SPEC §9.2 v1.53 のキー名どおり、**すべて nullable**(そのコンポーネントの値が取れなければ null。0 と区別):
- `decoder_link_dropped_frames` / `decoder_link_dropped_batches` / `decoder_link_dropped_heartbeats`(receiver の合計)
- `graw_frames`(graw-writer の `asad[].frames` 合計 + `ctrl_frames`)
- `decoder_frames_in`、`decoder_fragments_out`、`decoder_abandoned`(= `batches_abandoned + eos_abandoned`)
- `lost_receiver_to_graw` = Σ receiver(`frames − overflow_frames`)− `graw_frames`
- `lost_receiver_to_decoder` = Σ receiver(`frames − overflow_frames − decoder_link_dropped_frames`)− `decoder_frames_in`
- **`root_complete: bool`** = `overflow_frames == 0 ∧ decoder_link_dropped_frames == 0 ∧ eos_closed ∧ decoder_abandoned == 0`(どれかが null なら null)
- 材料は今どおり「Stop / Reset の前」の `collect_status`(graw-writer は Reset でファイル実績を手放す)。
- 注意: receiver の `frames` は溢れた分も含む受信数(`receiver.rs:453` で try_send の前に数える)。恒等式は欠けが無ければ `lost_* == 0`。

## 2. テスト(受け入れ — TDD: 先に赤、報告に赤の証拠)
既存の `src/controller/sequencer/tests_stop.rs`(モック transport・仮想時間)と `src/controller/test_support.rs` の流儀で。
1. 強制 EOS の後、decoder の `batches_out` が増え続ける間は `eos_timeout`(5 s)を超えて待ち、増えなくなってから `eos_timeout` 経過で打ち切る。途中で EOS が完了すれば即 true。
2. Heartbeat 系だけが動いても待ち続けない(打ち切る)。
3. **Abort でも第一段を通る**: receiver の `bytes` が増えている間は Stop(強制 EOS)を撃たない。
4. run_stop の新キーが全部載る。欠けの無いモックで `lost_* == 0`・`root_complete == true`、overflow / dropped / eos 未達 / abandoned のそれぞれで `root_complete == false`(4 ケース)、値が取れないときは null。
5. `/api/status` に Stopping 中だけ `stopping` が出る(REST の統合テスト — `tests/controller_integration.rs` の流儀)。
6. 既存の停止・歩き戻しのテスト(`eos_complete` の判定など)が無改変で green。
- 全体: `cargo fmt && cargo clippy --tests -- -D warnings && cargo test` green(テスト数を報告)。

## 3. 作業場所と範囲
- **git worktree で作業する**(184 が本体の作業ツリーで receiver を触るため)。着手時に `git merge --ff-only main` で最新の main(`a5d37e9` 以降)に合わせる。コミットは worktree のブランチにしてよい(主対話がマージする)。
- 触ってよい: `src/controller/`(`sequencer.rs`、`sequencer/tests_*.rs`、`status.rs`、`rest.rs`、`mod.rs`、`test_support.rs`)、ログブックの run_stop を書く箇所(`src/logbook.rs` 等、必要なら)、`tests/controller_*.rs`。
- **触らない**: `src/receiver.rs` / `src/decoder.rs` / `src/graw_writer.rs`(184)、`tools/root_sink/`(186)、`docs/`、`TODO/`、`ui/`(187)。監視タスク(自動異常停止)は **188** で別に作る — ここでは作らない。

## 4. 報告
- worktree のブランチ名とコミット、`git diff --stat`、テスト 1–6 の名前と場所と PASS / FAIL、赤の証拠、cargo のテスト数、**実行できなかった操作**(無ければ「なし」)、発注書からの逸脱と理由、残ったプロセスが無いこと。

## 結果
### 実装(2026-10-30、Opus implementer — 2026-10-24 の前任が中断した作業ツリーを引き継ぎ、worktree `worktree-agent-aa3458c9362209bb6` のコミット `eea2a8e`、親 = main `63d00c3`。主対話 Opus が照合)
- 10 ファイル +983 / −31: `src/controller/{mod.rs, rest.rs, sequencer.rs, sequencer/tests_stop.rs, sequencer/tests_walk_back.rs, status.rs, test_support.rs}`、`src/logbook.rs`、`src/runs_yaml.rs`、`tests/controller_integration.rs`。範囲内(receiver / decoder / graw_writer / tools / docs / TODO / ui は無変更)。
- 中身:
  - 中止の近道(`StopMode::Abort(_) => false`)を撤去 → 中止も受信の静止を待つ。
  - 第二段 = `wait_for_eos_while_downstream_moves`。観測 = receiver `batches`・graw-writer `asad[].frames` 合計 + `ctrl_frames`・decoder `frames_in` / `batches_out`。Heartbeat は入れない。不達 = 動いていない扱い。
  - 打ち切りの note は頭を従来の `EOS did not propagate within <eos_timeout> ms` のまま(p3_error_paths F1 の照合と両立)、後ろに待った総時間と最後に動いてからの秒数を足す。
  - `/api/status.stopping`(`stopping_since` は phase と同じロック区間)。
  - run_stop.counters の新キー 10 個は `Option`(材料が 1 つでも欠ければ null、`lost_*` は i64)。`root_complete` は停止経路と歩き戻しの両方で埋める。
- 受け入れテスト(すべて PASS): `tests_stop.rs` の `after_the_forced_eos_the_stop_waits_as_long_as_the_decoder_keeps_sending` / `a_forced_eos_that_completes_while_the_decoder_still_sends_returns_at_once` / `each_observed_counter_alone_keeps_the_second_stage_waiting` / `heartbeats_alone_do_not_keep_the_stop_waiting` / `an_abort_waits_for_the_receivers_to_go_quiet_before_forcing_eos` / `run_stop_counters_carry_the_link_counts_and_a_complete_run_is_root_complete` / `root_complete_is_false_for_overflow_drop_missing_eos_and_abandon` / `link_counts_are_null_when_a_component_cannot_tell`、`tests/controller_integration.rs` の `the_status_shows_stopping_only_while_the_stop_is_waiting`(後半でログブックの run_stop に新キーが全部載ることを検査)。
- 赤の証拠: 実装を一時的に旧挙動へ戻して FAILED を確認(中止で Stop を即撃ち `left: 0 right: 26`、進捗待ち `5.6s` 対 `13.8s`、`heartbeats_in` を観測に入れると `64.8s` 対 `5.6s`、`stopping` 無しで `Null` 対 `Number(48)` ほか)、cmp で原状復帰を確認。ログは scratchpad の `red/`。
- implementer の全体: `cargo test` 1122 passed / 0 failed / 1 ignored(既存の bench)、clippy 警告 0、fmt clean。env で有効化する e2e は別途実行: `p3_e2e` 5 passed、`p3_error_paths` 2 passed(F0 の停止 1.534 s、F1 12.361 s、note の照合も通る。E2E-D の 2 本は release 専用で debug では自分で SKIP)。
- **主対話の再実行(worktree、2026-10-30)**: 受け入れテスト 9 本の `fn` が `git show HEAD` にあることを確認、`cargo fmt --check` clean、`cargo clippy --tests -- -D warnings` 警告 0、`cargo test --lib controller` 94 passed / 0 failed、`--lib tests_stop` 27 passed、`--test controller_integration` 42 passed / 0 failed。
- 逸脱: ①既存テスト `abort_closes_with_eos_before_resetting_the_decoder` は「中止は待たずに強制 EOS」(v1.6)を固定していたので、前提を `eos_after_polls = Some(4)` に変えた(肝の検査 = decoder の Reset / Stop は EOS を見てから・ecc stop が最初、は残す)②`Counters` の構造体リテラルとログブックの期待 JSON に新キーの null を追加(v1.10 の nullable 規律、意味は不変)。
- 申し送り:
  - **Fable キュー**: 歩き戻し(114)の第二段は今も `wait_for_eos`(通算 `eos_timeout`)。SPEC v1.39 は「停止と同じ 2 段」と書くので、v1.53 の「下流が進む限り待つ」を歩き戻しにも適用するかは設計判断。
  - **188 へ**: 監視タスクが phase を Stopping にするときは `run_state.stopping_since` も同じロックで立てる(立てないと `waited_s` が null)。
  - 範囲外(未変更): `tools/scan/scan.py` の `urlopen(timeout=300)` は分単位の停止で超えうる / `eos_complete` は不達の decoder / graw-writer を覚えず毎回 `command_timeout` を燃やす(既存)— 中止も第一段を通るようになったぶん実時間が延びうる。
- 実行できなかった操作: なし。残プロセスなし。
- 残り: main へのマージ(184 の完了後、186 と一緒に)→ ELITPC への反映は 184–188 をまとめて。
- **マージ後の確認(2026-11-03、主対話、main = 184 + 185 + 186)**: `cargo fmt --check` OK、`cargo clippy --tests -- -D warnings` OK、root_sink `make -j`(警告なし)/ `make -j test`(tpc_wire 68・rs_core 120・eb_core 391・geo 426・monitor_hist 202・monitor_pub 92・ped_table 27、失敗 0)/ `make -j test-root`(recorder 192・pevent 163・raw_tree 119、失敗 0)、`TPCDAQ_ROOT_SINK_BIN=<ビルドした root_sink> cargo test --no-fail-fast` = **1,133 passed / 0 failed / 1 ignored**(2 回目)。1 回目と最初の実行では負荷依存の揺らぎが 1 件ずつ(`receiver_stale_link::eof_after_data_still_ends_the_run` = 184 の DONTWAIT で接続前のフレームを数えて落とす競合、`root_sink_monitor_pub::monitor_root_is_written_within_ten_seconds_of_the_eos` = 10 s の期限)。単独では 10 / 10 通る → 揺らぎのチケット(202)。
