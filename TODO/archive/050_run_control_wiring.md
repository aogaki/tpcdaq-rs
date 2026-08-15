# 050 — Run 制御 UI の実配線(P4 本体: disabled 解除)

**Status: COMPLETED**(2026-08-15 — 結果は末尾)
**Status(起票時): READY**(起票 2026-08-15 Fable — 041 統合デモ完了 + 044 窓クローズで前提成立)
**仕様**: SPEC v1.14 §8.1(REST API・操作権・二層アクセス制御)/ §11(破壊的操作の確認)
**前提**: エンドポイントは全て実在・実走証明済み(041 で curl 一周 / 042 で 3 相 id /
043 で ecc_error)。**controller 側の変更はゼロ**(044-C3 の再裁定: 分割見送りを確定)。
UI 側は 029 の設計どおり **切替点 = `RUN_CONTROL_ENABLED` 1 定数、配線点 = `run-view` の
`proceed()` 1 箇所**。
**発注先想定**: implementer/**Opus**(UI 状態制御の工学判断が残る)

---

## やること

### A. 操作系 API サービス(新規、閲覧系 `controller-api.ts` とは分離)

- `acquire {operator, passphrase}` → token 保持(メモリ + sessionStorage — リロード耐性。
  設計判断が割れたらメモリのみで戻る)/ `release`。横取りは仕様どおり常に可(§8.1)。
- 状態変更系の共通形: token 添付、HTTP エラー時は応答の `error` 文字列を**そのまま**表示
  (加工しない — 041 D-1 の「明瞭なエラー」を活かす)。`notes` 配列(run/stop 応答)も表示。

### B. `proceed()` の配線と `RUN_CONTROL_ENABLED = true`

- actions 表(`run-actions.ts`)の endpoint/body どおりに配線。表自体の変更は不要のはず
  (body 欄が実装と食い違っていたら**報告して戻る** — 表は 029 起票時点の写し)。
- run/start は `comment` 入力(任意)、run/next は正整数入力。
- 破壊的操作(stop/breakup/reset 等 `destructive: true`)は既存の確認ダイアログ scaffold を
  実配線。

### C. ボタンの有効/無効の状態制御

- `GET /api/status`(既存ポーリング)から: 操作権の有無(token の有効性)、run 実行中か、
  ECC state。**最小限の規則で**: token 無し → 操作系全 disabled / run 実行中 → start 系
  disabled・stop 有効 / ECC 段階操作は state に関わらず送信可(実 ECC が Ignored/Denied を
  正しく返すことは 036/049 で保証済み — UI 側で先回りのガードを作り込まない = KISS。
  結果のエラー表示で十分)。
- 送信中は当該ボタンをスピナー等で抑止(二重送信防止)。run/start は ≈7 s かかる(041 実測)
  ことを前提に。

### D. 受け入れ

- **UI 単体テスト**: 全アクションの請求形(URL・メソッド・body・token 添付)を
  HttpTestingController で機械照合。エラー表示・確認ダイアログ・disabled 規則のテスト。
  `ng test` 全 green(既存 127 + 新規)。`npm run build` 成功・初期バンドル予算内。
- **実スタック smoke**: `reference/_spike/demo/` のスタック(実 ECC + vcobo-daq +
  全コンポーネント)を起動し、**ビルド済み UI を controller が配信している状態**で、
  UI が発行するのと同一形のリクエスト列(acquire → run/start → run/stop → release)が
  通ることを確認(ブラウザ自動化は無いので HTTP レベルで可。ブラウザでの目視一周は
  完了後にユーザーが行う受け入れデモとする — 結果節に手順を書く)。
- Rust/C++ に触らない(`cargo` ゲート実行不要)。既存 UI テスト無変更。
- 結果節: 請求形テストの一覧 / smoke の記録 / ブラウザ確認手順。

## 非スコープ

- ecc_error / forced_eos / config_ids の**表示強化**(→ 051)。
- controller の変更(必要が出たら報告して戻る — 出ないはず)。

## 結果(2026-08-15 implementer/Opus → 発注側(Fable)レビュー PASS)

- **ng test 171 passed / 5 skipped**(127 → −3+7+30+10。書き換えた 3 件は「disabled 時代」を
  仕様としていたテストで、フラグ true 化と論理的に両立しない — 逸脱①として受理)。
  `npm run build` 成功・初期 501.67 kB(予算内)・prettier clean。Rust/C++ 非接触。
- **実スタック smoke 合格**(実 ECC + vcobo-daq、controller がビルド済み UI を配信):
  401(偽 token)→ acquire → run/next(41)→ **run/start 6.95 s → Running** → status
  (ecc NO_ERR)→ **run/stop 1.14 s(ok/normal/forced_eos:true/eos_closed:true)** →
  release → release 後の 401、まで全要求列が UI と同一形で通過。出力 graw 30,108,672 B +
  run0041.root。停止後の残留プロセスゼロ。
- 実装: 操作系 API を閲覧系と分離(`run-control-api.ts` — 純ロジック分離 + fetch 直、
  投げずに ActionResult)、token = signal + sessionStorage、`proceed()` 1 箇所配線、
  確認ダイアログ実配線、disabled 規則は最小限(ECC 段階操作に先回りガード無し)。
  **actions 表(endpoint/body/destructive)は無変更 = controller serde と完全一致を確認**。
- **逸脱の裁定(全て受理)**: ①run-actions.spec 3 件の書き換え(論理的必然。表検証 5 件と
  他 119 件は無変更)②fetch 差し替えでの請求形照合(リポは HttpClient 不使用 — 同等以上)
  ③200+ok:false を失敗表示(silent failure 禁止の正しい適用)④送信中は全ボタン抑止
  ⑤acquire 常時有効・next_run ローカル検証・passphrase クリア ⑥token 有効性は自前保持で判定。
- **未決の裁定**: SPA fallback 404 → **052 起票**(controller の S 修正)/
  demo.conf の ui_dir → start_demo.sh(ローカル専用)に発注側が直接追記 /
  notes 表示の実証は単体テストで足りる(異常 run の再現は 031 soak で自然に得られる)。
- **ユーザー受け入れデモ手順は本結果節の報告⑤のとおり**(ブラウザで
  http://127.0.0.1:8080/ → Run control。操作列 ①〜⑧)。
- 実行環境: macOS Darwin 25.5.0、2026-08-15。

**Status: COMPLETED**
