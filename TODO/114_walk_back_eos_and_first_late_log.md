# 114 — 113 / 108 の残課題: 歩き戻しで EOS を待つ / root_sink の初回 late ログを run 単位に

**Status: IN PROGRESS**(2026-10-12 起票 Opus。ユーザー「ランは止まってるので残った課題を解決してください」。2 レーン並行)

## A. 歩き戻しの Stop の後、EOS の伝播を待つ(Rust、Opus レーン)

### 事実(コード読み、2026-10-12)
- 113 の `walk_components_back_to_idle`(`src/controller.rs`)は、コンポーネントを 1 つずつ見て、`Running` なら `Stop` → **すぐ** `Reset` を送る。
- しかし `Stop` は非破壊で、未送分と EOS の送り出しは裏のスレッドが非同期に行う(`receiver::do_stop` / `decoder::do_stop`)。
  `Reset` は送出待ちを**打ち切る**(`abandon`)。したがって Stop 直後の Reset では EOS が流れ切る前に打ち切られ、
  `messages_abandoned` / `eos_abandoned` で可視化されるだけになる。
  113 の裁定(Fable)は「Running は Stop(EOS を流す)→ Reset」で、EOS を流すことが目的だった。
- decoder の Stop は残バッチを flush するが、**自分の EOS は出さない**。decoder の EOS は全 receiver の EOS を受けたときにだけ出る。
  したがって EOS を root-sink まで通すには、停止経路と同じく「receiver を Stop → EOS 到達を待つ → 下流を Stop」の順が要る。
- 典型例: **run 中に controller だけが落ちて再起動した**場合。新しい controller は Idle で、コンポーネントは全員 Running のまま、
  ECC も Running。113 以前は start が `Configure refused in state Running` で失敗した。
- **実機ベースライン(2026-10-12 08:42、ELITPC、113 のビルド、`~/crash_test.sh baseline-113`)**: α run 58 の 15 s 目に controller を kill -9 → 再起動
  (相 Idle、3 コンポーネントと ECC は Running のまま)→ start は通った(run 59)。audit `component_walk_back` は全員 `Running->Configured->Idle`、
  ECC は `status->Running, stop->Ready, breakup->Prepared, reset->Described, reset->Idle`。**run 58 は root-sink で EOS により閉じた**
  (`run 58 closed (… events_complete=68 …)` が `run 59 opened` より前、raw / PEventTPC とも 68 件で finalize、inprogress なし)。
  → **低レートでは EOS が Stop → Reset の REQ 往復より先に抜けて、競争に勝つ**(起票時の「閉じられないまま残る」は試験前の推定で、低レートでは誤り)。
  待ちが要るのは、送出待ちが溜まっているとき(高レートで下流が詰まり、PUSH が HWM で止まっている)。このときは Reset が EOS ごと打ち切る。
  この修正はロスレス規約上の正しさ(競争に頼らない)のためのもので、低レートの実運用では今も実害が出ていない。

### 仕様(Opus が実装、Fable の 113 裁定の実現。SPEC §1.3 v1.38 の 1 文を差し替え)
1. まず全コンポーネントの状態を(上流から)`GetStatus` で調べる。不達の扱いは今と同じ(stage `walk-back` で失敗)。
2. **全コンポーネントが `Running`**(チェーンが無傷)のときは、停止経路の強制 EOS と同じ手順で前の run を閉じる:
   - receiver に `Stop` を送る。
   - 既存の `wait_for_eos` で待つ。上限は `eos_timeout`。
   - 残り(decoder → graw-writer)に `Stop` を送る。
   - 待ちの結果を跡に 1 行残す。例: `"eos: closed"` / `"eos: did not propagate within 5000 ms"`。どちらの場合も start は止めない。
     届かなかったときは `warn!` を出す。Reset による打ち切りはコンポーネントの metrics で可視化される。
   - ECC は start_run がこれより先に歩き戻しているので、zCoBo は既に止まっている(停止経路の ecc stop → receiver Stop と同じ順)。
3. **一部だけ `Running`**(チェーンがどこかで既に Reset 済み)のときは、EOS が端まで届かず観測もできない。
   今と同じく Stop → Reset とし、`warn!`(「EOS は待たない — チェーンが途中で切れている」)と跡に 1 行を残す。
4. 最後に Idle でない全員へ `Reset` を送る(上流から)。コンポーネントごとの跡(`"decoder: Running->Configured->Idle"`)の形式は変えない。
5. `Stop` / `Reset` が断られた、または届かなかった場合の扱いは今と同じ(stage `walk-back` で失敗、`Stop` が通らなければ `Reset` に進まない)。

### テスト(TDD、red を先に)
- 単体(MockTransport): 全員 Running の場合、コマンド列が `receiver Stop → (decoder/graw-writer への GetStatus = EOS 待ち) → decoder Stop → graw-writer Stop → Reset × 全員 → Configure` になること。
  跡に `eos: closed` が残ること。`eos_after_polls` で「数回目で EOS 到達」を模擬する。
- 単体: 全員 Running だが EOS が来ない場合、跡に `eos: did not propagate…` が残り、start は通ること。
- 単体: 一部だけ Running(decoder だけ)の場合、EOS 待ちの GetStatus が出ないこと、跡に「待たない」旨が残ること。
- 統合(`tests/controller_integration.rs`、本物の状態機械を使うハーネス): run を開始する → controller だけを作り直して(相 Idle、コンポーネントは Running)start → 通ること、
  audit `component_walk_back` に `eos: closed` と各コンポーネントの `Running->Configured->Idle` が残ること。この形が作りにくければ報告する。

## B. root_sink の「最初の late fragment」ログを run 単位に(C++、Sonnet レーン)
- `tools/root_sink/root_sink.cxx` の consume 内 `if (c.late_fragments == 0)` は通算カウンタで判定している。
  1 つのプロセスで 2 本目以降の run では、最初の late を一度もログに出さない(108 で `run_at_open` を入れた後も同じ)。
- 直し方: その run で最初の 1 件のとき(`c.late_fragments == c.run_at_open.late_fragments`)に出す。
  判定を `rs_core.hpp` の純関数に切り出し、`test_rs_core.cpp` に「run 1 で late が出た後、run 2 の最初の late でも真」を 1 本足す(red → green)。

## ゲート
`cargo fmt && cargo clippy --tests -- -D warnings && cargo test` / `cd tools/root_sink && make -j test`(可能なら `make -j test-root`)。
実機(ELITPC): controller だけを kill -9 → 再起動 → start で、前の run が root-sink で閉じること(`run N closed`、`run_inprogress` が残らないこと)。

## Fable 一括レビュー(2026-10-12、スポット Fable、読み取り専用)
- **判定: 承認(main `36b5976`)+ 追補 1 件**。
  - (a) EOS が届かなくても start を止めない判断は、v1.12 の終端条項を start 側に当てはめたもので、ロスレス規約の範囲内。
    root-sink は次の run の最初の Data で exit 6 になる(正しい検出)。このことを SPEC に明記する。
  - (b) 「一部だけ Running なら待たない」には、取りこぼしの経路が 1 つある。controller が自分の停止の第二段の最中に落ちた場合
    (receiver が Configured のまま EOS を送出中)。狭く、`messages_abandoned` で可視なので、今回は入れない(KISS)。
  - (c) **silent の穴**: `receiver::drain_task` は Stop で即 break し、カーネル受信バッファの未読分を数えずに捨てる。
    停止経路はそのために `wait_for_eos_or_quiesce` を Stop の前に置いているが、114 の歩き戻しは静止を確かめずに Stop している。
    実害が出ていないのは、breakup で FIN が来るという書かれていないタイミングのおかげ。
- **追補**: 全員 Running の経路を停止経路の 2 段そのものにする。
  `wait_for_eos_or_quiesce` → 閉じていなければ全 receiver に `Stop` → `wait_for_eos` → 下流に `Stop`。跡の形式は変えない。
  SPEC v1.39 の確定文は Fable 作で、追補が green になってからコミットする。
- **実機の追加試験**: run 中に root-sink を `kill -STOP` → controller を kill -9 → 再起動 → start。
  期待は、跡に `eos: did not propagate`、`eos_abandoned` / `messages_abandoned` が出ること、`kill -CONT` 後に root-sink が exit 6 になること。
- **前の run の `run_stop`** → 115 として別チケット(Fable 裁定済み)。
