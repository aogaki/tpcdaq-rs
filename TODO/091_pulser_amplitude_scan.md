# 091 — D-3: AsAd 内蔵パルサーの振幅スキャン(GDED 遠隔、実 zCoBo)→ ゲイン直線性 + 電子回路応答

**Status: IN PROGRESS**(2026-09-23 起票 Fable。080 D-3 / 073-#10 / 066(b) 前半。データ取得 = Fable(遠隔)、解析 = Sonnet)

## 0. 取得(2026-09-23、GDED、Rust ecc_bridge + 実 ECC ネイティブ、VM の GET は停止)
- 構成は 076 結果節(Phase 3 前提)と同じ。1 run = 1 振幅、`asad-pulser-start <mV> 1000`(周期 1000 ms)、60 s(≈ 60 frames、frameType 2 compact、全 ch)。
- 振幅: **−100 / −200 / −300 / −500 / −700 / −1000 mV**(`scan_d3.sh`)。run 番号・ファイルは下の結果節。
- 注意: run 5(12:04–12:06)はスクリプトの変数展開ミスで `asad-pulser-start ${3:--1000}` を送っており **無効**(直前の −1000 mV 設定が残っていた可能性)。解析から除外。
- 2026-09-14 の run 1/2(−1000 mV、122 + 20 frames)も同条件の参照点として使える(`out/run0001`, `out/run0002`)。

## 1. 解析発注(Sonnet、データを Mac に持ち帰ってから)
- `calib pulser`(081)を各 run に掛け、ch 毎の応答(中央値)を振幅の関数にする → 直線性(残差)と ch 毎ゲイン(068 の相対ゲインと照合)。
- 平均波形(ch 平均、振幅ごと)を出し、立ち上がり・ピーク位置・幅を振幅の関数で表にする(066(b) の電子回路応答 = Unfolding の応答関数の元)。
- 出力は `macros/` の ROOT マクロか `src/bin/calib.rs` の拡張のどちらか(KISS で選び、理由を書く)。図は `reference/_spike/d3/` に。

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

