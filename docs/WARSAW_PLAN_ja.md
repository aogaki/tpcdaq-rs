# Warsaw 配備計画(ELITPC への tpcdaq-rs 導入)

- **status**: v1.0(2026-08-13 — Warsaw 先方ヒアリングを受けて策定)
- **前提文書**: [SPEC_ja.md](SPEC_ja.md)(実装の正本)、PROPOSAL v0.4(設計背景)
- **背景**: 先方は現行 DAQ の好感触と同時に 2 つの懸念を提示 —
  ①現在走っている Ubuntu 16 システムが壊れること、②「オールインワンで GUI を自由にいじりたい」。
  本書はその両方への回答と、デモまでの計画を固定する。

## 1. 配備方針: ゼロフットプリント(先方マシンに何もインストールしない)

**tpcdaq-rs 一式は持ち込みの安価な計算機 1 台で動かす。先方の Ubuntu 16 マシンは無変更。**

```
[zCoBo FW]──TCP──▶ ┌────────────────────────┐
[ECC/getHwServer]◀─Ice(1.1)── │ tpcdaq-rs 箱(現代 Linux) │──▶ graw / run.root / logbook
 (Ubuntu 16、無変更)          │ receiver/decoder/writer/    │
                              │ root-sink/controller/UI     │
                              └────────────────────────┘
```

- **制御プレーン**: ecc-bridge は ECC への Ice **クライアント**(encoding 1.1)。
  ECC / getHwServer / FW は 1 バイトも触らない(CLAUDE.md「やらないこと」)。
- **データプレーン**: DataLinkSet の向き先 IP:port を我々の箱にするだけ。これは run 開始時の
  ランタイムパラメータであり、先方システムへの永続的変更ではない。
- **ロールバック = 先方の既存スクリプトを叩くだけ**(我々の箱を無視すれば今日と同じ)。
  旧系/新系の日替わり A/B 併用も可能。
- Ubuntu 16 上での動作可否(Rust glibc ≥2.17 / musl 静的、TypeScript・JSROOT はブラウザ側)は
  検討済みだが、この配備形態では**そもそも問われない**。

## 2. 互換性の決定事項

| 面 | 決定 | 根拠 |
|---|---|---|
| 生 graw | 実機 DataRouter 命名・バイト互換(SPEC v1.1/v1.2 で対応済み) | 先方の bash 資産 + **grawToEventTPC(TPCReco)**を無改造で使えること |
| run.root の圧縮 | **既定を 101(ZLIB-1)に変更、設定可能に**(SPEC v1.5 → TODO/014) | **先方はオフライン解析も DAQ 計算機の同一(旧)ROOT で行う**(低レートゆえの運用、2026-08-13 確認)。ROOT < 6.20 は ZSTD を読めない。ZLIB は全時代互換 + C++ 版の「ROOT 既定」と一致。**実機 ELITPC 変換出力(PEventTPC、ROOT 6.08/06 書き)が ZLIB level 1 であることを直接確認**(2026-08-13、§4) |
| イベント ROOT 形式 | **PEventTPC(TPCReco)互換に変更**(SPEC v1.8 → TODO/020、2026-08-13 ユーザー裁定) | 先方は grawToEventTPC で PEventTPC に変換して解析している。我々の run.root を同形式にすれば**変換ステップ自体が消える**(オールインワン強化)。クラスは TPCReco-HIGS2026_online のビルド時参照(ライセンス未指定のためコミットせず — 許諾は夏休み明けに確認)。**2026-08-14 実証済み: 同一 run の実データで実機変換出力と全 3852 イベント完全一致**(`compared 3852 events, 0 differences` — TODO/021) |
| ハードウェア設定(xcfg) | **既存ファイルをそのまま使用**(2026-08-13 確認) | describe/prepare/configure の xcfg は生成・改変しない。ecc-bridge は `config_id` を渡すだけで、実体は ECC サーバ側の既存リポジトリ。CoBo/FPGA に入る内容は現行と完全同一。我々が作る XML は DataLinkSet(データ送り先)のみ |
| 最終確認 | デモ時に**先方 DAQ 計算機の ROOT で run.root を開く**のを受け入れ試験とする | 手元に旧 ROOT が無いため実機確認が正 |

## 3. GUI 方針(「オールインワン + 自由にいじりたい」への回答)

- 「オールインワン」= **単一の箱 + 単一の URL** という見え方で満たす(中の多プロセスは見せない)。
- 「いじりたい」の実体はヒストの中身・配置、**電源類(PSU)の表示**と確認(2026-08-13)。
  - **サポートモデルは「メンテナ・プッシュ」**: 物理屋が UI コードを触るのではなく、
    **要望を言ってもらえれば我々が変更して送る**(Aogaki の判断 2026-08-13 —
    その方が双方楽で速い)。これを前提に、ヒスト定義・表示構成は**設定駆動**にして
    変更コストを最小化する(delila-rs histograms.json の流儀)。
  - PSU 表示はロードマップ済み(SPEC: psu コンポーネント = P6、source_id 200、§9 psu レコード)。
    P4 のモニタ UI 設計時に PSU パネルを最初から枠として置く。
  - JSROOT は P4 の選択肢として維持(物理屋は run.root / monitor.root を自分の ROOT/JSROOT で
    開けばいつでも「完全に自由」— これが究極のエスケープハッチ)。

## 4. データ入手(実データ回帰の拡充)— **完了(2026-08-13)**

- Aogaki が現行 DAQ 計算機から 2022 / 2026 の実データを取得し `reference/exp_data/{2022,2026}/`
  に配置(各 `CoBo0_AsAd{0..3}_{TS}_0000.graw` の 4 ファイル + 2026 には実機オフライン変換の
  `PEventTPC_2026-08-11T07-47-37.051_0000.root`。**ローカルのみ、リポに入れない**)。
- **実機オフライン変換出力の実見**(PEventTPC ファイル、graw とは別 run):
  変換器は **TPCReco の `grawToEventTPC`**(GrawToROOT モジュール。ユーザー確認 2026-08-13 —
  「graw2root」は GET 付属の別ツール(→ GDataFrame)で、これまで双方が混同していた)。
  出力 = `TPCData` ツリー / `PEventTPC` クラス(eventraw::EventInfo 同梱)、
  entries=3852(_0000 4 本組に対応 — フル読み出し固定フレームなので _0000 は常に 3852 イベント)。
  **圧縮 = ZLIB level 1** — §2 の run.root 既定(101 ZLIB-1、SPEC v1.5)と完全一致を実機出力で
  直接確認。**書き手 = ROOT 6.08/06**(large-file フラグ付き)— 先方実運用の ROOT バージョンが
  確定(受け入れ試験の互換ターゲット)。同一 run でないため我々の GDataFrame 出力との
  サンプル照合オラクルにはならない(形式・圧縮・バージョンの参照として使う)。
- **grawToEventTPC の入出力規則**(HIGS2026_online ソース確認):
  入力 = **4 AsAd ファイルのカンマ結合**(graw 名のコロンはそのままでよい)。出力名 =
  `InputFileHelper::makeOutputFileName` が**リスト末尾ファイル**(AsAd 昇順なら AsAd3)の名前の
  `CoBo0_AsAd{A}` を `PEventTPC` に置換し、**コロン→ハイフン変換**して `.root` 化
  (実例: `...AsAd3_2026-08-11T07:47:37.051_0000.graw` → `PEventTPC_2026-08-11T07-47-37.051_0000.root`)。
  注意: HIGS2026_online スナップショットの同関数には**ミリ秒を落とす**分岐があるが、実ファイルは
  `.051` を保持 — 配備版バイナリとスナップショットに差異がある(出力名は実ファイルを正とし、
  D1 で配備版の実挙動を確認)。
  D1 の互換試験 = 我々の graw 4 本組をこの形で先方の grawToEventTPC に食わせて PEventTPC が
  出ること。
- 回帰追加済み: `TPCDAQ_REAL_GRAW_DIR` 環境変数の任意回帰(`tests/elitpc_real_graw.rs`、
  TODO/019)。両年 green。
- **実測での確定事項**(SPEC v1.7 に反映):
  - ELITPC はワイヤ上 **1 論理 CoBo × 4 AsAd**(2 枚の zCoBo を 1 CoBo として扱っている)。
    2-CoBo ジオメトリ .dat 問題(SPEC §13-7)は**解消** — 既存 ELITPC .dat がそのまま正。
  - 両年とも **frameType 2(compact)rev 5** — 2022 時点で既に compact であり、
    frameType 1 が実機オラクル対象になることはない(R2 は「frameType 1 の実データは存在しない」
    という形でクローズ。frameType 1 対応は合成フィクスチャでの保険として維持)。
  - ローテーション境界の実機挙動(書き込み後判定)を発見し graw-writer を修正
    (実ファイルとの完全バイト一致を回帰で固定)。

## 5. デモ計画(物理屋向け)

段階的に「こんな見た目になります」を見せる:

- **D1(今の実装で可能、2026 データ入手後すぐ)**: 2026 実データを graw_replay で
  フルチェーン(receiver → {graw-writer, decoder} → root-sink)に通し、
  ①実機命名の graw がバイト一致で出る、②run 毎単一の run.root が出る、
  ③**それを先方 DAQ 計算機の ROOT でそのまま開ける**(ZLIB 化後)、
  ④**我々の graw 4 本組を先方の grawToEventTPC に食わせて PEventTPC が出る**(§4 の入出力規則。
  オフラインチェーン無改造接続の本命試験)— を見せる。
  「何も壊さず、出てくるものは今と同じ形式」の実証 = 懸念①への直接回答。
- **D2(P3 完了後)**: run 制御のデモ — start/stop、JSONL ログブック、fake-ECC での全通し。
- **D3(P4 完了後)**: ライブモニタ UI — ヒスト・波形・(枠として)PSU パネル。
  スクリーンショット + ライブで「こんな見た目」を提示。
- **並行して**: PROPOSAL v0.4 をベースに**物理屋向け資料**(プロトコル詳細を落とし、
  見た目・運用・互換性保証を前面に出したもの)を作成する。D1/D3 の実画面を素材にする。

## 6. Warsaw への残確認事項

- ~~2-CoBo ジオメトリ .dat の有無~~ **解消**(2026-08-13 実データにより不要と確定 — SPEC §13-7)
- **データリンク本数**: 2 枚の zCoBo が 1 本の TCP で来るのか 2 本か(DataLinkSet の
  DataSender 数と receiver 台数 — どちらでも受けられる設計だが P5 で確認、SPEC §13-7)
- ネットワーク条件(DAQ LAN の帯域・MTU — SPEC §13。持ち込み箱の NIC 要件に影響)
- GUI 要望の具体化はメンテナ・プッシュ運用の中で随時受ける
