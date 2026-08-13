# 019 — ELITPC 実データ回帰 + ローテーション実機一致修正

**Status: COMPLETED**
**仕様**: SPEC §7(ローテーション)、§12-2(ロスレス分割オラクル)、§13-7(2-CoBo ジオメトリ)
**担当**: Fable 直接(実データから仕様を確定する仕事 — 発注書で縛れない)
**背景**: 2026-08-13、Aogaki が実 DAQ 計算機から ELITPC 実データを取得し
`reference/exp_data/{2022,2026}/` に配置(各 4 ファイル、リポにはコミットしない)。

## 実データ調査で確定した事実(2026-08-13、一時調査テストを Framer+Decoder に通して実測)

- **命名**: `CoBo0_AsAd{0..3}_{TS}_0000.graw` — **coboIdx=0 × asadIdx=0..3**。
  ELITPC は「2 CoBo × 2 AsAd」ではなく**ワイヤ上は 1 論理 CoBo × 4 AsAd**
  (Aogaki の解釈: 2 枚の zCoBo を 1 CoBo として扱っている)。
  実 ELITPC ジオメトリ .dat(全 ch cobo 0)と整合 — **SPEC §13-7 の矛盾が解消**。
- **形式**: 両年とも frameType **2**(compact)・revision 5・big-endian・blkSize 256。
  2022 時点で既に compact — frameType 1 は実機オラクル対象外(合成のみ)と確定。
- **フレーム**: 全フレーム固定 278,784 B(ヘッダ 256 B + 139,264 item × 2 B =
  4 AGET × 68 ch × 512 bucket のフル読み出し)。
- **各ファイル**: 3852 フレーム = 1,073,875,968 B、eventIdx 0..=3851 連続・単調、
  eventTime 単調、malformed=0・unsupported=0(制御フレームなし)・resync=0・残余 0 B。
- **ローテーション実機挙動**: 3852 フレーム目で 2^30 B を**超えてから**次ファイルへ。
  FrameStorage.cpp:190-197 の `write → tellp() > 1024 MiB → createNewFile` と一致。
  **我々の書く前判定(`cur + n > max`)は実機と 1 フレームずれる — 要修正**(SPEC §7 v1.6
  までの記述が誤り)。

## やること

1. `tests/elitpc_real_graw.rs`(env `TPCDAQ_REAL_GRAW_DIR` 未設定なら skip):
   - デコーダオラクル: ディレクトリ内 4 ファイルを Framer+Decoder に通し上記実測値を固定。
   - ローテーションオラクル: AsAd0 ファイル(1 GiB、3852 フレーム)を RunWriter に直接
     流し、既定 max_file_bytes(2^30)で **_0000 が入力と完全バイト一致**(3852 フレーム全部)
     + ローテーション直後の _0001 が空で残ることを照合(修正前は 3851/1 に割れて red)。
2. `src/graw_writer.rs`: ローテーションを実機一致へ — **書いた後に `cur > max`(strict)で
   rotate(次ファイルは即時オープン = createNewFile と同じ、TS 据え置き idx++)**。
   影響する単体テスト 4 本 + 統合テスト (b) を新意味論のオラクルへ書き換え。
3. 文書: SPEC v1.7(§7 修正、§12-2 に ELITPC オラクル行、§13-7 解消)、WARSAW_PLAN
   §4/§6 更新、P2 レビュー R2 クローズ、CLAUDE.md のトポロジ記述修正、CURRENT.md。

## 受け入れ

- `cargo fmt && cargo clippy --tests -- -D warnings && cargo test` green(env なし)。
- `TPCDAQ_REAL_GRAW_DIR=reference/exp_data/2022`(と 2026)で新テスト green。
- 既存 mini 回帰(`TPCDAQ_REAL_GRAW=<mini ファイル>`)が引き続き green(30 MB < 1 GiB で
  ローテーションなし — 意味論変更の影響を受けないことの確認)。

## 結果

実行環境: macOS(Darwin 25.5.0)、rustc stable、2026-08-13。担当: Fable 直接。

### 実行コマンドと結果

- `TPCDAQ_REAL_GRAW_DIR=reference/exp_data/2022 cargo test --release --test elitpc_real_graw -- --nocapture`
  → **2 passed / 0 failed**(4 ファイル全て frames=3852 / items=536,444,928)
- `TPCDAQ_REAL_GRAW_DIR=reference/exp_data/2026 cargo test --release --test elitpc_real_graw -- --nocapture`
  → **2 passed / 0 failed**(同上)
- TDD 照合: ローテーションオラクルテストは**修正前に red**(_0000 = 3851 フレーム、
  実機オラクル 3852 と 1 フレームずれ)→ write_to を書き込み後判定へ修正 → green。
- mini 回帰(意味論変更の無影響確認、`TPCDAQ_REAL_GRAW=$HOME/TPC/CoBo_2025-09-01T08_51_06.203_0000.graw`):
  - `decoder_real_graw`: events=108 / items=15,040,512 / malformed=0 / unsupported=1 → **green**
  - `graw_writer_real_graw`: AsAd 30,108,672 B + ctrl 12 B = 30,108,684 B 完全ロスレス分割 → **green**
- ゲート: `cargo fmt && cargo clippy --tests -- -D warnings && cargo test`(env なし)→ **全 green**
  (elitpc_real_graw は env 未設定で 2 skip = 仕様どおり)。

### オラクル実測値(両年同一、SPEC §12-1/2 v1.7 に固定)

- 各ファイル: 3852 フレーム × 278,784 B = 1,073,875,968 B、items=536,444,928、
  malformed=0 / unsupported=0 / resync=0 / 残余 0 B、eventIdx 0..=3851 連続、eventTime 単調
- ローテーション境界: _0000 が 2^30 B を 134,144 B(= 1 フレーム弱)超過 —
  FrameStorage.cpp:190-197 の書き込み後判定(strict `>`)と一致

### 変更ファイル

- `tests/elitpc_real_graw.rs`(新規、env-gated 2 テスト)
- `src/graw_writer.rs`(write_to 書き込み後判定 + rotate doc + 単体テスト 4 本書き換え +
  strict `>` 境界テスト 1 本追加)
- `tests/graw_writer_integration.rs`((b) ローテーション E2E を新意味論オラクルへ)
- 文書: SPEC v1.6→v1.7(§7/§12-1,2/§13-7/§14-5/変更履歴)、WARSAW_PLAN §4/§6、
  P2 レビュー R2 クローズ、CLAUDE.md(トポロジ・frameType 記述訂正)

### スキップ・未実施

- root-sink 側(TTree)の ELITPC 実データ照合は未実施 — 実機側の変換済み .root が無く
  オラクル不在のため(mini の §12-3 照合は P2 で完了済み。ELITPC の TTree 照合は
  Warsaw で変換済み .root を入手できたら追加)。

### 追記(2026-08-13、完了後)

ユーザーが `reference/exp_data/2026/` に実機オフライン変換出力
`PEventTPC_2026-08-11T07-47-37.051_0000.root`(11 GB、graw とは**別 run**)を追加。実見結果:

- 変換器 = TPCReco **grawToEventTPC**(ユーザー確認 — 「graw2root」は GET 付属の別ツールで混同していた)。`TPCData` ツリー / `PEventTPC` クラス、entries=**3852**(_0000 4 本組は
  フル読み出し固定フレームゆえ常に 3852 イベント — 別 run でも一致する普遍値)
- **圧縮 = ZLIB level 1**(SPEC v1.5 の既定 101 ZLIB-1 と一致を実機出力で直接確認)
- **書き手 = ROOT 6.08/06**(large-file フラグ付き)— 先方実運用 ROOT バージョン確定
- クラスが GDataFrame ではないため「未実施」とした TTree サンプル照合のオラクルには
  ならない(そもそも先方ワークフローに GDataFrame .root は存在しない可能性が高い)。
  形式・圧縮・バージョンの参照として WARSAW_PLAN §4 に記録。
