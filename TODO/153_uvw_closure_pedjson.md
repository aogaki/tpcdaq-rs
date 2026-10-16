# 153 — uvw_closure.C に `pedJson`(run_metrics と同じ表の経路)— 任意

**Status: READY**(2026-10-16 起票 Fable。154 の zs_restore で代替できるので任意。Sonnet)

## 仕様
- 8 番目の省略可能引数 `const char* pedJson = nullptr`。`runmetrics::loadPedTable` を使い、表があれば ped = mean、FPN 項なし(`v = w − ped`)、`ped_source` / `ped_missing` を JSON に。`agets_without_fpn` の WARNING は表モードでは出さない。
- テスト: 合成 ZS raw(`test_run_metrics.C` の ZS fixture の流儀)+ 表 → `sigma_per_view` が非 ZS の 5 % 以内。既存 263 チェック不変。回帰 5 本。

## 結果
