# 158 — decoder: frameType 別カウンタ + compact(frameType 2)のサイズ検査

**Status: READY**(2026-10-16 起票 Fable。156 の前に。Sonnet)

## 0. 根拠
ZS は frameType 1 で来る(実測: run 133 31.8 kB、165–169 65 kB。compact は 278.8 kB 固定)。`src/decode.rs:290-306` の compact 経路は full readout の順(ch 0..67 → 次の bucket)を前提にカーソルで (ch, bucket) を復元するので、FW が compact + ZS を出したら**黙って誤配置**する。今は frameType 別のカウンタが無く run_stop から確かめられない。

## 仕様
- decoder のカウンタに `frames_type1` / `frames_type2`(run 毎、rearm で戻さない — `acquisition_restarts` と同じ扱い)→ metrics → `run_stop.counters`(`src/logbook.rs` の Counters、`src/controller/status.rs`、UI の表示は数値 2 つ)。
- frameType 2 で items 数が `68 × nAget × 512` 系の期待と合わないフレーム → `compact_size_mismatch` に計数 + WARN 1 回(フレームは落とさない、Error にもしない)。
- テスト: `decode.rs` の既存 frameType 1 / 2 フィクスチャでカウンタ、サイズ不一致の合成 compact で `compact_size_mismatch`、logbook / status の往復。`cargo fmt && cargo clippy --tests -- -D warnings && cargo test`。

## 結果
