# 201 — fitProngs v0.1: 197 の開いていた判断を閉じる(有感の箱・端点の縛り・A の固定・共通モード・誤差模型・mm 表示)

**Status: TODO**(2026-11-03 起票 Fable、198 の前提。実装 = implementer / **Sonnet** — 発注書とテストで縛れる。エスカレーション規則は CLAUDE.md: 同一テストで 2 連続失敗・Minuit で堂々巡り → Opus)

## 動機
197(archive)で α 1 本の当てはめは動いたが、χ²/ndf が壁 α で中央 23、内部 86 本で 107 と ≫ 1。残差は ① 飛跡に沿った ≈ ピークの 10 % の構造 ② 全 strip 共通の縦縞(共通モード)③ 有感の外へ漏れる模型(73 の V / W)④ 固定 ROI の外へ逃げる端点(伸びる 11 本)。このままでは 198 の仮説選択(Δχ²)が意味を持たない。**198 に進む前に、197 の末尾「判断が要るところ」を全部ここで閉じる**。設計判断は Fable が下記で済ませた — implementer は選ばない。

## Fable の裁定(なぜそうするか)
1. **有感の箱で模型を切る**: 点が `x ∈ [−53, 53]`・`y ∈ [−58, 48]`(contained.C / stoppedAlphas.C と同じケージの定数、コメントで出どころ)の外なら電荷 0。strip は箱より長いので「strip が無い所は消える」だけでは足りない(73)。z は 512 cell の外で消える現行のまま。
2. **端点は初期値 ± 10 mm の箱に縛る**(Minuit2 の limited variable。Mikolaj の fitter の ± 7 mm と同じ発想)。ROI 15 mm の内側に収まるので「ROI の外に逃げて χ² が見えない」が消える。**限界に張り付いたら `atLimit` ビットで印**(silent にしない)。ROI の動的更新はしない(χ² が不連続になる)。
3. **σ_T は帯のまま**。`sigmaXY = sqrt(sigmaT² + 0.146)` を**派生値として tree に足すだけ**(197 ① の照合結果)。模型は変えない。
4. **A は run ごとに固定**(引数 `adcPerMeV` = 面あたりの積分 ADC / MeV = 197 の「A × 核の面積」。`0` = 自由 = 197 の挙動、テストと A_ref の測定に使う)。理由: A が自由だと α 1 本の仮説が塊の電荷を A に吸って(197 ④: 105 k vs 85 k)198 の仮説選択が効かない。長さ → E(ASTAR)→ 電荷 の拘束こそが使いたい物理。run 209 の流量変更(+ 7 %、47.07 h)は今回は 1 本の値で飲む(両仮説に同じに効くので選択には響かない。時間依存の A_ref は後の票)。
5. **共通モードは data 側で引く**: 面ごと・cell ごとに、**ROI の外の strip**(チャンネルあり・飽和なし)の中央値 c_t を取り、その面の全 strip から引く。使える strip が 8 本未満なら引かずに `cmSkipped[v]` を立てる。雑音 σ_v は引いた後の先頭 25 cell の RMS の中央値で取り直す。模型にパラメータを足さない(KISS)。
6. **誤差模型**: 画素の分散 `σ² = σ_v² + (f · max(data, 0))²`、**f = 0.10**(197 の残差 ≈ ピークの 10 % から)。**data で作る(model ではない)**ので当てはめ中は定数 = 推定量に偏りを入れない・ndf は変わらない。目的は「明るい芯の 10 % の構造(利得・pad の形)で χ² が支配される」のを止め、Δχ² を使える数にすること。strip ごとの利得表は作らない(まず下の診断で「strip 固有の模様か」を数字にする)。
7. **表示は fitProngs.C の中のまま**(別マクロにしない — 模型の描画器がここにあり、別に作ると重複か include になる)。軸を mm に(x = z [mm]、y = s [mm]、drawUVW と同じ向き)、**模型と残差の段にだけ**線分を細線(1 px)で重ね、始点 ○・止まる端 ●。データの段には何も重ねない(192 §3-2)。

## 入力
197 と同じ(`macros/data/run0209_raw.root`・`run0209_tracks.root`・`channel_map.csv`・`data/kernel/run0107_kernel.json`)。事象リスト: 壁 α 20 本 = 197 ③ の entry(15, 18, 20, 29, 36, 44, 52, 53, 56, 62, 65, 73, 79, 82, 84, 87, 89, 90, 91, 95)、内部 ≥ 25 mm の 86 本 = `…/scratchpad/t197/list4.txt`(無ければ 173 §4 の選び方で再現: contained.C の両端内側 + `lengthF ≥ 25`)。

## 作るもの(`macros/fitProngs.C`・`macros/test_fitProngs.C` の変更)
1. `fpPoint` / `fpRender`: 箱の外の点は電荷 0(定数 `FP_CAGE_X = 53`、`FP_CAGE_Y_LO = −58`、`FP_CAGE_Y_HI = 48`)。
2. `fpFitOne`: 端点 6 個を `SetLimitedVariable(初期値 ± FP_END_BOX_MM = 10)`。収束後、どれかが限界の 0.01 mm 以内なら `atLimit` にビット(1 << j)。
3. `fitProngs(...)` に引数 `double adcPerMeV = 0`(既定 = 自由)。> 0 なら A = adcPerMeV / 核の面積 を固定(Minuit に登録しない。ndf = bins − 8)。TParameter `adc_per_mev` に保存。
4. `fpFillData` の後に `fpSubtractCommonMode(FpData&, const mask[3], …)`: 上の 5。純関数(テスト可能)。
5. `fpChi2`: 上の 6。`f` は定数 `FP_FRAC_ERR = 0.10`、引数で 0 にできる(テスト [7] は f = 0 で現行の値のまま)。
6. tree に追加: `sigmaXY`、`atLimit`(int、bit)、`cmSkipped`(int、bit per view)、`noise[3]`(引いた後の σ_v)。既存のブランチ名は変えない。
7. `fpDrawEvent`: mm 軸 + 細線の重ね(上の 7)。
8. ヘッダコメント(1–66 行)を変更に合わせて更新。

## テスト(`test_fitProngs.C`、既存 [1]–[9] は通ったまま。f = 0・adcPerMeV = 0 で呼べば旧挙動 — 閉包 [6] はそのまま green のはず)
- [10] 箱: 60 mm の α を S = (40, 0, 30) → E = (100, 0, 30)(x = 53 で箱を出る)で描くと、総電荷 = A × [E(60) − E(47)] × 核面積(箱の内側 13 mm 分だけ)± 0.5 %。x = 52.99 の点は入る・53.01 は消える。y も同様に 1 例。
- [11] 端点の縛り: [6] の閉包(1.5 mm ずらした初期値)は通ったまま `atLimit = 0`。真の端点から 12 mm ずらした初期値では `atLimit ≠ 0` で、端点は初期値 ± 10.0 mm の内側。
- [12] 共通モード: 合成の面(41 strip × 512 cell)に cell ごとのオフセット c_t = 30 sin(2π t / 20) を全 strip に足し、ROI(10 strip)に飛跡を置く → 引いた後、ROI の外の画素は |x| < 1e-9、ROI の中の画素は 飛跡の値 ± 1e-9。ROI の外が 7 strip しか無い面では引かず `cmSkipped` のビットが立つ。飽和 strip は中央値の母集団に入らない(1 本を 4095 にして中央値が動かないこと)。
- [13] 誤差模型: [7] の 2 strip × 3 cell の格子で f = 0.1、data = 100 の画素は分散 4 + 100 = 104 → 手計算の χ²。f = 0 で [7] と同値。
- [14] A の固定: 核 {0.5, 1, 0.5}(面積 2)で adcPerMeV = 2000 → A = 1000 が固定される([4] の合成で確認)。[6] の 1 本(A 真値 5000、実核)を adcPerMeV = 5000 × 核面積 で当てはめて、A が動かない・ndf = bins − 8・端点 ± 1 mm。
- [15] `sigmaXY = sqrt(sigmaT² + 0.146)` が tree に出る([9] の e2e で確認)。
- [16] e2e: PNG が出て 10 kB 以上。

## 受け入れ(実データ、結果は報告に書く → 主対話がここに転記)
① **壁 α 20 本の χ²/ndf の表 2 × 2**: {共通モードなし / あり} × {f = 0 / 0.1}(箱と縛りは常に入れる、A は自由)。最小 / 中央 / 最大。**目標: 両方入れて中央 ≤ 3**(届かなければ、どの事象がなぜ残るかを残差 PNG で示す — 隠さない)。
② **A_ref**: 上の条件(A 自由)で 20 本の A × 核面積 の中央値と robust σ。**47.07 h の前後で分けた中央値も出す**(eventTime から。前後で 5 % 以上違っても 1 本の値を使う — 記録のため)。→ これが run 209 の `adcPerMeV`(198 が使う)。
③ **内部 86 本を A 固定で当て直す**: 縮む / 伸びる / 健全 の数(197 の判定法のまま)、`atLimit ≠ 0` の数、χ²/ndf の分布。期待: 伸びる 11 本は縛りで消える(`atLimit` に移る)、縮む 10 本は残る(198 の対象)。
④ PNG(mm 軸)= 15・73・27・94・89。73 で V / W の有感の外の帯が消えていること。
⑤ **診断(198 の判断材料、数字 1 つ)**: 壁 α 20 本で、飛跡に沿った残差 Σ(data − model) / Σ model を strip の偶奇で分けた比(U / V / W 別)。偶奇で系統的に違えば「pad の形か strip の利得」= 後の票、違わなければ 197 の「交互の段々」は別の原因。
⑥ 速さ: 1 事象の中央値(2 向き)。197 の 1.0–1.5 s から大きく増えないこと(共通モードは 1 回だけ)。

## 禁止・注意
- 触るのは `macros/fitProngs.C`・`macros/test_fitProngs.C`・`macros/README.md`(fitProngs の行 1 つを足す / 直す)だけ。**`macros/drawUVW.C`・`macros/selectNeutron.C`(とそのテスト)は触らない・include しない**。TPCReco(`reference/`)は読むだけ。コミットしない。
- `macros/data/` は読むだけ。出力は `/private/tmp/claude-501/-Users-aogaki-Workspace-tpcdaq-rs/c5696e1c-f7d9-4a2e-ab7f-ab99dda8905b/scratchpad/t201/`(mkdir -p してよい)。`…/scratchpad/t197/` は読んでよい(リスト・前回の fit4.root)。
- ROOT は `export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX27.sdk`。テストは ACLiC(`test_fitProngs.C+`、build dir は scratchpad)。
- **着手前に `git status` で fitProngs.C / test_fitProngs.C が未追跡(197 のまま未コミット)であることを確認し、変更前のコピーを scratchpad に取る**(diff の照合用)。
- 定数(f = 0.10、± 10 mm、8 本、箱)は変えない。合わないときは数字と理由を報告して戻る。発注書に無い設計の分岐も同様。

## 報告の形
変更したファイルと行数(197 のコピーとの diff の要約)/ テスト名ごとの PASS・FAIL と実行コマンド / red の証拠 / 受け入れ ①–⑥ の数値と PNG のパス / 発注書から外れたところとその理由 / **実行できなかった操作**(無ければ「なし」)。
