# 101 — dip 角が 2 倍に膨らんでいる(z スケール ×2): ドリフト電圧 1200 V の仮定か、サンプリング 25 MHz の仮定か

**Status: READY**(2026-10-05 起票 Fable。100 の分解能分解の副産物。**2026-10-08: 実際のサンプリングは 100 MHz と判明(追記 2)→ 主因はこれ。次 = samplingMHz 引数化と再生成**)

## 0. 事実(91 mbar run、²¹⁶Po ピークの飛跡 3,053 本、`reference/_spike/stop_0929/{egenWidth,zscale,tiltTest}.C`)
- 生成時 E の幅: 全飛跡 σ 0.077 MeV / |θ|,|φ| < 5° だけ σ 0.041 / 経路の角度補正なし(L = d)σ 0.051 / **x だけ(r = x_end + 53)σ 0.046** = ストラグリングのみの期待値 0.047。
  |θ| > 15° の平均は 6.999 MeV(+0.2)→ 1/dir_x の経路補正が過剰。
- **x 方向の到達深さ vs 測定 |θ|**(`tiltTest.C`): 94.0 / 93.9 / 93.0 / 92.6 / 92.0 / 91.4 mm(|θ| 0–5 … 25–30°)。本物の傾きなら cos θ で縮むはず。
  縮み方は **θ_true = atan(0.5 · tan θ_meas)** で 5 ビンとも合う(k = 0.5 ± 0.05)→ **測定 dz が真の 2 倍**。
- 同じ飛跡の |θ| 分布: < 10° 42 %、10–17° 25 %、17–25° 20 %、> 25° 12 %。点線源 + 高さ 200 mm のケージなら |θ| ≤ 17° のはず(k = 0.5 なら 2 倍で整合)。
- 影響: これまでの dip 角(093 の「θ FWHM 25°」、094 の受け入れ、097 のケージ縦横比の議論)、3D 長さ、z 窓(89 → 実は ≈ 45 mm)、stoppedAlphas の 1/dir_x 補正がすべて 2 倍の z で計算されている。xy と停止点 x、電荷、密度・距離フィット(x のみ)は無影響。

## 1. 候補(どちらも z を正確に 2 倍にする)
1. **ドリフト電圧が 1200 V でない**(analyzeUVW の `DRIFT_HV_V = 1200` は引き継いだ定数)。600 V なら E/p = 0.32、v = 0.24 cm/µs で k = 0.49。
2. **サンプリングが 25 MHz でない**(50 MHz なら 1 cell = 20 ns)。パルスの閾値超が ≈ 100 cell なのは、25 MHz + 整形 ~1 µs でも 50 MHz + 0.5 µs でも説明できる。

## 2. 発注(確認後)
- 該当する定数を引数化(`analyzeUVW.C(raw, pressure, driftHV, samplingMHz)`)し、README の注意書き。全 run を再生成。
- 再生成後: stoppedAlphas の生成時 E が |θ| に依らないこと(`zscale.C` で k ≈ 1)、σ ≈ 0.05 MeV(FWHM ≈ 1.7 %)、through 飛跡の |θ| ≤ 17° を確認。
- v_drift の最終確認は 069 C-4 のドリフト端法(別途)。

## 追記(2026-10-05、ユーザー回答「1200 V で 25 MHz」→ 両候補とも否定。精査の続き)
- 25 MHz はトリガ遅延(17.5 µs、RUNBOOK)が 20.5 µs 窓に収まることとも整合。
- **時間勾配は本物**(`zprofile.C`): 測定 θ −26° の飛跡で z(x) が U 60 点にわたり直線(70.6 → 27.8 mm = 9 µs)。
- **幾何学的な傾きはほぼ無い**(`peakVsTheta.C`、深さ窓なし): ²¹⁶Po ピークは測定 |θ| 0–5° で x ≈ +39〜45、20–30° で +36〜39(点線源 d = 319 なら 25° で +3 のはず)。真の傾きは数度。
- **基準は φ**(`peakVsPhi.C`、|θ| < 10°): |φ| 0–4° 2128 / 4–8° 1043 / 8–12° 135 / ≥ 12° 0 → ±8° = ポート開口(r ≈ 20 mm @250 mm → ±5°)と整合。丸いポートなら θ も ±5° のはずが、測定 θ は RMS 16°、32 % が 17° 超。
- θ/φ の広がりの比 4〜5 → **データの示す v_drift ≈ 0.1 cm/µs**(想定 0.48 の 1/5)。移動度則は HIGS(130 mbar / 70 V/cm)で拡散の熱限界一致(066)により支持されるので、**ドリフト領域の実電界が 10〜15 V/cm 程度**(1200 V がカソード対地で GEM1 上面が数百 V?)か、ガス組成を疑う。**→ ユーザーにカソードと GEM1 上面の電位を確認依頼**。
- パルス形状(`pulseShape.C`): 立ち上がり 10–90 % 49 cell、FWHM 70 cell、閾値超 174 cell(25 MHz なら 2.0 / 2.8 / 7 µs)。AGET の最長整形(1 µs)より長い = 検出器信号自体の広がり or 整形設定の確認要(091 のパルサー 100 MHz は立ち上がり 0.6 µs / FWHM 1.65 µs)。
- 経路補正を外す(L = d)と 4 枚目は σ 0.051(FWHM 1.8 %)。z を使わない量は全て有効。

## 追記 2(2026-10-08 夜、Opus — **サンプリングは 100 MHz だった**)
- 107 の移行作業で VM の GetController 設定を `VBoxManage guestcontrol` で読んだ(ユーザーがパスワードをファイルで用意)。
  `~/.config/actar-get/GetController.conf` の `lastWorkspace` = `/home/getuser/GetSoftware_config/workspace.xcfg`(2026-08-05)、
  データを TPC_Share に書くテスト `testPedestals` の Conditions = **`/home/getuser/Desktop/TPCConfigFiles/delay_test.xcfg`**(2026-09-03、md5 0845d6c4)。
  **ユーザー確認(同日): 「delay_test.xcfg が今データ取得用に使ってる設定」**。
- 中身: **`writingClockFrequency` = 100.0 MHz**(読み出し 25 MHz)、`peackingTime` = **232 ns**、`Gain` = 120 fC、`triggerMode` = external、
  `triggerDelay` = 320 × 10 ns = 3.2 µs(窓 512 cell = 5.12 µs)、WCK triggerDelay 16、ZS 無効、`isAllChannelRead` false / GlobalThreshold 1。
  雛形は同ディレクトリの `configure-physics-zCobo-extTrig_120fC_100MHz_70ns_trigDelay360.xcfg`(差は triggerDelay 360→320、WCK triggerDelay 0→16、peaking 70→232 の 3 行)。
  `.bu`(09-01)は triggerDelay 450 / peaking 70。
- **これで 101 の不整合がほぼ全部解ける**(1 cell = 10 ns。25 MHz 仮定の 40 ns は 4 倍):
  - パルス形状: FWHM 70 cell = **0.70 µs**、立ち上がり 49 cell = 0.49 µs → 整形 232 ns + 拡散で自然(25 MHz 仮定の 2.8 µs は AGET の最長整形より長いという矛盾が消える)。
  - θ/φ の広がりの比 4〜5 から出した v_drift ≈ 0.1 cm/µs は ×4 で **≈ 0.4 cm/µs**(移動度則の 0.48 と 2 割差)→ 「ドリフト領域の実電界が 10〜15 V/cm」の仮説は不要。
  - `tiltTest.C` の k = 0.5(dz 2 倍)は 4 倍と合わない → 再生成後に見直す(深さ窓や x_end の選び方の系統か)。
- 「1200 V / 25 MHz は確認済み」(10-05 の回答)は 25 MHz について**誤り**。カソード / GEM1 の電位確認は優先度を下げる。
- 発注(§2)は `samplingMHz` の引数化を先に: analyzeUVW の時間 → z 変換を 100 MHz に、全 run 再生成 → `zscale.C` / `tiltTest.C` で k ≈ 1 を確認。
  サンプリングを実機で裏取りするなら zCoBo の CKW 分周レジスタの読み取り(読むだけ)。

## 追記 3(2026-10-09、ユーザーの再解析 — 100 MHz で裾が消えた)
- ユーザーが macros をサンプリング 100 MHz にして再解析(α 線源、新トリガ系、91 mbar、stoppedAlphas の stopped inside 5,577 本、x_src = −370 仮定)
  → **生成時エネルギーの高エネルギー側の裾が消えた** = 25 MHz 前提で z が 4 倍 → 1/dir_x 経路補正が過剰、が主因と確定。
- 生成時エネルギーのピーク ≈ 5.95 / 6.19 / 6.66 MeV(画像からの読み取り)。間隔 0.24 / 0.47 MeV は ²¹²Bi(6.05–6.09)/ ²²⁰Rn(6.288)/ ²¹⁶Po(6.778)
  の 0.23 / 0.49 と一致。全体 −0.1 MeV は線源距離か密度(温度)— stoppedAlphas の密度・距離フィットで詰める。²¹²Bi が明瞭(トリガ系の修正 107)。
- 電荷 vs E: E ≈ 1.8–2.5 MeV に本線より低電荷の点が散らばる。**100 MHz の時間窓は 5.12 µs ≈ ドリフト 24 mm(v_drift 0.477 cm/µs)**しかなく、
  傾いた飛跡の一部が窓外に出て電荷が欠ける可能性(要確認: 電荷 vs dip / z 幅)。
- 残り(§2 発注): macros の `samplingMHz` 引数化(既定 100)と README 注記、`zscale.C` / `tiltTest.C` の見直し(k = 0.5 の件)。
- **ユーザー方針(同日)**: macros を 100 MHz に合わせるより、**データ取得を 25 MHz に戻したい**(時間窓 20.48 µs ≈ ドリフト 98 mm で飛跡全体が入る)。後日。
  そのときは triggerDelay(現 320 × 10 ns)も窓の位置に合わせて決め直す。111(Parameters UI)の最初の実用例になる。
