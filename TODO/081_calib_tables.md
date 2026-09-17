# 081 — 較正表ツール `calib`(pedestal / pulser run → per-channel JSON)

**Status: READY**(2026-09-17 起票 Fable、080 D-1。068 のサーベイコード `reference/_spike/asset_survey`
(HEAD 凍結 worktree に path 依存するスパイク)を**リポ本体の bin に昇格**して、実験中に毎回同じ表が出るようにする)

## 目的

pedestal run と pulser run(実機 .graw、AsAd 毎ファイル)から **per-channel の baseline / noise / 相対ゲイン /
異常フラグ**を JSON 1 つにまとめる。これが 078 の「例外リスト」(閾値個別化)と macros / ROOT プロダクトの
ゲイン正規化の入力になる(080 D-2)。

## 発注(Rust、Sonnet)

- `src/calib/`(`mod.rs`, `pedestal.rs`, `pulser.rs`, `table.rs`)+ `src/bin/calib.rs`。
  **フレーム切り出しは本体 `framer::Framer`、item 展開は本体 `decode::Decoder`** をそのまま使う(asset_survey の
  `fast.rs` の高速経路は持ち込まない — KISS。速度は 068 実測 pedestal 全走査 74 s なので Decoder 直でも許容。
  遅すぎたら結果節に実測を書いて報告、最適化はしない)。
- CLI:
  - `calib pedestal <graw...> --out pedestal.json` — ch 毎(asad, aget, ch 0–67)の `mean`, `rms`(全 bucket、
    全イベント)、run 数、`is_fpn`。
  - `calib pulser <graw...> --out pulser.json` — ch 毎のパルス波高中央値(per-channel baseline 差引後、生 ADC)、
    `relative_gain` = 中央値 / 全信号 ch の中央値。FPN は `saturated` フラグ(4095 飽和率)を出しゲインは `null`。
  - `calib merge pedestal.json pulser.json --out calib.json --k 5` — 1 ch 1 レコード:
    `{asad, aget, ch, fpn, baseline, noise, gain, flags: [...], threshold_suggest}`。
    `flags`: `noisy`(noise > median + k·MAD)、`dead`(rms < 0.5)、`gain_out`(|gain−1| > 2·MAD 換算)、
    `saturated`。`threshold_suggest = round(baseline + 5·noise)`(整数 ADC、閾値個別化の初期値)。
    **k と MAD の値を JSON の `meta` に書く**(機械基準を残す — 068 の流儀)。
  - `meta` に入力ファイル一覧・イベント数・日付・ツール版(`env!("CARGO_PKG_VERSION")`)。
- 数値の定義は **068 SUMMARY §0 と同じ**(reference/_spike/asset_survey/out/SUMMARY.md、`src/pedestal.rs` /
  `src/pulser.rs` を読んで踏襲。転記ではなく再実装だが、定義を変えない)。
- **JSON の順序と型を固定**(`serde` で構造体、`BTreeMap` 不要 — Vec を (asad, aget, ch) 昇順で)。

## 受け入れ

1. 合成テスト(`tests/calib_synthetic.rs`): 既存の合成 .graw フィクスチャ(tests/fixtures、frameType 2/1)で
   `pedestal` / `pulser` / `merge` を通し、手計算オラクル(定数 ADC のフレームなら mean = 定数、rms = 0 等)と
   一致。flags のロジックは表 5 ch 程度の小さな JSON で分岐を全部踏む。
2. **実データ回帰(env-gated `TPCDAQ_EXP_DATA_2026=/Users/aogaki/Workspace/tpcdaq-rs/reference/exp_data/2026`)**:
   pedestal 1 run + pulser 1 run で **068 の CSV(`B_pedestal_channel_stats.csv` / `C_pulser_channel_stats.csv`)と
   ch 毎 mean / rms / 波高中央値が丸め誤差内で一致**(どの run を使ったかは CSV の列名 / `run_ledger.csv` で選ぶ)。
   一致しない ch があれば **一致率と最大差を結果節に書いて止まる**(黙って通さない)。
3. `cargo fmt && cargo clippy --tests -- -D warnings && cargo test` green。実データ回帰の所要時間を結果節に。

## やらないこと

- 閾値の xcfg への流し込み(078)、ゲイン正規化の適用(D-2)、ROOT 出力、サンプリング。
- `reference/_spike/asset_survey` の削除・変更(参照のみ。worktree 依存なので触らない)。
