# 117 — graw-writer の受信スレッドは finalize(fsync)の間も PULL ポートを握っている

**Status: COMPLETED**(2026-10-12 起票・同日完了。116 の Fable 裁定 (b)。Sonnet 実装、Opus レビュー)

## 事実(コード読み、116 の implementer / Fable)
- `src/graw_writer.rs` の `run_writer_thread` は、受信ループを抜けた後に `finalize()`(開いている全ファイルの flush + fsync + close)を呼ぶ。
  PULL ソケット(`pull`)は関数の末尾まで生きているので、**fsync の間もポートを握っている**。
- Reset はスレッドを待たない。そのため Reset 直後の Arm が bind する時点で fsync が終わっていないと、`Address already in use` になりうる
  (Arm の予算は 620 ms。116 の decoder と同じ型の失敗)。高レートで dirty なページキャッシュが大きいと fsync は長くなる。
- 4.5 Hz の実機では出ていない。優先度は低いが約 30 行の修正なので、次の実機試験の前に入れる。

## 仕様(Fable 裁定)
1. **所有権で固定する**: 受信ループの部分を、`pull: zmq::Socket` を **move で受け取る**関数に切り出す。
   その関数から戻った時点で `pull` が drop される(RAII、コンパイル時保証)。**その後に** `finalize()` を呼ぶ。
2. **Reset は join しない**(fsync は `command_timeout` 5 s を超えうる。ポートは finalize より前に空くので、join する理由が無い。
   ファイル実績は controller が Reset の前に採る規約のまま)。
3. 挙動はほかに変えない(EOS の扱い、カウンタ、ファイル名、finalize の中身)。

## テスト(TDD、116 と同じ型)
- graw-writer の統合テスト(`tests/` の既存ハーネス)に 1 本: Arm → Start → データを少し送る → Stop → Reset → **同じ PULL エンドポイントに 1 回で bind できる**。
  - fsync を遅く見せる仕掛けが要るなら、finalize の前後でポートの状態を観測できる形にするか、テスト用の遅延フックを最小限に足す(本番経路に分岐を増やさない形)。
  - 作れなければ、作れない理由を報告する。
- 既存の graw-writer テスト(バイト一致・ファイル分割・EOS で閉じる)をすべて green のまま保つ。

## ゲート
`cargo fmt && cargo clippy --tests -- -D warnings && cargo test`

## 結果(2026-10-12)
- 実装(Sonnet `f56c0f8` → merge `b3a0abc`):
  - `src/graw_writer.rs` の受信ループを `consume_until_done(pull: zmq::Socket, …)` に切り出した。`pull` は move で渡すので、戻った時点で drop される。その後に `finalize()` を呼ぶ。
  - Reset は join しない。EOS の扱い・カウンタ・ファイル名・finalize の中身は変えていない(Opus が diff で確認)。
  - テスト用フック `GrawWriterParams.finalize_delay_ms`(本番は `from_config` で常に 0)。本番経路の分岐は `if !finalize_delay.is_zero()` の 1 つだけ。
- テスト: `tests/graw_writer_integration.rs` に `reset_frees_the_pull_port_before_finalize_finishes` を追加。
  - 手順: finalize_delay 2500 ms → Stop → Reset → 1500 ms 以内に同じポートへ bind できること。
  - red は、受信関数が socket を返して finalize の後まで保持する形に一時的に戻して作った(`Address already in use`、1.53 s で失敗)。
  - 発注書の「1 回で bind」は、受信スレッドが停止合図に気づくまで最大 `POLL_TIMEOUT_MS` 200 ms かかるのでリトライ付きにした。本番では controller の Arm リトライ(620 ms)がこの窓を覆う。
  - 統合 7 passed(新規 1 + 既存 6)。
- ゲート(main `b3a0abc`、Mac): fmt OK / clippy 警告 0 / **cargo test 896 passed / 0 failed / 1 ignored**(ignored は既存の `tests/root_sink_monitor_pub.rs:1031`)。
- 実機(ELITPC、10:05、α): `~/crash_test.sh after-117` → run 67 は歩き戻しの跡 `eos: closed` で閉じた(83 件、`run_stop 67` は `abort:controller-restart`)。run 68 は正常に開始・停止(57 件、`normal`)。退行なし。スタックは Idle。
