#!/usr/bin/env python3
"""スキャン台帳の表を生成する(TODO/143)。

scan.py の結果 JSON と run_metrics.C の指標 JSON(run%04d_metrics.json)から
Markdown の表を 1 つ標準出力へ出す(docs/SCANS_ja.md の貼り付け用)。
Python 3 標準ライブラリのみ。
"""
import argparse
import glob
import json
import os
import sys

DASH = "—"
HEADER = [
    "点", "run", "knobs", "events", "kB/event", "first hit 中央値 (p10–p90)", "last hit 中央値 (p90)",
    "幅 [cell]", "ヒット ch", "総電荷", "飽和 event", "クリップ 先頭 / 末尾", "ok", "長さ [mm] 中央値 / IQR",
    "飛跡電荷", "dEdx 中央値 / IQR", "dip RMS",
]


# ---------------------------------------------------------------- 書式(None -> 「—」)

def _num(x):
    """整数値なら整数、そうでなければ小数 1 桁。"""
    if x is None:
        return DASH
    return "%d" % round(x) if abs(x - round(x)) < 1e-9 else "%.1f" % x


def _int(x):
    return DASH if x is None else "{:,}".format(round(x))


def _fixed(x, nd):
    return DASH if x is None else "%.*f" % (nd, x)


def _pct(frac):
    return DASH if frac is None else "%.1f %%" % (frac * 100)


def _mega(x):
    return DASH if x is None else "%.2f M" % (x / 1e6)


def _pair(a, b, f):
    if a is None and b is None:
        return DASH
    return "%s / %s" % (f(a), f(b))


def _get(d, *keys):
    """ネストした dict を辿る。途中が無い / null なら None。"""
    for k in keys:
        if not isinstance(d, dict):
            return None
        d = d.get(k)
    return d


def knobs_text(knobs):
    if not knobs:
        return "(base)"
    parts = []
    for k, v in knobs.items():
        parts.append("%s=%s" % (k, v if isinstance(v, str) else json.dumps(v)))
    return ", ".join(parts)


# ---------------------------------------------------------------- 入力

def load_metrics(metrics_dir, run):
    if run is None:
        return None
    path = os.path.join(metrics_dir, "run%04d_metrics.json" % run)
    if not os.path.isfile(path):
        return None
    with open(path) as f:
        return json.load(f)


def graw_bytes(run_root, run):
    """DIR/run%04d/*.graw の合計バイト。graw が無ければ None。"""
    if run_root is None or run is None:
        return None
    files = glob.glob(os.path.join(run_root, "run%04d" % run, "*.graw"))
    if not files:
        return None
    return sum(os.path.getsize(p) for p in files)


# ---------------------------------------------------------------- 表

def row_cells(point, m, nbytes):
    t = _get(m, "tracks")
    events = _get(m, "events")
    kb = nbytes / events / 1000 if nbytes is not None and events else None
    fh, lh = _get(m, "first_hit_cell"), _get(m, "last_hit_cell")
    first = DASH if fh is None else "%s (%s–%s)" % (_num(fh.get("median")), _num(fh.get("p10")), _num(fh.get("p90")))
    last = DASH if lh is None else "%s (%s)" % (_num(lh.get("median")), _num(lh.get("p90")))
    dip = _get(t, "dip_abs_rms_deg")
    return [
        str(point.get("i", DASH)),
        DASH if point.get("run") is None else str(point["run"]),
        knobs_text(point.get("knobs")),
        _int(events),
        _fixed(kb, 1),
        first,
        last,
        _num(_get(m, "pulse_width_cells_median")),
        _num(_get(m, "hit_channels_median")),
        _mega(_get(m, "charge_median")),
        _pct(_get(m, "saturated_event_frac")),
        _pair(_get(m, "clip_front_edge"), _get(m, "clip_back_edge"), lambda x: DASH if x is None else "%.1f" % (x * 100))
        + ("" if m is None else " %"),
        _fixed(_get(t, "ok_frac"), 3),
        _pair(_get(t, "length", "median"), _get(t, "length", "iqr"), lambda x: _fixed(x, 1)),
        _mega(_get(t, "charge", "median")),
        _pair(_get(t, "dEdx", "median"), _get(t, "dEdx", "iqr"), _int),
        DASH if dip is None else "%.1f°" % dip,
    ]


def build_table(scan_path, metrics_dir, run_root=None, title=None):
    with open(scan_path) as f:
        scan = json.load(f)
    lines = []
    if title:
        lines += ["## " + title, ""]
    lines.append("| " + " | ".join(HEADER) + " |")
    lines.append("|" + "---|" * len(HEADER))
    for point in scan.get("points", []):
        run = point.get("run")
        cells = row_cells(point, load_metrics(metrics_dir, run), graw_bytes(run_root, run))
        lines.append("| " + " | ".join(cells) + " |")
    return "\n".join(lines) + "\n"


def main(argv=None):
    ap = argparse.ArgumentParser(description="scan.py の結果 JSON + 指標 JSON から台帳の Markdown 表を出す")
    ap.add_argument("scan_json")
    ap.add_argument("--metrics", required=True, help="run%%04d_metrics.json のあるディレクトリ")
    ap.add_argument("--run-root", help="DIR/run%%04d/*.graw があれば kB/event を出す")
    ap.add_argument("--title", help="先頭に '## <title>' 見出しを付ける")
    args = ap.parse_args(argv)
    sys.stdout.write(build_table(args.scan_json, args.metrics, args.run_root, args.title))
    return 0


if __name__ == "__main__":
    sys.exit(main())
