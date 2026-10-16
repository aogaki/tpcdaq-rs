# 158 — decoder: frameType 別カウンタ + compact(frameType 2)のサイズ検査

**Status: COMPLETED**(2026-10-16 起票 Fable。156 の前に。Sonnet)

## 0. 根拠
ZS は frameType 1 で来る(実測: run 133 31.8 kB、165–169 65 kB。compact は 278.8 kB 固定)。`src/decode.rs:290-306` の compact 経路は full readout の順(ch 0..67 → 次の bucket)を前提にカーソルで (ch, bucket) を復元するので、FW が compact + ZS を出したら**黙って誤配置**する。今は frameType 別のカウンタが無く run_stop から確かめられない。

## 仕様
- decoder のカウンタに `frames_type1` / `frames_type2`(run 毎、rearm で戻さない — `acquisition_restarts` と同じ扱い)→ metrics → `run_stop.counters`(`src/logbook.rs` の Counters、`src/controller/status.rs`、UI の表示は数値 2 つ)。
- frameType 2 で items 数が `68 × nAget × 512` 系の期待と合わないフレーム → `compact_size_mismatch` に計数 + WARN 1 回(フレームは落とさない、Error にもしない)。
- テスト: `decode.rs` の既存 frameType 1 / 2 フィクスチャでカウンタ、サイズ不一致の合成 compact で `compact_size_mismatch`、logbook / status の往復。`cargo fmt && cargo clippy --tests -- -D warnings && cargo test`。

## 結果
- 2026-10-16: Sonnet `1415c23`。`src/decode.rs` に `frames_type1 / frames_type2 / compact_size_mismatch`(compact は item 数が 68 × 512 × nAget(1..4) の整数倍でなければ不一致、0 件も不一致。frameType 1 は対象外)、`src/decoder.rs` で metrics に 3 キー + run 毎 1 回の WARN(rearm で戻らない = Decoder 内のカウンタ)、`src/logbook.rs` Counters に `Option<u64>` × 3(`serde(default)`、旧行は null)、`status.rs` は decoder 複数台の合計、UI は 3 行表示。テスト: `frames_are_counted_per_frame_type`、`compact_size_mismatch_counts_frames_off_the_full_readout_size`、`frame_type1_never_counts_as_compact_size_mismatch`、`frame_type_counters_and_compact_mismatch_reach_metrics_and_survive_rearm`、`a_full_size_compact_frame_is_not_a_mismatch`、`run_stop_counters_carry_the_frame_type_counters`、logbook golden 3 本、UI spec。`cargo test` 1087 / 0、clippy 0、`ng build` OK、`ng test` 395 passed。
- 逸脱: implementer は赤を取らずに実装後にテストを書いた(TDD 手順違反、申告あり)。テスト内容は発注の各項目を覆っているので受け入れ、次回の発注書で「赤を先に」を明記する。実機の値(536,444,928 / 3,852 = 139,264 = 68 × 512 × 4)で期待式が合うことを確認。

**Status: COMPLETED**(2026-10-16、Fable 照合)
