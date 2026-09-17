# 082 — `logbook2runs`: JSONL ログブック → minitpc-analysis `runs.yaml` 生成

**Status: COMPLETED**(2026-09-17 実装 Sonnet レーン → Fable レビュー 2 点差し戻し → main マージ `96e0997`。起票同日 Fable、080 D-6。run 条件の唯一の情報源を DAQ の logbook に寄せ、解析側
(別リポ `~/Workspace/Higs/minitpc-analysis`、**このセッションから書き換え禁止 — 読むだけ**)の `runs/runs.yaml` を
手打ちでなく生成する)

## 入出力

- 入力: `logbook.jsonl`(SPEC §9.2 — `run_start` / `run_stop` / `comment`、既存 `src/logbook.rs` の型で読む。
  末尾行の破損は警告して無視 = §9.1 の規律)+ 任意の **条件ファイル** `runs_defaults.toml`:
  ```toml
  [defaults]
  gas = "CO2"; temperature_K = 293.15; sampling_rate_MHz = 25.0; geometry = "geometry_mini_eTPC.dat"
  hypothesis = "alpha"
  [runs.57]            # run 番号ごとの上書き(圧力・HV は 079/B-4 が logbook に載せるまでここで与える)
  pressure_mbar = 100.0; drift_hv_V = 1600.0
  source = "Am (5.5 MeV alpha), on beam-entrance axis"
  ```
- 出力: `runs.yaml`(標準出力 or `--out`)。**形は現行 runs.yaml と同じキー**(下記)。1 run = 1 エントリ、
  run 番号昇順。`drift_velocity_cm_us` は **CO₂ かつ pressure と drift_hv が揃うときだけ `0.7233 × (E/p)`**
  (E = drift_hv_V / 20 cm、単位 V/cm / mbar。係数の出典 = SURVEY §2)で埋め、それ以外は `null` とコメント
  `# unknown`。
  ```yaml
  runs:
    - run_id: "<run_start.ts をファイル名の TS 形式に>"   # 実際は run_stop.files の graw 名から取る
      files: "<出力ルート>/CoBo*_<TS>_*.graw"
      format: graw
      n_events: <run_stop.counters.events_built または frames から>
      geometry: <run_start.geometry.path の basename>
      reco: { hypothesis: <defaults> }
      conditions: { gas, pressure_mbar, temperature_K, drift_hv_V, drift_velocity_cm_us, sampling_rate_MHz }
      source: { primary: <toml>, background: <toml> }
      notes: >  <run_start.comment + 同 run 中の comment レコード + run_stop.reason(normal 以外)>
  ```
- YAML は**自前の小さなエミッタ**(この固定スキーマだけ。文字列は常に二重引用符でエスケープ、数値は `{:?}`
  以外の素直な表記、null は `null`)。依存を増やさない。

## 発注(Rust、Sonnet)

- `src/bin/logbook2runs.rs` + `src/runs_yaml.rs`(純関数: `Vec<LogbookRecord>` + defaults → `RunsDoc` → String)。
- CLI: `logbook2runs <logbook.jsonl> [--defaults runs_defaults.toml] [--out runs.yaml] [--data-root <dir>]`
  (`--data-root` は `files` のパス頭。既定 = logbook の親ディレクトリ)。
- **silent 禁止**: run_stop の無い run(進行中/クラッシュ)は `notes` に `"no run_stop"` を書いて出す、
  parse できない行は行番号付きで stderr に数える。

## 受け入れ

1. 単体: SPEC §9.2 の `run_start` / `run_stop` 例(`src/logbook.rs` のオラクル文字列と同じもの)+ comment 1 行から、
   **期待 YAML を文字列ゴールデンで固定**(改行・インデント込み)。`drift_velocity` の計算例 1 つ(100 mbar /
   1600 V → 0.5786)。null 経路 1 つ。壊れ末尾行 1 つ。
2. 実ファイル: `reference/_spike/demo` の start_demo.sh で作った `logbook.jsonl`(手元にあれば。無ければ
   結果節に「未実施」と書く)で YAML が出て、Python `yaml.safe_load` が通ること(python3 に PyYAML があれば。
   無ければ結果節に記載)。
3. `cargo fmt && cargo clippy --tests -- -D warnings && cargo test` green。

## やらないこと

- Higs リポへの書き込み、runs.yaml の既存エントリ(2026-09-01/02)の再現(手打ちの注釈は再現対象外)、
  PEventTPC 形式の `format: peventtpc` 判定以外の解析側ロジック。

## 結果(2026-09-17、Mac、main `96e0997`)

- 実装: `src/runs_yaml.rs`(純関数 + TOML defaults + 寛容な JSONL リーダ、≈1,100 行)、`src/bin/logbook2runs.rs`(205 行)。
  依存追加なし(YAML は固定スキーマ用の自前エミッタ)。
- テスト: **cargo test 472 → 500 passed / 0 failed**(runs_yaml 23 + bin args 5)。`cargo fmt --check` / `cargo clippy --tests -- -D warnings` clean。
  文字列ゴールデン(SPEC §9.2 の run_start/run_stop 例 + comment)、v_drift 100 mbar / 1600 V → 0.5786、null 経路、壊れ末尾行、
  **4 AsAd で TS が数 ms ずれる run_stop.files**(実機命名の実例 …37.043/.046/.048/.051 をそのまま)。
- 実ファイル: `reference/_spike/demo/out/data/logbook_BC_saved.jsonl`(041 実証時の run 3 組)→ 3 run 分の YAML、
  `python3 -c "import yaml; yaml.safe_load(...)"` OK、`pressure_mbar` 等は float。
- レビュー(Fable)で直した 2 点: ①f64 は整数値でも `.0`(型を既存 runs.yaml に揃える)②**`files` の glob に TS を含めると
  複数 AsAd を取りこぼす実バグ** → run ディレクトリ単位 `<data-root>/run{NNNN}/CoBo*_AsAd*_*.graw` に変更(`graw_writer::run_dir` と整合)。
- 承認した逸脱: 入力は `LogbookLine`(comment を run に紐付けるため)/ 戻りは `(RunsDoc, Vec<String>)` で警告を可視化 /
  source は文字列 or `{primary, background}` テーブル(untagged)/ 文字列は全て二重引用符 / notes は `"; "` 結合。
- 使い方: `logbook2runs <logbook.jsonl> [--defaults runs_defaults.toml] [--out runs.yaml] [--data-root <dir>]`。
  圧力・HV は 079/B-4 が logbook に載るまで defaults TOML の `[runs.N]` で与える。
