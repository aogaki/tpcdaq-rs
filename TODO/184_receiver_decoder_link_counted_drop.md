# 184 — receiver: decoder リンクは待たずに送って数えて落とす + Error 中の `Stop` を実行する

**Status: IN PROGRESS**(2026-10-24 起票 Opus、実装 **Opus implementer**。設計 = TODO/181 §5(Fable レビュー済み)、仕様 = SPEC v1.53 §1.4-3 / §1.4-7 / §1.3「異常中止」②)

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
(記入中)
