# 153 — uvw_closure.C に `pedJson`(run_metrics と同じ表の経路)— 任意

**Status: COMPLETED**(2026-10-16 起票 Fable。Sonnet)

## 仕様
- 8 番目の省略可能引数 `const char* pedJson = nullptr`。`runmetrics::loadPedTable` を使い、表があれば ped = mean、FPN 項なし(`v = w − ped`)、`ped_source` / `ped_missing` を JSON に。`agets_without_fpn` の WARNING は表モードでは出さない。
- テスト: 合成 ZS raw(`test_run_metrics.C` の ZS fixture の流儀)+ 表 → `sigma_per_view` が非 ZS の 5 % 以内。既存 263 チェック不変。回帰 5 本。

## 結果
- 2026-10-16: Sonnet。8 引数 `pedJson`、表モード = ped = mean・FPN 項なし(`runmetrics::loadPedTable`)、JSON `ped_source` / `ped_missing`、表に無い ch は skip + 計数、表モードで `agets_without_fpn` は数えない。テスト [8]: 合成 ZS raw + 表 vs 非 ZS(同 seed、2,000 スライス)で σ/view 5 % 以内(0.364 vs 0.4 入力)。赤(コンパイル赤)→ 緑 284 ok(既存 263 不変)、回帰 6 本緑。

**Status: COMPLETED**(2026-10-16、Fable 照合)
