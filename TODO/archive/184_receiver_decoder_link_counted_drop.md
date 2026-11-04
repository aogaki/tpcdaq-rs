# 184 — receiver: decoder リンクは待たずに送って数えて落とす + Error 中の `Stop` を実行する

**Status: COMPLETED**(2026-11-03 コミット。実機の受け入れ = ELITPC 反映後、185・186 と一緒に — CURRENT)(2026-10-24 起票 Opus、実装 **Opus implementer**。設計 = TODO/181 §5(Fable レビュー済み)、仕様 = SPEC v1.53 §1.4-3 / §1.4-7 / §1.3「異常中止」②)

## 0. 背景(1 行)
run 197: receiver の送信スレッド 1 本が graw-writer と decoder の 2 リンクへ順に**ブロック送信**しているため、decoder → root-sink が遅れると graw への送信も止まり、内部キューが溢れて graw まで失った。さらに Error の receiver は `Stop` を拒否し、強制 EOS を出せなかった。

## 1. 変更(決定済み — 設計の分岐は無い。出会ったら実装せず報告)
### A. `src/receiver.rs` の送信(`sender_loop` / `flush` / `send_on`)
- `Link` に「この リンクはロスレスか」を持たせる(graw-writer = ロスレス、decoder = 数えて落とす)。リンクの組み立て箇所(`receiver.rs:1089` 付近)で決める。
- **decoder リンクの Data**: `ZMQ_DONTWAIT` で 1 回だけ送る。
  - 成功 → `sequence_number += 1`(今と同じ)。
  - **EAGAIN(HWM 満杯 / `ZMQ_IMMEDIATE` で相手不在)→ そのバッチは decoder へ送らない。`sequence_number` は進めない**。`decoder_link_dropped_frames += batch.len()`、`decoder_link_dropped_batches += 1`。初回だけ warn(「decoder link full or absent — dropping and counting (graw is unaffected, SPEC §1.4-7)」の趣旨)、以後はカウンタだけ。
  - 符号化失敗・EAGAIN 以外の send 失敗(= 本物の喪失)は**今どおり** `sequence_number` を進めて gap として見せる(`send_errors` / `encode_errors`)。
- **decoder リンクの Heartbeat**: 同じく `ZMQ_DONTWAIT`、EAGAIN なら `decoder_link_dropped_heartbeats += 1`(Heartbeat に seq は無い — `msg.rs:104` — ので番号の扱いは不要)。
- **graw-writer リンク**: 今と同じブロック送信(背圧で待つ、`abandon` のときだけ打ち切って数える)。
- **EndOfStream**: 両リンクとも今と同じブロック送信(間引かない、SPEC §2.2)。
- `batches` カウンタの意味は変えない(「プロセスから出て行ったバッチ」= どちらかのリンクへ届いたら 1)。
- 新しいカウンタ 3 つを `Metrics` に足し、GetStatus の metrics に `decoder_link_dropped_frames` / `decoder_link_dropped_batches` / `decoder_link_dropped_heartbeats` で出す。run ごとのリセットは既存のカウンタと同じ場所(`Metrics` の reset、`receiver.rs:180` 付近)。
- 送信スレッド・キューの本数は**変えない**(Fable 裁定: 2 キュー 2 スレッド案は不採用)。
### B. Error 中の `Stop`(receiver / decoder / graw-writer の 3 つ)
- 今は `command.rs` の遷移表に `(Error, Configured)` が無く、Error のコンポーネントは `Stop` を「illegal transition Error -> Configured」で拒否する。
- 新しい規則: **state == Error で `Stop` を受けたら、そのコンポーネントの通常の Stop の処理(receiver なら `do_stop` = drain を畳んで未送の EOS を出す)を実行し、状態は Error のまま、応答は成功**(メッセージで「stop executed in Error state」と分かるように)。続く `Reset` で Idle(今どおり)。
- 遷移表(`ComponentState::can_transition_to`)は変えない(Error → Configured を合法にすると「Error が消える」ので不可)。各コンポーネントの `handle()` に 1 分岐(3 つで同じ形)。
- decoder / graw-writer の Stop 処理が Error 中に安全か(既に止まっているスレッド・閉じたファイル)を確かめ、二重に畳んでも panic・二重 close しないこと。

## 2. テスト(受け入れ — TDD: 先に赤を確認し、報告に赤の証拠を書く)
既存の流儀に合わせる(`tests/receiver_overload.rs`、`tests/receiver_integration.rs`、`tests/zmq_backpressure.rs`、各 `src/*.rs` の `#[cfg(test)]` を参照)。
1. decoder リンクの PULL を bind しない(相手不在)/ 受け手を止めた状態で N バッチ送出 → graw-writer 側は全フレームを受け取る、`decoder_link_dropped_frames` = 落としたバッチのフレーム数の合計、receiver は **Running のまま(Error にならない)**、`overflow_frames == 0`。
2. 落とした後に decoder 側が受信を再開 → decoder が観測する seq に gap が無い(`seq_gaps == 0` 相当 — 落としたバッチで seq を進めていない)。
3. decoder リンクが満杯でも **EOS は届く**(空くのを待ってから送る)。
4. graw-writer 側を止めると、従来どおり内部キューが溢れて `overflow_frames` + Error(既存のテストが無改変で green ならそれでよい)。
5. **Error 状態の receiver に `Stop`** → 応答は成功、EOS が両リンクへ出る、状態は Error のまま、続く `Reset` で Idle。decoder / graw-writer も Error で `Stop` → 成功・Error のまま・`Reset` で Idle。
6. 正常な start 列(listen-before-start → decoder PUSH の connect)で**先頭バッチが落ちない**(既存の e2e が green、加えて `decoder_link_dropped_frames == 0` を確かめる箇所を 1 つ)。
7. decoder 不在のアイドル中、Heartbeat の落としが `decoder_link_dropped_heartbeats` に数えられる。
- 全体: `cargo fmt && cargo clippy --tests -- -D warnings && cargo test` green(テスト数を報告)。

## 3. 範囲
- 触ってよい: `src/receiver.rs`、`src/decoder.rs`、`src/graw_writer.rs`(B の分岐)、`src/command.rs`(必要なら説明コメントだけ — 遷移表は変えない)、`tests/` の receiver / zmq / e2e 系、新しいテストファイル。
- **触らない**: `src/controller/`(185 が並行して触る可能性)、`tools/root_sink/`(186)、`docs/`(SPEC v1.53 は主対話が書き済み)、`TODO/`。コミットしない。

## 4. 報告
- `git diff --stat`、テスト 1–7 の名前と場所と PASS / FAIL、赤の証拠、cargo のテスト数、**実行できなかった操作**(無ければ「なし」)、発注書からの逸脱と理由。
- 終わったら、自分が起動したプロセス(テストの子プロセス等)が残っていないことを確かめて報告に書く。

## 結果
### 実装(2026-10-24 前任 Opus implementer が本体の作業ツリーで実装 → セッション終了で報告なしに中断。2026-10-30 後任 Opus implementer が照合して引き継ぎ、主対話 Opus が照合)
- 後任の照合: A・B とも発注書どおりに実装済みで欠陥なし → **ソースは無変更**(後任が足したのは赤の採取・反復実行・実データ e2e・最終ゲート)。
- 本体の作業ツリー(未コミット): `src/command.rs` +5、`src/decoder.rs` +22、`src/graw_writer.rs` +23、`src/receiver.rs` +266 / −23、`tests/receiver_integration.rs` +9、`tests/receiver_overload.rs` +5 / −3、新規 `tests/receiver_decoder_link.rs`(542 行)・`tests/stop_in_error.rs`(436 行)。範囲外(TODO / docs / macros / controller / root_sink / ui)は無変更。
- 中身: decoder リンクは `ZMQ_DONTWAIT` で送り EAGAIN なら数えて落とす(seq を進めない、Heartbeat も同じ)、graw リンクと EOS は従来どおり待つ、`batches` の意味は不変、カウンタ `decoder_link_dropped_frames` / `_batches` / `_heartbeats`(start で reset)、スレッドとキューは 1 本のまま。receiver / decoder / graw-writer の `handle()` に Error 中の `Stop` を実行する同じ形の 1 分岐(遷移表は不変)。
- 受け入れテスト(すべて PASS):
  1. `tests/receiver_decoder_link.rs::an_absent_decoder_is_dropped_and_counted_while_graw_gets_every_frame`(graw 37 フレーム / 13 バッチ = dropped 37 / 13、Running のまま、overflow 0)
  2. `…::a_stalled_decoder_that_resumes_sees_no_sequence_gap`(総数 70 = decoder 13 + dropped 57、バッチ 27 = 8 + 19、seq は 0 から連続 — PULL 側で連続性を直接見る)
  3. `…::the_end_of_stream_waits_for_a_full_decoder_link_and_is_delivered`(flooded 64 = 6 + 58、EOS は待って届く、abandoned 0)
  4. `tests/receiver_overload.rs::queue_overflow_counts_frames_enters_error_and_keeps_draining`(既存、末尾の Stop の主張だけ変更 — 逸脱 1)
  5. `tests/stop_in_error.rs` の `a_receiver_in_error_executes_stop_forces_eos_on_both_links_and_stays_in_error` / `a_decoder_in_error_executes_stop_stays_in_error_and_resets_to_idle` / `a_graw_writer_in_error_executes_stop_closes_its_files_and_resets_to_idle`
  6. `tests/receiver_integration.rs::both_links_reassemble_the_cobo_bytes_exactly` に `decoder_link_dropped_*` == 0 を追加
  7. `tests/receiver_decoder_link.rs::heartbeats_to_an_absent_decoder_are_dropped_and_counted_while_idle`(Heartbeat 3 = dropped 3)
  - 単体: `receiver::tests::decoder_link_drops_are_counted_every_time_but_warned_only_once`、`start_resets_every_counter_including_the_new_ones`
- 赤の証拠(後から採取 — `git archive HEAD` に現行テストを重ねて別 target でビルド): receiver_decoder_link 4 / 4 FAILED(テスト 1 は `graw-writer: PULL recv failed/timed out` = 送信スレッドが decoder リンクで止まり graw が来ない = **run 197 の機序の再現**、他は `decoder_link_dropped_* is not a number`)、stop_in_error 3 / 3 と receiver_overload は `illegal transition Error -> Configured`、単体はコンパイルエラー(E0599 × 7)。
- 安定性: 4 テストバイナリ × 25 回 = 100 回で失敗 0。実データ(env ゲート、ローカルの実 .graw、root_sink は 10-24 19:12 版): `p2_e2e` 108 / 108 × 2 パス 0 differences、`mini_rawtree_e2e` 108 entries 0 differences、`decoder_pipeline_real_graw` / `graw_writer_real_graw` / `receiver_real_graw`(両リンクでバイト一致)/ `root_sink_intake` 11 本 PASS。未実行: `p3_e2e`・`soak_smoke`(固定ポート)、`elitpc_pevent_e2e`(ELITPC 資産)。
- implementer の最終ゲート: `cargo fmt && cargo clippy --tests -- -D warnings && cargo test` exit 0、**1120 passed / 0 failed / 1 ignored**。1 回目は `graw_replay_tool::connect_failure_exits_non_zero_with_clear_message` が 1 回だけ失敗(閉じたポートに繋ぎに行く間の取り合い = TOCTOU、185 の worktree の並走と重なった。単独 20 / 20 PASS、再ゲート green、本チケットと無関係)→ 既存の時間依存テストとして記録(要るなら別チケット)。
- **主対話の再実行(本体の作業ツリー、2026-10-30)**: テスト名がファイルにあることを確認、`cargo fmt --check` clean、`cargo clippy --tests -- -D warnings` 警告 0、`--test receiver_decoder_link` 4 / `stop_in_error` 3 / `receiver_integration` 7 / `receiver_overload` 3 passed、`--lib receiver` 21 passed、すべて 0 failed。
- 逸脱(主対話の判断: すべて受け入れ): ①テスト 4 の末尾 `assert!(!bad.success, "Error からの Stop は通らない")` は B と正面から矛盾 → 「Stop は成功・state は Error のまま・続く Reset で Idle」に差し替え(溢れの部分は無改変。graw だけ止めて decoder 健全の切り分けを単独で見るテストは無い)②初回 warn のフラグは Data と Heartbeat で共通の 1 本(`logged_decoder_link_drop`、run ごとに reset — 同じ状況なので 1 回でよい = KISS)③テスト 2 の gap 判定は PULL 側の seq の連続性(本物の decoder の `seq_gaps` ではない — 発注書の「相当」の範囲)。
- 実行できなかった操作: なし(1 回目のゲートの exit 101 は上の flaky、再実行で green)。残プロセスなし。
- 残り: コミット(本体)→ 185・186 のマージ → 188 / 187 → ELITPC への反映。
- **コミット前の確認(2026-11-03、主対話、main の作業ツリー)**: `cargo fmt --check` OK、`cargo clippy --tests -- -D warnings` OK、`cargo test` = **1,120 passed / 0 failed / 1 ignored**(63 の test binary)。

## 実機の受け入れ(2026-11-03、ELITPC、184 + 185 + 186 = `663b00d`、ユーザー「DAQ が開いているのでやっちゃいましょう」)
- 方法: `~/test/accept184/accept.sh`(bench183 を元に、daq.toml / daq.env は触らない = `pevent = false`・`raw_tree = true`、CoBo の周期トリガ、12.5 MHz・delay 2500・502 ns・120 fC・全読み出し、終了時に params を外部トリガへ自動で戻す)。
- **A1 = run 211**(周期 10 ms、60 s): 受信 4,652 = graw 4,652 = decoder 入力 4,652 = decoder 出力 4,652、decoder リンクの落とし 0、lost_receiver_to_graw / _to_decoder 0 / 0、overflow 0、ok / normal、停止 3.7 s、root_sink は停止と同時に閉じた(events_complete 4,652、incomplete 0)、`root_complete: true`。
- **A2 = run 212**(周期 1 ms、60 s): **受信 15,267 = graw 15,267 = decoder 入力 15,267 = decoder 出力 15,267(≈ 250 Hz)**、decoder リンクの落とし 0、lost 0 / 0、overflow 0、ok / normal、停止 5.2 s、root_sink は停止の 11 s 後に閉じた(events_complete 15,267、incomplete 0、late 0)、`root_complete: true`。graw 4 ファイル(≈ 4.25 GB)、raw.root 1.84 GB。
- 判定: **≈ 250 Hz で graw は 1 フレームも落ちず、端から端まで数が一致**(run 197 は 76 Hz で graw 19,855 喪失)。run_stop の新しいキー(リンクごとの数・root_complete)も期待どおり。
- **未検証**: 184 の本命の経路(下流が詰まったとき decoder リンクだけで数えて落とす)は今回発火しなかった — `pevent = false` の root_sink は 250 Hz の積み残しを停止後 11 s で書き切るので受信へ逆圧しない。run 197 そのもの(`pevent = true`、≈ 76 Hz)の再現は daq.toml の一時変更と daq_down / up 2 回が要る → ユーザーの了承待ち。186(次の run で未完を閉じる)は ELITPC では controller が EOS を必ず出すので通常は起きない(テストで担保)。
- **A3 = run 213(run 197 と同じ条件: `pevent = true`、周期 10 ms ≈ 73 Hz、60 s、`~/test/accept184/accept197.sh` — daq.toml を一時変更し daq_down / up、終了時に自動で戻す。ユーザー了承「やっちゃってください」)**: 受信 4,658 = graw 4,658 = decoder 入力 4,658 = decoder 出力 4,658、decoder リンクの落とし 0、lost 0 / 0、overflow 0、ok / normal、停止 3.7 s、root_sink は停止の 3 分半後に閉じた(PEventTPC ≈ 17 Hz、events_complete 4,658、incomplete 0、run0213.root / _0001 / _0002 + raw + monitor — inprogress 残りなし)、`root_complete: true`、receiver0 / decoder / graw_writer / controller のログに WARN / ERROR 0。15:09:21 に `pevent = false`・`--no-root`・Idle へ復元。
  - 判定: 60 s では run 197(同条件で graw 19,855 喪失)の失敗は起きない。ただし**決め手の経路(decoder リンクで数えて落とす)はまだ発火していない** — 60 s 分の積み残し ≈ 3,500 事象が root_sink の受け口(rcvhwm 1,000)+ 内部キュー(1,000)+ ZMQ のバッファに収まった。run 197 は数分続いた → 4–5 分の run で溢れさせる案をユーザーに確認中。
- **A4 = run 214(`pevent = true`、周期 10 ms、**300 s** — 決め手の経路を発火させる、ユーザー了承「やっておくれやす」)**: 受信 23,252 = **graw 23,252**(lost_receiver_to_graw 0)、**decoder リンクで数えて落とした 6,251**(batches 6,250・heartbeats 3、開始 ≈ 1.5 分後から)、decoder 入力 15,479、`lost_receiver_to_decoder` 1,522(差として記録)、overflow 0、`root_complete: false`、5 分間 run は止まらない。**→ SPEC v1.53 の核(graw はロスレス、派生側だけ数えて落とす、run は止めない、消えた数はリンクごとに残す)を実機で確認 = 合格。** ただし停止は `ok: false`・`error:eos-timeout`(EOS が詰まった decoder リンクで捨てられ、root_sink は 15,478 を全部書いたが run を閉じずに待った)→ **204** に起票(Fable キュー)。15:30:34 に `pevent = false`・`--no-root`・Idle・外部トリガへ復元。
- 後始末(2026-11-03 15:38、ユーザー了承): run 214 の未完の ROOT 2 本を削除、ログブックに comment(seq 6877)。ELITPC は `pevent = false`・外部トリガ・Idle。
