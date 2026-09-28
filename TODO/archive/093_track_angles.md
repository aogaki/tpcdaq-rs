# 093 — analyzeAngles.C: 飛跡の角度分布(全飛跡 / 左→右貫通)+ 入口 y 回帰による線源距離

**Status: COMPLETED**(2026-09-28 起票・実装 Fable。ユーザー「全てのトラックの角度分布、できれば左から右に通っている粒子で」)

## 0. 背景
- 2026-09-25 run(`macros/data/CoBo_2026-09-25T08-56-08.494_tracks.root`): ²²⁸Th 線源をケージから **約 400 mm** 離して設置、CO₂ 100 mbar。
- `analyzeTracks.C` の逆投影(histSource、本チケットで ±160 → **±500 mm** に拡張)は線源像を x ≈ −250〜−300 に出す = 真値より手前。
  角度誤差 σθ の直線束のくびれは D_app ≈ D0 / (1 + (σθ·D0/W)²) で手前に寄るため(W = ケージ半幅)。**偏りのない距離推定 = 入口 y に対する tanφ の回帰**(勾配 = 1/D)。

## 1. 実装 — `macros/analyzeAngles.C`(新規、ROOT 6.40 インタプリタ実行)
- 全飛跡を **+x 向きに再配向**(線源は −x 側。makeTracks の電荷ベース start/end は貫通飛跡ではコイン投げなので無視)。
- φ = atan2(dy, dx)(面内角、+x が 0°)/ θ = atan2(dz, |dxy|)(パッド面に対する dip)。
- 「貫通」= 両端がケージ x 壁(±53、fastqa)から EDGE_MM = 10 mm 以内。長さ ≥ 50 mm、|dxy| ≥ 1 mm。
- 3 枚目: 貫通飛跡の tanφ vs 左壁での入口 y(TH2 + TProfile + 直線フィット [−45, 45])→ D = 1/勾配、y_src = −切片/勾配。

## 結果(2026-09-28、Mac、ROOT 6.40.04、`root -l -b -q 'analyzeAngles.C("data/CoBo_2026-09-25T08-56-08.494_tracks.root")'`)
```
selected 71282 tracks (length >= 50 mm), through (left -> right) 12020
tan(phi) = -0.0053 + 0.00289 * y  ->  source at x = -399 (D = 346 +- 7 mm from the left wall), y = 1.8 mm
```
- **線源 x ≈ −399 mm(左壁から 346 ± 7 mm)、y ≈ +2 mm** — ユーザー申告「約 400 mm」と一致。逆投影の −250〜−300 は分解能による見かけ(σθ ≈ 5° 相当)で、回帰は偏らない。
- φ: 0° 中心、FWHM ≈ 10°、貫通 12,020 本は ±15° 内にほぼ収まる。
- **θ(dip)は ±17° に鋭いエッジ + そこにピーク、|θ| < 15° が空**(全飛跡・貫通とも)。飛跡長依存を見ると(`reference/_spike/angles_0925/thetaVsLen.png`)
  **|dz| に ≈ 20 mm の床が全長で一定**、貫通飛跡の (L_xy, dz) の尾根は dz が小さいほど L_xy が縮む(≈ 106 → 80 mm)。
  = **makeTracks の「ドリフト z スライス重心」トラッカーの系統**: 各ヒットはパルスの時間幅ぶん z に伸びる(閾値 35 ADC 超の幅 ≈ 20 mm ≈ 115 cell ≈ 4.6 µs @ 0.174 mm/cell)ので、
  測定 dz = 真 dz + w_z、測定 L_xy = L − w_z/tanθ。浅い飛跡は xy が潰れて z 向きになり(長さカットで落ちる)、残りは ±17° に積み上がる。**現状の dip 角は |θ| ≲ 20° で信用できない**(面内角 φ と入口 y 回帰は影響なし)。
  → 対処は [094](094_tracker_peak_time_z.md)(ストリップ毎のパルスピーク時刻を z にする)。
- 図: `reference/_spike/angles_0925/angles.png`(3 パッド)、`thetaVsLen.png`(|θ| vs L_xy、|dz| vs L_xy)。
- 付随変更: `analyzeTracks.C` histSource を ±500 mm(500 bin)に。テストなし(ROOT マクロ、目視 + 上記数値)。

## 追記(2026-09-28、ユーザー要望「ある φ でドリフトケージが見えているか」)
- `analyzeAngles.C` に 4 パッド目 **θ vs φ(全飛跡、1° ビン、±60°)** を追加(キャンバスは 2×2)。094 後の 09-25 データ: 中央のビーム塊(|φ| ≲ 25°、θ は ±50° まで)、|φ| ≈ 25–35° にほぼ空の帯、|φ| > 35° に広角の一様成分(ガス中 ²²⁰Rn 系列)。
  図: `reference/_spike/angles_0925/v2r3_angles4.png`。
