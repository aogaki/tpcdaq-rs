#!/usr/bin/env bash
#
# SPEC §10.4 — WS プロトコルのクロス言語適合性テスト。
#
#   ① Rust の本番エンコーダでサンプルストリームを生成(ws_proto_sample、026)
#   ② TS の本番デコーダを import する vitest で分解・デコードし既知値と照合
#   ③ 一時ファイルを掃除
#
# フィクスチャは**コミットしない**(毎回再生成 = 陳腐化が構造的に起きない)。
# 成功で終了コード 0、どこかで失敗すれば非 0。
set -euo pipefail

ui_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${ui_dir}/.." && pwd)"

tmp_dir="$(mktemp -d)"
cleanup() { rm -rf "${tmp_dir}"; }
trap cleanup EXIT

sample="${tmp_dir}/ws_sample.bin"

echo "[1/2] generating the sample stream with the Rust production encoder"
cargo run --quiet --manifest-path "${repo_root}/Cargo.toml" --bin ws_proto_sample -- --out "${sample}"
ls -l "${sample}"

echo "[2/2] decoding it with the TypeScript production decoder"
cd "${ui_dir}"
TPCDAQ_WS_SAMPLE="${sample}" npx ng test
