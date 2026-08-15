# 043 — ECC エラーフラグの可視化(status 応答の `ecc_error`、SPEC v1.14)

**Status: READY**(起票 2026-08-15 Fable — 041 D-1 の発見②。**P4 Run 制御 UI 実配線の前提**。
033/031 と独立、発注はユーザー合図待ち)
**仕様**: SPEC v1.14 §8.2(`ecc_error` の項 — 適用済み)
**根拠**: [archive/041_integrated_demo.md](archive/041_integrated_demo.md) 結果 D-1 —
実 ECC は describe 失敗後 `IDLE / WHEN_DESCRIBE` を抱えるが、`/api/status` の ecc は
`{"ok":true,"state":"Idle","error":""}` でオペレータから不可視。
**発注先想定**: implementer/**Opus**(一次資料の確認を含むため)

## やること

1. **一次資料の確認から**: 実 GET ソース(`reference/20190315_patched/GetBench/src/get/rc/`)で
   error フラグ(`NO_ERR`/`WHEN_DESCRIBE`/…)の **set/clear 規則**を確認する
   (どの遷移で立ち、何で消えるか)。テストダブルのコメントを根拠にしない。
2. ecc-bridge: `{"action":"status"}` 応答に `ecc_error`(GET error の文字列)を追加。
   既存 `error`(輸送層)の意味は変えない。
3. fake_ecc: 1. で確認した規則に**実機準拠**で追随(036 の流儀 — 甘いダブルは誤実装を
   green で通す)。
4. controller `/api/status`: `ecc_error` を素通しで載せる。UI 表示は**しない**(P4)。
5. 検証: test_ecc_bridge / ecc_e2e に error フラグの set/clear ケースを追加。
   可能なら実 ECC(`reference/_spike/prefix`)相手のスポット確認を結果節に記録。

## 受け入れ

- cargo fmt/clippy/test 全 green + C++ テスト green(make -j)。
- 既存フィールドの意味・値は不変(後方互換)。
- 結果節: set/clear 規則のソース行参照 / テスト数と green / 環境と日付。
