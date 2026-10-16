# 150 — 最大レート試験 2: 運用点 B(full)/ hit register partial(g4)/ ZS(ch ごと)— 何が上限か(FW の読み出し時間 vs 1 GbE)

**Status: IN PROGRESS**(2026-10-16 起票 Fable。ユーザー「ビーム実験では 100 Hz、フルサイズは 4 倍のストリップなので mini で 400 Hz が出れば完璧。光ファイバで更なる高レートに」→ 私の見立て(FW の 2.93 ms/event = 340 Hz が先に効く、光は zCoBo のリンクには効かない)を数字で確かめる)

## 0. 問い(archive/109 は partial(frameType 1、557 kB/event)での 182 Hz = 1 GbE 上限)
1. B(full、279 kB/event)の上限は FW(2.93 ms → 340 Hz)か網(360 Hz)か。
2. hit register partial(`only_if_hit`、global 4 = 読む ch 43 %)で **読み出し時間が 2.93 ms から縮むか**(縮めば不感時間を買える)。
3. ZS(32 kB/event)の上限(網は問題外 → FW と root_sink の CPU)。

## 1. 取得(`~/test/scripts/maxrate_B.sh`、周期トリガ、各 30 s、root_sink workers 4、終了後 workers 1 / B に復元)
| 系列 | テンプレート | 周期 [ms] |
|---|---|---|
| B full | `configure-elinp-per{10,5,3,2,1}.xcfg`(109 の周期版) | 10 / 5 / 3 / 2 / 1 |
| hit partial g4/l15 | `configure-elinp-hit-per{3,2,1}.xcfg`(hit テンプレートの周期版、本票で追加) | 3 / 2 / 1 |
| ZS ped+5σ | `configure-elinp-per{3,2,1}.xcfg` + `base_zs_k5.json` の exceptions | 3 / 2 / 1 |
指標: 達成レート(raw ツリーの eventTime から)、事象間隔の中央値 / p10 / p90 / 最大(`/tmp/rate_stats.C`。109 では「周期 + 2.93 ms」が一定)、eventId の欠番、kB/event、run/stop の所要。

## 結果

### 1 回目(周期トリガ、信号なし、08:25–08:43、run 148–155、workers 4)
| 系列 | 周期 [ms] | run | 達成レート [Hz] | 事象間隔 中央値 / 最大 [ms] | 欠番 | kB/event |
|---|---|---|---|---|---|---|
| B full(272 ch、frameType 2) | 10 | 148 | 77.3 | 12.938 / 12.9 | 0 | 278.8 |
| | 5 | 149 | 126.0 | 7.938 / 7.9 | 0 | 278.8 |
| | 3 | 150 | 168.4 | 5.938 / 5.9 | 0 | 278.8 |
| | 2 | 151 | 202.5 | 4.938 / 4.9 | 0 | 278.8 |
| | 1 | 152 | **253.9** | **3.938 / 3.9** | 0 | 278.8 |
| hit partial g4(信号なし → FPN 16 ch + 雑音数 ch) | 3 | 153 | 304.5 | 3.283 / 4.1 | 0 | 33.1 |
| | 2 | 154 | 438.0 | 2.283 / 2.3 | 0 | 33.0 |
| | 1 | 155 | **779.1** | **1.283 / 2.8** | 0 | 33.1 |
| ZS(ch ごと) | 3 / 2 / 1 | — | 中断(frames 0 で stall) | | | |

読み:
- **B full の限界は FW の読み出し時間**: 事象間隔 = 周期 + **2.938 ms** で全点一定、1 ms 周期でも最大間隔 3.9 ms = 中央値(網の背圧による乱れ無し)。279 kB × 254 Hz = 71 MB/s で 1 GbE に余裕。**周期トリガでは「周期 + 読み出し」が間隔**なので達成は 254 Hz、読み出し 2.94 ms が不感時間。
- **hit register の partial は読み出し時間を縮める**: 信号なし(読む ch ≈ FPN 16 本)で **0.283 ms**。272 ch で 2.938 ms なので **読み出し時間 ≈ 0.11 ms + 10.4 µs × 読む ch 数**。α の実事象(ヒット ch ≈ 120 + FPN 16)なら ≈ 1.5 ms = 不感時間が半分。
- ZS + 周期トリガは信号が無いので CoBo が 1 フレームも出さない(stall)— ZS のレート試験は信号が要る → 2 回目をパルサー(全 256 ch に −300 mV)で。
