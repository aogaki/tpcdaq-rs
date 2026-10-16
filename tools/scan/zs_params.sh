#!/usr/bin/env bash
# TODO/155 — pedestal run(または pedestal 表)から ZS 付きの params JSON を作る。PUT はしない。
#
#   zs_params.sh (--ped-run RUN_DIR | --ped-json FILE) --base BASE.json --out OUT.json
#                [--k 5] [--min-above 20] [--label 502ns] [--keep-fpn]
#                [--calib-bin PATH] [--calib-dir DIR]
#
# --ped-run: RUN_DIR/*.graw に `calib pedestal` を走らせ DIR/pedestal_<label>.json を作る
#            (DIR 既定 ~/test/scan/calib、calib 既定 <repo>/target/release/calib)。
# 作った params は UI の Parameters タブ or scan.py で適用する(docs/RUNBOOK_ja.md「ZS 運用」)。
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"

PED_RUN=""; PED_JSON=""; BASE=""; OUT=""; K=5; MIN_ABOVE=20; LABEL=""; KEEP_FPN=""
CALIB_BIN="$REPO/target/release/calib"; CALIB_DIR="$HOME/test/scan/calib"

die() { echo "zs_params: $*" >&2; exit 2; }

while [ $# -gt 0 ]; do
  case "$1" in
    --ped-run) PED_RUN="${2:-}"; shift 2;;
    --ped-json) PED_JSON="${2:-}"; shift 2;;
    --base) BASE="${2:-}"; shift 2;;
    --out) OUT="${2:-}"; shift 2;;
    --k) K="${2:-}"; shift 2;;
    --min-above) MIN_ABOVE="${2:-}"; shift 2;;
    --label) LABEL="${2:-}"; shift 2;;
    --keep-fpn) KEEP_FPN="--keep-fpn"; shift;;
    --calib-bin) CALIB_BIN="${2:-}"; shift 2;;
    --calib-dir) CALIB_DIR="${2:-}"; shift 2;;
    *) die "unknown argument: $1";;
  esac
done

[ -n "$BASE" ] && [ -n "$OUT" ] || die "--base and --out are required"
if [ -n "$PED_RUN" ] && [ -n "$PED_JSON" ]; then die "give either --ped-run or --ped-json, not both"; fi
[ -n "$PED_RUN" ] || [ -n "$PED_JSON" ] || die "one of --ped-run / --ped-json is required"
[ -f "$BASE" ] || die "base not found: $BASE"
python3 -c 'import json,sys; json.load(open(sys.argv[1]))' "$BASE" 2>/dev/null || die "base is not valid JSON: $BASE"

if [ -n "$PED_RUN" ]; then
  [ -d "$PED_RUN" ] || die "run dir not found: $PED_RUN"
  [ -n "$LABEL" ] || LABEL="$(basename "$PED_RUN")"
  GRAWS=()
  while IFS= read -r f; do GRAWS+=("$f"); done < <(ls "$PED_RUN"/*.graw 2>/dev/null | sort)
  [ "${#GRAWS[@]}" -gt 0 ] || die "no .graw in $PED_RUN"
  [ -x "$CALIB_BIN" ] || die "calib not found: $CALIB_BIN (cargo build --release)"
  mkdir -p "$CALIB_DIR" || die "cannot create $CALIB_DIR"
  PED_JSON="$CALIB_DIR/pedestal_$LABEL.json"
  "$CALIB_BIN" pedestal "${GRAWS[@]}" --out "$PED_JSON" || die "calib pedestal failed"
fi
[ -f "$PED_JSON" ] || die "pedestal table not found: $PED_JSON"
[ -n "$LABEL" ] || LABEL="$(basename "$PED_JSON" .json)"

TMP="$(mktemp)"; trap 'rm -f "$TMP"' EXIT
# shellcheck disable=SC2086
python3 "$HERE/zs_exceptions.py" "$PED_JSON" --k "$K" --min-above "$MIN_ABOVE" $KEEP_FPN \
    --merge "$BASE" --out "$TMP" || die "zs_exceptions.py failed"
cp "$TMP" "$OUT" || die "cannot write $OUT"

# 要約: ラベルの隣に base の peakingTime_ns / gain(表と設定の不一致を操作者が見る)
python3 - "$OUT" "$LABEL" <<'PY'
import json, statistics, sys
d = json.load(open(sys.argv[1]))
knobs = d.get("knobs", {})
ex = [e for e in d["exceptions"] if e["knob"] == "zeroSuppression.threshold"]
vals = [e["value"] for e in ex]
print(f"ZS params: table {sys.argv[2]} (base peakingTime_ns={knobs.get('peakingTime_ns')} gain={knobs.get('gain')})"
      " -- the table must be taken at the same peaking/gain")
print(f"  exceptions: {len(vals)}")
if vals:
    print(f"  threshold: min {min(vals)}  median {statistics.median(vals):g}  max {max(vals)}")
PY
python3 - "$PED_JSON" "$OUT" <<'PY'
import json, sys
table = json.load(open(sys.argv[1]))
d = json.load(open(sys.argv[2]))
fpn = {(c["asad"], c["aget"], c["ch"]) for c in table["channels"] if c.get("is_fpn")}
vals = sorted({e["value"] for e in d["exceptions"]
               if (e["at"]["asad"], e["at"]["aget"], e["at"]["ch"]) in fpn})
print("  FPN threshold: " + (", ".join(map(str, vals)) if vals else "(no FPN ch in table)"))
PY
echo "  wrote $OUT  (not applied: use the Parameters tab or scan.py)"
