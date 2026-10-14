#!/usr/bin/env python3
"""パラメータスキャンのドライバ(TODO/135)。

設定点の列を回す: PUT /api/params -> run/start -> status をポーリング ->
(events か max_s で)run/stop -> 記録。最後に base の params を PUT し直す。
Python 3 標準ライブラリのみ。HV / LV / power 系の API は呼ばない(SPEC §11)。
"""
import argparse
import copy
import hashlib
import json
import os
import re
import sys
import time
import urllib.error
import urllib.request

DEFAULT_PASSPHRASE_FILE = "/opt/tpcdaq/daq.toml"
COUNTER_KEYS = (
    "frames", "bytes", "overflow_frames", "malformed", "acquisition_restarts",
    "late_fragments", "frames_in", "seq_gaps",
)


class ScanAbort(Exception):
    """スキャンを止める理由(silent にしない)。"""


# ---------------------------------------------------------------- 純関数

def merge_point(base, knobs, name, i):
    """base の knobs を点の knobs で上書き(トップレベルキー単位)。(params, note) を返す。"""
    params = copy.deepcopy(base)
    diffs = []
    for key, value in knobs.items():
        old = base.get("knobs", {}).get(key)
        if old != value:
            diffs.append("%s %s->%s" % (key, _fmt(old), _fmt(value)))
        params["knobs"][key] = copy.deepcopy(value)
    note = "scan %s point %d: %s" % (name, i, ", ".join(diffs) if diffs else "(no change)")
    params["note"] = note
    return params, note


def _fmt(v):
    return v if isinstance(v, str) else json.dumps(v)


def receiver_frames(status):
    """receiver 系コンポーネントの frames 合計(mini では 1 フレーム = 1 イベント)。"""
    total = 0
    for c in status.get("components") or []:
        if str(c.get("name", "")).startswith("receiver"):
            total += int((c.get("metrics") or {}).get("frames") or 0)
    return total


def _metric_max(status, key):
    """全コンポーネントの metrics[key] の最大。無ければ None。"""
    vals = [
        (c.get("metrics") or {}).get(key)
        for c in status.get("components") or []
        if isinstance((c.get("metrics") or {}).get(key), (int, float))
    ]
    return max(vals) if vals else None


def should_stop(status, started_at, now, stop):
    """止める理由("events" / "max_s")か None。"""
    if receiver_frames(status) >= stop["events"]:
        return "events"
    if now - started_at >= stop["max_s"]:
        return "max_s"
    return None


def abort_reason(status, prev_frames, prev_change_at, now, stall_s=30):
    """異常で止める理由の文字列か None。

    prev_frames = 前回見た frames、prev_change_at = frames が最後に変わった時刻。
    """
    for key in ("malformed", "acquisition_restarts", "late_fragments"):
        v = _metric_max(status, key)
        if v:
            return "%s=%s (> 0)" % (key, v)
    if receiver_frames(status) == prev_frames and now - prev_change_at >= stall_s:
        return "stall: frames stuck at %d for %.0f s" % (prev_frames, now - prev_change_at)
    return None


def read_passphrase(toml_text):
    m = re.search(r'^\s*passphrase\s*=\s*"((?:[^"\\]|\\.)*)"', toml_text, re.M)
    if not m:
        raise ValueError("passphrase = \"...\" が見つからない")
    return m.group(1).replace('\\"', '"').replace("\\\\", "\\")


def counters_excerpt(status):
    out = {}
    for c in status.get("components") or []:
        m = {k: v for k, v in (c.get("metrics") or {}).items() if k in COUNTER_KEYS}
        if m:
            out[c.get("name")] = m
    return out


# ---------------------------------------------------------------- HTTP

class Api:
    def __init__(self, url):
        self.url = url.rstrip("/")

    def call(self, method, path, body=None, timeout=300):
        data = json.dumps(body).encode() if body is not None else None
        req = urllib.request.Request(self.url + path, data=data, method=method,
                                     headers={"Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=timeout) as r:
                code, raw = r.status, r.read()
        except urllib.error.HTTPError as e:
            code, raw = e.code, e.read()
        try:
            return code, json.loads(raw) if raw else {}
        except ValueError:
            return code, {"raw": raw.decode(errors="replace")}

    def ok(self, method, path, body=None):
        code, out = self.call(method, path, body)
        if not 200 <= code < 300:
            raise ScanAbort("%s %s -> HTTP %d: %s" % (method, path, code, json.dumps(out)[:300]))
        return out


# ---------------------------------------------------------------- スキャン本体

def write_record(path, record):
    tmp = path + ".tmp"
    with open(tmp, "w") as f:
        json.dump(record, f, indent=2)
    os.replace(tmp, path)


def run_point(api, token, params, note, i, cfg, poll_s, stall_s, clock, sleep):
    """1 点: Idle 確認 -> PUT -> start -> poll -> stop。点の記録 dict を返す(失敗は ScanAbort)。"""
    status = api.ok("GET", "/api/status")
    if status.get("phase") != "Idle":
        raise ScanAbort("point %d: phase が Idle でない (%s)" % (i, status.get("phase")))
    put = api.ok("PUT", "/api/params", {"token": token, "params": params})
    start = api.ok("POST", "/api/run/start", {"token": token, "comment": note})
    rec = {
        "i": i, "knobs": None, "params_id": start.get("params_id", put.get("id")),
        "run": start.get("run"), "started_ts": clock(), "start_notes": start.get("notes", []),
        "ok": False,
    }
    started_at = clock()
    prev_frames, prev_change = 0, started_at
    reason, abort = None, None
    try:
        while True:
            sleep(poll_s)
            status = api.ok("GET", "/api/status")
            now = clock()
            frames = receiver_frames(status)
            if frames != prev_frames:
                prev_frames, prev_change = frames, now
            abort = abort_reason(status, prev_frames, prev_change, now, stall_s)
            if abort:
                break
            reason = should_stop(status, started_at, now, cfg["stop"])
            if reason:
                break
    finally:
        # 何が起きても run は止める(例外を握りつぶさず、stop 自体の失敗は上に伝える)
        stop_reply = api.ok("POST", "/api/run/stop", {"token": token})
        rec["stopped_ts"] = clock()
        rec["stop_reply"] = stop_reply
    # stop 後に Idle へ戻るのを少し待ってから最終 status を記録
    final = status
    for _ in range(30):
        final = api.ok("GET", "/api/status")
        if final.get("phase") == "Idle":
            break
        sleep(max(poll_s, 0.1))
    rec["frames"] = max(prev_frames, receiver_frames(final))
    rec["elapsed_s"] = rec["stopped_ts"] - started_at
    rec["status_at_stop"] = counters_excerpt(final)
    rec["stop_reason"] = reason or "abort"
    if abort:
        rec["abort"] = abort
        raise PointAborted(abort, rec)
    rec["ok"] = True
    return rec


class PointAborted(ScanAbort):
    def __init__(self, reason, rec):
        super().__init__(reason)
        self.rec = rec


def run_scan(cfg, base, api, passphrase, operator, out_dir, resume_rec=None,
             poll_s=2.0, stall_s=30.0, clock=time.time, sleep=time.sleep):
    """スキャンを回す。0 = 完走、2 = 異常停止。"""
    path = os.path.join(out_dir, "scan_%s.json" % cfg["name"])
    base_sha = hashlib.sha256(json.dumps(base, sort_keys=True).encode()).hexdigest()
    record = resume_rec or {"name": cfg["name"], "started": clock(), "base_sha256": base_sha, "points": []}
    done = {p["i"] for p in record["points"] if p.get("ok")}
    record["points"] = [p for p in record["points"] if p.get("ok")]

    token = api.ok("POST", "/api/control/acquire", {"operator": operator, "passphrase": passphrase})["token"]
    code = 0
    try:
        for i, point in enumerate(cfg["points"]):
            if i in done:
                print("point %d: skipped (already ok)" % i, flush=True)
                continue
            params, note = merge_point(base, point["knobs"], cfg["name"], i)
            print("point %d: %s" % (i, note), flush=True)
            try:
                rec = run_point(api, token, params, note, i, cfg, poll_s, stall_s, clock, sleep)
            except PointAborted as e:
                rec = e.rec
                rec["i"], rec["knobs"] = i, point["knobs"]
                record["points"].append(rec)
                write_record(path, record)
                raise
            rec["knobs"] = point["knobs"]
            record["points"].append(rec)
            write_record(path, record)
            print("point %d: run %s frames=%d %.1f s (%s) ok" % (
                i, rec["run"], rec["frames"], rec["elapsed_s"], rec["stop_reason"]), flush=True)
    except ScanAbort as e:
        print("SCAN ABORTED: %s" % e, file=sys.stderr, flush=True)
        code = 2
    finally:
        # 現用設定に戻す(必ず)。失敗したら大きく表示。
        try:
            api.ok("PUT", "/api/params", {"token": token, "params": base})
            print("base params restored", flush=True)
        except Exception as e:  # noqa: BLE001 -- 何であれ大きく出す
            print("!" * 60 + "\n!!! BASE PARAMS NOT RESTORED: %s\n!!! 手で PUT し直すこと: %s\n" % (e, cfg["base"])
                  + "!" * 60, file=sys.stderr, flush=True)
            code = 2
        try:
            api.ok("POST", "/api/control/release", {"token": token})
        except Exception as e:  # noqa: BLE001
            print("release failed: %s" % e, file=sys.stderr, flush=True)
    return code


def dry_run(cfg, base):
    print("dry-run: nothing is sent. Per point: check Idle -> PUT /api/params -> run/start -> poll status "
          "-> run/stop (stop: %s); finally PUT base." % json.dumps(cfg["stop"]))
    for i, point in enumerate(cfg["points"]):
        params, note = merge_point(base, point["knobs"], cfg["name"], i)
        print("--- point %d: %s\nPUT /api/params {token, params} params =" % (i, note))
        print(json.dumps(params, indent=2, sort_keys=True))
    print("--- finally: PUT /api/params with base params (%s)" % cfg["base"])
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(description="TPC DAQ parameter scan driver")
    ap.add_argument("scan_json")
    ap.add_argument("--url", default="http://127.0.0.1:8080")
    ap.add_argument("--operator", default="scan")
    ap.add_argument("--passphrase-file", default=DEFAULT_PASSPHRASE_FILE)
    ap.add_argument("--out-dir", default=".")
    ap.add_argument("--poll-s", type=float, default=2.0)
    ap.add_argument("--stall-s", type=float, default=30.0)
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--resume")
    args = ap.parse_args(argv)

    with open(args.scan_json) as f:
        cfg = json.load(f)
    with open(cfg["base"]) as f:
        base = json.load(f)
    if args.dry_run:
        return dry_run(cfg, base)
    with open(args.passphrase_file) as f:
        passphrase = read_passphrase(f.read())
    resume_rec = None
    if args.resume:
        with open(args.resume) as f:
            resume_rec = json.load(f)
    return run_scan(cfg, base, Api(args.url), passphrase, args.operator, args.out_dir,
                    resume_rec, args.poll_s, args.stall_s)


if __name__ == "__main__":
    sys.exit(main())
