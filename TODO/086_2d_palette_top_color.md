# 086 — 2D パネルのパレット: 最上位色が白で飽和 ch が見分けにくい(077 目視の指摘)

**Status: COMPLETED**(2026-09-22 起票 Fable。Sonnet 0.5 日。**目視確認は未実施 — Fable/ユーザー待ち**)

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
