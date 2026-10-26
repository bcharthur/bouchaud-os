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

Politique decisions-20261024 (docs/p13/DECISIONS_2026-10-24.md) : le bras
on doit qualifier ; off est un controle negatif valide. P3 n'est comparable
que si p95 normal > p95 repos + max(0, p95 repos - p50 repos), sur les
observations propres du meme bras. Sinon P3 reste NON_QUALIFIANT, visible
et non bloquant si les autres proprietes sont qualifiees. Aucun seuil
P1/P2/P4/P5 n'est modifie. Le verdict historique est conserve en parallele.
Un defaut d'instrument ou un echec applicable retourne 1 ; NON_CONCLUANT
reste une mesure incomplete, refusee par la porte stricte qualifie_p13.py.
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
# Diagnostic de la queue au repos (cumuls du noyau ; differences par bras).
# Lu cle par cle sur la ligne REVEIL_PRECIS : un champ absent (noyau plus
# ancien) vaut 0. `serie_pire_us` est un maximum cumule : sa valeur finale.
DIAG_NOMS = ("tirs", "bornes_propres", "ipi_pendant_arme", "armes_dus", "refus_quantum_avant",
             "refus_sans_borne", "refus_autres", "tardifs_soi", "tardifs_autre", "tardifs_irq", "tardifs_inactif",
             "installes_ipi", "installes_eveilles", "serie_longues", "serie_pire_us",
             "armes_occupes", "tirs_anticipes", "avances_occupees",
             "attentes_noyau", "attentes_utilisateur", "attente_noyau_pire_us", "attente_utilisateur_pire_us",
             "cessions_interactives",
             # BOUCHAUD_ATTENTE_INTERACTIVE_CLASSEE_V1 : attentes > 1 ms par cause, et
             # battements qui n'ont pas commute, par raison.
             "longues_volees", "longues_b0", "longues_b1", "longues_b1_retard", "longues_b2",
             "renonce_anticipe", "renonce_file_vide", "renonce_garde", "renonce_rien", "renonce_meme",
             "renonce_transition", "renonce_porte", "renonce_differee_appel", "renonce_differee_fil_noyau",
             # BOUCHAUD_CONSOLE_ASYNCHRONE_V1 : diagnostic, aucun critere.
             "console_morceau_pire_us", "console_ligne_pire_us", "console_synchrones", "console_synchrone_pire_us")
MAXIMA = ("serie_pire_us", "attente_noyau_pire_us", "attente_utilisateur_pire_us",
          "console_morceau_pire_us", "console_ligne_pire_us", "console_synchrone_pire_us")


def diag_noyau(ligne):
    """Les compteurs de diagnostic d'une ligne REVEIL_PRECIS, ou None."""
    if "tirs=" not in ligne:
        return None
    vals = dict(re.findall(r"\b(\w+)=(\d+)\b", ligne))
    return tuple(int(vals.get(n, 0)) for n in DIAG_NOMS)


# BOUCHAUD_AB_CINQ_PROPRIETES_V1 : ce que chaque bras dit de sa DERNIERE
# tentative (la sonde remesure un NON_QUALIFIANT : seule la derniere compte).
PHASES = ("au repos", "sous charge (normal)", "sous charge (interactif)")
TENTATIVE = re.compile(r"^\s*tentative (\d+)/(\d+)$")
TOURS_CALCUL = re.compile(r"^\s+(\d+) tours de calcul$")
NON_JUGEABLE = re.compile(r"phase non jugeable -- (au repos|sous charge \((?:normal|interactif)\)) : (.*)$")
ATTRIBUTION = re.compile(r"^\s*attribution (au repos|sous charge \((?:normal|interactif)\)) : ")
GRAIN = re.compile(r"horloge : granularite (\d+) us")
OK, ECHEC, NQ = "QUALIFIE_OK", "QUALIFIE_ECHEC", "NON_QUALIFIANT"


def _p_neuf():
    return {"q": {}, "tours": [], "non_jugeable": {}, "attribution": set(),
            "reveils_attribues": {}, "effectifs": {}}


def _phase_valide(p, phase):
    """None si la phase est jugeable, sinon la raison."""
    if phase not in p["q"]:
        return f"{phase} : mesure absente"
    if phase in p["non_jugeable"]:
        return p["non_jugeable"][phase][:160]
    return None


def _agrege(par_bras, quorum):
    """ECHEC si un bras qualifie echoue ; OK si au moins `quorum` bras
    qualifies et aucun echec ; NON_QUALIFIANT sinon (jamais un succes)."""
    echecs = [b for b, (v, _) in par_bras if v == ECHEC]
    oks = [b for b, (v, _) in par_bras if v == OK]
    if echecs:
        return ECHEC, f"{len(echecs)} bras en echec, {len(oks)} qualifies OK sur {len(par_bras)}"
    if len(oks) >= quorum:
        return OK, f"{len(oks)}/{len(par_bras)} bras qualifies"
    return NQ, f"{len(oks)}/{len(par_bras)} bras qualifies (quorum {quorum})"


def cinq_proprietes(bras, tours, smp, nom, grain, politique=False):
    """Les cinq proprietes, avec leur statut et les mesures par bras."""
    sortie = ["  proprietes (BOUCHAUD_AB_CINQ_PROPRIETES_V1, proposition closeout 20/10 §6.5, "
              "NON JUGE : le verdict historique reste seul juge)"]
    if politique:
        sortie = ["  proprietes (decisions-20261024, P3 non comparable reste NON_QUALIFIANT)"]
    verdicts = {}
    par_tour = {}
    for b in bras:
        par_tour.setdefault(b["tour"], {})[b["v"]] = b
    paires = sorted(t for t, d in par_tour.items() if set(d) == {"on", "off"})

    def med(ts, v, phase, k):
        return statistics.median([par_tour[t][v]["p"]["q"][phase][k] for t in ts])

    # P1 precision au repos : A/B sur les paires dont les DEUX phases au
    # repos sont valides (jugeables, attribution presente) -- la porte de
    # comparaison des priorites n'est pas la question de l'A/B (§6.1 a).
    def valide_repos(b):
        r = _phase_valide(b["p"], "au repos")
        if r:
            return r
        if "au repos" not in b["p"]["attribution"]:
            return "au repos : attribution absente"
        return None
    p1 = [t for t in paires if not valide_repos(par_tour[t]["on"]) and not valide_repos(par_tour[t]["off"])]
    if len(p1) < QUORUM:
        raisons = sorted({r for t in paires for v in ("on", "off") for r in [valide_repos(par_tour[t][v])] if r})
        verdicts["P1"] = (NQ, f"paires valides {len(p1)}/{tours} (quorum {QUORUM})"
                          + (f" : {raisons[0]}" if raisons else ""))
    else:
        p50 = (med(p1, "off", "au repos", 0), med(p1, "on", "au repos", 0))
        p95 = (med(p1, "off", "au repos", 1), med(p1, "on", "au repos", 1))
        p99 = (med(p1, "off", "au repos", 2), med(p1, "on", "au repos", 2))
        c = [p50[1] <= 0.75 * p50[0], p95[1] <= p95[0], p99[1] <= 1.25 * p99[0]]
        verdicts["P1"] = (OK if all(c) else ECHEC,
                          f"paires {len(p1)}/{tours} : p50 on {p50[1]} <= 0,75 x off {p50[0]} ; "
                          f"p95 on {p95[1]} <= off {p95[0]} ; p99 on {p99[1]} <= 1,25 x off {p99[0]}")

    # P2 precision sous charge : p95 interactif, on <= 1,10 x off.
    inter = "sous charge (interactif)"
    p2 = [t for t in paires if not _phase_valide(par_tour[t]["on"]["p"], inter)
          and not _phase_valide(par_tour[t]["off"]["p"], inter)]
    if len(p2) < QUORUM:
        verdicts["P2"] = (NQ, f"paires valides {len(p2)}/{tours} (quorum {QUORUM})")
    else:
        i95 = (med(p2, "off", inter, 1), med(p2, "on", inter, 1))
        verdicts["P2"] = (OK if i95[1] <= 1.10 * i95[0] else ECHEC,
                          f"paires {len(p2)}/{tours} : p95 interactif on {i95[1]} <= 1,10 x off {i95[0]}")

    # P3, P4, P5 : par bras, sur la derniere tentative.
    p3, p4, p5 = [], [], []
    for b in bras:
        nb = f"tour {b['tour']} {b['v']}"
        p = b["p"]
        manque = next((r for r in (_phase_valide(p, ph) for ph in PHASES) if r), None)
        if manque:
            p3.append((nb, (NQ, manque)))
            p4.append((nb, (NQ, manque)))
        else:
            repos, normal, prio = p["q"]["au repos"], p["q"]["sous charge (normal)"], p["q"][inter]
            # P3 : la priorite raccourcit la queue -- jugee seulement si la
            # charge degrade la phase normale au-dela du BRUIT MESURE AU REPOS
            # (p95 - p50 au repos), sans constante de remplacement (D-6.2 a).
            bruit = max(0, repos[1] - repos[0])
            if normal[1] <= repos[1] + bruit:
                p3.append((nb, (NQ, f"charge dans le bruit : p95 normal {normal[1]} <= p95 repos {repos[1]} "
                                    f"+ bruit {bruit}")))
            else:
                ok = prio[2] < normal[2] or prio[3] < normal[3]
                p3.append((nb, (OK if ok else ECHEC,
                                f"p99 inter {prio[2]} < normal {normal[2]} ou pire inter {prio[3]} < normal {normal[3]}")))
            # P4 : le pire cas n'est pas aggrave au-dela du bruit au repos.
            ok = prio[3] <= normal[3] + repos[3]
            p4.append((nb, (OK if ok else ECHEC,
                            f"pire inter {prio[3]} <= normal {normal[3]} + pire repos {repos[3]}")))
        # P5 : equite, independante de tout reveil.
        if len(p["tours"]) < 2:
            p5.append((nb, (NQ, "tours de calcul absents")))
        else:
            normale, prio_t = p["tours"][-2], p["tours"][-1]
            ok = prio_t > 0 and (normale == 0 or prio_t * 10 >= normale)
            p5.append((nb, (OK if ok else ECHEC, f"tours interactif {prio_t} > 0 et x 10 >= normal {normale}")))
    for cle, liste in (("P3", p3), ("P4", p4), ("P5", p5)):
        verdicts[cle] = _agrege(liste, QUORUM)
        for nb, (v, txt) in liste:
            if v != OK:
                sortie.append(f"    {cle} {nb} : {v} -- {txt}")
    noms = {"P1": "precision au repos", "P2": "precision sous charge", "P3": "gain de la priorite",
            "P4": "pas d'aggravation du pire cas", "P5": "equite, non-famine"}
    for cle in ("P1", "P2", "P3", "P4", "P5"):
        v, txt = verdicts[cle]
        sortie.append(f"  {cle} {noms[cle]} : {v} -- {txt}")
    sortie.append("AB_PROPRIETES_V1 : " + " ".join(f"{k}={verdicts[k][0]}" for k in ("P1", "P2", "P3", "P4", "P5"))
                  + f" accel={nom} smp={smp}" + ("" if politique else " (historique, diagnostic)"))
    return sortie, {k: v for k, (v, _) in verdicts.items()}


def analyse_historique(texte, tours, smp, nom, preuves=None):
    sortie, fautes = [], []
    texte = re.sub(r"\x1b\[[0-9;]*m", "", texte)
    lignes = [re.sub(r"^(\[[^]]*\]){2}\[FPS:[^]]*\] ", "", l).rstrip("\r") for l in texte.splitlines()]
    if re.search(r"KERNEL PANIC|panicked at", texte):
        fautes.append("panique")
    bras, courant, grain = [], None, 0
    for l in lignes:
        m = BRAS.match(l)
        if m:
            if courant is not None:
                fautes.append(f"tour {courant['tour']} {courant['v']} : bras precedent non ferme")
            courant = {"tour": int(m.group(1)), "v": m.group(2), "q": {}, "verdict": None,
                       "compteurs": [], "diag": [], "noyau": [], "detail": [], "ferme": False,
                       "p": _p_neuf()}
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
            courant["p"]["q"][q.group(1)] = courant["q"][q.group(1)]
            effectifs = re.search(r"\[propres (\d+)/(\d+), regle=([^\]]+)\]", l)
            if effectifs:
                courant["p"]["effectifs"][q.group(1)] = (int(effectifs[1]), int(effectifs[2]), effectifs[3])
        # BOUCHAUD_AB_CINQ_PROPRIETES_V1 : l'etat de la tentative en cours.
        if TENTATIVE.match(l):
            courant["p"] = _p_neuf()
        m = GRAIN.search(l)
        if m:
            grain = int(m.group(1))
        m = TOURS_CALCUL.match(l)
        if m:
            courant["p"]["tours"].append(int(m.group(1)))
        m = NON_JUGEABLE.search(l)
        if m:
            courant["p"]["non_jugeable"][m.group(1)] = m.group(2)
        m = ATTRIBUTION.match(l)
        if m:
            courant["p"]["attribution"].add(m.group(1))
            compte = re.search(r"\breveils=(\d+)\b", l)
            if compte:
                courant["p"]["reveils_attribues"][m.group(1)] = int(compte[1])
        m = re.match(r"^VERDICT_SONDE : (\S+)(?: tentatives=\d+)?(?: raison=(.*))?", l)
        if m:
            courant["verdict"] = m.group(1)
            if m.group(2):
                courant["raison"] = m.group(2)[:300]
        m = COMPTEUR.search(l)
        if m:
            courant["compteurs"].append((int(m.group(1)), m.group(2), int(m.group(3))))
            n = diag_noyau(l)
            if n is not None:
                courant["noyau"].append(n)
            # BOUCHAUD_ATTENTE_INTERACTIVE_OCCUPANT_V1 : qui occupait le coeur
            # au pire cas (texte, valeur finale du bras).
            o = re.search(r"occupant_noyau=(\S+) occupant_utilisateur=(\S+)", l)
            if o:
                courant["occupants"] = o.groups()
        # Diagnostic seulement (aucun effet sur le verdict) : ce que le noyau
        # dit des reveils en retard au repos -- qui tenait le coeur, quelle
        # etape a coute. Run 38732049308 : sous `on`, p50 39 us mais p95 852 ;
        # la queue reste a attribuer.
        d = re.search(r"(attribution|ventilation|comptes) au repos : .*", l)
        if d:
            courant["diag"].append(d.group(0)[:400])
        # Diagnostic seulement : la meme decomposition SOUS CHARGE, pour
        # chaque bras. Run 39667691144 (6e8bf33e, KVM) : P2 (p95 interactif
        # on 81 contre off 48) et P4 (pire interactif 854 us) en echec sans
        # que le job dise ou le temps est passe -- les artefacts serie ne
        # sont pas toujours lisibles hors du job.
        d = re.search(r"(attribution|ventilation|comptes) sous charge \((?:normal|interactif)\) : .*", l)
        if d:
            courant.setdefault("diag_charge", []).append(d.group(0)[:1100])
        # Diagnostic seulement : la comparaison de la sonde qui a echoue,
        # chiffres compris. Run 39447642999 (d02e8294, KVM SMP4) : deux
        # QUALIFIE_ECHEC sous `on` au repos irreprochable (pire 154 us), sans
        # que le job dise quel critere -- le journal serie n'est pas lisible
        # hors du job. Comme les A/B IPI et cession.
        d = re.search(r"PREUVE (.*-> QUALIFIE_ECHEC.*)$", l)
        if d:
            courant["detail"].append(d.group(1)[:400])
    if courant is not None:
        fautes.append(f"tour {courant['tour']} {courant['v']} : bras final non ferme")
    if preuves is not None:
        preuves.extend(bras)
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
    # Une mesure NON_QUALIFIANT est informative, jamais une preuve.
    # Comparer seulement les deux bras QUALIFIE_OK du meme tour.
    paires_qualifiees = [t for t in paires
                          if par_tour[t]["on"]["verdict"] == "QUALIFIE_OK"
                          and par_tour[t]["off"]["verdict"] == "QUALIFIE_OK"]

    sortie.append(f"  A/B reveil precis, {nom} SMP{smp}, minuteur {minuteur} : {len(paires)}/{tours} tours complets")
    sortie.append(f"  tours qualifies : {len(paires_qualifiees)}/{tours} "
                  f"(tours mesures {len(paires)}/{tours}, quorum {QUORUM})")
    sortie.append(f"  {'tour':<5}{'bras':<5}{'verdict':<16}{'repos p50':>10}{'p95':>7}{'p99':>8}{'pire':>8}{'inter p95':>10}{'armes':>8}")
    for b in bras:
        r = b["q"].get("au repos", (0, 0, 0, 0))
        i = b["q"].get("sous charge (interactif)", (0, 0, 0, 0))
        sortie.append(f"  {b['tour']:<5}{b['v']:<5}{str(b['verdict']):<16}{r[0]:>10}{r[1]:>7}{r[2]:>8}{r[3]:>8}{i[1]:>10}{b.get('armes', 0):>8}")

    for b in bras:
        for d in b["diag"]:
            sortie.append(f"  diag tour {b['tour']} {b['v']} : {d}")
        for d in b.get("diag_charge", []):
            sortie.append(f"  diag charge tour {b['tour']} {b['v']} : {d}")
        if b["verdict"] != "QUALIFIE_OK" and b.get("raison"):
            sortie.append(f"  raison tour {b['tour']} {b['v']} : {b['verdict']} {b['raison']}")
        if b["verdict"] == "QUALIFIE_ECHEC":
            for d in b["detail"]:
                sortie.append(f"  preuve tour {b['tour']} {b['v']} : {d}")
            phases = [(n, b["q"][c]) for n, c in (("normal", "sous charge (normal)"),
                                                  ("interactif", "sous charge (interactif)")) if c in b["q"]]
            if phases:
                sortie.append(f"  charge tour {b['tour']} {b['v']} : " + " ; ".join(
                    f"{n} p50 {q[0]} p95 {q[1]} p99 {q[2]} pire {q[3]}" for n, q in phases))
        # Les compteurs du noyau entre le releve d'avant et celui d'apres la
        # sonde (l'avance maximale est un maximum cumule : sa valeur finale).
        if len(b["noyau"]) >= 2:
            a, z = b["noyau"][0], b["noyau"][-1]
            parts = [f"{n}={z[i]}" if n in MAXIMA else f"{n}=+{z[i] - a[i]}"
                     for i, n in enumerate(DIAG_NOMS)]
            if b.get("occupants"):
                parts += [f"occupant_noyau={b['occupants'][0]}", f"occupant_utilisateur={b['occupants'][1]}"]
            sortie.append(f"  noyau tour {b['tour']} {b['v']} : " + " ".join(parts))

    def med(v, phase, k):
        return statistics.median([par_tour[t][v]["q"][phase][k] for t in paires_qualifiees]) if paires_qualifiees else 0
    criteres = []
    if paires_qualifiees:
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
    # Imprimes avant le verdict historique, qui reste le dernier mot.
    sortie.extend(cinq_proprietes(bras, tours, smp, nom, grain)[0])
    if fautes:
        sortie.extend(f"  DEFAUT {f}" for f in fautes)
        sortie.append(f"AB_REVEIL_PRECIS_ECHEC n={len(fautes)} accel={nom} smp={smp}")
        return sortie, "DEFAUT", 1
    if len(paires_qualifiees) < QUORUM:
        sortie.append(f"AB_REVEIL_PRECIS : NON_CONCLUANT accel={nom} smp={smp} "
                      f"apparies_qualifies={len(paires_qualifiees)}/{tours} "
                      f"tours_mesures={len(paires)}/{tours} (quorum {QUORUM})")
        return sortie, "NON_CONCLUANT", 0
    verdict = "CONVAINCANT" if all(ok for _, ok in criteres) else "NON_CONVAINCANT"
    sortie.append(f"AB_REVEIL_PRECIS : {verdict} accel={nom} smp={smp} tours={len(paires)}/{tours} "
                  f"(politique par defaut : on depuis b6c99032 ; un KVM NON_CONVAINCANT la rend a off)")
    return sortie, verdict, 0



def qualifie_bras(b):
    """D-6.1/6.2: judge applicable properties without rewriting historical evidence."""
    p, raisons = b["p"], []
    for phase in PHASES:
        raison = _phase_valide(p, phase)
        if raison:
            raisons.append(raison)
        if phase not in p["attribution"]:
            raisons.append(f"{phase}: attribution absente")
        if p["reveils_attribues"].get(phase, 0) <= 0:
            raisons.append(f"{phase}: aucun reveil effectivement attribue")
        n, total, regle = p["effectifs"].get(phase, (0, 0, "?"))
        if not (0 < n <= total) or (total - n) * 20 >= total:
            raisons.append(f"{phase}: effectifs/contamination invalides {n}/{total}")
        if regle not in ("preuve-hote", "sans-preuve"):
            raisons.append(f"{phase}: regle d'exclusion inconnue")
        q = p["q"].get(phase)
        if q is not None and not (0 <= q[0] <= q[1] <= q[2] <= q[3]):
            raisons.append(f"{phase}: quantiles incoherents")
    if b["verdict"] not in (OK, NQ, ECHEC):
        raisons.append("verdict historique absent ou inconnu")
    if len(p["tours"]) != 2:
        raisons.append("effectifs de calcul incomplets")
    if raisons:
        return NQ, NQ, raisons

    repos, normal, inter = (p["q"][ph] for ph in PHASES)
    bruit = max(0, repos[1] - repos[0])
    effet = normal[1] > repos[1] + bruit
    p3 = (OK if inter[2] < normal[2] or inter[3] < normal[3] else ECHEC) if effet else NQ
    p4 = inter[3] <= normal[3] + repos[3]
    normale, priorite = p["tours"]
    p5 = priorite > 0 and (normale == 0 or priorite * 10 >= normale)
    if p3 == ECHEC:
        raisons.append("P3: priorite sans benefice au-dessus du bruit")
    if not p4:
        raisons.append("P4: pire cas aggrave")
    if not p5:
        raisons.append("P5: famine du calcul")
    # An unexplained historical failure is never silently discarded. The
    # sole reinterpretable failure is an explicitly attributed priority
    # comparison below the *measured* noise floor, with P4/P5 still checked.
    if b["verdict"] == ECHEC:
        details = b["detail"]
        seulement_p3 = details and all(
            d.startswith("la priorite interactive raccourcit la queue de latence :") for d in details)
        if effet or not seulement_p3:
            raisons.append("echec historique non explique par la seule porte P3")
    if b["verdict"] == NQ:
        # D-6.1: only the negative control may lack strict qualification.
        # An unexplained NQ (clock, attribution, disagreement) is not noise.
        if b["v"] == "on" or effet or "charge sans effet mesurable :" not in b.get("raison", ""):
            return NQ, p3, raisons + ["non-qualification historique non admissible pour ce bras"]
    return (ECHEC if raisons else OK), p3, raisons


def analyse(texte, tours, smp, nom, politique="decisions-20261024"):
    bras = []
    historique, ancien, code = analyse_historique(texte, tours, smp, nom, bras)
    if politique == "historique":
        return historique, ancien, code
    if politique != "decisions-20261024":
        raise ValueError("politique A/B inconnue")
    # Preserve the old verdict alongside the new gate, unmodified in meaning.
    sortie = [s.replace("AB_REVEIL_PRECIS", "AB_REVEIL_PRECIS_HISTORIQUE") for s in historique]
    sortie.append("AB_POLITIQUE : decisions-20261024 bruit_repos=max(0,p95-p50) unites=us")
    structure = [s for s in historique if s.startswith("  DEFAUT ")
                 and not s.endswith("QUALIFIE_ECHEC sous le reveil precis")]
    if any(b["tour"] not in range(1, tours + 1) for b in bras):
        structure.append("  DEFAUT numero de tour hors protocole")
    if structure:
        sortie.extend(structure)
        sortie.append(f"AB_REVEIL_PRECIS_ECHEC n={len(structure)} accel={nom} smp={smp}")
        return sortie, "DEFAUT", 1
    valides, inconnus, echecs = [], [], []
    for b in bras:
        v, p3, raisons = qualifie_bras(b)
        identite = f"tour={b['tour']} bras={b['v']}"
        role = "politique-active" if b["v"] == "on" else "controle-negatif"
        sortie.append(f"AB_BRAS_POLITIQUE {identite} role={role} verdict={v} P3={p3} historique={b['verdict']}")
        for phase in PHASES:
            n, total, regle = b["p"]["effectifs"].get(phase, (0, 0, "?"))
            sortie.append(f"  preuve {identite} phase={phase} quantiles_us={b['p']['q'].get(phase)} "
                          f"propres={n} total={total} exclus={total-n} regle={regle} "
                          f"attribues={b['p']['reveils_attribues'].get(phase, 0)}")
        sortie.extend(f"  {identite}: {r}" for r in raisons)
        if p3 == NQ and v == OK:
            sortie.append(f"  {identite}: P3 NON_QUALIFIANT charge dans le bruit mesure (non bloquant, jamais OK)")
        if v == ECHEC:
            echecs.append(identite)
        elif v == NQ:
            inconnus.append(identite)
        else:
            valides.append(b)
    proprietes, verdicts = cinq_proprietes(valides, tours, smp, nom, 0, politique=True)
    sortie.extend(s.replace("AB_PROPRIETES_V1", "AB_PROPRIETES_POLITIQUE") for s in proprietes)
    # P13_AUDIT_COUVERTURE_V1 : P1/P2/P4/P5 peuvent etre OK sur le
    # sous-ensemble admissible, mais une campagne 7/8 n'est pas 8/8.
    # Ce diagnostic explicite n'influence aucun critere ni code de sortie.
    if len(valides) != 2 * tours:
        sortie.append(
            f"AB_COUVERTURE_NON_EXHAUSTIVE bras_valides={len(valides)}/{2*tours} "
            "proprietes_calculees_sur_sous_ensemble=1 campagne_non_qualifiee=1"
        )
    if echecs or any(verdicts[k] == ECHEC for k in ("P1", "P2", "P4", "P5")):
        verdict, code = "NON_CONVAINCANT", 1
    elif (inconnus or len(bras) != 2 * tours or tours < QUORUM
          or any(verdicts[k] != OK for k in ("P1", "P2", "P4", "P5"))):
        verdict, code = "NON_CONCLUANT", 0
    else:
        verdict, code = "CONVAINCANT", 0
    sortie.append(f"AB_REVEIL_PRECIS : {verdict} accel={nom} smp={smp} politique=decisions-20261024 "
                  f"bras_valides={len(valides)}/{2*tours}")
    return sortie, verdict, code

def main():
    journal, tours, smp, nom = sys.argv[1], int(sys.argv[2]), sys.argv[3], sys.argv[4]
    with open(journal, encoding="utf-8", errors="replace") as f:
        lignes, _, code = analyse(f.read(), tours, smp, nom)
    print("\n".join(lignes))
    return code


if __name__ == "__main__":
    sys.exit(main())
