"""BOUCHAUD_LADYBIRD_PRODUCTION_V1 : aucun workflow ne prend « le dernier »
artefact Ladybird du depot.

Regression du run 42895079434 (GPU composition, branche c22/p13/p10) : le
job prenait l'artefact le plus recent du depot (empreinte ebc2d030, branche
de preuve) au lieu de celui de son SHA (6199f460) ; la verification l'a
refuse, a juste titre. Regle : tout telechargement de l'artefact Ladybird
depuis UN AUTRE run prend son `run-id` d'une decision de provenance
(`ladybird_provenance.py`), et est suivi de la verification `entrees`.
"""
import re
import unittest
from pathlib import Path

import yaml

RACINE = Path(__file__).resolve().parents[3]
ARTEFACT = "bouchaud-ladybird-native-browser"


def telechargements(w):
    """[(job, index, etape, etapes)] des download-artifact de l'artefact Ladybird avec run-id."""
    res = []
    for jid, job in (w.get("jobs") or {}).items():
        etapes = (job or {}).get("steps") or []
        for i, e in enumerate(etapes):
            avec = e.get("with") or {}
            if str(e.get("uses", "")).startswith("actions/download-artifact") and avec.get("name") == ARTEFACT \
                    and "run-id" in avec:
                res.append((jid, i, e, etapes))
    return res


class SelectionTests(unittest.TestCase):
    def setUp(self):
        self.workflows = {f.name: (f.read_text(encoding="utf-8"), yaml.safe_load(f.read_text(encoding="utf-8")))
                          for f in sorted((RACINE / ".github/workflows").glob("*.yml"))}

    def test_aucune_selection_au_plus_recent(self):
        for nom, (texte, _) in self.workflows.items():
            with self.subTest(nom):
                self.assertNotRegex(texte, r"artifacts\?name=" + ARTEFACT + r"[^\n]*\n[^\n]*sort_by\(\.created_at\)\s*\|\s*last")

    def test_run_id_issu_d_une_decision_de_provenance_puis_verifie(self):
        vus = 0
        for nom, (texte, w) in self.workflows.items():
            for jid, i, e, etapes in telechargements(w):
                vus += 1
                with self.subTest(f"{nom}:{jid}"):
                    rid = str(e["with"]["run-id"])
                    sources = re.findall(r"needs\.([\w-]+)\.outputs\.[\w-]+", rid)
                    self.assertTrue(sources, f"run-id {rid!r} ne vient pas d'une decision de provenance")
                    for src in sources:
                        corps = yaml.safe_dump(w["jobs"][src])
                        self.assertIn("ladybird_provenance.py", corps, f"{src} ne decide pas par la provenance")
                    suite = "\n".join(str(x.get("run") or "") for x in etapes[i + 1:])
                    self.assertIn("verifie_artefact_ladybird.sh entrees", suite,
                                  "artefact d'un autre run joue sans verification des entrees")
        self.assertGreaterEqual(vus, 2, "ladybird-native-browser (reutilise) et gpu-composition attendus")


if __name__ == "__main__":
    unittest.main()
