#!/usr/bin/env python3
"""Extrait les preuves chronologiques sans mélange des horloges ni URL privées."""
import argparse
import json
import re
from pathlib import Path
from urllib.parse import urlsplit

from analyse_session import R_PERF_PROC, lit

MOTIFS = (
    ("trame", "[LB:FRAME]"),
    ("navigation", "[LB:NAV]"),
    ("ordonnanceur", "[SCHED-NG-ATTENTE-VIVE]"),
    ("ordonnanceur_centiles", "[SCHED-NG-CENTILES]"),
    ("fautes", "[PERF-PROC]"),
    ("fautes_fichier", "FAULT_FILE_BREAKDOWN "),
    ("disque", "BACKING_DISK_DECOMP "),
    ("disque_pid", "BACKING_DISK_ATTRIB "),
    ("persistance", "[PERSIST-SLOW]"),
    ("persistance_totaux", "[PERSIST-TXN]"),
    ("fork", "CHILD_AFTER_FORK "),
    ("exec", "EXECVE_SYSCALL_ENTER "),
    ("exec_erreur", "EXECVE_SYSCALL_ERR "),
    ("sortie", "PROCESS_EXIT "),
    ("exception", "PROCESS_FAULT "),
    ("instruction", "PROCESS_FAULT_DETAIL "),
    ("parseur", "HTMLParserEndState: timed out in phase "),
)
HORLOGE = re.compile(r"^\[(\d\d):(\d\d):(\d\d)\]")
CHAMPS = re.compile(r"\b(\w+)=(-?(?:0x[\da-f]+|\d+))(?=\s|$)")
PARSEUR = re.compile(r"\b(\d+)\.(\d{3}) WebContent\((\d+)\):.*phase (\d+)")
NAVIGATION = re.compile(r"onglet=(\d+) (\w+) url=(\S+)")


def chronologie(texte):
    debut = dernier = None
    jour = 0
    for numero, ligne in enumerate(texte.splitlines(), 1):
        mur = HORLOGE.match(ligne)
        seconde = None
        if mur:
            heure, minute, sec = map(int, mur.groups())
            seconde = heure * 3600 + minute * 60 + sec
            if dernier is not None and dernier - seconde > 43200:
                jour += 86400
            dernier = seconde
            if debut is None:
                debut = seconde
        for categorie, marqueur in MOTIFS:
            prefixe, trouve, suite = ligne.partition(marqueur)
            if not trouve:
                continue
            champs = {k: int(v, 16 if v.startswith("0x") else 10)
                      for k, v in CHAMPS.findall(suite.partition(" url=")[0])}
            if categorie in ("trame", "fautes", "fautes_fichier", "disque", "fork", "exec", "exec_erreur", "sortie"):
                if "t" in champs:
                    champs["invite_ms"] = champs.pop("t")
            if categorie == "persistance" and "t_ms" in champs:
                champs["invite_ms"] = champs.pop("t_ms")
            if categorie.startswith("ordonnanceur") and (m := re.search(r"\bclasse=(\w+)", suite)):
                champs["classe"] = m[1]
            if categorie == "fautes" and (m := R_PERF_PROC.search(ligne)):
                champs.update(image=m[3], zero=int(m[9]), zero_us=int(m[10]),
                              fichier=int(m[11]), fichier_us=int(m[12]))
            if categorie == "parseur" and (m := PARSEUR.search(prefixe + trouve + suite)):
                sec, ms, pid, phase = map(int, m.groups())
                champs.update(invite_ms=1000 * sec + ms, pid=pid, phase=phase)
            if categorie == "navigation" and (m := NAVIGATION.search(suite)):
                onglet, etat, url = m.groups()
                try:
                    domaine = urlsplit(url).hostname
                except ValueError:
                    domaine = None
                champs.update(onglet=int(onglet), etat=etat, domaine=domaine)
            if categorie == "instruction" and (m := re.search(r"\binsn=([\da-f]+)", suite)):
                champs["insn"] = m[1]
            yield {"ligne": numero, "serie_s": seconde + jour - debut if mur else None,
                   "categorie": categorie, **champs}
            break


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("journal", type=Path)
    args = parser.parse_args()
    for evenement in chronologie(lit(args.journal)):
        print(json.dumps(evenement, ensure_ascii=True))


if __name__ == "__main__":
    main()
