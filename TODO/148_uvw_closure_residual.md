# 148 — U/V/W の閉合残差で 2D(ストリップ面)重心の精度を測る(peaking / HV / ドリフト電圧ごと)

**Status: IN PROGRESS**(2026-10-15 起票 Fable。ユーザー「2D だとどうですか?」→「オフィスに帰ってから」→「ガンガンお願いします」)

## 0. 原理
U/V/W の pitch ベクトル(macros: U (−1, 0)、V (0.5, +0.866)、W (0.5, −0.866))の和はゼロなので、同じ点 (x, y) のストリップ座標は **s_U + s_V + s_W = 一定(c)**。同じ時刻スライス(同じ z)の 3 面の電荷重心は同じ (x, y) を指すはずなので、閉合残差 r = s_U + s_V + s_W − c の広がりが、飛跡モデルに依らない **2D 重心の精度**(σ_r² ≈ 3 σ_s²、σ_s = 1 面の重心精度)。146(z 残差)の 2D 版。

## 1. 入力
- raw ツリー(`metrics/run%04d_raw.root`、run_metrics.C と同じ前処理: FPN 共通モード減算、ped cells 1..20、閾値 35 ADC、cell 範囲 [2, 508))。
- ストリップ対応: `channel_map.csv`(ELITPC `~/test/ana/ana_107/`、macros と同じ。`analyzeUVW.C` の読み方(列の意味、s = pos [mm]、view 0/1/2)を**読み取って同じにする** — macros は変更しない)。
- 飛跡の選別: `metrics/run%04d_tracks.root` の `ok == 1` のイベントだけ(146 と同じ)。

## 2. 作るもの `tools/scan/uvw_closure.C` + `test_uvw_closure.C`
- 入口 `int uvw_closure(const char* raw, const char* channelMap, const char* tracksRoot, const char* outJson, const char* outPng = nullptr, int maxEvents = 0)`。
- イベント(ok==1)ごと、cell t ごと(スライス幅は 1 cell、必要なら `kSliceCells` 定数で 2–4 cell にまとめられるように): 各 view で閾値を越えたストリップの電荷重心 s_v(t)(重み = その cell の ADC − ped − FPN)。3 面とも ≥ 1 本(かつ各 view のヒットが連続した 1 塊 = 複数飛跡を避ける。塊が 2 つ以上あるスライスは捨てて数える)のスライスだけ採用。
- c は run 全体の r の中央値(幾何のオフセット)。出力: r の robust σ(1.4826 × MAD)、RMS、中央値(= c)、採用スライス数、捨てたスライス数(view 不足 / 複数塊)、**スライス電荷の 4 分位ごとの robust σ**、view ごとの「閾値越えストリップ本数」の中央値(重心に使った幅)。PNG: r のヒスト + r vs スライス電荷。
- 単位は mm。σ_s = σ_r / √3 も JSON に出す(`sigma_per_view_mm`)。
- テスト: 合成 raw + 合成 channel_map(小さな 3 面、既知の pitch)で、既知の (x, y) の点を 3 面にガウス広がり(既知 σ)で載せ、閉合残差の σ が √3 σ になること、c が既知のオフセット和になること、塊が 2 つのスライスが捨てられること、ok==0 イベントが使われないこと、無いファイルで 1。赤 → 緑の実測、既存 5 本(`test_run_metrics` 88 / `test_pulser_kernel` 237 / `test_drift_end` 89 / `test_deconv` 133 / `test_hit_residuals` 145)の回帰。

## 3. 適用(主対話、ELITPC)
run 83–86(peaking)、87 / 122–129(HV)、130–132(ドリフト)。σ_s vs peaking の表 → 2D の精度は長い整形で良くなるか(S/N)を判定。146 の σ_z と並べて「ヒットの誤差 = (σ_s, σ_z)」を確定。

## 結果
