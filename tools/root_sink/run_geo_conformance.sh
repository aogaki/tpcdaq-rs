#!/bin/sh
# run_geo_conformance.sh — ジオメトリパーサの二重実装一致テスト(SPEC §4.5、TODO/018)。
#
#   ./run_geo_conformance.sh
#
#   1. Rust 側 CLI(`cargo run --bin geometry_dump`)で TSV ダンプを生成
#   2. C++ 側(`test_geo <file.dat>` ダンプモード、geo.hpp 本番パーサ)で同じ入力を
#      ダンプし、`cmp` で**バイト一致**を機械検証する
#
# 合成フィクスチャ(tests/fixtures/geometry_{mini_reduced,2cobo_fake,legacy}.dat、TODO/002
# で作成済み)は必ず比較する。実ジオメトリ .dat は env 変数
# (TPCDAQ_REAL_GEOMETRY_MINI / TPCDAQ_REAL_GEOMETRY_ELITPC — src/geometry.rs の
# tests/geometry_real_regression.rs / TODO/archive/002 と同じ変数名)が設定されていれば
# 追加で比較し、無ければ SKIP を明示する(実ジオメトリはリポに入れない)。

set -eu

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/tpcdaq_geo_conformance.XXXXXX")
trap 'rm -rf "$tmp"' EXIT INT TERM

status=0

compare_one() {
  label="$1"
  datfile="$2"
  rust_out="$tmp/rust.tsv"
  cpp_out="$tmp/cpp.tsv"

  (cd "$repo" && cargo run --quiet --bin geometry_dump -- "$datfile") >"$rust_out"
  "$here/test_geo" "$datfile" >"$cpp_out"

  if cmp -s "$rust_out" "$cpp_out"; then
    lines=$(wc -l <"$rust_out" | tr -d ' ')
    bytes=$(wc -c <"$rust_out" | tr -d ' ')
    echo "run_geo_conformance: OK   $label ($lines lines, $bytes bytes)"
  else
    echo "run_geo_conformance: FAIL $label — Rust/C++ dump mismatch"
    diff -u "$rust_out" "$cpp_out" | head -20
    status=1
  fi
}

echo "run_geo_conformance: building test_geo (C++)"
make -C "$here" test_geo >/dev/null

echo "run_geo_conformance: building geometry_dump (Rust)"
(cd "$repo" && cargo build --quiet --bin geometry_dump)

compare_one "mini (synthetic, NEW format)" "$repo/tests/fixtures/geometry_mini_reduced.dat"
compare_one "2-CoBo (synthetic, NEW format)" "$repo/tests/fixtures/geometry_2cobo_fake.dat"
compare_one "legacy (synthetic, LEGACY format)" "$repo/tests/fixtures/geometry_legacy.dat"

if [ -n "${TPCDAQ_REAL_GEOMETRY_MINI:-}" ]; then
  compare_one "mini (real, TPCDAQ_REAL_GEOMETRY_MINI)" "$TPCDAQ_REAL_GEOMETRY_MINI"
else
  echo "run_geo_conformance: SKIP real mini geometry (TPCDAQ_REAL_GEOMETRY_MINI not set)"
fi

if [ -n "${TPCDAQ_REAL_GEOMETRY_ELITPC:-}" ]; then
  compare_one "ELITPC (real, TPCDAQ_REAL_GEOMETRY_ELITPC)" "$TPCDAQ_REAL_GEOMETRY_ELITPC"
else
  echo "run_geo_conformance: SKIP real ELITPC geometry (TPCDAQ_REAL_GEOMETRY_ELITPC not set)"
fi

exit $status
