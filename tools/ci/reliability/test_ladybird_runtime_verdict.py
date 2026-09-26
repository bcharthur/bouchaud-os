#!/usr/bin/env python3
import importlib.util
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location(
    "runtime_verdict", ROOT / "tools" / "ci" / "ladybird_runtime_verdict.py"
)
v = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(v)


def good_log():
    return "\n".join(v.REQUIRED.values()) + "\nWORKER_ETAPE t=1 etape=pret\n"


class RuntimeVerdictTests(unittest.TestCase):
    def test_good(self):
        r = v.analyse(good_log(), "ok")
        self.assertTrue(r["ok"])

    def test_worker_missing(self):
        text = good_log().replace(v.REQUIRED["worker_http"], "")
        r = v.analyse(text, "ok")
        self.assertFalse(r["ok"])
        self.assertFalse(r["checks"]["worker_http"])

    def test_surface_missing(self):
        self.assertFalse(v.analyse(good_log(), "absente")["ok"])

    def test_fatal(self):
        self.assertFalse(v.analyse(good_log() + "*** KERNEL PANIC ***\n", "ok")["ok"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
