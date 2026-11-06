"""P13: jamais accepter SMP8 sur quatre coeurs SMT / topologie manquante."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

SOURCE = Path(__file__).resolve().parents[1] / "p13_kvm_host_preflight.py"
spec = importlib.util.spec_from_file_location("preflight_p13", SOURCE)
preflight = importlib.util.module_from_spec(spec)
spec.loader.exec_module(preflight)

class TestP13Cores(unittest.TestCase):
    def topology(self, root: Path, layout: list[tuple[int, int]]):
        for cpu, (package, core) in enumerate(layout):
            path = root / f"cpu{cpu}" / "topology"
            path.mkdir(parents=True)
            (path / "physical_package_id").write_text(str(package))
            (path / "core_id").write_text(str(core))

    def test_smt_ne_donne_pas_huit_coeurs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.topology(root, [(0, n // 2) for n in range(8)])
            cores = preflight.compte_coeurs_physiques(list(range(8)), root)
            self.assertEqual(cores, 4)
            self.assertTrue(preflight.controle(cores, None, 8, True, [], True, 0))
            self.assertEqual(preflight.controle(cores, None, 4, True, [], True, 0), [])

    def test_deux_socket_identiques_ne_se_confondent_pas(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.topology(root, [(0, 0), (0, 1), (1, 0), (1, 1)])
            self.assertEqual(preflight.compte_coeurs_physiques([0, 1, 2, 3], root), 4)

    def test_affinite_restreinte(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.topology(root, [(0, 0), (0, 0), (0, 1), (0, 1)])
            self.assertEqual(preflight.compte_coeurs_physiques([0, 2], root), 2)
            self.assertEqual(preflight.compte_coeurs_physiques([0, 1], root), 1)

    def test_topologie_manquante_refuse(self):
        with tempfile.TemporaryDirectory() as directory:
            self.assertIsNone(preflight.compte_coeurs_physiques([0], Path(directory)))
            self.assertIsNone(preflight.compte_coeurs_physiques([1], Path(directory)))

if __name__ == "__main__":
    unittest.main()
