# 185 — controller の停止: 中止も静止を待つ、強制 EOS の後は「下流が進んでいる限り待つ」、run_stop にリンクごとの送受と `root_complete`

**Status: IN PROGRESS**(2026-10-24 起票 Opus、実装 **Opus implementer**。設計 = TODO/181 §5-1「§4-2」(Fable レビュー済み)、仕様 = SPEC v1.53 §1.3(停止シーケンス 2・異常中止 ①・run 単位の照合)/ §1.4-8 / §9.2「v1.53 run_stop.counters の追加キー」)

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
(記入中)
