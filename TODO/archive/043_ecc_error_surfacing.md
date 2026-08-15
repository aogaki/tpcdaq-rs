# 043 — ECC エラーフラグの可視化(status 応答の `ecc_error`、SPEC v1.14)

**Status: COMPLETED**(2026-08-15 implementer/Opus → 発注側(Fable)レビュー PASS)

## 結果

- **全ゲート green(2026-08-15、macOS Darwin 25.5.0 arm64)**: `cargo test` **417 passed /
  0 failed / 1 ignored**(043 前 415)/ fmt・clippy クリーン / C++ `test_ecc_bridge`
  **200 passed**(前 187)/ `ecc_e2e` **52 passed**(前 29 — 旧版を別ビルドして実測比較)/
  UI **127 passed**(前 126。新テストは**一度わざと赤にして**から緑化)。
- **set/clear 規則を一次資料の行番号で確定**(ecc_core.hpp に出典コメントとして固定):
  enum 10 値 = `SM.h:58-70` / clear = **全 11 遷移の 1 番目のアクション**(`BackEnd.cpp:249-290`,
  `:1146-1149`)= 遷移を渡り始めた瞬間に消える / `Ignored`・ST_PREPARED ガードでは**残る** /
  set = 各 on* の末尾で相ごとのコード(describe `:808` 〜 breakup `:1143`、
  unDescribe は WHEN_RESET)/ **失敗時 state は動かない**(`SM::Exception` は std::exception
  非継承で dhsm に捕まらない — 041 D-1 の `IDLE/WHEN_DESCRIBE` の機序が判明)/
  getStatus で消えない / ワイヤは元から `Status{s, e}` の 2 フィールド(**我々が e を
  捨てていただけ**)。
- **実装**: ecc_bridge 応答に `ecc_error`(UPPER_SNAKE、GET の綴りと一致)/ fake_ecc は
  実機規則準拠(Applied のときだけ先に clear、Ignored/Denied は触らない)/ controller
  `/api/status` に素通し(不達時 `"Unknown"`)/ MockTransport も同規則。
- **実 ECC スポット確認 = 041 D-1 再現**: describe 失敗 → `ecc_error:"WHEN_DESCRIBE"`、
  status 2 回読んでも不変、遷移なし reset でも不変、getHwServer 起動後の describe 成功で
  `NO_ERR`(clear 実証)、再度落として prepare → `WHEN_PREPARE`(相コード実証)。
  GET 純正 `getEccClient sm-status` と綴りまで一致。
- **逸脱の裁定(発注側)**: ①`ecc_error` を全 action 応答に搭載 = **受理**(bridge は毎回
  getStatus を打つので追加コストゼロ、失敗応答でそのまま読めて有用。既存 3 キー不変)
  ②不達時 `"Unknown"` = **受理**(NO_ERR は嘘になる — state と対称)③`/api/ecc/{action}`
  応答への搭載は見送り = **受理**(P4 で必要になったら 1 行)④fake の能動フラグは
  WHEN_START のみ = **受理**(他相は実 ECC で担保。失敗注入機構は必要になってから)。
- **発見(記録のみ → CURRENT.md 留意)**: 実 ECC の `GetEccImpl::breakup` は SM::Exception を
  catch せず Ice UnknownException になり、`onUnPrepare` は CATCH 無しで dhsm が halt
  (我々の map では `Off` に見える)— P4/P5 の運用手順に影響し得る。
- 実行環境・日付: macOS Darwin 25.5.0(arm64)、2026-08-15。

---

(以下、起票時の発注書)

**Status(起票時): READY**(起票 2026-08-15 Fable — 041 D-1 の発見②。**P4 Run 制御 UI 実配線の前提**。
033/031 と独立)
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
