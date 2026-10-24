#!/usr/bin/env bash
#
# TODO/169 の受け入れ試験。
#   test_par_analyzeUVW.sh <raw.root> <workdir>
# 入力は workdir にシンボリックリンクを張って渡す(入力と同じ場所に出力を作らない)。
# 圧力 100 / 25 MHz / 1200 V は試験用の任意値(TODO/178: 3 つとも必須引数)。
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
rootq "analyzeUVW.C(\"${link}\", 100, 25, 1200)" >"${W}/serial.log"
rc=0; [[ -s "${W}/t_uvw.root" ]] || rc=1
report "T1a serial run produced output" "${rc}"
for j in 4 3; do
  "${here}/par_analyzeUVW.sh" -p 100 -s 25 -d 1200 -j "${j}" -o "${W}/par_j${j}.root" "${link}" >"${W}/par_j${j}.log" 2>&1
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
  o="$(rootq "analyzeUVW.C(\"${link}\", 100, 25, 1200, $2, $3, \"${W}/r_edge.root\")")"
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
o="$(rootq "analyzeUVW.C(\"${foo}\", 100, 25, 1200)")"; rc=1
grep -q "ERROR" <<<"${o}" && rc=0
report "T4a analyzeUVW.C refuses input without _raw.root" "${rc}"
"${here}/par_analyzeUVW.sh" -p 100 -s 25 -d 1200 "${foo}" >"${W}/t4.log" 2>&1; rc=$?
[[ "${rc}" != 0 ]] && grep -q "ERROR" "${W}/t4.log" && rc=0 || rc=1
report "T4b par_analyzeUVW.sh refuses input without _raw.root" "${rc}"
after="$(shasum "${foo}" | cut -d' ' -f1)"
[[ "${before}" == "${after}" ]]; report "T4c input shasum unchanged" "$?"

# ---- T5 引数の検査 ---------------------------------------------------------
t5() {  # name samplingMHz driftHvVolt
  rm -f "${W}/r_arg.root"
  local o
  o="$(rootq "analyzeUVW.C(\"${link}\", 100, $2, $3, 0, -1, \"${W}/r_arg.root\")")"
  local rc=0
  grep -q "ERROR" <<<"${o}" || rc=1
  [[ ! -e "${W}/r_arg.root" ]] || rc=1
  report "T5 $1 -> error, no output" "${rc}"
}
t5 "samplingMHz = 30" 30 1200
t5 "driftHvVolt = 0" 25 0
t5 "driftHvVolt < 0" 25 -5
"${here}/par_analyzeUVW.sh" -p 100 -d 1200 "${link}" >"${W}/t5.log" 2>&1; rc=$?
[[ "${rc}" == 2 ]] && grep -q "ERROR: -s " "${W}/t5.log" && rc=0 || rc=1
report "T5 par_analyzeUVW.sh without -s exits 2" "${rc}"
"${here}/par_analyzeUVW.sh" -p 100 -s 25 "${link}" >"${W}/t5.log" 2>&1; rc=$?
[[ "${rc}" == 2 ]] && grep -q "ERROR: -d " "${W}/t5.log" && rc=0 || rc=1
report "T5 par_analyzeUVW.sh without -d exits 2" "${rc}"
"${repo}/tools/scan/metrics_for_run.sh" --pressure 100 --drift-hv 1200 /nonexistent "${W}/m_out" >"${W}/t5.log" 2>&1; rc=$?
[[ "${rc}" == 2 ]] && grep -q "ERROR: --sampling " "${W}/t5.log" && rc=0 || rc=1
report "T5 metrics_for_run.sh without --sampling exits 2" "${rc}"
"${repo}/tools/scan/metrics_for_run.sh" --pressure 100 --sampling 25 /nonexistent "${W}/m_out" >"${W}/t5.log" 2>&1; rc=$?
[[ "${rc}" == 2 ]] && grep -q "ERROR: --drift-hv " "${W}/t5.log" && rc=0 || rc=1
report "T5 metrics_for_run.sh without --drift-hv exits 2" "${rc}"

# ---- T6 run の定数が uvw / tracks に記録される ------------------------------
# 手計算: v = 0.7233 * (1200/20/100) = 0.43398 cm/us; mm/cell = v / 25 MHz * 10
#         = 0.173592; z_window = 512 * 0.173592 = 88.879104 mm
cat >"${W}/getpar.C" <<'EOF2'
#include <TFile.h>
#include <TParameter.h>
void getpar(const char *f, const char *name)
{
  TFile in(f);
  auto *p = (TParameter<double> *)in.Get(name);
  printf("PAR=%.9g\n", p ? p->GetVal() : -1e30);
}
EOF2
getp() {  # file name
  rootq "${W}/getpar.C(\"$1\", \"$2\")" | sed -n 's/^PAR=//p'
}
rm -f "${W}/t_tracks.root"
rootq "makeTracks.C(\"${W}/t_uvw.root\")" >"${W}/mk.log"
[[ -s "${W}/t_tracks.root" ]]; report "T6 makeTracks produced tracks" "$?"
for f in uvw tracks; do
  for kv in pressure_mbar=100 sampling_MHz=25 drift_hv_V=1200 drift_v_cm_us=0.43398 mm_per_cell=0.173592 z_window_mm=88.879104; do
    k="${kv%%=*}"; e="${kv##*=}"
    v="$(getp "${W}/t_${f}.root" "${k}")"
    awk -v v="${v}" -v e="${e}" 'BEGIN{d=v-e; if(d<0)d=-d; exit !(d<1e-6)}'
    report "T6 ${f}: ${k} = ${e} (got ${v})" "$?"
  done
done
v="$(getp "${W}/par_j4.root" mm_per_cell)"
awk -v v="${v}" 'BEGIN{d=v-0.173592; if(d<0)d=-d; exit !(d<1e-6)}'
report "T6 hadd-merged parallel output keeps mm_per_cell (got ${v})" "$?"
# 12.5 MHz / 1600 V / 100 mbar: v = 0.7233*0.8 = 0.57864; mm/cell = 0.57864/12.5*10 = 0.462912
rm -f "${W}/r_alt.root"
rootq "analyzeUVW.C(\"${link}\", 100, 12.5, 1600, 0, 20, \"${W}/r_alt.root\")" >"${W}/alt.log"
v="$(getp "${W}/r_alt.root" mm_per_cell)"
awk -v v="${v}" 'BEGIN{d=v-0.462912; if(d<0)d=-d; exit !(d<1e-6)}'
report "T6 12.5 MHz / 1600 V: mm_per_cell = 0.462912 (got ${v})" "$?"

# ---- T7 stoppedAlphas の z 窓 ----------------------------------------------
# z_window_mm の無い tracks: 旧 tracks から TParameter を持たないコピーを作る
cat >"${W}/strip.C" <<'EOF2'
#include <TFile.h>
#include <TTree.h>
void strip(const char *src, const char *dst)
{
  TFile in(src);
  TFile out(dst, "RECREATE");
  TTree *t = ((TTree *)in.Get("tracks"))->CloneTree();
  t->Write();
  out.Close();
}
EOF2
rootq "${W}/strip.C(\"${W}/t_tracks.root\", \"${W}/old_tracks.root\")" >/dev/null
o="$(rootq "stoppedAlphas.C(\"${W}/old_tracks.root\", -370, 100)")"; rc=1
grep -q "ERROR.*z_window_mm" <<<"${o}" && grep -q "Re-run analyzeUVW" <<<"${o}" && rc=0
report "T7a stoppedAlphas on tracks without z_window_mm fails with message" "${rc}"
o="$(rootq "stoppedAlphas.C(\"${W}/t_tracks.root\", -370, 100)")"; rc=1
grep -q "ERROR" <<<"${o}" || rc=0
grep -q "WARNING: pressure argument" <<<"${o}" && rc=1
report "T7b stoppedAlphas on tracks with z_window_mm runs (no error, no pressure warning)" "${rc}"
o="$(rootq "stoppedAlphas.C(\"${W}/t_tracks.root\", -370, 101)")"; rc=1
grep -q "WARNING: pressure argument" <<<"${o}" && rc=0
report "T7c pressure argument 1 mbar off the tracks file -> WARNING" "${rc}"

echo "TOTAL: ${pass} passed, ${fail} failed"
[[ "${fail}" == 0 ]]
