# 186 — root_sink: EOS が来ないまま次の run が来たら、古い run を未完のまま閉じて新しい run を開く

**Status: IN PROGRESS**(2026-10-24 起票 Opus、実装 **Sonnet implementer**。設計 = TODO/181 §5-1「§4-3」(Fable レビュー済み)、仕様 = SPEC v1.53 §6.2-5 / §6.5 / §1.4-5 / §1.3 終端条項)

## 0. 背景
- 今は run 開放中に別の run_number の Data(または EOS)が来ると **fatal(exit 6 `run-number-mismatch`、`root_sink.cxx:808–823`)**。run 197 のように EOS を失った run の後で次の run を始めると root_sink が死に、次の run の ROOT が丸ごと無くなる。
- ユーザー裁定: EOS を失った run は「**未完であることが分かればよい**」。未完の印 = ファイル名が `run_inprogress_*` / `run_raw_inprogress_*` のまま(SPEC §6.5 の既存の規則)。

## 1. 変更(決定済み)
### A. `tools/root_sink/rs_core.hpp` の `RunState`
- `on_data`: run が開いていて `run_number` が違う → **新しい値 `DataAction::Reopened`** を返す(古い run を閉じて新しい run を開け、の意味)。`run_number_mismatch` は数えない。内部状態は新しい run で開き直す(`run_number_` 更新、`eos_` クリア)。`runs_closed_without_eos` を +1(新カウンタ)。
- `on_eos`: run が開いていて `run_number` が違う → 古い run を未完で閉じる指示(新しい値、例 `EosAction::ClosedIncomplete`)を返し、その EOS 自体は `stale_eos` として数えて無視(新しい run は開かない)。`runs_closed_without_eos` +1。
- 同じ run_number の Data の seq gap・malformed は**従来どおり fatal**(変えない)。
### B. `tools/root_sink/root_sink.cxx` の `consume`
- `Reopened` のとき: EventBuilder の pending を古い run に吐き出す(今の run 終了と同じ flush の手順 — incomplete は数える)→ Recorder / raw writer に「**未完で閉じる**」項目を流す(in-band、今の `RunClose` と同じキューの順序で)→ close 行 `root_sink: run N closed without EOS (incomplete) (…今の close 行と同じカウンタ…)` → 新しい run を `Opened` と同じ手順で開く(SeqCheck の reset、ヒストの reset、`run_at_open` の更新、`run N opened` のログ)→ そのバッチを新しい run として処理。
- `ClosedIncomplete`(別 run の EOS)のとき: 同じく古い run を未完で閉じる(新しい run は開かない)。
- exit 6 の `fatal(… kExitRunMismatch …)` 経路を撤去(定数は残してよい)。
### C. 未完で閉じる(Recorder・raw writer)
- `root_recorder.hpp` / `raw_tree.hpp` に「未完で閉じる」操作: 開いているファイルを **finalize せずに(inprogress の名前のまま)閉じ**、次の run を書ける状態に戻す(既存の `shutdown()` = `close_part(finalize=false)` に近いが、プロセスは続くので writer を再利用可能にする)。
- 未完の run には **`run{N}_monitor.root` を書かない**(SPEC §6.5 v1.53)。
- worker 並列(`--recorder-workers N>1`)でも同じ(全 worker が inprogress のまま閉じる)。
### D. run 内の `fragments`
- `RunCounts` に `fragments`(その run で受け取ったフラグメント数)を足し、close 行(`format_run_closed`)と status PUB(§5.3 の status JSON)に `fragments` を出す。未完の close 行にも出す。
- status JSON と終了 JSON に `runs_closed_without_eos` を出す。

## 2. テスト(受け入れ — TDD: 先に赤を確認し、報告に赤の証拠)
1. (rs_core 単体)run N 開放中に run N+1 の Data → `Reopened`、`runs_closed_without_eos == 1`、`run_number() == N+1`、mismatch は数えない。
2. (rs_core 単体)run N 開放中に run N+1 の EOS → 未完で閉じる指示、`stale_eos` +1、run は閉じた状態。
3. (Recorder / raw の単体 — `test_recorder.cxx` / `test_raw_tree.cxx`)未完で閉じる → ファイルは `run_inprogress_*` / `run_raw_inprogress_*` のまま、中身は読める(TTree の entries が書いた数)、monitor.root は無い、その後の run N+1 は通常どおり書けて EOS で finalize される。
4. (e2e — Rust の `tests/root_sink_*.rs`、実バイナリ。今の exit 6 のテストがあれば**置き換える**)run N に数事象(EOS なし)→ run N+1 に数事象 + EOS → root_sink は**落ちない**、run N は inprogress のまま・monitor.root 無し、run N+1 は `run{N+1}.root` / `_raw.root` / `_monitor.root` が揃う、close 行に `closed without EOS (incomplete)`。`--no-root --raw-tree` でも同じ。
5. close 行と status の `fragments` が送ったフラグメント数と一致。
- 全体: root_sink の `make -j`、`make -j test`、`make -j test-root` が 0 failed、`cargo fmt && cargo clippy --tests -- -D warnings && TPCDAQ_ROOT_SINK_BIN=<ビルドした root_sink> cargo test` green。

## 3. 作業場所と範囲
- **git worktree で作業する**(184 が同時に本体の作業ツリーで Rust を触るため)。着手時に `git merge --ff-only main` で最新の main(`a5d37e9` 以降)に合わせること。
- root_sink のビルドに要る TPCReco は gitignore の `reference/` にあり worktree には無い → 主チェックアウトのものを絶対パスで渡す:
  `make TPCDAQ_TPCRECO_DIR=/Users/aogaki/Workspace/tpcdaq-rs/reference/TPCReco/TPCReco-HIGS2026_online -j`(`test` / `test-root` も同じ)、`export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX27.sdk`。
- 触ってよい: `tools/root_sink/`(`rs_core.hpp`、`root_sink.cxx`、`root_recorder.hpp`、`raw_tree.hpp`、テスト)、`tests/root_sink_*.rs`(e2e)。**触らない**: `src/`(184)、`docs/`、`TODO/`。コミットは worktree のブランチにしてよい(主対話がマージする)。

## 4. 報告
- `git diff --stat`(worktree のブランチ名とコミット)、テスト 1–5 の名前と場所と PASS / FAIL、赤の証拠、C++ / cargo のテスト数、**実行できなかった操作**(無ければ「なし」)、発注書からの逸脱と理由、残ったプロセスが無いこと。

## 結果
(記入中)
