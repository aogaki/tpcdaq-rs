# 048 — UI・コメント衛生(dead チャンク除去 / ResizeObserver / 出典コメント一元化)(044 窓)

**Status: COMPLETED**(2026-08-15 — 結果は末尾)
**Status(起票時): READY**(起票 2026-08-15 Fable — 044 UI レーンの検証済み修正の適用)
**発注先想定**: implementer/**Sonnet**(修正は全て検証済み or 機械的。Rust に触らない)

## A. jsroot dead import 由来の 2.38 MB + 667 kB チャンク除去(検証済み修正の適用)

- 事実(レーン実測): `jsroot/modules/base/base3d.mjs:49-51` の **Node.js 専用フォールバック**
  (`isNodeJs()` false のブラウザでは到達不能な `import('three')` / `import('three/addons')`)が
  構文上存在するだけでバンドルされる。SSR/prerender 無し(確認済み)なので完全な dead code。
- やること: `ui/angular.json` の **`externalDependencies` に `"three"`, `"three/addons"` を
  追加**(同リストは jsroot の Node 専用依存対策として既存 — 同じパターン)。
- 検証済みの期待値: 当該 2 チャンク消滅 / **dist 7.8 MB → 4.9 MB** / 初期バンドル
  501.7〜502.0 kB で不変 / チャンク 92 → 88。`npm run build` で再現し数値を結果節へ。
  注記: ブラウザは元々このチャンクを fetch しないので**配布物衛生の改善**(ランタイム性能は
  不変)— 結果節にもその旨を正直に書く。

## B. ResizeObserver ライフサイクルの共有ヘルパ

- 事実: `ui/src/app/monitor/jsroot-panel.ts:144,155-163,167-168` と
  `ui/src/app/waveform/waveform-view.ts:108,132-137,143-144` で
  「field + viewChild ready で lazy 生成 + ngOnDestroy disconnect」が構造的に同一。
- やること: 小さな共有ヘルパ(関数 or ディレクティブ — UI の流儀に合わせて選択)で両者を
  置換(各 ~10-12 行削減)。表示挙動は不変。
- ポーリング重複(run-view / logbook-view の 5000 ms)は **rule of three 未達で触らない**
  (044 裁定 — P4 で 3 例目が出たら)。

## C. ecc_bridge / fake_ecc の一次資料出典コメント一元化

- 事実: 状態機械の set/clear 規則の出典(BackEnd.cpp 行番号)が `tools/ecc_bridge/
  ecc_core.hpp:95-133,378-407` と `fake_ecc.cpp:214-225,240-255` に**独立に二重記載**。
  訂正が 2 箇所編集になり、silent drift の芽(「一次資料はテストダブルではない」事故の類型)。
- やること: **正本 = ecc_core.hpp** とし、fake_ecc.cpp 側は「出典は ecc_core.hpp の規則
  ブロック参照」の 1 行ポインタ + 呼び出し箇所固有の事実だけ残す。**コメントのみの変更**
  (コード・テスト無変更)。

## 受け入れ

- UI: `npm run build` 成功 + `ng test` 全 green(件数不変)。初期バンドルサイズ不変を数値で。
- C++: `make -C tools/ecc_bridge -j test` green(コメントのみなので当然 — 確認として)。
- Rust ゲートは**実行不要**(非接触。並行ユニットが Rust を触っているため実行しない)。
- 結果節: A のバンドル実測 before/after / B の diff 要旨 / C の一元化後の形。

## 結果(2026-08-15 implementer/Sonnet → 発注側(Fable)レビュー PASS)

- **A**: angular.json `externalDependencies` +2 行 → `Addons`(2.38 MB)+ three 本体
  (667.72 kB)チャンク消滅。**dist 7.8 → 4.9 MB、初期バンドル 501.67 kB(不変)**。
  チャンク総数の実測 94→91(発注書予測 92→88 との差はビルド非決定性 — 主要期待値は全一致)。
  配布物衛生の改善であってランタイム性能は不変(発注書どおり明記)。
- **B**: `ui/src/app/display/resize-observer.ts` の `observeResize()` 関数に集約
  (Directive でなく関数 — 既存の流儀(DisplayClock 等)に合わせた選択 = 受理)。
  2 コンポーネント各 -8 行。`ng test` **127 passed / 5 skipped(件数不変)**。
- **C**: fake_ecc.cpp の出典コメント 3 箇所を ecc_core.hpp(正本)への 1 行ポインタ +
  呼び出し箇所固有の事実のみに圧縮(コメントのみ)。`test_ecc_bridge` **200 passed** で確認。
- Rust / cargo 非接触(並行 045/046 と非干渉)。コミットなし。逸脱 = チャンク数の記載差のみ
  (実測値を正として記録)。実行環境: macOS Darwin 25.5.0、2026-08-15。

**Status: COMPLETED**
