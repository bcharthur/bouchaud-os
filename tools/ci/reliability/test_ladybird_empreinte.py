#!/usr/bin/env python3
"""BOUCHAUD_LADYBIRD_EMPREINTE_V1 : empreinte des entrees du build Ladybird."""
import importlib.util
import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location("ladybird_empreinte", ROOT / "tools" / "ci" / "ladybird_empreinte.py")
le = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(le)

WORKFLOW = """name: ladybird-native-browser
on:
  workflow_dispatch:
jobs:
  dedoublonne:
    runs-on: ubuntu-latest
    steps:
      - run: echo ok
  build:
    needs: dedoublonne
    if: ${{ !cancelled() }}
    name: ladybird / build once
    runs-on: ubuntu-24.04
    timeout-minutes: 180
    steps:
      - uses: actions/checkout@v4
      - name: Regressions des analyseurs de preuves runtime
        run: python3 tools/ci/verifie-preuve-compositor.py
      - name: Toolchain C/C++
        id: toolchain
        run: |
          tools/ci/apt-installe.sh clang
      - name: Verifier la reutilisation des sources
        run: python3 tools/ladybird/test-incremental-source.py
      - name: Dependances navigateur
        run: tools/ladybird/browser-vcpkg.sh
      - name: Statistiques ccache avant
        run: ccache -s || true
      - name: Construire Ladybird
        run: |
          tools/ladybird/native-browser-final.sh --cible
      - name: Diagnostics de compilation
        if: failure()
        run: echo diag
      - name: Marquer la capacite UI V16
        run: |
          printf 'V16\\n' > third_party/native-browser-bouchaud/V16_UI_CAPABLE
      - name: Marquer la capacite UI V19
        run: |
          printf 'V19\\n' > third_party/native-browser-bouchaud/V19_UI_CAPABLE
      - name: Statistiques ccache apres
        if: always()
        run: ccache -s || true
      - name: Verifier les artefacts runtime
        run: test -d third_party/native-browser-bouchaud
      - name: Manifeste reproductible de l'artefact
        run: python3 tools/ladybird/artifact_manifest.py create
      - name: Publier Ladybird natif
        uses: actions/upload-artifact@v4
      - name: Corpus WPT
        run: python3 tools/ladybird/wpt/prepare-corpus.py a b
  wpt:
    needs: build
    runs-on: ubuntu-24.04
    steps:
      - run: echo wpt
"""

FICHIERS = {
    "tools/ladybird/native-browser-final.sh": "#!/bin/bash\n./tools/ladybird/browser-upstream.sh\n",
    "tools/ladybird/browser-upstream.sh": "#!/bin/bash\n# commentaire : tools/ci/ailleurs.sh\necho build\n",
    "tools/ladybird/browser-vcpkg.sh": "#!/bin/bash\necho vcpkg\n",
    "tools/ladybird/artifact_manifest.py": "print('manifeste')\n",
    "tools/ladybird/test-incremental-source.py": "print('ok')\n",
    "tools/ladybird/wpt/prepare-corpus.py": "print('corpus')\n",
    "third_party/UPSTREAM.md": "    sha     " + "a" * 40 + "\n",
    "tools/ci/apt-installe.sh": "#!/bin/bash\napt-get install \"$@\"\n",
    "tools/verifie-chain-ladybird.py": "ATTENDUS = ['tools/ladybird/disparu.py', 'tools/ci/donnee.sh']\n",
    "src/kernel/noyau.rs": "fn main() {}\n",
    ".github/workflows/ladybird-native-browser.yml": WORKFLOW,
}


class Depot:
    def __init__(self):
        self.td = tempfile.TemporaryDirectory()
        self.racine = Path(self.td.name)
        self.git("init", "-q")
        self.git("config", "user.email", "t@t")
        self.git("config", "user.name", "t")
        for chemin, contenu in FICHIERS.items():
            self.ecrit(chemin, contenu)
        self.commit("initial")

    def git(self, *args):
        return subprocess.run(["git", *args], cwd=self.racine, check=True, capture_output=True,
                              text=True).stdout.strip()

    def ecrit(self, chemin, contenu):
        p = self.racine / chemin
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(contenu, encoding="utf-8")

    def remplace(self, chemin, avant, apres):
        p = self.racine / chemin
        texte = p.read_text(encoding="utf-8")
        assert texte.count(avant) == 1, (chemin, avant)
        p.write_text(texte.replace(avant, apres), encoding="utf-8")

    def commit(self, message):
        self.git("add", "-A")
        self.git("commit", "-q", "--allow-empty", "-m", message)
        return self.git("rev-parse", "HEAD")


class EmpreinteTests(unittest.TestCase):
    def setUp(self):
        self.depot = Depot()
        self.ancien = os.getcwd()
        os.chdir(self.depot.racine)

    def tearDown(self):
        os.chdir(self.ancien)
        self.depot.td.cleanup()

    def e(self, commit="HEAD"):
        return le.empreinte(commit)["empreinte"]

    def change(self, chemin, avant, apres):
        base = self.e()
        self.depot.remplace(chemin, avant, apres)
        self.depot.commit("change")
        return base, self.e()

    def test_le_noyau_ne_change_pas_l_empreinte(self):
        base, apres = self.change("src/kernel/noyau.rs", "fn main() {}", "fn main() { let _x = 1; }")
        self.assertEqual(base, apres)

    def test_orchestration_et_commentaires_ne_changent_pas_l_empreinte(self):
        base = self.e()
        self.depot.remplace(".github/workflows/ladybird-native-browser.yml",
                            "      - name: Construire Ladybird\n        run: |",
                            "      # Commentaire nouveau.\n      - name: Construire Ladybird\n"
                            "        timeout-minutes: 150\n        if: ${{ !cancelled() }}\n        run: |")
        self.depot.remplace(".github/workflows/ladybird-native-browser.yml",
                            "    timeout-minutes: 180\n    steps:", "    timeout-minutes: 200\n    steps:")
        self.depot.remplace(".github/workflows/ladybird-native-browser.yml",
                            "  wpt:\n    needs: build", "  wpt:\n    needs: artefact")
        self.depot.commit("orchestration")
        self.assertEqual(base, self.e())

    def test_chaque_entree_change_l_empreinte(self):
        cas = [
            ("tools/ladybird/browser-upstream.sh", "echo build", "echo build -O3"),
            ("third_party/UPSTREAM.md", "a" * 40, "b" * 40),
            ("tools/ci/apt-installe.sh", "apt-get install", "apt-get install -y"),
            (".github/workflows/ladybird-native-browser.yml", "tools/ci/apt-installe.sh clang",
             "tools/ci/apt-installe.sh clang lld"),
            (".github/workflows/ladybird-native-browser.yml", "printf 'V19\\n'", "printf 'V19 onglets=1\\n'"),
            (".github/workflows/ladybird-native-browser.yml", "runs-on: ubuntu-24.04\n    timeout-minutes: 180",
             "runs-on: ubuntu-22.04\n    timeout-minutes: 180"),
        ]
        for chemin, avant, apres in cas:
            with self.subTest(chemin=chemin, apres=apres):
                base, nouveau = self.change(chemin, avant, apres)
                self.assertNotEqual(base, nouveau)

    def test_nouveau_fichier_et_mode_executable_changent_l_empreinte(self):
        base = self.e()
        self.depot.ecrit("tools/ladybird/patches/nouveau.patch", "+x\n")
        self.depot.commit("ajout")
        apres_ajout = self.e()
        self.assertNotEqual(base, apres_ajout)
        os.chmod(self.depot.racine / "tools/ladybird/browser-vcpkg.sh", 0o755)
        self.depot.commit("mode")
        self.assertNotEqual(apres_ajout, self.e())

    def test_etape_run_non_classee_est_rouge(self):
        self.depot.remplace(".github/workflows/ladybird-native-browser.yml",
                            "      - name: Corpus WPT\n",
                            "      - name: Etape inconnue\n        run: echo x > third_party/native-browser-bouchaud/X\n"
                            "      - name: Corpus WPT\n")
        self.depot.commit("inconnue")
        with self.assertRaisesRegex(le.EmpreinteErreur, "non classee"):
            self.e()
        self.assertTrue(any("non classee" in e for e in le.garde()))

    def test_etape_productrice_disparue_est_rouge(self):
        self.depot.remplace(".github/workflows/ladybird-native-browser.yml",
                            "      - name: Marquer la capacite UI V19\n", "      - name: Marquer V19\n")
        self.depot.commit("renommee")
        with self.assertRaises(le.EmpreinteErreur):
            self.e()

    def test_garde_refuse_une_lecture_hors_empreinte(self):
        self.assertEqual(le.garde(), [])
        self.depot.remplace("tools/ladybird/browser-upstream.sh", "echo build\n",
                            "echo build\npython3 tools/ci/genere.py\n")
        self.depot.commit("lecture")
        self.assertTrue(any("tools/ci/genere.py" in e for e in le.garde()))

    def test_garde_refuse_un_source_noyau_lu_par_la_recette(self):
        self.depot.remplace("tools/ladybird/browser-upstream.sh", "echo build\n",
                            'echo build\ncp "$ROOT/src/kernel/noyau.rs" /tmp/x\n')
        self.depot.commit("noyau")
        self.assertTrue(any("$ROOT/src" in e for e in le.garde()))

    def test_garde_suit_les_appels_transitifs(self):
        self.depot.ecrit("tools/ladybird/etape2.sh", "#!/bin/bash\nbash tools/ci/cache.sh\n")
        self.depot.remplace("tools/ladybird/browser-upstream.sh", "echo build\n",
                            "echo build\n./tools/ladybird/etape2.sh\n")
        self.depot.commit("transitif")
        self.assertTrue(any("tools/ci/cache.sh" in e for e in le.garde()))

    def test_lie_accepte_d_autres_heads_aux_memes_entrees(self):
        producteur = self.depot.git("rev-parse", "HEAD")
        self.depot.remplace("src/kernel/noyau.rs", "fn main() {}", "fn main() { loop {} }")
        self.depot.commit("noyau")
        m = self.depot.racine / "m.json"
        m.write_text(json.dumps({"producer": {"bouchaud_head": producteur}}), encoding="utf-8")
        source, courant = le.lie(m)
        self.assertEqual(source["commit"], producteur)
        self.assertNotEqual(courant["commit"], producteur)

    def test_lie_refuse_des_entrees_differentes(self):
        producteur = self.depot.git("rev-parse", "HEAD")
        self.depot.remplace("tools/ladybird/browser-upstream.sh", "echo build", "echo autre")
        self.depot.commit("recette")
        m = self.depot.racine / "m.json"
        m.write_text(json.dumps({"producer": {"bouchaud_head": producteur}}), encoding="utf-8")
        with self.assertRaisesRegex(le.EmpreinteErreur, "entrees divergentes"):
            le.lie(m)

    def test_lie_refuse_un_manifeste_sans_producteur(self):
        m = self.depot.racine / "m.json"
        for contenu in ({}, {"producer": {"bouchaud_head": "x" * 40}}, {"producer": {"bouchaud_head": None}}):
            m.write_text(json.dumps(contenu), encoding="utf-8")
            with self.assertRaises(le.EmpreinteErreur):
                le.lie(m)

    def test_entree_obligatoire_absente_est_rouge(self):
        (self.depot.racine / "third_party/UPSTREAM.md").unlink()
        self.depot.commit("sans upstream")
        with self.assertRaisesRegex(le.EmpreinteErreur, "UPSTREAM"):
            self.e()


class LiaisonTests(unittest.TestCase):
    def test_liaison_inconnue_ou_absente_est_rouge(self):
        for args in ([], ["head"], ["entree"], ["tete; true"]):
            with self.subTest(args=args):
                r = subprocess.run(["bash", str(ROOT / "tools/ci/verifie_artefact_ladybird.sh"), *args, "/nulle/part"],
                                   cwd=ROOT, capture_output=True, text=True)
                self.assertNotEqual(r.returncode, 0)
                self.assertNotIn("LADYBIRD_ARTEFACT_VERIFIE", r.stdout)


if __name__ == "__main__":
    unittest.main()
