#!/usr/bin/env python3
"""Analyse de l'A/B de la cession au quantum (BOUCHAUD_CESSION_PAR_CLASSE_V1).

    analyse_ab_cession.py JOURNAL_SERIE TOURS SMP ACCEL

Un demarrage, TOURS tours ; chaque tour passe `ordonnanceur-probe 8 1` sous
`ordo-cession historique` (une interactive cede a toute tache en attente) et
`classe` (une interactive sous budget garde son coeur face a des normales),
ordre AB aux tours impairs, BA aux pairs.

Seules les passes QUALIFIEES (QUALIFIE_OK / QUALIFIE_ECHEC) comptent, par tours
APPARIES ; moins de QUORUM tours apparies : NON_CONCLUANT. Sous KVM, chaque
passe doit porter la preuve de l'hote.

CRITERES DECLARES AVANT LA MESURE KVM (valeurs PROPRES de la phase interactive) :

  latence     mediane des p99 : classe <= historique, et mediane des pires :
              classe <= historique ;
  debit       mediane des tours de calcul en phase interactive :
              classe >= 0,90 x historique (la normale n'est pas affamee) ;
  equite      phase NORMALE (la regle n'y agit pas) : ecart <= 5 % ;
  robustesse  aucun QUALIFIE_ECHEC sous classe.

DEFAUTS (sortie 1) : passe sans verdict, bascule non relevee dans son bras,
cession gardee sous historique ou inversion laissee sous classe (le compteur
de l'autre politique avance), panique, preuve de l'hote absente sous KVM.
Sinon sortie 0 : CONVAINCANT / NON_CONVAINCANT / NON_CONCLUANT sont des
mesures. NON_CONCLUANT aussi quand la regle n'a jamais agi (aucune cession
gardee) : l'A/B n'a rien compare. Teste : tools/ci/reliability/test_ab_cession.py.
"""
import math
import re
import statistics
import sys

QUANT = re.compile(r"(sous charge \((?:normal|interactif)\)|au repos)\s+median\s+(\d+) us\s+p95\s+(\d+) us"
                   r"\s+p99\s+(\d+) us\s+pire\s+(\d+) us\s+\[propres")
COMPTEURS = re.compile(r"ORDO_CESSION politique=(\S+) gardees=(\d+) inversees=(\d+)")
H, C = "historique", "classe"


def analyse(texte, tours, smp, nom):
    sortie, fautes = [], []
    texte = re.sub(r"\x1b\[[0-9;]*m", "", texte)
    lignes = [re.sub(r"^(\[[^]]*\]){2}\[FPS:[^]]*\] ", "", l).rstrip("\r") for l in texte.splitlines()]
    bras, courant = [], None
    for l in lignes:
        # La sortie de `echo`, pas sa trace (`+ echo AB_CESSION ...`).
        m = re.match(r"^AB_CESSION tour=(\d+) politique=(\S+)$", l)
        if m:
            t = int(m.group(1))
            courant = {"tour": t, "v": m.group(2), "position": sum(1 for b in bras if b["tour"] == t),
                       "q": {}, "verdict": None, "compteurs": [], "tours": [], "preuve": set(), "detail": []}
            bras.append(courant)
            continue
        if courant is None:
            continue
        q = QUANT.search(l)
        if q:
            courant["q"][q.group(1)] = tuple(int(x) for x in q.groups()[1:])
        m = re.search(r"(\d+) tours de calcul", l)
        if m:
            courant["tours"].append(int(m.group(1)))
        m = re.match(r"^VERDICT_SONDE : (\S+)", l)
        if m:
            courant["verdict"] = m.group(1)
        # Diagnostic seulement : les comparaisons de la sonde qui n'ont pas
        # rendu QUALIFIE_OK, recopiees pour un bras non qualifie (le journal
        # serie n'est pas toujours lisible hors du job).
        d = re.search(r"PREUVE (.*-> (?:QUALIFIE_ECHEC|NON_QUALIFIANT).*)$", l)
        if d:
            courant["detail"].append(d.group(1)[:320])
        # Diagnostic seulement : la decomposition des pires reveils sous
        # charge (run 39667691144 : deux QUALIFIE_ECHEC sous classe, pire
        # 832 et 1 357 us, sans que le job dise ou le temps est passe).
        d = re.search(r"(attribution|ventilation) sous charge \((?:normal|interactif)\) : .*", l)
        if d:
            courant.setdefault("diag_charge", []).append(d.group(0)[:1100])
        m = re.match(r"^\s+comptes .* : preuve_hote=(\S+) ", l)
        if m:
            courant["preuve"].add(m.group(1))
        # Le releve de la bascule (avant le verdict) est le DEPART, le premier
        # releve apres le verdict la FIN ; la remise finale a `classe` n'est
        # pas au bras.
        m = COMPTEURS.search(l)
        if m and (courant["verdict"] is None or len(courant["compteurs"]) < 2):
            if courant["verdict"] is None and courant["compteurs"]:
                continue
            courant["compteurs"].append((m.group(1), int(m.group(2)), int(m.group(3))))
    if re.search(r"KERNEL PANIC|panicked at", texte):
        fautes.append("surete : panique")
    if not any(re.match(r"^AB_CESSION_FIN$", l) for l in lignes):
        fautes.append("le scenario n'est pas alle au bout")
    if len(bras) != 2 * tours:
        fautes.append(f"{len(bras)} passes sur {2 * tours}")
    gardees_classe = 0
    for b in bras:
        nb = f"tour {b['tour']} {b['v']}"
        if b["verdict"] is None:
            fautes.append(f"{nb} : passe sans verdict")
        if nom == "kvm" and b["preuve"] != {"kvm-steal"}:
            fautes.append(f"{nb} : sous KVM sans preuve de l'hote ({sorted(b['preuve']) or '-'})")
        if len(b["tours"]) >= 2:
            b["tours_normal"], b["tours_interactif"] = b["tours"][0], b["tours"][1]
        b["qualifie"] = b["verdict"] in ("QUALIFIE_OK", "QUALIFIE_ECHEC") \
            and "sous charge (interactif)" in b["q"] and "tours_interactif" in b
        if len(b["compteurs"]) != 2 or b["compteurs"][0][0] != b["v"] or b["compteurs"][1][0] != b["v"]:
            fautes.append(f"{nb} : bascule non relevee dans le bras")
            continue
        (_, g0, i0), (_, g1, i1) = b["compteurs"]
        b["gardees"], b["inversees"] = g1 - g0, i1 - i0
        if b["v"] == H and b["gardees"] != 0:
            fautes.append(f"surete : {b['gardees']} cession(s) gardee(s) sous historique ({nb})")
        if b["v"] == C and b["inversees"] != 0:
            fautes.append(f"surete : {b['inversees']} inversion(s) laissee(s) sous classe ({nb})")
        if b["v"] == C:
            gardees_classe += b["gardees"]
    for t in range(1, tours + 1):
        ordre = [b["v"] for b in bras if b["tour"] == t]
        voulu = [H, C] if t % 2 else [C, H]
        if ordre and ordre != voulu:
            fautes.append(f"tour {t} : ordre {ordre}, attendu {voulu} (AB/BA)")

    sortie.append(f"  A/B cession au quantum, {nom} SMP{smp}, {tours} tours AB/BA (valeurs PROPRES de la phase interactive)")
    sortie.append(f"  {'tour':<5}{'pos':<4}{'politique':<12}{'verdict':<16}{'p99':>7}{'pire':>8}{'tours_int':>11}"
                  f"{'tours_norm':>11}{'gardees':>9}{'inversees':>10}")
    for b in bras:
        i = b["q"].get("sous charge (interactif)", (0, 0, 0, 0))
        sortie.append(f"  {b['tour']:<5}{b['position']:<4}{b['v']:<12}{str(b['verdict']):<16}{i[2]:>7}{i[3]:>8}"
                      f"{b.get('tours_interactif', 0):>11}{b.get('tours_normal', 0):>11}{b.get('gardees', 0):>9}"
                      f"{b.get('inversees', 0):>10}")
    for b in bras:
        if b["verdict"] != "QUALIFIE_OK":
            for d in b["detail"]:
                sortie.append(f"  detail tour {b['tour']} {b['v']} : {d}")
    # Les deux bras d'un tour ou l'un echoue : la comparaison se lit cote a cote.
    tours_echec = {b["tour"] for b in bras if b["verdict"] == "QUALIFIE_ECHEC"}
    for b in bras:
        if b["tour"] in tours_echec:
            for d in b.get("diag_charge", []):
                sortie.append(f"  diag charge tour {b['tour']} {b['v']} : {d}")
    par_tour = {}
    for b in bras:
        par_tour.setdefault(b["tour"], {})[b["v"]] = b
    paires = [(t, v[H], v[C]) for t, v in sorted(par_tour.items())
              if H in v and C in v and v[H].get("qualifie") and v[C].get("qualifie")]
    quorum = max(3, math.ceil(2 * tours / 3))
    inversions = sum(b.get("inversees", 0) for b in bras if b["v"] == H)
    sortie.append(f"  tours apparies {len(paires)} (quorum {quorum}) ; inversions sous historique {inversions}, "
                  f"cessions gardees sous classe {gardees_classe}")
    if fautes:
        sortie.extend(f"  ECHEC   {f}" for f in fautes)
        sortie.append(f"AB_CESSION_ECHEC n={len(fautes)} accel={nom} smp={smp}")
        return sortie, "ECHEC", 1
    if len(paires) < quorum or gardees_classe == 0:
        raison = "trop peu de mesures qualifiees" if len(paires) < quorum else "la regle n'a jamais agi"
        sortie.append(f"AB_CESSION : NON_CONCLUANT accel={nom} smp={smp} tours={tours} apparies={len(paires)} ({raison})")
        return sortie, "NON_CONCLUANT", 0
    med = statistics.median
    p99 = (med([h["q"]["sous charge (interactif)"][2] for _, h, _ in paires]),
           med([c["q"]["sous charge (interactif)"][2] for _, _, c in paires]))
    pire = (med([h["q"]["sous charge (interactif)"][3] for _, h, _ in paires]),
            med([c["q"]["sous charge (interactif)"][3] for _, _, c in paires]))
    debit = (med([h["tours_interactif"] for _, h, _ in paires]), med([c["tours_interactif"] for _, _, c in paires]))
    equite = (med([h["tours_normal"] for _, h, _ in paires]), med([c["tours_normal"] for _, _, c in paires]))
    echecs = sum(1 for b in bras if b["v"] == C and b["verdict"] == "QUALIFIE_ECHEC")
    criteres = [
        ("latence", p99[1] <= p99[0] and pire[1] <= pire[0],
         f"p99 median {p99[0]} -> {p99[1]} us, pire median {pire[0]} -> {pire[1]} us"),
        ("debit", debit[1] >= 0.90 * debit[0], f"tours (phase interactive) {debit[0]} -> {debit[1]}"),
        ("equite", abs(equite[1] - equite[0]) <= 0.05 * equite[0], f"tours (phase normale) {equite[0]} -> {equite[1]}"),
        ("robustesse", echecs == 0, f"QUALIFIE_ECHEC sous classe : {echecs}"),
    ]
    for n, ok, detail in criteres:
        sortie.append(f"  {'ok     ' if ok else 'NON    '} {n:<11} {detail}")
    verdict = "CONVAINCANT" if all(ok for _, ok, _ in criteres) else "NON_CONVAINCANT"
    sortie.append(f"AB_CESSION : {verdict} accel={nom} smp={smp} tours={tours} apparies={len(paires)} "
                  "(politique par defaut : classe)")
    return sortie, verdict, 0


def main(argv):
    if len(argv) != 5:
        print(__doc__, file=sys.stderr)
        return 2
    with open(argv[1], encoding="utf-8", errors="replace") as f:
        sortie, _, code = analyse(f.read(), int(argv[2]), argv[3], argv[4])
    print("\n".join(sortie))
    return code


if __name__ == "__main__":
    sys.exit(main(sys.argv))
