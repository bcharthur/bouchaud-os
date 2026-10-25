"""Le banc C5 refuse les faux verts sans lancer QEMU dans les tests hote."""
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
RUNNER = (ROOT / 'tools/ci/run_faute_persistance.sh').read_text()
ANALYSE = RUNNER.split("<<'PY'\n", 1)[1].split('\nPY', 1)[0]


class FautePersistance(unittest.TestCase):
    def verdict(self, text):
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / 'serie.log'
            log.write_text(text)
            return subprocess.run(['python3', '-c', ANALYSE, str(log)],
                                  capture_output=True, timeout=10).returncode

    def temoin(self):
        return ('[kernel] PROCESS_FAULT pid=42\nFAUTE_MULTIFIL_OK essais=48\n'
                'PERSIST_COURSE_OK\nDMESG_INSTANTANE_DEBUT\n'
                + ''.join(f'[{i:02}] diagnostic\n' for i in range(64))
                + 'DMESG_INSTANTANE_FIN\nFAUTE_PERSISTANCE_FIN\n')

    def test_reference_and_expected_child_fault(self):
        self.assertEqual(self.verdict(self.temoin()), 0)

    def test_persistence_failure_is_not_masked_by_autorun_continuing(self):
        self.assertNotEqual(self.verdict(self.temoin().replace('PERSIST_COURSE_OK', 'PERSIST_COURSE_ECHEC')), 0)

    def test_unexpected_fault_even_with_success_marker(self):
        self.assertNotEqual(self.verdict(self.temoin().replace('PERSIST_COURSE_OK',
                            '[kernel] PROCESS_FAULT pid=64\nPERSIST_COURSE_OK')), 0)

    def test_echo_command_is_not_a_completion(self):
        self.assertNotEqual(self.verdict(self.temoin().replace('FAUTE_PERSISTANCE_FIN',
                            '+ echo FAUTE_PERSISTANCE_FIN')), 0)

    def test_missing_duplicate_and_out_of_order_records(self):
        for text in (self.temoin().replace('[63] diagnostic\n', ''),
                     self.temoin().replace('[63]', '[62]'),
                     self.temoin().replace('[00]', '[99]')):
            with self.subTest(text=text):
                self.assertNotEqual(self.verdict(text), 0)


if __name__ == '__main__':
    unittest.main()
