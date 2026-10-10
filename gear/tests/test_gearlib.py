"""意味の層（gear.json）の純粋テスト。Blender は要らない。
`python3 -m unittest discover -s gear/tests` で回す。"""
import json
import os
import sys
import unittest

HERE = os.path.dirname(__file__)
sys.path.insert(0, os.path.dirname(HERE))
import gearlib  # noqa: E402

ROOT = os.path.dirname(HERE)


class Ids(unittest.TestCase):
    def test_canonical_ids_follow_inventory(self):
        self.assertEqual(gearlib.canonical_id("ncxse"), "numa")
        self.assertEqual(gearlib.canonical_id("fgdp50"), "fgdp")
        self.assertEqual(gearlib.canonical_id("lpd8"), "lpd8")

    def test_rename_keeps_model_and_blend_root(self):
        spec = {"id": "ncxse", "title": "Numa Compact X SE", "parts": [],
                "sections": [{"id": "ncxse.keys", "kind": "keys", "parts": []}]}
        out = gearlib.normalize(spec)
        self.assertEqual(out["id"], "numa")
        self.assertEqual(out["model"], "Numa Compact X SE")
        self.assertEqual(out["blend_root"], "ncxse")
        self.assertEqual(out["sections"][0]["id"], "numa.keys")

    def test_all_specs_normalize_to_their_directory(self):
        for gid in gearlib.GEAR_IDS:
            spec = gearlib.load(ROOT, gid)
            self.assertEqual(spec["id"], gid, gid)
            self.assertTrue(spec["model"], gid)


class Controls(unittest.TestCase):
    """部品名（1 始まり）→ ControlEvent（0 始まり index）。rename ではなく対応を書く"""

    def test_nanokontrol_matches_device_input(self):
        spec = gearlib.load(ROOT, "nanokontrol")
        c = gearlib.control_map(spec)
        self.assertEqual(c["fader_1"], "fader.0")
        self.assertEqual(c["knob_8"], "knob.7")
        self.assertEqual(c["s_1"], "button.0")
        self.assertEqual(c["m_1"], "button.8")
        self.assertEqual(c["r_8"], "button.23")
        self.assertEqual(c["track_prev"], "button.24")
        self.assertEqual(c["track_next"], "button.25")
        self.assertNotIn("play", c, "transport は profile が読まない = control 無し")

    def test_lpd8_pads_and_knobs(self):
        c = gearlib.control_map(gearlib.load(ROOT, "lpd8"))
        self.assertEqual(c["pad_1"], "pad.0")
        self.assertEqual(c["knob_8"], "knob.7")

    def test_xtouch_faders_and_master(self):
        c = gearlib.control_map(gearlib.load(ROOT, "xtouch"))
        self.assertEqual(c["fader_1"], "fader.0")
        self.assertEqual(c["fader_master"], "fader.8")
        self.assertNotIn("fader_1_tick_0", c)

    def test_default_rule_uses_section_order(self):
        c = gearlib.control_map(gearlib.load(ROOT, "minilab"))
        self.assertEqual(c["pad_1"], "pad.0")
        self.assertEqual(c["encoder_16"], "knob.15")

    def test_controls_are_written_into_parts(self):
        spec = gearlib.load(ROOT, "lpd8")
        by_name = {p["name"]: p for p in spec["parts"]}
        self.assertEqual(by_name["pad_1"]["control"], "pad.0")
        self.assertNotIn("control", by_name["cc"])


class Keybed(unittest.TestCase):
    def test_expanded_spec_has_keys(self):
        spec = gearlib.expanded(gearlib.load(ROOT, "keystage"))
        keys = [p for p in spec["parts"] if p["kind"].startswith("key_")]
        self.assertEqual(len(keys), 61)
        self.assertEqual(keys[0]["name"], "key_36")


class Manifest(unittest.TestCase):
    def test_manifest_hashes_every_file(self):
        import tempfile
        with tempfile.TemporaryDirectory() as d:
            for name, body in [("lpd8.usdz", b"usd"), ("lpd8.glb", b"glb"), ("lpd8.json", b"{}")]:
                with open(os.path.join(d, name), "wb") as f:
                    f.write(body)
            m = gearlib.manifest(d, source_commit="abc1234", gear_ids=["lpd8"])
            self.assertEqual(m["schema"], gearlib.SCHEMA_VERSION)
            self.assertEqual(m["source_commit"], "abc1234")
            self.assertEqual(set(m["gear"]["lpd8"]), {"usdz", "glb", "json"})
            self.assertEqual(len(m["gear"]["lpd8"]["glb"]["sha256"]), 64)


if __name__ == "__main__":
    unittest.main()
