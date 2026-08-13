# 007 — graw-writer コンポーネント(AsAd 毎ファイル、バイト一致)

**Status: COMPLETED**
**改訂**: 2026-08-13 ファイル分割単位を CoBo 毎 → **AsAd 毎**、命名を**実機 DataRouter 形式に
完全一致**へ訂正(ユーザー指示 — オフライン bash 資産の無改造流用のため → SPEC v1.1 §6.5/§7)
**仕様**: SPEC §7(全部)、§6.5(出力配置)、§1.3/§1.4(状態機械・過負荷)、§2.2/§2.3(Batch/EOS)
**依存**: 003(msg/command/zmq_helper)、004(decode — asadIdx ヘッダ読み出しの流用)、
006(receiver — E2E で使用)、005(graw_replay — E2E で使用)

## やること

1. `src/graw_writer.rs` — graw-writer 本体:
   - **PULL bind**(既定 `tcp://*:47001`、zmq_helper の有限 HWM)。受信スレッド(専用 OS スレッド、
     006 送信側と同じ理屈で同期 zmq)→ 書き込み処理。
   - RawFrames バッチ内の各フレームを **(cobo, asad) 毎のファイル**へバイトそのまま append。
     cobo = Batch.source_id、asad = フレームヘッダの asadIdx。読み出しは
     **`pub fn peek_asad(frame: &[u8]) -> Option<u8>` を `src/decode.rs` に新設**して使う
     (ヘッダオフセットの知識を decode に集約 — graw_writer に `frame[27]` を焼き込まない。
     共通 MFM ヘッダなので frameType 1/2 共通。短小フレームは None。単体テスト付き、
     decode の既存コード・テストは無改変)。リシリアライズ禁止(AsAd 毎連結 =
     入力を asadIdx で分別した列と同一)。peek_asad = None の短小フレーム = malformed → Error
     状態 + カウント(silent 禁止)。
   - 出力: `<output_root>/run{run:04}/CoBo{K}_AsAd{A}_{TS}_{idx:04}.graw` — **実機 DataRouter
     命名に完全一致**(SPEC §6.5/§7。graw 名に run 番号は入れない — 対応は run ディレクトリと
     metrics/ログが持つ)。TS = 当該 (cobo, asad) の最初のフレーム到着時刻。**localtime** の
     ISO 8601 拡張 + ミリ秒 3 桁: **`chrono` を依存に追加**し
     `Local::now().format("%Y-%m-%dT%H:%M:%S%.3f")`(例 `2022-04-12T08:03:44.531`)。
     K/A はゼロ埋めなし 10 進、idx は 4 桁ゼロ埋め。
     run ディレクトリは当該 run の最初のフレーム到着時、ファイルは **(cobo, asad) 毎の最初の
     フレーム到着時に遅延作成**(run_number は Batch に載っている。**AsAd 数は設定にもコードにも
     焼き込まない** — mini = 1、ELITPC = 2 CoBo × 2 AsAd = 4 が観測から自然に出る)。
     ハンドルは run 中開きっぱなし(per-frame open/close 禁止)。
   - ローテーション: **(cobo, asad) 毎に独立の idx。TS は据え置き、idx++ のみ**(実機 FrameStorage
     `createNewFile(newTimeStamp=false)` と同一挙動。新 run で新 TS + idx=0000)。
     `cur + n > max_file_bytes(既定 1 GiB)` で次 idx へ。**フレームはファイル間で分割しない**。
     単発の巨大フレームはそのまま書く(`cur_bytes > 0` ガード — C++ 版と同じ)。
   - flush 1 秒毎、fsync はローテーションと close 時(ホットパスで fsync しない)。
   - **ロスレス検証**: ソース毎 sequence_number 連続性チェック。ギャップ = Error 状態 + カウント
     (silent 禁止)。EOS 前に run_number が変わるのもプロトコル違反 = Error。
   - **EOS**: 期待ソース集合(設定の `[[cobo]]` 全 id)から全 EOS 受領 → 当該 run の全ファイルを
     flush + fsync + close。metrics にファイル実績(パス・バイト数)を出す(将来の run_stop 記録の材料)。
   - 書き込み失敗(ディスクフル等)= Error 状態 + write_errors カウント + **PULL 消費停止**
     (HWM が詰まり上流へ背圧 → receiver 側 overflow が可視化される、SPEC §1.4 のカスケード)。
   - 状態機械: Configure(設定確定)→ Arm(PULL bind)→ Start{run}(消費開始)→ Stop / Reset。
     REP は 003 `run_command_task`。カウンタ: bytes/frames((cobo, asad) 毎)、batches(cobo 毎)、
     seq_gaps、write_errors、malformed、files(パス + サイズ)。
2. `src/bin/graw_writer.rs` — `graw_writer --config <toml>`(tracing 初期化 + 起動)。
3. config: `[graw_writer]` セクション(`pull_bind` 既定 47001 / `max_file_bytes` 既定 1 GiB /
   `flush_interval_ms` 既定 1000)を `src/config.rs` に追加(既存フィールド・テストは無改変)。

## テスト

- 単体: ファイル命名(**実機形式の regex 照合**:
  `^CoBo\d+_AsAd\d+_\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}_\d{4}\.graw$`)、
  **ローテーションで TS 不変・idx++**、**asadIdx 振り分け(合成 2 AsAd 混在バッチ)**、
  ローテーション境界(フレーム非分割・連結一致)、巨大フレーム、遅延作成、
  **短小フレーム = malformed → Error**。
- 統合(port 0、PUSH で直接 Batch 投入):
  (a) 2 CoBo × 2 AsAd 分のバッチ → (cobo, asad) 毎の 4 ファイルがそれぞれ
      **入力を asadIdx で分別した列とバイト一致**
  (b) ローテーション跨ぎでも連結一致
  (c) 全 EOS でファイル close + metrics にファイル実績
  (d) seq ギャップ → Error / EOS 前の run 変更 → Error
  (e) Configure→Arm→Start→Stop の全シーケンス
- **E2E(env `TPCDAQ_REAL_GRAW` 時)**: graw_replay(全速)→ receiver(006)→ graw-writer で、
  出力ファイル連結が**元 .graw とバイト完全一致**(mini 実 graw は 1 AsAd なので実質単一ファイル
  = SPEC §12-2 の受け入れそのもの)。
  実測値(バイト数・ファイル数・所要)を `## 結果` に記録。

## 受け入れ

- 上記全テスト green。E2E バイト完全一致。`cargo fmt && cargo clippy --all-targets -- -D warnings
  && cargo test` 通過。

## 結果

**実行環境**: macOS 26.5.2 / arm64(Apple Silicon)、rustc 1.97.1、cargo 1.97.1、2026-08-13。

**実行コマンド**

```
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
TPCDAQ_REAL_GRAW=/Users/aogaki/TPC/CoBo_2025-09-01T08_51_06.203_0000.graw \
  cargo test --test graw_writer_real_graw -- --nocapture
```

**テスト数**: リポ全体 **182 passed / 0 failed**(007 で新規 25)。`cargo fmt --check` 差分なし、
`cargo clippy --all-targets -- -D warnings` クリーン。

新規テスト:

| ファイル | テスト | 対応 |
|---|---|---|
| `src/decode.rs` | `peek_asad_reads_offset_27_without_full_decode` / `peek_asad_is_frame_type_agnostic` / `peek_asad_matches_the_header_field_across_builders` / `peek_asad_is_none_for_short_frames` / `peek_asad_reads_the_minimal_28_byte_boundary`(5) | `peek_asad` 新設(既存コード・テストは無改変) |
| `src/config.rs` | `graw_writer_section_defaults_when_omitted` / `graw_writer_section_values_override_the_defaults` / `graw_writer_unknown_field_is_err` / `graw_writer_command_listen_matches_spec_3_2`(4) | `[graw_writer]` 追加(既存フィールド・テストは無改変) |
| `src/graw_writer.rs`(`RunWriter` 単体、ZMQ なし・実ファイル IO あり) | `file_naming_matches_the_real_datarouter_format` / `rotation_keeps_the_same_ts_and_increments_idx_only` / `frames_are_routed_by_asad_index_within_a_mixed_batch` / `rotation_boundary_frames_are_not_split_and_concatenation_matches` / `a_single_frame_larger_than_the_rotation_limit_is_written_whole` / `files_are_created_lazily_on_first_frame_of_each_asad` / `a_short_frame_that_cannot_carry_asad_is_malformed_and_latches_error` / `finalize_closes_every_open_file_and_keeps_the_file_report`(8) | 単体テスト全項目(命名 regex 相当・ローテーション TS 不変・asad 振り分け・境界・巨大フレーム・遅延作成・malformed) |
| `src/bin/graw_writer.rs` | `parses_the_config_path` / `rejects_missing_or_malformed_arguments`(2) | CLI |
| `tests/graw_writer_integration.rs`(port 0、PUSH で直接 Batch 投入) | `two_cobo_two_asad_batches_produce_four_byte_identical_files_and_close_on_eos` | (a)+(c) |
| | `rotation_across_a_boundary_still_concatenates_to_the_input` | (b) |
| | `sequence_gap_latches_error_and_counts_it` / `run_number_change_before_eos_latches_error_and_counts_it` | (d) |
| | `configure_arm_start_stop_sequence_succeeds`(計 5) | (e) |
| `tests/graw_writer_real_graw.rs` | `real_graw_replay_through_receiver_matches_the_source_file_byte_for_byte`(1) | E2E |

**実 .graw E2E 実測**(`CoBo_2025-09-01T08_51_06.203_0000.graw` = 30,108,684 B、graw_replay 全速 →
receiver(006、既定パラメタ)→ graw-writer、3 回連続実行)

| 項目 | 実測 | オラクル |
|---|---|---|
| 出力ファイル数 | 1(mini = 1 AsAd) | 1 ✔ |
| 出力バイト数 | 30,108,672 B | 後述(asadIdx 分別列と完全一致) ✔ |
| バイト一致 | graw-writer 出力 == 入力を asadIdx で分別した列(peek_asad 通過フレームのみ連結) | 完全一致 ✔(SPEC §12-2 一般規則) |
| malformed | 1(= `decoder_real_graw.rs` オラクルの `unsupported=1` と同一の 12 B 制御フレーム) | 除外数 1 と一致 ✔ |
| seq_gaps / write_errors | 0 / 0 | 0 / 0 ✔ |
| 所要 | 0.151–0.194 s(3 回)。graw_replay 全速送出 → receiver → graw-writer 全経路 | 100 Hz 相当(mini ≈28 MB/s)を大きく超える速度で drop 0 |

**逸脱・報告事項(SPEC §12-2 括弧書きとの不一致 — 実装は変更していない)**

SPEC §12-2 は一般規則として「リプレイ入力を asadIdx で分別した列と出力ファイルの完全一致」を定め、
括弧書きで「mini 実 graw は 1 AsAd なので元ファイルとの完全一致」と続ける。実際にこの実 .graw で
E2E を回すと、**元ファイルの 109 フレーム中 1 本(12 B)が asadIdx を持たない制御フレーム**
(`decoder_real_graw.rs` オラクルの `unsupported=1` と同一物 — frameType が 1/2 以外で decode 上は
"unsupported"、28 B に満たず offset 27 も読めないので graw-writer の `peek_asad` 上は "malformed"
になる)であるため、**発注書 §7 の malformed 規則(「短小フレームは None → malformed → Error」)を
そのまま実装すると、この 1 フレームは正当にどの (cobo, asad) にも書かれず除外され、出力は
元ファイルと完全一致しない**(30,108,672 B ≠ 30,108,684 B、差 12 B)。

これは実装上の未規定分岐ではなく、発注書に明記された malformed 規則を実データに適用した結果として
必然的に生じる。実装は発注書の一般規則(asadIdx で分別した列との一致)どおりに動作しており、
E2E テストもその一般規則で照合している(`tests/graw_writer_real_graw.rs` の
`asad_separated_column` — 元ファイルを framer で切り、`peek_asad` が `Some` を返すフレームだけを
連結したものをオラクルとして使用)。**括弧書き「mini なら元ファイルと完全一致」は、この実ファイルの
ような非 AsAd 制御フレームが 1 本も無いことを暗黙に仮定した記述であり、この実測値とは厳密には
食い違う** — SPEC 文言の修正要否(括弧書きを一般規則の言い換えに留めるか、注記を足すか)は
判断が必要なため報告する。malformed カウンタ自体は「silent にしない」という CLAUDE.md/SPEC の
要請どおり可視化されており、機能上の欠陥ではない。

**その他の逸脱・裁量点**

1. **graw-writer コマンド REP の bind 先**: 発注書・SPEC §3.2 とも具体的なポート番号を明記して
   いない(「47100 + 連番(receiver k = 47110+k)」という式のみ)。receiver 群が `47110+k` を
   予約している以上、単一コンポーネントである graw-writer は連番の先頭 `47100` を使うと解釈し
   `GRAW_WRITER_COMMAND_LISTEN = "tcp://*:47100"` を `src/config.rs` に定数追加した(receiver の
   `RECEIVER_COMMAND_PORT_BASE` と同じ流儀)。他コンポーネント(decoder 等)のポート採番と衝突しない
   限りにおいて技術的な影響はないが、正式な採番表が別途あるなら差し替えが必要。
2. **metrics に `files_open`/`files_closed`/各 `files[].closed` を追加**(発注書は「files(パス+
   サイズ)」とだけ指定)。統合テストで「(c) 全 EOS でファイル close」を検証する際、`files` 配列は
   まだ flush されていない open 中のファイルも含む設計(GetStatus で run 中の進捗を見せるため)
   なので、「close 済みかどうか」を外部から判別する手段が無いと E2E/統合テストが
   open 状態(BufWriter 内バッファのみ、ディスク未反映)を close 済みと誤認するレースが生じた
   (実際に統合テストで確認・修正済み — 下記参照)。SPEC に明記の無い追加だが、「ファイル実績」
   という要求を正しく満たすための最小限の可観測性強化と判断し実装した。
3. **`RunWriter`(ZMQ 非依存の書き込みコア)を新設**し、単体テストはこれを直接叩く(ZMQ・スレッド
   なし)。発注書は「単体」テストの中身(命名・ローテーション・振り分け等)を明記しているのみで
   構造は指定していないため、decode/framer と同じ Clean Architecture の切り方(CLAUDE.md 設計原則
   3)を踏襲した。
4. **peek_asad の実装**は `frame.get(27).copied()` のみ(frameType 判定なし)。発注書「共通 MFM
   ヘッダなので frameType 1/2 共通。短小フレームは None」の記述どおり、offset 27 が読めるかどうか
   だけを見る(decode.rs の既存 `decode()` は別途 frameType を検証しているが、graw-writer はそこまで
   要求されていない — リシリアライズ禁止の趣旨に沿う最小実装)。

**フレーク調査メモ(実装中に発見・修正済み、報告として残す)**

`tests/graw_writer_integration.rs` の (a) テストで、`GetStatus` の `files` 配列長だけを完了条件に
すると約 1〜3 割の頻度で失敗した(ファイルは作られ metrics のバイト数も正しいのに
`std::fs::read` が空を返す)。原因は BufWriter の flush タイミング(発注書どおり「flush は
ローテーションと close 時のみ」)で、「files に載っている = 書き込み済みディスク反映済み」とは
限らないことだった(flush 前でも `bytes` カウンタは正しく増える)。**バグではなく発注書どおりの
挙動**だが、テストの完了条件としては不十分だったため、上記の `files_open`/`files_closed` を足して
「flush+fsync+close が完了した」ことを外部から確実に観測できるようにし、統合テスト側もそちらを
待つよう修正した(単体テストはこの問題を踏まない — 同一プロセス内で `flush_all()`/`finalize()` を
直接呼んで検証している)。修正後、統合テストを 45 回連続実行して 0/45 失敗を確認。

**スキップしたテスト**: なし(`TPCDAQ_REAL_GRAW` が実行環境にローカル実 .graw を指しており E2E も
実行・記録済み)。

## レビュー指摘(2026-08-13 Fable → v1.2 対応の修正発注)

逸脱報告 1(SPEC §12-2 不一致)は**実装の勝ち** — 発注書側の穴だった。当該 12 B は実 2025 run
先頭の **frameType 7 制御フレーム**(実機 FrameStorage は警告して捨てる種類)。ユーザー決定により
SPEC v1.2 で「**ctrl/ サブディレクトリへ保全**」と規定した(§6.5・§7・§12-2 参照)。
逸脱 2(REP 47100)は SPEC §3.2 v1.2 で正式採番として固定 = 受理。逸脱 3(files_open/closed)・
4(RunWriter 分離)・peek_asad 最小実装の報告 = 受理(ただし peek_asad は下記 1 で契約変更)。

**修正項目(SPEC v1.2 に一致させる)**:

1. `decode::peek_asad` を frameType-aware に: **frameType 1/2 かつヘッダ 28 B 以上のときだけ
   Some(asad)**、それ以外は None(28 B 超の非 1/2 制御フレームがオフセット 27 の任意バイトで
   誤った AsAd ファイルに混入するのを塞ぐ)。既存 5 テストを契約に合わせ更新 +
   「frameType ≠ 1/2 の 28 B 超フレーム → None」テストを追加。
2. graw-writer: peek_asad = None → **Error にせず** `run{run:04}/ctrl/CoBo{K}_{TS}_{idx:04}.graw`
   へバイトそのまま保全 + `ctrl_frames` カウンタ + info ログ。malformed カウンタと content 起因の
   Error ラッチは廃止(Error = write 失敗・seq ギャップ・EOS 前 run 変更のみ)。ctrl ファイルの
   TS・idx・ローテーション・遅延作成・EOS finalize・metrics files 掲載は per-AsAd ファイルと同一規則。
3. テスト更新: 単体「短小フレーム = Error」→「非 AsAd フレームは ctrl/ に保全され Error に
   ならない(内容バイト一致まで検証)」。統合 (a) に非 AsAd フレームを 1 本混ぜて ctrl/ 保全を検証。
   E2E: AsAd ファイル 30,108,672 B + ctrl/ 12 B = **合計 30,108,684 B が入力と完全ロスレス分割**、
   `ctrl_frames=1` をオラクル値として照合。
4. ゲート再実行(fmt/clippy/test + E2E)→ 本節の下に「### v1.2 対応結果」を追記 → Status を
   IMPLEMENTED(レビュー待ち)へ戻す。

### v1.2 対応結果

**実行環境**: macOS 26.5.2 / arm64(Apple Silicon)、rustc 1.97.1、cargo 1.97.1、2026-08-13。

**実行コマンド**

```
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
TPCDAQ_REAL_GRAW=/Users/aogaki/TPC/CoBo_2025-09-01T08_51_06.203_0000.graw \
  cargo test --test graw_writer_real_graw -- --nocapture
```

**テスト数**: リポ全体 **185 passed / 0 failed**(v1.2 対応で純増 +3: decode.rs +1、
graw_writer.rs 単体 +2〔malformed 系 1 本を ctrl 系 3 本に置換〕)。`cargo fmt --check` 差分なし、
`cargo clippy --all-targets -- -D warnings` クリーン。

**実装した 4 項目(発注どおり)**:

1. `decode::peek_asad` を frameType-aware に変更(`src/decode.rs`)。**28 B 以上 かつ
   frameType 1/2 のときだけ `Some(asad)`**、それ以外は `None`。既存 5 テストを契約に合わせ更新
   (`peek_asad_reads_the_minimal_28_byte_boundary` は frameType=1 を明示するよう修正、
   `peek_asad_is_frame_type_agnostic` は `peek_asad_supports_frame_type_2_as_well_as_1` に改名)+
   新規 `peek_asad_is_none_for_non_cobo_frame_types_even_when_long_enough`(88 B の frameType=7
   フレームで、offset 27 自体は読めるのに `None` になることを検証)。
2. `src/graw_writer.rs`: 内部を `FileKey::{Asad(cobo,asad), Ctrl(cobo)}` で統一し、per-AsAd/ctrl
   の書き込み・ローテーション・遅延作成・finalize を同一コードパスで扱うよう再設計。
   `peek_asad = None` のフレームは `ctrl_frames` カウンタ + `info!` ログのうえ
   `run{run:04}/ctrl/CoBo{K}_{TS}_{idx:04}.graw` へバイトそのまま保全し、**Error にはしない**
   (`malformed` フィールド・その Error ラッチは削除)。Error は write 失敗・seq ギャップ・
   EOS 前の run 変更のみに限定。`FileReport.asad` は `Option<u8>`(ctrl は `None`)に変更、
   `metrics_json()` に `ctrl_frames` を追加(`malformed` キーは削除)。
3. テスト更新: 単体は「短小フレーム = malformed → Error」を削除し、
   `a_non_asad_frame_is_preserved_under_ctrl_and_does_not_latch_error`(短小 12 B、内容バイト一致・
   `ctrl/` 配置・`Error` にならないことまで検証)、
   `a_long_non_cobo_frame_is_routed_to_ctrl_not_misinterpreted_as_asad_data`(88 B・frameType=7 —
   frameType-aware 化の核心を直接検証)、
   `ctrl_files_rotate_with_ts_unchanged_and_idx_incrementing_like_asad_files` を追加。
   統合 (a) は CoBo0 のバッチ先頭に 12 B の非 AsAd フレームを混ぜ、per-AsAd 4 ファイル +
   ctrl 1 ファイル(計 5、`files_open==0 && files_closed==5` で待機)のバイト一致・
   `ctrl_frames==1`・`Error` にならないことを検証。E2E は下記。
4. ゲート(fmt/clippy/test)+ E2E を再実行、本節を追記、Status を IMPLEMENTED(レビュー待ち)へ。

**実 .graw E2E 実測(v1.2、3 回連続実行)**

| 項目 | 実測 | オラクル |
|---|---|---|
| 出力ファイル数 | 2(per-AsAd 1 + ctrl 1) | 2 ✔(SPEC §7/§12-2 v1.2) |
| AsAd 出力バイト数 | 30,108,672 B | 30,108,672 B ✔ |
| ctrl 出力バイト数 | 12 B | 12 B ✔(frameType 7 ×1) |
| **AsAd + ctrl 合計** | **30,108,684 B** | **元 .graw と完全一致(30,108,684 B)** ✔ |
| AsAd フレーム数 | 108 | 108 ✔ |
| `ctrl_frames` メトリクス | 1 | 1 ✔ |
| seq_gaps / write_errors / run_mismatches | 0 / 0 / 0 | 0 / 0 / 0 ✔ |
| 状態 | `Running`(Error に遷移せず) | Error にならない ✔(SPEC §7 v1.2) |
| 所要 | 0.155–0.201 s(3 回) | 100 Hz 相当を大きく超える速度で drop 0 |

v1.1 実装時の逸脱報告(SPEC §12-2 括弧書きとの不一致)は本対応で解消: AsAd 出力 + ctrl 出力の
**合計**が元 .graw と完全ロスレス分割で一致することを実測で確認した(上記「逸脱・報告事項」節は
v1.1 時点の記録として残す — 経緯の参照用)。

**フレーク再確認**: `graw_writer_integration` を(a)含め 15 回連続実行して 0/15 失敗
(単一スレッド実行、files_open/files_closed による完了待ちは v1.2 でも有効)。E2E は 3 回連続で
同一オラクル値を再現。

**スキップしたテスト**: なし(`TPCDAQ_REAL_GRAW` が実行環境にローカル実 .graw を指しており
E2E も実行・記録済み)。

### 最終レビュー(2026-08-13 Fable)

- **判定: 受理(COMPLETED)**。v1.1 実装の逸脱報告 1 は発注書の穴を暴いた正当な発見
  (→ SPEC v1.2 の ctrl/ 保全規定へ)。逸脱 2(REP 47100)は SPEC §3.2 で正式採番、
  逸脱 3(files_open/closed)・4(RunWriter 分離)も受理。
- レビュー側で独立再検証済み: fmt/clippy クリーン、**cargo test 185 passed / 0 failed**、
  実 .graw E2E で **AsAd 30,108,672 B + ctrl 12 B = 30,108,684 B = 入力と完全ロスレス分割**
  (ctrl_frames=1、seq_gaps=0、write_errors=0、Error 遷移なし)を実測一致。
  `peek_asad` はエンディアン判定込みの frameType 検査を確認(SPEC v1.2 §7 準拠)。
