# 178 — macros: サンプリング周波数とドリフト電圧を引数に、run の定数を出力ファイルに記録

**Status: COMPLETED**(2026-10-24。起票 Opus、実装 Sonnet implementer。tpcsim の TODO/14 §3「analyzeUVW.C の SAMPLING_MHZ / DRIFT_HV_V を引数かカードから取る形に
(12.5 MHz や 1600 V のランを同じマクロで解析するため)」、ユーザー「進めてください」)

## 0. 問題
- `macros/analyzeUVW.C` の `SAMPLING_MHZ = 25.0`・`DRIFT_HV_V = 1200.0` はデータから読まない定数(170 の README で注意)。run 180 以降は 12.5 MHz・1,600 V で、z の換算(ドリフト速度 × 時間)がずれる。
- `macros/stoppedAlphas.C` の z 窓は `Z_HI = 89 mm × 100 / p`(25 MHz・1,200 V を前提)で、12.5 MHz・1,600 V では窓(≈ 200 mm 超)を大きく切る。
- 169 の方針: 物理パラメータに既定値を持たせない(省略したら大きな音で失敗)。

## 1. 変更(決定済み)
### A. `macros/analyzeUVW.C`
- 署名を `analyzeUVW(const char* rawFile, double pressureMbar, double samplingMHz, double driftHvVolt, Long64_t first = 0, Long64_t n = -1, const char* outFile = "")` に。
  `SAMPLING_MHZ` と `DRIFT_HV_V` の定数を削除(`DRIFT_LEN_CM = 20` は残す — 幾何)。値の検査: samplingMHz ∈ {100, 50, 25, 12.5}(GET の取り得る値)、driftHvVolt > 0、外れたらエラーで出力を作らない。
- 出力の `_uvw.root` に **run の定数**を `TParameter<double>` で書く: `pressure_mbar`、`sampling_MHz`、`drift_hv_V`、`drift_v_cm_us`(計算値)、`mm_per_cell`、`z_window_mm`(= 512 × mm_per_cell)。
- 既存のドリフト速度の計算(E/p の表、0.54–1.05 の範囲外は WARNING)はそのまま。
### B. `macros/makeTracks.C`
- 入力の `_uvw.root` にある上記 `TParameter` を、出力の `_tracks.root` にそのまま写す(無ければ WARNING を 1 行出して写さない — 古い uvw ファイル)。
### C. `macros/stoppedAlphas.C`
- z 窓の上端を、tracks ファイルの `z_window_mm` から取る(`Z_HI × 100 / p` を廃止)。**無ければエラーで止める**(「古い tracks です。analyzeUVW → makeTracks を作り直してください」)。
- 署名は不変(`(trackFile, srcX, pressureMbar)`)。圧力の引数と tracks の `pressure_mbar` が 0.5 mbar 以上違えば WARNING(密度は引数の値を使う)。
### D. 呼び出し側
- `macros/par_analyzeUVW.sh`: `-s <samplingMHz>` と `-d <driftHvVolt>` を **必須**に(`-p` と同じ流儀、無ければ使い方を出して exit 2)。各分割に渡す。
- `macros/test_par_analyzeUVW.sh`: 新しい署名で呼ぶ(既存の T1–T4 は意味不変)。
- `tools/scan/metrics_for_run.sh`: `--sampling` と `--drift-hv` を必須に(`--pressure` と同じ)、analyzeUVW に渡す。
- `macros/README.md`: 使い方の例(`analyzeUVW.C("…_raw.root", 91, 25, 1200)`)と、注意の節(170 の「定数」)を「引数になった、出力に記録される」に書き換え。
## 2. テスト(受け入れ)
- `macros/test_par_analyzeUVW.sh data/run0014_raw.root <W>` が全 PASS(T1 同値性は新しい署名で、直列と並列が一致)。追加:
  - **T5** 引数の検査: samplingMHz = 30 や driftHvVolt = 0 でエラー・出力なし。sampling / drift-hv の無い `par_analyzeUVW.sh` が exit 2。
  - **T6** run の定数: `_uvw.root` と(makeTracks 後の)`_tracks.root` に 6 つの `TParameter` があり、値が引数と一致(25 MHz・1,200 V・100 mbar で mm_per_cell = 0.7233 × (1200 / 20 / 100) × 10 / 25 = 0.173592 mm、125 の値と一致)。
  - **T7** stoppedAlphas: `z_window_mm` の無い tracks ファイルで失敗(エラー文言)、ある tracks ファイルでは従来と同じ出力(25 MHz・1,200 V・100 mbar なら z_window = 88.9 mm ≈ 従来の 89 mm)。
- 改修前との同一性: 既定に相当する呼び方(100 mbar・25 MHz・1,200 V)で、`_uvw.root` の `uvw` 木が改修前と compareUVW で一致(TParameter の追加以外は不変)。
- `tools/scan/metrics_for_run.sh` の引数不足が exit 2(169 の T3d と同様)。
## 3. 報告
- `git diff --stat`、テスト項目ごとの PASS / FAIL、**実行できなかった操作**(無ければ「なし」)。
## 4. 範囲外
- ELITPC の `~/test/scan` の写し(scan.py 等の呼び出し)の更新は主対話が後で。

## 結果
### 実装(2026-10-24、Sonnet implementer、主対話 Opus が照合)
- 変更 7 ファイル(+238 / −44): `macros/{analyzeUVW.C, makeTracks.C, stoppedAlphas.C, par_analyzeUVW.sh, test_par_analyzeUVW.sh, README.md}`、`tools/scan/metrics_for_run.sh`。許可範囲の外は無し。
- 発注書からの追加: analyzeUVW.C は `pressureMbar > 0` も検査(0 割り防止)。
- **実装中に見つけた罠**: `TParameter<double>` は既定の merge モードだと **hadd で足し合わされる**(4 分割で mm_per_cell が 4 倍の 0.694368)。analyzeUVW.C は 6 つとも merge モード `'f'`(最初の値を保つ)で書く。
  主対話で makeTracks.C の写しも `'f'` にそろえた(tracks を hadd しても壊れないように)。
- テスト(`macros/test_par_analyzeUVW.sh data/run0014_raw.root <W>`、Mac、SDKROOT=MacOSX27.sdk):
  - T1 同値性(新署名 100 mbar / 25 MHz / 1200 V、直列 = 並列 `-j 4` / `-j 3`、17,596 entries identical)、T2–T4 は意味不変で新署名
  - T5 samplingMHz = 30 / driftHvVolt = 0 / < 0 → エラー・出力なし、`par_analyzeUVW.sh` の `-s` / `-d` 無し・`metrics_for_run.sh` の `--sampling` / `--drift-hv` 無しは exit 2
  - T6 `_uvw.root` と `_tracks.root` に 6 つの TParameter(25 MHz / 1200 V / 100 mbar: drift_v 0.43398 cm/µs、mm_per_cell **0.173592**、z_window 88.879 mm)、hadd 後も 0.173592、12.5 MHz / 1600 V で 0.462912(= 0.7233 × 0.8 × 0.8 mm)
  - T7a z_window_mm の無い tracks でエラー文言、T7b ある tracks で正常(圧力の WARNING なし)、T7c 圧力の引数を 1 mbar ずらすと WARNING
  - 改修前との同一性: `git show HEAD` の旧 analyzeUVW.C(`(raw, 100, 0, 3000)`)と新(`(raw, 100, 25, 1200, 0, 3000)`)の `uvw` 木が compareUVW で **3,000 entries identical**
  - red の証拠: 旧マクロ・旧スクリプトの上で新テスト一式 → 15 passed / 25 failed(implementer は実装を先に書き、red は後から確認 — TDD の順番からの逸脱として記録)
- **主対話の再実行**(2026-10-24 17:15、makeTracks の `'f'` 修正後): `TOTAL: 40 passed, 0 failed`(約 6.5 分)。
- 実行できなかった操作: なし(implementer 報告。置換の事故で固まった root を pkill して再試験)。
### 主対話の追加(呼び出し側の残り)
- 169 から壊れていた 1 引数の呼び出し 2 本を新署名に: `tools/macro_oracle/run_macro_oracle.sh`(`analyzeUVW.C("oracle_raw.root", 100, 25, 1200)` = `src/reco` の定数 100 mbar / 25 MHz / 1200 V)、
  `tools/event_mix/run_real_check.sh`(src と mix で同じ 100 / 25 / 1200)。README 2 本(`tools/macro_oracle`、`tools/scan` の usage 行)。
- **macros オラクルの再実行**: run 14 の先頭 300 entries を CopyTree した raw(scratchpad)→ `run_macro_oracle.sh` → `ws_proto_sample --strip-geometry-out`(実 mini ジオメトリ)→
  `cargo test --release --test reco_macro_oracle`: **7 passed**、production / csv-s とも events 300・ok 297・notOk 3(両側一致)・okMismatch 0・hitSetMismatch 0・hits 30,909、max|dpos| 4.65e-4 mm(production、s の丸め)/ 4.67e-6 mm(csv-s)。
- ELITPC の `~/test/scan/metrics_for_run.sh` と `~/test/ana/ana_107` は**リポと独立の古い組**(2 引数版どうし)で、そのままでも動く。リポの新しい metrics_for_run.sh を ELITPC で使うときは macros も 178 以降に入れ替えること。
