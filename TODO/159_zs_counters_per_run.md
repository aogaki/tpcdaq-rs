# 159 — ZS カウンタを run ごとに見える経路へ(root_sink close 行 / status / run_stop)+ PEventTPC 電荷和の照合

**Status: READY**(2026-10-16 起票 Fable。156 判定 4 の残件。SPEC v1.49 §6.4 / §5.4 / §9.2 の「run ごとの経路 = 159」を埋める)
**モデル**: Opus(root_sink の C++ と Rust の controller をまたぐ。設計は本票で固定、工学判断 = RunCounts の run 単位化とキーの追加だけ)

## 背景
151 / 152 の ZS カウンタは **終了 JSON(root_sink)と停止ログ(monitor)にしか出ない**。156 の 4 run では
「表で引いたフレーム数 = events」を run ごとに確かめられず、`pevent_zs_frames_no_table` が 0 であることも
daq_down のときにしか読めない。RUNBOOK「ZS 運用」の「`*_no_table` が 0 でない = 表が読まれていない」は
run 中・run 直後に読めて初めて意味がある。

## 発注(やること)
1. **root_sink の close 行**(`tools/root_sink/rs_core.hpp` `RunCounts` / `format_run_closed`): run 単位の差分として
   `zs_frames_table` / `zs_frames_no_table` / `table_missing_channels` を末尾に足す(既存の初期化 `{a,b,c,d}` を壊さない —
   `RunCounts` の末尾に追加)。`run_counts()`(root_sink.cxx)は Recorder から 3 値を取る(`rec == nullptr` なら 0)。
   行の形: `… restart_duplicates=N zs_frames_table=N zs_frames_no_table=N table_missing_channels=N)`。
2. **root_sink の status PUB**(`tools/root_sink/monitor_pub.hpp` `Status` + シリアライザ): `sources_behind`(13 キー目)の後ろに
   **`zs_frames_table` / `zs_frames_no_table`(14・15 キー目、run 累計 = Recorder の値そのまま)**を足す。monitor は §5.3 の
   status を**そのまま**通す(`StatusPayload` に 2 フィールドを `serde(default)` で足し、`status_json` の既存キーと衝突しないこと)。
   UI の status バーには出さなくてよい(Logbook の counters 行だけ)。
3. **run_stop.counters**(`src/logbook.rs` `RunStopCounters`、`src/controller/status.rs`): `pevent_zs_frames_table` /
   `pevent_zs_frames_no_table`(`Option<u64>`、取れなければ null = 既存の `events_built` と同じ扱い)。取得元は
   controller が持つ root-sink の直近 status(monitor 経由でなく、controller が root-sink PUB を購読していないなら
   **この票では null のまま**にし、理由を結果節に書く — 経路の新設は別票)。golden JSON(logbook.rs の run_stop 例)を更新。
4. **PEventTPC の chargeMap 和の照合**(156 判定 4 の未実施分): `tools/scan/pevent_charge.C`(新、TPCReco の辞書が要る →
   ELITPC の `~/ELITPC/TPCReco` ビルドを `gSystem->Load`。Mac では skip を明示)で run の `runNNNN.root` から
   イベントごとの chargeMap 総和(正値のみ / 全部)と hit ch 数を JSON に。run 130(非 ZS)と run 170(B + ZS)で
   総和が ±5 % 以内であること(156 の run_metrics は raw ツリー側の確認なので、保存系の PEventTPC 側は未照合)。
5. **UI の Event Display の基線**(目視のみ、ユーザー): ZS run 中に 0x02 の基線が 0 でないこと。結果節に run 番号と所見。

## テスト(赤を先に)
- `tools/root_sink/test_rs_core.cxx`: `format_run_closed` の新 3 項目が run 差分であること(at_open を引く)。
- `tools/root_sink/test_monitor_pub.cxx`: status JSON に 14・15 キー目が出る、既存 13 キーの順序不変。
- `src/monitor/json.rs`: root-sink status に新キーがあれば通し、無ければ(古い root_sink)壊れない。
- `src/logbook.rs`: golden の run_stop に 2 キー追加、`serde(default)` で古い行も読める。
- `cargo fmt && cargo clippy --tests -- -D warnings && cargo test`、`make -j test`(root_sink)。

## 実行できなかった操作
(実装時に記入。無ければ「なし」)

## 結果
(実装時に記入)
