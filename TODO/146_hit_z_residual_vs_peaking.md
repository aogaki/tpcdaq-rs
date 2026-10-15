# 146 — ストリップヒットの z 残差(飛跡直線との差)を peaking / HV ごとに実測する(信号幅を誤差に使うかの判断材料)

**Status: IN PROGRESS**(2026-10-15 起票 Fable。ユーザー「結局は信号幅をトラックの誤差として使用する可能性があるのでそこを明確にしたい」→「c もやりましょう」)

## 0. 問い
- ヒット 1 本の z の決定精度 σ_z(= ピーク時刻の精度 × v_drift)は peaking(70 / 232 / 502 / 1014 ns)でどう変わるか。整形が長いと S/N は上がるが立ち上がりが鈍る。測らないと分からない。
- 「信号幅を誤差に使う」の是非: 測定幅は固有幅 ⊕ 整形幅(139: 核を二乗で引くと固有幅 ≈ 14.5 cell で peaking に依らない)。幅を使うなら核を引いた固有幅。z 残差の実測と比べて、どちらが誤差として妥当かを決める。

## 1. 入力(ELITPC `~/test/scan/metrics/`、ユーザー macros の出力。**macros は読むだけ、変更しない**)
- `run00NN_uvw.root` の TTree `uvw`: イベントごとの strip ヒット `slView`(0/1/2 = U/V/W)、`slZ`(z [mm] = t_peak × v_drift)、`slS`(view 内のストリップ座標 [mm])、`slQ`(電荷)、`width[3]`、`nhits[3]`。
- `run00NN_tracks.root` の TTree `tracks`: `ok`、直線の両端 `xStart..zEnd`(`macros/makeTracks.C` の手順 4: z = z0 + tanθ·l を全ヒットで当てはめ。l は XY 面での飛跡に沿う座標)。
- run: peaking スキャン 83(70 ns)/ 84(232)/ 85(502)/ 86(1014)、同 HV 250 × 3、120 fC。追加で 123–127(HV 違い)。

## 2. 作るもの `tools/scan/hit_residuals.C` + `test_hit_residuals.C`
- 入口 `int hit_residuals(const char* uvwRoot, const char* tracksRoot, const char* outJson, const char* outPng = nullptr, int maxEvents = 0)`。
- イベントごと(ok == 1 のみ): 直線(start→end)から XY 方向と z(l) を復元し、各ヒットについて **ヒットの l を view の射影から求める**(makeTracks.C と同じ幾何: view の方向ベクトルと `slS` の定義を macros から読み取って同じ式にする — 発注書に式を書かないのは macros が正だから。実装前に macros の該当箇所を引用して報告に書く)。残差 r = slZ − (z0 + tanθ·l)。
- 出力(JSON): run、events、hits、view ごと・全体の残差の **RMS と robust σ(1.4826 × MAD)**、残差の中央値(バイアス)、電荷 4 分位ごとの robust σ(S/N 依存)、`width` の中央値(uvw の width[3])。PNG: 残差ヒスト(view 別)+ 残差 vs 電荷。
- 注意: 飛跡の両端(l の 2–98 % 分位の外)のヒットは除く(端の電荷の落ちは幅の問題でない)。直線自体がそのヒットを含んで当てはめられている(自己相関)ので、σ は下限側に少し偏る — 報告に明記。ヒット数が多い(≥ 20)ので補正は小さい。
- テスト: 合成 uvw/tracks(既知の直線 + 既知の σ のガウス残差 + 端のヒット + ok=0 イベント)で RMS / MAD / 中央値 / 分位の期待値を固定、無いファイルで 1。既存テスト(`test_run_metrics` 88 / `test_pulser_kernel` 237 / `test_drift_end` 88 / `test_deconv` 133)の回帰。

## 3. 適用と判断(主対話)
run 83–86 で σ_z(robust)vs peaking の表 → 139 の固有幅(14.5 cell × 40 ns × v_drift)と比較 → 「誤差 = 固有幅」か「誤差 = 実測 σ_z(peaking 依存)」かを決める。HV 違い(123–127)で S/N 依存を分ける。

## 結果
