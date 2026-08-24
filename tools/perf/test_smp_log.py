import tempfile
import unittest
from pathlib import Path
from smp_log import parse, summarize

SAMPLE = "[SMP-SAMPLE] v=1 t_ns=100 window_ns=1000000000 load=[80,20] runnable=[1,1] rq=[0,0] ctx_delta=10 mig_delta=2 steal_ok_delta=[1,0] steal_try_delta=[2,1] steal_rej_bal_delta=[1,1] steal_rej_aff_delta=[0,0] bkl_wait_delta_ns=1000 bkl_hold_delta_ns=2000 bkl_acq_delta=3 pf_delta=[4,5] tlb_delta=1\r\n"

class LogEncodingTests(unittest.TestCase):
    def check_encoding(self, payload):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "run.log"
            path.write_bytes(payload)
            smp, _, _, _ = parse(path)
            self.assertEqual(len(smp), 1)
            self.assertEqual(summarize(path)["mig_s"], 2)

    def test_utf8(self): self.check_encoding(SAMPLE.encode("utf-8"))
    def test_utf8_bom(self): self.check_encoding(b"\xef\xbb\xbf" + SAMPLE.encode("utf-8"))
    def test_utf16le_bom(self): self.check_encoding(b"\xff\xfe" + SAMPLE.encode("utf-16-le"))
    def test_utf16le_without_bom(self): self.check_encoding(SAMPLE.encode("utf-16-le"))

    def test_malformed_sample_is_not_silent(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "bad.log"
            path.write_text("[SMP-SAMPLE] broken\n", encoding="utf-8")
            with self.assertRaises(ValueError):
                parse(path)

    def test_mm_lifetime_absent_is_unavailable(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "old.log"
            path.write_text(SAMPLE, encoding="utf-8")
            self.assertIsNone(summarize(path)["mm_lifetime"]["fault_registry_peak"])

    def test_mm_lifetime_fields_are_parsed(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "ng6.log"
            path.write_text(SAMPLE + "[MM-NG6] fault_retry=7 fault_registry_current=2 fault_registry_peak=9 ata_wait_ns=44\n", encoding="utf-8")
            lifetime = summarize(path)["mm_lifetime"]
            self.assertEqual(lifetime["fault_retry"], 7)
            self.assertEqual(lifetime["fault_registry_peak"], 9)
            self.assertIsNone(lifetime["exec_max_ns"])

if __name__ == "__main__": unittest.main()
