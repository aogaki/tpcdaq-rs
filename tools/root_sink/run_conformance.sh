#!/bin/sh
# run_conformance.sh — クロス言語適合性テスト(SPEC §10.4 方式)を 1 本で回す。
#
#   ./run_conformance.sh
#
#   1. Rust の**本番エンコーダ**(tpcdaq::msg::write_golden_stream)で golden fixture を生成
#   2. C++ の**本番デコーダ**(tpc_wire.hpp)で読み、既知値と機械照合(make test)
#
# フィクスチャはコミットしない(毎回再生成 = 陳腐化が構造的に起きない —— C++ 版 ctest
# FIXTURES 方式)。終了コードは make test のもの。

set -eu

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
golden="${TMPDIR:-/tmp}/tpcdaq_golden_stream_$$.bin"

cleanup() { rm -f "$golden"; }
trap cleanup EXIT INT TERM

echo "run_conformance: generating golden fixture -> $golden"
(cd "$repo" && cargo run --quiet --example write_golden_stream -- "$golden") >/dev/null

echo "run_conformance: building and running C++ tests"
make -C "$here" test GOLDEN="$golden"
