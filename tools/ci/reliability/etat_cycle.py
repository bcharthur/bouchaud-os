"""L'etat d'un cycle d'endurance : OK, NON_QUALIFIANT ou ECHEC.

BOUCHAUD_ENDURANCE_NON_QUALIFIANT_V1

# Pourquoi

Endurance hebdomadaire 38645418495, SMP8 sous TCG sur un runner de quatre
coeurs : la sonde d'ordonnanceur rend NON_QUALIFIANT (126 reveils sur 300
retardes sans explication, aucune preuve de l'hote) ; l'autorun finit avec le
statut 2 ; `soak.py` imprimait « cycle 1: ECHEC », `budgets_endurance.py`
« noyau : cycle 1 en echec », et la campagne de trente minutes s'arretait
apres 71 s. Deux fautes :

  * une IMPOSSIBILITE DE JUGER la latence etait presentee comme un echec du
    noyau -- ce qu'aucune mesure n'a montre ;
  * la campagne s'arretait la : sur SMP8, l'endurance ne mesurait plus rien
    de la stabilite (paniques, familles de charge, budgets) au-dela du
    premier cycle.

# La regle

Un cycle est NON_QUALIFIANT seulement si TOUT le reste du cycle est sain et
que la seule chose qui manque est le jugement de la sonde :

  * aucune faute fatale, QEMU sorti normalement, pas de delai depasse ;
  * les marqueurs absents sont EXACTEMENT le marqueur de la famille de la
    sonde et `=== AUTORUN FIN === statut=0` (marqueurs EMIS, jamais l'echo
    d'une commande : BOUCHAUD_MARQUEUR_HORS_ECHO_V1) ;
  * l'autorun a fini avec le statut 2 (code de sortie NON_QUALIFIANT de la
    sonde), et le journal porte une seule ligne `VERDICT_SONDE`, qui dit
    NON_QUALIFIANT.

Tout autre cycle rate est un ECHEC. Un cycle NON_QUALIFIANT n'est JAMAIS un
cycle reussi : la campagne continue pour mesurer la stabilite, et son verdict
final est NON_QUALIFIANT (code de sortie 2), jamais OK.
"""

from __future__ import annotations

import re

from logscan import est_echo_de_commande

FIN_AUTORUN = re.compile(r"=== AUTORUN FIN === statut=(\d+)")
VERDICT_SONDE = re.compile(r"VERDICT_SONDE : (\S+)")
FIN_REUSSIE = "=== AUTORUN FIN === statut=0"
# Le code de sortie d'une sonde qui n'a pas pu juger (ordonnanceur-probe).
STATUT_NON_QUALIFIANT = 2

OK = "OK"
NON_QUALIFIANT = "NON_QUALIFIANT"
ECHEC = "ECHEC"


def etat_du_cycle(resultat: dict, texte: str, marqueur_sonde: str | None) -> tuple[str, str]:
    """(etat, raison) d'un cycle `qemu_matrix.run_one`, d'apres son journal."""
    if resultat.get("ok"):
        return OK, ""
    if marqueur_sonde is None:
        return ECHEC, "aucune sonde designee pour un NON_QUALIFIANT"
    if resultat.get("fatal_findings"):
        return ECHEC, f"{len(resultat['fatal_findings'])} faute(s) fatale(s)"
    if resultat.get("timed_out"):
        return ECHEC, "cycle coupe au delai"
    if not resultat.get("qemu_exit_ok", False):
        return ECHEC, f"QEMU a quitte avec le code {resultat.get('returncode')}"
    absents = set(resultat.get("required_markers_missing", []))
    attendus = {marqueur_sonde, FIN_REUSSIE}
    if absents != attendus:
        autres = sorted(absents - attendus)
        if autres:
            return ECHEC, "marqueur(s) absent(s) hors de la sonde : " + ", ".join(autres)
        return ECHEC, "marqueurs incoherents : " + ", ".join(sorted(attendus - absents)) + " present(s)"
    emises = [l for l in texte.splitlines() if not est_echo_de_commande(l)]
    fins = [int(m.group(1)) for l in emises for m in [FIN_AUTORUN.search(l)] if m]
    if fins != [STATUT_NON_QUALIFIANT]:
        return ECHEC, f"fin d'autorun {fins} (attendu : une seule, statut {STATUT_NON_QUALIFIANT})"
    verdicts = [m.group(1) for l in emises for m in [VERDICT_SONDE.search(l)] if m]
    if verdicts != [NON_QUALIFIANT]:
        return ECHEC, f"verdicts de sonde {verdicts} (attendu : un seul, NON_QUALIFIANT)"
    raison = next((l.split("raison=", 1)[1].strip() for l in emises
                   if VERDICT_SONDE.search(l) and "raison=" in l), "raison non imprimee")
    return NON_QUALIFIANT, raison


def verdict_campagne(etats: list[str]) -> str:
    """OK si tous les cycles le sont ; ECHEC des qu'un l'est ; sinon NON_QUALIFIANT."""
    if not etats:
        return ECHEC
    if ECHEC in etats:
        return ECHEC
    if NON_QUALIFIANT in etats:
        return NON_QUALIFIANT
    return OK


def code_de_sortie(verdict: str) -> int:
    return {OK: 0, NON_QUALIFIANT: 2}.get(verdict, 1)
