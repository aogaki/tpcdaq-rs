# 169 — macros: analyzeUVW の並列化(イベント範囲分割 + hadd)+ 圧力・線源位置の引数を必須に

**Status: COMPLETED**(2026-10-21 起票・同日完了 Opus、実装 Sonnet implementer。ユーザー「これって完全にイベントが独立なんで並列化が簡単ですが」
「run175 を macros/data で行いました。10 分以上かかったので短縮されれば今後の解析に役立ちます」。168 で圧力の既定値 100 の罠を踏んだ件も同梱)

## 0. 方針(決定済み、変えない)
- 並列はプロセス単位: イベント範囲で N 本の ROOT を並べ、`hadd` で合併。スレッド・RDataFrame・`EnableImplicitMT` は使わない(大きいブランチ `adc` が 1 本なので IMT は効かない)。
- 並列化するのは analyzeUVW だけ(makeTracks / stoppedAlphas は小さい入力で速い)。
- 物理パラメータ(圧力・線源位置)は**既定値を持たない**。省略したら大きな音で失敗させる(CLAUDE.md「silent failure を作らない」)。

## 1. 絶対条件
- **`macros/data/` の `_raw.root`(入力、run0175 は 130 GB)を変更・削除・移動しない**。生成物(`_uvw` / `_tracks.root`)は上書きしてよい(ユーザー「上書きしてくれても問題ないです。再構築は簡単です」2026-10-21)。
  ただしテストの出力は散らからないよう作業ディレクトリ `W = /private/tmp/claude-501/-Users-aogaki-Workspace-tpcdaq-rs/c5696e1c-f7d9-4a2e-ab7f-ab99dda8905b/scratchpad/par169/` に置き、入力は W への symlink で渡す。
- ELITPC(ssh)に触らない。git commit しない。
- analyzeUVW.C は `channel_map.csv` をカレントから読む → ROOT は `macros/` の中で実行する。
- Mac の ROOT 6.40 は `export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX27.sdk` が必須(無いと cling が落ちる)。
- 着手前に `git -C /Users/aogaki/Workspace/tpcdaq-rs status` を確認。作業はメインの作業ツリーで(worktree は使わない — macros/data が無いため)。触ってよいファイルは §2 のものだけ。

## 2. 変更
### A. `macros/analyzeUVW.C`
- 署名を `void analyzeUVW(const char *rawFile, double pressureMbar, Long64_t first = 0, Long64_t n = -1, const char *outFile = "")` に。
  - `pressureMbar` の既定値を外し、定数 `PRESSURE_MBAR` とそのコメントを削除(ヘッダの使い方の説明も直す)。
  - 処理する範囲 = エントリ [first, min(first + n, N))。n < 0 は末尾まで。`first < 0`、`first >= N`、`n == 0` はエラー表示して return(**出力ファイルを作らない** — `TFile out(...)` より前で判定)。
  - ループの添字は**全体の通し番号 i のまま**(`for (Long64_t i = first; i < last; ++i)`)。これで `i < NDISPLAY` のイベント表示は範囲に 0–4 を含む分割にだけ出る。
  - `outFile` が空でなければそこへ書く。空なら従来どおり `_raw.root` → `_uvw.root`。**出力名が入力名と同じになる場合(入力名に `_raw.root` が無い)はエラーで return**(入力の上書き防止)。
  - 末尾に処理範囲と件数を 1 行表示(例 `processed entries [0, 264430) = 264430 of 1057719`)。
  - 既定の引数(first = 0、n = −1、outFile = "")での出力は、改修前のマクロ(圧力を渡した場合)と**同一**であること。
### B. `macros/stoppedAlphas.C`
- 署名を `void stoppedAlphas(const char *trackFile, double srcX, double pressureMbar)`(既定値なし)。ヘッダの使い方の説明を直す。`-470` の既定値は古い(現行配置は ≈ −370)。
### C. `macros/par_analyzeUVW.sh`(新規、bash、`set -euo pipefail`)
- 使い方: `par_analyzeUVW.sh -p <pressure_mbar> [-j N] [-o <out_uvw.root>] [--tracks] <run_raw.root>`
  - `-p` 必須(無ければ使い方を出して exit 2)。ログの冒頭に圧力・N・入力・出力を表示。
  - N の既定 = コア数 − 2(Mac `sysctl -n hw.ncpu`、Linux `nproc`)、最小 1。
  - 全エントリ数は ROOT の 1 行で tree `raw` から取得。N 本の連続した範囲に分割(余りは前の方に 1 件ずつ)。N > エントリ数なら N = エントリ数。
  - スクリプトのあるディレクトリ(= macros)で、各分割を `root -l -b -q 'analyzeUVW.C("<raw の絶対パス>", p, first, n, "<tmp>/part_KK.root")'` として**並列**に実行(バックグラウンド + `wait`、各ログは `<tmp>/part_KK.log`)。
  - どれか 1 本でも非 0 終了か出力が無ければ、どの分割かを表示し、tmp を残して exit 1(**合併出力は書かない**)。
  - 出力の既定 = 入力の `_raw.root` → `_uvw.root`(従来の置き場所 = drop-in)。既存の出力は直列版と同じく上書きする。**出力名が入力名と同じになる場合はエラー**(入力の上書き防止)。
  - `hadd -f <out> part_00 … part_{N-1}`(分割の順に)。`hadd` が PATH に無ければエラー。
  - 検証: 合併後の `utree` のエントリ数 = 各分割の件数の和 = 処理範囲の件数。合わなければ exit 1。
  - `--tracks` なら合併後に `makeTracks.C(<out>)` を 1 本で実行。
  - 成功したら tmp を消す。経過時間を表示。
### D. `tools/scan/metrics_for_run.sh`
- `--pressure` を必須に(既定 100 を削除)。無ければ使い方を出して exit 2。ヘッダのコメントを直す。

## 3. テスト(受け入れ)
- `macros/compareUVW.C`(新規): 2 つの `_uvw.root` を比べ、`utree` のエントリ数、全エントリ・全ブランチの値(eventId の順序を含む、vector は要素ごと)、`hCharge` と `hWidth_*` の全ビン、`evt_*` キーの集合が一致すれば `PASS`、違えば最初の差を表示して `FAIL`(終了コードにも反映)。
- `macros/test_par_analyzeUVW.sh <raw.root> <workdir>`(新規): 以下を順に実行し、各項目の PASS / FAIL と最後に合計を表示。
  - **T1 同値性**: 直列(`analyzeUVW.C(raw, 100)`、出力は workdir)と、並列 `-j 4` および `-j 3`(余りあり)の出力が compareUVW で一致。
  - **T2 範囲の端**: `first >= N`、`n == 0`、`first < 0` でエラー表示かつ出力ファイルが作られない。
  - **T3 必須引数**: `analyzeUVW.C("…_raw.root")`(圧力なし)、`stoppedAlphas.C("…")`(引数不足)、`par_analyzeUVW.sh`(`-p` なし)、`metrics_for_run.sh`(`--pressure` なし)がすべて失敗する(終了コード非 0 またはエラー文言)。
  - **T4 入力の保護**: 入力名に `_raw.root` を含まないファイル(例 W に作った `foo.root` symlink)を渡すと、analyzeUVW.C も `par_analyzeUVW.sh` もエラーで終わり、入力(symlink 先)の shasum が変わらない。
- 入力: `W/run0014_raw.root` → `/Users/aogaki/Workspace/tpcdaq-rs/macros/data/run0014_raw.root` への symlink(2.3 GB)。圧力 100 は同値性の試験用の任意値。
- **ベンチマーク**: run 175(`W/run0175_raw.root` → `macros/data/run0175_raw.root` の symlink、130 GB)を `par_analyzeUVW.sh -p 91 -o W/run0175_uvw_par.root` で既定 N の並列実行し、経過時間を記録。
  直列の比較は全件でなく先頭 100,000 件(`analyzeUVW.C(raw, 91, 0, 100000, "W/serial_100k.root")`)の時間から外挿。
  同じ 100,000 件の範囲を並列でも作って compareUVW で一致を確認。

## 4. 報告(implementer → 主対話)
- `git diff --stat` と新規ファイルの一覧。
- §3 の各テストの名前・結果(PASS / FAIL)、T1 のエントリ数、ベンチマーク(N、並列の経過時間、直列 100k の時間と全件への外挿、速度比)。
- **実行できなかった操作**(権限拒否・失敗したコマンド)。無ければ「なし」。

## 5. 範囲外
- ELITPC 側の macros の写し(ユーザーが必要なら後で反映)。graw2root の並列化。報告書類。

## 結果(2026-10-21、Mac aarch64 14 コア、ROOT 6.40.04 + SDKROOT=MacOSX27.sdk、実装 Sonnet implementer → 主対話 Opus が照合)
### 変更
- `macros/analyzeUVW.C`(+45 / −):署名 `(rawFile, pressureMbar, first = 0, n = -1, outFile = "")`、`PRESSURE_MBAR` 削除、範囲・出力名の判定を `TFile out` の前に、入力と同名の出力はエラー、末尾に `processed entries [a, b) = c of N`。
- `macros/stoppedAlphas.C`:`(trackFile, srcX, pressureMbar)` 既定値なし、使い方の例を −370 に。
- `tools/scan/metrics_for_run.sh`:`--pressure` 必須(無ければ exit 2)。
- 新規:`macros/par_analyzeUVW.sh`(並列 + hadd + 件数検証、`--tracks`)、`macros/compareUVW.C`(ビット一致の比較)、`macros/test_par_analyzeUVW.sh`(T1–T4)。
- 主対話:`macros/README.md` の使い方を新しい署名と並列版に更新(implementer の範囲外だった L8 と圧力の注記)。

### テスト
`macros/test_par_analyzeUVW.sh data/run0014_raw.root <W>` → **15 / 15 PASS、0 FAIL**(implementer と、主対話が空の別ディレクトリ `par169_verify` で再実行して同じ結果、34 s)。
| 項目 | 結果 |
|---|---|
| T1a 直列が出力を作る | PASS |
| T1 同値性 直列 == 並列 `-j 4` / `-j 3`(余りあり)、compareUVW | PASS / PASS(17,596 エントリ、全ブランチ・全ビン一致) |
| T1b `par_analyzeUVW.sh` の終了コード(j 4 / j 3) | PASS / PASS |
| T2 `first >= N` / `n == 0` / `first < 0` → エラー、出力なし | PASS × 3 |
| T3a–d 必須引数(analyzeUVW 圧力なし / stoppedAlphas 引数不足 / par `-p` なし exit 2 / metrics_for_run `--pressure` なし exit 2) | PASS × 4 |
| T4a–c `_raw.root` の無い入力を拒否(analyzeUVW / par)、入力の shasum 不変 | PASS × 3 |
- 改修前との同一性:`git show HEAD:macros/analyzeUVW.C` を run 14・100 mbar で流した出力と、改修後の既定引数の出力が compareUVW で PASS(17,596)。
- 主対話の照合:`git status` で変更は §2 のファイルのみ。各テスト名が `test_par_analyzeUVW.sh` に存在。エントリ数が取れない・空・不一致のときは exit 1(`|| true` は `sed` の後ろだけで、空値は直後の判定で弾く)。`macros/data/run0175_raw.root` は 130,525,848,843 B、`run0014_raw.root` は 2,277,706,776 B のまま。

### ベンチマーク(run 175、1,057,719 エントリ、130 GB)
- `par_analyzeUVW.sh -p 91`(既定 N = 12):**経過 90 s**(user 1,008 s、CPU 1,145 %)、合併 1,057,719 エントリを検証。
- 直列 先頭 100,000 件:79.8 s → 全件外挿 ≈ 844 s(≈ 14 分、ユーザーの体感「10 分以上」と整合)。**速度比 ≈ 9.4 倍**。
- 範囲 [0, 100000) の直列と並列(25,000 × 4 + hadd)は compareUVW で PASS(100,000 エントリ)。
- 出力は scratchpad `par169/`(`run0175_uvw_par.root` ほか)。`macros/data/` には何も書いていない。

### 逸脱・補足
- implementer が初回に「`root -e` の非 0 終了 + `set -euo pipefail` で無言終了」を踏み、件数取得の `sed` の後ろに `|| true` を付けて修正(空値は直後に判定)。
- 100k の並列側は `par_analyzeUVW.sh` に範囲指定が無いため analyzeUVW の `first` / `n` + hadd を直接使用(発注どおりの確認目的)。
- スキップした試験:なし。ELITPC 側の macros の写しは範囲外(必要ならユーザーが反映)。

## 実行できなかった操作
なし(implementer・主対話とも)。
