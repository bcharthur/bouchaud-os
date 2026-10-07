#!/usr/bin/env python3
"""Profil statistique des processus du navigateur, symbolise.

BOUCHAUD_PROFIL_RIP_V1

    tools/ci/profil_rip.py JOURNAL_SERIE DOSSIER_BINAIRES [--images A,B,...] [--noyau ELF]

Le noyau echantillonne, a chaque tic de quantum, le RIP interrompu du fil
courant, et publie toutes les 5 s, par fil :

    [PERF-RIP] t=... pid=19 tid=23 image=WebContent user=41 noyau=9 base=0x400000400000 top=0x4000012345:7,... topn=0xffff...:3,...

`user`/`noyau` : echantillons pris en espace utilisateur / dans le noyau
(appel systeme, faute). `top` : les RIP utilisateur les plus frequents (un
RIP exact par seau de 256 octets, avec son compte) ; `topn` : de meme pour
les echantillons pris dans le noyau, resolus contre l'ELF du noyau (--noyau).
Ce script additionne tout
le journal, rapporte chaque RIP a la base et le resout par addr2line contre
le binaire EXACT du run, puis imprime par image et par role (fil principal
= tid == pid, autres fils) les fonctions qui portent le plus d'echantillons.

Ne juge rien : imprime et rend 0.
"""
import re
import shutil
import subprocess
import sys
from collections import Counter, defaultdict
from pathlib import Path

ANSI = re.compile(r"\x1b\[[0-9;]*m")
LIGNE = re.compile(
    r"\[PERF-RIP\] t=\d+ pid=(\d+) tid=(\d+) image=(\S*) user=(\d+) noyau=(\d+) base=(0x[0-9a-f]+) top=(\S+)(?: topn=(\S+))?"
)
# Ligne « fichier:ligne » d'addr2line (une fonction C++ ne finit jamais par
# « :chiffres »).
FICHIER_LIGNE = re.compile(r":(\d+|\?)( \(discriminator \d+\))?$")
# /bo-navigateur est une copie de BouchaudBrowserHost.
ALIAS = {"bo-navigateur": "BouchaudBrowserHost"}
IMAGES_DEFAUT = "WebContent,RequestServer,Compositor,ImageDecoder,BouchaudBrowserHost"


def outil():
    for nom in ("llvm-addr2line", "addr2line", "eu-addr2line"):
        chemin = shutil.which(nom)
        if chemin:
            return chemin
    return None


def symbolise(a2l, binaire, decalages):
    """{decalage: (fonction la plus interne, fonction englobante)}."""
    noms = {}
    if a2l is None or not binaire.is_file() or not decalages:
        return noms
    liste = sorted(decalages)
    try:
        sortie = subprocess.run(
            [a2l, "-f", "-C", "-i", "-a", "-e", str(binaire)] + [hex(d) for d in liste],
            capture_output=True, text=True, timeout=600,
        ).stdout.splitlines()
    except (OSError, subprocess.TimeoutExpired):
        return noms
    # Avec -a, chaque adresse ouvre un bloc : 0x..., puis paires fonction /
    # fichier:ligne (la plus interne d'abord, -i).
    courant, fonctions = None, []
    for ligne in sortie + ["0x"]:
        if ligne.startswith("0x"):
            if courant is not None:
                noms[courant] = (fonctions[0] if fonctions else "??", fonctions[-1] if fonctions else "??")
            courant = int(ligne, 16) if len(ligne) > 2 else None
            fonctions = []
        elif not FICHIER_LIGNE.search(ligne):
            fonctions.append(ligne.strip())
    return noms


def main(argv):
    if len(argv) < 3:
        print(__doc__.strip().splitlines()[4], file=sys.stderr)
        return 0
    journal, dossier = Path(argv[1]), Path(argv[2])
    images = set(IMAGES_DEFAUT.split(","))
    noyau_elf = None
    reste = argv[3:]
    while len(reste) >= 2:
        if reste[0] == "--images":
            images = set(reste[1].split(","))
        elif reste[0] == "--noyau":
            noyau_elf = Path(reste[1])
        reste = reste[2:]
    if not journal.is_file():
        return 0
    # (image, role) -> compteurs
    user = Counter()
    noyau = Counter()
    rips = defaultdict(Counter)
    rips_noyau = defaultdict(Counter)
    base_vue = {}
    for brut in journal.read_text(errors="replace").splitlines():
        m = LIGNE.search(ANSI.sub("", brut))
        if not m:
            continue
        pid, tid, image = int(m[1]), int(m[2]), Path(m[3]).name
        image = ALIAS.get(image, image)
        if image not in images:
            continue
        cle = (image, "principal" if pid == tid else "autres fils")
        user[cle] += int(m[4])
        noyau[cle] += int(m[5])
        base_vue[image] = int(m[6], 16)
        if m[7] != "-":
            for paire in m[7].split(","):
                rip, n = paire.split(":")
                rips[cle][int(rip, 16)] += int(n)
        if m[8] and m[8] != "-":
            for paire in m[8].split(","):
                rip, n = paire.split(":")
                rips_noyau[cle][int(rip, 16)] += int(n)
    if not user and not noyau:
        print("PROFIL_RIP absent (aucune ligne [PERF-RIP])")
        return 0
    a2l = outil()
    print()
    print("== profil des processus du navigateur (echantillons au quantum, symbolises) ==")
    # Les 8 couples (image, role) les plus echantillonnes : le banc tourne
    # sous `pipefail`, la borne est ici et pas dans un `| head`.
    for cle in sorted(set(user) | set(noyau), key=lambda c: -(user[c] + noyau[c]))[:8]:
        image, role = cle
        total = user[cle] + noyau[cle]
        vus = sum(rips[cle].values())
        print(f"PROFIL_RIP image={image} role={role} echantillons={total} user={user[cle]} noyau={noyau[cle]}"
              f" user_pct={100 * user[cle] // max(1, total)} user_nommes={vus}")
        base = base_vue.get(image, 0)
        noms = symbolise(a2l, dossier / image, {r - base for r in rips[cle] if r >= base})
        par_fonction = Counter()
        for rip, n in rips[cle].items():
            interne, englobante = noms.get(rip - base, ("?", "?"))
            nom = interne if interne == englobante else f"{interne} <- {englobante}"
            par_fonction[nom[:160]] += n
        for nom, n in par_fonction.most_common(12):
            print(f"    {n:5d} {100 * n // max(1, vus):3d}%  {nom}")
        if noyau_elf is not None and rips_noyau[cle]:
            noms_n = symbolise(a2l, noyau_elf, set(rips_noyau[cle]))
            par_fn = Counter()
            for rip, n in rips_noyau[cle].items():
                interne, englobante = noms_n.get(rip, ("?", "?"))
                par_fn[(interne if interne == englobante else f"{interne} <- {englobante}")[:160]] += n
            vus_n = sum(rips_noyau[cle].values())
            print(f"    -- noyau ({vus_n} nommes)")
            for nom, n in par_fn.most_common(6):
                print(f"    {n:5d} {100 * n // max(1, vus_n):3d}%  {nom}")
        if a2l is None:
            print("    (addr2line absent : adresses non symbolisees)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
