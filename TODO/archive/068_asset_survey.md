# 068 — pedestal/pulser 全ラン特性サーベイ(台帳・ノイズ・ゲイン一様性・経時安定性)

**Status: COMPLETED**(起票 Fable 2026-08-18 / 実施 2026-08-18。067/066 と並行実行 — リポ本体は変更していない)

## 背景

2026-08-18 に実データ到着(`reference/exp_data/2026/` pedestal 32 run / 35 GB、
pulser 26 run + 中断 3 本 / 4.3 GB。形式実測は 067 起票時に確定済み: pedestal =
frameType 2 compact、pulser = frameType 1・全フレーム 557,312 B 固定)。
ゲイン正規化(059 の優先 2 位)とイベント選別(066)の土台になる per-channel 特性を
**全ラン規模**で抽出し、18 日間の経時安定性まで見る。

## やること

- **A. ラン台帳**: `exp_data/2026` 全 .graw(physics / pedestal / pulser)のラン単位台帳
  (開始時刻・ファイル数とサイズ・フレーム数・イベント数・frameType・topology frame 有無)。
  機械可読(JSON または CSV)+ md 要約。0 バイト run(pulser に 3 本)も記載。
- **B. pedestal 特性**: 全 32 run の per-channel mean / RMS(生 ADC)。
  信号 1024 ch と FPN は**分離して**集計(FPN リオーダの規約は CLAUDE.md /
  reuse/rust_reference 参照。FPN={11,22,45,56})。ラン内(前半/後半)とラン間(18 日)の
  安定性、ノイズ異常 ch リスト(閾値は中央値 + k·MAD の機械基準、k を記録)。
- **C. pulser ゲイン**: 全 26 run の per-channel パルス波高(ベースライン差引、生 ADC)→
  ゲイン一様性マップ、AGET / AsAd 単位の系統差、経時安定性。
- 走査量が大きいので、**まず 1 run で手法の正しさを固めてから全走査**。サンプリング
  (run あたり先頭 N イベント等)で足りる場合は N と根拠を記録(silent 間引き禁止)。

## 実装の縛り

- **リポ本体を変更しない**(067 が src/ を並行編集中)。独立 cargo プロジェクトを
  `reference/_spike/asset_survey/` に作る。デコードは当リポの decode 系を再利用するのが第一
  選択だが、**067 の編集と衝突しないよう `git worktree add <scratch>/tpcdaq-survey HEAD` で
  HEAD を凍結したコピーに path 依存**すること(HEAD は cargo 454 green。作業後 worktree は
  `git worktree remove` で片付け)。lib として使えない構成なら、既存 bin の活用または最小の
  自前デコードで代替してよい — **自前実装する場合は frameType 2 の 1 フレームを既存テストの
  オラクル値か合成フィクスチャで照合してから使う**(誤ったデコードで統計を出さない)。
- frameType 1(pulser)は HEAD 実装(合成照合のみ)を使ってよいが、先頭イベントの波形が
  「ベースライン + 立ち上がり」の形をしていることの目視サニティを 1 ch 分残す
  (本格照合は 067-B の担当 — 重複しない)。
- 実データはローカルのみ。成果物(CSV/JSON/md)も reference/_spike 内に置き、リポに入れない。

## 検収

- 台帳 + 統計成果物 + `SUMMARY.md`(異常 ch リスト、経時傾向の要旨、ゲイン分布の要約統計、
  AGET/AsAd 系統差)。全数値に再現コマンドを併記。
- 発注書に書いていない設計分岐(物理解釈を含む)は実装せず報告。

## モデル

- implementer(Opus)。066(選別カット設計)への引き渡しを意識し、SUMMARY は
  「066/ゲイン正規化ユニットが次に使う数値」を先頭に置く。

---

## 結果

**実行日**: 2026-08-18 / macOS Darwin 25.5.0(Apple Silicon)/ rustc release build
**実装**: `reference/_spike/asset_survey`(独立 cargo プロジェクト。リポ本体は 1 行も変更していない)
**凍結コピー**: `git worktree add /private/tmp/claude-501/tpcdaq-survey HEAD`(HEAD = `c76f057`)へ
path 依存。作業後 `git worktree remove` 済み。**再実行時は同じ worktree を作り直す必要がある**
(`Cargo.toml` の `tpcdaq = { path = "/private/tmp/claude-501/tpcdaq-survey" }`)。

### 実行コマンド

```
cargo run --release --bin survey -- verify   <graw>          # デコード照合(門番)
cargo run --release --bin survey -- ledger   out             # A: 6.9 s
cargo run --release --bin survey -- pedestal out             # B: 73.9 s
cargo run --release --bin survey -- pulser   out             # C: 2.5 s
cargo run --release --bin survey -- dumpwf   <graw> 0 0 0  0  # 波形目視サニティ(信号 ch)
cargo run --release --bin survey -- dumpwf   <graw> 0 0 11 0  # 波形目視サニティ(FPN ch)
cargo run --release --bin survey -- inspect  <graw> 2         # ヘッダ / hit pattern / ch census
cargo run --release --bin survey -- evtscan  <graw> 10        # eventIdx / AsAd 並びの検査
```

`cargo fmt` 済み・`cargo clippy --all-targets` 警告ゼロ。

### サンプリング

**していない。** 全ラン・全ファイル・全フレーム・全イベント・全 512 bucket を走査した
(A+B+C 合計 40 GB を 84 s)。サンプリングを設計する前に全走査で足りることが分かったため。

### デコードの正しさ(統計を出す前の照合 — 発注書の縛り)

`survey verify` が実 .graw で高速展開 `fast::for_each_item` の `(aget, raw_ch, bucket, adc)` を
本体 `tpcdaq::decode::Decoder` + `msg::unpack_item` と 1 item ずつ突き合わせ、
`FrameHeader` の asad / event_idx を `decode::peek_asad` / `peek_event_idx` と突き合わせた。

```
pedestal(frameType 2): verified 8 data frames (1114112 items), control frames skipped=1, mismatches=0
pulser  (frameType 1): verified 8 data frames (1114112 items), control frames skipped=1, mismatches=0
physics (frameType 2): verified 8 data frames (1114112 items), control frames skipped=0, mismatches=0
```

framer resets = 0、全ファイルの末尾未消費バイト = 0。

### A — ラン台帳(要約数値)

| 種別 | run 数 | 0 バイト | バイト | フレーム | イベント | frameType | frame サイズ |
|---|---|---|---|---|---|---|---|
| physics | 1 | 0 | 4,295,503,872 | 15,408 | 3,852 | 2 | 278,784 固定 |
| pedestal | 36 | 0 | 37,164,695,472 | 133,346 | 33,334 | 2 + topology 7 | 278,784 固定 |
| pulser | 28 | 3 | 4,654,670,124 | 8,377 | 2,089 | 1 + topology 7 | 557,312 固定 |

- **発注書の run 数(pedestal 32 / pulser 26)は実測と不一致。実測が正**: pedestal 36 run(62 ファイル)、
  pulser 25 run + 0 バイト 3 本(28 ファイル)。
- 067 の検収参照値と一致: pulser `2026-08-17T08:09:11` = topology 1 + type1 304 frames
  (76 events × 4 AsAd)/ pedestal `2026-08-16T17:37:09` = topology 1 + type2 1868 frames
  (467 events × 4 AsAd)。dataSource は全データフレームで **1**。
- topology frame は**単一ファイル形式の `_0000` 先頭のみ各 run 1 個**。`_0001` には無く、
  physics(AsAd 毎ファイル)には 1 個も無い。

### B — pedestal 特性(全 36 run、1088 ch = 信号 1024 + FPN 64)

| 量 | 信号 ch | FPN ch |
|---|---|---|
| mean 中央値(ラン中央値のレンジ) | 345.0 – 347.6 ADC | 357.7 – 360.3 ADC |
| RMS 中央値(同) | 6.58 – 6.71 ADC | 1.98 – 2.17 ADC |
| RMS プール分布 min/p50/p95/max | 4.45 / 6.64 / 7.39 / 9.40 ADC | 1.72 / 2.05 / 2.39 / 2.56 ADC |
| ch 毎 mean の全域 | 224 – 512 ADC | — |

- **ラン内安定性**: 前半/後半の ch 毎 mean シフト最大値がラン全体で 0.30 – 0.93 ADC(RMS 6.6 に対し無視可)。
  分割点は `(first_event_idx + last_event_idx)/2`、`B_pedestal_run_summary.csv` の `split_event_idx` に記録。
- **ラン間安定性(18 日)**: ch 毎 mean のラン間 peak-to-peak が中央値 3.65 / p90 5.01 / p99 5.83 / max 6.24 ADC。
  RMS のラン間 peak-to-peak は中央値 0.148 ADC。単調ドリフトなし。
- **異常 ch 数**: 判定基準 = **中央値 ± k·MAD、k = 5.0**(mean と RMS の両方に適用)。
  各ランで信号 ch 7–9 本、**FPN は 0 本**。全 36 run の過半で flag された恒常異常 ch = **7 本**:

| asad/aget/raw_ch | signal_ch | mean | RMS | 種別 |
|---|---|---|---|---|
| 0/0/62 | 58 | 431.3 | 9.31 | 高ノイズ |
| 0/0/64 | 60 | 408.7 | 9.22 | 高ノイズ |
| 0/0/66 | 62 | 390.9 | 9.28 | 高ノイズ |
| 3/0/62 | 58 | 247.4 | 4.53 | 低ノイズ + 低ベースライン |
| 3/0/64 | 60 | 274.0 | 4.52 | 低ノイズ + 低ベースライン |
| 3/0/66 | 62 | 258.6 | 4.54 | 低ノイズ + 低ベースライン |
| 1/1/14 | 13 | 515.1 | 6.32 | ベースライン飛び出し(ノイズ正常) |

境界例: AsAd0/AGET2 raw 65(5/36 run)、raw 67(3/36 run)。

### C — pulser ゲイン(全 25 run、0 バイト 3 本除外)

波高定義 = `max(ADC, 全 512 bucket) − baseline`、`baseline = bucket 0..64 の平均`。
窓 64 は実測波形が根拠(信号 ch のピーク ~183 / FPN ch のピーク ~77、立ち上がりは FPN が ~72)。
実測 `peak_bucket_min` は信号 176–177 / FPN 76–77 で、どちらも窓に食い込まない。

| 量 | 値 |
|---|---|
| 信号 ch 波高中央値(全ラン中央値) | **3223.8 ADC** |
| ch 間 MAD(ラン毎) | 59.5 – 62.7 ADC(≈ 1.9%) |
| 相対ゲイン min/p1/p5/p50/p95/p99/max | 0.915 / 0.934 / 0.954 / 1.000 / 1.041 / 1.056 / 1.084 |
| k=5·MAD の異常 ch | **全 25 run で 0 本** |
| ch 毎波高のラン間 peak-to-peak | 中央値 26.0 / p90 33.5 / max 47.8 ADC(0.8 – 1.5%) |

AsAd/AGET 系統差(全ラン中央値、ADC): AsAd3 が全 AGET で系統的に低い(3138–3168)、
AsAd1/AGET1 と AsAd2/AGET2 が高い(3305/3304)。最小 3138 – 最大 3305 = **5.3% の AGET 単位系統差**。

**B と C のクロスチェック**: pedestal で恒常異常だった 7 ch は pulser では全て
relative_gain 0.993 – 1.029 の正常値。**死んだ ch は 1 本も無い**。

### 想定外の発見(4 件)

1. **単一ファイル形式は「イベント毎の連続塊」ではない** — pedestal で 25,438 箇所、
   同一イベントの 4 フレームが連続しない(例: `(104,0)(104,1)(104,2)(105,2)(104,3)(105,3)(105,0)(105,1)`)。
   eventIdx は単調で飛び・巻き戻りなし、乱れは常に隣接 1 イベント分。pulser と physics は 0 箇所。
2. **不完全イベント 15 個(pedestal 13 / pulser 2)は全て run 末尾の切断** — AsAd が抜ける順が
   3→0→1→2 と規則的で、run 中のデータ落ちではない(`incomplete_events.csv`)。
3. **pulser run では FPN 64 ch が 12 bit フルスケール 4095 に飽和**(全イベントの 98.7%)。
   FPN からゲインは取れない。`saturated_events` 列に計上済み。
4. **hit pattern のビット順を実測で確定(067-B の宿題)** — offset 31 からの AGET 毎 9 バイトを
   big-endian 72 bit 整数とみなし LSB から bit `c` = raw ch `c`(上位 4 bit は未使用)。
   pedestal は 0–67 全部立ち、pulser は **{11,22,45,56} = FPN だけが落ちる**。
   それでも item は 68 ch × 512 bucket = 139,264 個そろっている(ch census で確認)。
   067 背景節の「hit pattern の歯抜け」= **FPN の除外**であって readout の欠落ではない。

### 成果物(全て `reference/_spike/asset_survey/out/`、リポには入れない)

`SUMMARY.md`(066/ゲイン正規化が使う数値を冒頭に配置)/ `A_run_ledger.md` /
`run_ledger.csv` / `run_ledger.json` / `file_ledger.csv` / `incomplete_events.csv` /
`B_pedestal_channel_stats.csv`(36 run × 1088 ch)/ `B_pedestal_run_summary.csv` /
`B_pedestal_anomalies.csv` / `B_pedestal_persistent_anomalies.csv` / `B_pedestal_run_to_run.csv` /
`C_pulser_channel_stats.csv` / `C_pulser_run_summary.csv` / `C_pulser_aget_systematics.csv` /
`C_pulser_anomalies.csv` / `C_pulser_gain_map.csv` /
`C_waveform_sanity_asad0_aget0_ch0.txt` / `C_waveform_sanity_asad0_aget0_ch11_FPN.txt`

### 発注書に無い分岐として持ち帰る事項(実装せず報告)

1. **イベントビルダの並べ替え窓の深さ** — 実測では隣接 1 イベントで足りるが、SPEC への
   書き方は設計判断。
2. **run 末尾の不完全イベント(15 個)の扱い** — 捨てるか部分保存かは保存系ポリシー
   (Absolute Rule に触れる)。
3. **ゲイン正規化の基準点** — ここでは「全信号 ch の中央値 = 1.0」で正規化した。
   絶対スケール換算や FPN 基準の採否は物理側の決定。

### スキップしたテスト

なし(サンプリング・部分走査ともに行っていない)。

### Fable 裁定(2026-08-18、クローズ時)

1. **並べ替え窓の深さ**: 固定窓は設けない。eventIdx キー + build_timeout の現行方式が正で、
   実測上限(隣接 1 イベント)に対し十分 — SPEC §6.3 に実測値ごと追補済み(v1.23 同日追補)。
2. **run 末尾の不完全イベント**: ソース側 run stop 由来であり、既定の
   「incomplete フラグ付き emit(捨てない)」がそのまま適用される。追加ポリシー不要。
   Absolute Rule(保存系ロスレス)は「届いたものを落とさない」であり、ソースが送らなかった
   フラグメントの捏造・破棄のどちらもしない現行意味論で整合。
3. **ゲイン正規化の基準点**: 「全信号 ch 中央値 = 1.0」を作業正規化として承認。絶対スケール・
   基準の最終決定は物理側(ゲイン正規化ユニット起票時に Mikolaj/ユーザー判断へ)。
   発注書の run 数不一致(32→36 / 26→25+3)は台帳の実測が正 — 起票後にユーザーが
   ファイルを追加したため(ディレクトリ mtime で確認)。
