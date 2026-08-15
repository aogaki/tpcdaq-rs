# 051 — 状態・台帳の表示強化(P4: ecc_error / forced_eos / config_ids)

**Status: COMPLETED**(2026-08-15 — 結果は末尾)
**仕様**: SPEC v1.14 §8.2(`ecc_error`)/ §9.2(run_stop の `forced_eos`/`eos_closed` と
v1.14 注記、`config_ids`)
**発注先想定**: implementer/**Sonnet**(表示のみ。発注書とテストで縛れる)

## やること

1. **status バーに `ecc_error`**: `NO_ERR` 以外のとき表示(例: `Idle (WHEN_DESCRIBE)`)。
   `Unknown`(ブリッジ不達)は既存の不達表示と整合させる。
   **`Off` state の注記**: 実 ECC の unPrepare 失敗は halt = 我々の map で `Off` に見える
   (043 発見 — CURRENT.md 保留節)。`Off` 表示に「ECC 停止または halt — 実機では
   getEccServer の再起動が必要な場合あり」程度のツールチップを付ける。
2. **logbook の run_stop 表示**: `eos_closed:false` を**明確な異常**として強調表示。
   `forced_eos:false` を**警告**として表示(v1.14 注記どおり「stop 前にリンクが死んだ
   可能性」の文言 — 実機 TCP flow では true が常態)。旧行(フィールド欠落)は無印
   (「記録なし」≠ false — 033-A の意味論)。
3. **run_start の `config_ids`**: 存在するとき(3 相非同値)のみ 3 相を表示。無ければ
   従来どおり `config_id` のみ。

## 受け入れ

- `ng test` 全 green(新規: 3 項目それぞれの表示分岐 — 値あり/なし/旧行)。
  `npm run build` 成功・初期バンドル予算内。既存テスト無変更。
- Rust/C++ 非接触。
- 結果節: 表示分岐のテスト一覧 / スクリーンショット相当の記録(テキストで可)。

## 結果(2026-08-15 implementer/Sonnet → 発注側(Fable)レビュー PASS)

- **ng test 186 passed / 5 skipped**(171 + 新規 15)。`npm run build` 成功・初期 501.67 kB
  (不変)。Rust/C++ 非接触。
- **1(ecc_error)**: NO_ERR 以外のみ warn 色で表示。**不達センチネル `Unknown` は既存の
  不達表示に一本化**(二重表示しない — 発注文言により忠実な形への自己修正 = 受理)。
  Off にのみ halt 注記ツールチップ。
- **2(run_stop)**: `eos_closed:false` = error 強調(行全体)/ `forced_eos:false` 単独 =
  attention 強調 + 「stop 前にリンクが死んだ可能性」/ **旧行(欠落)= 無印**(記録なし ≠
  false — 033-A の意味論を表示でも遵守)。優先順位 error > attention(受理)。
- **3(config_ids)**: 非同値時のみ 3 相表示、同値は従来どおり 1 行。
- golden は `src/logbook.rs` の実テスト文字列を転記(Rust 側との整合を担保)。
- 043 placeholder テストの更新は意図された継続点(「表示は P4」と明記済み)= 破壊ではない。
- 実行環境: macOS Darwin 25.5.0、2026-08-15。

**Status: COMPLETED**
