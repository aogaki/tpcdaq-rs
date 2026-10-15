# 141 — 読み出しモード / hit register 閾値 / ZS のスキャン(実飛跡、E 設定 502 ns × 240 fC)

**Status: COMPLETED**(2026-10-15 起票 Fable。CURRENT「今日の進行」の C。ユーザー「続けてください」)

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

## 結果(2026-10-15 07:56–08:37 EEST、run 115–121、各 ≈ 1,500 events、base = run 114 の設定、終了後 base 復元・Idle)

| 点 | run | readout / global / lsb / ZS | graw [kB/event] | ヒット ch | 総電荷 | 飽和 | クリップ 先頭/末尾 | ok | 長さ [mm] | 飛跡電荷 | dEdx | dip | 揺れ A0–A3 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | 115 | **full** | **279** | 117 | 1.69 M | 0.5 % | 2.5 / 7.2 % | 0.995 | 82.1 | 0.67 M | 11,933 | 6.9 | 15.5 / 12.8 / 12.3 / 16.0 |
| 1 | 116 | partial 1 / 15(現用) | **557** | 119 | 1.73 M | 0.4 % | 2.3 / 7.6 % | 0.993 | 83.6 | 0.70 M | 11,679 | 7.7 | 15.5 / 13.3 / 12.7 / 16.1 |
| 2 | 117 | partial 1 / 8 | 557 | 117 | 1.69 M | 0.5 % | 2.9 / 6.9 % | 0.997 | 81.8 | 0.67 M | 11,973 | 6.7 | 同上 |
| 3 | 118 | partial 1 / 4 | 557 | 120 | 1.72 M | 0.7 % | 3.1 / 7.4 % | 0.993 | 83.5 | 0.69 M | 11,934 | 6.8 | 同上 |
| 4 | 119 | partial 0 / 15 | 557 | 117 | 1.70 M | 0.9 % | 2.3 / 7.2 % | 0.997 | 82.6 | 0.68 M | 11,726 | 7.2 | 同上 |
| 5 | 120 | partial 2 / 15 | 557 | 117 | 1.70 M | 1.0 % | 2.9 / 8.1 % | 0.997 | 81.8 | 0.67 M | 11,941 | 6.7 | 同上 |
| 6 | 121 | partial 1 / 15 + **ZS on 600** | **14.8** | 121 | (3.50 M ※) | 0.7 % | 1.1 / 1.1 % | 0.997 | 86.0 | 1.37 M ※ | 18,027 ※ | 7.4 | (103 ※) |

※ ZS の run は `run_metrics.C` の ped(cells 1..20)と FPN 減算が成り立たない(抑制されたセルが 0)ので、電荷・揺れの列は無効。macros の飛跡(ok / 長さ / dip)は有効。

### 読み
1. **現在の partial readout は 1 本も ch を減らしていない**。テンプレートの `channel.Reading = always`(非管理ノブ、xcfg の注記どおり `isAllChannelRead=false` かつ `enableWriteHittedregister=true` のときに効く)なので hit register の結果に関わらず全 272 ch を読む。しかも partial は frameType 1(サンプル 4 B)になるため、**full(frameType 2 compact、2 B)の 2 倍のデータ量(557 vs 279 kB/event)**。指標は全点で同じ(ヒット ch 117–121、ok 0.993–0.997、長さ 82–84 mm)= 閾値 global / lsb は何も変えていない。
2. **ZS(CoBo、一律 600)は端から端まで通る**(decoder / root-sink / macros): 14.8 kB/event = full の 1/19、飛跡 ok 0.997・長さ 86 mm と遜色なし。ただし ped が 364–548 ADC とばらつくので一律 600 は ch により ped +50〜+240 で、低 ped の ch で小さい信号を失い、高 ped の ch で雑音を残す。**ch ごとの ZS 閾値 = ped + k·σ(080 D-2、D の pedestal 表から作れる)**が次。解析側(run_metrics / 136 系)は ZS 対応(ped を外部表から、FPN は無い前提)が要る。
3. **運用の即効薬: base を `readout: full` に**(データ半分、FPN は読まれる、指標同一)。hit register による本当の部分読み出しを試すには `Reading = only_if_hit` のテンプレート(非管理ノブ → `configure-elinp-hit.xcfg` を pulser 用と同じ流儀で)を作り、閾値スキャンをやり直す(frameType 1 のまま ch 数が減るかを見る)。

### 次(起票は着手時)
- base_E を `readout: full` に変えて E の設定を更新(ユーザーの了解後)。
- `Reading=only_if_hit` テンプレート + 閾値スキャン(141 の再走、30 分)。
- ch ごとの ZS 閾値(D の pedestal 表 → `exceptions` で `zeroSuppression.threshold` を ch ごとに)+ 解析側の ZS 対応。

### 実行できなかった操作
なし。
