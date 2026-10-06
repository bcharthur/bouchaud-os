#!/usr/bin/env python3
"""Rapport WPT Ladybird-sur-Bouchaud, tire du journal serie (P11).

BOUCHAUD_WPT_V1

    rapport.py serie-wpt.log manifeste.json > rapport.md

Lit les lignes `HOST_WPT`, `HOST_WPT_ECART`, `HOST_WPT_FIN` emises par
`wpt/runner.html` (via la console JS, recopiee sur la serie), et rend un
tableau : sur Bouchaud contre attendu sous Linux (meme commit). Un fichier du
manifeste qui n'a pas de ligne est dit MANQUANT -- jamais compte comme passe.
"""
import json
import re
import sys

LIGNE = re.compile(r'"(HOST_WPT(?:_ECART|_FIN|_DEBUT)?) ([^"]*)"')


def champs(texte):
    return dict(m.groups() for m in re.finditer(r"(\w+)=(\S+)", texte))


def main() -> int:
    journal = open(sys.argv[1], encoding="utf-8", errors="replace").read()
    journal = re.sub(r"\x1b\[[0-9;]*m", "", journal)
    manifeste = json.load(open(sys.argv[2], encoding="utf-8"))
    fichiers, ecarts, fin = {}, [], None
    for genre, texte in LIGNE.findall(journal):
        if genre == "HOST_WPT":
            c = champs(texte)
            fichiers.setdefault(c.get("fichier"), c)
        elif genre == "HOST_WPT_ECART":
            ecarts.append(texte)
        elif genre == "HOST_WPT_FIN":
            fin = champs(texte)

    print("# WPT — Ladybird sur Bouchaud OS contre Ladybird sous Linux\n")
    print("Corpus : tests WPT vendorisés par Ladybird (commit épinglé), joués dans le navigateur "
          "de l'invite QEMU par `tools/ladybird/wpt/runner.html` ; référence : "
          "`Tests/LibWeb/Text/expected/wpt-import` (Ladybird headless, Linux, même commit).\n")
    if fin:
        print(f"**Bilan** : {fin.get('fichiers')} fichiers, {fin.get('pass')} sous-tests réussis "
              f"(attendu {fin.get('attendu')}) — égaux {fin.get('egaux')}, mieux {fin.get('mieux')}, "
              f"moins {fin.get('moins')}, échéances {fin.get('echeances')}.\n")
    else:
        print("**Bilan** : le runner n'a PAS conclu (aucune ligne HOST_WPT_FIN).\n")
    print("| Fichier | Statut | Bouchaud | Attendu (Linux) | ms |")
    print("|---|---|---|---|---|")
    manquants = 0
    for m in manifeste:
        c = fichiers.get(m["chemin"])
        if c is None:
            manquants += 1
            print(f"| `{m['chemin']}` | MANQUANT | — | {m['attendu_pass']}/{m['attendu_total']} | — |")
            continue
        print(f"| `{m['chemin']}` | {c.get('statut')} | {c.get('pass')}/{c.get('total')} | "
              f"{c.get('attendu')} | {c.get('ms')} |")
    if manquants:
        print(f"\n{manquants} fichier(s) sans résultat.")
    if ecarts:
        print("\n## Sous-tests qui passent sous Linux et pas ici (trois au plus par fichier)\n")
        for e in ecarts:
            print(f"- {e}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
