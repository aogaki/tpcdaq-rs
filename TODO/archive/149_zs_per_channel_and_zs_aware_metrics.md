# 149 — ch ごとの ZS 閾値(pedestal 表から exceptions を生成)と、ZS データに対応した run_metrics

**Status: COMPLETED**(2026-10-15 起票 Fable。141 の次: 「ch ごとの ZS 閾値(S5 の表)」「解析側の ZS 対応」。080 D-2)

## 0. 狙い
141 で CoBo の ZS(一律 600)が端まで通り 14.8 kB/event(full の 1/19)だった。ped が ch により 364–548 ADC とばらつくので、**ch ごとに threshold = ped + k·σ** にすれば小さい信号を失わずに同じ圧縮が得られる。解析側は ZS データ(抑制されたセルが 0、FPN も抑制される)で ped / FPN 減算が成り立たないので、外部の pedestal 表を使う経路が要る。

## 1. 生成ツール `tools/scan/zs_exceptions.py`(標準ライブラリのみ、Sonnet)
- 入力: `calib pedestal` の JSON(`channels[] {asad, aget, ch, mean, rms, is_fpn}`、ELITPC `~/test/scan/calib/pedestal_502ns.json`)、k(既定 5)、下限 `--min-above`(既定 20 ADC)。
- 出力: params の `exceptions` 配列(JSON、標準出力または `--out`): 各 ch(FPN 含む)に `{"knob": "zeroSuppression.threshold", "at": {"asad": a, "aget": g, "ch": c}, "value": round(mean + max(k*rms, min_above))}`(`at` のキーは schema どおり `ch`)。range [0, 4095] にクリップ。`--merge BASE.json` があれば base params に `exceptions` を差し込み `zeroSuppression.enabled = true` にした params JSON を出す。
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

## 結果(2026-10-15 14:30–14:40、run 133、運用点 B + ZS(ch ごと ped + 5σ、`base_zs_k5.json`、272 exceptions、閾値 中央値 504 / 410–674 ADC)、full readout、1,500 events。電源復帰直後の最初の run/start は configure(AGET reg 読み戻し 0)で失敗、再試行で通った)

| run | 設定 | kB/event | ped の出所 | ヒット ch | 総電荷 | 飽和 | ok | 長さ [mm] | dEdx / IQR | σ_z [mm] |
|---|---|---|---|---|---|---|---|---|---|---|
| 130 | B、full、ZS なし | 278.8 | cells 1..20 | 121.5 | 2.09 M | 0.6 % | 0.998 | 84.4 | 13,673 / 6,413 | 0.21 |
| 121 | A、partial、ZS 一律 600 | 14.8 | (無効) | 121 | (無効) | 0.7 % | 0.997 | 86.0 | (無効) | — |
| **133** | **B、full、ZS ch ごと ped+5σ** | **31.8** | **表(`--ped`)** | 123 | **2.02 M** | 1.1 % | 0.989 | **126.2 ※** | 13,530 / 5,649 | **0.73 ※** |

- **道具は成立**: `run_metrics --ped` で ZS データの電荷(2.02 M vs 2.09 M = −3 %)・ヒット ch・飽和が非 ZS と一致。データ量は full の **1/8.8**(一律 600 の 1/19 より大きいのは、閾値が多くの ch で 600 より低く(中央値 504)小さい信号を残しているため = 意図どおり)。
- **※ ユーザー macros(analyzeUVW / makeTracks)は ZS データで壊れる**: 長さ 126 mm(84 が正)、σ_z 0.73 mm(0.21 が正)。macros が波形内から ped を取る(ZS で抑制されたセルが 0)ためと思われる。**macros はユーザーのもので触らない** → ZS を本番に使うなら、macros 側に pedestal 表(`calib/pedestal_*.json`)を読む経路が要る(ユーザー判断)。それまで運用は ZS なし(full、279 kB/event)。
- 141 の「一律 600 の run 121 の電荷が 2 倍に見えた」は ped 未減算によるもので、`--ped` で解決する種類の問題(確認済み)。

## 実行できなかった操作
なし(HV/LV の ON はユーザーが UI から)。
