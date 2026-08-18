# CLAUDE.md — tpcdaq-rs 絶対ルール(常時適用)

> **実装の正本は [docs/SPEC_ja.md](docs/SPEC_ja.md) v1.0**(2026-08-12 レビュー通過)。
> PROPOSAL v0.4(日英)は設計背景 + Warsaw 回覧文書(SPEC との差分は SPEC §14)。タスクは [TODO/](TODO/)。

## Absolute Rule — データ保全(tpcdaq 版)

**保存系は絶対にデータを落とさない。モニタ系は最新優先で落としてよい。この区別を混ぜない。**
- 保存系(graw-writer / root-sink): ロスレス。PUSH/PULL の背圧で守る。意図的ドロップ禁止。
- モニタ系(monitor / UI 配信): PUB/SUB + 間引き。落としてよいが **silent にしない**(ドロップはカウントして可視化)。
- receiver は drain のみ。下流の遅延がソケットへ逆圧してはならない(never-stop)。
- 「保存と可視化の分離」(C++ 版ハードニング C3 の教訓)をコンポーネント境界として維持する。

## User Profile

- **User:** Aogaki — Senior Engineer, 27yr C++ experience, PhD in Computer Engineering
- **Role:** Claude = "Junior Rust Partner"。Rust は C++ アナロジーで説明。所有権・メモリレイアウト・性能に焦点。
- 基本アルゴリズムの講釈は不要。Rust 固有の構文と borrow checker の解決に集中。

## Project Overview

- **Goal:** GET ベース TPC(mini eTPC @ ELI-NP → ELITPC @ Warsaw)のフル DAQ 置換
  (受信 → デコード → 保存二系統 → ライブモニタ + run 制御 + Web UI)。最低目標トリガー 100 Hz。
- **Architecture:** delila-rs 型コンポーネントシステム(ZMQ)。
  receiver(CoBo 毎) → decoder → {graw-writer, root-sink(C++), monitor} + controller → ecc-bridge(C++)
- **Reference(ローカル参照、リポには入れない):**
  - `~/test/get/tpcdaq/` — C++ 版 = リファレンス + テストオラクル(events=108/items=15,040,512)+ サテライト供給元
  - `~/WorkSpace/delila-rs/` — コンポーネントシステムの手本。`tools/root_sink`(C++)= イベントビルダ + ROOT 書き出しの流用元
  - **`reference/`(リポ内、.gitignore 済み — コミット絶対禁止)** = 「これらと同等のことをやる」対象 + 変更不能な制約(2026-08-12 配置):
    - `20190315_patched/` — GET software(**実験使用中と同一版**)。ecc-bridge の検証相手、.ice 定義、GetController/MDaq の意味論、CoBoFrameViewer(デコード照合元)
    - `TPCReco/` — ジオメトリ .dat の正(`GeometryTPC`)+ オフライン互換ターゲット(**grawToEventTPC** — ELITPC 実運用の変換器。「graw2root」は GET 付属の別ツール(→ GDataFrame)で、混同注意 — 2026-08-13 確認)。全スナップショット調査済み(2-CoBo .dat は「不要」で決着 — SPEC §13-7 v1.7)
    - `ZC706_20181031_ELINP/` — zCoBo FW(**変更不能の足枷 = ワイヤ上の挙動の真実**)。SD_image / 1・2 AsAd 設定 / describe・configure xcfg / 運用スクリプト。要注目: `README_SCRIPTS.txt` の実 DataLinkSet 例(`DataSender id="CoBo[0]"` を FW 側資料でも確認済み 2026-08-12)、`CoboFormats-Rev-5-Compact.xcfg`(frameType 2 形式定義)、`MergedDataFormats-ByEventId/ByEventTime-*.xcfg`(Q3 順序問題の一次資料)

## Tech Stack

Rust 2021 + tokio + ZMQ + serde 系(delila-rs に揃える。確定は仕様書)。
UI: Angular + Angular Material + ECharts(delila-rs operator UI と同一スタック)。
C++ サテライト(tools/): ROOT(root-sink)、ZeroC Ice(ecc-bridge、**encoding 1.1 固定**。
ローカル実態は Ice 3.8 系 — 旧「3.6.3」表記は 2026-08-14 訂正。実 ECC ローカルビルドは
3.7 keg(unlinked)で共存、docs/VIRTUAL_ZCOBO_ja.md §4.1)。

## Design Principles (Priority Order)

1. **KISS** — 動く最小のものを書く。先回りした抽象化をしない。**KISS は他のどの指針にも優先する。**
2. **TDD** — テストファースト。red → green → refactor。テストのないコードは存在しないのと同じ。
3. **Clean Architecture** — KISS と両立する範囲で依存は内向き。ドメイン核(framer/decode/geometry/monitor 集計)は IO(net/ZMQ/WS)に依存しない。KISS と衝突したら KISS が勝つ。

## このプロジェクト固有の不変条件(PROPOSAL 由来)

- **ch 数をコードに焼き込まない** — mini(256 信号 ch/272 FPN 込み)/ ELITPC(1024/1088)はジオメトリ設定(TPCReco `.dat` 形式)で切替。C++ 版の mini 前提を持ち込まない。
- **FPN リオーダ必須** — GRAW 0–67 ↔ geometry 0–63、FPN={11,22,45,56}(reuse/rust_reference 参照)。波高・波形は**生 ADC(減算なし)**が既定。
- **複数 CoBo 前提** — receiver は CoBo 毎。生 graw は **AsAd 毎ファイル・実機 DataRouter 命名に完全一致**(`CoBo{K}_AsAd{A}_{TS}_{idx:04}.graw`、バイト一致 append。mini = 1、**ELITPC = 1 論理 CoBo × 4 AsAd = 4**(2 枚の zCoBo を 1 CoBo として扱う — 実データ 2026-08-13 確認、SPEC v1.7)。run 番号管理はログブック・ROOT 側 — 2026-08-13 決定、SPEC v1.1)。ビルド後のイベントデータは run 毎に単一 ROOT ファイル(**全 CoBo/AsAd マージが理想形**)。イベントビルダは ELITPC(4 AsAd マージ)で必須。多 CoBo 能力は設計として維持(合成 2-CoBo フィクスチャでテスト)。
- **frameType 1(2018 形式 — 実 pulser データで照合済み 2026-08-18、SPEC v1.23)/ 2(compact rev 5, blkSize256/big-endian — 実機は 2022 時点で既にこれ。SPEC v1.7)両対応**。topology frame(frameType 7)は decoder が防御(カウンタ + INFO)。
- **listen-before-start** — `ecc start` 前に受信ポートを listen。
- **実機プロトコル既知の罠**: DataSender id は `CoBo[0]` 形式・flowType は大文字 `TCP`。Ice encoding は**レッグで違う**: ecc-bridge→ECC = 1.1、**ECC→ハードノード = 1.0**(ECC がプロキシに `-e 1.0` を焼き込む — docs/VIRTUAL_ZCOBO_ja.md §4.2、2026-08-14 確定)。
- **oxyroot でヒストを書かない** — TH1/TH2 型が存在しない(2026-08-12 ソース+実コンパイルで実証)。ヒストの ROOT 化は常に C++ 側(root-sink)。
- **実データ検証** — graw_replay で実 .graw をリプレイ(検出器不要)。実 .graw はローカルのみ(環境変数パスの任意回帰)。**リポには合成フィクスチャのみ**。
- **内部データをリポに入れない**(将来の公開を容易に): 実 .graw、FW、実ジオメトリ .dat、機器マニュアル PDF、コラボ内部情報。GET 由来コード(CeCILL)を持ち込む場合は `third_party/` に隔離しライセンス表示。

## Coding Standards

- Rust: `.unwrap()` 禁止(production)。`Result<T, E>` + `?`。`unsafe` は原則禁止(必要になったら隔離 + 理由記録)。
- コミット前: `cargo fmt && cargo clippy --tests -- -D warnings && cargo test` を通す。
- **Silent failure を作らない** — cache miss / 範囲外値 / プロトコル不一致は `info!` 以上で可視化(delila-rs 2026-05-04 事案の教訓)。
- ホットパスで per-frame の heap 確保・ログ整形・open/close をしない(旧 DataBloc の失敗を繰り返さない)。
- C++ サテライト: C++17。ROOT/Ice は tools/ 内に閉じ込め、Rust 側に漏らさない(境界は ZMQ のみ)。

## プロセス(TODO 運用)

- `TODO/CURRENT.md` — セッション開始時に必ず読む。
- 連番ユニット(`NNN_名前.md`)= 独立にテスト可能な小単位(目安 数百行未満)。テストと共に出荷。
- **詳細チケットは 1 フェーズ先まで** — 次フェーズの詳細は現フェーズが約 8 割できてから起票。
- **完了時(絶対ルール)**: ①ユニット md に **`## 結果` 節としてテスト結果の詳細を記録**
  (実行コマンド / テスト数と green・red / オラクル照合値・カウンタ実測値 / 実行環境と日付 /
  スキップしたテストとその理由)→ ②`**Status: COMPLETED**` に更新 → ③`TODO/archive/` へ移動 →
  ④CURRENT.md 更新。**テスト結果の記録なしに archive へ移動することを禁止**(C++ 版 TODO/done の
  「結果」節の流儀を踏襲)。

## モデル使い分け(コスト運用、2026-08-12 決定)

判断軸: **設計の裁量が残る仕事 = 上位モデル、発注書 + テストで縛れる仕事 = 下位モデル**。
下位モデルの出力は信頼ではなく**テストとオラクルで受け入れる**(この体制は SPEC §12 で整備済み)。

- **Fable**: 仕様改訂・設計判断、チケット(発注書)の起票と精度上げ、フェーズ境界の一括レビュー、
  言語横断・並行性の難所(例: root-sink ロスレス反転)、原因が局所化しないデバッグ、性能調査。
  **機械作業(リネーム・fmt/clippy 修正・フィクスチャ生成)をさせない。**
- **Opus**: 実装の主力。仕様は決まっているが Rust 工学の判断が残るユニット(tokio タスク分離、
  背圧、イベントビルダ等)。対話実装セッションのデフォルト。
- **Sonnet**: 発注書とテストで完全に縛れる実装(パーサ・設定・直列化・パック/アンパック・
  エンコーダ・フィクスチャ)、調査系サブエージェント(Agent の `model` 指定を忘れない)。
- **委譲パターン**: implementer エージェント + `model` 指定で並列実行、Fable はオーケストレーションと
  **完了時の一括レビュー(diff + テスト出力)のみ**。途中で張り付かない。
- **主対話モデルが Opus のセッションでも同じ体制(2026-08-14 決定、運用細目は同日 037)**:
  週制限等で主モデルが Fable でないときも、この使い分けを**そのまま適用**する。主モデル(Opus)が
  Fable の役割(オーケストレーション・発注書の起票と精度上げ・完了時一括レビュー)を代行し、
  実装は従来どおり implementer サブエージェント + `model` 指定(工学判断が残る = Opus、発注書と
  テストで縛れる = Sonnet)で出す。**主コンテキストで実装を抱え込まない**。
  - **Fable キュー(CURRENT.md「Fable 待ち」節)**: Opus セッション中に出た設計判断・SPEC 疑義・
    レビュー依頼はキューに積み、Fable セッション 1 回で**まとめて消化**する(細切れに使わない)。
    Opus は SPEC の diff 案まで作ってよいが、**確定は Fable**。
  - **スポット Fable(Agent `model: "fable"`)**: 入力を自己完結にパッケージできる仕事
    (典型: フェーズ境界レビュー = diff + テスト出力 + 関係 SPEC 節)に限り、Opus 主対話のまま
    Fable サブエージェントへ一発投げしてよい。サブエージェントは会話文脈を持たないので、
    オープンエンドな設計対話には使わない — そちらは Fable セッション(キュー消化)まで保留。
  - **滞空時間の緩和**: Fable が常駐しない間は設計の誤りが長生きしやすい。フェーズを小さく保ち、
    Fable レビューを「完成後」でなく**フェーズ境界ごと**に必ず入れる。
- **エスカレーション規則**: 同一テストで 2 連続失敗 / borrow checker 堂々巡り / 修正が SPEC に
  触れそう → 一段上げる。下位モデルの失敗が続いたらまず**チケットの不備を疑う**。チケット修正は
  主対話(Opus)が一次対応し、それでも割れる / SPEC の解釈に踏み込む場合は **Fable キュー行き**。

## やらないこと

- 制御プレーン(ECC/getHwServer/FW)を改変しない。Ice **クライアント**として話すだけ。
- オンライン 3D/トラック再構成(R5 非スコープ)。TPCReco・オフライン解析チェーンに触らない(grawToEventTPC が我々の .graw を無改造で読めることで接続)。
- TLS・認証の自前実装(SSH トンネル + 必要ならリバースプロキシ)。
