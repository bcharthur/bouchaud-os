#!/usr/bin/env python3
"""Symbolise les fautes des processus Ladybird d'un journal serie.

BOUCHAUD_SYMBOLISE_FAUTES_V1

    tools/ci/symbolise_fautes.py JOURNAL_SERIE DOSSIER_BINAIRES

Le noyau imprime, pour chaque faute utilisateur fatale :

    PROCESS_FAULT pid=19 reason=faute de page rip=0x400009209064 ... cr2=...

et, pour chaque exec, la base de chargement de l'image PIE :

    PERF_EXEC_PRET image=/usr/libexec/ladybird/WebContent pid=19 ... base=0x400000000000

L'adresse seule ne dit rien. Rapportee a la base et resolue par addr2line
contre le binaire EXACT qui a tourne (l'artefact du meme run, verifie par son
manifeste), elle nomme la fonction. Run 37585729384, WPT : WebContent faute
sur cr2=0xffff413f021ee480 -- la MEME adresse non canonique qu'aux runs
37518121906 et 405 : une valeur deterministe, qu'il faut localiser.

Ne juge rien : imprime une ligne `FAUTE_SYMBOLE` par faute et rend 0.
"""
import re
import shutil
import subprocess
import sys
from pathlib import Path

ANSI = re.compile(r"\x1b\[[0-9;]*m")
EXEC = re.compile(r"PERF_EXEC_PRET image=(\S+) pid=(\d+) .*?base=(0x[0-9a-f]+)")
EXECVE = re.compile(r"PERF_EXECVE .*?image=(\S+) pid=(\d+)")
FAUTE = re.compile(r"PROCESS_FAULT pid=(\d+) reason=(.*?) rip=(0x[0-9a-f]+) rsp=(0x[0-9a-f]+) cr2=(0x[0-9a-f]+)")
# /bo-navigateur est une copie de BouchaudBrowserHost.
ALIAS = {"bo-navigateur": "BouchaudBrowserHost"}


def outil():
    for nom in ("llvm-addr2line", "addr2line", "eu-addr2line"):
        chemin = shutil.which(nom)
        if chemin:
            return chemin
    return None


def main(argv):
    if len(argv) != 3:
        print(__doc__.strip().splitlines()[4], file=sys.stderr)
        return 0
    journal, dossier = Path(argv[1]), Path(argv[2])
    if not journal.is_file():
        return 0
    images, bases, fautes = {}, {}, []
    for ligne in journal.read_text(errors="replace").splitlines():
        ligne = ANSI.sub("", ligne)
        if m := EXEC.search(ligne):
            images[m[2]] = m[1]
            bases[m[2]] = int(m[3], 16)
        elif m := EXECVE.search(ligne):
            images.setdefault(m[2], m[1])
        if m := FAUTE.search(ligne):
            fautes.append(m)
    if not fautes:
        return 0
    a2l = outil()
    print()
    print(f"== fautes des processus, symbolisees ({len(fautes)}) ==")
    for f in fautes[:12]:
        pid, raison, rip, cr2 = f[1], f[2], int(f[3], 16), f[5]
        image = images.get(pid, "?")
        nom = ALIAS.get(Path(image).name, Path(image).name)
        binaire = dossier / nom
        base = bases.get(pid)
        entete = f"FAUTE_SYMBOLE pid={pid} image={image} rip={rip:#x} cr2={cr2} raison={raison.strip()}"
        if base is None or not binaire.is_file():
            print(f"{entete} -> non symbolisable (base={'?' if base is None else hex(base)} binaire={'present' if binaire.is_file() else 'absent'})")
            continue
        decalage = rip - base
        if a2l is None:
            print(f"{entete} decalage={decalage:#x} -> addr2line absent")
            continue
        try:
            sortie = subprocess.run([a2l, "-f", "-C", "-i", "-e", str(binaire), hex(decalage)],
                                    capture_output=True, text=True, timeout=120).stdout.strip().splitlines()
        except (OSError, subprocess.TimeoutExpired) as erreur:
            print(f"{entete} decalage={decalage:#x} -> addr2line : {erreur}")
            continue
        # Paires (fonction, fichier:ligne), la plus interne d'abord (-i : inlines).
        paires = [f"{sortie[i]} ({sortie[i + 1] if i + 1 < len(sortie) else '?'})" for i in range(0, len(sortie), 2)]
        print(f"{entete} decalage={decalage:#x}")
        for p in paires[:6]:
            print(f"    {p}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
