# 013 — decoder の 2 ソース飢餓修正(fair-queue ロックイン)

**Status: COMPLETED**
**仕様**: SPEC v1.4 §1.4(過負荷時もロスレス系は背圧で守る — 片ソース飢餓は §6.3 ビルダの
全イベント incomplete 化を誘発)、§2.3
**依存**: 009(COMPLETED)。再現手段 = 012 の E2E-B(dev ビルド)
**発注先想定**: implementer/Opus(受信ループ設計に触る)

## 症状(012 で発見・切り分け済み — archive/012_p2_e2e.md の「発見した不具合」参照)

decoder が入力レートに追いつけないとき、**上流 2 ソースの片方を run 丸ごと飢餓させる**:
- receiver ×2 は同期(first batch / EOS とも一致)。ZMQ 素の PULL は正常に fair-queue
  (テスト PULL 直結で `0,1,0,1,…`、消費側 20 ms sleep でも不変)
- decoder の `batches_in` だけが `{"0":26,"1":1}` → `{"0":72,"1":1}` と偏る
- 結果: 片方の 108 フラグメントが 1.1〜1.4 s 遅延 → `build_timeout_ms=1000` 超過 →
  **events_complete=0 / incomplete=108**(ビルダは SPEC §6.3 どおりの正しい動作)
- dev ビルド 5/5 再現、release 3/3 非再現(= decoder が追いつくと出ない)。飢餓の向きは run 毎に揺れる
- **影響**: mini(1 CoBo)は無関係。ELITPC(2 CoBo)で decoder が一瞬でも遅れると全イベント
  incomplete。§12-5(24 h)/§12-6(全速)を 2 ソースで回すと必ず踏む

仮説(012、未検証): `src/decoder.rs` の `POLL_TIMEOUT_MS = 2` の tight loop が libzmq の
fair-queue パイプ非活性化と噛み合ってロックインする。

## やること

1. **原因の確定**(仮説の検証から。決め打ちで直さない): decoder の受信ループを最小再現に切り出し、
   poll タイムアウト・recv 回数/poll・パイプ活性化の関係を観測。結果を結果節に記録。
2. **修正**: 方向性は「**1 回の poll wake で EAGAIN まで全部 drain する**(受信予算を
   per-wake にしない)+ poll タイムアウトを tight loop でなくす(receiver/root_sink と同じ
   100 ms 級)」を第一候補とするが、1 の観測結果が別を指せばそちら(その場合は理由を記録)。
   ロスレス背圧(PULL 停止 → 上流 HWM)と Reset 打ち切り(009)の既存動作は不変であること。
3. **回帰**:
   - `tests/p2_e2e.rs` E2E-B の `cfg!(debug_assertions)` skip を**外す**(dev でも green になる
     ことが修正の受け入れ)。
   - 可能なら env 不要の統合再現(合成 2 ソース + decoder を人工的に遅くする手段が要るなら
     テスト専用フックの追加を検討 — 発注書に無い設計になるので、必要なら停止して報告)。
4. **ついで(同一ファイル所有のため本ユニットで)**: decoder に `cobo_mismatch` カウンタを追加 —
   Batch.source_id と Fragment.cobo(ヘッダ値)の不一致を数えて可視化(DataLinkSet /
   --cobo-id 誤設定の早期検出。P2 批判的レビュー指摘。Error にはしない — カウント + warn 初回)。

## 受け入れ

- dev / release 両ビルドで E2E-B green(skip なし)。E2E-A・既存 222 テスト・make test /
  test-root / run_conformance.sh 無影響。
- 実 .graw E2E 4 本(receiver / graw_writer / decoder / pipeline)のオラクル維持。
- 原因の観測記録が結果節にあること(「直ったから良い」で終わらせない)。
- ファイル所有権: src/decoder.rs、tests/decoder_integration.rs、tests/p2_e2e.rs(skip 除去)。

## 結果

**実行環境**: macOS 26.5.2(Darwin 25.5.0)/ arm64 / rustc 1.97.1 / **libzmq 4.3.4**(`zmq::version()` 実測)/
ROOT 6.36.10 @ `/opt/ROOT` / dev・release 両プロファイル。**2026-08-13**。

### 1. 原因の確定(仮説は**棄却**、別機構を計測で特定)

発注書の仮説「`POLL_TIMEOUT_MS = 2` の tight loop が噛んでいる」は **反証された**。
真因は **poll のタイムアウト値ではなく、`recv` と `recv` の間で `process_commands()` が
走るかどうか**だった。

#### 機構(libzmq 4.3.4 のソースと実測の両方で確認)

1. PULL の fair-queue(`fq_t`)は「自分の番で空だったパイプ」を**非活性化**してラウンドロビンから外す。
2. 外れたパイプの**再活性化は `activate_read` コマンド**としてソケットのメールボックスに届き、
   **`socket_base_t::process_commands()` でしか取り込まれない**。
3. `socket_base_t::recv()` が `process_commands()` を呼ぶのは
   **(a) 成功 100 回に 1 度**(`inbound_poll_rate = 100`)か **(b) `xrecv` が EAGAIN を返したとき**だけ。
4. decoder が入力に追いつけないときは**残ったパイプに常にデータがある** → (b) が起きない
   → 外れた上流は **100 通ぶん**戻ってこられない。

これが 012 の観測 `batches_in = {"0":72,"1":1}`(t=1.06 s)と**数値まで一致**する
(1 バッチ ≈ 1 フレーム ≈ 10 ms/frame × 100 = 約 1 s = 観測された 1.1〜1.4 s 遅延)。

#### 最小再現(2 PUSH → 1 PULL、実 .graw の 1 フレーム相当 278,784 B、108 通/ソース)

`MAX_RUN` = 同一ソースが連続で消費された最大通数(飢餓の直接の尺度)/ `MAX_STARVE` = 片ソースが 1 通も来ない最長時間。

| # | 受信ループ構造 | interval=10 ms / work=12 ms | interval=5 ms / work=10 ms |
|---|---|---|---|
| A | **現行**(`rcvtimeo=2`、1 wake = 1 recv) | MAX_RUN=**101** / 1.214 s | MAX_RUN=**101** / 1.012 s |
| B | **発注書の第一候補**: poll(100 ms) + EAGAIN まで drain | MAX_RUN=**100** / 1.201 s | MAX_RUN=**100** / 1.001 s |
| C | poll(100 ms) を **recv 毎**に挟む | MAX_RUN=**2** / 0.036 s | MAX_RUN=**2** / 0.021 s |
| D | `rcvtimeo=2` のまま `getsockopt(ZMQ_EVENTS)` を recv 毎に挟む | MAX_RUN=**2** / 0.025 s | MAX_RUN=**2** / 0.022 s |
| E | poll + **高々 8 通**だけ drain | MAX_RUN=**9** / 0.109 s | MAX_RUN=**9** / 0.091 s |
| F | poll + **高々 64 通**だけ drain | MAX_RUN=**65** / 0.781 s | MAX_RUN=**65** / 0.651 s |
| 対照 | A のまま **work=0**(消費が追いつく) | MAX_RUN=**2** / 0.013 s | — |

この表が示したこと(**発注書の第一候補を採らなかった根拠**):

- **B(1 wake で EAGAIN まで drain)では直らない** — 消費が供給より遅い間 drain ループは EAGAIN に
  到達しないので、poll に戻れず `process_commands()` が走らない。**A とほぼ同じ数字**になる。
- **`MAX_RUN` = バースト上限 + 1 がきれいに出る**(E: 8→9、F: 64→65、無制限: →101)。
  無制限のときの 101 は libzmq 内部の `inbound_poll_rate = 100` そのもの。
- **D が C と同じに直る** = poll という API が効いているのではなく、**`process_commands()` の頻度**だけが
  効いている(D は 2 ms の tight loop を保ったまま公平になる = **発注書の仮説の直接の反証**)。
- 対照行 = 「消費が追いつけば起きない」= 012 の「release 3/3 非再現」と整合。

補足: 同じループでも**メッセージが 256 B のときは再現しない**(初回プローブ、interval 5 ms / work 10 ms で
MAX_RUN=1)。パイプが一瞬空になる過渡が要るので、**配送の粒度が大きい(= 実データの)ときだけ踏む**。
実機条件でしか出ない罠だったということで、012 が実 .graw で踏んだのは幸運。

#### 決定的な確認(速度勝負に依存しない形で機構を分離)

「一度外れたパイプが戻るまでに相手を何通消費させるか」を**タイミングに依存せず**組み立てて測った
(テスト `a_pipe_that_left_the_fair_queue_only_returns_when_commands_are_processed`):

| ループ構造 | 外れたパイプが戻るまでに相手を食う通数 |
|---|---|
| `recv` だけ | **96** |
| `recv` の前に `poll` | **1** |

96 = `inbound_poll_rate`(100)− 準備で消費した 5 通 + 1。**dev 3/3・release 3/3 で完全に同じ値**
(乱れゼロ)。デコード速度もマシン負荷も関係しないので、機構の同定はこれで確定とみなす。

#### 参考: `Decoder::decode` の実測スループット(飢餓条件の成立範囲を押さえるため)

| フレーム | dev | release |
|---|---|---|
| 139,264 item(557,144 B、実 .graw 1 フレーム相当) | **14.284 ms** | **0.124 ms** |
| 34,816 item(139,352 B) | **3.085 ms** | **0.026 ms** |

release は dev の **約 115 倍速い**。これが「release では飢餓条件そのものが成立しない」理由であり、
**env 不要の統合再現を release でも意味のあるものにするには decoder を人工的に遅くするフックが要る**
という結論の根拠(→ 未解決点 1)。

### 2. 修正

**`src/decoder.rs` の受信ループを「recv の前に必ず poll する」に変える。** 受信予算は設けない
(読めるなら読み続ける)— 上表 C/D/E/F が示すとおり、公平性を決めているのは「何通で区切るか」ではなく
**「recv と recv の間で `process_commands()` が走るか」**だけだからである。

| 変更 | 前 | 後 | 理由 |
|---|---|---|---|
| 受信 | `pull.set_rcvtimeo(2)` + `recv_bytes(0)` | `zmq::poll([POLLIN], wait)` → `recv_bytes(DONTWAIT)` | `zmq_poll` は各ソケットの `ZMQ_EVENTS` を読み、その中で `process_commands()` が走る = 外れたパイプが必ず戻る |
| 待ち時間 | 固定 2 ms(tight loop) | **次のバッチ期限まで**を **100 ms** で頭打ち | receiver の `sender_loop`(`wake.saturating_duration_since(now)`)と同じ流儀。発注書の「100 ms 級」を満たしつつ SPEC §2.3 の「10 ms 経過で close」を鈍らせない |
| ms への丸め | — | **切り上げ** | 切り捨てると期限手前 1 ms 未満で `poll(0)` を回す busy loop になる(バッチ毎に CPU を焼く)。1 ms 遅く閉じるほうが害がない |
| `EINTR` | — | 失敗扱いにせず次の周回で待ち直す | シグナル中断は失敗ではない(warn を撒かない) |

新設 `RunDecoder::next_deadline()` は溜まっているバッチの close 期限を返すだけの読み取り
(コアは相変わらず ZMQ も時計も持たない)。

**不変であることを確認した既存動作**:

- **ロスレス背圧**: 送出は従来どおり `send_lossless` のブロッキング。PULL を止めれば上流 HWM で
  背圧がかかる経路に変更なし(`zmq_helper` は 1 行も触っていない)。
- **Reset 打ち切り(009)**: `abandon` フラグ・`batches_abandoned` / `eos_abandoned` の経路は無変更。
  `reset_abandons_the_blocked_send_and_counts_what_was_dropped` が green。
- **アイドル時 Heartbeat**: 待ち上限 100 ms に対し周期 1 s なので粒度は十分。
  `configure_arm_start_stop_sequence_succeeds_and_idle_sends_heartbeats` が green。
- 副次的な改善として、**アイドル時の目覚ましが 500 回/s → 10 回/s** になった(tight loop の解消)。

### 3. `cobo_mismatch` カウンタ(発注書 4)

`Batch.source_id`(receiver の `--cobo-id`)と `Fragment.cobo`(GRAW ヘッダ実値)の不一致を
**フレーム単位で**数え、`metrics_json` の `"cobo_mismatch"` に載せる。**初回だけ warn**、以降はカウントのみ
(`logged_unsupported` / `logged_malformed` と同じ流儀)。**Error にはしない**(データは正しく、下流は
`Fragment.cobo` で識別するので run は続けてよい)。Fragment にならないフレーム(制御フレーム)は数えない。

### 4. 回帰

`tests/p2_e2e.rs` の E2E-B から **`cfg!(debug_assertions)` skip を除去**(skip を戻すことが修正の退行に
なるよう、除去した箇所に理由をコメントで残した)。

#### E2E-B(`cargo test [--release] --test p2_e2e`、env 3 つ)— **dev / release とも skip なしで green**

| | dev pass1 | dev pass2 | release pass1 | release pass2 |
|---|---|---|---|---|
| 所要 | 2.919 s | 2.976 s | 3.061 s | 2.932 s |
| `events_complete` / `incomplete` / `late_fragments` | **108 / 0 / 0** | **108 / 0 / 0** | **108 / 0 / 0** | **108 / 0 / 0** |
| `fragments` / `items` | 216 / 30,081,024 | 同左 | 同左 | 同左 |
| `entries_written` / ROOT ファイル数 | **216** / 1 | **216** / 1 | **216** / 1 | **216** / 1 |
| `unexpected` / `duplicate` / `pending_events` | 0 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| root_sink `batches` | 200 | 199 | 94 | 81 |
| receiver 毎受信フレーム数 | 109 / 109 | 109 / 109 | 109 / 109 | 109 / 109 |

**修正前の dev はここが `events_complete=0 / incomplete=108 / late_fragments=108` だった**(012、5/5 再現)。
決定性チェック(`compare_gdataframe --strict-order`)も dev / release とも
`216 エントリ / 58752 チャンネル / 30081024 サンプル一致 / cobo_entries=0:108,1:108 / event_idx_nondecreasing=yes`。
`--build-timeout-ms` は**既定 1000 ms のまま**(タイムアウトを緩めて誤魔化していない)。
リプレイのペーシングも 012 のまま `--rate-mbps 224`。

#### テスト数

| コマンド | 結果 |
|---|---|
| `cargo test`(env 未設定、**dev**) | **226 passed / 0 failed**(012 時点 222 + 新規 4) |
| `cargo test --release`(env 未設定) | **226 passed / 0 failed** |
| `cargo test --test p2_e2e`(env 3 つ、**dev**) | **2 passed / 0 failed**(**skip なし** — 以前は E2E-B が skip) |
| `cargo test --release --test p2_e2e`(env 3 つ) | **2 passed / 0 failed** |
| `cargo clippy --all-targets -- -D warnings` | 警告ゼロ |
| `cargo clippy --tests -- -D warnings` | 警告ゼロ |
| `cargo fmt --all -- --check` | 差分なし |
| `make -C tools/root_sink test` | **68 / 71 / 175 / SKIP** すべて 0 failed |
| `make -C tools/root_sink test-root` | **test_recorder: 163 passed / 0 failed** |
| `tools/root_sink/run_conformance.sh` | exit 0(**68 / 71 / 175 / 49** すべて 0 failed) |

#### 実 .graw E2E 4 系統(オラクル維持、dev / release 両方で実行)

| テスト | dev | release |
|---|---|---|
| `receiver_real_graw` | 1 passed | 1 passed |
| `graw_writer_real_graw` | 1 passed | 1 passed |
| `decoder_real_graw` | 1 passed | 1 passed |
| `decoder_pipeline_real_graw` | 1 passed | 1 passed |

E2E-A(§12-3 の実機オラクル照合)も dev / release で green:
`compare_gdataframe` が **108 エントリ / 29,376 チャンネル / 15,040,512 サンプル一致**(許容差は
012 と同じ `fDataSource` / `fHitPatterns` の明示 2 件のみ)。
オラクル `~/TPC/CoBo_2025-09-01T08_51_06.203_0000.root` は実行前後で
`sha256 = f844cc79df27f27b239e9bd7f2058afb03f04bacb5cbaca33c206fb65b76ecaa` / 63,747,101 B / mtime `Sep 1 2025`
が**不変**(READ のみ)。

#### 新規テスト(4 本)

| テスト | 場所 | 何を固定するか |
|---|---|---|
| `a_decoder_that_cannot_keep_up_still_never_starves_one_of_two_sources` | `tests/decoder_integration.rs` | **env 不要の統合再現**。34,816 item/frame の**本物のデコード**を 2 ソースから 1 ms 間隔で浴びせ、出力 Fragment 列の同一 CoBo 連続数 ≤ 8 を要求。**修正前は「片ソースが 60 通連続」で red**(= run 丸ごと飢餓、012 の症状そのもの) |
| `a_pipe_that_left_the_fair_queue_only_returns_when_commands_are_processed` | `tests/decoder_integration.rs` | libzmq の機構そのもの(96 通 vs 1 通)。**決定的**に組み立てるので dev/release・並列負荷を問わず同じ数字。`zmq_helper` の「IMMEDIATE なし PUSH は送れたふりをする」対比テストと同じ流儀 |
| `a_cobo_header_that_disagrees_with_the_batch_source_is_counted_but_is_not_an_error` | `src/decoder.rs` | `cobo_mismatch` をフレーム単位で数える / Error にしない / Fragment を捨てない / metrics に出る |
| `matching_cobo_headers_never_count_a_mismatch` | `src/decoder.rs` | 一致時は 0、制御フレームは数えない |

既存 `metrics_json_carries_every_counter_of_the_ticket` に `"cobo_mismatch"` を追加。

### 変更ファイル

| ファイル | 区分 | 中身 |
|---|---|---|
| `src/decoder.rs` | 改変 | 受信ループを poll → recv(DONTWAIT)へ / `POLL_TIMEOUT_MS` 2 → 100(待ちは期限から算出・切り上げ)/ `RunDecoder::next_deadline()` 新設 / `cobo_mismatch` カウンタ + 初回 warn + metrics / 単体テスト 2 本追加 |
| `tests/decoder_integration.rs` | 改変 | 回帰テスト 2 本追加((g) 節) |
| `tests/p2_e2e.rs` | 改変 | E2E-B の `cfg!(debug_assertions)` skip を除去(理由コメントに置換)。**それ以外は 1 行も触っていない** |

発注書の所有権リストどおり、**`src/receiver.rs` / `src/graw_writer.rs` / `src/zmq_helper.rs` / `tools/` /
`Cargo.toml` / `docs/` には一切触っていない**。ZMQ テストのポートはすべて 0(ephemeral)。

### 発注書からの逸脱

1. **修正の方向が発注書の第一候補と違う**(発注書 §2 が明示的に許している「1 の観測結果が別を指せば
   そちら + 理由を記録」に該当)。第一候補「1 回の poll wake で EAGAIN まで全部 drain」は
   **計測で効かないことを確認済み**(上表 B: MAX_RUN=100、A とほぼ同じ)。採ったのは
   「recv の前に必ず poll」。ただし**「受信予算を per-wake にしない」という発注書の意図は満たしている**
   (読めるなら読み続ける。区切っているのは通数ではなく poll 1 回ぶんのコスト)。
   「poll タイムアウトを 100 ms 級に」も満たした(ただし実待ち時間は期限から算出 — 固定 100 ms にすると
   SPEC §2.3 の 10 ms close が壊れるため)。
2. **ms への切り上げ丸めを入れた**(発注書に記載なし)。切り捨てだと期限手前で `poll(0)` の busy loop に
   なるという実装上の落とし穴を塞ぐためで、挙動としては「バッチが最大 1 ms 遅く閉じうる」だけ。
3. **統合再現テストを 2 本にした**(発注書 §3 は「作れるならなお良い」)。1 本は本物の decoder を使う
   統合再現(dev で discriminating)、もう 1 本は libzmq の機構を決定的に固定するもの。
   後者を足したのは、前者が **release では飢餓条件そのものを作れない**(未解決点 1)ため、
   profile 非依存の固定が別途要ると判断したから。

### 未解決点・持ち帰り

1. **env 不要の統合再現は release では「弱いテスト」にしかならない**。release の `decode` は 0.124 ms/frame
   = 実質ネットワークが律速で、**decoder を人工的に遅くするフックなしには飢餓条件を作れない**。
   発注書 §3 の指示どおりフックは**実装していない**。フックを入れるか否かは設計判断なので親に返す
   (入れるなら `DecoderParams` にテスト専用の遅延を足す形になり、production の型に試験専用フィールドが
   増える。KISS 的には現状の 2 本立てで足りているとも言える)。
2. **`Stop` / `Reset` の反応が最大 100 ms 遅くなった**(従来 2 ms)。receiver / root_sink と同じ粒度で、
   既存テストはすべて green。即応が要るなら poll 集合に inproc の停止合図ソケットを足す形になるが、
   発注書に無い設計なので**やっていない**。
3. 本ユニットの修正は decoder のみ。**root_sink 側の PULL も同じ構造なら同じ罠を踏む**
   (`tools/root_sink` は所有権外のため未調査)。上流が decoder 1 本(単一 PUSH = パイプ 1 本)なので
   現時点では fair-queue 自体が効かず無害だが、将来 root_sink が複数上流を受けるなら要確認。

### 最終レビュー(2026-08-13 Fable)

- **判定: 受理(COMPLETED)**。仮説を鵜呑みにせず**測定で棄却**し(D 行: tight loop のまま
  `get_events()` を挟むだけで直る = 仮説の直接反証)、真因を libzmq の
  `inbound_poll_rate = 100`(recv 成功 100 回に 1 度しか `process_commands()` が走らない)まで
  特定した過程は本フェーズ最良の仕事。96 vs 1 のタイミング非依存テストで機構を固定した点も含め、
  発注書が要求した「直ったから良いで終わらせない」を完全に満たす。
- レビュー側で独立再検証: cargo test 226 passed / 0 failed、**dev ビルドの E2E-A/B が skip なしで
  2 passed**(修正前は dev 5/5 失敗の条件)。
- 未解決点の裁定: ①人工遅延フックは**入れない**(KISS — 機構固定テスト + dev E2E-B の 2 本立てで
  十分。production 型に試験専用フィールドを増やさない)。②Stop/Reset 最大 100 ms は
  receiver/root_sink と同粒度で**許容**(DAQ のコマンドは人間スケール)。③root_sink の単一上流構造は
  SPEC §2.3 の設計そのものなので現状無害 — 複数上流化する設計変更時の必須確認事項として
  P2 レビュー文書に追記。
