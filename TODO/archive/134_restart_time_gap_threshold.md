# 134 — 取得の再開の判定に「時刻の後退幅」の閾値を足す(偽陽性で新しいイベントを捨てる穴を塞ぐ)

**Status: COMPLETED(2026-10-14、結果は末尾)** — 旧: READY(2026-10-14 起票 Fable。110 完了時の批判的レビューで発見。小さいが、起きると ROOT からデータを**意図して**捨てる経路なので Warsaw(4 AsAd)前に必須。Sonnet)

## 0. 穴(SPEC v1.46 §6.3 の判定、`tools/root_sink/eb_core.hpp` `feed`)
- 判定は `event_idx == 0 && (complete か emit 済み) && max_seen > 8 && event_time < time_floor`。
- 「complete か emit 済み」が守るのは event 0 が **pending の間**だけ。4 AsAd で event 0 が build timeout(既定 1 s)で **incomplete のまま emit された後**、
  遅れた板の event 0 フラグメントが僅かに小さい `event_time`(板の時刻ずれ、µs)で来ると、`have_emitted` が真なので再開と判定する。
  → pending を flush、高水位を消し、`retired` に 0..k が入る → **以後の新しい 1..k が `restart_duplicates` として ROOT から捨てられる**。
- 本物の再開は時刻が数十秒戻る(実機 run 73: 79.6 s → 0.28 s)。板のずれは µs。**後退幅で区別できる**。
- decoder(`src/decoder.rs`)には complete/emit の守りすら無い(`max > 8 && time < floor` のみ)。数えるだけなので実害は台帳の誤記だが、同じ閾値を入れて両層の定義を揃える。

## 1. 仕様
- 両層に定数 **`RESTART_MIN_TIME_GAP` = 10,000,000 ticks(= 100 ms。CoBo の event_time は 10 ns 刻み — 073 #7 / 110 の graw ヘッダ解析で確認)** を足し、判定を
  `event_time + RESTART_MIN_TIME_GAP < time_floor` にする(他の条件は不変)。定数のコメントに根拠(板のずれは µs、本物の再開は秒)を書く。
- SPEC v1.46 §6.3 / §2.3 の判定文に「かつ、その epoch の最小 event_time より 100 ms 以上前」を足す文案を報告に書く(SPEC の編集は主対話)。

## 2. テスト(TDD、red を先に実測)
- `tools/root_sink/test_eb_core.cpp`:
  - `test_a_late_event_zero_fragment_after_an_incomplete_emit_with_a_slightly_earlier_clock_is_late_not_a_restart`:
    4 AsAd。AsAd 0–2 の event 0..9 を feed → build timeout 経過で poll → event 0..9 が incomplete で emit される → AsAd 3 の event 0 が `time_floor − 100`(1 µs 早い)で来る
    → `acquisition_restarts == 0`、`late_fragments == 1`、`restart_duplicate_events == 0`。その後の event 10..20(4 AsAd 完全)が complete で出ること(= 捨てられていない)。
    **現行コードではこのテストが赤になる(restarts 1、以後が dup)ことを報告に書く。**
  - `test_a_genuine_restart_tens_of_seconds_back_is_still_detected`: 既存の受け入れ列(古い 2038..2056 の time 406e6..409e6 → 新 0 の time 16e3)が引き続き restarts 1。
    後退幅がちょうど閾値(`time_floor − RESTART_MIN_TIME_GAP`)では検出せず、それより 1 小さければ検出する境界も 1 本。
- `src/decoder.rs`: `a_small_clock_skew_on_event_zero_is_not_a_restart`(idx 0..9 の後に idx 0 が floor − 100 で来る → restarts 0)と、境界 1 本。既存 4 本は不変。
- 既存テスト(eb_core 272、decoder、recorder、raw_tree)を落とさない。

## 3. ゲート
`cd tools/root_sink && make -j test && make -j test-root`(Mac は `source /opt/ROOT/bin/thisroot.sh && export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX27.sdk`、worktree では `TPCDAQ_TPCRECO_DIR=/Users/aogaki/Workspace/tpcdaq-rs/reference/TPCReco/TPCReco-HIGS2026_online`)、`cargo fmt && cargo clippy --tests -- -D warnings && cargo test`。

## 4. 報告に必ず書くこと
変更ファイル、テスト名、赤の実測(落ちた CHECK / assert)、ゲートの総数、SPEC 文案、**実行できなかった操作**(無ければ「なし」)。

## 参照
SPEC v1.46 §2.3 / §6.3、archive/110、133(4 AsAd の送出順 — こちらは設計が要る。134 はそれと独立に今入れる)。

## 結果(2026-10-14、Sonnet `f2f06af` → merge `49e4b01`、Fable 照合)
- 両層に閾値 100 ms(`kRestartMinTimeGap` / `RESTART_MIN_TIME_GAP` = 10,000,000 ticks)。判定は `event_time + gap < time_floor`(decoder は saturating_add)。
- 新規テスト: C++ `test_a_late_event_zero_fragment_after_an_incomplete_emit_with_a_slightly_earlier_clock_is_late_not_a_restart` / `test_a_genuine_restart_tens_of_seconds_back_is_still_detected`(境界: ちょうど閾値は 0、1 tick 超で 1)、Rust `a_small_clock_skew_on_event_zero_is_not_a_restart` / `the_restart_time_gap_boundary_is_exclusive`。
- **赤の実測(閾値なし)**: C++ 1 本目で `restarts == 0 (got 1)`、`late == 1 (got 0)`、**`out.size() == 11 (got 0)`** = 以後の event 10..20 が 1 件も出ない(発注書の予想「dup で捨てる」とは症状が違い、偽の新世代の incomplete な event 0 の後ろで head-of-line が詰まる — どちらにせよ新しい取得が出てこない)。279 passed / 5 failed。Rust 2 本は restarts 1 で赤。
- 既存テストの時刻データ 28 件を閾値より大きい後退に直した(期待値は不変、実機の後退は秒〜数十秒)。
- ゲート: `make -j test` eb_core **284** ほか全 passed、`make -j test-root` 174 / 99 / 91、cargo fmt / clippy / test green(lib 761)。
- SPEC v1.47(§2.3 / §6.3)。
- 実行できなかった操作: `source thisroot.sh` が worktree 隔離で拒否 → 環境変数直指定で test-root を実行(結果に影響なし)。

**Status: COMPLETED(2026-10-14)**
