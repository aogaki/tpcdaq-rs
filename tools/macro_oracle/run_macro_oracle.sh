#!/usr/bin/env bash
#
# TODO/125 改訂 2 レーン D — macros オラクル。
# **本物の** macros/analyzeUVW.C(100 mbar / 25 MHz / 1200 V = src/reco の定数、TODO/178 で引数必須)と makeTracks.C を一時ディレクトリにコピーして実行し、
# dumpMacroOracle.C で raw の adc・uvw の strip ヒット・tracks の全ブランチを JSON に書き出す。
# Rust の env-gated テスト(tests/reco_macro_oracle.rs — TODO/127)がこの JSON を読む。
#
#   tools/macro_oracle/run_macro_oracle.sh RAW.root OUT.json [MAX_EVENTS] [MACROS_DIR]
#
#   RAW.root    graw2root.C / root-sink --raw-tree の raw ツリー(実データ — ローカルのみ)
#   OUT.json    出力(実データ由来 — **リポの外に置く**)
#   MAX_EVENTS  先頭から何イベント書くか(既定 = 全部)
#   MACROS_DIR  analyzeUVW.C / makeTracks.C / channel_map.csv のあるディレクトリ
#               (既定 = リポの macros/。channel_map.csv は .gitignore 済みなので worktree では
#               主チェックアウトの macros/ を指す)
#
# macros/ は書き換えない(コピーして実行)。入力の RAW.root も一時ディレクトリへコピーする
# (analyzeUVW.C は名前の "_raw.root" を "_uvw.root" に置き換えて出力するので、名前が合わないと
# 入力を上書きしてしまう — コピーに "_raw.root" で終わる名前を付けて避ける)。
set -euo pipefail

if [[ $# -lt 2 || $# -gt 4 ]]; then
  echo "usage: $0 RAW.root OUT.json [MAX_EVENTS] [MACROS_DIR]" >&2
  exit 2
fi

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${here}/../.." && pwd)"
raw="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
out_dir="$(mkdir -p "$(dirname "$2")" && cd "$(dirname "$2")" && pwd)"
out="${out_dir}/$(basename "$2")"
max_events="${3:--1}"
macros_dir="${4:-${repo_root}/macros}"

for f in analyzeUVW.C makeTracks.C channel_map.csv; do
  if [[ ! -f "${macros_dir}/${f}" ]]; then
    echo "ERROR: ${macros_dir}/${f} not found (pass MACROS_DIR)" >&2
    exit 1
  fi
done

# macOS(CLT 27)では ACLiC / cling が libc++ のヘッダを見失う — macros/README.md の SDKROOT 注記。
if [[ -z "${SDKROOT:-}" ]] && command -v xcrun >/dev/null 2>&1; then
  SDKROOT="$(xcrun --sdk macosx --show-sdk-path)"
  export SDKROOT
fi
root_bin="$(command -v root || echo /opt/ROOT/bin/root)"

work="$(mktemp -d)"
cleanup() { rm -rf "${work}"; }
trap cleanup EXIT

cp "${macros_dir}/analyzeUVW.C" "${macros_dir}/makeTracks.C" "${macros_dir}/channel_map.csv" \
  "${here}/dumpMacroOracle.C" "${work}/"
cp "${raw}" "${work}/oracle_raw.root"

cd "${work}"
echo "[1/3] analyzeUVW.C (real macro, 100 mbar / 25 MHz / 1200 V = src/reco)"
"${root_bin}" -l -b -q 'analyzeUVW.C("oracle_raw.root", 100, 25, 1200)'
echo "[2/3] makeTracks.C (real macro)"
"${root_bin}" -l -b -q 'makeTracks.C("oracle_uvw.root")'
echo "[3/3] dumpMacroOracle.C -> ${out}"
"${root_bin}" -l -b -q "dumpMacroOracle.C(\"oracle_raw.root\", \"oracle_uvw.root\", \"oracle_tracks.root\", \"channel_map.csv\", \"${out}\", ${max_events})"
ls -l "${out}"
