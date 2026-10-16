#!/usr/bin/env bash
# TODO/155 — zs_params.sh のテスト。 bash tools/scan/test_zs_params.sh
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
SCRIPT="$HERE/zs_params.sh"
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
FAIL=0
ok()   { echo "ok   $1"; }
fail() { echo "FAIL $1"; FAIL=$((FAIL+1)); }
check() { if [ "$2" = "$3" ]; then ok "$1"; else fail "$1 (got '$2', want '$3')"; fi; }
nonzero() { if [ "$2" -ne 0 ]; then ok "$1"; else fail "$1 (exit 0)"; fi; }

# 合成 3 ch 表(test_zs_exceptions.py と同じ。手計算: 430 / 520 / FPN 4090.2+20 → 4095)
cat > "$T/ped.json" <<'J'
{"meta":{"note":"synthetic"},"channels":[
 {"asad":0,"aget":0,"ch":3,"mean":400.0,"rms":6.0,"runs":1,"is_fpn":false},
 {"asad":1,"aget":2,"ch":40,"mean":500.4,"rms":2.0,"runs":1,"is_fpn":false},
 {"asad":3,"aget":3,"ch":11,"mean":4090.2,"rms":4.0,"runs":1,"is_fpn":true}]}
J
cat > "$T/base.json" <<'J'
{"knobs":{"peakingTime_ns":502,"gain":"240fC","zeroSuppression":{"enabled":false}},"exceptions":[]}
J
echo 'not json' > "$T/bad.json"

pyget() { python3 -c "import json,sys; d=json.load(open(sys.argv[1])); print($2)" "$1"; }

# 1) 正常系
OUT="$("$SCRIPT" --ped-json "$T/ped.json" --base "$T/base.json" --out "$T/out.json" --label 502ns 2>&1)"; rc=$?
check "exit 0" "$rc" 0
check "zs enabled" "$(pyget "$T/out.json" 'd["knobs"]["zeroSuppression"]["enabled"]')" True
check "3 exceptions" "$(pyget "$T/out.json" 'len(d["exceptions"])')" 3
check "base knobs kept" "$(pyget "$T/out.json" 'd["knobs"]["gain"]')" 240fC
case "$OUT" in *502ns*) ok "summary has label";; *) fail "summary has label";; esac
case "$OUT" in *peakingTime_ns=502*gain=240fC*) ok "summary shows base peaking/gain";; *) fail "summary shows base peaking/gain: $OUT";; esac
case "$OUT" in *"exceptions: 3"*) ok "summary exception count";; *) fail "summary exception count: $OUT";; esac
case "$OUT" in *"min 430"*"median 520"*"max 4095"*) ok "summary min/median/max";; *) fail "summary min/median/max: $OUT";; esac
case "$OUT" in *"FPN threshold: 4095"*) ok "summary FPN";; *) fail "summary FPN: $OUT";; esac

# 2) --keep-fpn は FPN だけ 0
"$SCRIPT" --ped-json "$T/ped.json" --base "$T/base.json" --out "$T/out2.json" --keep-fpn >/dev/null 2>&1
check "keep-fpn: FPN threshold 0" "$(pyget "$T/out2.json" '[e["value"] for e in d["exceptions"]]')" "[430, 520, 0]"

# 3) 不正 base / 不正引数 / 表なし → 非 0
"$SCRIPT" --ped-json "$T/ped.json" --base "$T/bad.json" --out "$T/o3.json" >/dev/null 2>&1
nonzero "bad base -> nonzero" "$?"
[ ! -e "$T/o3.json" ] && ok "no output on bad base" || fail "no output on bad base"
"$SCRIPT" --ped-json "$T/ped.json" --base "$T/nonexistent.json" --out "$T/o4.json" >/dev/null 2>&1
nonzero "missing base -> nonzero" "$?"
"$SCRIPT" --base "$T/base.json" --out "$T/o5.json" >/dev/null 2>&1
nonzero "no ped source -> nonzero" "$?"

# 4) --ped-run: fake calib で引数を検査(graw 昇順 + --out)
mkdir "$T/run0042"; touch "$T/run0042/b.graw" "$T/run0042/a.graw"
cat > "$T/fakecalib" <<F
#!/usr/bin/env bash
echo "\$@" > "$T/calib_args"
out=""; while [ \$# -gt 0 ]; do [ "\$1" = --out ] && out="\$2"; shift; done
cp "$T/ped.json" "\$out"
F
chmod +x "$T/fakecalib"
"$SCRIPT" --ped-run "$T/run0042" --calib-bin "$T/fakecalib" --calib-dir "$T/calib" --label 502ns \
    --base "$T/base.json" --out "$T/out6.json" >/dev/null 2>&1
check "ped-run exit 0" "$?" 0
check "calib args" "$(cat "$T/calib_args" 2>/dev/null)" "pedestal $T/run0042/a.graw $T/run0042/b.graw --out $T/calib/pedestal_502ns.json"
check "ped-run 3 exceptions" "$(pyget "$T/out6.json" 'len(d["exceptions"])')" 3
mkdir "$T/empty"
"$SCRIPT" --ped-run "$T/empty" --calib-bin "$T/fakecalib" --calib-dir "$T/calib" --base "$T/base.json" --out "$T/o7.json" >/dev/null 2>&1
nonzero "ped-run without graw -> nonzero" "$?"

if [ "$FAIL" -eq 0 ]; then echo "ALL PASSED"; else echo "$FAIL FAILED"; fi
exit "$FAIL"
