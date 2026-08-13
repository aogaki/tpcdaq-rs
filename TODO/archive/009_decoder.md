# 009 — decoder コンポーネント(Rust、Fragments 単一ストリーム + EOS 集約)

**Status: COMPLETED**
**仕様**: SPEC v1.2 §2.3(decoder のソース性 = 本ユニットの核)、§2.2(Batch/EOS/Heartbeat)、
§2.4(Fragment)、§1.3/§1.4(状態機械・過負荷)、§3.1/§3.2([decoder]、PULL 47002・REP 47101)
**依存**: 003(msg/command/zmq_helper)、004(decode 純コア — 変換はすべて既存 `Decoder` を使う)、
005/006(E2E で使用)
**発注先想定**: implementer/Opus(tokio/スレッド分離・EOS 集約・停止設計の Rust 工学判断が残る)

## 責務(SPEC §2.3 の要約 — 迷ったら SPEC が正)

receiver(CoBo 毎)から RawFrames を受け、decode して **Fragments の単一ストリーム**
(`source_id = 100`、自前 sequence_number)として root-sink へ送る。
**上流全 CoBo の EOS 受領 + 対応 Fragment の送出完了後に、自分の EOS を 1 本だけ送る。**
CoBo の識別は Fragment.cobo が運ぶ(下流は上流の数を知らなくてよい)。

## やること

1. `src/decoder.rs` — 本体:
   - **PULL bind**(既定 `tcp://*:47002`、zmq_helper の有限 HWM)← receiver 群が PUSH connect。
   - **PUSH connect**(既定 `tcp://127.0.0.1:47003`、有限 SNDHWM)→ root-sink。
   - 受信・デコード・送出は**単一の専用 OS スレッド**(同期 zmq、006/007 と同じ流儀)。
     `[decoder] workers` は設定として受理するが本ユニットでは未使用(>1 指定時に info ログ
     「P5 実測まで単線」— SPEC §13 の「実測で決める」流儀。並列化は将来ユニット)。
   - 入力バッチ処理:
     - ソース(cobo)毎 sequence_number 連続性検証。ギャップ = **Error 状態ラッチ + カウント**
       (007 と同じ扱い: ラッチ後も消費・送出は継続)。EOS 前の run_number 変化も同様。
     - 各フレームを `decode::Decoder` へ。**Fragment 化されるのは frameType 1/2 のみ**。
       unsupported(frameType ∉ {1,2}、例: 実 run 先頭の frameType 7)= カウント + info ログ、
       **Error にしない**(graw-writer が ctrl/ に保全済み — SPEC v1.2 §7。decoder は数えて跳ばす)。
       malformed(frameType 1/2 だが構造破損)= カウント + warn + Error ラッチ(P1 オラクルは 0)。
   - 出力バッチ詰め: **8 MiB 到達 or 10 ms 経過**の早い方で close(SPEC §2.3)。
     source_id=100、run_number は入力 Batch から、sequence_number は run 開始 0 から自前単調増加、
     created_ns 付与。ホットパスで per-frame の heap 確保・ZMQ send をしない
     (Fragment 蓄積バッファは再利用)。
   - **EOS 集約**: 期待ソース集合 = 設定の `[[cobo]]` 全 id。全 EOS 受領 → 残バッチを flush →
     **自分の EOS{100, run} を 1 本**送出 → run 状態リセット(seq 検証・自前 seq とも 0 へ)。
     idle 時は Heartbeat{100, run, counter} を 1 Hz で送出(SPEC §2.2)。上流 Heartbeat は
     カウントのみ(転送しない)。
   - **停止設計(006 レビュー積み残しへの本ユニットの答え)**: 通常運転の送出はブロッキング
     (ロスレス — 下流の背圧で待つ)。ただし **Reset コマンド処理中に限り**送出待ちを打ち切れる:
     PUSH の sndtimeo を有限(既定 1000 ms)にし、Reset 中のタイムアウトは
     `eos_abandoned` / `batches_abandoned` としてカウント + warn(Reset はオペレータの明示
     オーバーライド = 破棄が可視化されていれば許される唯一の経路)。Stop は EOS 送出完了まで待つ。
   - 状態機械: Configure → Arm(PULL bind)→ Start{run} → Stop / Reset。REP = 003
     `run_command_task`、**bind は `tcp://*:47101`**(SPEC §3.2 v1.2 で固定済み)。
   - カウンタ(GetStatus): batches_in(cobo 毎)/ frames_in / fragments_out / batches_out /
     items_out / unsupported / malformed / seq_gaps / run_mismatches / eos_in / heartbeats_in /
     eos_abandoned / batches_abandoned。
2. `src/bin/decoder.rs` — `decoder --config <toml>`(tracing 初期化 + 起動)。
3. config: `[decoder]` に `pull_bind`(既定 47002)/ `push_connect`(既定 127.0.0.1:47003)/
   `batch_max_bytes`(既定 8 MiB)/ `batch_max_ms`(既定 10)を追加。既存の `workers`
   フィールドが 001 実装に既にあるなら残す・無ければ追加(既定 1)。既存フィールド・テスト無改変。

## テスト

- 単体(純コア、ZMQ なし): EOS 集約(2 ソースの片方だけでは出ない・全部で 1 本だけ出る・
  再 run で再武装)、バッチ close 条件(サイズ / 時間)、seq 検証、run 変化検出。
- 統合(port 0、PUSH で RawFrames 直接投入 + PULL で Fragments 受け):
  (a) 2 ソース混在 → Fragments が source_id=100・seq 0..N 連続・Fragment.cobo で識別可能
  (b) 全ソース EOS → 自分の EOS がちょうど 1 本、全 Fragment の後に届く
  (c) 片ソース seq ギャップ → Error ラッチ + カウント、消費は継続
  (d) unsupported フレーム(frameType 7 の 12 B)混在 → カウントのみ・Error にならず・
      Fragment は出ない
  (e) Configure→Arm→Start→Stop 全シーケンス + idle Heartbeat が届く
  (f) 下流を意図的に詰まらせて(受け側 PULL を作らない/小 HWM)Reset →
      eos_abandoned/batches_abandoned がカウントされ、プロセスは畳める
- **E2E(env `TPCDAQ_REAL_GRAW` 時)**: graw_replay(全速)→ receiver(006)→ decoder →
  テスト側 PULL で全 Fragments を受け、**events(distinct event_idx)=108 /
  items 合計 = 15,040,512 / malformed = 0 / unsupported = 1**(P1 オラクル)を照合。
  実測値を `## 結果` に記録。

## 受け入れ

- 上記全テスト green。E2E オラクル一致。`cargo fmt && cargo clippy --all-targets -- -D warnings
  && cargo test` 通過。既存テスト(185)無影響。
- ファイル所有権: src/decoder.rs・src/bin/decoder.rs・src/config.rs([decoder] 追記のみ)・
  src/lib.rs(1 行)・tests/decoder_integration.rs・tests/decoder_pipeline_real_graw.rs。
  **これ以外に触らない**(並列で 010 が tools/root_sink/ を作業中)。

## 結果

**実行環境**: macOS 26.5.2 / arm64(Apple Silicon)、rustc 1.97.1、cargo 1.97.1、2026-08-13。

**実行コマンド**

```
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
TPCDAQ_REAL_GRAW=/Users/aogaki/TPC/CoBo_2025-09-01T08_51_06.203_0000.graw \
  cargo test --test decoder_pipeline_real_graw -- --nocapture
TPCDAQ_REAL_GRAW=... cargo test --release --test decoder_pipeline_real_graw -- --nocapture  # 速度実測のみ
```

**テスト数**: リポ全体 **215 passed / 0 failed**(009 で新規 27)。`cargo fmt` 差分なし、
`cargo clippy --all-targets -- -D warnings` クリーン。009 着手時点の他ユニット
(007 完了時 182 + 並行作業中の 008 root-sink intake 分)には影響なし — 変更したのは
`src/config.rs`(`[decoder]` 追記のみ)と `src/lib.rs`(mod 1 行)だけで、既存フィールド・
既存テストは無改変。

新規テスト:

| ファイル | テスト | 対応 |
|---|---|---|
| `src/config.rs` | `decoder_section_defaults_when_the_new_keys_are_omitted` / `decoder_section_values_override_the_defaults` / `decoder_unknown_field_is_err` / `decoder_endpoints_match_spec_3_2`(4) | `[decoder]` に pull_bind / push_connect / batch_max_bytes / batch_max_ms 追加(`workers` は既存のまま必須) |
| `src/decoder.rs`(`RunDecoder` 単体、ZMQ なし・IO なし) | `eos_is_emitted_only_after_every_expected_source_has_reported` / `a_repeated_eos_does_not_emit_a_second_one` / `the_run_state_rearms_after_the_eos_so_the_next_run_starts_from_zero`(3) | EOS 集約(片方だけでは出ない・1 本だけ・再 run で再武装) |
| | `a_batch_closes_when_the_pending_items_reach_the_size_limit` / `a_batch_closes_when_the_time_limit_elapses` / `an_empty_buffer_never_closes_on_a_tick`(3) | バッチ close 条件(サイズ / 時間) |
| | `a_sequence_gap_is_counted_latches_error_and_keeps_consuming` / `sequence_numbers_are_verified_per_source` / `a_run_number_change_before_eos_is_counted_and_latches_error`(3) | seq 検証・run 変化検出 |
| | `unsupported_frames_are_counted_without_latching_error` / `malformed_frames_are_counted_and_latch_error`(2) | unsupported / malformed の区別 |
| | `sending_a_batch_advances_the_output_sequence_and_counters` / `an_abandoned_batch_is_counted_and_leaves_a_visible_sequence_gap` / `metrics_json_carries_every_counter_of_the_ticket`(3、計 14) | 自前 seq・カウンタ・GetStatus 材料 |
| `src/bin/decoder.rs` | `parses_the_config_path` / `rejects_missing_or_malformed_arguments`(2) | CLI |
| `tests/decoder_integration.rs`(port 0、PUSH で RawFrames 直接投入 + PULL で Fragments 受け) | `two_sources_become_one_fragment_stream_and_exactly_one_eos_closes_it` | (a) + (b) |
| | `one_source_eos_alone_does_not_close_the_stream` | (b) 反証側 |
| | `a_sequence_gap_on_one_source_latches_error_but_consumption_continues` | (c) |
| | `an_unsupported_frame_is_counted_but_never_becomes_a_fragment_nor_an_error` | (d) |
| | `configure_arm_start_stop_sequence_succeeds_and_idle_sends_heartbeats` | (e) |
| | `reset_abandons_the_blocked_send_and_counts_what_was_dropped`(計 6) | (f) |
| `tests/decoder_pipeline_real_graw.rs` | `real_graw_replayed_through_receiver_and_decoder_matches_the_p1_oracle`(1) | E2E |

**実 .graw E2E 実測**(`CoBo_2025-09-01T08_51_06.203_0000.graw` = 30,108,684 B、graw_replay 全速 →
receiver(006、既定パラメタ)→ decoder → テスト側 PULL。debug ビルドで 4 回連続実行、値は全回同一)

| 項目 | 実測 | オラクル |
|---|---|---|
| events(distinct event_idx) | **108** | 108 ✔(P1) |
| items 合計(受信 Fragment の items から算出) | **15,040,512** | 15,040,512 ✔(P1) |
| malformed(GetStatus) | **0** | 0 ✔(P1) |
| unsupported(GetStatus) | **1** | 1 ✔(実 run 先頭の frameType 7・12 B) |
| fragments_out / frames_in | 108 / 109 | 108 データ + 1 制御フレーム ✔ |
| items_out(GetStatus)| 15,040,512 | 受信側の実測と一致 ✔ |
| 出力バッチ数 / 自前 seq | 4 通 / 0,1,2,3 連続 | 単一ストリーム・欠番なし ✔ |
| 自分の EOS | ちょうど 1 本(全 Fragment の後) | 1 ✔(SPEC §2.3) |
| source_id / cobo | 100 / {0} | decoder = 100、CoBo 識別は Fragment.cobo ✔ |
| seq_gaps / run_mismatches / batches_abandoned / eos_abandoned | 0 / 0 / 0 / 0 | すべて 0 ✔ |
| 状態 | Error にならず | unsupported は Error にしない ✔ |
| 所要(全経路) | debug 1.243–1.296 s(4 回)/ **release 0.190 s** | release は 30.1 MB / 0.190 s ≈ **158 MB/s** = mini 100 Hz 相当(28 MB/s)の約 5.6 倍、ELITPC 定常(111 MB/s)超え |

debug ビルドの 1.3 s は decode(15,040,512 item の展開 + パック)が CPU 律速なため。性能の
評価は release 値で行うこと(§12-5/6 の負荷試験は別ユニット)。

**フレーク確認**: `tests/decoder_integration.rs` 8 回連続実行 = 8/8 green、
実 .graw E2E 4 回連続実行 = 4/4 green(オラクル値も毎回同一)。

**スキップしたテスト**: なし(`TPCDAQ_REAL_GRAW` がローカル実 .graw を指しており E2E も実行・記録済み)。

**逸脱・裁量点(レビュー要)**

1. **PUSH に `ZMQ_IMMEDIATE` を立てた**(`src/decoder.rs::Handler::build_push`)。libzmq の既定では
   「まだ接続していない相手」にも HWM 分(既定 1000 通)をキューに積んで **send が成功を返す**。
   発注書のテスト (f) は「受け側 PULL を作らない」で詰まらせることを求めているが、既定のままだと
   詰まらず送出が成功してしまう(実測で確認)。より本質的に、root-sink 不在時に最大 1000 通
   (8 MiB × 1000)を「送れた」ことにしてプロセス内に抱えるのは「下流が居ない = 背圧」という
   ロスレスの契約(SPEC §1.4-2)と食い違う。`ZMQ_IMMEDIATE` は**接続確立済みの相手にだけ積む**
   ようにする(相手が居るときの一時的停滞は従来どおり HWM が吸収)。発注書に明記が無い挙動変更
   なので報告する。SPEC §1.4/§3.2 に一文足すか、他コンポーネント(receiver の PUSH ×2)にも
   同じ扱いを広げるかは**判断が必要**。
2. **出力バッチの close 判定は入力バッチ 1 通を処理し終えた時点**で行う(フレーム途中では切らない)。
   したがって 1 通の出力は最大で「入力バッチ相当 + 1 Fragment」まで 8 MiB を超えうる
   (上流 receiver も同じ 8 MiB 規則なので有界)。フレーム単位で厳密に 8 MiB で切る実装も可能だが、
   コアの API がフレーム単位に割れて複雑になるため KISS を採った。実 .graw E2E の出力は 4 通
   (合計 ≈ 57 MiB、1 通あたり ≈ 14 MiB)。
3. **`Reset` 後もコア(カウンタ)を保持する**(007 の `do_reset` は writer を捨てる)。
   「破棄が可視化されていれば許される」が Reset を許す条件なので、Reset 後の `GetStatus` で
   `batches_abandoned` / `eos_abandoned` を読めなければ意味がないため。
4. **`heartbeat_ms` / `send_timeout_ms` は TOML キーにしていない**(発注書の `[decoder]` 追加キー
   一覧に無いため)。`config.rs` の定数(`DEFAULT_HEARTBEAT_MS` / `DEFAULT_DECODER_SEND_TIMEOUT_MS`
   = 1000 ms)から `DecoderParams` に入る。設定可能にする必要があれば追記は容易。
5. **unsupported / malformed のログは初回のみ**(カウンタは常に進み metrics に出る)。ホットパスで
   per-frame のログ整形をしない(CLAUDE.md)ための措置で、receiver の `record_dropped_frame` と
   同じ流儀。silent ではない。
6. **`Fragment.cobo` はフレームヘッダの値のまま**(Batch の `source_id` で上書きしない)。
   SPEC §2.3「CoBo の識別は Fragment.cobo が担う」+ §2.4(ヘッダ全フィールドを運ぶ)に従った。
   受信 CoBo 番号とヘッダ値の不一致検出は本ユニットの要求に無いので入れていない。

## レビュー指摘(2026-08-13 Fable → v1.4 対応の修正発注)

逸脱 1(`ZMQ_IMMEDIATE`)は**重要な発見として採用し、SPEC v1.4 で仕様に昇格**した(§1.2:
「ロスレス PUSH は ZMQ_IMMEDIATE 必須。実装は zmq_helper の PUSH ヘルパに集約。モニタ系 PUB には
適用しない」)。逸脱 2〜6 はすべて受理(3 の Reset 後カウンタ保持は可視化の趣旨に合致、
6 の cobo 不一致検出は P2 批判的レビューの論点として継続事項へ)。

**修正項目(SPEC v1.4 への一致)**:

1. `zmq_helper::apply_push_hwm`(ロスレス PUSH 用ヘルパ)に `set_immediate(true)` を統合し、
   decoder の直書き `set_immediate` はヘルパ経由に置き換え(焼き込み分散禁止)。
2. receiver の PUSH ×2(→graw-writer / →decoder)にも同ヘルパ経由で適用。receiver の既存テストが
   「未接続相手への send 成功」を前提にしていたら期待値をロスレス契約側(不在 = 詰まる/overflow
   可視化)へ更新し、never-stop(TCP drain 継続)が保たれることを確認。
3. ゲート再実行(fmt/clippy/test)+ receiver と graw-writer の実 .graw E2E 再実行(バイト一致が
   保たれること)。本節の下に「### v1.4 対応結果」を追記 → Status を IMPLEMENTED(レビュー待ち)へ。

### v1.4 対応結果(2026-08-13)

**実行環境**: macOS 26.5.2 / arm64、rustc 1.97.1、cargo 1.97.1、2026-08-13。

**修正内容**

1. **`zmq_helper` へ集約**(`src/zmq_helper.rs`): `apply_push_hwm(socket)` に `set_immediate(true)` を
   統合し、HWM を設定で変えるコンポーネント向けに `apply_push_hwm_with(socket, hwm)` を新設
   (`hwm <= 0` は `apply_hwm` と同じく拒否)。モジュール doc に「なぜロスレス PUSH に
   `ZMQ_IMMEDIATE` が要るか(SPEC §1.2 v1.4)」を追記。**PUB には適用しない**(落として良いリンク)。
2. **decoder**(`src/decoder.rs::Handler::build_push`): 直書きの `set_immediate` を削除しヘルパ経由へ。
   このコンポーネント固有の `sndtimeo`(Reset 時の打ち切り粒度)だけを残した。
3. **receiver**(`src/receiver.rs::Handler::build_links`): `apply_hwm`(送受両方向)→
   `apply_push_hwm_with(socket, params.hwm)` へ置換(PUSH に rcvhwm を設定していた分も解消)。
   never-stop との関係を doc に明記: 詰まるのは送信タスクだけで、drain タスクは有界キューへ
   `try_send` するだけなので止まらない。

**既存テストの期待値更新**: **不要だった**。receiver の全既存テストは下流 PULL を bind してから
Start するため(未接続相手への send 成功に依存したテストは存在しなかった)、期待値の変更なしで green。
代わりに v1.4 が作る**新しい状況(下流がそもそも居ない)**の回帰テストを 1 本追加した。

**新規テスト(3)**

| ファイル | テスト | 主張 |
|---|---|---|
| `src/zmq_helper.rs` | `lossless_push_gets_immediate_but_the_monitor_pub_does_not` | ロスレス PUSH(既定/HWM 明示の両方)は IMMEDIATE、PUB は立てない |
| | `without_a_peer_a_lossless_push_blocks_while_a_bare_socket_would_pretend_to_send` | 下流不在で helper 経由の send は `EAGAIN`、素の PUSH(HWM のみ)は `Ok` を返す — 差分そのものを固定 |
| `tests/receiver_overload.rs` | `an_absent_downstream_never_counts_as_sent_and_still_does_not_stop_the_drain` | 下流不在でも drain は 4 MiB を読み切り、溢れは可視化、**batches = 0**(送れたことにしない) |

**テスト結果**: リポ全体 **218 passed / 0 failed**(v1.4 対応で +3)。`cargo fmt --check` 差分なし、
`cargo clippy --all-targets -- -D warnings` クリーン。全体 3 回連続 + receiver 系 5 回連続で green
(フレークなし)。

**never-stop の実測**(`tests/receiver_overload.rs`、queue_frames=2 / hwm=1 / 4 MiB 投入)

| 下流の状態 | bytes | frames | overflow_frames | batches |
|---|---|---|---|---|
| **不在**(誰も bind していない、v1.4 の新状況) | 4,194,304 | 512 | 509 | **0**(1 通も「送れたこと」にしない) |
| 停止(bind 済みだが recv しない、従来) | 4,194,304 | 512 | 448 | 64(詰まるまでは送れる) |

どちらも fake CoBo の write はブロックせず(never-stop 維持)、全バイトをソケットから読み切り、
溢れは `overflow_frames` + Error として可視化された。

**実 .graw E2E 再実行**(`CoBo_2025-09-01T08_51_06.203_0000.graw` = 30,108,684 B、v1.4 適用後)

| テスト | 実測 | 判定 |
|---|---|---|
| `decoder_real_graw`(純コア) | events=108 / items=15,040,512 / malformed=0 / unsupported=1 / framer resets=0 | オラクル一致 ✔ |
| `receiver_real_graw` | 両リンクともバイト一致、frames 109 × 2 / batches 5 × 2 / overflow_frames=0、0.163 s | バイト一致維持 ✔ |
| `graw_writer_real_graw` | AsAd 30,108,672 B + ctrl 12 B = 30,108,684 B(= 入力)、asad_frames=108 / ctrl_frames=1 / seq_gaps=0 / write_errors=0、0.194 s | バイト一致維持 ✔ |
| `decoder_pipeline_real_graw` | events(distinct)=108 / items=15,040,512 / malformed=0 / unsupported=1 / fragments_out=108 / frames_in=109 / 4 バッチ seq 0..3 / EOS 1 本 / abandoned 0、1.303 s(debug) | P1 オラクル一致 ✔ |

`overflow_frames = 0`(receiver E2E)は、下流が居る通常運転では IMMEDIATE が**何も変えない**ことの
実測でもある(変わるのは「下流不在」のときだけ)。

**スキップしたテスト**: なし。

**逸脱**: なし(発注 3 項目をそのまま実装)。`apply_hwm`(送受両方向の汎用版)は
テスト用途で残存(`tests/zmq_backpressure.rs` / `tests/receiver_overload.rs` の PULL 側設定)。
production のロスレス PUSH はすべて PUSH ヘルパ経由になった
(`grep -rn "set_immediate\|apply_hwm" src/` で確認可能)。

### 最終レビュー(2026-08-13 Fable)

- **判定: 受理(COMPLETED)**。本体実装の逸脱 1(ZMQ_IMMEDIATE)は SPEC v1.4 に昇格、
  2〜6 受理(6 の cobo 不一致検出は P2 批判的レビューの論点として CURRENT.md 継続事項へ)。
  v1.4 対応は逸脱なし + receiver の潜在不備(PUSH ソケットへの RCVHWM 適用)も解消。
- レビュー側で独立再検証: `set_immediate` は zmq_helper 1 箇所に集約(grep 確認)、
  fmt/clippy クリーン、**cargo test 218 passed / 0 failed**、receiver / decoder-pipeline の
  実 .graw E2E 再実行 green(オラクル維持: events=108 / items=15,040,512 / unsupported=1)。
- 特筆: 「helper 経由 = EAGAIN、素の PUSH = Ok(送れたふり)」の差分そのものを固定するテストと、
  下流不在での never-stop 実測(drain 継続 + overflow 可視 + batches=0)は v1.4 の意味を
  そのまま回帰化しており模範的。
