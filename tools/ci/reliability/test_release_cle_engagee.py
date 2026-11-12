"""BOUCHAUD_RELEASE_V1 (revue #306) : le jour ou une cle de production est engagee,
les tests de release restent verts.

Les suites release (fabrique, garde, artefacts) sont rejouees dans une COPIE
du depot dont src/kernel/update/cles_production.rs engage une vraie cle
publique. Si une suite lisait la configuration reelle du depot (au lieu de
ses fixtures), elle echouerait ici.
"""
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

RACINE = Path(__file__).resolve().parents[3]
SUITES = ("test_fabrique_release.py", "test_garde_release.py", "test_verifie_artefacts_release.py")


class SuitesIndependantesDesClesDuDepot(unittest.TestCase):
    @unittest.skipIf(os.environ.get("BOUCHAUD_TEST_COPIE_CLE"), "deja dans la copie")
    def test_suites_vertes_avec_une_cle_engagee(self):
        with tempfile.TemporaryDirectory() as t:
            copie = Path(t)
            shutil.copytree(RACINE / "tools/release", copie / "tools/release",
                            ignore=shutil.ignore_patterns("__pycache__"))
            (copie / "tools/ci/reliability").mkdir(parents=True)
            for s in SUITES:
                shutil.copy(RACINE / "tools/ci/reliability" / s, copie / "tools/ci/reliability" / s)
            (copie / "src/kernel/update").mkdir(parents=True)
            pem = copie / "k.pem"
            subprocess.run(["openssl", "genpkey", "-algorithm", "ed25519", "-out", str(pem)], check=True)
            pub = subprocess.run([sys.executable, str(copie / "tools/release/cle_publique.py"), str(pem),
                                  "--nom", "release-2026-11"], check=True, capture_output=True, text=True)
            ligne = pub.stdout.splitlines()[1]
            (copie / "src/kernel/update/cles_production.rs").write_text(
                f"pub const CLES_PRODUCTION: &[(&str, &str)] = &[\n{ligne}\n];\n")
            pem.unlink()
            r = subprocess.run([sys.executable, "-m", "unittest", "discover", "-s", "tools/ci/reliability",
                                "-p", "test_*release*.py"], cwd=copie, capture_output=True, text=True,
                               env=dict(os.environ, BOUCHAUD_TEST_COPIE_CLE="1"), timeout=300)
            self.assertEqual(r.returncode, 0, r.stderr[-3000:])
            self.assertIn("OK", r.stderr)


if __name__ == "__main__":
    unittest.main()
