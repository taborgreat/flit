"""Tests for flit_flp.py on the sample .flp files (FL 20.8.4).
    python -I -m unittest flp/test_flit_flp.py
"""
import json
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import flit_flp  # noqa: E402

SAMPLES = os.path.join(HERE, "samples")


class FlpTests(unittest.TestCase):
    def test_main_fixture(self):
        s = flit_flp.snapshot(os.path.join(SAMPLES, "FL 20.8.4.flp"))
        self.assertEqual(s["version"], "20.8.4.2576")
        self.assertEqual(s["ppq"], 96)
        self.assertAlmostEqual(s["tempo"], 69.42, places=2)
        self.assertEqual(len(s["channels"]), 19)
        names = [c["name"] for c in s["channels"]]
        self.assertIn("BooBass", names)
        vst = [c for c in s["channels"] if c["plugin"] == "Sylenth1"]
        self.assertEqual(len(vst), 1)
        self.assertEqual(sum(len(p["notes"]) for p in s["patterns"]), 48)
        self.assertEqual(len(s["patterns"]), 5)
        ott = [sl for i in s["inserts"] for sl in i["slots"] if sl["plugin"] == "OTT"]
        self.assertEqual(len(ott), 1)
        self.assertEqual(s["warnings"], [])
        json.dumps(s)  # serializable

    def test_minimal_fixture_and_diff(self):
        s = flit_flp.snapshot(os.path.join(SAMPLES, "multi-channel.flp"))
        self.assertAlmostEqual(s["tempo"], 140.0)
        self.assertEqual(sum(len(p["notes"]) for p in s["patterns"]), 2)
        n = s["patterns"][0]["notes"][0]
        self.assertEqual(n["key"], 60)  # C5 = middle C
        self.assertIn(n["channel"], (0, 1))
        a = flit_flp.snapshot(os.path.join(SAMPLES, "FL 20.8.4.flp"))
        d = flit_flp.diff(s, a)
        self.assertTrue(any(x.startswith("tempo") for x in d))
        self.assertTrue(any("added" in x for x in d))

    def test_cli_json(self):
        out = subprocess.run([sys.executable, "-I", os.path.join(HERE, "flit_flp.py"), os.path.join(SAMPLES, "multi-channel.flp")], capture_output=True, text=True, check=True)
        s = json.loads(out.stdout)
        self.assertEqual(s["ppq"], 96)

    def test_roundtrip_save_is_byte_identical(self):
        import pyflp
        flit_flp._patch_pyflp()
        src = os.path.join(SAMPLES, "multi-channel.flp")
        p = pyflp.parse(src)
        with tempfile.TemporaryDirectory() as d:
            out = os.path.join(d, "out.flp")
            pyflp.save(p, out)
            with open(src, "rb") as f1, open(out, "rb") as f2:
                self.assertEqual(f1.read(), f2.read())


if __name__ == "__main__":
    unittest.main()
