#!/usr/bin/env python3
"""Analyse de l'A/B du reveil a l'echeance exacte (BOUCHAUD_REVEIL_PRECIS_V1, P13+).

    analyse_ab_reveil_precis.py JOURNAL_SERIE TOURS SMP ACCEL

Un seul demarrage, TOURS tours ; chaque tour passe `ordonnanceur-probe 8 1`
sous `reveil-precis off` et `on`, ordre alterne (AB aux tours impairs, BA aux
pairs). Ce qui est compare : le RETARD DE REVEIL AU REPOS (la phase ou le
coeur dort et ou seul le minuteur peut reveiller), et que la charge n'en
souffre pas.

CRITERES DECLARES AVANT LA MESURE (la politique par defaut est passee a `on`
sur l'A/B KVM CONVAINCANT de b6c99032, run 38732049308 ; un A/B KVM
NON_CONVAINCANT la rend a `off`) :

  surete     chaque passe rend un verdict ; aucune panique ; aucun
             QUALIFIE_ECHEC sous `on` ; armements precis > 0 pendant les bras
             `on`, = 0 pendant les bras `off` ;
  repos      mediane (sur les tours) des p50 au repos : on <= 0,75 x off ;
             mediane des p95 au repos : on <= off ;
  queue      mediane des p99 au repos : on <= 1,25 x off ;
  charge     mediane des p95 interactifs sous charge : on <= 1,10 x off ;
  quorum     au moins 4 tours complets, sinon NON_CONCLUANT.

Sortie 1 sur un defaut de surete ou d'instrument ; 0 sinon (CONVAINCANT,
NON_CONVAINCANT, NON_CONCLUANT sont des mesures). Teste :
tools/ci/reliability/test_ab_reveil_precis.py.
"""
import re
import statistics
import sys

QUORUM = 4
# Les valeurs PROPRES (ligne `[propres ...]`), comme les A/B IPI et cession.
# Cherchee n'importe ou dans la ligne : sous KVM, le noyau ecrit des releves
# (`[SCHED-TACHE] ...`) sur la meme ligne que la sonde (run 38704459647 : six
# bras sur huit sans mesure lue).
QUANT = re.compile(r"(au repos|sous charge \((?:normal|interactif)\))\s+median\s+(\d+) us\s+p95\s+(\d+) us"
                   r"\s+p99\s+(\d+) us\s+pire\s+(\d+) us\s+\[propres")
BRAS = re.compile(r"^AB_PRECIS tour=(\d+) reveil=(on|off)$")
FIN_BRAS = re.compile(r"^AB_PRECIS_BRAS_FIN tour=(\d+) reveil=(on|off)$")
COMPTEUR = re.compile(r"REVEIL_PRECIS voulu=(\d) minuteur=(\S+) armes=(\d+)")


def analyse(texte, tours, smp, nom):
    sortie, fautes = [], []
    texte = re.sub(r"\x1b\[[0-9;]*m", "", texte)
    lignes = [re.sub(r"^(\[[^]]*\]){2}\[FPS:[^]]*\] ", "", l).rstrip("\r") for l in texte.splitlines()]
    if re.search(r"KERNEL PANIC|panicked at", texte):
        fautes.append("panique")
    bras, courant = [], None
    for l in lignes:
        m = BRAS.match(l)
        if m:
            if courant is not None:
                fautes.append(f"tour {courant['tour']} {courant['v']} : bras precedent non ferme")
            courant = {"tour": int(m.group(1)), "v": m.group(2), "q": {}, "verdict": None,
                       "compteurs": [], "diag": [], "ferme": False}
            bras.append(courant)
            continue
        m = FIN_BRAS.match(l)
        if m:
            if courant is None:
                fautes.append("fin de bras sans debut")
            elif int(m.group(1)) != courant["tour"] or m.group(2) != courant["v"]:
                fautes.append(f"fermeture du mauvais bras : {m.group(0)}")
                courant = None
            else:
                courant["ferme"] = True
                courant = None
            continue
        if courant is None:
            continue
        q = QUANT.search(l)
        if q:
            courant["q"][q.group(1)] = tuple(int(x) for x in q.groups()[1:])
        m = re.match(r"^VERDICT_SONDE : (\S+)", l)
        if m:
            courant["verdict"] = m.group(1)
        m = COMPTEUR.search(l)
        if m:
            courant["compteurs"].append((int(m.group(1)), m.group(2), int(m.group(3))))
        # Diagnostic seulement (aucun effet sur le verdict) : ce que le noyau
        # dit des reveils en retard au repos -- qui tenait le coeur, quelle
        # etape a coute. Run 38732049308 : sous `on`, p50 39 us mais p95 852 ;
        # la queue reste a attribuer.
        d = re.search(r"(attribution|ventilation|comptes) au repos : .*", l)
        if d:
            courant["diag"].append(d.group(0)[:400])
    if courant is not None:
        fautes.append(f"tour {courant['tour']} {courant['v']} : bras final non ferme")
    minuteur = "?"
    for b in bras:
        nb = f"tour {b['tour']} {b['v']}"
        if not b["ferme"]:
            fautes.append(f"{nb} : marqueur de fermeture absent")
        if b["verdict"] is None:
            fautes.append(f"{nb} : passe sans verdict")
        if b["v"] == "on" and b["verdict"] == "QUALIFIE_ECHEC":
            fautes.append(f"{nb} : QUALIFIE_ECHEC sous le reveil precis")
        if "au repos" not in b["q"] or "sous charge (interactif)" not in b["q"]:
            fautes.append(f"{nb} : mesure incomplete")
        # Le releve de la bascule (avant la sonde) et celui qui la suit.
        if len(b["compteurs"]) != 2:
            fautes.append(f"{nb} : exactement deux compteurs attendus, trouves {len(b['compteurs'])}")
            continue
        (voulu0, minuteur, armes0), (voulu1, _, armes1) = b["compteurs"][0], b["compteurs"][-1]
        attendu = 1 if b["v"] == "on" else 0
        if voulu0 != attendu or voulu1 != attendu:
            fautes.append(f"{nb} : politique {voulu0}/{voulu1}, attendue {attendu}")
        delta = armes1 - armes0
        b["armes"] = delta
        if b["v"] == "on" and delta <= 0:
            fautes.append(f"{nb} : aucun armement precis (minuteur {minuteur})")
        if b["v"] == "off" and delta != 0:
            fautes.append(f"{nb} : {delta} armement(s) precis sous off")
    for t in range(1, tours + 1):
        ordre = [b["v"] for b in bras if b["tour"] == t]
        voulu = ["off", "on"] if t % 2 else ["on", "off"]
        if ordre and ordre != voulu:
            fautes.append(f"tour {t} : ordre {ordre}, attendu {voulu} (AB/BA)")
    par_tour = {}
    for b in bras:
        if b["verdict"] is not None and "au repos" in b["q"] and "sous charge (interactif)" in b["q"]:
            par_tour.setdefault(b["tour"], {})[b["v"]] = b
    paires = sorted(t for t, d in par_tour.items() if set(d) == {"on", "off"})

    sortie.append(f"  A/B reveil precis, {nom} SMP{smp}, minuteur {minuteur} : {len(paires)}/{tours} tours complets")
    sortie.append(f"  {'tour':<5}{'bras':<5}{'verdict':<16}{'repos p50':>10}{'p95':>7}{'p99':>8}{'pire':>8}{'inter p95':>10}{'armes':>8}")
    for b in bras:
        r = b["q"].get("au repos", (0, 0, 0, 0))
        i = b["q"].get("sous charge (interactif)", (0, 0, 0, 0))
        sortie.append(f"  {b['tour']:<5}{b['v']:<5}{str(b['verdict']):<16}{r[0]:>10}{r[1]:>7}{r[2]:>8}{r[3]:>8}{i[1]:>10}{b.get('armes', 0):>8}")

    for b in bras:
        for d in b["diag"]:
            sortie.append(f"  diag tour {b['tour']} {b['v']} : {d}")

    def med(v, phase, k):
        return statistics.median([par_tour[t][v]["q"][phase][k] for t in paires]) if paires else 0
    criteres = []
    if paires:
        p50 = (med("off", "au repos", 0), med("on", "au repos", 0))
        p95 = (med("off", "au repos", 1), med("on", "au repos", 1))
        p99 = (med("off", "au repos", 2), med("on", "au repos", 2))
        inter = (med("off", "sous charge (interactif)", 1), med("on", "sous charge (interactif)", 1))
        criteres = [
            (f"repos p50 : on {p50[1]} <= 0,75 x off {p50[0]}", p50[1] <= 0.75 * p50[0]),
            (f"repos p95 : on {p95[1]} <= off {p95[0]}", p95[1] <= p95[0]),
            (f"queue p99 au repos : on {p99[1]} <= 1,25 x off {p99[0]}", p99[1] <= 1.25 * p99[0]),
            (f"charge p95 interactif : on {inter[1]} <= 1,10 x off {inter[0]}", inter[1] <= 1.10 * inter[0]),
        ]
        for txt, ok in criteres:
            sortie.append(f"  {'ok     ' if ok else 'ECHOUE '} {txt}")
    if fautes:
        sortie.extend(f"  DEFAUT {f}" for f in fautes)
        sortie.append(f"AB_REVEIL_PRECIS_ECHEC n={len(fautes)} accel={nom} smp={smp}")
        return sortie, "DEFAUT", 1
    if len(paires) < QUORUM:
        sortie.append(f"AB_REVEIL_PRECIS : NON_CONCLUANT accel={nom} smp={smp} tours={len(paires)}/{tours}")
        return sortie, "NON_CONCLUANT", 0
    verdict = "CONVAINCANT" if all(ok for _, ok in criteres) else "NON_CONVAINCANT"
    sortie.append(f"AB_REVEIL_PRECIS : {verdict} accel={nom} smp={smp} tours={len(paires)}/{tours} "
                  f"(politique par defaut : on depuis b6c99032 ; un KVM NON_CONVAINCANT la rend a off)")
    return sortie, verdict, 0


def main():
    journal, tours, smp, nom = sys.argv[1], int(sys.argv[2]), sys.argv[3], sys.argv[4]
    with open(journal, encoding="utf-8", errors="replace") as f:
        lignes, _, code = analyse(f.read(), tours, smp, nom)
    print("\n".join(lignes))
    return code


if __name__ == "__main__":
    sys.exit(main())
