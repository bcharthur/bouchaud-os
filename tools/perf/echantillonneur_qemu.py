#!/usr/bin/env python3
"""BOUCHAUD_ECHANTILLONNEUR_HOTE_V1 -- profil SANS BIAIS d'un invite QEMU.

Le profil `[PERF-RIP]` du noyau echantillonne a l'IRQ0 : il ne voit jamais le
code qui tourne interruptions masquees (IF=0), et lui impute son temps au
premier `sti` qui suit. Ce script regarde la machine DE L'EXTERIEUR : il
interroge un moniteur QEMU (`info registers -a`) a frequence fixe et releve,
pour chaque vCPU, RIP, CPL, IF, HLT et CR3.

  echantillonneur_qemu.py collecte SOCKET SORTIE.json [--hz 200] [--duree 120]
  echantillonneur_qemu.py rapport SORTIE.json NOYAU.elf [--top 40] [--depuis S] [--jusqua S]

Le moniteur doit etre un second moniteur HMP dedie, p. ex. :
  BO_SMOKE_QEMU_ARGS="-monitor unix:/tmp/ech.sock,server,nowait"

Sous TCG, l'etat lu est celui du dernier bloc traduit : la precision est celle
d'un echantillonneur, pas d'un traceur. Un echantillon est une observation,
pas une duree ; seules les PROPORTIONS ont un sens.
"""
import argparse
import collections
import json
import os
import re
import socket
import subprocess
import sys
import time

RE_CPU = re.compile(r"^CPU#(\d+)", re.M)
RE_RIP = re.compile(r"RIP=([0-9a-f]+) RFL=([0-9a-f]+) \[[^\]]*\] CPL=(\d) .*?HLT=(\d)")
RE_CR3 = re.compile(r"CR3=([0-9a-f]+)")


def connecte(chemin, delai_s=600):
    fin = time.time() + delai_s
    while True:
        try:
            s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            s.connect(chemin)
            return s
        except OSError:
            if time.time() > fin:
                raise
            time.sleep(0.2)


def lit_invite(s, tampon=b""):
    while b"(qemu) " not in tampon:
        morceau = s.recv(65536)
        if not morceau:
            raise EOFError
        tampon += morceau
    avant, _, reste = tampon.partition(b"(qemu) ")
    return avant.decode(errors="replace"), reste


def collecte(args):
    s = connecte(args.socket)
    _, reste = lit_invite(s)
    periode = 1.0 / args.hz
    debut = time.time()
    echantillons = []
    prochain = debut
    while time.time() - debut < args.duree:
        try:
            s.sendall(b"info registers -a\n")
            texte, reste = lit_invite(s, reste)
        except (EOFError, OSError):
            break
        t = round(time.time() - debut, 4)
        blocs = RE_CPU.split(texte)
        # blocs = [prefixe, cpu0, texte0, cpu1, texte1, ...]
        for i in range(1, len(blocs) - 1, 2):
            cpu = int(blocs[i])
            corps = blocs[i + 1]
            m = RE_RIP.search(corps)
            c = RE_CR3.search(corps)
            if not m:
                continue
            rfl = int(m.group(2), 16)
            echantillons.append([t, cpu, m.group(1), int(m.group(3)), int(bool(rfl & 0x200)),
                                 int(m.group(4)), c.group(1) if c else "0"])
        prochain += periode
        attente = prochain - time.time()
        if attente > 0:
            time.sleep(attente)
        else:
            prochain = time.time()
    with open(args.sortie, "w") as f:
        json.dump({"version": 1, "hz": args.hz, "champs": ["t", "cpu", "rip", "cpl", "if", "hlt", "cr3"],
                   "echantillons": echantillons}, f)
    print(f"ECHANTILLONNEUR_HOTE echantillons={len(echantillons)} duree_s={time.time() - debut:.1f}")


def symbolise(elf, adresses):
    """adresse -> pile d'inlining (externe en dernier), via addr2line -i."""
    resultat = {}
    adresses = sorted(set(adresses))
    for lot in range(0, len(adresses), 500):
        sous = adresses[lot:lot + 500]
        sortie = subprocess.run(["addr2line", "-i", "-f", "-C", "-a", "-e", elf] + sous,
                                capture_output=True, text=True).stdout.splitlines()
        courant = None
        pile = []
        i = 0
        while i < len(sortie):
            ligne = sortie[i]
            if ligne.startswith("0x") and re.fullmatch(r"0x[0-9a-f]+", ligne):
                if courant is not None:
                    resultat[courant] = pile
                courant = hex(int(ligne, 16))
                pile = []
                i += 1
                continue
            fonction = ligne
            fichier = sortie[i + 1] if i + 1 < len(sortie) else "?"
            pile.append((fonction, fichier))
            i += 2
        if courant is not None:
            resultat[courant] = pile
    return resultat


def nom_court(fonction):
    fonction = re.sub(r"<[^<>]*>", "", fonction)
    fonction = re.sub(r"<[^<>]*>", "", fonction)
    return fonction.replace("bouchaud_os::", "")


def rapport(args):
    d = json.load(open(args.json))
    ech = [e for e in d["echantillons"]
           if (args.depuis is None or e[0] >= args.depuis) and (args.jusqua is None or e[0] <= args.jusqua)]
    total = len(ech)
    if not total:
        print("aucun echantillon")
        return
    hlt = sum(1 for e in ech if e[5])
    user = sum(1 for e in ech if not e[5] and e[3] == 3)
    noyau = [e for e in ech if not e[5] and e[3] == 0]
    noyau_if0 = sum(1 for e in noyau if not e[4])
    print(f"echantillons={total} repos(HLT)={hlt} ({100*hlt/total:.1f}%) user={user} ({100*user/total:.1f}%) "
          f"noyau={len(noyau)} ({100*len(noyau)/total:.1f}%) dont_IF0={noyau_if0}")
    actifs = user + len(noyau)
    if actifs:
        print(f"part noyau du temps actif = {100*len(noyau)/actifs:.1f}%")
    adresses = [hex(int(e[2], 16)) for e in noyau]
    piles = symbolise(args.elf, adresses)
    feuille = collections.Counter()
    inclusif = collections.Counter()
    for a in adresses:
        pile = piles.get(a, [])
        if not pile:
            feuille["?"] += 1
            continue
        feuille[nom_court(pile[0][0])] += 1
        vus = set()
        for f, _ in pile:
            n = nom_court(f)
            if n not in vus:
                inclusif[n] += 1
                vus.add(n)
    print(f"\n== noyau : fonctions feuilles (top {args.top}) ==")
    for n, c in feuille.most_common(args.top):
        print(f"{c:6d} {100*c/max(1,len(noyau)):5.1f}%  {n[:150]}")
    print(f"\n== noyau : inclusif sur la pile d'inlining (top {args.top}) ==")
    for n, c in inclusif.most_common(args.top):
        print(f"{c:6d} {100*c/max(1,len(noyau)):5.1f}%  {n[:150]}")
    cr3 = collections.Counter(e[6] for e in ech if not e[5] and e[3] == 3)
    print("\n== user par espace d'adressage (CR3) ==")
    for c, n in cr3.most_common(12):
        print(f"{n:6d} cr3={c}")


def main():
    p = argparse.ArgumentParser()
    sp = p.add_subparsers(dest="cmd", required=True)
    c = sp.add_parser("collecte")
    c.add_argument("socket")
    c.add_argument("sortie")
    c.add_argument("--hz", type=float, default=200)
    c.add_argument("--duree", type=float, default=120)
    r = sp.add_parser("rapport")
    r.add_argument("json")
    r.add_argument("elf")
    r.add_argument("--top", type=int, default=40)
    r.add_argument("--depuis", type=float)
    r.add_argument("--jusqua", type=float)
    a = p.parse_args()
    collecte(a) if a.cmd == "collecte" else rapport(a)


if __name__ == "__main__":
    main()
