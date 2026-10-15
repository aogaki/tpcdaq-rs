# 141 — 読み出しモード / hit register 閾値 / ZS のスキャン(実飛跡、E 設定 502 ns × 240 fC)

**Status: IN PROGRESS**(2026-10-15 起票 Fable。CURRENT「今日の進行」の C。ユーザー「続けてください」)

## 0. 問い
- partial readout(hit register)の閾値(`globalThreshold` 0–7、`lsbThreshold` 0–15)で**弱いストリップをどこまで拾えるか**と**データ量**の関係。full readout を基準にする。
- CoBo の zero suppression(`zeroSuppression.threshold` 600、全 ch 一律)を実飛跡で有効にすると何が起きるか(データ量、decoder / root-sink / run_metrics が壊れないか、FPN 減算が成り立つか)。
- 出口: E 以降の運用設定(readout / 閾値)と、080 D-2「較正表の適用」(ch ごとの ZS 閾値)に進むかの判断。

## 1. 取得(`~/test/scan/readout.json`、base = `base_E.json` = run 114 の設定、各 1,500 events ≈ 5.5 min、07:56 開始)
| 点 | readout | global / lsb | ZS |
|---|---|---|---|
| 0 | full | 1 / 15 | off |
| 1 | partial(現用) | 1 / 15 | off |
| 2 | partial | 1 / 8 | off |
| 3 | partial | 1 / 4 | off |
| 4 | partial | 0 / 15 | off |
| 5 | partial | 2 / 15 | off |
| 6 | partial | 1 / 15 | **on、600** |

## 2. 指標
`run_metrics.C`(ヒット ch、電荷、ok、長さ、dEdx、クリップ)+ **データ量 / event**(graw のバイト数 ÷ events)+ FPN が読めているか(`agets_without_fpn`)+ 揺れ。full(点 0)で、閾値をソフトで模擬した「拾えるはずの ch 数」も出せれば出す。

## 結果(取得後に記入)
