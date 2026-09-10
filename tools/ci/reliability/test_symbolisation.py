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
