"""ledger.py のテスト(実サーバ・ROOT 不要)。`python3 -m unittest tools/scan/test_ledger.py`。"""
import contextlib
import io
import json
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ledger  # noqa: E402

HEADER_COLS = [
    "点", "run", "knobs", "events", "kB/event", "first hit 中央値 (p10–p90)", "last hit 中央値 (p90)",
    "幅 [cell]", "ヒット ch", "総電荷", "飽和 event", "クリップ 先頭 / 末尾", "ok", "長さ [mm] 中央値 / IQR",
    "飛跡電荷", "dEdx 中央値 / IQR", "dip RMS",
]

SCAN = {
    "name": "demo_2026-10-15",
    "points": [
        {"i": 0, "knobs": {}, "run": 5, "params_id": "a", "frames": 1500, "elapsed_s": 1.0, "stop_reason": "events"},
        {"i": 1, "knobs": {"triggerDelay": 1750, "gain": "240fC"}, "run": 6, "frames": 1500, "elapsed_s": 1.0,
         "stop_reason": "events"},
        {"i": 2, "knobs": {"zeroSuppression": {"enabled": True, "threshold": 600}}, "run": 7, "frames": 10,
         "elapsed_s": 1.0, "stop_reason": "events"},
    ],
}


def metrics(run, tracks):
    return {
        "run": run, "raw_file": "x", "events": 1500, "rate_hz": 4.6,
        "first_hit_cell": {"median": 71.0, "p10": 55.0, "p90": 76.0},
        "last_hit_cell": {"median": 94.5, "p10": 80.0, "p90": 142.0},
        "clip_front_peak": 0.05, "clip_front_edge": 0.026, "clip_back_peak": 0.1, "clip_back_edge": 0.078,
        "saturated_sample_frac": 0.001, "saturated_event_frac": 0.0062,
        "pulse_width_cells_median": 62.0, "hit_channels_median": 119.0, "charge_median": 3240000.0,
        "tracks": tracks,
    }


TRACKS = {
    "n": 1400, "ok_frac": 0.9953,
    "length": {"median": 83.44, "iqr": 59.6},
    "charge": {"median": 1770000.0, "iqr": 1.0},
    "dEdx": {"median": 11861.4, "iqr": 5919.2},
    "dip_abs_rms_deg": 7.14, "phi_axial_rms_deg": 50.0,
}


class LedgerTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        d = self.tmp.name
        self.mdir = os.path.join(d, "metrics")
        self.rroot = os.path.join(d, "data")
        os.makedirs(self.mdir)
        self.scan_path = os.path.join(d, "scan.json")
        with open(self.scan_path, "w") as f:
            json.dump(SCAN, f)
        # run 5: 通常、run 6: tracks null、run 7: 指標なし
        for run, tr in ((5, TRACKS), (6, None)):
            with open(os.path.join(self.mdir, "run%04d_metrics.json" % run), "w") as f:
                json.dump(metrics(run, tr), f)
        # 擬似 graw(スパース): run 5 = 600,000,000 + 150,000,000 B
        os.makedirs(os.path.join(self.rroot, "run0005"))
        for name, size in (("a.graw", 600_000_000), ("b.graw", 150_000_000)):
            with open(os.path.join(self.rroot, "run0005", name), "wb") as f:
                f.truncate(size)
        os.makedirs(os.path.join(self.rroot, "run0006"))  # graw 無し

    def table(self, **kw):
        return ledger.build_table(self.scan_path, self.mdir, **kw)

    def rows(self, text):
        lines = [l for l in text.splitlines() if l.startswith("|")]
        return [[c.strip() for c in l.strip("|").split("|")] for l in lines]

    def test_header_and_separator(self):
        rows = self.rows(self.table())
        self.assertEqual(rows[0], HEADER_COLS)
        self.assertEqual(set(rows[1]), {"---"})
        self.assertEqual(len(rows), 2 + 3)

    def test_full_row_formats(self):
        # 手計算: 750e6 B / 1500 events = 500,000 B/event = 500.0 kB(1 kB = 1000 B)
        r = self.rows(self.table(run_root=self.rroot))[2]
        self.assertEqual(r, [
            "0", "5", "(base)", "1,500", "500.0",
            "71 (55–76)", "94.5 (142)", "62", "119", "3.24 M", "0.6 %", "2.6 / 7.8 %", "0.995",
            "83.4 / 59.6", "1.77 M", "11,861 / 5,919", "7.1°",
        ])

    def test_knobs_are_point_override_only(self):
        rows = self.rows(self.table())
        self.assertEqual(rows[3][2], "triggerDelay=1750, gain=240fC")
        self.assertEqual(rows[4][2], 'zeroSuppression={"enabled": true, "threshold": 600}')

    def test_no_run_root_gives_dash_for_kb_per_event(self):
        self.assertEqual(self.rows(self.table())[2][4], "—")

    def test_run_root_without_graw_gives_dash(self):
        self.assertEqual(self.rows(self.table(run_root=self.rroot))[3][4], "—")

    def test_tracks_null_row_kept_with_dashes(self):
        r = self.rows(self.table())[3]
        self.assertEqual(r[0:2], ["1", "6"])
        self.assertEqual(r[3], "1,500")  # 指標側の列は出る
        self.assertEqual(r[12:], ["—", "—", "—", "—", "—"])  # ok 以降は tracks 由来

    def test_missing_metrics_row_kept_with_dashes(self):
        r = self.rows(self.table())[4]
        self.assertEqual(r[0:3], ["2", "7", 'zeroSuppression={"enabled": true, "threshold": 600}'])
        self.assertEqual(r[3:], ["—"] * 14)

    def test_title_heading(self):
        self.assertTrue(self.table(title="S10 demo").startswith("## S10 demo\n"))
        self.assertTrue(self.table().startswith("| 点 |"))

    def test_main_prints_table(self):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = ledger.main([self.scan_path, "--metrics", self.mdir, "--run-root", self.rroot, "--title", "T"])
        self.assertEqual(code, 0)
        self.assertIn("## T", out.getvalue())
        self.assertIn("| 0 | 5 | (base) |", out.getvalue())


if __name__ == "__main__":
    unittest.main()
