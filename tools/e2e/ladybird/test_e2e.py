#!/usr/bin/env python3
"""Tests de l'orchestrateur E2E (BOUCHAUD_E2E_LADYBIRD_V1), sans QEMU.

    python3 -m unittest discover -s tools/e2e/ladybird -p 'test_e2e.py' -v

Ils protegent ce qui ferait mentir le banc : une definition incoherente (une
metrique qui cite une etape inexistante serait toujours `null`), une touche
logique mal traduite en AZERTY (Ctrl+W tape sur la touche Z), un controle de
pixels qui laisserait passer un ecran vide, un disque prepare qui garderait
l'autostart (le banc ne testerait plus l'ouverture par le menu), ou WHPX lance
avec `-cpu max`.
"""
import json
import subprocess
import sys
import tarfile
import tempfile
import threading
import unittest
import urllib.request
from pathlib import Path
from types import SimpleNamespace

ICI = Path(__file__).resolve().parent
sys.path.insert(0, str(ICI))
import e2e  # noqa: E402
import fixture  # noqa: E402

ACTIONS = {"attend", "absent", "menu", "touches", "tape", "pause_s", "capture", "fixture_vu"}


class Definition(unittest.TestCase):
    def setUp(self):
        self.d = json.loads((ICI / "scenarios.json").read_text(encoding="utf-8"))

    def test_identifiants_uniques_et_prerequis_existants(self):
        ids = [s["id"] for s in self.d["scenarios"]]
        self.assertEqual(len(ids), len(set(ids)))
        for s in self.d["scenarios"]:
            for r in s.get("requiert", []):
                self.assertIn(r, ids, f"{s['id']} requiert {r}")

    def test_toutes_les_etapes_sont_connues(self):
        for s in self.d["scenarios"]:
            for e in s.get("etapes", []):
                self.assertTrue(ACTIONS & e.keys(), f"{s['id']} : etape sans action connue {e}")
                if "couleur" in e:
                    self.assertIn(e["couleur"], fixture.COULEURS)

    def test_metriques_citent_des_etapes_nommees(self):
        for s in self.d["scenarios"]:
            noms = {e["nom"] for e in s.get("etapes", []) if "nom" in e} | {"lancement_qemu"}
            for m in s.get("metriques", []):
                if "de" in m:
                    self.assertIn(m["de"], noms, f"{s['id']} {m['nom']}")
                    self.assertIn(m["a"], noms, f"{s['id']} {m['nom']}")

    def test_les_dix_parcours_sont_declares_et_les_absents_disent_pourquoi(self):
        ids = {s["id"] for s in self.d["scenarios"]}
        for n in range(1, 11):
            self.assertIn(f"E2E-{n:02d}", ids)
        for s in self.d["scenarios"]:
            if "etapes" not in s:
                self.assertTrue(s.get("non_implemente"), s["id"])


class Clavier(unittest.TestCase):
    def test_raccourcis_logiques_en_touches_physiques_azerty(self):
        self.assertEqual(e2e.combinaison("ctrl-w"), "ctrl-z")
        self.assertEqual(e2e.combinaison("ctrl-l"), "ctrl-l")
        self.assertEqual(e2e.combinaison("ctrl-t"), "ctrl-t")
        self.assertEqual(e2e.combinaison("alt-left"), "alt-left")
        self.assertEqual(e2e.combinaison("ret"), "ret")

    def test_saisie_d_une_url(self):
        t = e2e.touches_de("https://a.com")
        self.assertEqual(t[:5], ["h", "t", "t", "p", "s"])
        self.assertIn("dot", t)            # ':'
        self.assertIn("shift-dot", t)      # '/'
        self.assertIn("q", t)              # 'a' AZERTY
        with self.assertRaises(ValueError):
            e2e.touches_de("é")


class Pixels(unittest.TestCase):
    def test_png_et_ppm(self):
        l, h = 4, 3
        rgb = bytes(fixture.COULEURS["accueil"]) * (l * h)
        with tempfile.TemporaryDirectory() as t:
            ppm = Path(t) / "a.ppm"
            ppm.write_bytes(b"P6\n# c\n4 3\n255\n" + rgb)
            ll, hh, lu = e2e.lit_ppm(ppm)
            self.assertEqual((ll, hh, lu), (l, h, rgb))
            e2e.ecrit_png(Path(t) / "a.png", l, h, rgb)
            self.assertTrue((Path(t) / "a.png").read_bytes().startswith(b"\x89PNG"))

    def test_bloc_de_couleur_et_ecran_change(self):
        fond = bytes((255, 255, 255)) * 100
        bloc = bytes(fixture.COULEURS["resultats"]) * 100
        self.assertEqual(e2e.pixels_de_couleur(fond, fixture.COULEURS["resultats"]), 0)
        self.assertEqual(e2e.pixels_de_couleur(bloc, fixture.COULEURS["resultats"]), 100)
        self.assertEqual(e2e.part_differente(fond, fond), 0.0)
        self.assertEqual(e2e.part_differente(fond, bloc), 1.0)


class Qemu(unittest.TestCase):
    def test_whpx_jamais_cpu_max_tcg_max_kvm_host(self):
        o = SimpleNamespace(memoire=4096, smp=4, boot="b", disque="d", accel="whpx", whpx_irqchip="off")
        a = e2e.arguments_qemu(o, Path("s.log"), 4444)
        self.assertIn(e2e.CPU_WHPX, a)
        self.assertNotIn("max", a)
        self.assertIn("format=raw,file=d,snapshot=on", a)
        o.accel = "tcg"
        self.assertIn("max", e2e.arguments_qemu(o, Path("s.log"), 4444))
        o.accel = "kvm"
        self.assertIn("host", e2e.arguments_qemu(o, Path("s.log"), 4444))


class Disque(unittest.TestCase):
    def test_image_utilisateur_autostart_retire_url_remplacee_source_intacte(self):
        with tempfile.TemporaryDirectory() as t:
            t = Path(t)
            (t / "autorun").write_text("uname\nexport BO_AUTOSTART_BROWSER=1\nexport BOUCHAUD_M9_URL='https://x'\ndesktop\n")
            src = t / "src.img"
            with tarfile.open(src, "w", format=tarfile.USTAR_FORMAT) as tar:
                tar.add(t / "autorun", arcname="./autorun")
            avant = e2e.sha256(src)
            sortie = t / "e2e.img"
            subprocess.run([sys.executable, "-I", str(ICI / "prepare_disque.py"), "--image", str(src),
                            "--url", "http://10.0.2.2:18300/accueil.html", "--sortie", str(sortie)], check=True,
                           capture_output=True)
            self.assertEqual(e2e.sha256(src), avant)
            self.assertEqual(sortie.stat().st_size % 512, 0)
            autorun = tarfile.open(sortie).extractfile("./autorun").read().decode()
            self.assertNotIn("BO_AUTOSTART_BROWSER", autorun)
            self.assertIn("export BOUCHAUD_M9_URL='http://10.0.2.2:18300/accueil.html'\ndesktop", autorun)
            self.assertNotIn("https://x", autorun)


class Fixture(unittest.TestCase):
    def test_requete_echappee_et_journalisee(self):
        with tempfile.TemporaryDirectory() as t:
            journal = Path(t) / "j.jsonl"
            fixture.Gestionnaire.journal = str(journal)
            journal.write_text("")
            serveur = fixture.ThreadingHTTPServer(("127.0.0.1", 0), fixture.Gestionnaire)
            threading.Thread(target=serveur.serve_forever, daemon=True).start()
            try:
                url = f"http://127.0.0.1:{serveur.server_address[1]}/resultats?q=%3Cscript%3E"
                corps = urllib.request.urlopen(url, timeout=10).read().decode()
            finally:
                serveur.shutdown()
            self.assertNotIn("<script>alert", corps)
            self.assertIn("&lt;script&gt;", corps)
            self.assertIn(fixture.hexa(fixture.COULEURS["resultats"]), corps)
            self.assertEqual(json.loads(journal.read_text().splitlines()[0])["q"], "<script>")


if __name__ == "__main__":
    unittest.main()
