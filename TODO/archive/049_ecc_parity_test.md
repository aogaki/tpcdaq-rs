# 049 — ECC 遷移表パリティテスト(Rust MockTransport ↔ C++ ecc_core)(044 窓)

**Status: COMPLETED**(2026-08-15 — 結果は末尾)
**Status(起票時): READY**(起票 2026-08-15 Fable — 044 テストレーン #6。**統一はしない**)
**発注先想定**: implementer/**Opus**(言語横断のテスト設計判断が残る)

## 背景

ECC 状態機械は **意図的に 2 実装**ある: `src/controller.rs`(テスト用 `MockTransport::
ecc_transition`、:2693-2709)と `tools/ecc_bridge/ecc_core.hpp`(`next_state`、:389-445。
fake_ecc と bridge の正本)。二重化は 036 の事故(甘い Rust ダブルが実バグを green で通した)
に由来する**負荷分散された防御**であり、言語境界 = ZMQ のみの規約からも統一しない(044 裁定)。
残るリスクは**将来の片側だけの修正によるドリフト**。これを CI で捕まえる。

## やること

- **共有 golden 遷移表**を 1 ファイルで定義(置き場・形式は実装判断: 例 `tests/fixtures/
  ecc_transitions.txt` の「state action → 期待(次状態 or Ignored/Denied, error フラグ)」の
  行形式。**コメントで一次資料の出典(BackEnd.cpp 行)を併記** — 出典の正本は
  ecc_core.hpp(048-C)なのでポインタでよい)。
- **Rust 側**: golden 表を読み、`MockTransport::ecc_transition` + error フラグ規則を全行照合
  する単体テスト 1 本。
- **C++ 側**: 同じ表を読み、`ecc::next_state` + error 規則を全行照合するテストを
  `test_ecc_bridge` に追加(表ファイルへの相対パスは env or 引数 — 既存テストの流儀に従う)。
- 表の内容は**現行両実装が一致している範囲の全遷移**(全 state × 全 action)。もし照合で
  **現時点の不一致が見つかったらそれ自体が発見** — 修正せず報告して戻る(裁定は発注側。
  一次資料 BackEnd.cpp が最終審級)。

## 受け入れ

- 既存テスト無変更(追加のみ)。cargo ゲート全 green + `make -C tools/ecc_bridge -j test`
  green(新テスト含む)。
- 表の行数(カバーした (state, action) 組の数)を結果節に記録。両側が**同じファイル**を
  読んでいることが diff で自明であること。

## 結果(2026-08-15 implementer/Opus → 発注側(Fable)レビュー PASS)

- **不一致 0 件**: 全 64 組(8 state × 8 action)で状態遷移・error フラグとも両実装一致。
- 共有 golden 表 = `tests/fixtures/ecc_transitions.txt`(4 列形式、64 行、行数 assert 付き =
  半読 green の防止。表は仕様の正本ではなく、正本ポインタ = ecc_core.hpp → BackEnd.cpp を
  ヘッダ明記)。Rust 1 本(controller tests)+ C++ 1 本(test_ecc_bridge、Makefile で
  同一ファイルを指すことが 1 画面で可視)。
- **red 確認済み(TDD)**: `Ready reset` を「034 の事故そのもの」(Applied:Idle)に書き換えると
  Rust・C++ 両側が FAIL することを実測 → 復元。
- ゲート: cargo **432 passed / 0 failed**(+1)/ fmt・clippy clean /
  `test_ecc_bridge` **457 passed**(+257 = 64 行 × 4 CHECK + 行数 1)。
- **裁定**: ①`Observed` トークン追加 = **受理**(status の表現差 — Rust None vs C++
  Applied-不変 — を観測挙動の同一性で吸収し、「fake_ecc が将来 status を step() 経由に
  すると C++ 側だけフラグが消える」潜在リスクを表とコメントで固定。正しい処理)
  ②action 集合 = `is_known_action` の 8 個 = **受理**(未知 action の Denied は既存
  chirp テストがカバー)。
- 実行環境: macOS Darwin 25.5.0、2026-08-15。

**Status: COMPLETED**
