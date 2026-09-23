# 090 — 波形ビューで AsAd 1 枚分(4 AGET × 68 ch = 272 系列)を一度に表示する

**Status: READY**(2026-09-23 起票 Fable。ユーザー要望「64 ch が上限。AGET 全てを一度に表示したい」。Sonnet 0.5 日、UI のみ)

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
