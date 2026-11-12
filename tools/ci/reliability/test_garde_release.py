"""BOUCHAUD_RELEASE_V1 : la garde de publication refuse tout ce qu'elle doit refuser."""
import importlib.util
import sys
import unittest
from pathlib import Path

RACINE = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("gr", RACINE / "tools/release/garde_release.py")
gr = importlib.util.module_from_spec(spec)
sys.modules["gr"] = gr
spec.loader.exec_module(gr)

SHA = "b" * 40
CLE = '    ("release-test", "' + "c" * 64 + '"),\n'


def verts(sha=SHA):
    return [{"name": n, "head_sha": sha, "status": "completed", "conclusion": "success",
             "run_number": 10, "run_attempt": 1} for n in gr.QUALIFICATION_REQUISE]


class GardeTests(unittest.TestCase):
    def pub(self, **kw):
        a = dict(sha=SHA, ref="refs/heads/main", version="2026.11.12-1", sequence=5,
                 runs=verts(), tags=[], cles_texte=CLE)
        a.update(kw)
        return gr.publier(**a)

    def refuse(self, motif, **kw):
        with self.assertRaises(gr.Refus) as c:
            self.pub(**kw)
        self.assertIn(motif, str(c.exception))

    def test_cas_nominal(self):
        self.assertEqual(self.pub(), ("trigkey-s5-2026.11.12-1", None))
        self.assertEqual(self.pub(tags=["trigkey-s4-x", "v1", "userland-abc"]), ("trigkey-s5-2026.11.12-1", 4))

    def test_hors_main_refuse(self):
        self.refuse("ref_non_main", ref="refs/heads/claude/x")
        self.refuse("ref_non_main", ref="refs/pull/3/merge")
        self.refuse("ref_non_main", ref="refs/tags/v1")

    def test_depot_sans_cle_de_production_refuse(self):
        self.refuse("aucune_cle_de_production_engagee", cles_texte="")
        self.refuse("aucune_cle_de_production_engagee", cles_texte="//" + CLE)
        self.refuse("aucune_cle_de_production_engagee",
                    cles_texte='pub const CLES_PRODUCTION: &[(&str, &str)] = &[\n    // ("x", "y"),\n];\n')

    def test_qualification_exigee_sur_le_sha_exact(self):
        self.refuse("qualification_absente", runs=verts("d" * 40))
        self.refuse("qualification_absente", runs=verts()[1:])
        r = verts(); r[0]["conclusion"] = "failure"
        self.refuse("qualification_non_verte", runs=r)
        r = verts(); r[0]["conclusion"] = "cancelled"
        self.refuse("qualification_non_verte", runs=r)
        r = verts(); r[0]["conclusion"] = "skipped"
        self.refuse("qualification_non_verte", runs=r)
        r = verts(); r[0]["status"] = "in_progress"; r[0]["conclusion"] = None
        self.refuse("qualification_en_cours", runs=r)

    def test_la_derniere_execution_fait_foi(self):
        r = verts() + [dict(verts()[0], run_number=11, conclusion="failure")]
        self.refuse("qualification_non_verte", runs=r)
        r = [dict(verts()[0], run_number=9, conclusion="failure")] + verts()
        self.assertEqual(self.pub(runs=r)[0], "trigkey-s5-2026.11.12-1")
        r = verts() + [dict(verts()[0], run_attempt=2, conclusion="failure")]
        self.refuse("qualification_non_verte", runs=r)

    def test_versions_immuables_et_sequence_monotone(self):
        self.refuse("etiquette_existante", tags=["trigkey-s5-2026.11.12-1"])
        self.refuse("version_deja_publiee", tags=["trigkey-s3-2026.11.12-1"])
        self.refuse("sequence_non_croissante", tags=["trigkey-s5-autre"])
        self.refuse("sequence_non_croissante", tags=["trigkey-s9-autre"])

    def test_entrees_invalides(self):
        self.refuse("sha_invalide", sha="B" * 40)
        self.refuse("version_invalide", version="a b")
        self.refuse("sequence_invalide", sequence=0)

    def test_promotion(self):
        gr.promouvoir("trigkey-s5-v", ["trigkey-s5-v"], "true")
        for args in [("trigkey-s5-v", [], "true"), ("trigkey-s5-v", ["trigkey-s5-v"], "false"),
                     ("v1", ["v1"], "true")]:
            with self.assertRaises(gr.Refus):
                gr.promouvoir(*args)


if __name__ == "__main__":
    unittest.main()
