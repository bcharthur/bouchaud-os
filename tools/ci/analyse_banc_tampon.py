#!/usr/bin/env python3
"""Analyse de l'A/B copie / composition directe du scanout virtio-gpu
(BOUCHAUD_VIRTIO_GPU_TAMPONS_V1, P10+.2b).

    analyse_banc_tampon.py JOURNAL_SERIE TOURS ACCEL

Un demarrage (`-vga none -device virtio-gpu-pci`), TOURS tours ; chaque tour
passe `banc-present tache` sous `gpu-tampon copie` et `gpu-tampon direct`,
ordre AB aux tours impairs, BA aux pairs.

CRITERES DECLARES AVANT LA MESURE KVM (la politique par defaut est passee a la
composition directe sur l'A/B KVM CONVAINCANT de 77e0e5de, run 38704459647 ; un
A/B KVM NON_CONVAINCANT la rend a la copie) :

  surete     chaque banc : 640 presentations acceptees, aucun echec, scanout
             actif a la fin ; sous `direct` : zero octet copie dans chaque
             categorie, et aucun acces au tampon pendant un transfert ; sous
             `copie` : zero octet presente sans copie ; aucune panique ;
  plein      mediane (sur les tours) des p50 plein ecran : direct <= 0,85 x copie ;
  partiel    medianes des p50 bande et tuile : direct <= 1,10 x copie ;
  quorum     4 tours complets, sinon NON_CONCLUANT.

Sortie 1 sur un defaut de surete ou d'instrument ; 0 sinon. Teste :
tools/ci/reliability/test_banc_tampon.py.
"""
import re
import statistics
import sys

QUORUM = 4
CATEGORIES = {"plein": 40, "bande": 200, "tuile": 400}
LIGNE = re.compile(r"\[BANC-PRESENT\] ecran=virtio-gpu-2d categorie=(\w+) rect=\S+ n=(\d+) moyen_us=(\d+) "
                   r"p50_us=(\d+) p95_us=(\d+) .* tampon=(\w+) octets_copies=(\d+) octets_directs=(\d+) "
                   r"acces_en_transfert=(\d+)")
FIN = re.compile(r"\[BANC-PRESENT\] ecran=virtio-gpu-2d fin presentations_virtio_ok=(\d+) echecs_virtio=(\d+) actif=(\d)")
BRAS = re.compile(r"^AB_TAMPON tour=(\d+) tampon=(copie|direct)$")


def analyse(texte, tours, nom):
    sortie, fautes = [], []
    texte = re.sub(r"\x1b\[[0-9;]*m", "", texte)
    lignes = [re.sub(r"^(\[[^]]*\]){2}\[FPS:[^]]*\] ", "", l).rstrip("\r") for l in texte.splitlines()]
    if re.search(r"KERNEL PANIC|panicked at", texte):
        fautes.append("panique")
    if not any(l == "AB_TAMPON_FIN" for l in lignes):
        fautes.append("le scenario n'est pas alle au bout")
    bras, courant = [], None
    for l in lignes:
        m = BRAS.match(l)
        if m:
            courant = {"tour": int(m.group(1)), "v": m.group(2), "cat": {}, "fin": None}
            bras.append(courant)
            continue
        if courant is None:
            continue
        m = LIGNE.search(l)
        if m:
            courant["cat"][m.group(1)] = {
                "n": int(m.group(2)), "p50": int(m.group(4)), "p95": int(m.group(5)), "tampon": m.group(6),
                "copies": int(m.group(7)), "directs": int(m.group(8)), "acces": int(m.group(9))}
        m = FIN.search(l)
        if m:
            courant["fin"] = tuple(int(x) for x in m.groups())
    for b in bras:
        nb = f"tour {b['tour']} {b['v']}"
        if b["fin"] != (640, 0, 1):
            fautes.append(f"{nb} : banc incomplet ou en echec (fin={b['fin']})")
        for c, n in CATEGORIES.items():
            d = b["cat"].get(c)
            if d is None or d["n"] != n:
                fautes.append(f"{nb} : categorie {c} absente")
                continue
            if d["tampon"] != b["v"]:
                fautes.append(f"{nb} : {c} mesure sous {d['tampon']}")
            if b["v"] == "direct" and (d["copies"] != 0 or d["directs"] == 0):
                fautes.append(f"{nb} : {c} copie {d['copies']} octets en composition directe")
            if b["v"] == "copie" and d["directs"] != 0:
                fautes.append(f"{nb} : {c} presente {d['directs']} octets sans copie en mode copie")
            if d["acces"] != 0:
                fautes.append(f"{nb} : {c} {d['acces']} acces au tampon pendant un transfert")
    for t in range(1, tours + 1):
        ordre = [b["v"] for b in bras if b["tour"] == t]
        voulu = ["copie", "direct"] if t % 2 else ["direct", "copie"]
        if ordre and ordre != voulu:
            fautes.append(f"tour {t} : ordre {ordre}, attendu {voulu} (AB/BA)")
    par_tour = {}
    for b in bras:
        if b["fin"] == (640, 0, 1) and all(c in b["cat"] for c in CATEGORIES):
            par_tour.setdefault(b["tour"], {})[b["v"]] = b
    paires = sorted(t for t, d in par_tour.items() if set(d) == {"copie", "direct"})

    sortie.append(f"  A/B copie / composition directe, virtio-gpu 2D, {nom} : {len(paires)}/{tours} tours complets")
    sortie.append(f"  {'tour':<5}{'tampon':<8}" + "".join(f"{c + ' p50':>12}" for c in CATEGORIES) + f"{'copie plein':>14}")
    for b in bras:
        sortie.append(f"  {b['tour']:<5}{b['v']:<8}" + "".join(f"{b['cat'].get(c, {}).get('p50', 0):>12}" for c in CATEGORIES)
                      + f"{b['cat'].get('plein', {}).get('copies', 0):>14}")
    if fautes:
        sortie.extend(f"  DEFAUT {f}" for f in fautes)
        sortie.append(f"BANC_TAMPON_ECHEC n={len(fautes)} accel={nom}")
        return sortie, "DEFAUT", 1
    if len(paires) < QUORUM:
        sortie.append(f"BANC_TAMPON : NON_CONCLUANT accel={nom} tours={len(paires)}/{tours}")
        return sortie, "NON_CONCLUANT", 0

    def med(v, c):
        return statistics.median([par_tour[t][v]["cat"][c]["p50"] for t in paires])
    criteres = []
    for c, seuil in (("plein", 0.85), ("bande", 1.10), ("tuile", 1.10)):
        copie, direct = med("copie", c), med("direct", c)
        criteres.append((f"{c} p50 : direct {direct} <= {seuil:.2f} x copie {copie}", direct <= seuil * copie))
    for txt, ok in criteres:
        sortie.append(f"  {'ok     ' if ok else 'ECHOUE '} {txt}")
    verdict = "CONVAINCANT" if all(ok for _, ok in criteres) else "NON_CONVAINCANT"
    sortie.append(f"BANC_TAMPON : {verdict} accel={nom} tours={len(paires)}/{tours} (politique par defaut : direct depuis 77e0e5de ; un KVM NON_CONVAINCANT la rend a la copie)")
    return sortie, verdict, 0


def main(argv):
    if len(argv) != 4:
        print(__doc__, file=sys.stderr)
        return 2
    with open(argv[1], encoding="utf-8", errors="replace") as f:
        sortie, _, code = analyse(f.read(), int(argv[2]), argv[3])
    print("\n".join(sortie))
    return code


if __name__ == "__main__":
    sys.exit(main(sys.argv))
