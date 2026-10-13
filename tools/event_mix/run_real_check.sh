#!/usr/bin/env bash
#
# TODO/128 — 実データ確認(env-gated・記録のみ)。
# 実 raw ツリーから N 組を作り、本物の macros(analyzeUVW.C → makeTracks.C)を元データと混合データに
# それぞれ掛け、mixReport.C で「単一飛跡フィットが元のどちらに寄るか」を出す。
#
#   tools/event_mix/run_real_check.sh RAW.root WORKDIR [N=100] [SEED=7] [MAX_SHIFT=0] [MACROS_DIR]
#
#   RAW.root    graw2root.C / root-sink --raw-tree の raw ツリー(実データ — ローカルのみ)
#   WORKDIR     作業・出力ディレクトリ(実データ由来 — **リポの外に置く**)
#   MACROS_DIR  analyzeUVW.C / makeTracks.C / channel_map.csv のある場所(既定 = リポの macros/。
#               channel_map.csv は .gitignore 済みなので worktree では主チェックアウトの macros/ を渡す)
#
# macros/ は書き換えない(WORKDIR にコピーして実行 — tools/macro_oracle/run_macro_oracle.sh と同じ流儀)。
set -euo pipefail

if [[ $# -lt 2 || $# -gt 6 ]]; then
  echo "usage: $0 RAW.root WORKDIR [N] [SEED] [MAX_SHIFT] [MACROS_DIR]" >&2
  exit 2
fi
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${here}/../.." && pwd)"
raw="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
work="$(mkdir -p "$2" && cd "$2" && pwd)"
n="${3:-100}"
seed="${4:-7}"
max_shift="${5:-0}"
macros_dir="${6:-${repo_root}/macros}"

for f in analyzeUVW.C makeTracks.C channel_map.csv; do
  [[ -f "${macros_dir}/${f}" ]] || { echo "ERROR: ${macros_dir}/${f} not found (pass MACROS_DIR)" >&2; exit 1; }
done

if [[ -z "${SDKROOT:-}" ]] && command -v xcrun >/dev/null 2>&1; then
  SDKROOT="$(xcrun --sdk macosx --show-sdk-path)"
  export SDKROOT
fi
root_bin="$(command -v root || echo /opt/ROOT/bin/root)"

make -C "${here}" event_mix >/dev/null
cp "${macros_dir}/analyzeUVW.C" "${macros_dir}/makeTracks.C" "${macros_dir}/channel_map.csv" \
  "${here}/mixReport.C" "${work}/"
# analyzeUVW.C は名前の "_raw.root" を "_uvw.root" に置き換えて出力する(名前が合わないと入力を上書き)
cp "${raw}" "${work}/src_raw.root"
"${here}/event_mix" --in "${raw}" --out "${work}/mix_raw.root" --random "${n}" --seed "${seed}" --max-shift "${max_shift}"

cd "${work}"
for base in src mix; do
  "${root_bin}" -l -b -q "analyzeUVW.C(\"${base}_raw.root\")" >"${base}_analyze.log" 2>&1
  "${root_bin}" -l -b -q "makeTracks.C(\"${base}_uvw.root\")" >"${base}_tracks.log" 2>&1
done
"${root_bin}" -l -b -q 'mixReport.C("mix_raw.root","src_tracks.root","mix_tracks.root")'
