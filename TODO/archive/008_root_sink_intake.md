# 008 — root-sink 取り込み骨格(C++、ROOT 非依存、ロスレス PULL)

**Status: COMPLETED**
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
   (例: `EndOfStream{source_id:0, run_number:7}` = `81 ab "EndOfStream" 92 00 07` —
   2026-08-13 訂正: 当初 `{1,7}` と誤記。003 実装の golden 実値が正)と機械照合。
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

## 結果

**実行環境**: macOS 26.5.2 / arm64 / Apple clang 21.0.0 / libzmq 4.3.5(pkg-config)/
rustc 1.97.1。**2026-08-13**。

### 実行コマンドと結果

| コマンド | 結果 |
|---|---|
| `make -C tools/root_sink clean && make -C tools/root_sink test` | 3 テスト green(下表) |
| `tools/root_sink/run_conformance.sh` | exit 0(fixture 生成 → 照合まで自動) |
| `make -C tools/root_sink`(root_sink 本体) | 警告ゼロでビルド(`-Wall -Wextra`) |
| `TPCDAQ_ROOT_SINK_BIN=$PWD/tools/root_sink/root_sink cargo test --test root_sink_intake` | **4 passed / 0 failed**(2.27 s) |
| `cargo test`(env 未設定) | **176 passed / 0 failed**(root_sink_intake の 4 本は skip 動作 = 即 ok) |
| `cargo clippy --all-targets -- -D warnings` | 警告ゼロ |
| `rustfmt --check tests/root_sink_intake.rs examples/write_golden_stream.rs` | 差分なし |

### C++ 単体テスト(`make test`)

| テスト | 結果 |
|---|---|
| `test_tpc_wire` | **68 passed / 0 failed**(前セッションから不変) |
| `test_rs_core` | **71 passed / 0 failed**(新規 green。死亡テスト = closed への push が SIGABRT を含む) |
| `test_conformance` | **49 passed / 0 failed**(GOLDEN 指定時)/ 未指定時は SKIP して exit 0 |

### 適合性照合(クロス言語、SPEC §10.4)

Rust `write_golden_stream` の出力 **144 バイト / 4 通**を C++ 本番デコーダで読んで一致:

- `EndOfStream{0,7}` = `81 ab "EndOfStream" 92 00 07`(16 バイト、バイト列そのものを照合)
- Batch(RawFrames): source_id=0 / run=7 / seq=0 / created_ns=1755000000123456789 /
  frames=[`08 05 00 01 02 03`, `ff 7f`]
- Batch(Fragments): source_id=100 / run=7 / seq=1 / event_idx=42 /
  event_time=0x0000123456789abc / mult=[68,0,17,3] / last_cell=[7,9,11,13] /
  items = `0x00000000`, `0x96cb04d2`, `0xffffcfff`(予約ビット [13:12] = 0 も確認)
- Heartbeat: source_id=0 / run=7 / counter=3

### 実配線試験(`tests/root_sink_intake.rs`、C++ バイナリを spawn)

| テスト | 実測 |
|---|---|
| `counts_data_batches_and_closes_the_run_on_eos` | Data×3 + EOS → `batches=3 fragments=6 items=27 eos=1 runs=1`、異常カウンタ全 0、SIGTERM で exit 0(手計算: 3+5 / 7 / 2+4+6 = 27 items) |
| `a_malformed_message_kills_the_process_instead_of_being_dropped` | fixarray(3) を 1 通 → **exit 2**、`fatal="malformed-message"`、`batches=0`(§6.2-6) |
| `a_sequence_number_gap_is_fatal` | seq 0 → seq 2 → **exit 3**、`fatal="sequence-break"`、`batches=1`(§6.2-5) |
| `a_throttled_sink_makes_the_sender_block_without_losing_anything` | `--rcvhwm 1 --queue 1 --throttle-ms 50`、1 通 256 KiB(65536 items)× 16 通。**非ブロッキング送信は 2 通目で EAGAIN**(= 背圧の実在)。残りをブロッキング送信すると `batches=16 fragments=16 items=1048576 eos=1 runs=1` で**ロス 0** |

背圧の実測値: **EAGAIN after 2 non-blocking sends, all 16 delivered**。

### SPEC §6.2 チェックリストの充足状況

| # | 項目 | この波での状態 |
|---|---|---|
| 1 | SUB → PULL、sink 側 bind | 済(`ZMQ_PULL` + `zmq_bind`、既定 `tcp://*:47003`) |
| 2 | RCVHWM 有限 | 済(既定 1000、`--rcvhwm` で変更可。0 以下は起動失敗) |
| 3 | 内部 Channel 有界 | 済(`Channel<T>` は容量 0 を禁止。実配線試験で背圧を実測) |
| 4 | モニタ tee は有界 + 落とし可 | **未**(PUB がまだ無い波。`try_push` は意図的に未移植) |
| 5 | seq 連続性 = Error | 済(`SeqCheck`、gap/regression とも fatal) |
| 6 | malformed = Error 停止 | 済(exit 2) |
| 7 | closed channel への push を明示化 | 済(`assert` + 死亡テスト。Makefile は NDEBUG を定義しない) |
| 8 | run 境界でブロッキング外部 IO 禁止 | 該当なし(この波に外部 IO が無い。ROOT 書き出しの波で再確認) |

### スキップしたテストとその理由

- `tests/root_sink_intake.rs` の 4 本は **`TPCDAQ_ROOT_SINK_BIN` 未設定なら skip**(発注書 §4)。
  C++ ビルド(libzmq 必須)を `cargo test` の前提にしないため。skip 時も `cargo test` は green。
- `test_conformance` は GOLDEN 未指定なら SKIP(exit 0)。`make test` 単体を cargo に依存させないため。
- 実 .graw を使う回帰は本ユニットの対象外(このユニットは合成メッセージのみ)。

### 発注書からの逸脱・追加(レビュー対象)

1. **`--throttle-ms N`(集計スレッドの人工遅延)を追加**。発注書に無いが、**背圧試験を成立させるのに
   必要**(消費が速いと満杯にならず、送り手はブロックしない)。この波にはまだ ROOT 書き出しという
   「遅い消費者」が存在しないための代役で、既定 0 = 無効。
2. **異常終了時も JSON 1 行を stdout に出す**(`"fatal"` フィールドに理由)。発注書は SIGINT/SIGTERM
   時の JSON のみを要求しているが、落ちた瞬間のカウンタを捨てるのは事故調査上もったいないため。
   終了コードは malformed=2 / seq 違反=3 / usage=1 / ZMQ 失敗=4。
3. **`RunState` が EOS の `run_number` 食い違いも `run_number_mismatch` に数える**(テストは要求して
   いない)。EOS 自体は受理する(捨てると writing のまま固まる = delila-rs の実地事故)。
4. **カウンタを発注書の 5 つより増やした**: `heartbeats` / `unknown` / `stale_eos` /
   `unexpected_sources` / `run_number_mismatch`。「silent failure を作らない」(CLAUDE.md)の実装。
5. **`tools/root_sink/.gitignore` を追加**(C++ ビルド生成物をコミットしないため。リポ直下の
   .gitignore は触っていない)。
6. `Channel<T>` は delila-rs の `try_push` / `pop_for` / 容量 0(無制限)を**移植しなかった**
   (この波に用途が無い = KISS。§6.2-4 のモニタ tee が要る波で持ってくる)。

### 未解決・次の波への申し送り

- **`Cargo.toml` への依存追加は一切していない**。SIGTERM 送出は `libc` を足さずに `kill(1)` の
  サブプロセス起動で行っている(`Child::kill` は SIGKILL なので JSON が出ない)。libc を足す判断が
  出たら差し替え可能。
- 実配線試験の 2 箇所で固定 sleep(500 ms / 1.6 s)を使っている。root_sink 側にも停止後 200 ms の
  ドレイン猶予があり二重の余裕はあるが、進捗を機械的に待てる口(例: 統計の逐次出力)を後の波で
  用意すればより堅くなる。
- 期待ソース集合は `{decoder=100}` を**コードに直書き**している(SPEC §2.3 の P2 前提)。設定から
  与える形にするのは設定配線の波の仕事。

### レビュー(2026-08-13 Fable、diff + テスト出力の一括レビュー)

- **判定: 受理(COMPLETED)**。逸脱 1–6 はすべて受理 — いずれも「silent failure を作らない」
  「KISS」に沿い、発注書の意図を超えない範囲の実装判断。
- レビュー時に修正 2 点(いずれも挙動不変): ① root_sink.cxx の未使用 `#include <atomic>` を除去し
  `make test` 再実行で green 再確認(68/71/49)。② 本発注書 §3 の golden 例示 `{1,7}` の誤記を
  `{0,7}` に訂正(実装側の指摘 7 が正しい — src/msg.rs golden_messages の実値で裏取り)。
- **申し送り(次の波で扱う)**: `SeqCheck` は run 開始時の初回 seq を基準値として受理する
  (初回 = 0 を強制しない)。decoder リンクでは SPEC §2.2 が「run 開始で 0 リセット」を規定して
  おり、先頭バッチ喪失が不可視になる余地がある。009(decoder)/ 012(E2E)の波で
  「期待初回 = 0」の強制を検討 → CURRENT.md 継続事項に記載。
