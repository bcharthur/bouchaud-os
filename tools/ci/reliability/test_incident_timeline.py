"""Offline regression for Bouchaud serial incident timeline."""
import importlib.util
from pathlib import Path
import unittest
import tempfile

PATH = Path(__file__).resolve().parents[2] / "health" / "incident_timeline.py"
SPEC = importlib.util.spec_from_file_location("incident_timeline", PATH)
assert SPEC is not None and SPEC.loader is not None
timeline = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(timeline)


class TimelineTests(unittest.TestCase):
    def test_usb_and_health_chain_is_timeline_not_invented_cause(self):
        lines = [
            "BOUCHAUD_BOOT_RECORD boot_id=0x7a42 system_slot=A\n",
            "BOUCHAUD_USB_STOCKAGE_IO_ECHEC slot=1 erreur=bot-csw-echeance\n",
            "BOUCHAUD_USB_BOT_REPRISE_OK slot=1 index=2\n",
            "SERVICED_UPDATE_STATE rc=-13 active_slot=0 state_b=1\n",
            "SERVICED_HEALTH_COMMIT rc=-13 result=refused\n",
        ]
        report = timeline.decode_lines(lines)
        self.assertEqual(report["events_kept"], 4)
        self.assertEqual([e["kind"] for e in report["events"]],
                         ["usb_io_failure", "usb_bot_recovery",
                          "update_state_error", "health_commit_refused"])
        self.assertTrue(all(e["boot_id"] == "0x7a42" for e in report["events"]))
        self.assertIn("not-inferred", report["causality"])

    def test_logd_loss_and_sensitive_values_not_exported(self):
        lines = [
            "LOGD ts_ns=123 boot=3 sev=4 svc=updated pid=7 type=retry dropped=5 token=SECRET\n",
            "user password=SECRET TOKEN=x cookie=SECRET\n",
        ]
        report = timeline.decode_lines(lines)
        self.assertEqual(report["logd_declared_losses"], 5)
        self.assertEqual(report["events_kept"], 1)
        self.assertNotIn("SECRET", str(report))
        self.assertNotIn("token", report["events"][0]["fields"])

    def test_bounded_output_must_report_truncation(self):
        report = timeline.decode_lines(["*** KERNEL PANIC *** cpu=1\n"] * 4, max_events=2)
        self.assertEqual(report["events_kept"], 2)
        self.assertEqual(report["event_counts"]["scheduler_panic"], 4)
        self.assertTrue(report["events_truncated"])

    def test_unknown_messages_never_copied_to_report(self):
        report = timeline.decode_lines(["arbitrary terminal: secret text\n"])
        self.assertEqual(report["events_kept"], 0)
        self.assertNotIn("secret", str(report))

    def test_ab_boot_directory_is_separated_and_ordered(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp)
            (p / "boot-10.log").write_text(
                "BOUCHAUD_AB_BOOT slot=B\nSERVICED_UPDATE_STATE rc=0 token=SECRET\n",
                encoding="utf-8")
            (p / "boot-2.log").write_text(
                "BOUCHAUD_USB_STOCKAGE_IO_ECHEC slot=2 erreur=bot-csw-echeance\n"
                "SERVICED_UPDATE_STATE rc=-13 secret=SECRET\n", encoding="utf-8")
            out = timeline.decode_boot_directory(p)
        self.assertEqual(out["boot_numbers"], [2, 10])
        self.assertEqual(out["boot_count"], 2)
        self.assertTrue(out["boots"][0]["critical_observed"])
        self.assertFalse(out["boots"][1]["critical_observed"])
        self.assertEqual(out["boots"][0]["first_critical_line"], 1)
        self.assertIn("not-inferred", out["causality"])
        self.assertNotIn("SECRET", str(out))

    def test_directory_rejects_missing_and_symlinked_logs(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp)
            with self.assertRaises(ValueError):
                timeline.decode_boot_directory(p)
            (p / "boot-1.log").write_text("BOUCHAUD_AB_BOOT slot=A\n")
            (p / "boot-2.log").symlink_to(p / "boot-1.log")
            with self.assertRaises(ValueError):
                timeline.decode_boot_directory(p)

    def test_directory_reports_truncation_without_green_claim(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp)
            (p / "boot-1.log").write_text("*** KERNEL PANIC *** cpu=1\n" * 5)
            out = timeline.decode_boot_directory(p, max_events=2)
        self.assertTrue(out["evidence_truncated"])
        self.assertEqual(out["event_counts"]["scheduler_panic"], 5)
        self.assertEqual(out["boots"][0]["events_kept"], 2)



if __name__ == "__main__":
    unittest.main()
