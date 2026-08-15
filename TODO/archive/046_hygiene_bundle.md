# 046 — S 級ハイジーン束(A〜D 各項独立・挙動不変)(044 窓)

**Status: COMPLETED**(2026-08-15 — 結果は末尾)
**Status(起票時): READY**(起票 2026-08-15 Fable — 044 レビュー C2/C5 + 033 裁定③ + 相乗り 1 件)
**発注先想定**: implementer/**Opus**(4 ファイル横断 + borrow 分割の判断が 1 箇所)

## A. geometry `lookup_ref`(044-C2 — 026 申し送りの正しい閉じ方)

- 事実: `src/geometry.rs:165-179` の `lookup` が **item(サンプル)毎に `ChannelRole` を
  clone**(1 イベント ≈ 557k 回)。`Aux{name: String}` を持つため、**AUX 行入りジオメトリ
  (ELITPC 想定)を積んだ瞬間 per-sample malloc が発火**する。現行 mini .dat は AUX 0 件。
- やること: `pub fn lookup_ref(&self, ...) -> &ChannelRole` を追加(`unmapped_hits` 加算は
  こちらへ)。`lookup` は `lookup_ref(..).clone()` の薄いラッパで**存置**(既存テスト 40+ 箇所
  無変更のため)。production 唯一の呼び出し元 `src/monitor.rs:596-601`(`accumulate_uvw`)を
  `lookup_ref` + 参照束縛に切替。borrow は `self.geometry` と `self.counts`/`self.grids` の
  **フィールド分割借用で通る**(レビュー確認済み)。

## B. graw_writer の `.expect()` 4 箇所 → counted error(044-C5)

- 対象: `src/graw_writer.rs:559, :566, :600, :621`(production 全体で残る panic 起点は
  この 4 つだけ — レビュー全走査済み)。
- やること: `let .. else { self.write_errors += 1; self.errored = true; return Err(..) }` 型へ
  (到達不能分岐を「黙って落ちる」から「数えて報告」に)。`metrics_json` の形は不変。
  よりによってロスレス保存系に panic 起点がある状態の解消(panic → Mutex 毒 → run 残り
  全損の芽)。

## C. `eos_quiesce_ms = 0` の拒否 validation(033 裁定③)

- 0 は「即・強制 EOS」= 033 裁定が明示的に不採用とした挙動。`eos_timeout_s = 0` 拒否の
  既存前例(config.rs)と同じ形で起動時エラーに。**新規テスト 1 本**(0 が拒否される)。
  既存テストは無変更。

## D. `bind_pull` の共通化(044 レビューの唯一の相乗り推奨)

- `src/decoder.rs:1051-1069` と `src/graw_writer.rs:933-951` の 18 行同一を
  `src/zmq_helper.rs` へ抽出。receiver 等の他実装(意味が違うもの)は触らない。

## 受け入れ(共通)

- **既存テストを 1 文字も変えずに**(C の新規 1 本を除き追加のみ)`cargo fmt && cargo clippy
  --tests -- -D warnings && cargo test` 全 green。実 graw 回帰も green。
- 結果節: 項目毎の diff 要旨 / テスト数(before 430 → after)/ 環境と日付。

## 結果(2026-08-15 implementer/Opus → 発注側(Fable)レビュー PASS)

- **ゲート**: cargo test **431 passed / 0 failed / 1 ignored**(before 430 → +1 = C の新規
  テストのみ。既存テスト無変更)。fmt / clippy(--all-targets 含む)clean。実 graw 回帰
  全 green(decoder 108/15,040,512・malformed=0 / writer バイト一致 30,108,684 B /
  ELITPC 3852×4・ローテーション境界一致。ELITPC 実 .dat のみ従来どおり SKIP)。
- **A**: `lookup_ref` 新設(unmapped_hits 加算移設 + static UNMAPPED)。`lookup` は 1 行
  ラッパで存置 = geometry テスト 40+ 無変更。monitor.rs はフィールド分割借用で素直に通過。
  per-sample clone/malloc の芽を解消。
- **B**: `.expect()` ×4 → `WriteError::Internal` + `internal_error()` ヘルパで counted error 化
  (write_errors/errored/error! は既存 IO 失敗と同一経路、metrics_json 不変)。
  **production の panic 起点ゼロ**。
- **C**: `eos_quiesce_ms=0` 拒否を config.rs の既存前例と同形・同置き場に。TDD(red 実測 →
  green)。
- **D**: `zmq_helper::bind_pull` 共通化(decoder/graw_writer 各 18 行 → 2 行。info! は
  コンポーネント名/SPEC 節が違うため呼び手に残置 — 妥当)。
- **逸脱の裁定**: ①`WriteError::Internal` 新設 = **受理**(到達不能の内部不整合に偽の
  io::Error を合成しない — 意味的に正しい。exhaustive match 不在確認済み)
  ②counted 処理のヘルパ化 = **受理**(8 行×4 の重複回避、中身は発注書どおり)
  ③A のテスト非追加 = 指示どおり。
- 実行環境: macOS Darwin 25.5.0、2026-08-15。

**Status: COMPLETED**
