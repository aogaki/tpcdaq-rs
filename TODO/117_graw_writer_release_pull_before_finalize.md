# 117 — graw-writer の受信スレッドは finalize(fsync)の間も PULL ポートを握っている

**Status: IN PROGRESS**(2026-10-12 起票 Opus。116 の Fable 裁定 (b)。Sonnet 向け — 発注書で縛れる)

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
