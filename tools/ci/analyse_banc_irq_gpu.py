#!/usr/bin/env python3
"""Analyse de l'A/B scrutation / interruption de virtio-gpu
(BOUCHAUD_VIRTIO_GPU_IRQ_V1, P10+.1).

    analyse_banc_irq_gpu.py JOURNAL_SERIE TOURS ACCEL

Un seul demarrage, TOURS tours ; chaque tour presente la meme suite de trames
(`banc-present tache`, contexte du bureau) sous les deux politiques
d'attente, dans un ordre alterne (AB aux tours impairs, BA aux pairs).

CRITERES DECLARES AVANT LA MESURE (la politique par defaut est passee a
l'interruption sur l'A/B KVM CONVAINCANT de f23a1247, run 38697040371 ; un A/B
KVM NON_CONVAINCANT la rend a la scrutation) :

  surete     chaque bras de chaque tour : 640 presentations acceptees, aucun
             echec, aucune attente echue, scanout actif ; aucune panique ;
             bras `irq` : attente=irq et ZERO cession ; bras `scrute` :
             attente=scrute et zero sommeil ;
  processeur categorie plein : mediane (sur les tours) de la part du coeur
             restee inactive, irq >= scrute + 100 pour mille ;
  latence    par categorie : mediane des p50 irq <= 1,15 x scrute, mediane
             des p95 irq <= 1,30 x scrute ;
  relecture  achevements vus par la relecture de secours (interruption
             perdue) <= 1 % des presentations du bras irq ;
  quorum     au moins 4 tours complets, sinon NON_CONCLUANT.

Sortie 1 sur un defaut de surete ou d'instrument (ligne absente, armement
MSI-X absent, echec, panique) ; 0 sinon -- CONVAINCANT, NON_CONVAINCANT et
NON_CONCLUANT sont des mesures. Teste : tools/ci/reliability/test_banc_irq_gpu.py.
"""
import re
import statistics
import sys

CATEGORIES = {"plein": 40, "bande": 200, "tuile": 400}
QUORUM = 4
LIGNE = re.compile(
    r"\[BANC-PRESENT\] ecran=virtio-gpu-2d categorie=(\w+) rect=\S+ n=(\d+) moyen_us=(\d+) p50_us=(\d+) "
    r"p95_us=(\d+) max_us=(\d+) mpix_s=\d+ contexte=(\w+) attente=(\w+) cessions=(\d+) sommeils=(\d+) irq=(\d+) "
    r"irq_sans_progres=(\d+) vus_par_relecture=(\d+) echues=(\d+) coeur=(\d+) inactif_pour_mille=(\d+)")
FIN = re.compile(r"\[BANC-PRESENT\] ecran=virtio-gpu-2d fin presentations_virtio_ok=(\d+) echecs_virtio=(\d+) "
                 r"actif=(\d) contexte=(\w+) attente=(\w+)")
BRAS = re.compile(r"^GPU_AB_BRAS tour=(\d+) attente=(\w+)$")


def analyse(texte, tours, nom):
    """Rend (lignes imprimees, verdict, code)."""
    sortie, fautes = [], []
    texte = re.sub(r"\x1b\[[0-9;]*m", "", texte)
    lignes = [re.sub(r"^(\[[^]]*\]){2}\[FPS:[^]]*\] ", "", l).rstrip("\r") for l in texte.splitlines()]
    if re.search(r"KERNEL PANIC|panicked at", texte):
        fautes.append("panique pendant le banc")
    if not any(l.startswith("VIRTIO_GPU_IRQ msix ") for l in lignes):
        fautes.append("MSI-X non arme : l'A/B n'a pas de bras interruption")
    bras, courant = [], None
    for l in lignes:
        m = BRAS.match(l)
        if m:
            courant = {"tour": int(m.group(1)), "voulu": m.group(2), "cat": {}, "fin": None}
            bras.append(courant)
            continue
        if courant is None:
            continue
        m = LIGNE.search(l)
        if m:
            g = m.groups()
            courant["cat"][g[0]] = {
                "n": int(g[1]), "p50": int(g[3]), "p95": int(g[4]), "contexte": g[6], "attente": g[7],
                "cessions": int(g[8]), "sommeils": int(g[9]), "relecture": int(g[12]), "echues": int(g[13]),
                "inactif": int(g[15]),
            }
            continue
        m = FIN.search(l)
        if m:
            courant["fin"] = {"ok": int(m.group(1)), "echecs": int(m.group(2)), "actif": int(m.group(3))}

    complets = {}
    for b in bras:
        t, v = b["tour"], b["voulu"]
        nomb = f"tour {t} bras {v}"
        if b["fin"] is None or set(b["cat"]) != set(CATEGORIES):
            fautes.append(f"{nomb} : mesure incomplete")
            continue
        if b["fin"]["ok"] != sum(CATEGORIES.values()) or b["fin"]["echecs"] or not b["fin"]["actif"]:
            fautes.append(f"{nomb} : {b['fin']['ok']} acceptees, {b['fin']['echecs']} echecs, actif={b['fin']['actif']}")
        for c, d in b["cat"].items():
            if d["contexte"] != "tache" or d["attente"] != v:
                fautes.append(f"{nomb} {c} : contexte={d['contexte']} attente={d['attente']} (attendu tache/{v})")
            if d["echues"]:
                fautes.append(f"{nomb} {c} : {d['echues']} attente(s) echue(s)")
            if v == "irq" and d["cessions"]:
                fautes.append(f"{nomb} {c} : {d['cessions']} cession(s) sous attente par interruption")
            if v == "scrute" and d["sommeils"]:
                fautes.append(f"{nomb} {c} : {d['sommeils']} sommeil(s) sous scrutation")
        complets.setdefault(t, {})[v] = b
    paires = sorted(t for t, d in complets.items() if set(d) == {"irq", "scrute"})
    attendus = list(range(1, tours + 1))
    if [b["tour"] for b in bras[::2]] != attendus[:len(bras[::2])]:
        fautes.append("tours hors d'ordre")
    for t in attendus:
        ordre = [b["voulu"] for b in bras if b["tour"] == t]
        voulu = ["scrute", "irq"] if t % 2 else ["irq", "scrute"]
        if ordre and ordre != voulu:
            fautes.append(f"tour {t} : ordre {ordre}, attendu {voulu} (AB/BA)")

    sortie.append(f"  A/B attente virtio-gpu ({nom}) : {len(paires)}/{tours} tours complets, quorum {QUORUM}")
    sortie.append(f"  {'categorie':<8} {'p50 scrute':>10} {'p50 irq':>8} {'p95 scrute':>10} {'p95 irq':>8} "
                  f"{'inactif scrute':>14} {'inactif irq':>11} {'cessions scrute':>15}")
    criteres = []
    med = {}
    for c in CATEGORIES:
        def m(v, cle):
            vals = [complets[t][v]["cat"][c][cle] for t in paires]
            return statistics.median(vals) if vals else 0
        med[c] = {k: (m("scrute", k), m("irq", k)) for k in ("p50", "p95", "inactif", "cessions")}
        s = med[c]
        sortie.append(f"  {c:<8} {s['p50'][0]:>10} {s['p50'][1]:>8} {s['p95'][0]:>10} {s['p95'][1]:>8} "
                      f"{s['inactif'][0]:>14} {s['inactif'][1]:>11} {s['cessions'][0]:>15}")
        criteres.append((f"latence {c} : p50 irq {s['p50'][1]} <= 1,15 x scrute {s['p50'][0]}",
                         s["p50"][1] <= 1.15 * s["p50"][0]))
        criteres.append((f"latence {c} : p95 irq {s['p95'][1]} <= 1,30 x scrute {s['p95'][0]}",
                         s["p95"][1] <= 1.30 * s["p95"][0]))
    criteres.append((f"processeur plein : inactif irq {med['plein']['inactif'][1]} >= scrute "
                     f"{med['plein']['inactif'][0]} + 100 pour mille",
                     med["plein"]["inactif"][1] >= med["plein"]["inactif"][0] + 100))
    relecture = sum(complets[t]["irq"]["cat"][c]["relecture"] for t in paires for c in CATEGORIES)
    presentations = len(paires) * sum(CATEGORIES.values())
    criteres.append((f"relecture : {relecture} achevement(s) vus sans interruption sur {presentations} presentations "
                     f"(<= 1 %)", relecture * 100 <= max(presentations, 1)))
    for texte_c, ok in criteres:
        sortie.append(f"  {'ok     ' if ok else 'ECHOUE '} {texte_c}")

    if fautes:
        sortie.extend(f"  DEFAUT {f}" for f in fautes)
        return sortie, "DEFAUT", 1
    if len(paires) < QUORUM:
        return sortie + [f"GPU_IRQ_AB NON_CONCLUANT accel={nom} tours={len(paires)}/{tours}"], "NON_CONCLUANT", 0
    verdict = "CONVAINCANT" if all(ok for _, ok in criteres) else "NON_CONVAINCANT"
    sortie.append(f"GPU_IRQ_AB {verdict} accel={nom} tours={len(paires)}/{tours} (politique par defaut : interruption depuis f23a1247 ; un KVM NON_CONVAINCANT la rend a la scrutation)")
    return sortie, verdict, 0


def main():
    journal, tours, nom = sys.argv[1], int(sys.argv[2]), sys.argv[3]
    with open(journal, encoding="utf-8", errors="replace") as f:
        lignes, _, code = analyse(f.read(), tours, nom)
    print("\n".join(lignes))
    return code


if __name__ == "__main__":
    sys.exit(main())
