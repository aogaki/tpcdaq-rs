# 126 — monitor.rs と controller.rs をサブモジュールに分ける(挙動不変のリファクタ)

**Status: IN PROGRESS**(2026-10-13 起票 Opus。ユーザー「リファクタリングもここらですべきかも」→ 範囲を絞って合意:
大きいファイルの機械的な分割と、再構成の monitor への移設(127)。全体の作り直しはしない)

## 目的
- 複数飛跡の再構成(後続)は monitor を触る。その前に、1 ファイルに詰まった区画を分けて、読む範囲を小さくする
  (125 の implementer は 1 人 30〜38 万トークンを大きなファイルの読み込みに使った)。
- **挙動・公開 API・テストの中身は一切変えない**。移動と `use` の調整だけ。

## 現状(本体 = `#[cfg(test)]` より前)
- `src/controller.rs` 6,840 行(本体 3,936 / テスト 2,904)。区画: 定数・起動パラメタ・ecc-bridge 応答・Transport・
  シーケンサ(:423〜、純ロジック)・スナップショット・操作権・run 状態・Controller・run_start 材料・REST ハンドラ(:2028〜)・
  パラメータ REST(:2931〜)・HV/LV 電源(:3342〜)・ログ投稿 PULL(:3733〜)・エントリポイント(:3834〜)。
- `src/monitor.rs` 3,077 行(本体 1,972 / テスト 1,105)。区画: ワイヤ(payload・decode_message・GapTracker、:60〜330)・
  `pub mod ws`(:333〜)・表示変換(DisplayConverter・ItemWords、:505〜798)・JSON(meta / stripGeometry / status / run /
  subscribe、:799〜966)・WS サンプル(:967〜1060)・パラメタ・カウンタ・Hub(:1061〜1480)・SUB スレッド(:1482〜1673)・
  WS / HTTP(axum、last-event、:1674〜1876)・`run_monitor`(:1877〜)。

## やること
### レーン A — monitor(Sonnet)
- `src/monitor.rs` → `src/monitor/mod.rs` + 区画ごとのファイル(目安: `wire.rs` / `ws.rs`(既存の `pub mod ws` をそのまま)/
  `display.rs` / `json.rs` / `sample.rs` / `hub.rs` / `sub.rs` / `http.rs`)。`run_monitor` と `MonitorParams` 等は `mod.rs`。
- **`crate::monitor::X` で今参照できるものは全部、同じパスで参照できるままにする**(`pub use` で再公開)。
  bin(`src/bin/*.rs`)・統合テスト(`tests/*.rs`)・他モジュールは**変更しない**(変更が要るならそれは API を変えたということ — 戻す)。
- テストは各ファイルの `#[cfg(test)] mod tests` に、対象のコードと一緒に移す。テスト関数の中身・名前は変えない。
  分け方に迷うテストは `mod.rs` 側に残してよい。
### レーン B — controller(Sonnet)
- 同じやり方で `src/controller.rs` → `src/controller/mod.rs` + 区画ごと(目安: `sequencer.rs` / `transport.rs` / `rest.rs` /
  `params_api.rs` / `power.rs` / `logpost.rs`。起動パラメタ・Controller・操作権・run 状態は `mod.rs` か `state.rs`)。
- 公開 API・bin・統合テストは変えない(同上)。
- 2 レーンは別ファイルしか触らない(`src/lib.rs` の `pub mod monitor;` / `pub mod controller;` は変えずに済むはず)。

## 受け入れ(各レーン)
- `cargo fmt && cargo clippy --tests -- -D warnings && cargo test` が green で、**テスト数が着手前と同じ**
  (main 0cb660b 時点 1018 passed / 0 failed / 1 ignored)。テスト名の集合が着手前と同じ(`cargo test -- --list` の差分が空)。
- `git diff --stat` で、`src/bin/` と `tests/` に変更が無い。
- 移した後の各ファイルが 1,000 行を大きく超えない(テスト込み。超える場合は理由を報告)。
- 報告: 新しいファイル構成(各ファイルの行数)、`cargo test -- --list` の差分が空であることの証拠、ゲート結果、
  **実行できなかった操作**(無ければ「なし」)。

## 結果
(未記入)
