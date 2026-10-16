# 154 — `tools/scan/zs_restore.C`(ZS raw → 抑制セルを ped で埋めた raw)+ `metrics_for_run.sh --zs-fill` + analyzeUVW.C のパッチ案(文面のみ)

**Status: READY**(2026-10-16 起票 Fable。ユーザー macros は変更しない — 橋で無改造のまま ZS データを通す。Sonnet)

## 0. 根拠
macros(`macros/analyzeUVW.C:192-198`、読むだけ)は ped = cells 1..20 の平均、閾値 35、FPN なし。ZS の raw(欠けたセル = 0)では ped ≈ 0 で壊れる(archive/149: 長さ 126 mm、σ_z 0.73)。抑制セルを表の round(mean) で埋めれば、ped = 表の mean(20 セル全部抑制が実質常に成り立つ、P ≈ 3e-7)、埋めたセルは `val = 0 ≤ 35` で無視 → 非 ZS と同じ振る舞い(差は ZS 固有の (ped+35, ped+max(5σ,20)) の損失と ≤ 0.5 ADC の丸めだけ)。

## 1. 仕様
- `int zs_restore(const char* rawIn, const char* pedJson, const char* rawOut, int maxEvents = 0)`: raw ツリーの契約(`eventId/i`, `eventTime/l`, `adc[N][68][512]/S`、N は leaf の長さから)をそのまま書き出し、規則 **cell == 0 かつ ch が表にある → round(mean)**(FPN ch も)。表に無い ch は不変 + 計数(WARN 1 回)。events / 埋めたセル数を印字。`rawOut == rawIn` は拒否(1)。
- `metrics_for_run.sh --zs-fill`(`--ped FILE` 必須): `OUT_DIR/zsfilled/runNNNN_raw.root` に復元(**同名必須** — macros は `_raw.root → _uvw.root` の置換で出力名を作る。別名だと analyzeUVW が入力を RECREATE する)→ そこで macros + `run_metrics --ped` → 指標 JSON は OUT_DIR に(`raw_file` = 復元パス)。
- README に節。票に **analyzeUVW.C の最小パッチ案**(任意 3 引数 `pedJson`、cells 1..20 のループを `ped = table[a][c]` に置き換え、約 15 行、makeTracks は無改造)を文面で置く(適用しない)。

## 2. テスト(`tools/scan/test_zs_restore.C`)
(a) ZS 合成 → 各セルが元の値か round(mean)、パルスのセルは同一。(b) 非 ZS 入力 → 同一ツリー。(c) **健全性**: `runmetrics::computeRunMetrics(復元, 表なし)` == `computeRunMetrics(ZS raw, 表)`(charge_median ±1 %、hit_channels 一致 — run_metrics のヒット定義は analyzeUVW のもの)。(d) 表に無い ch → 不変 + 計数。(e) 同名拒否。赤 → 緑、回帰(`test_run_metrics` 119 / `test_pulser_kernel` 237 / `test_drift_end` 89 / `test_deconv` 133 / `test_hit_residuals` 145 / `test_uvw_closure` 263)。
受け入れ(主対話、ELITPC): run 133 を `--zs-fill` で通すと macros の長さ ≈ 84 mm、σ_z ≈ 0.21 に戻る。

## 結果
