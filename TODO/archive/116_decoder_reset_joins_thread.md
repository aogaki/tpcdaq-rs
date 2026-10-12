# 116 — decoder の Reset がスレッドの終了を待たずに返り、直後の Arm が bind に失敗する

**Status: COMPLETED**(2026-10-12 起票・同日完了。114 の SIGSTOP 実機試験で発見。Opus implementer、Fable 承認)

## 事実(ELITPC、2026-10-12 09:28–09:32、`~/sigstop_test.sh`)
- 手順: α run 64 の途中で root-sink を `kill -STOP`。decoder の `batches_out` が 988 で止まり、ログは `downstream is full — blocking on backpressure`。
  そこで controller を kill -9 → 再起動 → start。
- 歩き戻し(114 / 115)は期待どおりに動いた。
  - 跡は `eos: did not propagate`。
  - `run_stop 64` は `abort:controller-restart`、`ok:false`、`eos_closed:false`、`duration_s` 258.4。
- **しかし新しい run 65 の start は Arm 段で失敗した**: `decoder: Arm refused in state Configured: bind tcp://*:47002 failed: Address already in use (gave up after 6 attempts over 627 ms)`。
  - decoder のログ: Reset 06:32:29.430 → Arm の bind 失敗 6 回(29.434〜30.061)→ `Fragments batch abandoned … batches_abandoned=1` → `decoder thread stopped` 06:32:30.328。
  - つまり **Reset からスレッドが抜けて PULL ポートを放すまで約 900 ms** かかった。
- 原因:
  - `decoder::do_reset` は `abandon` を立てて stop を合図するだけで、スレッドを待たずに返る。スレッドは detach されていて、JoinHandle を持っていない。
  - 送信は `send_timeout_ms` = 1000 ms(`DEFAULT_DECODER_SEND_TIMEOUT_MS`)ごとにしか `abandon` を見ない。
  - controller の Arm リトライ予算は合計 620 ms(`ARM_RETRY_BACKOFF` 20 ms × 倍々 × 6 回)なので、構造的に足りない。
- receiver の Reset は `join_sender` でスレッドの終了を待ってから返る(「打ち切り合図の後なので join は有限時間で返る」)。decoder にはこれが無い。
- **試験の作りによるものではない**: 通常の停止経路でも、root-sink が詰まっている状態で stop(EOS タイムアウト → Reset)の直後に start すると(Apply & restart など)同じ失敗になる。
- 失敗の後は全員 Idle に戻り(rollback)、スレッドも抜けているので、もう一度 start すれば通る。復旧手段はあり、silent でもない。
- run 64 のデータ:
  - graw は 1219 フレームで、eventIdx 0–1218 の連番(正本は無傷)。
  - ROOT は 1041 件で `inprogress` のまま(EOS 未着、`stopped mid-run — kept … NOT finalized`)。

## 仕様(Opus implementer)
1. `decoder` はスレッドの `JoinHandle` を持つ。`do_reset` は `abandon` + stop の合図の後に **join してから返る**(receiver の `join_sender` と同じ形)。
   - 上限は `send_timeout_ms` + 1 周程度。`abandon` の後は送信が `SendOutcome::Abandoned` で抜けることを確かめる。
   - 受信側(`recv` のタイムアウト / poll)も有限時間で抜けることをコードで確認する。join が永久にブロックする経路を作らないこと。
2. **`Stop` は join しない**(Stop は非破壊で、背圧の下では下流が空くまで送り続けるのが正しい。REQ をブロックさせない)。
3. graw-writer は受信ループが `POLL_TIMEOUT_MS` = 200 ms で、送信が無いので今回の問題は出ない。KISS で今回は触らない。触るべきだと判断したら報告する。
4. テスト(TDD):
   - decoder の単体または統合で、「下流が詰まって送信がブロックしている decoder に Reset → Reset が返った時点でスレッドは終わっていて、同じポートに即 bind できる」を red → green。
   - 既存の decoder テストのヘルパで下流不在(PUSH の相手なし、`ZMQ_IMMEDIATE`)を作れるはず。
   - 既存の「Reset 後もカウンタは読める」を落とさない。
5. ゲート: `cargo fmt && cargo clippy --tests -- -D warnings && cargo test`。

## 実機ゲート
`~/sigstop_test.sh` を再実行し、歩き戻しの後の start が Arm を通ることを確認する。
期待: run 65 が始まり、`kill -CONT` の後に root-sink が新しい run の最初の Data で exit 6(v1.12 終端条項の正しい検出)。

## Fable キュー(この票では直さない)
- **Reset で decoder の PULL ソケットを閉じたとき、受信キューの未読分はどのカウンタにも載らない**。
  run 64 では graw 1219 に対し ROOT 1041。差 178 のうち、カウントされたのは `batches_abandoned=1` だけ。
  Reset(明示の打ち切り)の経路に限られ、receiver の送出数と decoder の `frames_in` の差としてしか見えない。
  可視化の方法(例: Reset 時に close の前に PULL を drain して数える)は Fable が判断する。

## 結果(2026-10-12)
- 実装(Opus `07d87f4` → merge `99e7704`):
  - `src/decoder.rs` の `Handler` に `thread: Option<JoinHandle<()>>` を持たせ、`do_start` で保持する。
  - `do_reset` は abandon → 停止の合図 → `join_thread()`(receiver の `join_sender` と同じ形。panic していたら `error!`)。`do_stop` は従来どおり待たない。
  - join の上限(コード読み): poll ≤ 100 ms + 合図の後の送出は最大 2 本 × `send_timeout_ms` = 1000 ms → **≈ 2.1 s < `command_timeout` 5 s**。
    `send_timeout_ms` は TOML から変えられず、常に 1000 なので、永久ブロックの経路は本番では無い。
- テスト(`tests/decoder_integration.rs` +2、修正前は `Address already in use` で red):
  - `reset_returns_only_after_the_thread_has_released_the_pull_port`: Reset の応答に最終カウンタ `batches_abandoned=1` / `eos_abandoned=1` が載る。
  - `stop_does_not_wait_for_a_blocked_thread_but_the_following_reset_frees_the_port`
  - 40 回連続で green、並列 3 × 10 で 30/30。
  - ゲート(main `99e7704`): clippy 0、**cargo test 895 passed / 0 failed / 1 ignored**。
- **実機(ELITPC、`~/sigstop_test.sh` の再実行、09:46–09:52)**:
  - 手順: run 65 の途中で root-sink を SIGSTOP → decoder の `batches_out` が 989 で詰まる → controller を kill -9 → 再起動 → start。
  - 歩き戻しの跡は `eos: did not propagate within 5000 ms`、`run_stop 65` は `abort:controller-restart` / eos_closed false / 255.6 s。
  - **decoder の Reset(06:51:03.300)から Arm(.305)まで 5 ms で、run 66 は始まった**(修正前は 627 ms で bind をあきらめて失敗)。
  - `kill -CONT` → root-sink は `FATAL run-number-mismatch run_number_open=65 run_number_incoming=66`(exit 6 = v1.12 の終端条項どおりの検出)。
  - run 66 の停止は `error:eos-timeout` / eos_closed false(root-sink が不在なので正しい)。decoder の Reset は打ち切ったバッチを数えて join した。
  - graw: run 65 = 1207 フレーム、run 66 = 102 フレーム、どちらも eventIdx 連番(**正本は無傷**)。run 65 の ROOT は inprogress(可視)。
  - 試験の後は daq_down / up でスタックを Idle に戻した。
- **Fable 裁定(2026-10-12)**: 116 は承認。
  - (a) join の上限 0.1 s + 2T は見送り(縮めると下流が生きていても捨てる方向になる)。制約は SPEC の不等式として固定した。
  - (b) graw-writer → **117**(Sonnet。PULL を所有権で finalize より前に drop する。Reset は join しない)。
  - (c) Reset 中に status が不達に見える件は SPEC に明記のみ。
  - (d) **Fable キュー項目「PULL の未読分」は裁定済み**: 新しいカウンタは足さない。`eos_closed:false` の run の実損は、run 単位で receiver の frames と root-sink の entries を照合する(SPEC v1.40 §1.3)。
- ゲート: 最終 main `99e7704` で fmt OK / clippy 0 / cargo 895 passed / 0 failed / 1 ignored(既存の `tests/root_sink_monitor_pub.rs:1031`)。
