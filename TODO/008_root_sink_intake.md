# 008 — root-sink 取り込み骨格(C++、ROOT 非依存、ロスレス PULL)

**Status: OPEN**
**仕様**: SPEC §6.1(流用範囲)、§6.2(ロスレス化チェックリスト = 本ユニットの核)、
§2.1–§2.4(ワイヤ)、§1.4(過負荷)。
**流用元**(いずれもユーザー自身のプロジェクトのコード、読み取り + 移植可):
delila-rs `tools/root_sink/eb_core.hpp`(`Channel<T>`)、`tools/delila2root/TDelila.hpp`
(ヘッダオンリー MessagePack リーダ)
**依存**: 003(Rust 側 golden fixture 生成 `write_golden_stream` — 適合性テストの相手)

## やること

1. `tools/root_sink/` を新設(C++17、この段階では **ROOT 非依存・libzmq のみ**):
   - `tpc_wire.hpp` — MessagePack リーダ(TDelila.hpp の `mp::Reader` を流用コピー、出自コメント
     明記)+ TPC スキーマデコード: `Message` fixmap(1) {Data/EndOfStream/Heartbeat}、
     `Batch` positional array(5) [source_id, run_number, sequence_number, created_ns, payload]、
     `Fragment` positional array(12)、items = bin(u32 LE 連結)。
     **先頭 N フィールドを読み、残りは skip**(前方互換 — delila 方式)。
   - `rs_core.hpp` — 純ロジック(ZMQ 非依存):
     - `Channel<T>`(有界、cap>0 で push ブロック — eb_core.hpp から移植。closed への push は
       assert で明示化 — SPEC §6.2-7)
     - `RunState`(期待ソース集合ベースの run open/close。P2 の期待集合 = {decoder=100} のみ —
       SPEC §2.3 の decoder ソース性を参照)
     - `SeqCheck`(ソース毎 sequence_number 連続性。**ギャップ = fatal** — SPEC §6.2-5)
   - `root_sink.cxx` — main(この波では「数えるだけ」の骨格):
     ZMQ **PULL bind**(既定 `tcp://*:47003`)、**RCVHWM 有限**(既定 1000、CLI で変更可)—
     §6.2 チェックリスト 1–3 の実装点。受信スレッド → 有界 Channel → 集計スレッド
     (counts: batches / fragments / items / eos / runs)。SIGINT/SIGTERM で統計を stdout に
     JSON 1 行で出して終了。**malformed バッチ = 即 exit 非 0**(§6.2-6。黙って捨てない)。
2. ビルド: `tools/root_sink/Makefile`(g++ -std=c++17、libzmq は pkg-config)。
   単体テストは素の assert + main(delila root_sink の試験流儀): `test_tpc_wire.cpp` /
   `test_rs_core.cpp`。`make test` で全部走る。
3. **クロス言語適合(§10.4 方式)**: Rust の `write_golden_stream`(003 実装済み、
   `tpcdaq::msg::golden_messages`)が吐く長さ前置ストリームを C++ 側テストが読み、既知値
   (例: `EndOfStream{1,7}` = `81 ab "EndOfStream" 92 01 07`)と機械照合。
   `tools/root_sink/run_conformance.sh` 1 本で「cargo で fixture 生成 → make test で照合」を自動化。
4. **実配線 + 背圧試験**: Rust 統合テスト `tests/root_sink_intake.rs`(env `TPCDAQ_ROOT_SINK_BIN`
   未設定なら skip — C++ ビルドを cargo test の前提にしない)から C++ バイナリを spawn し、
   PUSH connect → Data×N + EOS → 終了時 JSON counts を機械検証。小 HWM で送信側がブロックする
   こと(背圧の実在)も確認。

## 受け入れ

- ヘッダ単体テスト + クロス言語適合 + 実配線試験すべて green(`make test` と
  `run_conformance.sh` が単独で通る)。
- malformed = 非 0 exit の動作確認。seq ギャップ = fatal の動作確認。
- Rust 側は `cargo fmt/clippy/test` 無影響(追加は env-gated テスト 1 ファイルのみ)。
