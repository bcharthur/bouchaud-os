#!/usr/bin/env python3
"""Preuve FONCTIONNELLE de la symbolisation BLACKBOX.

Ce n'est pas une recherche de chaine dans un source : l'outil est LANCE, sur un
vrai ELF, avec un vrai manifeste, et ce qui est verifie est sa REPONSE.

# Les trois proprietes qui comptent

  1. il symbolise une adresse en fonction, contre le noyau designe ;
  2. il REFUSE de repondre quand le noyau ne correspond pas au manifeste --
     c'est la propriete centrale : entre deux constructions l'editeur de liens
     deplace tout, et une reponse rendue sur le mauvais binaire ressemble
     exactement a une reponse juste ;
  3. il refuse aussi quand on ne lui donne aucun noyau.
"""

import hashlib
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

RACINE = Path(__file__).resolve().parents[3]
OUTIL = RACINE / "tools/reference/symbolise-blackbox.py"
NOYAUX = [
    RACINE / "target/x86_64-bouchaud_os/debug/bouchaud-os",
    RACINE / "target/x86_64-bouchaud_os_uefi/debug/bouchaud-os",
]
BASE = 0x8000000000


def noyau_disponible():
    for chemin in NOYAUX:
        if chemin.is_file():
            return chemin
    return None


def sha256(chemin):
    h = hashlib.sha256()
    with open(chemin, "rb") as f:
        for bloc in iter(lambda: f.read(1 << 20), b""):
            h.update(bloc)
    return h.hexdigest()


def lance(*args):
    return subprocess.run(
        [sys.executable, str(OUTIL)] + list(args),
        capture_output=True, text=True, cwd=RACINE,
    )


def premiere_fonction(noyau):
    """Une adresse dont on SAIT qu'elle appartient a une fonction nommee."""
    sortie = subprocess.run(
        ["nm", "--defined-only", "-n", str(noyau)],
        capture_output=True, text=True,
    )
    for ligne in sortie.stdout.splitlines():
        morceaux = ligne.split(None, 2)
        if len(morceaux) == 3 and morceaux[1] in ("t", "T"):
            try:
                return int(morceaux[0], 16), morceaux[2]
            except ValueError:
                continue
    return None, None


class Symbolisation(unittest.TestCase):
    def setUp(self):
        self.noyau = noyau_disponible()
        if self.noyau is None:
            self.skipTest(
                "aucun noyau construit ; lancer `cargo build` d'abord. "
                "Ce cas ne peut pas s'inventer un ELF."
            )

    def test_une_adresse_connue_est_symbolisee(self):
        adresse, nom = premiere_fonction(self.noyau)
        if adresse is None:
            self.skipTest("nm n'a rendu aucun symbole de texte")
        r = lance(hex(BASE + adresse), "--noyau", str(self.noyau))
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn(hex(BASE + adresse), r.stdout)
        # Le nom mangle ou demangle : les deux sont acceptables, l'absence de
        # reponse ne l'est pas.
        self.assertNotIn("<inconnu>", r.stdout, r.stdout)

    def test_un_noyau_qui_ne_correspond_pas_est_refuse(self):
        with tempfile.TemporaryDirectory() as tmp:
            manifeste = Path(tmp) / "m.json"
            manifeste.write_text(json.dumps({
                "commit": "0000000000000000000000000000000000000000",
                "noyau": {
                    "chemin": str(self.noyau),
                    # Une somme volontairement fausse.
                    "sha256": "0" * 64,
                    "base": hex(BASE),
                },
            }), encoding="utf-8")
            r = lance("0x8000001000", "--manifeste", str(manifeste))
            self.assertNotEqual(r.returncode, 0, "un noyau errone doit etre REFUSE")
            self.assertIn("NE CORRESPOND PAS", r.stdout)

    def test_un_noyau_qui_correspond_est_accepte(self):
        with tempfile.TemporaryDirectory() as tmp:
            manifeste = Path(tmp) / "m.json"
            manifeste.write_text(json.dumps({
                "commit": "abc",
                "noyau": {
                    "chemin": str(self.noyau),
                    "sha256": sha256(self.noyau),
                    "base": hex(BASE),
                },
            }), encoding="utf-8")
            adresse, _ = premiere_fonction(self.noyau)
            if adresse is None:
                self.skipTest("nm n'a rendu aucun symbole de texte")
            r = lance(hex(BASE + adresse), "--manifeste", str(manifeste))
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
            self.assertIn("commit  : abc", r.stdout)

    def test_sans_noyau_l_outil_refuse_de_deviner(self):
        r = lance("0x8000001000")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("il faut --manifeste ou --noyau", r.stdout)


if __name__ == "__main__":
    unittest.main()


class LecteurElfAutonome(unittest.TestCase):
    """Le repli qui repond quand la machine n'a ni addr2line ni nm.

    Le 16 septembre, l'outil a rendu ceci sur le poste de l'utilisateur :

        0x8001392340  ->  +0x1392340
            <inconnu>
            ??:0

    Il n'avait pas echoue a trouver le symbole : il n'avait AUCUN outil pour
    chercher. `addr2line` et `nm` viennent des binutils ou de LLVM, absents
    d'un poste Windows ordinaire. L'outil degradait en silence vers
    « inconnu », ce qui ressemble a « cette adresse n'a pas de symbole » --
    une reponse, et fausse. Une adresse non resolue bloquait une enquete
    entiere.
    """

    def setUp(self):
        self.noyau = noyau_disponible()
        if self.noyau is None:
            self.skipTest("aucun noyau construit")
        import importlib.util
        spec = importlib.util.spec_from_file_location("sym", OUTIL)
        self.m = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.m)

    def test_la_table_des_symboles_se_lit_sans_outil_externe(self):
        symboles = self.m.symboles_elf(self.noyau)
        self.assertGreater(len(symboles), 100,
                           "un noyau porte des milliers de fonctions")
        adresses = [s[0] for s in symboles]
        self.assertEqual(adresses, sorted(adresses), "la table doit etre triee")

    def test_une_adresse_dans_une_fonction_la_nomme(self):
        symboles = self.m.symboles_elf(self.noyau)
        adresse, taille, _ = next(s for s in symboles if s[1] > 8)
        trouve = self.m.cherche_symbole(symboles, adresse + 4)
        self.assertIsNotNone(trouve)
        self.assertIn("+0x4", trouve[0])

    def test_une_adresse_hors_de_toute_fonction_ne_ment_pas(self):
        """Rendre « le dernier symbole avant » designerait n'importe quoi.

        La taille du symbole est dans l'ELF ; s'en servir evite d'attribuer
        du bourrage inter-sections a la fonction qui le precede.
        """
        symboles = self.m.symboles_elf(self.noyau)
        derniere, taille, _ = symboles[-1]
        self.assertIsNone(
            self.m.cherche_symbole(symboles, derniere + taille + 0x100_000))

    def test_le_nom_rendu_est_lisible(self):
        """Un nom mangle est juste et inutilisable dans une enquete."""
        symboles = self.m.symboles_elf(self.noyau)
        mangles = [s for s in symboles if s[2].startswith("_R") and s[1] > 8]
        if not mangles:
            self.skipTest("aucun symbole mangle v0 dans ce noyau")
        adresse, _, _ = mangles[0]
        nom = self.m.cherche_symbole(symboles, adresse)[0]
        self.assertFalse(nom.startswith("_R"), "le nom doit etre demangle")
        self.assertIn("::", nom, "un chemin Rust porte des separateurs")

    def test_le_prefixe_de_caisse_ne_fuit_pas_dans_le_nom(self):
        """`Cs<empreinte>_` contient des chiffres.

        Les lire comme une longueur de composant decoupait n'importe ou et
        produisait des prefixes parasites du genre `ObZ1v::_11bou::`.
        """
        self.assertEqual(
            self.m.demangle("_RNvNtNtCsjLR5ObZ1vZ6_11bouchaud_os2fs11persistance5monte"),
            "bouchaud_os::fs::persistance::monte")

    def test_un_nom_non_mangle_passe_tel_quel(self):
        self.assertEqual(self.m.demangle("memcpy"), "memcpy")
