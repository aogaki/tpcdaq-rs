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

