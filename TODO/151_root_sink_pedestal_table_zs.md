# 151 — root_sink: pedestal 表(`[system] ped_table`)と ZS 対応の PEventTPC fill

**Status: READY**(2026-10-16 起票 Fable。ZS 対応計画(plan 2026-10-16、ユーザー承認「全部」)の中核。Opus)

## 0. 問題
`tools/root_sink/pevent_fill.hpp:304-358` の pedestal(cells 5..25 の平均 − FPN のセル平均、TPCReco `PedestalCalculatorGRAW` の忠実な移植)は、zCoBo の ZS データ(閾値未満のサンプルが**存在しない**。FPN ch も抑制)で offset ≈ 0 / 偏り、FPN 項 0 になり、PEventTPC の chargeMap が生 ADC のまま(= TPCReco でも同じ壊れ方)。archive/149 run 133。

## 1. 仕様
1. **設定**: `src/config.rs` の `SystemConfig` に `ped_table: Option<PathBuf>`(`[system] ped_table = "/path/pedestal_502ns.json"`)。`geometry` と同じく parse 時に存在検査(無ければエラー)。`config/daq_example.toml` に注記つきで追加。`tools/deploy/daq_up.sh` は `toml_get` で読んで root_sink に `--pedestal-table FILE` を渡す(キー名はファイル内で一意にする)。
2. **表**: `calib pedestal` の JSON(`src/calib/pedestal.rs`: `{meta, channels:[{asad, aget, ch, mean, rms, runs, is_fpn}]}`)。root_sink には JSON パーサが無いので `tools/root_sink/ped_table.hpp`(新)に `tools/scan/run_metrics.C:211-249 loadPedTable` と同じ平坦オブジェクト走査を書き、`double mean[nAsad][4][68]`(無い ch は NaN)に入れる。CLI `--pedestal-table FILE`(開けない / `channels` 無し = 起動エラー、SPEC §3.2 の流儀)。起動時に path・`meta` の生成日時・ch 数をログ。
3. **ZS らしいフレームの述語**(`Filler::frame_is_zero_suppressed()`、`grid_` の件数から): その (cobo, asad) フレームで「サンプルが 1 つ以上ある ch」のうち**過半**が「ped 窓 [min_pedestal_cell, max_pedestal_cell](既定 5..25 = 21 cell)内のサンプル数 < 21」なら ZS らしい。full / partial(非 ZS)は読んだ ch が全セル揃うので false、ZS は pre-trigger のサンプルがほぼ無いので true。
4. **表モード**(ZS らしい && 表あり): correction = 表の mean(asad, aget, raw_ch)、**FPN 項なし**、信号窓は不変、存在するサンプルだけ `value = adc − mean` で `AddValByStrip`(欠けたサンプルは chargeMap に無いまま)、`pedestalSubtracted = true`。表に無い ch は skip して `pevent_table_missing_channels` に計数(WARN 1 回)。
5. **表なし**(ZS らしい && 表なし): 従来算法のまま + `pevent_zs_frames_no_table` に計数、WARN は run 毎 1 回。**保存は止めない**(graw が保険)。
6. **非 ZS フレーム**: 表が設定されていても**従来算法のまま**(ビット一致)→ §12-3 のオラクル(grawToEventTPC 一致)は不変。
7. カウンタ `pevent_zs_frames_table / pevent_zs_frames_no_table / pevent_table_missing_channels` を終了 JSON(run_stop に載る経路)に。close 行は変えない。

## 2. テスト(`tools/root_sink/test_pevent.cxx` 系、既存の流儀)
- (a) 合成 ZS フラグメント(各 ch で mean + 10 未満のセルを除去、FPN は全部無し)+ 表 → chargeMap = 手計算(`adc − mean`、信号窓、無いキーは無い)。
- (b) 同じフラグメント、表なし → `no_table` 1、WARN、値は従来どおり。
- (c) full(非 ZS)フラグメント + 表設定 → **従来とビット一致**(述語 false)。
- (d) partial 非 ZS(いくつかの ch が丸ごと無い)→ 述語 false。
- (e) 表に無い ch → skip + 計数。
- `ped_table.hpp` の単体: 無いファイル、`channels` 無し、無い ch は NaN。`src/config.rs` のキーのテスト(存在検査)。
- 赤 → 緑の実測。`make -j` 警告 0、`make test-root` 緑、`run_conformance.sh` 緑、`cargo test` 緑(config)。`tests/elitpc_pevent_e2e.rs` は不変(env-gated で実 graw + `--pedestal-table` 付きで再実行しても 0 differences = 述語が full データで発火しない証拠。実資産の場所はメモリ local-real-assets)。

## 3. 報告に必須
変更ファイル(`git show --stat`)、仕様 1–7 と対応するテスト名、赤/緑の実測、オラクル不変の証拠、「実行できなかった操作」(無ければ「なし」)、発注書に無い判断、SPEC §3.1/3.2/§6.4 の文案(編集はしない)。設計上の分岐で迷ったら実装せず報告。

## 結果
