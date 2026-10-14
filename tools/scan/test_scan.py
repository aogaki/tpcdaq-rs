"""scan.py のテスト(実サーバ不要)。`cd tools/scan && python3 -m unittest`。"""
import contextlib
import copy
import io
import json
import os
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer

import scan

SECRET = "s3cret-pass"

BASE = {
    "schema": "tpcdaq-params/1",
    "template": "configure-elinp.xcfg",
    "knobs": {
        "triggerDelay": 1500,
        "gain": "120fC",
        "zeroSuppression": {"enabled": False, "threshold": 600},
    },
    "exceptions": ["x"],
    "note": "base note",
}


def status_json(phase="Idle", run=None, frames=0, malformed=0, restarts=0):
    return {
        "phase": phase,
        "run": {"run": run} if run is not None else None,
        "components": [
            {"name": "receiver0", "metrics": {"frames": frames, "bytes": 7}},
            {"name": "decoder0", "metrics": {"malformed": malformed, "acquisition_restarts": restarts}},
        ],
    }


class MergeAndPure(unittest.TestCase):
    def test_merge_point_overrides_only_the_given_knobs(self):
        before = copy.deepcopy(BASE)
        params, note = scan.merge_point(BASE, {"triggerDelay": 1000}, "tdelay", 2)
        self.assertEqual(params["knobs"]["triggerDelay"], 1000)
        # 無いキーはそのまま(gain, ネストした zeroSuppression)
        self.assertEqual(params["knobs"]["gain"], "120fC")
        self.assertEqual(params["knobs"]["zeroSuppression"], {"enabled": False, "threshold": 600})
        self.assertEqual(params["exceptions"], ["x"])
        self.assertEqual(params["template"], "configure-elinp.xcfg")
        self.assertEqual(note, "scan tdelay point 2: triggerDelay 1500->1000")
        self.assertEqual(params["note"], note)
        self.assertEqual(BASE, before, "base を破壊しない")

    def test_should_stop_on_events_or_elapsed(self):
        stop = {"events": 1500, "max_s": 600}

        def st(f):
            return status_json("Running", 5, frames=f)

        self.assertIsNone(scan.should_stop(st(1499), 100.0, 150.0, stop))
        self.assertEqual(scan.should_stop(st(1500), 100.0, 150.0, stop), "events")
        self.assertEqual(scan.should_stop(st(10), 100.0, 700.0, stop), "max_s")
        self.assertIsNone(scan.should_stop(st(10), 100.0, 699.9, stop))

    def test_abort_on_malformed_or_restart_or_stall(self):
        ok = status_json("Running", 5, frames=100)
        self.assertIsNone(scan.abort_reason(ok, 90, 10.0, 12.0, 30))
        self.assertIn("malformed", scan.abort_reason(status_json(malformed=1), 0, 0, 1, 30))
        self.assertIn("acquisition_restarts", scan.abort_reason(status_json(restarts=2), 0, 0, 1, 30))
        # 停滞: frames が前回と同じで 30 s 経過
        self.assertIn("stall", scan.abort_reason(ok, 100, 10.0, 40.0, 30))
        self.assertIsNone(scan.abort_reason(ok, 100, 10.0, 39.9, 30))
        late = status_json()
        late["components"].append({"name": "root-sink", "metrics": {"late_fragments": 3}})
        self.assertIn("late_fragments", scan.abort_reason(late, 0, 0, 1, 30))

    def test_passphrase_is_read_from_toml_and_never_logged(self):
        toml = '[controller]\nrest_listen = "0.0.0.0:8080"\npassphrase = "%s"\n' % SECRET
        self.assertEqual(scan.read_passphrase(toml), SECRET)
        with self.assertRaises(ValueError):
            scan.read_passphrase("[controller]\n")
        # 実走(下のフェイク)で stdout/stderr/出力 JSON に漏れないことを確認している


class FakeDaq:
    """状態を持つ擬似 controller。"""

    def __init__(self, malformed_on_start=None):
        self.lock = threading.Lock()
        self.phase = "Idle"
        self.run = 100
        self.frames = 0
        self.starts = 0
        self.puts = []
        self.log = []
        self.malformed_on_start = malformed_on_start
        self.malformed = 0

    def handle(self, method, path, body):
        with self.lock:
            self.log.append(path)
            if path == "/api/control/acquire":
                if body.get("passphrase") != SECRET:
                    return 401, {"error": "bad"}
                return 200, {"token": "tok"}
            if path == "/api/control/release":
                return 200, {"ok": True}
            if path == "/api/params" and method == "PUT":
                if self.phase != "Idle":
                    return 409, {"error": "running"}
                self.puts.append(body["params"])
                return 200, {"id": "pid%d" % len(self.puts)}
            if path == "/api/run/start":
                self.starts += 1
                self.run += 1
                self.phase = "Running"
                self.frames = 0
                if self.malformed_on_start == self.starts:
                    self.malformed = 1
                return 200, {
                    "run": self.run,
                    "phase": "Running",
                    "params_id": "pid%d" % len(self.puts),
                    "notes": [],
                }
            if path == "/api/run/stop":
                self.phase = "Idle"
                return 200, {"run": self.run, "phase": "Idle"}
            if path == "/api/status":
                if self.phase == "Running":
                    self.frames += 400
                run = self.run if self.phase == "Running" else None
                return 200, status_json(self.phase, run, self.frames, self.malformed)
            return 404, {"error": path}


def serve(daq):
    class H(BaseHTTPRequestHandler):
        def _do(self):
            n = int(self.headers.get("Content-Length") or 0)
            body = json.loads(self.rfile.read(n)) if n else {}
            code, out = daq.handle(self.command, self.path, body)
            data = json.dumps(out).encode()
            self.send_response(code)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        do_GET = do_POST = do_PUT = _do

        def log_message(self, *a):
            pass

    srv = HTTPServer(("127.0.0.1", 0), H)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv


class FakeServerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        d = self.tmp.name
        with open(os.path.join(d, "base.json"), "w") as f:
            json.dump(BASE, f)
        with open(os.path.join(d, "daq.toml"), "w") as f:
            f.write('passphrase = "%s"\n' % SECRET)
        scan_def = {
            "name": "t",
            "base": os.path.join(d, "base.json"),
            "stop": {"events": 1000, "max_s": 60},
            "points": [{"knobs": {"triggerDelay": v}} for v in (1000, 1250, 1500, 1750, 2000)],
        }
        with open(os.path.join(d, "scan.json"), "w") as f:
            json.dump(scan_def, f)

    def run_main(self, daq, extra=()):
        srv = serve(daq)
        self.addCleanup(srv.server_close)
        self.addCleanup(srv.shutdown)
        d = self.tmp.name
        argv = [
            os.path.join(d, "scan.json"),
            "--url", "http://127.0.0.1:%d" % srv.server_port,
            "--passphrase-file", os.path.join(d, "daq.toml"),
            "--out-dir", d,
            "--poll-s", "0",
            "--stall-s", "5",
        ] + list(extra)
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = scan.main(argv)
        return code, out.getvalue() + err.getvalue()

    def read_record(self):
        with open(os.path.join(self.tmp.name, "scan_t.json")) as f:
            return f.read()

    def test_five_point_scan_records_each_run_and_restores_base(self):
        daq = FakeDaq()
        code, output = self.run_main(daq)
        self.assertEqual(code, 0, output)
        self.assertNotIn(SECRET, output)
        raw = self.read_record()
        self.assertNotIn(SECRET, raw)
        rec = json.loads(raw)
        self.assertEqual(rec["name"], "t")
        self.assertEqual(len(rec["base_sha256"]), 64)
        self.assertEqual([p["i"] for p in rec["points"]], [0, 1, 2, 3, 4])
        self.assertEqual([p["run"] for p in rec["points"]], [101, 102, 103, 104, 105])
        self.assertTrue(all(p["ok"] for p in rec["points"]))
        self.assertEqual(rec["points"][1]["knobs"], {"triggerDelay": 1250})
        self.assertEqual(rec["points"][1]["params_id"], "pid2")
        self.assertGreaterEqual(rec["points"][0]["frames"], 1000)
        for key in ("started_ts", "stopped_ts", "elapsed_s", "stop_reply", "status_at_stop"):
            self.assertIn(key, rec["points"][0])
        # PUT は 5 点 + 最後に base 戻し
        self.assertEqual(len(daq.puts), 6)
        self.assertEqual(
            [p["knobs"]["triggerDelay"] for p in daq.puts], [1000, 1250, 1500, 1750, 2000, 1500]
        )
        self.assertEqual(daq.puts[-1], BASE)
        self.assertEqual(daq.log[-1], "/api/control/release")

    def test_abort_mid_scan_stops_the_run_and_restores_base(self):
        daq = FakeDaq(malformed_on_start=3)
        code, output = self.run_main(daq)
        self.assertEqual(code, 2, output)
        self.assertIn("malformed", output)
        self.assertNotIn(SECRET, output)
        self.assertEqual(daq.phase, "Idle", "異常時に run を止める")
        self.assertEqual(daq.puts[-1], BASE, "異常時も base に戻す")
        self.assertEqual(daq.starts, 3, "4 点目には進まない")
        rec = json.loads(self.read_record())
        self.assertEqual([p["ok"] for p in rec["points"]], [True, True, False])

    def test_resume_skips_finished_points(self):
        self.run_main(FakeDaq(malformed_on_start=3))
        daq2 = FakeDaq()
        code, output = self.run_main(
            daq2, ["--resume", os.path.join(self.tmp.name, "scan_t.json")]
        )
        self.assertEqual(code, 0, output)
        self.assertEqual(daq2.starts, 3)  # 点 2,3,4 のみ
        rec = json.loads(self.read_record())
        self.assertEqual([p["i"] for p in rec["points"]], [0, 1, 2, 3, 4])
        self.assertTrue(all(p["ok"] for p in rec["points"]))

    def test_dry_run_sends_nothing(self):
        daq = FakeDaq()
        code, output = self.run_main(daq, ["--dry-run"])
        self.assertEqual(code, 0, output)
        self.assertEqual(daq.log, [])
        self.assertIn('"triggerDelay": 1000', output)


if __name__ == "__main__":
    unittest.main()
