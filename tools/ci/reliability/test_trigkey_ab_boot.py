"""Preflight QA: QEMU boot evidence cannot be mistaken for physical readiness."""
import importlib.util
from pathlib import Path
import unittest

path = Path(__file__).resolve().parents[2] / "release" / "verify_trigkey_ab_boot.py"
spec = importlib.util.spec_from_file_location("trigkey_boot", path)
assert spec and spec.loader
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)
MANIFEST = {"proof_level": "BUILD_ONLY", "physical": "NOT_RUN", "commit": "a" * 40}
GOOD = (
    "BOUCHAUD_AB_BOOT slot=A\n"
    "BOUCHAUD_HWPROBE_BEGIN\n"
    "SERVICED_UPDATE_STATE rc=0 active_slot=A\n"
    "SERVICED_HEALTH_COMMIT result=already_good\n"
)


class TrigkeyBootProof(unittest.TestCase):
    def test_valid_qemu_never_becomes_physical(self):
        proof = mod.inspect(GOOD, MANIFEST)
        self.assertEqual(proof["qemu_boot"], "PASS")
        self.assertEqual(proof["physical"], "NOT_RUN")
        self.assertFalse(proof["release_approved"])

    def test_no_boot_or_no_health_is_rejected(self):
        for sample in ("", "BOUCHAUD_AB_BOOT slot=A\n", GOOD.replace("result=already_good", "")):
            with self.subTest(sample=sample[:20]), self.assertRaises(ValueError):
                mod.inspect(sample, MANIFEST)

    def test_red_even_if_success_markers_present_before_panic(self):
        with self.assertRaises(ValueError):
            mod.inspect(GOOD + "*** KERNEL PANIC ***\n", MANIFEST)
        with self.assertRaises(ValueError):
            mod.inspect(GOOD + "BOUCHAUD_AB_BOOT slot=B\n", MANIFEST)

    def test_state_error_and_unrelated_healthy_message_do_not_pass(self):
        with self.assertRaises(ValueError):
            mod.inspect(GOOD.replace("rc=0", "rc=7"), MANIFEST)
        with self.assertRaises(ValueError):
            mod.inspect(GOOD.replace("SERVICED_HEALTH_COMMIT ", "OTHER_MODULE "), MANIFEST)

    def test_unverified_image_cannot_be_promoted(self):
        with self.assertRaises(ValueError):
            mod.inspect(GOOD, {**MANIFEST, "physical": "PASS"})
        with self.assertRaises(ValueError):
            mod.inspect(GOOD, {**MANIFEST, "commit": "main"})


if __name__ == "__main__":
    unittest.main()
