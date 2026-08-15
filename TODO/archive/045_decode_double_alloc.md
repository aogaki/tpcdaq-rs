# 045 — decode.rs のフレーム毎二重確保の解消(044 窓 / 最優先)

**Status: COMPLETED**(2026-08-15 — 結果は末尾)
**Status(起票時): READY**(起票 2026-08-15 Fable — 044 レビュー C1)
**発注先想定**: implementer/**Sonnet**(単一ファイル・発注書とオラクルで完全に縛れる)

## 事実(レビュー実測)

`decode_items`(`src/decode.rs:195-234`、呼び出し元は :147 の 1 箇所のみ)が `Vec<u32>` を
確保 → `items_to_bytes`(`src/msg.rs:221`)が同内容を `Vec<u8>` に**再確保 + 全コピー** →
`Vec<u32>` 即破棄。実データで 1 フレーム = 139,264 item = **フレーム毎 557 KB の余分な確保 +
557 KB の memcpy**。100 Hz × 4 AsAd(ELITPC)で **~223 MB/s の無駄**。
CLAUDE.md「ホットパスで per-frame の heap 確保をしない」に抵触。

## やること

- `decode_items` が最初から `Vec<u8>`(capacity = `item_count * 4`)に `word.to_le_bytes()` を
  直接書く形へ。`self.items` の加算は `bytes.len() / 4`。
- `pack_item` の範囲エラー → `None` → malformed 計上の経路は**そのまま**。
- 出力バイト列は現行(`items_to_bytes` の LE u32 連結)と**完全同一**であること。
- `items_to_bytes` 自体は他の利用者がいれば残す(いなければ削除可 — grep で確認して判断を記録)。

## 受け入れ

- **既存テストを 1 文字も変えずに** `cargo fmt && cargo clippy --tests -- -D warnings &&
  cargo test` 全 green。特に実 graw オラクル(`decoder_real_graw` / `elitpc_real_graw`、
  events=108 / items=15,040,512)無変更 green(env: `TPCDAQ_REAL_GRAW=~/TPC/CoBo_2025-09-01T08_51_06.203_0000.graw`、
  `TPCDAQ_REAL_GEOMETRY_MINI=~/TPC/miniTPC_UVW_pcb_info/new_geometry_mini_eTPC.dat` 相当 —
  実際の変数名はテスト内の SKIP メッセージで確認)。
- **before/after の実測**を結果節に記録(実 graw 1 ファイルのデコード所要 — 簡便な計測で可、
  計測コードはコミットしない)。

## 結果(2026-08-15 implementer/Sonnet → 発注側(Fable)レビュー PASS)

- 変更 = `src/decode.rs` のみ(git diff で確認)。`decode_items` が `Vec<u8>`(capacity =
  item_count×4)を直接構築、中間 `Vec<u32>` と `items_to_bytes` 経由の再確保+コピーを撤去。
  malformed 経路・出力バイト列(LE u32 連結)は完全同一。
- ゲート: fmt 差分なし / clippy 警告ゼロ / cargo test 全 green(実 graw オラクル
  `frames=108 / items=15,040,512` 個別確認、ELITPC 回帰・下流パイプライン実データ系も green)。
  **既存テスト 1 バイトも無変更**。
- **実測: 29.70 ms → 21.99 ms(約 26% 短縮、実 graw 1 ファイル全デコード、release、
  best of 5)**。計測コードは非コミット。
- `items_to_bytes` は monitor.rs + 統合テスト 5 箇所が利用中のため存置(発注書どおりの判断)。
- 逸脱なし。実行環境: macOS Darwin 25.5.0、2026-08-15。

**Status: COMPLETED**
