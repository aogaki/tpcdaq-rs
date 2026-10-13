# 092 — 自然 EOF 後の receiver: 「downstream is full — blocking on backpressure」→ teardown で 1 通 abandoned(069 #13 で観測)

**Status: COMPLETED(2026-10-13、結果は末尾)** — 旧: READY(2026-09-23 起票 Fable。要調査、Opus 0.5 日)

## 0. 事実(069 #13、run 27)
- 板の getHwServer 停止で receiver が `TCP EOF — end of stream` → `EndOfStream sent to both links`(10:28:59)。
- 2 s 後 `downstream is full or absent — blocking on backpressure (lossless; only Reset / teardown may …)`(10:29:01)。
- 68 s 後、controller の stop(ECC stop の 60 s timeout 後の Reset)で `send abandoned while stopping (Reset / component teardown) — this message is lost and counted`(10:30:07)。
- 疑問: EOS 送出後に receiver は何を送ろうとしたか(ハートビート? 空バッチ?)、なぜ下流が「full」になったか(graw-writer / decoder は EOS 受領後に PULL を止める?)。
  データは失われていない(EOS の前に全フレームは送出済み)はずだが、**silent 禁止の観点で「何が 1 通」かを特定して記録する**。

## 1. 発注
- `src/receiver.rs` の EOF 経路を読み、EOS 送出後にループが送るメッセージ(Heartbeat?)と、下流 PULL 側(graw_writer / decoder)が EOS 後に recv を続けるかを確認。
- 再現テスト: 既存の receiver 統合テスト(EOF → EOS)に「EOS 後 3 s 待ってから Reset」を足し、abandoned カウンタが 0 であるべきか 1 であるべきかを仕様として決めて固定
  (仕様案: **EOS 後はハートビートも送らない**(下流は EOS で run を閉じる)= abandoned 0 が正)。
- SPEC §1.3 / §2.2 に「EOS 後の receiver は静止」を 1 行(Fable)。

## 2. 参照
069 #13 結果、032(silent stall)、SPEC §1.3(停止シーケンス)、§2.2(EOS はバリア)。

## 結果(2026-10-13、Opus implementer → 主対話で照合・マージ `ad18c73`)

### 原因
- EOS を送った後に送ろうとしていたのは**アイドルの Heartbeat**(空バッチや 2 本目の EOS ではない)。
  - EOF の後、drain タスクは `'accept` ループに戻り、送信キューは生きたまま。
  - そのため `sender_loop` は `last_send + heartbeat`(既定 1000 ms)ごとに Heartbeat を両リンクへ送っていた。
- 「下流が full」は満杯ではなく**相手がいない**状態:
  - graw-writer は期待ソース全部の EOS で `consume_until_done` を抜け、PULL を drop する(117)。
  - receiver の PUSH は `ZMQ_IMMEDIATE` なので、相手のいない send は EAGAIN でブロックする。
  - EOS の 1 s 後に Heartbeat、SNDTIMEO の 1 s 後に warn。計 2 s で、実機ログの 10:28:59 → 10:29:01 と一致。
- abandoned は実際には **2**(graw-writer 宛てと、まだ送っていない decoder 宛ての 2 通)。warn は初回しか出ないので、ログでは 1 通に見えていた。
- データは失われていない。全フレームと EOS は Heartbeat より前に両リンクへ届いている(テストで確認)。

### 修正(`src/receiver.rs` の `sender_loop` のみ、+15 / −3、コミット `65cb42e`)
- EOS を送った時点で `eos_sent` を立て、以後 Heartbeat を出さない。
- EOS 後の待ちは打ち切り確認の間隔(100 ms)だけにした。過去の期限のままだと `recv_timeout(0)` で空回りするため。
- 同じ run の中で再接続した後の Data は従来どおり流す(既存テスト `eof_returns_to_accept_and_the_next_run_resets_sequence_numbers` が仕様として固定)。
  `eos_sent` は戻さないので、その後のアイドルでも Heartbeat は再開しない。下流はすでに run を閉じているので、これで足りる(主対話の判断)。

### テスト
- 新規 `tests/receiver_integration.rs::after_a_natural_eos_the_receiver_stays_silent_and_reset_abandons_nothing`。
  - 手順: run 27 → 2 フレーム → EOF → 両リンクで EOS → graw-writer の PULL を drop(実機の再現)→ decoder の PULL を 3 s poll して何も来ないことを確認 → Running から Reset。
  - 確認する値: `messages_abandoned == 0`、`send_errors == 0`、`frames == 2`。heartbeat は実機既定の 1000 ms。
- **赤(修正前の実測)**: `messages_abandoned` left: 2 / right: 0 で FAILED。
- **緑**: `receiver_integration` は 7 passed(既存 6 + 新規 1)。
- **main でのゲート**(マージ後、Mac、2026-10-13):
  - `cargo fmt -- --check` OK
  - `cargo clippy --tests -- -D warnings` 警告 0
  - `cargo test` **1070 passed / 0 failed / 1 ignored**(ignored は既存のベンチ `bench_storage_throughput_vs_snapshot_rate`)
- スキップしたテスト: なし。
- 実機での再確認(板の getHwServer を止める試験)はしていない。必要なら 131 で。

### 照合(主対話)
- `git show --stat`: `src/receiver.rs` と `tests/receiver_integration.rs` の 2 ファイル。上記のテスト名が diff にある。
- implementer の「実行できなかった操作」: 複合コマンド(`cd … && cat >> … && cargo test`)が worktree 隔離のチェックで拒否された → Edit と単独コマンドでやり直し、影響なし。

### 別件の観察
- decoder も自分の EOS の後に root-sink へ Heartbeat を出し続ける。
- ただし root-sink はプロセスが生きている間ずっと PULL を開けている(`tools/root_sink/root_sink.cxx` の `zmq_close(pull)` は終了時だけ)ので、同じ詰まりは起きない。対応不要。

### SPEC
§1.3 / §2.2 に 1 行(EOS を送った後の receiver はそのリンクに Heartbeat を出さない)→ CURRENT の Fable キュー(c)。

**Status: COMPLETED(2026-10-13)**
