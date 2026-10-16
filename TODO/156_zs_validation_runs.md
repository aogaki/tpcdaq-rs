# 156 — ZS の検証 α run(B + ZS、B + hit partial g4 + ZS、FPN あり / なし)→ SCANS S18、FPN を残すかの判定

**Status: READY**(2026-10-16 起票 Fable。151 / 152 / 154 / 158 を ELITPC に反映した後、主対話が実施)

## 手順
1. `daq.toml [system] ped_table = ~/test/scan/calib/pedestal_502ns.json`(502 ns = 運用点 B の表、必要なら取り直し)→ RUNBOOK §7.1 で反映(PATH / node の注意)→ daq_down/up。
2. run(各 1,500 events、運用点 B の電子回路): ① B + ZS(FPN 抑制)② B + ZS(FPN 閾値 0 = 残す)③ B + hit partial g4 + ZS(FPN 抑制)④ 同 FPN 残す。params は `zs_params.sh`(FPN 残しは `zs_exceptions.py` に `--keep-fpn` を足すか、票 155 で決める)。
3. 比較(run 130 / 141 = 非 ZS が基準): `run_metrics --ped`(電荷 −3 % 以内、ヒット ch)、`--zs-fill` の macros(長さ 84 ± 1、σ_z 0.21)、`hit_residuals` / `uvw_closure`(zsfilled)、PEventTPC の chargeMap 和の中央値(小マクロ、run 130 と ±5 %)、root_sink / monitor のカウンタ(`zs_frames_table` = フレーム数、`no_table` = 0)、decoder の frameType カウンタ(type 2 = 0)、UI の Event Display の基線。kB/event。
4. 判定: FPN を残すか(品質差 vs +32 kB/event)。ビーム run の既定 params を決める。記録: `docs/SCANS_ja.md` S18、RUNBOOK、CURRENT。

## 結果
