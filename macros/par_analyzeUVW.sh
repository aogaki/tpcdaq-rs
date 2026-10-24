#!/usr/bin/env bash
#
# TODO/169 — analyzeUVW.C をイベント範囲で N 本に分けて並列実行し、hadd で合併する。
#
#   par_analyzeUVW.sh -p <pressure_mbar> -s <samplingMHz> -d <driftHvVolt> [-j N] [-o <out_uvw.root>] [--tracks] <run_raw.root>
#
#   -p        必須。圧力 [mbar](ドリフト速度 = z の換算。既定値なし)
#   -s        必須。サンプリング周波数 [MHz](100 / 50 / 25 / 12.5。z の換算。既定値なし)
#   -d        必須。ドリフト電圧 [V](既定値なし)
#   -j N      並列数(既定 = コア数 - 2、最小 1。エントリ数を超えれば エントリ数)
#   -o OUT    出力(既定 = 入力の `_raw.root` → `_uvw.root`。入力と同名になる場合はエラー)
#   --tracks  合併後に makeTracks.C を 1 本で実行
#
# 各分割は macros(このスクリプトのあるディレクトリ)で
# `root -l -b -q 'analyzeUVW.C(raw, p, s, d, first, n, part_KK.root)'` として動かす。
# 1 本でも失敗したら tmp を残して exit 1(合併出力は書かない)。
# 終了コード: 0 = 成功、1 = 実行時の失敗、2 = 使い方の誤り。
set -euo pipefail

usage() {
  echo "usage: $0 -p <pressure_mbar> -s <samplingMHz> -d <driftHvVolt> [-j N] [-o <out_uvw.root>] [--tracks] <run_raw.root>" >&2
  exit 2
}

pressure=""
sampling=""
drifthv=""
njobs=""
out=""
tracks=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    -p) [[ $# -ge 2 ]] || usage; pressure="$2"; shift 2 ;;
    -s) [[ $# -ge 2 ]] || usage; sampling="$2"; shift 2 ;;
    -d) [[ $# -ge 2 ]] || usage; drifthv="$2"; shift 2 ;;
    -j) [[ $# -ge 2 ]] || usage; njobs="$2"; shift 2 ;;
    -o) [[ $# -ge 2 ]] || usage; out="$2"; shift 2 ;;
    --tracks) tracks=1; shift ;;
    -h | --help) usage ;;
    -*) echo "unknown option: $1" >&2; usage ;;
    *) break ;;
  esac
done
[[ $# -eq 1 ]] || usage
if [[ -z "${pressure}" ]]; then
  echo "ERROR: -p <pressure_mbar> is required (no default)" >&2
  usage
fi
if [[ -z "${sampling}" ]]; then
  echo "ERROR: -s <samplingMHz> is required (no default)" >&2
  usage
fi
if [[ -z "${drifthv}" ]]; then
  echo "ERROR: -d <driftHvVolt> is required (no default)" >&2
  usage
fi

[[ -f "$1" ]] || { echo "ERROR: $1 not found" >&2; exit 1; }
raw="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
if [[ -z "${out}" ]]; then
  if [[ "${raw}" == *_raw.root ]]; then out="${raw%_raw.root}_uvw.root"; else out="${raw}"; fi
fi
out="$(mkdir -p "$(dirname "${out}")" && cd "$(dirname "${out}")" && pwd)/$(basename "${out}")"
if [[ "${out}" == "${raw}" || "${out}" -ef "${raw}" ]]; then
  echo "ERROR: output ${out} is the same file as the input; refusing to overwrite the input" >&2
  echo "       (the input needs '_raw.root' in its name, or pass -o)" >&2
  exit 1
fi

if [[ -z "${njobs}" ]]; then
  if [[ "$(uname)" == Darwin ]]; then ncpu="$(sysctl -n hw.ncpu)"; else ncpu="$(nproc)"; fi
  njobs=$((ncpu - 2))
fi
[[ "${njobs}" =~ ^[0-9]+$ ]] || { echo "ERROR: -j needs a positive integer" >&2; exit 2; }
((njobs >= 1)) || njobs=1

command -v hadd >/dev/null 2>&1 || { echo "ERROR: hadd not found in PATH" >&2; exit 1; }
root_bin="$(command -v root || echo /opt/ROOT/bin/root)"
if [[ -z "${SDKROOT:-}" ]] && command -v xcrun >/dev/null 2>&1; then
  SDKROOT="$(xcrun --sdk macosx --show-sdk-path)"
  export SDKROOT
fi
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${here}"  # analyzeUVW.C reads channel_map.csv from the current directory

t0=${SECONDS}
total="$("${root_bin}" -l -b -q -e "TFile f(\"${raw}\"); TTree *t=(TTree*)f.Get(\"raw\"); printf(\"NENTRIES=%lld\\n\", t ? t->GetEntries() : -1LL);" |
  sed -n 's/^NENTRIES=//p' || true)"
if [[ -z "${total}" || "${total}" -le 0 ]]; then
  echo "ERROR: cannot read tree 'raw' (or it is empty) in ${raw}" >&2
  exit 1
fi
((njobs <= total)) || njobs=${total}

echo "par_analyzeUVW: pressure=${pressure} mbar  sampling=${sampling} MHz  drift_hv=${drifthv} V  N=${njobs}  entries=${total}"
echo "  input : ${raw}"
echo "  output: ${out}"

tmp="$(mktemp -d "${TMPDIR:-/tmp}/par_analyzeUVW.XXXXXX")"
base=$((total / njobs))
rem=$((total % njobs))
pids=()
firsts=()
counts=()
first=0
for ((k = 0; k < njobs; ++k)); do
  cnt=$((base + (k < rem ? 1 : 0)))
  firsts+=("${first}")
  counts+=("${cnt}")
  part="$(printf '%s/part_%02d' "${tmp}" "${k}")"
  "${root_bin}" -l -b -q "analyzeUVW.C(\"${raw}\", ${pressure}, ${sampling}, ${drifthv}, ${first}, ${cnt}, \"${part}.root\")" \
    >"${part}.log" 2>&1 &
  pids+=($!)
  first=$((first + cnt))
done

failed=0
for ((k = 0; k < njobs; ++k)); do
  part="$(printf '%s/part_%02d' "${tmp}" "${k}")"
  if ! wait "${pids[k]}"; then
    echo "ERROR: part $(printf '%02d' "${k}") (first=${firsts[k]} n=${counts[k]}) exited non-zero; log ${part}.log" >&2
    failed=1
  elif [[ ! -s "${part}.root" ]]; then
    echo "ERROR: part $(printf '%02d' "${k}") (first=${firsts[k]} n=${counts[k]}) produced no output; log ${part}.log" >&2
    failed=1
  fi
done
if ((failed)); then
  echo "FAILED: no merged output written; parts kept in ${tmp}" >&2
  exit 1
fi

parts=()
for ((k = 0; k < njobs; ++k)); do parts+=("$(printf '%s/part_%02d.root' "${tmp}" "${k}")"); done
hadd -f "${out}" "${parts[@]}" >"${tmp}/hadd.log" 2>&1 ||
  { echo "ERROR: hadd failed; see ${tmp}/hadd.log (parts kept)" >&2; exit 1; }

# verify: merged utree entries == sum of the parts == the whole range
sum=0
for ((k = 0; k < njobs; ++k)); do
  part="$(printf '%s/part_%02d' "${tmp}" "${k}")"
  c="$(sed -n 's/^processed entries \[[0-9]*, [0-9]*) = \([0-9]*\) of .*/\1/p' "${part}.log" | tail -1)"
  if [[ "${c}" != "${counts[k]}" ]]; then
    echo "ERROR: part ${k} processed '${c}' entries, expected ${counts[k]}; log ${part}.log (parts kept)" >&2
    exit 1
  fi
  sum=$((sum + c))
done
merged="$("${root_bin}" -l -b -q -e "TFile f(\"${out}\"); TTree *t=(TTree*)f.Get(\"uvw\"); printf(\"NENTRIES=%lld\\n\", t ? t->GetEntries() : -1LL);" |
  sed -n 's/^NENTRIES=//p' || true)"
if [[ "${merged}" != "${total}" || "${sum}" != "${total}" ]]; then
  echo "ERROR: entry mismatch: merged=${merged} parts-sum=${sum} expected=${total} (parts kept in ${tmp})" >&2
  exit 1
fi
echo "merged ${njobs} parts -> ${out} (${merged} entries, verified)"

if ((tracks)); then
  echo "makeTracks.C -> ${out}"
  "${root_bin}" -l -b -q "makeTracks.C(\"${out}\")"
fi

rm -rf "${tmp}"
echo "par_analyzeUVW: done in $((SECONDS - t0)) s"
