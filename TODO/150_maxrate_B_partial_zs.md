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
