import tempfile
import unittest
from pathlib import Path

from logscan import scan_file, scan_text
from reproducibility import first_difference, sha256

class ReliabilityTests(unittest.TestCase):
    def test_clean_log(self):
        findings, missing = scan_text("[boot] ok\n[SMP-SAMPLE] v=2\n")
        self.assertEqual(findings, ())
        self.assertEqual(missing, ())

    def test_panic_is_fatal(self):
        findings, _ = scan_text("*** KERNEL PANIC ***\n")
        self.assertEqual(findings[0].kind, "kernel_panic")

    def test_bkl_violation_is_fatal(self):
        findings, _ = scan_text("[BKL-FR] VIOLATION try_reenter cpu=0\n")
        self.assertEqual(findings[0].kind, "bkl_violation")

    def test_lockdep_zero_violations_is_benign(self):
        findings, _ = scan_text("[LOCKDEP] acquisitions=270 violations=0 max_depth=1\n")
        self.assertEqual(findings, ())

    def test_lockdep_nonzero_violations_is_fatal(self):
        findings, _ = scan_text("[LOCKDEP] acquisitions=270 violations=2 max_depth=3\n")
        self.assertEqual(findings[0].kind, "lockdep")

    def test_bkl_uaf_diagnostic_token_is_benign(self):
        findings, _ = scan_text("[BKL-DETACHED] uaf=2 task=1 tid=101 pid=5\n")
        self.assertEqual(findings, ())

    def test_explicit_use_after_free_is_fatal(self):
        findings, _ = scan_text("use-after-free detected in task registry\n")
        self.assertEqual(findings[0].kind, "use_after_free")

    def test_required_marker(self):
        findings, missing = scan_text("hello\n", required_markers=["READY"])
        self.assertEqual(findings, ())
        self.assertEqual(missing, ("READY",))

    def test_empty_file_fails(self):
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "empty.log"
            path.write_bytes(b"")
            self.assertFalse(scan_file(path).ok)

    def test_reproducibility(self):
        with tempfile.TemporaryDirectory() as td:
            a, b = Path(td) / "a", Path(td) / "b"
            a.write_bytes(b"abcdef")
            b.write_bytes(b"abcdef")
            self.assertIsNone(first_difference(a, b))
            self.assertEqual(sha256(a), sha256(b))
            b.write_bytes(b"abcxef")
            self.assertEqual(first_difference(a, b), 3)

if __name__ == "__main__":
    unittest.main()
