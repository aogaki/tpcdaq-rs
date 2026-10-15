#!/usr/bin/env python3
"""TODO/149 §1 — `calib pedestal` の JSON から ch ごとの ZS 閾値の `exceptions` を作る。

  python3 tools/scan/zs_exceptions.py PED.json [--k 5] [--min-above 20] [--out FILE] [--merge BASE.json]

各 ch(FPN 含む)に
  {"knob": "zeroSuppression.threshold", "at": {"asad": a, "aget": g, "ch": c},
   "value": round(mean + max(k*rms, min_above))}        # [0, 4095] にクリップ
を作る(形は src/params/schema.rs の Exception / At。`at` のキーは "ch"、"channel" ではない)。
--merge BASE.json: base params の exceptions のうち zeroSuppression.threshold のものを置き換え、
knobs.zeroSuppression.enabled = true にした params JSON を出す(他の knob・exceptions・note は保つ)。
丸めは四捨五入(0.5 は切り上げ)。標準ライブラリのみ。
"""
import argparse
import json
import math
import sys

KNOB = "zeroSuppression.threshold"
ADC_MAX = 4095


def threshold(mean, rms, k, min_above):
    """mean + max(k*rms, min_above) を四捨五入して [0, 4095] に収める。"""
    value = math.floor(mean + max(k * rms, min_above) + 0.5)
    return max(0, min(ADC_MAX, value))


def make_exceptions(table, k=5.0, min_above=20.0):
    return [
        {
            "knob": KNOB,
            "at": {"asad": ch["asad"], "aget": ch["aget"], "ch": ch["ch"]},
            "value": threshold(ch["mean"], ch["rms"], k, min_above),
        }
        for ch in table["channels"]
    ]


def merge_into(base, exceptions):
    out = json.loads(json.dumps(base))  # deep copy
    knobs = out.setdefault("knobs", {})
    zs = knobs.get("zeroSuppression")
    if not isinstance(zs, dict):
        zs = {}
        knobs["zeroSuppression"] = zs
    zs["enabled"] = True
    kept = [e for e in out.get("exceptions", []) if e.get("knob") != KNOB]
    out["exceptions"] = kept + exceptions
    return out


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("pedestal", help="`calib pedestal` の JSON")
    ap.add_argument("--k", type=float, default=5.0, help="σ の倍数(既定 5)")
    ap.add_argument("--min-above", type=float, default=20.0, help="ped からの最小の余裕 [ADC](既定 20)")
    ap.add_argument("--out", help="出力先(既定 標準出力)")
    ap.add_argument("--merge", metavar="BASE.json", help="base params に差し込んだ params JSON を出す")
    args = ap.parse_args(argv)

    with open(args.pedestal) as f:
        table = json.load(f)
    result = make_exceptions(table, args.k, args.min_above)
    if args.merge:
        with open(args.merge) as f:
            result = merge_into(json.load(f), result)
    text = json.dumps(result, indent=1) + "\n"
    if args.out:
        with open(args.out, "w") as f:
            f.write(text)
    else:
        sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
