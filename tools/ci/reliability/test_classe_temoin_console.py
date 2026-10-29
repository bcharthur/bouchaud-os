"""P13_TEMOIN_CONSOLE_FIL_SEUL_V1 : classement du bras temoin console (cible
fil, sans secours), par la fonction meme que le banc source.

Temoins : les deux bilans du meme run 40802102934 (2a151fef, campagne 3/3).
TCG, rouge : pas de repli, les ecrivains ont differe puis vide eux-memes
(`verrou_abandons=1271`, `differees_place_faite=1066`) -- la precondition
« le fil seul vide » n'a pas tenu : NON_QUALIFIANT, jamais vert. KVM, vert :
repli revenu : OK. Sans repli et sans aide : ECHEC (le vrai rouge du temoin).
"""
import subprocess
import unittest
from pathlib import Path

RACINE = Path(__file__).resolve().parents[3]
CLASSE = RACINE / "tools/ci/classe_temoin_console.sh"

# Debuts des bilans (le banc ne lit que `active=` et `repli_securite=`).
ROUGE_TCG = ("BOUCHAUD_CONSOLE_BILAN active=1 repli_securite=0 lignes=1998 emises=1997 pleines=623 "
             "replis_differes=1271 vidanges_aidees=3863 reentrees=0")
VERT_KVM = ("BOUCHAUD_CONSOLE_BILAN active=0 repli_securite=1 lignes=648 emises=648 pleines=334 "
            "replis_differes=0 vidanges_aidees=6239 reentrees=0")


def classe(bilan: str, aides: str) -> str:
    r = subprocess.run(
        ["bash", "-c", '. "$1"; classe_temoin_console "$2" "$3"', "_", str(CLASSE), bilan, aides],
        capture_output=True, text=True, check=True,
    )
    return r.stdout.strip()


class ClasseTemoinConsole(unittest.TestCase):
    def test_repli_revenu_ok(self):
        self.assertEqual(classe(VERT_KVM, "0"), "OK")

    def test_repli_revenu_ok_meme_avec_une_aide(self):
        # Le defaut est apparu : la precondition n'a pas manque.
        self.assertEqual(classe(VERT_KVM, "1"), "OK")

    def test_sans_repli_avec_aides_non_qualifiant(self):
        # Le rouge de 40802102934 si le compte avait existe : jamais vert.
        self.assertEqual(classe(ROUGE_TCG, "1066"), "NON_QUALIFIANT")

    def test_sans_repli_sans_aide_echec(self):
        # Le fil etait seul, et le repli n'est pas venu : le temoin echoue.
        self.assertEqual(classe(ROUGE_TCG, "0"), "ECHEC")

    def test_sans_mesure_echec(self):
        for aides in ("", "x", "-1"):
            self.assertEqual(classe(ROUGE_TCG, aides), "ECHEC", aides)
            self.assertEqual(classe(VERT_KVM, aides), "ECHEC", aides)

    def test_le_banc_source_la_fonction(self):
        banc = (RACINE / "tools/ci/run_serie_entrelacement.sh").read_text(encoding="utf-8")
        self.assertIn(". tools/ci/classe_temoin_console.sh", banc)
        self.assertIn('CLASSE=$(classe_temoin_console "$BILAN" "$AIDES")', banc)


if __name__ == "__main__":
    unittest.main()
