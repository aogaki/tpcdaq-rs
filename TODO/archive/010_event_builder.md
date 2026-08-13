# 010 — eventIdx イベントビルダ(C++ 純ヘッダ、root-sink 組み込み)

**Status: COMPLETED**
**仕様**: SPEC v1.2 §6.3(イベントビルダ = 本ユニットの核)、§2.4(Fragment)、§6.2(ロスレス化
チェックリスト)、§2.2(seq 規則)。008 の成果物(tpc_wire.hpp / rs_core.hpp / root_sink.cxx)が土台。
**依存**: 008(COMPLETED — archive/008_root_sink_intake.md のレビュー節も読むこと)
**発注先想定**: implementer/Opus(多ソースの順序・タイムアウト・遅延到着の判断が残る)

## やること

1. `tools/root_sink/eb_core.hpp` — 純ロジック(ZMQ 非依存・ROOT 非依存。rs_core.hpp と同じ流儀):
   - `OwnedFragment` — Fragment のヘッダ全フィールド + items バイト列の**所有コピー**
     (`tpcwire::FragmentView` から作る。ビルダはバッチ寿命を超えて保持するため所有が必要。
     ゼロコピー化は P5 実測で必要になったら — KISS)。
   - `EventBuilder`:
     - コンストラクタ: 期待フラグメント集合 `std::set<std::pair<uint8_t,uint8_t>>`(= {(cobo,asad)})
       と `build_timeout_ms`(既定 1000)。
     - キー = `(run_number, event_idx)`。`feed(OwnedFragment, now_ms)` で蓄積。
       **時刻は必ず引数で注入**(内部で clock を読まない — テストが sleep なしで書けるように)。
     - 全期待 (cobo,asad) が揃ったら complete。`now_ms - first_arrival_ms > build_timeout_ms` で
       **incomplete フラグ付きで emit 対象**(捨てない — SPEC §6.3)。
     - **emit は event_idx 昇順**: 先頭(最小 event_idx)が complete または timeout になるまで
       後続は emit しない(`poll(now_ms)` が emit 可能な BuiltEvent 列を昇順で返す)。
     - **遅延到着**(emit 済み event_idx への到着)= 捨てずに `LateFragment` として返し、
       `late_fragments` カウント(SPEC §6.3「順序より可逆性優先」)。
     - `flush()`(EOS 時): 残イベント全部を昇順 emit(complete 済みは complete のまま、
       揃っていないものは incomplete — 2026-08-13 レビューで字面を訂正。実装の解釈が正)。
     - カウンタ: events_complete / events_incomplete / late_fragments / pending(瞬間値)。
   - 単一 CoBo 構成(期待集合 1 要素)では実質素通しになることをテストで確認(SPEC §6.3)。
2. `rs_core.hpp` の `SeqCheck` に**厳格モード**を追加(008 レビュー申し送りの解消):
   `SeqCheck(bool expect_zero_start)` — true なら各ソースの**初回 sequence_number は 0 のみ受理**
   (≠0 は Gap 扱い)。root_sink.cxx は decoder リンクに対し true で使う(SPEC §2.2:
   run 開始で 0 リセット。先頭バッチ喪失を不可視にしない)。既存 test_rs_core は既定
   (false = 従来挙動)で無改変のまま通ること + 厳格モードのテストを追加。
3. `root_sink.cxx` 組み込み(まだ「数えるだけ」の延長 — ROOT はまだ入れない):
   - `--expect 0:0,0:1,1:0,1:1` 形式の CLI で期待 (cobo,asad) 集合を与える(既定 `0:0` = mini)。
     ジオメトリ由来の導出は後続ユニット(設定配線の波)。`--build-timeout-ms N`(既定 1000)。
   - 集計スレッドで Data バッチの各 Fragment を EventBuilder へ feed、poll の結果を数える。
     run close(全 EOS)で flush。終了 JSON に events_complete / events_incomplete /
     late_fragments を追加。
   - **run 中に wall-clock を注入する頻度**: バッチ到着毎 + アイドル時も timeout が働くよう、
     Channel pop のタイムアウト付き版(`pop_for` — delila-rs eb_core.hpp から移植してよい。
     出自コメント明記)で最大 100 ms 毎に poll を回す。
4. テスト:
   - `test_eb_core.cpp`(assert + main、Makefile の TESTS へ追加):
     2 ソース完全ビルド / 到着順シャッフルでも昇順 emit / 先頭未完のうちは後続を emit しない /
     timeout で incomplete emit(時刻注入、sleep なし)/ 遅延到着 = LateFragment + カウント /
     flush で残り全 emit / run_number が別なら別イベント / 単一ソース素通し /
     SeqCheck 厳格モード(初回 0 のみ受理)。
   - `tests/root_sink_intake.rs` に統合テストを追加(env `TPCDAQ_ROOT_SINK_BIN` ゲートは既存踏襲):
     2 CoBo × 2 AsAd の Fragment バッチを event_idx 交互・順序シャッフルで投入 →
     JSON counts で events_complete = N・events_incomplete = 0・late_fragments = 0。
     1 fragment 欠けの run → EOS flush 後 events_incomplete = 1。
   - `make test` と `run_conformance.sh` が引き続き単独 green(既存 68/71/49 を壊さない)。

## 受け入れ

- 上記全テスト green(`make test` 4 本 + Rust 側 env-gated 統合)。既存 Rust テスト無影響。
- malformed = 非 0 exit・seq gap = fatal(厳格モード)の既存動作維持。
- ファイル所有権: tools/root_sink/(eb_core.hpp 新規、rs_core.hpp / root_sink.cxx / Makefile /
  test_*.cpp の改変可)+ tests/root_sink_intake.rs。**src/ と Cargo.toml に触らない**
  (並列で 009 が src/ を作業中)。

## 結果

**実行環境**: macOS 26.5.2(Darwin 25.5.0)/ arm64 / Apple clang 21.0.0 /
libzmq 4.3.5(pkg-config)/ rustc(cargo test プロファイル dev)。**2026-08-13**。

### 変更・新規ファイル

| ファイル | 区分 | 中身 |
|---|---|---|
| `tools/root_sink/eb_core.hpp` | 新規 | `OwnedFragment` / `BuiltEvent` / `LateFragment` / `EventBuilder`(ZMQ 非依存・ROOT 非依存) |
| `tools/root_sink/test_eb_core.cpp` | 新規 | 単体テスト(assert + main、17 ケース) |
| `tools/root_sink/rs_core.hpp` | 改変 | `SeqCheck(bool expect_zero_start)` 厳格モード + `Channel<T>::pop_for`(delila-rs から移植)+ `PopResult` |
| `tools/root_sink/root_sink.cxx` | 改変 | `--expect` / `--build-timeout-ms`、EventBuilder 組み込み、decoder リンクを厳格 SeqCheck に、JSON カウンタ 6 個追加 |
| `tools/root_sink/Makefile` | 改変 | `test_eb_core` を TESTS へ追加 |
| `tests/root_sink_intake.rs` | 改変 | 統合テスト 3 本追加(2 CoBo×2 AsAd 完全ビルド / 1 fragment 欠け / 厳格 seq)+ `fragment_at` ヘルパ |

`src/*.rs` / `Cargo.toml` / `examples/` には**一切触っていない**(並列作業中の 009 の領域)。

### 実行コマンドと結果

| コマンド | 結果 |
|---|---|
| `make -C tools/root_sink clean && make -C tools/root_sink test` | **4 テスト green**(下表) |
| `make -C tools/root_sink`(本体) | 警告ゼロ(`-Wall -Wextra`、NDEBUG なし) |
| `tools/root_sink/run_conformance.sh` | exit 0(68/71/152/49 —— 既存 68/71/49 を壊していない) |
| `TPCDAQ_ROOT_SINK_BIN=$PWD/tools/root_sink/root_sink cargo test --test root_sink_intake` | **7 passed / 0 failed**(2.01 s。既存 4 + 新規 3) |
| `cargo test --no-fail-fast`(env 未設定) | **215 passed / 0 failed**(root_sink_intake の 7 本は skip 動作 = 即 ok。途中 `tests/decoder_integration.rs` の 1 本が red だった時期があるが、これは並列作業中の 009 の領域で、その後の同エージェントの修正で green。本ユニットの変更は `tools/` と `tests/root_sink_intake.rs` のみ) |
| `cargo clippy --all-targets -- -D warnings` | 警告ゼロ |
| `rustfmt --edition 2021 --check tests/root_sink_intake.rs` | 差分なし |

### C++ 単体テスト(`make test`)

| テスト | 結果 |
|---|---|
| `test_tpc_wire` | **68 passed / 0 failed**(無改変) |
| `test_rs_core` | **71 passed / 0 failed**(**ファイル無改変** —— 厳格モードのテストは test_eb_core 側に置いた) |
| `test_eb_core` | **152 passed / 0 failed**(新規) |
| `test_conformance` | **49 passed / 0 failed**(GOLDEN 指定時)/ 未指定は SKIP |

`test_eb_core` のケース(17 関数 / 152 CHECK):
所有コピー(元バッファ破壊後も無傷)/ 2 ソース完全ビルド / 到着順シャッフルでも昇順 emit /
先頭未完のうちは後続を emit しない / timeout で incomplete emit(境界 1000 ms は出さない・
1001 ms で出す)/ timeout した先頭が後続の門を開ける / 遅延到着 = LateFragment + カウント /
flush で残り全 emit(揃っていたものは complete のまま)/ flush 後も遅延判定が生きる /
run_number が別なら別イベント(遅延判定も run 毎)/ 単一ソース素通し / 期待外 (cobo,asad) を
数えて載せる / 重複を数えて載せる / 期待集合が空なら abort(死亡テスト)/
SeqCheck 厳格モード 4 本(初回 0 受理・初回≠0 は Gap・既定は従来どおり・ソース毎 + reset 後)。
**タイムアウト系は時刻注入なので sleep ゼロ**(スイート全体 < 10 ms)。

### 実配線試験(`tests/root_sink_intake.rs`、実測 JSON)

| テスト | 実測 |
|---|---|
| `two_cobo_two_asad_shuffled_fragments_build_complete_events` | `--expect 0:0,0:1,1:0,1:1 --build-timeout-ms 60000`。6 イベント × 4 ソース = 24 フラグメントを 4 ラウンド(ラウンド毎に event_idx の向きを反転)で投入 → `{"batches":6,"fragments":24,"items":84,"events_complete":6,"events_incomplete":0,"late_fragments":0,"unexpected_fragments":0,"duplicate_fragments":0,"pending_events":0,"eos":1,"runs":1,"fatal":""}`(手計算: items = 4 × (1+2+3+4+5+6) = 84) |
| `a_missing_fragment_is_emitted_as_an_incomplete_event_on_eos` | 4 イベント × 4 ソースから (event 2, cobo 1, asad 0) を 1 個抜く → `{"batches":4,"fragments":15,"items":37,"events_complete":3,"events_incomplete":1,"late_fragments":0,"pending_events":0,"runs":1,"fatal":""}`(手計算: items = 4×1 + 4×2 + **3**×3 + 4×4 = 37。incomplete を出したのは timeout ではなく **EOS の flush**) |
| `a_run_whose_first_batch_is_missing_is_fatal`(追加) | 初回 seq=3 の 1 通 → **exit 3**、`fatal="sequence-break"`、`batches=0`(厳格モードが root_sink.cxx で効いていることの実配線確認) |
| 既存 4 本(counts / malformed / seq gap / 背圧) | 無改変で green(背圧: EAGAIN after 2 non-blocking sends, all 16 delivered) |

### スキップしたテストとその理由

- `tests/root_sink_intake.rs` の 7 本は **`TPCDAQ_ROOT_SINK_BIN` 未設定なら skip**(008 踏襲。
  C++ ビルドを `cargo test` の前提にしない)。
- `test_conformance` は GOLDEN 未指定なら SKIP(`make test` 単体を cargo に依存させないため)。
- ROOT / TTree 書き出しは**本ユニットの対象外**(011)。組み上がった `BuiltEvent` は
  `absorb()` で数えるだけで破棄している(011 の接続点にコメントを置いた)。
- 実 .graw を使う回帰は対象外(合成メッセージのみ)。

### 発注書からの逸脱・追加(レビュー対象)

1. **`OwnedFragment` に `run_number` を持たせた**。発注書の `feed(OwnedFragment, now_ms)` という
   シグネチャと「キー = (run_number, event_idx)」を両立させるため(Fragment 自身は run を
   持たない —— run は Batch ヘッダ側 §2.2)。`from_view(view, run_number)` で呼び手が入れる。
2. **カウンタを 2 個追加**: `unexpected_fragments`(期待 (cobo,asad) 集合外)/
   `duplicate_fragments`(同一イベントに同じソースが 2 回)。どちらも**フラグメント自体は捨てず
   イベントに載せる**が、complete 判定には数えない。黙って捨てる/黙って complete にするのは
   CLAUDE.md の絶対ルール違反なので数えて JSON に出した。
3. **`flush()` は「揃っていたイベント」を complete のまま出す**。発注書の字面は「残イベント全部を
   incomplete として」だが、揃っているものを incomplete と記録すると events_complete /
   events_incomplete の意味が壊れる。「complete を待たずに全部出す」と解釈した(テスト
   `test_flush_emits_everything_left_in_order` が仕様として固定)。
4. **停止時(SIGTERM ドレイン後)にも `flush()` する**。発注書は run close の flush のみ要求。
   run 途中で止めたときに組み上げ中のイベントがカウンタから消えるのを避けるため。
5. **`pop_for` の戻り値を bool → `PopResult` 3 値にした**(delila-rs からの移植点の変更)。
   delila の bool 版は「timeout も closed も false」で、呼び手が別途 closed を見るしかなく、
   *pop_for が空で戻る → 生産者が push + close → 呼び手が closed を見て break* の順序で
   **最後の 1 通を落とす**。キューの状態と closed を同じロック内で判定して塞いだ。
6. **`--expect` の重複指定(`0:0,0:0`)と範囲外(`0:256`)を起動失敗にした**(SPEC §3.2
   「設定パースエラーは起動失敗」)。期待集合が空の `EventBuilder` は assert で abort。
7. **統合テストを 1 本追加**(`a_run_whose_first_batch_is_missing_is_fatal`)。発注書の Rust 側
   受け入れ 2 本に加えて、「厳格モードを **root_sink.cxx の decoder リンクで**有効化」を
   実配線で確かめるため(C++ 単体テストだけでは配線の確認にならない)。
8. **厳格モードのテストは `test_eb_core.cpp` に置き、`test_rs_core.cpp` は 1 文字も触っていない**
   (発注書 §2「既存 test_rs_core は無改変のまま通ること」を最大限に取った。§4 のテスト一覧でも
   厳格モードは test_eb_core 側に挙げられている)。

### 未解決・次の波への申し送り

- ~~**イベント内のフラグメント順は「到着順」**~~ → **解決**(SPEC v1.3 §6.3 で
  「(cobo, asad) 昇順で決定的」と確定。下の「### v1.3 対応結果」を参照)。
- `emitted_upto_`(run → emit 済み最大 event_idx)は run が閉じても消さない(EOS 後の迷子を
  pending に復活させないため)。run 番号 1 個あたり 8 バイトなので実運用で問題にならないが、
  長寿命プロセスでの掃除は 011/012 で必要になったら。
- `--expect` は CLI 直指定。**ジオメトリ .dat からの導出は設定配線の波**(発注書どおり)。
- `absorb()` が `BuiltEvent` を捨てている(この波では数えるだけ)。011 でここが TTree 書き出しに
  なる。`LateFragment` も同様に「返ってきているが捨てている」—— 011 で TTree に書くこと
  (SPEC §6.3「emit 後に遅延到着したフラグメントも必ず TTree に書く」)。
- 遅延到着のログは **run 中 1 回だけ**(ホットパスで per-frame のログ整形をしない)。以降は
  `late_fragments` カウンタが可視化を担う。

### v1.3 対応結果(レビュー指示 = イベント内フラグメント順の確定、2026-08-13)

**確定した仕様**: SPEC v1.3 §6.3「**イベント内のフラグメント順は (cobo, asad) 昇順で
決定的にする**」(到着順は run 毎に揺れるため、§12-4 の 2 ソースビルド一致・TTree 比較が
順序で偽陰性にならないように。遅延到着分 = LateFragment はこの限りではない)。

**実装**(`eb_core.hpp` の `EventBuilder::emit()` の 1 箇所のみ。poll / flush は同じ
`emit()` を通るので両経路に効く):

- `std::stable_sort` で `OwnedFragment::key()`(= `(cobo, asad)`)昇順に整列してから返す。
  期待集合は高々数要素なので **emit 時に 1 回並べ替える**方式を採った(feed 時の挿入整列より
  素直 —— KISS)。
- **stable_sort を選んだ理由**: 同キー(= 重複フラグメント。捨てないと決めたもの)同士の
  前後関係を勝手に作り変えないため。テストで到着順の保存を固定した。
- 整列は**期待集合の内外を問わない**(期待外の `(3,3)` も昇順の位置に入る)。

**テスト**(`test_eb_core.cpp`):

- 新規 2 本 —— `test_fragments_inside_an_event_are_sorted_by_cobo_asad`(4 ソースを昇順の逆
  1:1→1:0→0:1→0:0 で投入 → 出口は (0,0),(0,1),(1,0),(1,1)。items のマーカー語で
  「並べ替えで中身が入れ替わっていない」ことも照合)、
  `test_flush_also_sorts_fragments_by_cobo_asad`(EOS flush 経路 + incomplete イベントでも同じ規則)。
- 書き換え 3 本 —— `test_two_sources_build_one_complete_event`(**到着を cobo1→cobo0 の逆順に
  変更**して整列を実際に判別するテストにした)、`test_unexpected_fragment_is_counted_and_still_emitted`
  (先頭到着の (3,3) が末尾に来る)、`test_duplicate_fragment_is_counted_and_still_emitted`
  (2 個目の重複に `event_time=0xdead` を入れて**同キーの到着順保存**を照合)。
- **red 確認済み**: 整列実装前に 17 CHECK が fail(すべて順序関連)→ 実装後 green。

**再実行(2026-08-13、環境は上記と同じ)**:

| コマンド | 結果 |
|---|---|
| `make -C tools/root_sink clean && make -C tools/root_sink test` | **68 / 71 / 175 / SKIP**(test_eb_core が 152 → **175 passed / 0 failed**、19 関数) |
| `make -C tools/root_sink`(本体) | 警告ゼロ |
| `tools/root_sink/run_conformance.sh` | exit 0(**68 / 71 / 175 / 49** すべて 0 failed) |
| `TPCDAQ_ROOT_SINK_BIN=... cargo test --test root_sink_intake` | **7 passed / 0 failed**(JSON カウンタは v1.3 前と同値: シャッフル `events_complete=6 incomplete=0 late=0 items=84`、欠け `complete=3 incomplete=1 items=37`。整列はイベント**内**の話なのでカウンタは不変 = 期待どおり) |
| `cargo test --no-fail-fast`(env 未設定) | **215 passed / 0 failed** |
| `cargo clippy --all-targets -- -D warnings` | 警告ゼロ |
| `rustfmt --edition 2021 --check tests/root_sink_intake.rs` | 差分なし |

**この対応での逸脱**: なし(Rust 側は無変更 —— 整列は C++ 側の emit 内で完結し、
JSON カウンタに現れないため統合テストの変更は不要)。

### 最終レビュー(2026-08-13 Fable)

- **判定: 受理(COMPLETED)**。逸脱 1–5(+v1.3 対応)すべて受理。特に:
  - `pop_for` の `PopResult` 3 値化は**移植元 delila-rs のバグ発見**(bool 版は timeout/closed の
    区別不能で「push+close と競合すると最後の 1 通を落とす」)— delila-rs 側への申し送りとして記録。
  - `flush()` の complete 維持は実装の解釈が正(発注書の字面を訂正済み)。
  - 保留された「イベント内フラグメント順」は SPEC v1.3 §6.3((cobo,asad) 昇順・stable)として
    確定 → v1.3 対応で実装・red→green 確認済み。
- レビュー側で独立再検証: make test(68/71/**175**/49 全 0 failed)、run_conformance.sh 単独 green、
  cargo test 215 passed / 0 failed。eb_core の stable_sort 実装・コメントを直接確認。
- 011 への接続点: `absorb()` が BuiltEvent を破棄している箇所(コメント明記済み)が TTree 書き出しの
  差し込み位置。
