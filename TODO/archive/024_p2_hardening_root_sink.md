# 024 — P2 レビュー改修: root-sink C++ + spawn テストインフラ

**Status: COMPLETED**(2026-08-14 implementer/Sonnet(worktree)→ Fable レビュー PASS →
main 取り込み)

## 結果

- **実装**(7 ファイル +526/-96): `kExitRunMismatch = 6` — consume() の Data/EOS 両経路で
  `run_number_mismatch()` の増分検知 → fatal(**転写を fatal 直前に移す副次バグ修正つき —
  `_Exit` で戻らないため旧位置では終了 JSON に乗らなかった**)/ Recorder 混在 run 防御を
  finalize=false + PROTOCOL VIOLATION 明示(到達不能になった最後の砦)/
  `maybe_autosave()` を write()/tick() 共通化(R-P2-2)/ Status `pending_events`
  (11 キー目)+ `kPendingWarnThreshold=1000` 純関数 + 一度だけ warn(R-P2-5)/
  テストインフラ: SinkGuard(Drop kill)+ spawn 早期死リトライ(exit 4 検知・最大 3 回・
  eprintln 可視)+ intake 全 spawn に `--pub` 空きポート明示。
- **テスト(worktree でエージェント実行 + Fable が main 取り込み後ゲートで裏取り、
  2026-08-14、macOS Darwin 25.5.0 / ROOT 6.36.10)**:
  `make test` 全 green(monitor_pub 87→**92**)、`make test-root` 全 green
  (recorder 216→**233**。pevent 構造一致は TPCDAQ_REAL_PEVENT 未設定 SKIP — 既存どおり)、
  fmt / clippy クリーン、cargo test 全 green。新規: C++ 混在 run 防御(inprogress 残置)/
  write() 単独 AutoSave(peek_tree_entries ヘルパ — 読み専 TFile で GetEntries のみ)/
  閾値純関数。Rust: run 混在 Data / EOS 各 → **exit 6** + JSON run_number_mismatch 照合。
  **intake + monitor_pub の 3 回連続 0 fail**(+ 実装過程で 15 回以上 green =
  TOCTOU flake の構造的解消)。
- **main 取り込み後(Fable 実測)**: make test / test-root 全 green、intake+monitor
  3 回連続 **17/17**、フルゲート(実データ + ecc env)**337 passed / 0 failed**。
  取り込み時の教訓 2 件: ①`make test`/`test-root` は root_sink **本体**を再ビルドせず、
  stale バイナリで新テスト 2 本が偽 red になった(Fable が test-root に root_sink 依存を
  追加して恒久化 — Makefile 1 行)。②全スイート並列の 1 ラウンドで名前不明の 1 fail が
  単発発生(直後 2 周は 0 fail・名前捕捉できず — 再発したらフルログで採取)。
- **レビュー(Fable)**: 判断 3 件受理 — AutoSave 検証の読み専 TFile 方式 / mismatch 判定を
  on_data/on_eos 直後に置く(stale EOS・期待外 source は RunState 実装上 mismatch を
  増分させないことを確認済み)/ idle status テストの観測窓 2.5→4 s(spawn 200 ms 猶予の
  直接的副作用への最小是正、閾値は不変)。worktree は main へ ff 同期して作業
  (自コミットなし)、TPCDAQ_TPCRECO_DIR は Makefile 既定のオーバーライド機構を使用
  (無改変)。
**仕様**: SPEC **v1.10** §6.2-5(run_number 食い違い = fatal exit 6)/ §5.3
(status `pending_events` + 警告閾値 1000)。所見の詳細 = [P2_REVIEW.md](P2_REVIEW.md) の
R-P2-1 / R-P2-2 / R-P2-5、および archive/022 結果節「既知の申し送り」(free_endpoint
TOCTOU)。
**発注先想定**: implementer/**Sonnet**(発注書とテストで縛れる)

## やること

1. **R-P2-1 run_number 食い違いを fatal に昇格**(root_sink.cxx、SPEC §6.2-5 v1.10):
   - `kExitRunMismatch = 6` を追加。`consume()` で `run.on_data` / `run.on_eos` の後に
     `run_number_mismatch()` の**増分**を検知したら
     `fatal(c, 6, "run-number-mismatch", detail)`(detail に source_id / 開いている run /
     来た run)。idle 中 stale EOS・期待外 source は従来どおり fatal にしない
     (RunState 自体は無改変 — 計数はそのまま)。
   - root_recorder.hpp の混在 run 防御分岐(`write()` の「finalizing the old one」)は
     **finalize=false に変更** + stderr を「protocol violation upstream should have been
     fatal」旨に(上流 fatal 化で到達不能になるが、防御は「完成 run に化けない」側に倒す)。
2. **R-P2-2 AutoSave 飢餓の解消**(root_recorder.hpp): `write()` 内でも
   `last_autosave_ms_` の期限を見て AutoSave する(現在は呼び手の tick() 頼みで、
   データが途切れない run では一度も走らない)。
3. **R-P2-5 status に `pending_events`**(SPEC §5.3 v1.10):
   - `rsmon::Status` にフィールド追加、Encoder の status map を 10 → 11 キーに
     (キー名 `pending_events`、`late_fragments` の次)。`update_status_material` で
     `c.pending_events` をコピー。
   - 警告閾値: `kPendingWarnThreshold = 1000` 超過で**一度だけ** warn(判定は純関数に
     切って単体テスト。022 の logged-once 方式)。
4. **spawn テストインフラ(022 結果節の申し送り)**:
   - tests/root_sink_intake.rs / tests/root_sink_monitor_pub.rs の sink 起動を
     **早期死リトライ**化: spawn → ~200 ms 内に exit code 4(zmq bind 失敗 =
     free_endpoint TOCTOU)で死んでいたら新ポートで再スポーン(最大 3 回、リトライは
     eprintln で可視)。
   - intake の各 spawn に **`--pub` を空きポートで明示指定**(既定 47004 の並列 bind
     競合ノイズを消す)。
   - monitor_pub 側の `Sink` **Drop ガード(panic 時に子を kill)を intake へ横展開**。
5. Rust 統合テスト: run_number 混在(Data / EOS 各 1 本)→ **exit code 6** + 終了 JSON
   `run_number_mismatch` ≥ 1 の機械照合。status の `pending_events` フィールドが
   パースできること(値は ≥ 0)。

## テスト(TDD)

- C++: test_recorder に「tick() を呼ばず write() 連打で now_ms が 30 s を跨ぐ →
  AutoSave が走る」(既存 tick-AutoSave テストと同じ観測方法)+ 混在 run 防御分岐が
  inprogress のまま残すこと。test_monitor_pub に status 11 キー(pending_events)の
  バイト照合 + 閾値純関数の単体。
- Rust: 上記 5(exit 6 ×2 経路、pending_events パース)+ 既存 9 本が並列で green
  (リトライ導入後、TOCTOU flake が構造的に消えることの確認 — スイートを 3 回連続
  実行して 0 fail を結果に記録)。

## 受け入れ

- `make -C tools/root_sink test` / `make test-root` 全 green。
  `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test` 全 green。
  `TPCDAQ_ROOT_SINK_BIN=… cargo test --test root_sink_intake --test root_sink_monitor_pub`
  を 3 回連続 0 fail。
- ファイル所有権: tools/root_sink/{root_sink.cxx, root_recorder.hpp, monitor_pub.hpp,
  test_monitor_pub.cpp, test_recorder.cxx} + tests/{root_sink_intake.rs,
  root_sink_monitor_pub.rs}。**rs_core.hpp・monitor_hist.hpp・Rust src/ に触らない**。
  発注書に無い設計分岐に出会ったら実装せず報告して戻る。
