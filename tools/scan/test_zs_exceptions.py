"""TODO/149 §1 — zs_exceptions.py のテスト。 python3 -m unittest tools/scan/test_zs_exceptions.py"""
import json
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import zs_exceptions as zs  # noqa: E402


def ch(asad, aget, c, mean, rms, fpn=False):
    return {"asad": asad, "aget": aget, "ch": c, "mean": mean, "rms": rms, "runs": 1, "is_fpn": fpn}


# 手計算(k = 5, min_above = 20):
#   (0,0,3)  mean 400.0 rms 6.0  : 5*6 = 30  > 20 → 400 + 30 = 430
#   (1,2,40) mean 500.4 rms 2.0  : 5*2 = 10  < 20 → 500.4 + 20 = 520.4 → 520
#   (3,3,11) FPN mean 4090.2 rms 4.0 : 20 (= 20) → 4110.2 → 4095 にクリップ
# 丸め: mean 100.3 rms 1.0 → 120.3 → 120 / mean 100.5 → 120.5 → 121(0.5 は切り上げ)
# 下限: mean −50 rms 0 → −30 → 0
TABLE = {"meta": {"note": "synthetic"}, "channels": [
    ch(0, 0, 3, 400.0, 6.0),
    ch(1, 2, 40, 500.4, 2.0),
    ch(3, 3, 11, 4090.2, 4.0, fpn=True),
]}


class ThresholdTest(unittest.TestCase):
    def test_k_sigma_wins_when_larger(self):
        self.assertEqual(zs.threshold(400.0, 6.0, 5, 20), 430)

    def test_min_above_wins_when_larger_and_rounds(self):
        self.assertEqual(zs.threshold(500.4, 2.0, 5, 20), 520)
        self.assertEqual(zs.threshold(100.3, 1.0, 5, 20), 120)

    def test_half_rounds_up(self):
        self.assertEqual(zs.threshold(100.5, 1.0, 5, 20), 121)

    def test_clipped_to_adc_range(self):
        self.assertEqual(zs.threshold(4090.2, 4.0, 5, 20), 4095)
        self.assertEqual(zs.threshold(-50.0, 0.0, 5, 20), 0)

    def test_k_and_min_above_are_parameters(self):
        self.assertEqual(zs.threshold(400.0, 6.0, 10, 20), 460)
        self.assertEqual(zs.threshold(400.0, 6.0, 5, 50), 450)


class MakeExceptionsTest(unittest.TestCase):
    def test_keep_fpn_sets_fpn_threshold_zero_only(self):  # TODO/155: FPN(3,3,11)だけ 0、他は不変
        vals = [e["value"] for e in zs.make_exceptions(TABLE, keep_fpn=True)]
        self.assertEqual(vals, [430, 520, 0])

    def test_one_exception_per_channel_including_fpn(self):
        self.assertEqual(zs.make_exceptions(TABLE), [
            {"knob": "zeroSuppression.threshold", "at": {"asad": 0, "aget": 0, "ch": 3}, "value": 430},
            {"knob": "zeroSuppression.threshold", "at": {"asad": 1, "aget": 2, "ch": 40}, "value": 520},
            {"knob": "zeroSuppression.threshold", "at": {"asad": 3, "aget": 3, "ch": 11}, "value": 4095},
        ])

    def test_at_key_is_ch_not_channel(self):  # src/params/schema.rs At: asad / aget / ch(deny_unknown_fields)
        for e in zs.make_exceptions(TABLE):
            self.assertEqual(sorted(e["at"]), ["aget", "asad", "ch"])


class MergeTest(unittest.TestCase):
    BASE = {
        "schema": "tpcdaq-params/1", "template": "t.xcfg",
        "knobs": {"zeroSuppression": {"enabled": False, "threshold": 600}, "triggerDelay": 1750},
        "exceptions": [
            {"knob": "zeroSuppression.threshold", "at": {"asad": 0}, "value": 700},
            {"knob": "peakingTime", "at": {"asad": 1}, "value": 502},
        ],
        "note": "keep me",
    }

    def test_merge_enables_zs_replaces_old_thresholds_keeps_rest(self):
        exc = zs.make_exceptions(TABLE)
        out = zs.merge_into(self.BASE, exc)
        self.assertTrue(out["knobs"]["zeroSuppression"]["enabled"])
        self.assertEqual(out["knobs"]["zeroSuppression"]["threshold"], 600)  # 一様値は触らない
        self.assertEqual(out["knobs"]["triggerDelay"], 1750)
        self.assertEqual(out["note"], "keep me")
        self.assertEqual(out["exceptions"][0], self.BASE["exceptions"][1])  # 無関係な例外は残る
        self.assertEqual(out["exceptions"][1:], exc)  # 古い閾値の例外は置き換わる
        self.assertEqual(len(out["exceptions"]), 4)
        self.assertFalse(self.BASE["knobs"]["zeroSuppression"]["enabled"])  # 入力は壊さない

    def test_merge_into_base_without_zs_knob_or_exceptions(self):
        out = zs.merge_into({"schema": "tpcdaq-params/1", "knobs": {}}, zs.make_exceptions(TABLE))
        self.assertEqual(out["knobs"]["zeroSuppression"], {"enabled": True})
        self.assertEqual(len(out["exceptions"]), 3)


class CliTest(unittest.TestCase):
    def run_cli(self, *args):
        return subprocess.run([sys.executable, os.path.join(HERE, "zs_exceptions.py"), *args],
                              capture_output=True, text=True, check=True).stdout

    def test_stdout_out_and_merge(self):
        with tempfile.TemporaryDirectory() as d:
            ped = os.path.join(d, "ped.json")
            base = os.path.join(d, "base.json")
            out = os.path.join(d, "out.json")
            with open(ped, "w") as f:
                json.dump(TABLE, f)
            with open(base, "w") as f:
                json.dump(MergeTest.BASE, f)
            self.assertEqual(json.loads(self.run_cli(ped)), zs.make_exceptions(TABLE))
            self.assertEqual(json.loads(self.run_cli(ped, "--k", "10")), zs.make_exceptions(TABLE, 10))
            self.assertEqual(self.run_cli(ped, "--out", out), "")
            with open(out) as f:
                self.assertEqual(json.load(f), zs.make_exceptions(TABLE))
            merged = json.loads(self.run_cli(ped, "--merge", base))
            self.assertTrue(merged["knobs"]["zeroSuppression"]["enabled"])
            self.assertEqual(len(merged["exceptions"]), 4)


if __name__ == "__main__":
    unittest.main()
