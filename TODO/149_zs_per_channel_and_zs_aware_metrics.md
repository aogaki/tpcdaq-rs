# 149 — ch ごとの ZS 閾値(pedestal 表から exceptions を生成)と、ZS データに対応した run_metrics

**Status: IN PROGRESS**(2026-10-15 起票 Fable。141 の次: 「ch ごとの ZS 閾値(S5 の表)」「解析側の ZS 対応」。080 D-2)

## 0. 狙い
141 で CoBo の ZS(一律 600)が端まで通り 14.8 kB/event(full の 1/19)だった。ped が ch により 364–548 ADC とばらつくので、**ch ごとに threshold = ped + k·σ** にすれば小さい信号を失わずに同じ圧縮が得られる。解析側は ZS データ(抑制されたセルが 0、FPN も抑制される)で ped / FPN 減算が成り立たないので、外部の pedestal 表を使う経路が要る。

## 1. 生成ツール `tools/scan/zs_exceptions.py`(標準ライブラリのみ、Sonnet)
- 入力: `calib pedestal` の JSON(`channels[] {asad, aget, ch, mean, rms, is_fpn}`、ELITPC `~/test/scan/calib/pedestal_502ns.json`)、k(既定 5)、下限 `--min-above`(既定 20 ADC)。
- 出力: params の `exceptions` 配列(JSON、標準出力または `--out`): 各 ch(FPN 含む)に `{"knob": "zeroSuppression.threshold", "at": {"asad": a, "aget": g, "channel": c}, "value": round(mean + max(k*rms, min_above))}`。range [0, 4095] にクリップ。`--merge BASE.json` があれば base params に `exceptions` を差し込み `zeroSuppression.enabled = true` にした params JSON を出す。
- テスト `tools/scan/test_zs_exceptions.py`(unittest): 3 ch の合成 pedestal で値・丸め・クリップ・min_above・FPN の扱い・merge の結果を固定。
- 注意: `exceptions[].at` の形(`{asad, aget, channel}`)と knob 名は `docs/PARAMS_KNOBS_ja.md` §4-3 / §6 と `src/params/` のスキーマに合わせる(読んで確認し、報告に引用)。

## 2. `run_metrics.C` の ZS 対応(Opus 相当だが発注書で縛る → Sonnet)
- 新しい省略可能引数 `const char* pedJson = nullptr`(`run_metrics(raw, outJson, pressure, tracksRoot?, pedJson)` — 既存の呼び出し互換を保つ。`metrics_for_run.sh` に `--ped FILE` を追加)。
- pedJson があれば: ped は表の `mean`(ch ごと)、**FPN 共通モード減算はしない**(ZS で FPN が抑制されるため)、ヒット閾値は従来どおり 35 ADC(ped 減算後)。無ければ従来どおり。
- ZS データの検出: 1 イベント中、信号 ch の cells 1..20 が全部 0 の ch の割合が 50 % を超え、pedJson が無ければ **WARNING を 1 回出す**(黙って誤った ped を使わない)。JSON に `ped_source: "cells_1_20" | "table"`、`zs_suspected: bool`。
- テスト `test_run_metrics.C` に追加: 合成 ZS raw(閾値未満のセルを 0 にしたもの)で、pedJson ありなら電荷・ヒット ch が非 ZS と一致(±5 %)、pedJson なしなら `zs_suspected` が true で WARNING。既存 88 チェックは不変。回帰: `test_pulser_kernel` 237 / `test_drift_end` 89 / `test_deconv` 133 / `test_hit_residuals` 145(すべて run_metrics.C を include している)。

## 3. 適用(主対話)
pedestal_502ns.json → exceptions(k = 5)→ params(B + ZS on)→ 1 run(1,500 events)→ `metrics_for_run.sh --ped` で指標、kB/event。141 の一律 600(run 121)・full(run 130)と比較。

## 結果
