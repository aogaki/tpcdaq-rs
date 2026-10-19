# 161 — ガス流量 100 → 50 ml/s の比較(run 174 / 175、週末 run)

**Status: IN PROGRESS**(2026-10-19 起票 Fable。ユーザー「何か違いは見えそうですか。転送に時間がかかるので向こう(ELITPC)で解析を」)

## 入力
- run 174: 2026-10-16 11:26–12:08(42 min、11,145 events、B、ZS なし)、ガス **100 ml/s**。
- run 175: 2026-10-16 12:08 – 10-19 07:52(67.7 h、1,057,719 events、B、ZS なし、raw 130 GB)、ガス **50 ml/s**(12:08 に変更)。
- 条件は同一(B = GEM 240 × 3、502 ns × 120 fC、full、25 MHz、delay 1750、DRIFT 600 × 2、CO2 91 mbar、228Th)。

## 方法(ELITPC、`~/test/scan/gas_run_161.sh` — 1 本のシーケンサ、nohup)
1. `tools/scan/gas_trend.C`(新、テスト 13 チェック緑): raw を 1 回走査し、時間ビンごと(174 = 10 分、175 = 1 時間)に
   電荷の中央値 + 16/84 %、ヒット ch、飽和率、帯の first/last、パルス幅、電荷重心時刻、レート → `~/test/scan/gas/run017N_trend.{json,png}`。
2. `metrics_for_run.sh --pressure 91`(標準の指標 + macros の飛跡)を 174 全体と 175 全体に → `~/test/scan/metrics/run017N_metrics.json`。
3. 175 の `_uvw.root` ができたらユーザーの `chargeVsTime.C`(ana_107)で電荷 vs 時刻(1 h ビン)を描く(出力は uvw の隣 = `metrics/`)。
4. 見るもの: ①電荷中央値の時間変化(不純物による付着なら全体が下がり、レートも落ちる)②ヒット ch・飛跡長・dEdx の 174 → 175 差
   ③帯位置・パルス幅(ドリフト速度・拡散 = ガス組成の変化)④飽和率(ゲイン変化)。

## 結果
(処理中)

## 実行できなかった操作
なし。
