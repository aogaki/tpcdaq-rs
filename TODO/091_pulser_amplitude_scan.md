# 091 — D-3: AsAd 内蔵パルサーの振幅スキャン(GDED 遠隔、実 zCoBo)→ ゲイン直線性 + 電子回路応答

**Status: IN PROGRESS**(2026-09-23 起票 Fable。080 D-3 / 073-#10 / 066(b) 前半。データ取得 = Fable(遠隔)、解析 = Sonnet)

## 0. 取得(2026-09-23、GDED、Rust ecc_bridge + 実 ECC ネイティブ、VM の GET は停止)
- 構成は 076 結果節(Phase 3 前提)と同じ。1 run = 1 振幅、`asad-pulser-start <mV> 1000`(周期 1000 ms)、60 s(≈ 60 frames、frameType 2 compact、全 ch)。
- 振幅: **−100 / −200 / −300 / −500 / −700 / −1000 mV**(`scan_d3.sh`)。run 番号・ファイルは下の結果節。
- 注意: run 5(12:04–12:06)はスクリプトの変数展開ミスで `asad-pulser-start ${3:--1000}` を送っており **無効**(直前の −1000 mV 設定が残っていた可能性)。解析から除外。
- 2026-09-14 の run 1/2(−1000 mV、122 + 20 frames)も同条件の参照点として使える(`out/run0001`, `out/run0002`)。

## 1. 解析発注(Sonnet、2026-09-23 — データは Mac の `/Users/aogaki/Workspace/tpcdaq-rs/reference/exp_data/2026/d3_pulser_scan/run00{06..11}/*.graw`)

先に Fable が `calib pulser`(081)で見た数字(signal ch 中央値、FPN 除く): −100 mV → **369.0** ADC / −300 → **1074.0** / −1000 → **3239.8**(飽和 ch 0)。
比 −300/−100 = 2.91、−1000/−100 = 8.78(高振幅側で圧縮)。FPN の中央値は 543.6 / 1589.8 / 3427.0。= 梯子は本物。

### 作るもの
1. `src/calib/scan.rs` + `calib scan` サブコマンド(`src/bin/calib.rs`): 入力 = `--amp <mV>=<graw>` を複数(または `--runs <json>`)。各 run を既存の
   `calib::pulser` と同じ復号で読み、**ch 毎**に (a) 振幅中央値(既存)(b) **平均波形 512 点**(ペデスタル = 先頭 25 セル平均を引いた後、イベント平均)を出す。
   出力 JSON: `{meta, amps:[mV], channels:[{asad,aget,ch,is_fpn, amplitude:[per amp], fit:{slope, intercept, max_resid_frac}, waveform:[[512] per amp]}]}`。
   直線性: 振幅 ≤ −700 mV(圧縮前)の点で最小二乗、全点の残差(fraction of fit)を出す。既存の `calib pulser` / `merge` は変えない。
2. `macros/pulserScan.C`(ROOT、JSON は `nlohmann` を使わず `calib scan --csv` で **CSV も出す**方が KISS): 図 = ①ch 毎の応答 vs 振幅(全 ch 重ね、FPN 別色)
   ②残差 vs 振幅(ch 毎の分布、中央値 ± 16–84 %)③平均波形(振幅ごとに 1 本、ch 平均、FPN 除く)④立ち上がり時間(10–90 %)・ピーク位置・FWHM vs 振幅の表。
   出力は `reference/_spike/d3/`(gitignore)に PNG、表は結果節に転記。
3. 参照点: 2026-09-14 の run0001/run0002(−1000 mV)は GDED にあり Mac には無い → 使わない(今日の run 11 で足りる)。

### テスト
- `calib scan` の単体: 合成フレーム(既存 `tests/calib_synthetic.rs` の生成器)で 3 振幅 × 数イベント → slope / intercept / 残差 / 平均波形の手計算オラクル。
- 実データは env `TPCDAQ_EXP_DATA_2026` 配下のときだけ(既存 `tests/calib_real_graw.rs` の流儀): 6 run で 272 ch、飽和 0、−100/−300/−1000 の中央値が上の数字と 1 ADC 以内。
- `cargo fmt && cargo clippy --tests -- -D warnings && cargo test` green(現 838)、production `.unwrap()` 無し。ROOT マクロは `root -l -b -q` で通ること(SDKROOT は
  メモリ mac-root-cling-sdkroot、`macros/README.md` に行を足す)。

### 受け入れ・結果節
ch 毎の slope の分布(中央値・±%)、068 の相対ゲイン(±4.4 %)との整合、残差の最大(圧縮の開始振幅)、平均波形の立ち上がり / ピーク / FWHM の表、図の所在。
設計判断が残る分岐(例: 圧縮のモデル化)は実装せず報告。

## 2. 参照
080 D-3、073 #10、066(b)、081(calib)、069 結果(09-14)、076 結果(09-23)。

## 結果(取得、2026-09-23 12:07–12:14、GDED、Fable)
| run | 振幅 [mV] | frames | bytes | ファイル(GDED `out/`、Mac `reference/exp_data/2026/d3_pulser_scan/`) |
|---|---|---|---|---|
| 6 | −100 | 60 | 16,727,040 | run0006/CoBo0_AsAd0_2026-09-23T12:07:16.744_0000.graw |
| 7 | −200 | 60 | 16,727,040 | run0007/CoBo0_AsAd0_2026-09-23T12:08:31.454_0000.graw |
| 8 | −300 | 60 | 16,727,040 | run0008/CoBo0_AsAd0_2026-09-23T12:09:46.202_0000.graw |
| 9 | −500 | 60 | 16,727,040 | run0009/CoBo0_AsAd0_2026-09-23T12:11:00.883_0000.graw |
| 10 | −700 | 60 | 16,727,040 | run0010/CoBo0_AsAd0_2026-09-23T12:12:15.583_0000.graw |
| 11 | −1000 | 60 | 16,727,040 | run0011/CoBo0_AsAd0_2026-09-23T12:13:30.285_0000.graw |
- 各 run: run/start 7.97–8.04 s、run/stop 3.64–3.66 s、`asad-pulser-start <mV> 1000` の応答は正常、malformed 0 / overflow 0。
  6 run とも 60 frames = 60 × 278,784 B(frameType 2 compact、全 ch)。周期 1000 ms で 60 s → ちょうど 60 イベント(欠損 0)。
- 参考: run 3/4(−1000 mV、076 の確認 run)、run 5 は無効(§0)。GDED の logbook を `logbook_gded_2026-09-23.jsonl` として同梱。
- 取得後、ECC を breakup → reset → reset で Idle に戻し、bench スタック・Rust ブリッジ・ECC を停止(VM の GET 再開可)。

## 結果(解析、2026-09-23、Sonnet レーン)

### 実行コマンド
```
# worktree セットアップ(発注書どおり)
git merge --ff-only main   # 809eb29 → 2e861fd

# TDD: tests/calib_synthetic.rs / tests/calib_real_graw.rs に先にオラクル付きテストを追加 → red 確認
cargo test --test calib_synthetic   # E0432: unresolved import `tpcdaq::calib::scan`

# 実装: src/calib/scan.rs + src/calib/mod.rs(`pub mod scan;`)+ src/bin/calib.rs(`scan` サブコマンド)
cargo fmt && cargo clippy --tests -- -D warnings && cargo test

# 実データ(6 run)を通して JSON + CSV 2 本を生成
./target/release/calib scan \
  --amp -100=.../run0006/CoBo0_AsAd0_2026-09-23T12:07:16.744_0000.graw \
  --amp -200=.../run0007/CoBo0_AsAd0_2026-09-23T12:08:31.454_0000.graw \
  --amp -300=.../run0008/CoBo0_AsAd0_2026-09-23T12:09:46.202_0000.graw \
  --amp -500=.../run0009/CoBo0_AsAd0_2026-09-23T12:11:00.883_0000.graw \
  --amp -700=.../run0010/CoBo0_AsAd0_2026-09-23T12:12:15.583_0000.graw \
  --amp -1000=.../run0011/CoBo0_AsAd0_2026-09-23T12:13:30.285_0000.graw \
  --out reference/_spike/d3/scan.json \
  --csv-channels reference/_spike/d3/scan_channels.csv \
  --csv-waveform reference/_spike/d3/scan_waveform.csv
# -> "wrote .../scan.json (272 channels, 6 amps, 360 events)"

# ROOT マクロ(macOS CLT 27、SDKROOT 必須 — mac-root-cling-sdkroot)
SDKROOT=$(xcrun --sdk macosx --show-sdk-path) root -l -b -q \
  'macros/pulserScan.C("reference/_spike/d3/scan_channels.csv","reference/_spike/d3/scan_waveform.csv","reference/_spike/d3")'

# 実データ回帰(env 経由、任意)
TPCDAQ_EXP_DATA_2026=/Users/aogaki/Workspace/tpcdaq-rs/reference/exp_data/2026 cargo test --test calib_real_graw
```

### テスト数(before → after)
- `cargo test`: **838 → 844 passed**(0 failed、6 本純増)。
  - `src/calib/scan.rs` 単体 3 本: `least_squares_matches_hand_calculation_for_two_points` /
    `least_squares_is_nan_with_fewer_than_two_points` /
    `compute_fit_excludes_the_1000mv_point_and_reports_its_residual`
  - `tests/calib_synthetic.rs` 2 本: `scan_compute_matches_hand_calculation_for_slope_intercept_residual_and_average_waveform`
    (合成 3 振幅 × 2 イベント、slope/intercept/残差/平均波形を手計算オラクルと突合) /
    `scan_table_round_trips_through_a_json_file`
  - `tests/calib_real_graw.rs` 1 本(env `TPCDAQ_EXP_DATA_2026` 配下のときだけ):
    `scan_real_d3_pulser_ladder_matches_the_091_fable_pre_numbers_within_1_adc`
    (272 ch、−100/−300/−1000 の signal・FPN 中央値が §1 の Fable 先行数値と 1 ADC 以内、実測)。
  - 既存 2 本(`pedestal_real_run_matches_the_068_csv_oracle` /
    `pulser_real_run_matches_the_068_csv_oracle_...`)は本作業と無関係に FAIL する
    (このマシンの `reference/exp_data/2026/` には `d3_pulser_scan/` しか無く、068 由来の
    `pedestal/`・`pulser/` ディレクトリが未取得のため — 変更前に stash して同一条件で再現し、
    無関係の環境ギャップであることを確認済み。本チケットのスコープ外なので手を付けていない)。
  - `cargo fmt --check` clean、`cargo clippy --tests -- -D warnings` clean、production
    `.unwrap()` 無し(`scan.rs` は `unwrap_or(0)` / `?` / エラー型のみ)。

### ch 毎 slope の分布と 068 の相対ゲイン(±4.4 %)との整合
- 272 ch(1 AsAd × 4 AGET × 68 raw ch)中、signal 256 ch / FPN 16 ch。
- signal ch の `|fit.slope|` 中央値 = **3.370 ADC/mV**。中央値からの相対ずれ:
  **中央値 1.37 %、最悪 5.19 %**(範囲 −5.09 %〜+5.19 %)。
- 068(`calib pulser` の単一振幅比での相対ゲイン)は中央値 ±4.4 %・最悪 ±8.4 %。本ツールの
  slope は −700 mV 以下(絶対値、圧縮前の 5 点)だけで最小二乗した傾きなので、圧縮の影響を
  受けにくい分だけ 068 よりタイトに出ている(整合: 同じオーダー〔数 %〕、068 の単一振幅比に
  乗る「振幅依存の非線形性」が本ツールでは分離されて外れている、という解釈で整合的)。
- AGET 単位(この run は 1 AsAd のみなので AGET 0–3 の 4 通り): 中央値からの相対ずれは
  −0.21 % / +1.24 % / −0.67 % / +0.52 %(全て 1.3 %未満)。068 が報告する「AGET 単位系統差
  5.3 %・AsAd3 低め」は 4 AsAd × 4 AGET = 16 AGET にまたがる集計なので、1 AsAd だけのこの
  run と直接比較はできない(スコープが違う点を明記)。

### 残差の最大(圧縮の開始振幅)
- 振幅ごとの残差(signal ch, `(actual − fit) / fit`、中央値・最大):

  | 振幅 [mV] | 残差 中央値 | 残差 最大 |
  |---|---|---|
  | −100  | 4.69 % | 5.77 % |
  | −200  | 0.81 % | 1.20 % |
  | −300  | 1.28 % | 1.77 % |
  | −500  | 0.71 % | 1.09 % |
  | −700  | 0.57 % | 0.76 % |
  | −1000 | 5.33 % | 8.34 % |

- **−200〜−700 mV は残差 2 %未満で概ね線形**。**圧縮は −700〜−1000 mV の間で始まる**
  (中央値が 0.57 %→5.33 %に跳ぶ)— フィットから −1000 を除外した発注書の解釈(モジュール
  doc 参照)と整合する。
- `fit.max_resid_frac`(ch 毎、全振幅中の最大 = ほぼ全 ch で −1000 の点)は
  signal ch で中央値 5.38 %・p90 6.70 %・最大 8.34 %。
- **観察(要レビュー、モデル化はしていない)**: −100 mV でも残差中央値 4.69 %と、単純な
  「圧縮は高振幅だけ」という想定に反する値が出ている。飽和ではなく、低振幅側の別要因
  (オフセット/しきい値的な非線形性)の可能性がある。発注書は「圧縮のモデル化はしない」と
  明示しているため、原因の特定・モデル化はせず観察として記録するに留めた。

### 平均波形の立ち上がり・ピーク・FWHM(signal ch 平均、FPN 除く、単位 = ADC サンプルバケット)
| 振幅 [mV] | peak bucket | peak [ADC] | rise 10–90 % [buckets] | FWHM [buckets] |
|---|---|---|---|---|
| −1000 | 188 | 3229.22 | 57.41 | 167.90 |
| −700  | 188 | 2394.61 | 59.36 | 163.14 |
| −500  | 187 | 1749.24 | 59.63 | 161.63 |
| −300  | 187 | 1076.11 | 60.01 | 160.89 |
| −200  | 188 |  728.98 | 60.74 | 160.45 |
| −100  | 187 |  368.76 | 60.93 | 162.93 |

- peak bucket はほぼ一定(187–188)— パルサーのタイミングは振幅に依らない、想定どおり。
- rise time は振幅が上がるほどわずかに短くなる(60.9 → 57.4 buckets、約 6 %)、FWHM も
  −1000 だけ明らかに広い(168 vs 160–163)— いずれも高振幅側の波形整形の非線形性
  (圧縮)の別の現れとして整合的。
- **単位について(要レビュー)**: 入力 CSV(`calib scan`)には AGET のサンプリングクロックが
  乗っていない(発注書にも記載が無い)ため、buckets → ns の換算はしていない。ns 換算が
  必要ならこの run の xcfg のクロック設定を教えてほしい。

### 図の所在
- `reference/_spike/d3/pulserScan.png`(本体、gitignore)— 4 パネル(① ch 毎応答、
  ② 残差分布、③ 平均波形、④ 立ち上がり/ピーク/FWHM 表)。
- 中間生成物(同ディレクトリ、gitignore): `scan.json`(24 MB)、`scan_channels.csv`
  (273 行 = 272 ch + header)、`scan_waveform.csv`(835,585 行 = 272 ch × 6 振幅 × 512
  bucket + header)。

### 発注書からの逸脱・承認が要る判断(実装せず報告する分岐は無かったが、解釈を記録)
1. **「振幅 ≤ −700 mV(圧縮前)」の読み**: 実測振幅は全て負なので文字どおりの実数比較では
   圧縮側の 2 点(−700, −1000)しか残らず「圧縮前」の意図と逆になる。**絶対値 ≤700 mV**
   (−100/−200/−300/−500/−700 の 5 点)をフィットに使う解釈を採用した
   (`src/calib/scan.rs` モジュール doc に明記)。
2. **`--runs <json>` は未実装**: 発注書は `--amp` の代替として挙げているが JSON の形が
   指定されていない。推測でスキーマを決めるのは設計判断になるため、実際に必要な
   `--amp <mV>=<path>` の複数指定だけを実装した(KISS、6 run の実運用はこれで足りる)。
3. **CSV は 2 本に分割**(`--csv-channels` / `--csv-waveform`、単一 `--csv` フラグではない):
   ch 毎のスカラ(振幅ラダー + フィット)と 512 点 × 6 振幅の波形は形が違うので、1 本の
   CSV にすると小さい表が肥大化するか、列名からの振幅値パースに頼ることになる。channels.csv
   はヘッダの `amp_<値>` から振幅を自己完結で読め、waveform.csv は行ごとに `amp_mv` 列を
   持つので、2 ファイル間の順序一致を仮定しない設計にした。
4. **FPN も `fit`/残差の計算対象に含めている**(発注書はパネル③〔平均波形〕でだけ
   「FPN 除く」と明記)。FPN のフィットは飽和のため無意味に近い値になるが、
   除外の指示が無い箇所は含める方を選んだ(そのまま出力、無害)。
5. **圧縮そのもののモデル化はしていない**(発注書が明示的に除外)。線形フィット + 残差報告
   のみ。

### 変更ファイル
- 新規: `src/calib/scan.rs`, `macros/pulserScan.C`
- 変更: `src/calib/mod.rs`(`pub mod scan;`)、`src/bin/calib.rs`(`scan` サブコマンド)、
  `tests/calib_synthetic.rs`、`tests/calib_real_graw.rs`、`macros/README.md`
  (SDKROOT note + pulserScan.C 行)、本チケット(この結果節)。

