# 156 — ZS の検証 α run(B + ZS、B + hit partial g4 + ZS、FPN あり / なし)→ SCANS S18、FPN を残すかの判定

**Status: COMPLETED**(2026-10-16 起票 Fable。151 / 152 / 154 / 158 を ELITPC に反映した後、主対話が実施)

## 手順
1. `daq.toml [system] ped_table = ~/test/scan/calib/pedestal_502ns.json`(502 ns = 運用点 B の表、必要なら取り直し)→ RUNBOOK §7.1 で反映(PATH / node の注意)→ daq_down/up。
2. run(各 1,500 events、運用点 B の電子回路): ① B + ZS(FPN 抑制)② B + ZS(FPN 閾値 0 = 残す)③ B + hit partial g4 + ZS(FPN 抑制)④ 同 FPN 残す。params は `zs_params.sh`(FPN 残しは `zs_exceptions.py` に `--keep-fpn` を足すか、票 155 で決める)。
3. 比較(run 130 / 141 = 非 ZS が基準): `run_metrics --ped`(電荷 −3 % 以内、ヒット ch)、`--zs-fill` の macros(長さ 84 ± 1、σ_z 0.21)、`hit_residuals` / `uvw_closure`(zsfilled)、PEventTPC の chargeMap 和の中央値(小マクロ、run 130 と ±5 %)、root_sink / monitor のカウンタ(`zs_frames_table` = フレーム数、`no_table` = 0)、decoder の frameType カウンタ(type 2 = 0)、UI の Event Display の基線。kB/event。
4. 判定: FPN を残すか(品質差 vs +32 kB/event)。ビーム run の既定 params を決める。記録: `docs/SCANS_ja.md` S18、RUNBOOK、CURRENT。

## 結果

## 結果(2026-10-16 11:00–11:50、ELITPC、ユーザー承認で `daq.toml [system] ped_table` + root_sink make + daq_down/up。root_sink と monitor が表(272 ch、10-14 生成)を読み込んだログを確認。各 1,500 events、運用点 B の電子回路、`zs_params.sh` の params(ped + 5σ、`--keep-fpn` で FPN 残し))

| run | 設定 | kB/event | ped | ヒット ch | 総電荷 | 飽和 | ok | 長さ / IQR(zsfilled の macros) | dEdx / IQR | σ_z | σ_2D(表モード) |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 130 | B full、ZS なし(基準) | 278.8 | cells 1..20 | 121.5 | 2.09 M | 0.6 % | 0.998 | 84.4 / 58.5 | 13,673 / 6,413 | 0.21 | 0.314 |
| **170** | **B + ZS**(FPN 抑制) | **30.9(1/9.0)** | 表 | 121 | 2.02 M(−3 %) | 1.0 % | 0.998 | 83.9 / 58.5 | 13,639 / 6,402 | 0.20 | 0.320 |
| 171 | B + ZS、FPN 残し | 63.1 | 表 | 120 | 2.03 M | 0.9 % | 0.997 | 83.2 / 59.0 | 13,597 / 6,499 | 0.20 | 0.319 |
| **172** | **hit g4 + ZS**(FPN 抑制) | **23.1(1/12)** | 表 | 117 | 2.03 M | 0.9 % | 0.996 | 82.9 / 60.2 | 13,921 / 6,600 | 0.19 | 0.357 |
| 173 | hit g4 + ZS、FPN 残し | 55.7 | 表 | 116 | 1.99 M | 0.9 % | 0.997 | 82.0 / 60.4 | 13,808 / 6,606 | 0.19 | 0.354 |

decoder(run_stop.counters): 4 run とも `frames_type1` = events、`frames_type2` = 0、`compact_size_mismatch` = 0(ZS は frameType 1 のみ、158 の検査は発火せず)。macros は `--zs-fill`(154)で無改造、σ_2D は `uvw_closure --pedJson`(153)。

### 判定
1. **ZS(ch ごと ped + 5σ)は品質を落とさない**: B + ZS は基準と同じ指標(ヒット ch 121 / 121.5、ok 0.998、長さ 83.9 / 84.4、dEdx IQR 同、σ_z 0.20、σ_2D 0.320 / 0.314)で、電荷 −3 %(閾値未満の裾)。**データは 1/9**。
2. **FPN は抑制してよい(ユーザー裁定事項 → 推奨「抑制」)**: FPN を残しても指標は 1 つも改善せず(171 ≈ 170、173 ≈ 172)、+32 kB/event。共通モード補正の価値は B の S/N では見えない。
3. **hit partial g4 + ZS は最小(23 kB、1/12)**で、ヒット ch −4、σ_2D +12 %(弱いストリップが hit register で落ちる)と小さな代償。不感時間を縮める(150)のはこちらだけなので **ビーム run の既定候補 = hit g4 + ZS(FPN 抑制)**。線源 run は B full(現行)か B + ZS。
4. 未確認: root_sink の `pevent_zs_*` と monitor の `uvw_zs_*` は終了時 JSON / 停止ログにしか出ず run ごとに見えない(151 #11、152)→ **159 で close 行と status に出す**(157 の SPEC と同時)。UI の Event Display の基線は ZS run 中に目視していない(次の ZS run で)。PEventTPC の chargeMap 和の比較は未実施(TPCReco の辞書が要る小マクロ、159 に含める)。

## 実行できなかった操作
`daq.toml` 編集・daq_down/up はユーザー承認のもとで実施。
