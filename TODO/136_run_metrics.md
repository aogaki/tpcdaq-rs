# 136 — run 1 本 → 指標 JSON(`_raw.root` の帯位置・クリップ率・飽和率 + macros の tracks から飛跡の統計)

**Status: READY**(2026-10-14 起票 Fable。135 のスキャンの各点を数字にする側。Opus、1 日。macros はユーザーのもの — **読むだけ、呼ぶだけ、改変しない**)

## 0. 入力と置き場
- root-sink の raw ツリー(SPEC §6.7、`<run>_raw.root`): `eventId/i`、`eventTime/l`、`adc[4][68][512]/S`(生 ADC、FPN 込み、未読は −1)。mini = 1 AsAd(ch 0..67、FPN = {11,22,45,56})。
- ユーザーの macros(リポ `macros/`、読み取り専用。ELITPC の写しは `~/test/ana/ana_107/`、`channel_map.csv` 同梱): `analyzeUVW(rawFile, pressureMbar)` → `_uvw.root`(utree: `qview[3]`, `width[3]`, `nhits[3]`, `slView/slZ/slS/slQ`)、`makeTracks(uvwFile)` → `_tracks.root`(ttree: `ok`, `xStart..zEnd`, `dirX/Y/Z`, `length`, `charge`, `dEdx`, `*F`)。107 で ELITPC(ROOT 6.40 / gcc 15)で動作確認済み。
  注意: analyzeUVW の時間 → z は 25 MHz 前提(現用は 25 MHz なので整合。run ≤ 56 は 100 MHz)。

## 1. 作るもの — `tools/scan/run_metrics.C`(ROOT マクロ、C++17、`root -b -q 'run_metrics.C("<raw.root>","<out.json>",<pressure>)'`)+ 薄いシェル `tools/scan/metrics_for_run.sh <run_dir> <out_dir>`
- **raw から(macros 非依存、全イベント)**:
  - 1 イベントの各信号 ch(FPN 除く)について: pedestal = cells 1..20 の平均、hit = ped + 35 ADC 超のセルがある ch(src/reco と同じ定数: ADC_THRESHOLD 35、ped cells 1..20)。
  - **帯の位置**: イベントの最初のヒットセル(全 ch の min)と最後(max)。分布の中央値・10/90 % 点を出す(080 B-4 の c = 窓 − delay − L)。
  - **窓端クリップ率**: 最初のヒットが cell ≤ 2 または最後が ≥ 509 のイベントの割合(前・後ろ別々に)。
  - **飽和率**: hit セルのうち ADC == 4095 の割合、飽和 ch を 1 つでも含むイベントの割合。
  - **パルス幅**: ヒット ch ごとの閾値超の連続セル数の中央値(peakingTime スキャンの指標)。
  - **ヒット ch 数 / イベント**の中央値、総電荷(ped 減算後の閾値超の和)の中央値。
  - イベント数、`eventTime` の最初と最後(→ レート)。
- **tracks から(あれば)**: `metrics_for_run.sh` が `_uvw.root` / `_tracks.root` が無ければ macros を呼んで作る(`root -b -q 'analyzeUVW.C("<raw>",<p>)'` → `makeTracks.C`。macros のパスは引数 `--macros <dir>`、既定 `~/test/ana/ana_107`)。run_metrics.C は `_tracks.root` があれば読む:
  - ok 率、`length` の中央値・IQR、`charge` と `dEdx` の中央値・IQR、dip 角 `atan2(dirZ, sqrt(dirX²+dirY²))` の RMS、φ の RMS。
- 出力 JSON(1 run 1 ファイル、キーは固定。無い値は null): `{run, raw_file, events, rate_hz, first_hit_cell: {median, p10, p90}, last_hit_cell: {...}, clip_front, clip_back, saturated_sample_frac, saturated_event_frac, pulse_width_cells_median, hit_channels_median, charge_median, tracks: {n, ok_frac, length: {median, iqr}, charge: {median, iqr}, dEdx: {median, iqr}, dip_rms_deg, phi_rms_deg} | null}`。
- ホットパスではないが 1,500 events × 4×68×512 の走査なので、ROOT の配列ブランチを `SetBranchAddress` で直読み(TTreeReader の配列でも可)。1 run 10 s 以内が目安。

## 2. テスト(TDD)
- `tools/scan/test_run_metrics.C`(`root -b -q` で走る、`check.hpp` 流儀でなくてよい — assert と exit code): **合成 raw.root を自分で書いて**(10 イベント、既知のヒット位置・飽和・幅)期待値ちょうどになること。
  - 帯の位置と幅: ヒットを cell 100..130 に置く → first 100 / last 130 / 幅 31。
  - クリップ: 1 イベントを cell 0 から、1 イベントを 511 まで → clip_front 0.1 / clip_back 0.1。
  - 飽和: 1 ch を 4095 → 比率が合う。
  - FPN ch(11/22/45/56)は無視される。
  - tracks 無し → `tracks: null`。
- 実データ: ELITPC の `run0077_raw.root`(α、25 MHz、delay 1500)で走らせ、数値が常識的(帯が窓内、クリップ ≪ 10 %)であることを報告に書く。**macros の呼び出しは `~/test/ana/ana_107` を使い、リポの macros/ は触らない**。

## 3. 受け入れ
合成テスト green、実データ 1 run の JSON と所要時間、README に 5 行(入出力・前提・定数の出典)。`tools/scan/` の C++ は ROOT に閉じる(Rust に漏らさない)。

## 4. 報告
変更ファイル、テスト、赤の実測、実データの JSON、**実行できなかった操作**(無ければ「なし」)。

## 5. 追補(2026-10-14、実データ run0076 で発覚 — Fable): ヒット定義を macros / src/reco の流儀に寄せる
- 実測: `hit_channels_median 250`(信号 ch は 256)、`last_hit_cell 511 / clip_back 1.0`、`charge_median 1.6e6`。
- 原因(生波形): 全 ch で基線が窓の中で **20〜40 ADC 単調に上がる**(cell 1 → 511 で 480 → 511、442 → 477、358 → 387)+ ノイズ ±10。common-mode なので FPN ch で引ける。
- 直し方:
  1. **FPN で common-mode を引く**: AGET ごとに FPN 4 ch {11,22,45,56} のセル毎平均を作り、同 AGET の各信号 ch から引く。その後 ped = cells 1..20、閾値 35。partial 読み出しで全セル 0 の ch は除く。
  2. **電荷の下限**: ピーク ±5 セルの ped 減算後の和 > 2000 の ch だけをヒットに(macros STRIP_Q_MIN、`src/reco/mod.rs` の halo cut と同じ定数、出典をコメントに)。
  3. **帯の位置はピークセル**: first / last = ヒット ch のピークセルの min / max。クリップは「ピークが cell ≤ 5 / ≥ 506」と「閾値超が端に掛かる」の両方(`clip_front_peak` / `clip_front_edge`、back も同様)。
  4. **向きを畳む**: φ は 180° で畳んだ軸角の RMS(`phi_axial_rms_deg`)、dip は |dip| の RMS(`dip_abs_rms_deg`)。makeTracks の頭尻はコイン投げ。
  5. `pressure` 引数は残す。
- テスト: 合成 raw に全 ch 共通の基線の傾き(+40 まで、FPN にも同じ)を入れ、修正前は全 ch ヒット・last 511 になる(red)、修正後は置いたヒットだけ。電荷 < 2000 の小パルスはヒットにならない。φ を 180° 反転させた飛跡を混ぜても axial RMS が変わらない。
