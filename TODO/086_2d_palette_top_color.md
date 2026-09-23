# 086 — 2D パネルのパレット: 最上位色が白で飽和 ch が見分けにくい(077 目視の指摘)

**Status: READY(追補)**(2026-09-22 起票 Fable。Sonnet 0.5 日。**目視確認は未実施 — Fable/ユーザー待ち**)

## 0. 結論
- `ui/src/app/monitor/jsroot-loader.ts` の `PALETTE = 56` は最上位色が**白**。ダークテーマの黒地では最大値付近(パルサー run で 4095 飽和の FPN、
  通常 ch のパルス頂上)が白〜淡い水色に潰れて区別できない(077 結果 第 2 弾のスクリーンショット)。
- 最上位が白でない連続パレットに替える: 第一候補 **57(kBird: 濃紺 → 緑 → 黄)**。UVW / AsAd / Histo2d の全 2D パネル共通(1 定数)。
  1D ヒストの線色には影響しない。

## 1. 発注
- `PALETTE` を 57 に変更し、コメントに理由(最上位が白だと飽和が見えない、077 結果 第 2 弾)を書く。
- 受け入れ: `npm test -- --watch=false` green(既存 201 + 変更なし)、`npm run build` green。目視: demo スタックの monitor タブで
  AsAd パネルの FPN 行(4095 飽和)が通常 ch のパルス頂上と別の色で見えること(結果節にスクリーンショットの所在)。
- 57 でダークテーマとの相性が悪ければ 55(rainbow)を試し、どちらにしたかと理由を結果節に。SPEC の改訂は不要(§11 はパレットを規定していない)。

## 結果(2026-09-23、Sonnet レーン)

### 変更ファイル
- `ui/src/app/monitor/jsroot-loader.ts` — `PALETTE` を `56` → `57`(kBird)に変更 + コメント全面書き直し。

### 選んだパレットと理由
- **57(kBird)を採用**(発注書の第一候補どおり。55 への差し替えは行わなかった)。
- `node_modules/jsroot/modules/base/colors.mjs` の `case 56` / `case 57` の rgb 表(9 stop)を実際に読み、
  `DARK_MODE=true` の `invert(100%)`(= `255 - x`)を手計算して比較した:
  - **56(旧)**: stop0 ≈ rgb(242,243,230)(低)→ stop8 = rgb(0,0,0)(高)。反転後は低 ≈ 黒、**高 = 白**。
    最大値付近(飽和 4095 と、ただ波高が高いだけの通常 ch)が両方とも白〜淡色に潰れる(077 で確認済みの不具合)。
  - **57(kBird、新)**: stop0 ≈ rgb(53,42,135) 濃紺 → stop4 ≈ rgb(46,183,164) 緑 → stop8 ≈ rgb(249,251,14) 黄。
    反転後は stop0 ≈ rgb(202,213,120)(淡黄緑)、**stop8(最上位 = 飽和)≈ rgb(6,4,241) の濃い青**。白には潰れない。
  - **55(rainbow)は不採用**: 同じ rgb 表(`case 55`)で手計算すると stop0 ≈ rgb(0,0,99) → 反転 rgb(255,255,156)(淡黄)、
    stop8 ≈ rgb(110,0,2) → 反転 rgb(145,255,253)(淡シアン)。**両端がどちらも「明るい淡色」になり**、
    56 と同じ「頂上が明るく潰れる」問題を引き継ぐ可能性が高い。57 の方が明確に条件(最上位が白/淡色でない)を満たすため、
    55 を試すまでもなく 57 を採用した。
  - トレードオフ: 57 は反転後の低域が淡黄緑(明度が高め)になり、028 が当初 57 を避けた理由(低い値が画面で一番明るく見える)が
    一部再発する。086 はこのトレードオフを許容し、飽和の可視化を優先する裁定(発注書 §0)。
- **この判断は rgb 表からの数値計算のみに基づく**。実機画面での目視確認(AsAd パネルの FPN 行 4095 飽和が
  通常 ch のパルス頂上と別の色で見えること)は今回**行っていない** — demo スタックがユーザーの実行中スタックと
  ポートを衝突させるため、この Sonnet レーンでは demo を立てなかった(発注元の指示どおり)。
  **目視確認は Fable/ユーザーの担当**。見え方が悪ければ発注書の代替案どおり 55 に差し替える。

### 実行コマンドとテスト結果
- `cd ui && npm test -- --watch=false`
  - `Test Files: 14 passed | 1 skipped (15)`
  - `Tests: 201 passed | 5 skipped (206)`(発注書の基準どおり、既存 201 のまま新規テスト無し — 定数変更のみで
    `jsroot-loader.ts` に既存 spec ファイルは無く、`PALETTE` の値を直接検証するテストも元々存在しない)。
- `cd ui && npm run build`
  - green。`Application bundle generation complete.`(9.078s)。`monitor-view` チャンク 58.43 kB など、警告・エラー無し。
- `cd ui && npx prettier --check src/app/monitor/jsroot-loader.ts`
  - `All matched files use Prettier code style!`

### 発注書からの逸脱
- なし。発注書の第一候補(57)をそのまま採用し、代替(55)への切り替えは行わなかった。目視確認の非実施は
  発注元(オーケストレータ)の明示指示どおり。

### 実行環境
- 2026-09-23、worktree `.claude/worktrees/agent-afad42a06416202d2`(`git merge --ff-only main` で `a11013a` まで同期済み)。
- Node/npm は本体 `ui/node_modules` をシンボリックリンクで再利用(`ln -s /Users/aogaki/Workspace/tpcdaq-rs/ui/node_modules ui/node_modules`)。

## 追記(2026-09-23、Fable レビュー後の方式変更、Sonnet レーン)

### レビューでの指摘
- 上記(第一弾)の「57 をそのまま `settings.Palette` に指定」は**不採用**と裁定された。
- 理由: `DARK_MODE=true` は canvas SVG 全体への `filter: invert(100%)` なので、57(kBird)をそのまま指定すると
  画面には「kBird を反転した配色」が出る。手計算のとおり低域が淡黄緑(明るい)・最上位が濃い青になり、
  028 が当初 57 を避けた理由(低い値が画面で一番明るく見える)にそのまま戻ってしまう。
- レビューで標準パレット番号 51–113 の反転後の両端を全数計算したが、「反転後に低域が暗く、最上位が白でない
  飽和色」になる既製パレットは無かった(kBird を反転してまた kBird に戻る標準パレットは存在しないため)。

### 採用した方式 — 反転前提のカスタムパレット
- `ui/src/app/monitor/jsroot-loader.ts`:
  - `getColorPalette(57).palette`(255 色、`rgb(r, g, b)` 形式)で kBird の生の配色を取得。
  - 新設の純関数 `invertColors(colors: string[]): string[]`(export 済み)で各色を 255-x 反転
    (`rgb(...)` と `#rrggbb` の両形式に対応。未知形式は例外にせず `console.warn` で可視化した上でそのまま通す
    — CLAUDE.md「silent failure を作らない」)。
  - `setColorPalette(CUSTOM_PALETTE_ID, 反転後の配色)` で標準番号と衝突しない `1057` にカスタムパレットとして登録し、
    `settings.Palette = 1057`。
  - `import('jsroot/colors')` を `import('jsroot/core')` / `import('jsroot/draw')` に追加(サブパスのみ、`jsroot` 本体
    (main.mjs)は引かない — 既存の KISS 方針を維持)。
  - `src/jsroot.d.ts` に `declare module 'jsroot/colors'`(`getColorPalette` / `setColorPalette` の最小限の型)を追加。
- 数値的な検証(実際に `node` で `jsroot/modules/base/colors.mjs` の `getColorPalette` / `setColorPalette` を呼んで確認、
  スクラッチ用の一時スクリプトで実行・破棄):
  ```
  raw kbird first/last:                              rgb(53, 42, 135)   rgb(249, 249, 15)
  custom(registered, pre-SVG-invert) first/last:      rgb(202,213,120)   rgb(6,6,240)
  what the screen actually shows (custom + invert):   rgb(53,42,135)     rgb(249,249,15)
  ```
  「画面が実際に見せる色」(カスタムパレット値にさらに `invert(100%)` を適用した結果)は raw kBird と一致する
  ことを確認した — 反転後 = 本物の kBird(低 = 濃紺 rgb(53,42,135)・高 = 黄 rgb(249,249,15))になる、
  という設計どおりの結果。

### 新規テスト(TDD — red を確認してから実装)
- `ui/src/app/monitor/jsroot-loader.spec.ts` を新設(jsroot を読み込まず `invertColors` の文字列変換だけを検証。
  KISS — jsroot 本体との統合の数値検証は上記のスクラッチスクリプトで別途確認済み)。
  1. `rgb(r,g,b) は各成分を 255-x する` — 出典: colors.mjs `case 57` stop0 ≈ rgb(53,42,135)、
     255-53=202 / 255-42=213 / 255-135=120 → `rgb(202,213,120)`。
  2. `#rrggbb は 255-x をバイトごとに計算し小文字 hex を保つ` — 出典: stop8 ≈ rgb(249,251,14) = `#f9fb0e`、
     255-249=6 / 255-251=4 / 255-14=241 → `#0604f1`。
  3. `未知の色形式は例外にせずそのまま通し、console.warn で可視化する` — `vi.spyOn(console, 'warn')` で
     1 回呼ばれたことを確認。
  - 実装前に `npm test -- --watch=false` を実行し、`TS2305: Module '"./jsroot-loader"' has no exported member
    'invertColors'` で red を確認(ビルド時型エラーで落ちる — vitest は Angular ビルドの上に乗っているため)。

### 実行コマンドとテスト結果(方式変更後)
- `cd ui && npm test -- --watch=false`
  - `Test Files: 15 passed | 1 skipped (16)`
  - **`Tests: 204 passed | 5 skipped (209)`**(既存 201 + 新規 3 = 204。発注書の想定どおり)。
  - 新規テスト名(`describe` = `086 invertColors — DARK_MODE の invert(100%) を打ち消す反転`):
    `rgb(r,g,b) は各成分を 255-x する(...)`、`#rrggbb は 255-x をバイトごとに計算し小文字 hex を保つ(...)`、
    `未知の色形式は例外にせずそのまま通し、console.warn で可視化する(silent failure 禁止)`。
- `cd ui && npm run build`
  - green。`Application bundle generation complete.`(10.236s)。`monitor-view` チャンク 58.99 kB、警告・エラー無し
    (`jsroot/colors` の追加 import で `chunk-D7FHpZU6.js`(95.62 kB)が新たに分離されただけ)。
- `cd ui && npx prettier --check src/app/monitor/jsroot-loader.ts src/app/monitor/jsroot-loader.spec.ts src/jsroot.d.ts`
  - `All matched files use Prettier code style!`

### 発注書 / レビュー指示からの逸脱
- なし。レビューで指示された方式(`getColorPalette`/`setColorPalette` によるカスタムパレット登録、
  `invertColors` の 3 テスト、`PALETTE_ID` を 51–113 と衝突しない値に)をそのまま実装した(`CUSTOM_PALETTE_ID = 1057`)。
- 目視確認は今回も**未実施**(demo スタックがユーザーの実行中スタックとポートを衝突させるため)。
  引き続き **Fable/ユーザーの担当**。

### 変更ファイル(方式変更分、累積)
- `ui/src/app/monitor/jsroot-loader.ts`(`invertColors` 追加・`loadJsroot` を `jsroot/colors` 経由のカスタムパレット登録に変更・コメント全面書き直し)
- `ui/src/app/monitor/jsroot-loader.spec.ts`(新設)
- `ui/src/jsroot.d.ts`(`declare module 'jsroot/colors'` 追加)

## 追補 2 発注(2026-09-23、ユーザー目視「Light モードに最高」= ダークで眩しい — Sonnet)

- 目視(ユーザー + Fable の headless スクリーンショット `reference/_spike/demo/cdp_shot.py`): kBird 反転方式は最上位が黄で飽和 FPN は見えるが、
  **ゼロ付近が一面の鮮やかな青**になりダークテーマで眩しい。JSROOT の colz は最下位レベルに `calcColorIndex(0, 20)` ≈ 255 色中 13 番目を使うため、
  kBird の先頭(53,42,135)ではなく (38,62,170) 相当の青になる。
- 裁定: **自前のダーク用ランプ**(9 stop、線形補間で 255 色)を pre-invert して登録する(方式は同じ、配色だけ差し替え):
  `0.000 (0,0,0)` 黒 / `0.125 (30,10,60)` 暗紫 / `0.250 (60,20,140)` 藍 / `0.375 (30,80,200)` 青 / `0.500 (0,160,200)` シアン / `0.625 (40,200,120)` 緑 /
  `0.750 (180,220,60)` 黄緑 / `0.875 (255,210,0)` 黄 / `1.000 (255,120,0)` 橙。ゼロ付近(5 %)は ≈ (12,4,24) の黒に近い色、パルス頂上(≈ 3500/3700)は黄、
  4095 飽和の FPN は橙で区別できる。
- 実装: `jsroot-loader.ts` に純関数 `rampPalette(stops: ReadonlyArray<[number, [r,g,b]]>, n = 255): string[]`(位置で線形補間、`rgb(r,g,b)`)を export し、
  `TPCDAQ_DARK_STOPS` 定数 → `rampPalette` → `invertColors` → `setColorPalette(1057, …)`。`getColorPalette(57)` は使わない(kBird 依存を外す)。
  spec: ①`rampPalette` の端点と中点(0.0625 = 黒と暗紫の中間)の手計算オラクル ②長さ 255 ③既存の `invertColors` 3 本は不変。
- 受け入れ: `npm test`(268 → +2〜3)、`npm run build`、prettier。目視は Fable が `cdp_shot.py` で行う。

