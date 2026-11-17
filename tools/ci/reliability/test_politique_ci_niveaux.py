"""BOUCHAUD_CI_TROIS_NIVEAUX_V1 : le contrat de CI echoue sur chaque regression.

Chaque test copie `.github/` et `tools/ci/` dans un dossier temporaire, y
introduit UNE regression du decoupage en niveaux, et exige que
`check_pipeline_policy.py` echoue en la nommant. Un contrat qui resterait vert
ici ne protegerait rien.
"""
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

RACINE = Path(__file__).resolve().parents[3]


class PolitiqueNiveaux(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="politique-ci-"))
        shutil.copytree(RACINE / ".github", self.tmp / ".github")
        shutil.copytree(RACINE / "tools" / "ci", self.tmp / "tools" / "ci")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def politique(self):
        r = subprocess.run([sys.executable, "tools/ci/check_pipeline_policy.py"], cwd=self.tmp,
                           capture_output=True, text=True, timeout=60)
        return r.returncode, r.stdout + r.stderr

    def modifie(self, nom, ancien, nouveau, regex=False):
        chemin = self.tmp / ".github" / "workflows" / nom
        texte = chemin.read_text(encoding="utf-8")
        neuf = re.sub(ancien, nouveau, texte, count=1, flags=re.M) if regex else texte.replace(ancien, nouveau, 1)
        self.assertNotEqual(texte, neuf, f"mutation sans effet sur {nom}")
        chemin.write_text(neuf, encoding="utf-8")

    def test_reference_verte(self):
        code, sortie = self.politique()
        self.assertEqual(code, 0, sortie)
        self.assertIn("CI_POLICY_OK", sortie)

    def test_ci_fast_redeclenche_sur_pr(self):
        self.modifie("ci.yml", "on:\n  push:\n", "on:\n  pull_request:\n  push:\n")
        code, sortie = self.politique()
        self.assertNotEqual(code, 0)
        self.assertIn("ci.yml: declencheur pull_request", sortie)

    def test_ci_fast_filtre_par_chemins(self):
        self.modifie("ci.yml", "    branches: ['**']\n", "    branches: ['**']\n    paths: ['src/**']\n")
        code, sortie = self.politique()
        self.assertNotEqual(code, 0)
        self.assertIn("push de TOUTE branche", sortie)

    def test_integration_double_push_et_pr(self):
        self.modifie("integration.yml", r"^  push:\n    branches: \[main\]", "  push:\n    branches: [main, 'claude/**']",
                     regex=True)
        code, sortie = self.politique()
        self.assertNotEqual(code, 0)
        self.assertIn("integration.yml: pull_request ET push de branches de travail", sortie)

    def test_ladybird_double_push_et_pr(self):
        self.modifie("ladybird-native-browser.yml", r"^    branches: \[main\].*$", "    branches: [main, 'feat/**']",
                     regex=True)
        code, sortie = self.politique()
        self.assertNotEqual(code, 0)
        self.assertIn("ladybird-native-browser.yml: pull_request ET push", sortie)

    def test_integration_derriere_un_libelle(self):
        self.modifie("integration.yml", "INTEGRATION_GATE_DOCS_ONLY", "INTEGRATION_NON_DEMANDEE")
        code, sortie = self.politique()
        self.assertNotEqual(code, 0)
        self.assertIn("INTEGRATION_GATE_DOCS_ONLY", sortie)

    def test_p10_apres_le_noyau_par_defaut(self):
        chemin = self.tmp / ".github" / "workflows" / "ci.yml"
        texte = chemin.read_text(encoding="utf-8")
        p10 = texte.index("      - name: P10 GPU - compilation avec et sans compositeur VIRGL")
        noyau = texte.index("      - name: Compiler le noyau")
        gardes = texte.index("      - name: Garde-fous d'architecture")
        bloc_p10, bloc_noyau = texte[p10:noyau], texte[noyau:gardes]
        chemin.write_text(texte[:p10] + bloc_noyau + bloc_p10 + texte[gardes:], encoding="utf-8")
        code, sortie = self.politique()
        self.assertNotEqual(code, 0)
        self.assertIn("P10 construit APRES", sortie)

    def test_temoins_des_declencheurs(self):
        r = subprocess.run([sys.executable, str(RACINE / "tools" / "ci" / "declencheurs.py")],
                           capture_output=True, text=True, timeout=30)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)


if __name__ == "__main__":
    unittest.main()
