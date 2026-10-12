# 114 — 113 / 108 の残課題: 歩き戻しで EOS を待つ / root_sink の初回 late ログを run 単位に

**Status: COMPLETED**(2026-10-12 起票・同日完了 Opus。ユーザー「ランは止まってるので残った課題を解決してください」。2 レーン並行、Fable 一括レビュー 2 回)

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

## 結果(2026-10-12)
- **B**(Sonnet `9f223b2` → merge `15b1863`):
  - `rs_core.hpp` に `first_late_of_run(now, at_open)`、`root_sink.cxx` はそれで判定する。
  - `test_rs_core` に `test_first_late_of_run_is_per_run` を追加(red → green)。
  - Mac の `make -j test`: tpc_wire 68 / **rs_core 78** / eb_core 175 / geo 426 / monitor_hist 202 / monitor_pub 92、failed 0。test-root 170 / 99 / 88 green(Sonnet)。
- **A**(Opus `c9a9c1b` + `bc41682` → merge `36b5976`、追補 `3cd5ef2` → merge `b18c109`):
  - `walk_components_back_to_idle` を書き直した。状態調べ → 全員 Running なら停止経路の 2 段(`wait_for_eos_or_quiesce` → 強制 EOS → `wait_for_eos`)→ 下流 Stop → Reset。
  - 一部だけ Running なら待たない(跡は `eos: not awaited`)。
  - `walk_back_step` は失敗時に、その時点の実状態を名乗る。
  - テスト: 単体 +6(順序 / slept 800 ms / 自然 EOS で強制なし / 届かない 5600 ms / 一部だけ / 文言が Configured)、統合 +1(controller だけを作り直して start → 跡 `eos: closed`)。
  - ゲート(main `b18c109`、115 込み): clippy 0、**cargo test 893 passed / 0 failed / 1 ignored**。
- **実機(ELITPC、α 4.5 Hz)**:

  | 試験 | ビルド | 結果 |
  |---|---|---|
  | crash(run 58 → 59) | 113 | start は通った。run 58 は EOS で閉じた(低レートでは競争に勝つ)。跡に eos 行なし |
  | crash(run 60 → 61) | 114A | 跡 `eos: closed` + 全員 `Running->Configured->Idle`、graw 63 = ROOT 63、inprogress なし。run 61 の close 行は `events_complete=43`(108 の per-run 修正の実機確認) |
  | crash(run 62 → 63) | 114 追補 + 115 | 跡 `eos: closed`(`forced_eos:false` = breakup の EOF で第一段が自然に閉じた)。`run_stop 62` = `abort:controller-restart`、ok false、eos_closed true、frames 89 = ROOT 89、run_start 63 より前 |
  | **SIGSTOP**(run 64 → 65) | 114 追補 + 115 | root-sink 停止で decoder の batches_out が 988 で詰まった → controller kill -9 → start: 歩き戻しは `eos: did not propagate`、`run_stop 64` は eos_closed:false で書かれた。**ただし run 65 の Arm が decoder の bind 失敗で落ちた → 116**。graw 1219 フレーム(連番、無傷)/ ROOT 1041 で inprogress のまま(EOS 未着、可視) |

- Fable 一括レビュー: 承認 + 追補 1 件(静止確認)→ 追補は実装済み。SPEC v1.39 / v1.40 の確定は 116 と合わせて Fable に出す。
- **最終ゲート**(main `99e7704`、115 / 116 込み、Mac、2026-10-12): fmt OK / clippy 警告 0 / **cargo test 895 passed / 0 failed / 1 ignored**。
  ignored は `tests/root_sink_monitor_pub.rs:1031`(P3 E2E の素材で、cargo test では走らない既存の 1 件)。root_sink `make -j test` は上記のとおり。
- Fable 2 回目の一括レビュー(114 / 115 / 116 + SPEC): **SPEC v1.39 / v1.40 確定**(3 箇所差し替え)、archive 可。
  未解決として残すのは、レビュー (b) の狭い経路(controller が自分の停止の第二段で落ちた場合は `eos: not awaited`)。入れない判断で、記載済み。
