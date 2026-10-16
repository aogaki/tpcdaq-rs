# 150 — 最大レート試験 2: 運用点 B(full)/ hit register partial(g4)/ ZS(ch ごと)— 何が上限か(FW の読み出し時間 vs 1 GbE)

**Status: COMPLETED**(2026-10-16 起票 Fable。ユーザー「ビーム実験では 100 Hz、フルサイズは 4 倍のストリップなので mini で 400 Hz が出れば完璧。光ファイバで更なる高レートに」→ 私の見立て(FW の 2.93 ms/event = 340 Hz が先に効く、光は zCoBo のリンクには効かない)を数字で確かめる)

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

### 2 回目(AsAd パルサー −300 mV で全 256 ch に信号 = 占有率 100 % の上限側、08:50–09:05、run 159–169、workers 4(168/169 は 1))
| 系列 | パルサー周期 [ms] | run | 達成 [Hz] | 事象間隔 中央値 / p90 / 最大 [ms] | kB/event | 読み |
|---|---|---|---|---|---|---|
| full(272 ch、frameType 2) | 3 | 159 | 295 | 3.010 / 5.9 / 6.1 | 278.8 | 3 ms ≥ 読み出し 2.94 → ほぼ毎パルス |
| | 2 | 160 | 250 | 3.999 / 4.04 / 5.6 | 278.8 | **読み出し中のトリガは捨てられる**(1 個おき = 4 ms) |
| | 1 | 161 | 310 | 3.004 / 3.06 / **117.9** | 278.8 | 3 個に 1 個。86 MB/s で 1 GbE の縁、最大 118 ms の stall |
| hit partial g4(全 ch ヒット → 全 272 ch、frameType 1) | 3 | 162 | 171 | 3.092 / 5.97 / **297** | 557.3 | 95 MB/s = **1 GbE で頭打ち**(109 と同じ 182 Hz 級) |
| | 2 / 1 | 163 / 164 | 168 / 156 | 4.0 / 3.9、最大 452 / 563 | 557 | 同上、stall |
| ZS(ch ごと ped+5σ、全 ch) | 3 | 165 | 333 | 3.000 / 3.012 / 5.9 | 64.9 | 読み出しは 2.94–3.0 ms のまま(ZS は読んだ後の圧縮) |
| | 2 / 1 | 166 / 167 | 250 / 333 | 4.0 / 3.0、最大 4.6 / 4.4 | 63–65 | 22 MB/s、stall 無し |
| hit partial g4 + ZS | 3 / 1 | 168 / 169 | 333 / 323 | 3.000 / 3.01 / 5.9、3.0 / 3.04 / 4.3 | 65.4 / 65.9 | 両立する(frameType 1 + ZS) |

## 結論(Fable)
1. **上限は FW の読み出し時間で、網ではない**: 272 ch で **2.94 ms**(= 340 Hz、1 AsAd)。full / ZS で同じ。1 GbE は full(279 kB)なら ≈ 340 Hz まで余裕があり、1 ms パルサー(310 Hz、86 MB/s)で縁に触れた。
2. **読み出し中のトリガは失われる**(キューされない): パルサー 2 ms → 1 個おき。= 不感時間 = レート × 読み出し時間。ビーム 100 Hz では full で **29 %**。
3. **読み出し時間は読む ch 数に比例**: ≈ 0.11 ms + 10.4 µs × ch(16 ch 0.28 ms、272 ch 2.94 ms)。**hit register の partial(`only_if_hit`)だけが不感時間を縮める手段**。α の実事象(≈ 136 ch)で ≈ 1.5 ms → 100 Hz で 15 %。ただし partial は frameType 1(4 B/sample)で全 ch ヒットだと 557 kB = 1 GbE で 170 Hz 止まり。
4. **ZS はバイトだけを減らす**(読み出し時間は不変)。全 ch ヒットでも 65 kB(full の 1/4.3、α 事象なら 1/8.8)。**partial + ZS は両立**し、不感時間(partial)と帯域・ディスク(ZS)の両方を取れる。
5. **光ファイバ / 10 GbE は zCoBo のリンクには効かない**(FW の MAC が 1 GbE、SFP+ に 1 GbE アダプタ)。PC 側の 10 GbE は 2 枚目の zCoBo を受ける分には既にある。
6. **Warsaw(2 枚 × 2 AsAd)の見積もり**: 1 枚 544 ch → 読み出し ≈ 5.8 ms → 最大 170 Hz、100 Hz で不感 58 %(full)。partial(占有 25 %、≈ 150 ch)なら ≈ 1.7 ms → 17 %。帯域は 1 枚 1 GbE で full 558 kB × 170 Hz = 95 MB/s(縁)、partial + ZS なら余裕。**ビーム 100 Hz の設計 = hit register partial + ZS + 2 本の 1 GbE**。FW の読み出し時間(2 AsAd で本当に 2 倍か)は Warsaw で要実測。
7. mini の「400 Hz」は FW の 340 Hz(1 AsAd)で物理的に無理。partial で ch を減らせば 779 Hz(FPN だけ)まで行くので、**占有率次第**。

## 運用への反映
- 今の α 測定(4.5 Hz)は full のまま(変更なし)。
- ビーム実験の前に: ① macros の ZS 対応(pedestal 表)②partial + ZS の運用 params(`configure-elinp-hit.xcfg` + exceptions)で α run の指標確認 ③不感時間の記録(run_stop に「読み出し ch 数 × 10 µs」の推定を載せるかは別途)。

## 実行できなかった操作
なし(テンプレート `configure-elinp-hit-per{1,2,3}.xcfg`、`configure-elinp-hit-pulser.xcfg` を `GetSoftware_config` に追加 — 承認済みの hit テンプレートの派生)。workers は 1 に戻し、params は B。
