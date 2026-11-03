# 197 — 順方向の模型による当てはめ v0(N = 1: α 1 本、頭尻つき)`fitProngs.C`

**Status: COMPLETED**(2026-11-02 起票 Opus、同日完了。Fable 裁定 = archive/192「Fable 所見」§1–3。実装 = implementer / **Opus**)

## 動機
多本の事象(¹⁶O(n,α)¹³C、3α)は 3 面の 1 次元射影から 3D の点を作る方法(191)では戻らない。TPCReco の Mikolaj Cwiok の試作(192 §1(d))と同じく、**模型(3D の線分 + ブラッグ曲線)を strip × cell の画像に描いて、データとの χ² で当てはめる**。Fable の裁定で、**自前で小さく、N = 1(α 1 本)から**作る。198 で「α + 頂点の塊」と仮説選択に広げる。

## 物の配置と前提
- 中性子は **+x 向き**に入る(線源はチャンバーの壁から ≈ 1 m、−x 側、y・z は中心 — ユーザー 2026-11-02)。v0 では使わない(198 以降の運動学で使う)。
- z の目盛り = analyzeUVW.C の外挿(`driftVelocityCmUs`、198 mbar / 1,600 V / 12.5 MHz で 0.2338 mm / cell)。**当てはめは v_drift を決められない**(066 の厳密縮退)。絶対値は 196 で。
- 削るもの(Fable): 運動量保存の罰則・CMS ブースト、¹³C・⁹Be の表、Minuit の段階的な固定・解放の儀式。
- 足すもの(Fable): **138 の実測の応答核**、**飽和**、χ² は **strip × cell の生の格子**で。

## 入力
- `macros/data/run0209_raw.root`(tree `raw`、`eventId`、`adc[4][68][512]`)、`macros/data/run0209_tracks.root`(初期値)、`macros/channel_map.csv`(aget, raw ch → 面, strip, s_mm。s = 1.5 × strip + 定数で一直線)。
- 応答核 `macros/data/kernel/run0107_kernel.json`(502 ns = run 209 の peaking、`all.mean_norm` = ピークで 1 の平均波形 341 点、**25 MHz**、ピークは 40 番目)。12.5 MHz に直す(方法は実装の判断、テストで縛る)。
- α の飛程: `macros/stoppedAlphas.C` の NIST ASTAR(CO2、射影飛程 g/cm² vs MeV)を写して(出どころをコメント)、密度(圧力・293.15 K、stoppedAlphas.C と同じ関数)で mm に。dE/dx(残りの飛程)= dE/dR。

## 作るもの: `macros/fitProngs.C` + `macros/test_fitProngs.C`
1. **データの格子**: drawUVW.C の `fillUVW` と同じ規則(strip ごとに ADC を加算 → 先頭 25 cell の平均を引く)。**drawUVW.C は include しない**(193 で編集中)— 同じ規則を写し、コメントで出どころを書く。面ごとの雑音 σ = strip ごとの先頭 25 cell の RMS の中央値。飽和の天井 = strip ごとに 4095 − ベースライン。
2. **模型**(α 1 本): パラメータ = 始点 S (x, y, z)、終点 E (x, y, z)、振幅 A、σ_T、σ_L(9 個)。向き = 「止まる端」を E とする版と S とする版の 2 通り。
   - 線分に沿って 0.1 mm ごとに点、点の電荷 = A × dE/dx(止まる端までの残りの飛程)× 0.1 mm。
   - 横: 面 v の strip k への割合 = 幅 pitch の帯の上でのガウス(σ_T)の積分(中心 s₀ = p_v · r_xy、p_v = makeTracks.C の PITCH)。有感の strip が無い所に落ちた電荷は消える(= 飛跡が有感の外へ出てもそのまま扱える)。
   - 縦: z のガウス(σ_L)を cell の幅で積分 → 12.5 MHz の核と畳み込む(核のピークを電荷の到着の cell に合わせる — z の原点はどのみち相対)。
   - 飽和: 模型の値を strip ごとの天井で切る。
3. **χ²** = Σ_面 Σ_ROI (data − model)² / σ_v²。ROI = 初期の線分の各面への投影から strip 方向 15 mm・z 方向 15 mm 以内の (strip, cell)(当てはめ中は固定)。ndf = ROI のビン数 − 9。
4. **当てはめ**: 初期値 = tracks の `xStartF…zEndF`(z は同じ mmPerCell なので揃う)、A は data の和から、σ_T = σ_L = 1 mm。ROOT の Minuit2(Migrad)を **2 つの向きで 1 回ずつ**、χ² の小さい方を採る。σ_T・σ_L は v0 では自由(0.3–5 mm)にして、分布を報告する(198 以降で run の定数に固定するか決める)。収束しない場合の扱いは実装の判断(報告)。
   - A について: Fable は「貫通飛跡の q/mm で run ごとに固定」と書いたが、長い飛跡の q/mm は α のエネルギーで変わる(貫通の 6.5 k ADC/mm は α で、MIP ではない)ので、v0 では**自由**にし、事象どうしでの安定性(ADC / MeV に直した値の散らばり)を報告する。これは Fable キューに戻す。
5. 出力: `void fitProngs(const char *rawFile, const char *tracksFile, double pressureMbar, double samplingMHz, double driftHvVolt, const char *kernelJson, const char *outFile, const char *entryList = "", const char *pngDir = "")` — tree `fit`(eventId、entry、向き、χ² ×2、ndf、S、E、A、σ_T、σ_L、長さ、収束の状態、所要時間)。`pngDir` があれば事象ごとに 3 面 × 3 段(データ / 模型 / 残差、strip × cell の軸でよい — 見せ方は 198)。
6. 速さの目安: 1 事象 ≲ 2 s(Mac)。超えるなら何が重いかを報告。

## テスト(`test_fitProngs.C`、exit code = 失敗数、純関数を直接呼ぶ。合成データは一時ディレクトリ)
1. ブラッグ曲線: ∫ dE/dx dR = E(ASTAR の表から、5.0 MeV で ± 0.5 %)、密度の換算(198 mbar で飛程が stoppedAlphas.C の値と一致)。
2. 帯の積分: 全 strip の和 = 1(有感の内側)、中心が帯の境のとき隣どうし 0.5 / 0.5。
3. 核の 12.5 MHz 化: ピークの位置と FWHM が 25 MHz の値(19.7 cell @ 40 ns)と ≈ 一致(± 1 cell @ 80 ns)。
4. 描画の電荷の保存: 飽和なし・全部有感の内側で、模型の総和 = A × E_total ×(核の面積 / ピーク)の手計算どおり。
5. 飽和: 天井を超える画素が切られる。
6. **閉包**: 自分の模型で合成した α(20・40・60 mm、dip 0° / 30° / 60°、雑音 σ = 7 ADC のガウス)を当てはめて**端点 ± 1 mm**、向きが正しく選ばれる。
7. χ² と ndf の手計算(小さな格子)。

## 受け入れ
① **TPCReco の `StripResponseCalculator` との照合**: 合成の飛跡 3 本で、同じ σ_xy・σ_z・peaking(この照合のときだけ核を TPCReco の理想の AGET 応答に合わせる)の画像を画素ごとに比べ、目標 ± 5 %(ピーク比)。TPCReco のビルドは `TODO/archive/066_unfolding_spike.md` と `reference/_spike/unfold/tpcreco_probe/` の直リンクの手順(11 .cpp を無改変、`GeometryTPC.cpp:508` のスタック overflow に注意)。**± 5 % に届かないときは、どこがなぜ違うか**(菱形の pad の形とガウスの帯の差など)を報告する — 隠さない。TPCReco は**改変しない**。
② 閉包(テスト 6)。
③ run 209 の長い飛跡(`ok = 1`・`lengthF > 80`)20 本: χ²/ndf の分布、σ_T・σ_L の分布、残差の画像(PNG のパス)。
④ 内部で止まる ≥ 25 mm の 86 本(173 §4: contained.C の両端内側 + `lengthF ≥ 25`): 頭尻が決まる割合(Δχ² の分布を出し、決まる閾値は実装が提案)、A を ADC / MeV に直した値の散らばり。
結果は報告に書く(主対話がチケットに転記する)。

## 禁止・注意
- 触るのは `macros/fitProngs.C`・`macros/test_fitProngs.C` だけ。**`macros/README.md`・`macros/drawUVW.C` は触らない**(193 で編集中 — README は主対話が後で足す)。TPCReco(`reference/`)は**読む・リンクするだけで改変しない**、リポにコミットしない。コミットしない。
- `macros/data/` は読むだけ。出力は `/private/tmp/claude-501/-Users-aogaki-Workspace-tpcdaq-rs/c5696e1c-f7d9-4a2e-ab7f-ab99dda8905b/scratchpad/t197/`(mkdir -p してよい)。
- ROOT は `export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX27.sdk` を付けて動かす。コンパイル速度が要るなら ACLiC(`.C+`)でよい(生成物はリポに残さない)。
- 発注書に無い設計の分岐(模型の形を変える、パラメータを足す・減らす等)に出会ったら、選択肢と推しを報告して戻る。

## 報告の形
変更したファイルと行数 / テスト名ごとの PASS・FAIL と実行コマンド / red の証拠 / 受け入れ ①–④ の数値と PNG のパス / 発注書から外れたところとその理由 / **実行できなかった操作**(無ければ「なし」)。

## 結果(2026-11-02、implementer / Opus。主対話が照合)
- 新規 `macros/fitProngs.C`(961 行)・`macros/test_fitProngs.C`(722 行)。ほかは無変更(`git status`)。照合のハーネスと出力 = `…/scratchpad/t197/`(`$T`、リポ外)。ACLiC の生成物は scratchpad の build ディレクトリ。
- テスト: `export SDKROOT=…/MacOSX27.sdk; cd macros && root -l -b -q -e 'gSystem->SetBuildDir("<build>", true)' 'test_fitProngs.C+'` → **104 PASS / 0 FAIL / 1 SKIP、exit 0**。`FITPRONGS_REAL_KERNEL=<macros>/data/kernel/run0107_kernel.json` を付けると [3b] も走り **107 PASS / 0 FAIL**(主対話が再実行して確認、2026-11-02、Mac / ROOT 6.40.04)。
  - [1] ASTAR(密度 3.575334e-4、R(5 MeV) = 121.75086 mm が stoppedAlphas.C と 1e-9 で一致、ΣdE/dx·h = E ± 0.5 %)16 / [2] 帯の積分 8 / [3] 核の 12.5 MHz 化 15 / [3b] 実核 FWHM 9.82 cell(786 ns)3 / [4][5] 電荷の保存・天井 9 / [6] 閉包 9 本 + mmPerCell 28 / [7] χ² 9 / [8] 格子・雑音・天井・4090 の境 8 / [9] e2e 11。
  - red: シグネチャだけのスタブで exit 66(66 FAIL)。途中の失敗 = Minuit2 の変数の登録順、[5] のテスト設計、ASTAR の節点で χ² が ≈ 3 跳ぶ(下の外れ 1 で解決)。
- 受け入れ:
  - **① TPCReco の `StripResponseCalculator` との照合**(11 .cpp を無改変でビルド、`-fno-stack-protector` で GeometryTPC.cpp:508 の overflow を回避、mini の .dat の scratch コピーで v_drift と 12.5 MHz の 2 行だけ変更、3 本 × 3 面の最悪 max|T − kM| / max T): 整形なし σ_xy = σ_z = 1.0 で **3.85 %**(目標 ± 5 % を満たす)、こちらの σ_T = 0.92 で 0.58 %。
    - 横の差は**菱形の pad の形**: 菱形の開口は strip を横切る方向に三角形(分散 0.042 mm²)、こちらの 1.5 mm の帯(0.1875)より細い → **σ_xy = √(σ_T(帯)² + 0.146)**(1.0 → 0.92、0.8 → 0.70 が実測と一致)。
    - **TPCReco の不具合を発見**: peaking > 0 のとき σ_z を捨てる(σ_z 1.3 でも 0.3 でも出力が同一)。原因 = `StripResponseCalculator.cpp:923` の `index_fill` が cell の index を足していない。同じパラメータでは比べられないので、TPCReco 流の核と小さな σ_L で模して 2.6–5.7 %。TPCReco は改変していない。ほかに TPCReco は絶対値 1e-5 未満の電荷の断片を捨てる(`Utils::NUMERICAL_TOLERANCE` — MeV 単位では裾が削れる)、応答を cell で積分し cell 内の位置も平均する規約(25 → 12.5 MHz の比較で 10.4 % の差 = 規約の差。実測核は位相で平均済みなので、こちらの 2 点平均が標本化の模型として正しい)。
  - **② 閉包**: 向き 9 / 9、端点のずれ最大 0.057 mm、A 4,988–5,021(真 5,000)、σ_T 0.776–0.802(0.8)、σ_L 1.296–1.305(1.3)、0.29–1.0 s / 事象。
  - **③ run 209 の長い飛跡 20 本**(entry 15, 18, 20, 29, 36, 44, 52, 53, 56, 62, 65, 73, 79, 82, 84, 87, 89, 90, 91, 95): **20 本とも「壁から入って中で止まる α」**(L 61–119 mm、E 3.1–4.9 MeV、入る端は 20 / 20 が壁か窓の上端 — 頭尻は物理と合う、Δχ² / (χ²/ndf) ≥ 5,293)。**χ²/ndf = 3.5 / 17 / 22.7 / 39 / 215(最小 / 四分位 / 中央 / 四分位 / 最大)≫ 1**。σ_T 中央 0.96 mm(pad 換算 σ_xy ≈ 1.03)、σ_L 中央 0.93 mm。**A × 核面積 = 85.3 k ADC / MeV / 面、robust σ 7.6 %**。1.46 s / 事象(最大 7.1 s)。
    - 残差(主対話が entry 15 で目視): 模型は形をよく再現、残差は ≈ ピークの 10 %。飛跡に沿って交互の段々(strip ごとの利得か pad の形)、飛跡の上下で逆符号(位置・幅の小さなずれ)、全 strip 共通の縦縞(共通モード)、≈ 1.8 µs のリンギング(73 で ± 400)、曲がった飛跡(91)、有感の外でも V / W に帯が続く(73)。
    - 飽和 strip を外す効果: entry 89 で χ²/ndf 31.7(外す)vs 246.7(戻す)/ 115.5(外さずに当てはめ直す — σ が下限に張り付く)、91 で 214.9 vs 316.4 / 265.6。
  - **④ 内部で止まる ≥ 25 mm の 86 本**(173 と同じ選び方で再現): χ²/ndf = 0.97 / 26 / 107 / 227 / 488。Migrad の収束 80 / 86。**壊れ方 2 種**: 縮む 10 本(94, 994, 1836, 1869, 3207, 3547, 3729, 4448, 6505, 7102 — 明るい塊へ模型が縮む = 198 の塊の腕の対象)、伸びる 11 本(1634, 2160, 2558, 3208, 3333, 3477, 5267, 5511, 5759, 5925, 6889 — 暗い飛跡で端が固定 ROI の外へ逃げる)。健全 62 本。Δχ² / (χ²_best / ndf) の分布 10 % 点 535、中央 4,592。提案の閾値(≥ 25、status 0、縮む・伸びるでない)で健全 62 / 62 が「頭尻が決まる」— ただし χ²/ndf ≫ 1 で残差が相関するので有意さは過大(正しさを確かめられたのは ③ の壁 α だけ)。makeTracks の「電荷の重い半分が End」と食い違うのは 24 / 86。entry 27 は χ²/ndf 190(α 1 本では合わない — 198)。健全 62 本の A × 面積 = 105.4 k(robust σ 18.5 %、流量変更前 97.8 k / 後 111.7 k)— ③ より ≈ 24 % 高い(塊の電荷が A に入る、と読む)。
  - 速さ: 中央 1.0–1.5 s / 事象。重いのは電荷の配置(erf)1.50 ms / 描画 1.70 ms × 300–500 回 / 向き。
- 発注書から外れたところ(受け入れ): ① 点の電荷を A·[E(R + h/2) − E(R − h/2)](表の内側で dE/dx·h と 0.1 % 以内、節点の段で Migrad が止まるのを避ける)② 0.1 MeV 未満は表の最初の行のべき乗則(b = 1.446)を延長(stoppedAlphas.C の √R では段と尖り)— dE/dx の最大は止まる端の 14–15 mm 手前 ③ 点の数は当てはめ中固定 ④ ROI から飽和 strip とチャンネルの無い strip を除く ⑤ 核の床を引いて 2 点平均(195 の指摘)⑥ tree に `status[2]`・`ncalls[2]`・`nSatStrips`・`energy`、TParameter に run 定数と核の面積 ⑦ Minuit2 / Migrad 許容 0.1・上限 4,000 回、σ ∈ [0.3, 5]、A ≥ 0、sampling は 25 / 12.5 だけ。
- **判断が要るところ(implementer の推し)** → 198 へ: 有感の範囲(推し = ケージの箱 x ± 53・y −58〜48 で模型を切る)、固定 ROI からの逃げ(推し = 端点を ROI の内側に縛る)、σ_T の扱い(推し = 帯のまま σ_xy = √(σ_T² + 0.146) で換算)、A を run ごとに固定するか(壁 α で 85.3 k ± 7.6 % — Fable キュー)、χ² の相関(halo・共通モード・pad の模様が χ²/ndf を支配 — 雑音 / 模型の誤差の扱い)。
- 実行できなかった操作: なし。
