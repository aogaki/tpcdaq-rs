# 090 — 波形ビューで AsAd 1 枚分(4 AGET × 68 ch = 272 系列)を一度に表示する

**Status: COMPLETED**(目視待ち — 2026-09-23 実装 Sonnet レーン。272 選択で AsAd 1 枚分が全部載ることのユーザー目視・`cdp_shot.py` は未実施。demo スタックはユーザーのポートと衝突するため起動しなかった)

## 0. 事実
- 上限は検出器の値ではなく表示側の選択肢: `ui/src/app/waveform/waveform-view.ts` の `SERIES_LIMITS = [4, 8, 16, 32, 64]`(`maxSeries` 既定 16)。
  データ(0x03)は AsAd 1 枚 = 4 AGET × 68 ch × 512 bucket を全部運んでいる(SPEC §5.4、R13)。AGET の複数選択と ch「all」は実装済み。
- 描画: ECharts、`overlay`(1 グリッドに重ね描き、legend scroll)と `grid`(系列ごとに小グリッド、`gridShape` は最大 4 列 × 190 px/行)。
  272 系列 × 512 点 = 約 14 万点。

## 1. 発注
1. `SERIES_LIMITS` に **68 / 136 / 272** を足す(68 = AGET 1 個 FPN 込み、272 = AsAd 全部)。既定 16 は変えない。
2. `overlay`: 系列数 × 点数が **100,000 を超えたら** 各 series に `sampling: 'lttb'` と `large: true`(ECharts 6 の line series)を付け、`lineStyle.width` を 1 のまま、
   `legend` は 64 系列を超えたら非表示(272 個の凡例は役に立たない。代わりにツールチップの系列名で判別)。`animation: false` は既存どおり。
3. `grid`: 系列数に応じて列数を増やす — `gridShape(count)`: count ≤ 16 → 最大 4 列(現状)、≤ 64 → 最大 8 列、それ以上 → **12 列**、行の高さは 4 列 190 px / 8 列 140 px / 12 列 110 px。
   各小グリッドのタイトル(`aget/ch`)は 12 列では文字を 10 px に。x/y 軸ラベルは 8 列以上で間引く(`axisLabel.interval` と `show: false` の併用は可)。
4. 「描かなかった系列数」(`droppedSeries`)と「範囲外」(`outOfRange`)の表示は維持。272 を選んだときに AsAd 1 枚分が全部載ることを **ユーザー目視**
   (demo、`cdp_shot.py` で Fable も撮る)。
5. 純関数に寄せる: `gridShape` と「大量描画かどうか」(`isDense(seriesCount, pointsPerSeries)`)は `waveform-select.ts`(または新規 `waveform-layout.ts`)に置き spec で縛る。

## 2. テスト(vitest)
- `waveform-select.spec.ts`: `maxSeries = 272` で 4 AGET × 68 ch の合成メッセージから 272 系列が全部出る(dropped 0)/ `maxSeries = 68` で 1 AGET 分。
- `gridShape`: 16 → 4 列、64 → 8 列、272 → 12 列 × 23 行、行の高さ。`isDense`: 64×512 = false、272×512 = true。
- 既存 12 + 増分を結果節に。`npm test -- --watch=false`(現 272 passed)、`npm run build`、prettier。

## 3. 参照
SPEC §5.4 / §11(波形ビュー R13、クライアント側間引き)、077(AsAd 2D パネル = 一目で見る用途はこちら)。

## 結果(2026-09-23、Sonnet レーン)

### 実行コマンド
```
cd ui && npm test -- --watch=false
cd ui && npm run build
cd ui && npx prettier --check src/app/waveform/waveform-view.ts src/app/waveform/waveform-select.ts \
  src/app/waveform/waveform-select.spec.ts src/app/waveform/waveform-layout.ts src/app/waveform/waveform-layout.spec.ts
```

### テスト数(before → after)
- before(着手前ベースライン、090 起票コミットで確認): Test Files 21 passed | 1 skipped (22)、Tests **272 passed** | 5 skipped (277)
- after: Test Files 22 passed | 1 skipped (23)、Tests **283 passed** | 5 skipped (288)
- 増分 11 件、すべて green(赤 → 緑を確認済み: `waveform-layout.ts` 追加前に import エラーで `ng build` が失敗することを確認してから実装)。

新規テスト:
- `ui/src/app/waveform/waveform-layout.spec.ts`(新設、9 件)
  - `gridShape(count)`: 「16 系列以下は最大 4 列(現状)、行の高さ 190px」「64 系列は最大 8 列、行の高さ 140px」
    「272 系列(AsAd 1 枚分)は 12 列 × 23 行、行の高さ 110px」「小さい系列数は正方形に近い列数に収める(4 → 2 列 × 2 行)」
    「count=0 でも例外を投げない」
  - `isDense(seriesCount, pointsPerSeries)`: 「64 系列 × 512 点 → 密ではない」「272 系列 × 512 点 → 密」
    「ちょうど 100,000 点は境界(超えたら、なので密ではない)」「100,000 を 1 点超えたら密」
- `ui/src/app/waveform/waveform-select.spec.ts`(既存ファイルに追加、2 件)
  - 「maxSeries=272 なら AsAd 全部(4 AGET × 68 ch)が出て、何も落とさない」
  - 「maxSeries=68 なら AGET 1 個分(68 ch)しか描かない」

### build / prettier 出力要旨
- `npm run build`: `Application bundle generation complete.`(エラーなし)
- `npx prettier --check`: `All matched files use Prettier code style!`

### 変更ファイル
- `ui/src/app/waveform/waveform-view.ts` — `SERIES_LIMITS` に 68/136/272 を追加(既定 16 は不変)。
  `gridShape`/`isDense` を `waveform-layout.ts` から import(旧 `gridShape` ローカル定義は削除)。
  overlay: 系列数 × 点数が 100,000 を超えたら各 series に `sampling: 'lttb'`/`large: true` を付与、
  凡例は 64 系列超で非表示(`legend.show`)。grid: `gridShape` が返す `rowHeightPx` で
  `chartHeightPx` を計算(4/8/12 列 = 190/140/110 px/行)、タイトルは 12 列のとき 10px(それ以外 11px)、
  軸ラベルは 8 列以上で非表示(`axisLabel.show`)。
- `ui/src/app/waveform/waveform-layout.ts`(新設)— `gridShape(count)` / `isDense(seriesCount, pointsPerSeries)`
  の純関数。Angular/ECharts に非依存。
- `ui/src/app/waveform/waveform-layout.spec.ts`(新設)— 上記の spec。
- `ui/src/app/waveform/waveform-select.spec.ts` — 272/68 系列の選択を縛るテストを追加(`waveform-select.ts` 本体は無変更)。
- `ui/src/app/waveform/waveform-view.html` — 変更なし(`seriesLimits` を汎用ループで描画しているため 68/136/272 追加だけで対応済み)。

### 承認が要る逸脱(実装は完了、判断はレビューで確認してほしい)
1. **grid モードのセル余白を固定 % ポイントから比例値に変更**: 旧実装は `left/top/width/height` を
   `cellW - 10`/`cellH - 10` のような固定 % ポイントで計算していた。列数が 4 → 8 → 12 と増えると
   `cellW`(= 100/cols %)がそれぞれ 25% → 12.5% → 8.33% と縮み、12 列では `cellW - 10 < 0` になって
   グリッドの幅が負値になる(=描画が壊れる)。発注書に余白の式そのものの指定はなかったため、
   `padX = cellW * 0.1` のようにセル寸法に**比例**する余白に直して壊れないようにした。
   純粋な数値パラメータの調整であり、レイアウト方式(percentage-of-chart グリッド)自体は変えていない。
2. **軸ラベルの間引き方法**: 発注書は「`axisLabel.interval` と `show: false` の併用は可」としていたが、
   8 列以上の小グリッドでは目盛りがほぼ読めないため、KISS を優先して `axisLabel.show: false`
   (完全非表示)のみを採用した(interval の段階的間引きは実装していない)。
3. **272 選択で AsAd 1 枚分が全部載ることのユーザー目視・`cdp_shot.py`**: 発注書に明記どおり Fable の作業
   (このセッションは demo スタックを起動していない — ユーザー稼働中のスタックとポート衝突のため)。

### 実行環境
- macOS(darwin)、Node/npm は `ui/node_modules` シンボリックリンク(`/Users/aogaki/Workspace/tpcdaq-rs/ui/node_modules`)経由。
- 2026-09-23、worktree `/Users/aogaki/Workspace/tpcdaq-rs/.claude/worktrees/agent-aac3c11918e496c56`(090 起票コミット `e6e0405` の直後で作業)。

### スキップしたテスト
- なし(新規追加分は全て green)。既存の 5 skipped は本チケット着手前からの既存スキップで、090 の変更対象外。
