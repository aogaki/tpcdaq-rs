# 094 — makeTracks の dip 角バイアス(|dz| 床 ≈ 20 mm)の解消: ストリップ毎ピーク時刻 → z

**Status: READY**(2026-09-28 起票 Fable。093 の派生。Opus 0.5 日。実機不要 — 09-25 データで検証)

## 0. 事実(093 結果節)
- 現行チェーン `analyzeUVW.C`(hit = (ch, time cell) above threshold → z = t·mmPerCell → 3 mm z スライスの電荷重心)→ `makeTracks.C`(スライス重心の PCA)。
- パルスの時間幅(閾値超 ≈ 4.6 µs ≈ 20 mm)がそのまま z の広がりになり、測定 |dz| ≥ ≈ 20 mm の床、L_xy は w_z/tanθ 縮む。dip 角は |θ| < ~20° が空、±17° にピーク。
- 面内角 φ・入口 y 回帰(線源距離 346 ± 7 mm)は無影響。影響するのは θ、3D 長さ、dE/dx(length)。

## 1. 発注
- `analyzeUVW.C`: ヒットを (ch, cell) でなく **ストリップ毎に 1 点**にする — 波形のピーク時刻(またはピーク近傍 ±N cell の電荷重み付き時刻)を z、積分電荷を q。パルス幅は z に入らない。
  複数パルス(同一ストリップを 2 回横切る飛跡)は mini 1 本直線の前提では無視してよい(最大ピークのみ)。
- `makeTracks.C`: スライス化を **z でなくストリップ座標 s で**行うか、3 view のストリップ点 (s, z, q) から直接 3D 直線を当てる(view 毎 2D 直線 z(s) と s の三角測量。KISS ならまず view 毎に PCA → 交線)。
- 受け入れ(09-25 データ、`analyzeAngles.C`): |dz| の床が消える(θ 分布が 0° 中心の単峰)、貫通飛跡の L_xy が dz に依らず ≈ 106 mm、入口 y 回帰の D が 346 ± 7 から動かない。
- 旧チェーンの出力(083/091 の数値)は再解析しない。新チェーンは別ファイル名(`_tracks2.root` 等)で並走させ、083 の飛跡数だけ再確認。

## 2. 参照
093 結果節、`reference/_spike/angles_0925/thetaVsLen.png`、analyzeUVW.C 定数(ADC_THRESHOLD 35、SLICE_MM 3、25 MHz、v_drift 0.7233×E/p)。
