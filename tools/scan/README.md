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
