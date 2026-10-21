#!/usr/bin/env bash
#
# TODO/169 の受け入れ試験。
#   test_par_analyzeUVW.sh <raw.root> <workdir>
# 入力は workdir にシンボリックリンクを張って渡す(入力と同じ場所に出力を作らない)。
# 圧力 100 は同値性の試験用の任意値。
set -uo pipefail

[[ $# -eq 2 ]] || { echo "usage: $0 <raw.root> <workdir>" >&2; exit 2; }
[[ -f "$1" ]] || { echo "ERROR: $1 not found" >&2; exit 2; }
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
raw_src="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
W="$(mkdir -p "$2" && cd "$2" && pwd)"
repo="$(cd "${here}/.." && pwd)"
if [[ -z "${SDKROOT:-}" ]] && command -v xcrun >/dev/null 2>&1; then
  SDKROOT="$(xcrun --sdk macosx --show-sdk-path)"
  export SDKROOT
fi
cd "${here}"

pass=0
fail=0
report() {  # name rc
  if [[ "$2" == 0 ]]; then echo "PASS $1"; pass=$((pass + 1)); else echo "FAIL $1"; fail=$((fail + 1)); fi
}
rootq() { root -l -b -q "$@" 2>&1; }

link="${W}/t_raw.root"
ln -sf "${raw_src}" "${link}"
rm -f "${W}"/t_uvw.root "${W}"/par_j*.root "${W}"/r_*.root

# ---- T1 同値性 -------------------------------------------------------------
rootq "analyzeUVW.C(\"${link}\", 100)" >"${W}/serial.log"
rc=0; [[ -s "${W}/t_uvw.root" ]] || rc=1
report "T1a serial run produced output" "${rc}"
for j in 4 3; do
  "${here}/par_analyzeUVW.sh" -p 100 -j "${j}" -o "${W}/par_j${j}.root" "${link}" >"${W}/par_j${j}.log" 2>&1
  report "T1b par_analyzeUVW.sh -j ${j} exit 0" "$?"
  out="$(rootq "compareUVW.C(\"${W}/t_uvw.root\", \"${W}/par_j${j}.root\")")"
  rc=$?
  echo "    ${out##*$'\n'}"
  report "T1 equivalence serial == parallel -j ${j}" "${rc}"
done
n_entries="$(rootq "-e" "TFile f(\"${W}/t_uvw.root\"); printf(\"UVWN=%lld\\n\", ((TTree*)f.Get(\"uvw\"))->GetEntries());" | sed -n 's/^UVWN=//p')"
echo "    T1 entries = ${n_entries}"

# ---- T2 範囲の端 -----------------------------------------------------------
t2() {  # name first n
  rm -f "${W}/r_edge.root"
  local o
  o="$(rootq "analyzeUVW.C(\"${link}\", 100, $2, $3, \"${W}/r_edge.root\")")"
  local rc=0
  grep -q "ERROR" <<<"${o}" || rc=1
  [[ ! -e "${W}/r_edge.root" ]] || rc=1
  report "T2 $1 -> error, no output" "${rc}"
}
t2 "first >= N" 99999999999 10
t2 "n == 0" 0 0
t2 "first < 0" -1 10

# ---- T3 必須引数 -----------------------------------------------------------
rm -f "${W}/r_noarg.root"
o="$(rootq "analyzeUVW.C(\"${link}\")")"; rc=1
grep -qi "error\|too few" <<<"${o}" && rc=0
[[ ! -e "${W}/t_uvw.root.new" ]] || rc=1
report "T3a analyzeUVW.C without pressure fails" "${rc}"
o="$(rootq "stoppedAlphas.C(\"${W}/none_tracks.root\")")"; rc=1
grep -qi "error\|too few" <<<"${o}" && rc=0
report "T3b stoppedAlphas.C with missing args fails" "${rc}"
"${here}/par_analyzeUVW.sh" "${link}" >/dev/null 2>&1; rc=$?
[[ "${rc}" == 2 ]] && rc=0 || rc=1
report "T3c par_analyzeUVW.sh without -p exits 2" "${rc}"
"${repo}/tools/scan/metrics_for_run.sh" /nonexistent "${W}/m_out" >/dev/null 2>&1; rc=$?
[[ "${rc}" == 2 ]] && rc=0 || rc=1
report "T3d metrics_for_run.sh without --pressure exits 2" "${rc}"

# ---- T4 入力の保護 ---------------------------------------------------------
foo="${W}/foo.root"
ln -sf "${raw_src}" "${foo}"
before="$(shasum "${foo}" | cut -d' ' -f1)"
o="$(rootq "analyzeUVW.C(\"${foo}\", 100)")"; rc=1
grep -q "ERROR" <<<"${o}" && rc=0
report "T4a analyzeUVW.C refuses input without _raw.root" "${rc}"
"${here}/par_analyzeUVW.sh" -p 100 "${foo}" >"${W}/t4.log" 2>&1; rc=$?
[[ "${rc}" != 0 ]] && grep -q "ERROR" "${W}/t4.log" && rc=0 || rc=1
report "T4b par_analyzeUVW.sh refuses input without _raw.root" "${rc}"
after="$(shasum "${foo}" | cut -d' ' -f1)"
[[ "${before}" == "${after}" ]]; report "T4c input shasum unchanged" "$?"

echo "TOTAL: ${pass} passed, ${fail} failed"
[[ "${fail}" == 0 ]]
