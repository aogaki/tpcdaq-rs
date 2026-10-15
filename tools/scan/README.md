# tools/scan — パラメータスキャンのドライバ(TODO/135)

設定点の列を回して run を取る。Python 3 標準ライブラリのみ(ELITPC でそのまま動く)。

- 使い方: `python3 scan.py examples/trigger_delay.json --url http://127.0.0.1:8080 --out-dir ./out`
  - `--dry-run` = PUT 本文と手順を表示するだけ(何も送らない)/ `--resume out/scan_<name>.json` = ok=true の点を飛ばす
  - passphrase は `--passphrase-file`(既定 `/opt/tpcdaq/daq.toml` の `passphrase = "…"`)。ログ・JSON には出さない
- 各点: Idle 確認 → `PUT /api/params`(base の knobs を点の knobs で上書き)→ `run/start` → status を 2 s ごとに読む →
  receiver `frames` ≥ `stop.events` または経過 ≥ `stop.max_s` で `run/stop`。最後に **base を PUT し直す**(失敗は大きく表示)。
- 異常停止(run を stop し base に戻して exit 2): PUT/start/stop が 2xx でない、phase が Idle でない、decoder `malformed > 0`、
  `acquisition_restarts > 0`、`late_fragments > 0`、frames が 30 s 伸びない。出力は `scan_<name>.json`(1 点ごとに書き直す)。
- **HV / LV / power 系の API は一切呼ばない**(SPEC §11)。振るのは GET のパラメータだけ。
- テスト: `cd tools/scan && python3 -m unittest`

## run 1 本 → 指標 JSON(TODO/136)

- 使い方: `tools/scan/metrics_for_run.sh [--macros DIR] [--pressure MBAR] <output_root>/run0077 OUT_DIR` → `OUT_DIR/run0077_metrics.json`
  (中身は `root -l -b -q 'run_metrics.C("…/run0077_raw.root", "out.json", 100)'`。所要は 1,500 events で約 1 s)。
  `_tracks.root` が OUT_DIR に無ければ `--macros`(既定 `~/test/ana/ana_107` — 読むだけ・そこで実行するだけ)の analyzeUVW.C →
  makeTracks.C で作る。raw は OUT_DIR へのシンボリックリンクで渡すので run ディレクトリは汚さない。macros が無ければ WARNING + `tracks: null`。
- 入力の前提: raw ツリー(SPEC §6.7 `adc[{4×nAsad}][68][512]/S`、`eventTime` = 10 ns 刻み)。先頭次元はリーフから読む(mini / ELITPC 共通)。
- ヒットの定義(136 §5 — 実データで全 ch の基線が窓の中で 20〜40 ADC 上がる common mode が出たので macros / src/reco に寄せた):
  AGET ごとに読めた FPN ch のセル毎平均を引き(FPN が未読の AGET は引かず、数を WARNING に出す)、ped = cell 1..20 の平均、
  v = ADC − FPN − ped。ピーク(v 最大の最初の cell)が 35 超、かつピーク ±5 の閾値超の和 q ≥ 2000 の ch = ヒット ch。全セル 0 の ch(部分読み出しの未読)は除く。
- raw の指標(macros 非依存): 帯の位置 `first/last_hit_cell`(ヒット ch のピーク cell の min / max の median・p10・p90)、
  窓端クリップ率(÷ 全イベント)`clip_front_peak` / `clip_back_peak` = ピーク ≤ 5 / ≥ 506、`clip_front_edge` / `clip_back_edge` = ヒット ch の
  閾値超セルが ≤ 2 / ≥ 509。飽和率(ヒット ch の閾値超セルのうち生 4095)、パルス幅(ヒット ch ごとの最長連続)、ヒット ch 数・総電荷(Σ v)の中央値、レート。
- tracks の指標(ok の飛跡だけ): length / charge / dEdx の median・IQR。頭尻(dir の符号)は makeTracks 6 のコイン投げなので向きを畳む —
  `dip_abs_rms_deg` = |dip| の RMS、`phi_axial_rms_deg` = φ を (−90, 90](makeTracks 2 の phi の値域)に畳んだ軸角の RMS(√平均二乗)。
- 定数の出典: ADC_THRESHOLD 35・PED_LO..=PED_HI 1..20・PEAK_HALF_CELLS 5・STRIP_Q_MIN 2000(halo cut、q < 2000 を捨てる)=
  `src/reco/mod.rs` 冒頭(= macros/analyzeUVW.C)、FPN {11,22,45,56} = `src/geometry.rs` FPN_RAW_CHANNELS、4095 = `src/msg.rs` ITEM_ADC_MAX、
  クリップの 5 / 506・2 / 509 = TODO/136 §5-3 / §1。分位点は線形補間 h = (n−1)·q。
- テスト: `root -l -b -q tools/scan/test_run_metrics.C`(合成 raw / tracks を一時ディレクトリに書く。終了コード = 失敗数)

## パルサー run → 応答核と形の指標(TODO/138 §2)

- 使い方: `root -l -b -q 'tools/scan/pulser_kernel.C("…/run0090_raw.root", "run0090_pulser.json", "run0090_pulser.png")'`
  (第 4 引数 `maxEvents` = 先頭から何イベント読むか、0 = 全部。PNG は `nullptr` で省略)。終了コード 0 = 書いた、1 = 失敗(理由は stderr)。
- 入力の前提・定数・FPN の扱いは run_metrics.C と同じ(それを include して使う): AGET ごとに読めた FPN ch のセル毎平均を引き、
  信号 ch ごとに ped = cell 1..20 の平均を引いた v。ピーク = v 最大の最初の cell、振幅 A = v(ピーク)。
  **A > 100 ADC の ch を使い、そのうち生 ADC に 4095 を含む ch は飽和として数えて除く**。全セル 0 の ch(未読)は除く。
- 使う ch の v をピークで揃え(offset k = −40 … +300 cell)、AGET ごと・全体で全イベントを足して平均 —
  生の平均 `mean_raw` と、波形ごとに A で割った正規化平均 `mean_norm`。ピークが読み出し窓の端に寄って
  はみ出した offset は在るものだけで平均し、その数を `n` に出す(全体で欠けた offset があれば WARNING)。
- 指標(AGET ごと `agets[]` + 全体 `all`。形は `mean_norm` から): `waveforms` / `channels`(使った波形 = イベント × ch / 使った ch)、
  `saturated_waveforms` / `saturated_channels`、`amplitude_median_adc`、`rise_10_90_cells`(ピークの前で 0.9 と 0.1 を下回る交点の差)、
  `fwhm_cells`(0.5 の交点、前後)、`undershoot`(k = +1…+300 の `mean_norm` の最小 `min_ratio`、その k の `mean_raw` = `min_adc`、その k = `cell`)、
  `tail_50_300`(k = +50…+300 の **0 を基準にした** RMS √(Σy²/個数) とピーク間 max − min、`_ratio` = `mean_norm`、`_adc` = `mean_raw` — 25 MHz で 2 µs 以降の「揺れ」)、
  `peak_cell`(使った波形のピーク cell の median / p10 / p90)、`peak_cell_range_in_event`(イベント内の ch 間のピーク cell の max − min の分布 = パルサーの同時性)、
  `mean_norm` / `mean_raw` / `n`(341 点の配列、値が無い所は null)= 139 デコンボリューションの核。交点はすべて隣のセルとの線形補間、分位点は h = (n−1)·q。
- **FPN の扱い(第 5 引数 `subtractFpn`、既定 true)**: AsAd 内蔵パルサーは FPN ch {11,22,45,56} にもパルスを入れる(振幅は信号 ch の ≈ 1.5 倍)ので、
  pulser run は `subtractFpn=false` で使う(`pulser_kernel("…_raw.root", "out.json", nullptr, 0, false)`)。共通モード減算をせず ped(cell 1..20)減算だけにし、
  FPN ch 4 本 × AGET を別グループ JSON 上位キー `fpn`(`all` と同じ構造)に集計する(`all` / `agets[]` には入れない)。減算するとき(α run 等)は `fpn` = null。
  JSON に `subtract_fpn`。`agets_without_fpn` の WARNING は減算するときだけ。
- PNG: 正規化平均を AGET ごとに 1 本重ねた 1 枚(線形、x = −40…+300、y = −0.3…1.05)。
- 定数の出典: 100 ADC・窓 −40…+300・尾 +50…+300 = TODO/138 §2 の発注。他は run_metrics.C と同じ(上の節)。
- テスト: `root -l -b -q tools/scan/test_pulser_kernel.C`(区分線形の既知の形 — 10–90 % 6.4 cells、FWHM 16.5、
  アンダーシュート −0.1 @ +30、非対称な尾の揺れ — を 20 イベント × 数 ch、全 ch に common mode の傾き、飽和 ch、100 ADC ちょうどの ch、
  FPN だけのおとりを置いた合成 raw を一時ディレクトリに書く。終了コード = 失敗数)

## α run → 最初 / 最後のヒット cell とドリフト端(CURRENT「今日の進行」A、12.5 MHz ドリフト端 run)

- 使い方: `root -l -b -q 'tools/scan/drift_end.C("…/run0100_raw.root", "run0100_drift_end.json", "run0100_drift_end.png")'`
  (第 4 引数 `maxEvents` = 先頭から何イベント読むか、0 = 全部。PNG は `nullptr` で省略)。終了コード 0 = 書いた、1 = 失敗(理由は stderr)。
- 入力の前提・定数・FPN の扱いは run_metrics.C と同じ(それを include して使う): AGET ごとに読めた FPN ch のセル毎平均を引き、
  信号 ch ごとに ped = cell 1..20 の平均を引いた v。全セル 0 の ch(未読)と FPN ch は除く。
- **ヒット cell = v > 35 の cell のうち cell [2, 508) だけ**(先頭の整定ノイズと窓端 4 cell(508..511)の跳ねを除く。JSON `hit_cell_range: [2, 508]`)。
  **ch 条件**: run_metrics.C の「ヒット ch」(ピーク > 35 かつ q ≥ 2000)を満たす ch の cell だけ数える(同じ定数・同じ計算、JSON `channel_condition: "run_metrics"`)。
  cell < 508 に限るので `clip_back_frac` は構造的に 0。イベントの first = 全ヒット ch のヒット cell の最小、last = 最大。
  ヒットの無いイベントは `events` に数えるだけで分布に入れない(`events_with_hits`)。
- 出力: `first_hit_cell`(median / p10 / p90)、`last_hit_cell`(median / p10 / p90 / p99)、`first_hit_hist` / `last_hit_hist`(cell 0..511 ごとのイベント数、512 個)、
  `edge` = last の分布の上端 — `cell_p99`(上から累積 1 % = `last_hit_cell.p99`)、`width_cells`(上端の立ち下がり幅 = 上から累積 1 % と 10 % の cell の差 = p99 − p90)、
  `t_max_cells` = `cell_p99 − first_hit_cell.median`(トリガ帯からドリフト端までの cell 数 → v_drift = L / t_max)。
  `clip_back_frac` = last ≥ 509 のイベント数 ÷ ヒットのあるイベント数(窓で切れていて端が見えていない割合。`cell_p99` ≥ 509 なら WARNING)。
  分位点は線形補間 h = (n−1)·q。値が無ければ null。
- PNG: first / last のヒストを重ねた 1 枚(x = cell、対数 y)。
- テスト: `root -l -b -q tools/scan/test_drift_end.C`(101 イベントで first = 40 + e mod 21、last = 100 + e と窓端 508 / 509 / 510 / 511 の矩形パルス、
  全 ch に common mode の傾き、窓端の跳ね(cell 508–511)・先頭の整定(cell 0–1)・単独 cell のノイズ(q < 2000)・閾値ちょうど 35 と FPN ch のおとり、ヒット無しの 3 イベント、maxEvents = 50 で補間の分位点、ヒット無しだけの run を
  一時ディレクトリに書く。終了コード = 失敗数)

## α run → 整形核の逆畳み込み(TODO/139 §1)

- 使い方: `root -l -b -q 'tools/scan/deconv.C("…/run0084_raw.root", "run0105_kernel.json", "run0084_l0.01.json", 0.01, "run0084_l0.01.png")'`
  (第 4 引数 = λ_rel、第 5 = PNG(`nullptr` で省略)、第 6 = `maxEvents`(先頭から何イベント、0 = 全部))。終了コード 0 = 書いた、1 = 失敗(理由は stderr)。
  核 JSON = `pulser_kernel.C` の出力(α run と同じ peaking の核)。λ は 3 点(例 `0.001` / `0.01` / `0.1`)で 3 回呼ぶ — JSON は run × λ ごとに 1 本。
- **速さ**: ROOT が FFTW 付きなら `TVirtualFFT` で速い。FFTW 無し(`TVirtualFFT::FFT` が null — Mac の /opt/ROOT がそう。
  stderr に `Error in <TVirtualFFT::FFT>: handler not found` と `deconv: INFO … O(N^2) DFT fallback`、JSON `"transform": "dft"`)だと
  素朴な O(N²) DFT で **1 波形 15 ms(cling)/ 1.5 ms(ACLiC = `'tools/scan/deconv.C+(…)'`)**(Mac 実測)。1,500 events × 200 波形なら
  cling ≈ 75 分、ACLiC ≈ 8 分 / λ。ACLiC の生成物(`*.so` / `*.d` / `*.pcm`)は .gitignore 済み。`maxEvents` で間引ける。
- 核: `all.mean_norm`(341 点、k = −40 … +300 cell、index 40 = ピーク = 1)だけを読む(`fpn` / `agets` は読まない)。
  尾 k = +50 … +300 の平均(≈ +1 % の一定オフセット — パルサー run の ped を cell 1..20 で取るための基線のずれ)を **341 点すべてから**引く
  (JSON `kernel_offset`)。長さ 341 でない・null がある・最大が index 40 でない核は失敗。
- 方法(Wiener 型): N = 1024 の円環で H = DFT(h を k mod N に置く)、`x̂ = IDFT(DFT(y)·H*/(|H|² + λ_abs))` の cell 0..511。
  y = 前処理後の 512 cell、512..1023 は 0。**λ_abs = λ_rel × max|H|²**(JSON `lambda_rel` / `lambda_abs` / `max_h2`。λ_rel = 0 = 厳密な逆)。
  |H|² + λ_abs = 0 の周波数は W = 0 にして `zero_denominator_bins` に数える(WARNING)。
  x̂ の単位は「ピーク相当 ADC」: 核のピークを 1 にしてあるので、1 cell に来た点電荷は x̂ でも元の波形のピーク高さになる
  (幅のある電荷は低く広く出る)。非負の核は max|H| = H(0) なので **x̂ の総和 = Σy / Σh / (1 + λ_rel)**(Σh = JSON `kernel_sum`)
  — λ_rel = 0.1 なら電荷は 1/1.1 に縮む(run 間の比較は同じ λ_rel どうしで)。
- 前処理とヒット ch は run_metrics.C と同じ(pulser_kernel.C 経由で include): FPN のセル毎平均と ped(cell 1..20)を引いた v、
  ヒット ch = ピーク > 35 かつ q ≥ 2000。それ以外の読めた信号 ch = 雑音 ch(「ped 領域(ヒット無し ch)」)。全セル 0 の未読 ch は除く。
- 指標(前 = v、後 = x̂。和・RMS・ピーク・FWHM はすべて cell **[2, 509)** — drift_end.C と同じ範囲、JSON `cell_range`):
  `charge_event` = イベントごとの Σ(ヒット ch の和)の median / p10 / p90(ヒットのあるイベント)と `ratio_median`(後 / 前)、
  `fwhm_cells` = ヒット ch ごとの FWHM(ピーク = 最大の最初の cell、半値の交点は線形補間)、
  `ped_rms` = 雑音 ch ごとの RMS √(Σw²/507) の中央値と `ratio`(後 / 前 = 点電荷に対する S/N の悪化率)、
  `peak_cell_spread_in_event` = イベント内のヒット ch のピーク cell の母標準偏差(ヒット 2 本以上のイベント)。分位点は h = (n−1)·q。
  **注意**: partial readout で読めた非ヒット ch はハードのしきい値を越えた halo なので、`ped_rms` は純粋な雑音より大きく出うる
  (`ped_rms.before` が ≈ 3 ADC から離れていれば halo が支配)。
- PNG: ヒットのある最初のイベントで前のピークが最大の ch の前後重ね(x = cell 0..511)+ 指標の表。
- テスト: `root -l -b -q tools/scan/test_deconv.C`(約 10 s。[1] 核 JSON の読み込み(オフセットを全点から、おとりの fpn / agets、
  無いファイル・`all` 無し・null・340 点・ピーク位置違い・閉じない配列)、[2] CR-RC⁴ 核 × 矩形 + 指数尾の電流で λ = 0 の厳密な逆・
  電荷 = Σx/(1+λ_rel)・σ = 2 の雑音で λ_rel = 1e-3 の電荷 1 % 以内・λ_rel 1e-3 → 1e-1 で雑音 RMS が下がり幅が広がる、
  [3] 二進の短い核で全セル整数になる合成 raw(partial readout、FPN のおとり、halo、ヒット 1 本のイベント、ヒット無しのイベント)で
  電荷・FWHM・雑音 RMS(手計算 3·|W(π)|)・ピーク cell のばらつき・PNG の波形。終了コード = 失敗数)

## α run → ストリップヒットの z 残差(TODO/146 §2)

- 使い方: `root -l -b -q 'tools/scan/hit_residuals.C("…/run0083_uvw.root", "…/run0083_tracks.root", "run0083_hit_residuals.json", "run0083_hit_residuals.png")'`
  (第 4 引数 = PNG(`nullptr` で省略)、第 5 = `maxEvents`(先頭から何イベント、0 = 全部))。終了コード 0 = 書いた、1 = 失敗(理由は stderr)。
- 入力 = ユーザー macros の出力(**macros は読むだけ**): analyzeUVW.C の `_uvw.root`(`uvw`: `eventId`、`width[3]`、`slView/slZ/slS/slQ`)と
  makeTracks.C の `_tracks.root`(`tracks`: `eventId`、`ok`、`xStart..zEnd`)。同じ run であること(エントリ数と各エントリの eventId を
  突き合わせ、違えば失敗)。`ok == 1` に、どれかの view のヒットが無ければ失敗(別 run の組み合わせ)。
- 幾何は makeTracks.C と同じ式(冒頭のコメントに行番号): s = pitch_v·(x, y)、pitch_U = (−1, 0)、pitch_V = (0.5, +√3/2)、pitch_W = (0.5, −√3/2)。
  ヒットの **l = (s − ms_v)/(pitch_v·d_xy)**、ms_v = その view の全ヒットの電荷加重平均の s。tracks には ms_v と anchor m
  (pitch_v·m = ms_v の最小二乗)が残っていないので uvw のヒットから作り直し、start / end を l に戻して(l = (点_xy − m)·d_xy、
  d_xy = (end − start)_xy の単位ベクトル)z(l) = z0 + tanθ·l を復元する。残差 **r = slZ − z(l)**。
  start 点を pitch に素朴に射影する方式は makeTracks と一致しない(view ごとに tanθ·(Σ ms_v / 3)/proj_v のずれ)。
- 使うヒット: `ok == 1`、|pitch_v·d_xy| ≥ 0.3(makeTracks の MIN_PROJ。外は `hits_cut_proj`)、l が [start, end] の中
  (= makeTracks 5 の l の 2–98 % 電荷分位。閉区間、float の丸めに 1e-3 mm の許容。外は `hits_cut_ends`)。
  start == end(xy)は向きが決まらないので `events_degenerate` に数えて使わない(WARNING)。
  **注意**: makeTracks の分位(`quantileRange`)は両側とも累積を含めて数えるので、1 本の電荷が全体の 2 % を超えるふつうのイベントでは
  lo = l の最小(低い側は 1 本も切らない)、hi = 最後から 2 番目(高い側の 1 本だけ切る)。端の切り方は makeTracks の d_xy(φ ∈ (−90°, 90°])
  について非対称。
- 出力 JSON: `run`、`events`(読んだエントリ)、`events_ok`、`events_degenerate`、`hits`(使ったヒット)、`hits_cut_ends`、`hits_cut_proj`、`min_proj`、
  `residual_mm` = `all` / `U` / `V` / `W` の `{n, rms, robust_sigma, median}`(RMS = √(Σr²/n)、0 基準。robust σ = 1.4826 × MAD、
  MAD = |r − 中央値| の中央値。中央値 = バイアス)、`charge_quartiles` = 使ったヒットの slQ の 4 分位
  [min, q25] / (q25, q50] / (q50, q75] / (q75, max] ごとの `{q_lo, q_hi, n, rms, robust_sigma, median}`(S/N 依存)、
  `width_median_mm` = uvw の `width[3]` の view ごとの中央値(使ったイベント)。分位点は線形補間 h = (n−1)·q。値が無ければ null。
- **σ は少し小さめに出る**: 直線は makeTracks がそのヒットも含めて当てはめたもの(自己相関)。ヒットが多い(≥ 20)ので補正は小さい。
- PNG: 左 = 残差のヒスト(view 別に重ねる、範囲 ±5 robust σ)、右 = 残差 vs 電荷。
- テスト: `root -l -b -q tools/scan/test_hit_residuals.C`(約 1 s。[1] makeTracks の l の座標系で作った合成 uvw / tracks —
  view ごとに勝手な ms_v、start / end の入れ替え、ok=0 のゴミ、|proj| < 0.3 の view、端の外のヒット(重み無しの平均だと view ごとに違うずれ)、
  端点ちょうどのヒット、start == end — で view・全体・電荷 4 分位の RMS / robust σ / 中央値、width の中央値、数え上げを手計算で固定、maxEvents = 1、
  [2] σ = 0.8 mm のガウス残差 7200 本で robust σ と RMS が 5 % 以内、[3] 真の幾何(view ごとの原点のずれ)で書いた uvw に
  **本物の `macros/makeTracks.C` を実行して**作った tracks で、残差が入れたとおり(U = ρ、V = −ρ、W = 0)に戻る、
  [4] 無いファイル・エントリ数 / eventId の不一致・ヒットの無い view、[5] 使えるイベントが無い run で null。終了コード = 失敗数)

## α run → U/V/W の閉合残差 = 2D 重心の精度(TODO/148 §2)

- 使い方: `root -l -b -q 'tools/scan/uvw_closure.C("…/run0083_raw.root", "…/channel_map.csv", "…/run0083_tracks.root", "run0083_uvw_closure.json", "run0083_uvw_closure.png")'`
  (第 5 引数 = PNG(`nullptr` で省略)、第 6 = `maxEvents`(先頭から何エントリ、0 = 全部))。終了コード 0 = 書いた、1 = 失敗(理由は stderr)。
- 原理: pitch_U + pitch_V + pitch_W = 0(macros/makeTracks.C:7-8: (−1, 0)、(0.5, +√3/2)、(0.5, −√3/2))なので、同じ点のストリップ座標の和
  **s_U + s_V + s_W = c**(map の view ごとの原点の和、点に依らない)。同じ時刻スライスの 3 面の電荷重心の和の広がり = 飛跡モデルに依らない
  2D 重心の精度。3 面が独立に σ_s なら σ_r = √3·σ_s。pitch の値そのものは使わない(map の s をそのまま足す)。146(z 残差)の 2D 版。
- 入力: raw ツリー(`eventId`、`adc[N][68][512]/S`)、`channel_map.csv`(読み方は macros/analyzeUVW.C:97-118 と同じ — `#` 行と空行を飛ばし、
  `aget,raw_channel,view,strip,s_mm`、view 0/1/2 = U/V/W、aget = adc の先頭の index、s_mm = slS になる値)、makeTracks.C の `_tracks.root`
  (`eventId`、`ok`。raw のエントリ i = tracks のエントリ i — エントリ数と eventId を突き合わせ、違えば失敗)。**`ok == 1` のイベントだけ使う**。
  map は黙って読み飛ばさない: 読めない行・view ∉ {0,1,2}・ch ∉ [0, 68)・aget < 0・FPN ch・(aget, ch) / (view, strip) の重複・
  ストリップの無い view・raw に無い aget は失敗(analyzeUVW は読めない行を飛ばす)。
- 前処理は run_metrics.C と同じ(include して使う): AGET ごとの FPN のセル毎平均と ped(cell 1..20)を引いた v。全セル 0 の ch は除く。
- スライス = cell **[2, 508)**(drift_end.C と同じ範囲)を `kSliceCells`(既定 1)cell ずつ。ストリップの重み w = スライス内で **v > 35 の cell の v の和**
  (閾値は cell ごと)。view ごとに w > 0 のストリップを strip 番号順に見て、番号が 1 ずつ続く塊を数え、重心 s_v = Σw·s/Σw。
  何かあるスライスのうち、0 本の view がある → `slices_missing_view`、そうでなく塊が 2 つ以上の view がある → `slices_multi_cluster`
  (両方に当たるときは view 不足)、3 面とも 1 塊 → 採用。
  **ヒット ch(第 7 引数 `hitChannelsOnly`、既定 true)**: イベントごとに run_metrics.C / drift_end.C と同じ「ヒット ch」(ピーク > 35 かつ ピーク ±5 cell の q ≥ 2000、FPN + ped 減算後)を満たす ch だけを重心の候補にし、満たさない ch は全 cell で無視(halo — 飛跡の時刻に全ストリップに乗る 35–200 ADC、analyzeUVW.C:67-73 — を除く)。
  false = 閾値 35 だけで選ぶ以前の挙動(halo は `slices_multi_cluster` / `strips_median` に出る)。JSON に `hit_channels_only` と `channels_used_median`(ok イベントあたりの候補 ch 数の中央値。false のときは読めたストリップ数)。
- **cell 条件(hitChannelsOnly の true / false 共通)**: スライス t で ch を使うのは **v(t) > 35 かつ |t − tpeak| ≤ `kSliceWindowCells`**(15 cell ≈ 固有幅 14.5、
  tpeak = その ch の FPN + ped 減算後の v が最大の cell)。別の z に本体がある ch の halo / 尾を、そのスライスの重心から外す。JSON に `slice_window_cells`。
  **限界**: 1 ch に長い堆積(斜めの飛跡が同じ ch に数十 cell 載る)があると、その ch はピーク ±15 cell のスライスにしか寄与しない(精度の測定としては近傍の S/N の良いスライスに偏る)。
- 出力 JSON: `run`、`events`(読んだエントリ)、`events_ok`、`slice_cells`、`cell_range`、`adc_threshold`、`slice_window_cells`、`slices`(採用)、`slices_missing_view`、
  `slices_multi_cluster`、**`c_mm`** = 和の中央値、`residual_mm` = r = 和 − c の `{n, rms, robust_sigma}`(RMS = √(Σr²/n)、c 基準。
  robust σ = 1.4826 × MAD)、**`sigma_per_view_mm`** = robust σ / √3、`charge_quartiles` = スライス電荷(3 面の Σw)の 4 分位
  [min, q25] / (q25, q50] / (q50, q75] / (q75, max] ごとの `{q_lo, q_hi, n, rms, robust_sigma, median}`、`strips_median` = view ごとの
  塊の本数(重心に使った幅)の中央値。分位点は線形補間 h = (n−1)·q。値が無ければ null。
- PNG: 左 = r のヒスト(範囲 ±5 robust σ)、右 = r vs スライス電荷。
- テスト: `root -l -b -q tools/scan/test_uvw_closure.C`(約 1 s。合成 channel_map = 3 面 × 24 本・pitch 2 mm・view ごとの原点
  (和 4.5)・map を読まないと解けない (aget, ch) の並び。[1] 4 イベントで c = 4.5、r の robust σ / RMS / σ_s、電荷 4 分位、本数の中央値、
  ok=0・窓の外(cell 0 / 508..511)のおとり、塊 2 つ・1 本の隙間・view 不足・閾値ちょうど 35 の数え上げを手計算で固定、maxEvents = 1、
  [2] 3 面独立に σ = 0.4 mm のガウス 6000 スライスで robust σ ≈ √3·0.4・σ_s ≈ 0.4(5 %)・c ≈ 4.5、[3] スライス幅 2 cell の電荷加重と
  cell ごとの閾値、[4] 無いファイル・エントリ数 / eventId の不一致・map の不正 8 種、[5] 使えるスライスが無い run で null。終了コード = 失敗数)

## スキャン結果 → 台帳の表(TODO/143 §2)
`docs/SCANS_ja.md` に貼る Markdown の表を、scan.py の結果 JSON と run_metrics.C の指標 JSON から生成する(標準出力)。標準ライブラリのみ。

```
python3 tools/scan/ledger.py ~/test/scan/scan_<name>_<date>.json --metrics ~/test/scan/metrics \
    [--run-root /data/tpcdaq] [--title "S10 …"]
```

- 1 点 = 1 行。列 = 点 / run / knobs / events / kB/event / first・last hit / 幅 / ヒット ch / 総電荷 / 飽和 event / クリップ(先頭 / 末尾、edge)/ ok / 長さ / 飛跡電荷 / dEdx / dip RMS。
- knobs は点の上書き(scan JSON の `points[].knobs`)だけ。空なら `(base)`。ネストした値は JSON 1 行。
- 指標は `--metrics DIR/run%04d_metrics.json`。無い run、tracks が null の run も行は残し、無い値は「—」。
- `--run-root DIR` があれば `DIR/run%04d/*.graw` の合計バイト ÷ 指標の events ÷ 1000 を kB/event に(graw が無ければ「—」)。
- 書式: 電荷 = `x.xx M`、比率 = `x.x %`(クリップは `先頭 / 末尾 %`)、dEdx は桁区切り整数、dip RMS は `x.x°`。
- テスト: `python3 -m unittest tools/scan/test_ledger.py`。

## ch ごとの ZS 閾値と ZS 対応の指標(TODO/149)
```
python3 tools/scan/zs_exceptions.py PED.json [--k 5] [--min-above 20] [--out FILE] [--merge BASE.json]
tools/scan/metrics_for_run.sh --ped PED.json RUN_DIR OUT_DIR
```
- `zs_exceptions.py`: `calib pedestal` の JSON から各 ch(FPN 込み)の `zeroSuppression.threshold` 例外
  (`at` = `{asad, aget, ch}`、値 = round(mean + max(k·rms, min-above))、[0, 4095])を作る。`--merge` は base params に差し込み
  `zeroSuppression.enabled = true` にする(base にあった閾値の例外は置き換え)。テスト: `python3 -m unittest tools/scan/test_zs_exceptions.py`。
- `run_metrics.C` の第 4 引数 / `--ped`: ZS データ(閾値未満のセルと FPN が 0)用に ped = 表の mean、FPN 共通モードは引かない。
  表に無い信号 ch は飛ばして数を WARNING に出す。指標 JSON に `ped_source`(`table` / `cells_1_20`)と `zs_suspected`
  (過半数のイベントで読めた AGET の信号 ch の過半数が cells 1..20 全 0)が入り、表なしで疑いがあれば WARNING を 1 回出す。
