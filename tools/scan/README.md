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
