import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("analyse-whpx-browserhost.py")
SPEC = importlib.util.spec_from_file_location("analyse_whpx_browserhost", SCRIPT)
ANALYSER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ANALYSER)


class BrowserHostCampaignTest(unittest.TestCase):
    def test_reports_complete_publications_and_rectangles_by_arm(self):
        scroll = json.dumps({"click": 0, "end": 2400, "frames": list(range(10, 2401, 10))})
        callback = json.dumps({"count": 240, "scrollOnly": False, "callbackTotalMs": 12})
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for index, arm in enumerate(ANALYSER.ORDER, 1):
                trial = root / f"{index:02d}-{arm}"
                trial.mkdir()
                evidence = dict.fromkeys(ANALYSER.COMMON, "shared")
                evidence.update(dict.fromkeys(ANALYSER.ARM, arm))
                evidence.update(arm=arm, mode="FullStack", boot_commit=arm,
                                boot_sha256=arm, disk_state_sha256="same state")
                (trial / "evidence.json").write_text(json.dumps(evidence), encoding="utf-8")
                complete = int(arm == "A")
                (trial / "serial.log").write_text(
                    '1.000 WebContent(9): (js log) "WHPX_P0_BEGIN"\n'
                    '[LB:FRAME] onglet=2 seq=10 t=1100 degat=x vue=100x90 '
                    f'publie=0,36 100x90 copie_px=9000 complet={complete} present_us=100\n'
                    '[LB:FRAME] onglet=2 seq=11 t=1200 degat=x vue=100x90 '
                    'publie=0,36 10x10 copie_px=100 complet=0 present_us=200\n'
                    f'3.400 WebContent(9): (js log) "WHPX_P0_SCROLL {scroll}"\n'
                    f'3.401 WebContent(9): (js log) "WHPX_P0_CALLBACK {callback}"\n',
                    encoding="utf-8",
                )

            report = ANALYSER.campaign(root)
            last = root / "06-B" / "serial.log"
            original = last.read_text(encoding="utf-8")
            last.write_text(original.replace("seq=11", "seq=12"), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "incomplete"):
                ANALYSER.campaign(root)
            last.write_text(original.replace("vue=100x90", "vue=101x90"), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "dimensions"):
                ANALYSER.campaign(root)
            last.write_text(original, encoding="utf-8")
            for index, arm in enumerate(ANALYSER.ORDER, 1):
                info = root / f"{index:02d}-{arm}" / "evidence.json"
                evidence = json.loads(info.read_text(encoding="utf-8"))
                evidence.update(mode="CommonKernel", boot_commit="A", boot_sha256="same")
                info.write_text(json.dumps(evidence), encoding="utf-8")
            self.assertEqual(ANALYSER.campaign(root)["mode"], "CommonKernel")

        self.assertEqual(report["mode"], "FullStack")
        reference, candidate = [trial["metrics"] for trial in report["trials"][:2]]
        self.assertEqual(reference["published_rects"], {"0,36 100x90": 1, "0,36 10x10": 1})
        self.assertEqual(reference["copied_pixels"], 9100)
        self.assertEqual(reference["browserhost_interval_ms"], dict(p50=100, p95=100, p99=100, max=100))
        self.assertEqual(reference["browserhost_complete_count"], 1)
        self.assertEqual(reference["browserhost_complete_pct"], 50)
        self.assertEqual(candidate["browserhost_complete_pct"], 0)
        self.assertEqual(report["paired_dispersion_pct"]["browserhost_complete_pct"]["median"], -100)
