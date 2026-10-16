# 155 — `tools/scan/zs_params.sh`(pedestal run → 表 → ZS 付き params)+ RUNBOOK「ZS 運用」

**Status: READY**(2026-10-16 起票 Fable。Sonnet)

## 仕様
- `zs_params.sh (--ped-run RUN_DIR | --ped-json FILE) --base BASE.json --out OUT.json [--k 5] [--min-above 20] [--label 502ns]`: `--ped-run` なら `calib pedestal <graw...> --out ~/test/scan/calib/pedestal_<label>.json`(構文は `src/bin/calib.rs` を読む)→ `tools/scan/zs_exceptions.py --k --min-above --merge BASE --out OUT` → 要約(例外数、閾値の中央値 / 最小 / 最大、FPN の閾値、**base の peakingTime_ns / gain をラベルの隣に表示** = 表と設定の不一致を操作者が見られるように)。PUT はしない(UI か scan.py で適用)。
- `test_zs_params.sh`: 合成 3 ch 表 + base fixture で `--ped-json` → `zeroSuppression.enabled = true`、例外 3、要約にラベル、不正 base → 非 0 終了。
- `docs/RUNBOOK_ja.md` に「ZS 運用」節: 手順(pedestal run = `triggerMode periodically` + `readout full` 150 events → `zs_params.sh` → params 適用)、**電子回路設定(peaking / gain)を変えたら表を取り直し、`[system] ped_table` を更新して daq_down/up**、root_sink / monitor / decoder のカウンタの読み方(`zs_frames_no_table` が 0 でないときの意味)。README に節。

## 結果
