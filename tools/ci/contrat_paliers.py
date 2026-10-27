#!/usr/bin/env python3
"""Contrat des paliers du workflow ladybird-native-browser (BOUCHAUD_CI_PALIERS_V2).

SOURCE UNIQUE. `verdict_convergence.py` juge un run avec ce contrat ;
`tools/verifie-ci-paliers.py` verifie que le workflow le cable a l'identique
(variables du job de verdict, conditions des jobs) et SIMULE chaque palier
avec la semantique d'ordonnancement de GitHub (`simule_paliers.py`).

Exigences :
  requis    le job doit avoir conclu `success` ; `failure`, `cancelled`,
            `skipped` ou une conclusion absente sont bloquants ;
  hors-run  le job n'est PAS joue dans ce palier : il doit etre `skipped`.
            Toute autre conclusion est bloquante -- une execution imprevue
            signale un cablage qui ne correspond plus au contrat.

Paliers :
  qualification  workflow_dispatch (defaut), branche d'audit. Build canonique
                 dans le run, primitives P13 (os-primitives), les 13 epreuves
                 Ladybird. Seul palier qui imprime CONVERGENCE_OK.
  developpement  push / pull_request / dispatch palier=developpement. Les
                 MEMES 13 epreuves Ladybird, memes scripts, memes seuils, sur
                 un artefact construit OU reutilise (entrees identiques,
                 reverifie). Les primitives P13 sont hors-run ici : elles sont
                 jouees par p13-primitives-noyau.yml sur les chemins noyau.
                 Imprime INTEGRATION_OK, jamais CONVERGENCE_OK.

Il n'existe PAS de palier « provenance seule » : un run qui ne jouerait que la
provenance n'a aucun verdict Ladybird a rendre.
"""
from __future__ import annotations

from typing import NamedTuple

QUALIFICATION = "qualification"
DEVELOPPEMENT = "developpement"
PALIERS = (QUALIFICATION, DEVELOPPEMENT)
REQUIS = "requis"
HORS_RUN = "hors-run"

NOM_VERDICT = {
    QUALIFICATION: "ladybird / verdict de convergence",
    DEVELOPPEMENT: "ladybird / verdict integration developpement",
}


class Epreuve(NamedTuple):
    cle: str          # variable d'environnement du job de verdict
    job: str          # identifiant du job dans le workflow
    nom: str
    exigence: dict    # palier -> REQUIS | HORS_RUN


def _partout(cle, job, nom):
    return Epreuve(cle, job, nom, {QUALIFICATION: REQUIS, DEVELOPPEMENT: REQUIS})


EPREUVES = (
    _partout("R_PROVENANCE", "provenance", "provenance de l'artefact (garde d'empreinte, decision)"),
    _partout("R_ARTEFACT", "artefact", "artefact disponible : un producteur et un seul"),
    Epreuve("R_PRIMITIVES", "primitives", "os-primitives : TCG/KVM, horloges, stockage, services et sondes QEMU",
            {QUALIFICATION: REQUIS, DEVELOPPEMENT: HORS_RUN}),
    _partout("R_PERFORMANCE", "performance", "budgets de performance"),
    _partout("R_MEMOIRE", "memoire-onglets", "memoire apres process swaps : deux repetitions, 20/20 swaps chacune"),
    _partout("R_ORDRE_WORKER", "ordre-worker", "ordre worker sur deux demarrages froids"),
    _partout("R_CRASH_SERVICES", "crash-services", "politique de crash des services"),
    _partout("R_COMPOSITOR_PIEGE", "compositor-piege", "piege Compositor : huit demarrages froids"),
    _partout("R_ENDURANCE_LONGUE", "endurance-longue", "endurance 20 min sous KVM"),
    _partout("R_SMOKE", "browser-host-smoke",
             "smoke BrowserHost (canvas, codecs, JS, workers x10, molette, audio, isolation, lien Compositor)"),
    _partout("R_CACHE", "cache", "cache HTTP et SQLite apres redemarrage du navigateur"),
    _partout("R_WPT", "wpt", "WPT smoke (50 fichiers, compares a Linux)"),
    _partout("R_ENDURANCE", "endurance",
             "endurance 10 min sous TCG : stabilite (cadres, workers, onglets cross-site, aucun crash)"),
    # BOUCHAUD_PORTE_PERF_SOUS_KVM_V1 : la cadence est jugee sous KVM (docs/ENDURANCE.md).
    _partout("R_ENDURANCE_KVM", "endurance-kvm",
             "endurance 5 min sous KVM : stabilite des deux bras A/B et performance du bras par defaut"),
    _partout("R_SITES", "sites", "sites reels HTTPS (Example exige, Wikipedia mesure), pixels du Compositor"),
    _partout("R_CRASH_RENDU", "crash-rendu",
             "robustesse : crash d'un WebContent (reprise), isolation des cadres, cycle de vie des workers"),
)

# Producteurs de l'artefact : exactement un, celui que la decision annonce.
PRODUCTEURS = {"construit": ("R_BUILD", "build"), "reutilise": ("R_REUTILISE", "reutilise")}
MODES_PERMIS = {QUALIFICATION: ("construit",), DEVELOPPEMENT: ("construit", "reutilise")}


def juge(env) -> tuple[list[str], list[str]]:
    """(lignes, causes) d'un run d'apres les conclusions de ses jobs."""
    lignes, causes = [], []
    palier = env.get("BOUCHAUD_PALIER") or "absent"
    mode = env.get("PROVENANCE_MODE") or "absent"
    lignes.append(f"BOUCHAUD_PALIER={palier} provenance={mode}")
    if palier not in PALIERS:
        return lignes, [f"palier={palier}"]
    if mode not in PRODUCTEURS:
        causes.append(f"provenance={mode}")
    elif mode not in MODES_PERMIS[palier]:
        causes.append(f"provenance={mode} interdite en {palier}")
    for m, (cle, job) in PRODUCTEURS.items():
        resultat = env.get(cle) or "absent"
        attendu = "success" if m == mode else "skipped"
        ok = resultat == attendu
        lignes.append(f"  {'ok     ' if ok else 'ECHEC  '} producteur {job} : {resultat} (attendu {attendu})")
        if not ok:
            causes.append(f"{job}={resultat}")
    for e in EPREUVES:
        exigence = e.exigence[palier]
        resultat = env.get(e.cle) or "absent"
        attendu = "success" if exigence == REQUIS else "skipped"
        ok = resultat == attendu
        etiquette = ("ok     " if ok else "ECHEC  ") if exigence == REQUIS else ("hors-run" if ok else "ECHEC  ")
        lignes.append(f"  {etiquette} {e.nom} : {resultat}")
        if not ok:
            causes.append(f"{e.job}={resultat}" + ("" if exigence == REQUIS else " (hors-run attendu)"))
    return lignes, causes
