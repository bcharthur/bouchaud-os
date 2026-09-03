#!/usr/bin/env python3
"""Budgets versionnes : ce qui a ete gagne ne doit pas se reperdre.

# Pourquoi des budgets, et pas seulement des tests

Un test dit « cela marche ». Il ne dit pas « cela ne s'est pas degrade ». Or
les regressions qui comptent ici sont graduelles : un site d'acquisition du
gros verrou rajoute dans un sous-systeme qu'on venait d'alleger, une tenue
maximale qui remonte de dix millisecondes par semaine. Aucune n'echoue a un
test ; toutes annulent le travail, et on ne s'en apercoit qu'au moment ou la
machine rame de nouveau.

Un budget est une valeur de reference VERSIONNEE. Le depasser echoue la
barriere ; faire mieux est signale, et s'adopte explicitement.

# Les deux familles

  * ARCHITECTURE -- se calcule sur la SOURCE, donc partout et tout de suite :
    combien de sites prennent encore le gros verrou, et dans quel domaine.
    C'est la mesure directe du chantier « sortie du gros verrou ».

  * EXECUTION -- se lit dans un journal QEMU (`--journal`). Tenue maximale,
    attente maximale, regressions de domaine. Sans journal, ces budgets sont
    ANNONCES COMME NON VERIFIES et non silencieusement reussis : un budget
    qu'on croit vert alors qu'il n'a pas tourne est pire que pas de budget.
"""

import argparse
import json
import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent.parent
REFERENCE = Path(__file__).resolve().parent / "budgets" / "bkl-sites.json"
SRC = RACINE / "src"

ACQUISITION = re.compile(r"\bsmp_lock::(?:enter|try_enter|try_enter_depuis_zero)\(\)")
PORTEE = re.compile(r"\bportee\(\s*(?:crate::kernel::sync::)?Domaine::(\w+)")

EXEMPTS_PREFIXES = ("src/kernel/sync/bkl/",)
EXEMPTS = {"src/kernel/sync/bkl.rs", "src/kernel/sync/mod.rs"}


def sites_par_domaine() -> dict[str, int]:
    """Combien de sites prennent le gros verrou, et sous quel domaine."""
    compte: dict[str, int] = {}
    for chemin in sorted(SRC.rglob("*.rs")):
        relatif = chemin.relative_to(RACINE).as_posix()
        if relatif in EXEMPTS or relatif.startswith(EXEMPTS_PREFIXES):
            continue
        lignes = chemin.read_text(encoding="utf-8", errors="replace").split("\n")
        for numero, ligne in enumerate(lignes):
            if ligne.lstrip().startswith("//") or not ACQUISITION.search(ligne):
                continue
            domaine = "Indetermine"
            for precedente in reversed(lignes[max(0, numero - 3):numero]):
                trouve = PORTEE.search(precedente)
                if trouve:
                    domaine = trouve.group(1)
                    break
            compte[domaine] = compte.get(domaine, 0) + 1
    return compte


# Les grandeurs lues dans un journal d'execution, et leur sens.
EXECUTION = {
    "bkl_max_hold_ms": (
        re.compile(r"\[BKL-MAX-HOLD\]\s+ns=(\d+)"),
        lambda v: v / 1_000_000,
        "plus longue tenue du gros verrou",
    ),
    "bkl_attente_max_ms": (
        re.compile(r"\[BKL-COMPTES\].*?attente_max_ns=(\d+)"),
        lambda v: v / 1_000_000,
        "plus longue attente avant acquisition",
    ),
    "bkl_regressions_domaine": (
        re.compile(r"\[BKL-DOMAINES\]\s+normaux=\d+\s+regressions=(\d+)"),
        lambda v: v,
        "chemins declares sortis ayant repris le verrou",
    ),
    # --- ordonnanceur ------------------------------------------------------
    #
    # Ces deux-la mesurent ce que l'utilisateur RESSENT. Une preemption
    # differee trop longtemps, ou une tache prete qui attend son coeur, sont
    # exactement ce qui se voit comme une interface qui ne repond pas -- et
    # aucune des deux ne fait echouer un test.
    "preempt_max_defer_ms": (
        re.compile(r"\[SCHED-NG-PREEMPT\].*?max_defer_ns=(\d+)"),
        lambda v: v / 1_000_000,
        "plus long report SUBI d'une preemption -- du premier refus au service",
    ),
    "ready_latency_max_ms": (
        re.compile(r"\[SCHED-NG-LAT\].*?\bmax_ns=(\d+)"),
        lambda v: v / 1_000_000,
        "plus longue attente entre « prete » et « sur un coeur »",
    ),
    "ready_latency_interactive_max_ms": (
        re.compile(r"\[SCHED-NG-LAT\].*?interactive_max_ns=(\d+)"),
        lambda v: v / 1_000_000,
        "la meme, pour les taches interactives",
    ),
    # --- centiles de latence, chantier 2 ------------------------------------
    #
    # Un maximum peut venir d'un seul evenement de boot ; une moyenne noie les
    # quelques pour cent de reveils lents qui font qu'une interface accroche.
    # Le p99 interactif est le chiffre qui correspond a ce qui se voit : un
    # clic sur cent qui repond en retard.
    "ready_latency_interactive_p99_ms": (
        re.compile(r"\[SCHED-NG-CENTILES\] classe=interactive .*?p99_ns=(\d+)"),
        lambda v: v / 1_000_000,
        "p99 de l'attente entre « prete » et « sur un coeur », classe interactive",
    ),
    "ready_latency_normale_p99_ms": (
        re.compile(r"\[SCHED-NG-CENTILES\] classe=normale .*?p99_ns=(\d+)"),
        lambda v: v / 1_000_000,
        "la meme, classe normale -- borne la FAMINE du travail de fond",
    ),
    # --- runqueue du chantier 2 ---------------------------------------------
    #
    # `anti_famine` compte les tours rendus a la bande normale. Ce n'est pas une
    # faute : c'est la preuve que la borne existe. Ce qui serait une faute est
    # qu'elle n'existe pas, et cela ne se voit qu'en la mesurant.
    "runqueue_doublons_max": (
        re.compile(r"\[SCHED-NG-FILE\].*?doublons=(\d+)"),
        lambda v: v,
        "mises en file dedupliquees -- une tache publiee alors qu'elle y etait deja",
    ),
    # --- reseau, chantier 9 -------------------------------------------------
    #
    # Un TOUR vaut mille instructions `pause`, donc quelques microsecondes.
    # Le chiffre doit BAISSER : c'est la mesure de ce que le chantier 9 doit
    # retirer au coeur occupe a interroger l'anneau.
    "tcp_busy_poll_tours_max": (
        re.compile(r"\[NET-TCP\].*?busy_poll_tours=(\d+)"),
        lambda v: v,
        "tours d'attente active TCP -- le coeur occupe a interroger l'anneau",
    ),
    # --- BKL herite, chantier 1 ---------------------------------------------
    #
    # Le futex ne PREND plus le gros verrou ; il en HERITE encore un de son
    # appelant. Tant que ce chiffre n'est pas nul, le domaine reste
    # `EnMigration`, et le voir monter serait une regression de ses appelants.
    "futex_bkl_herites_max": (
        re.compile(r"\[BKL-FUTEX\].*?herites=(\d+)"),
        lambda v: v,
        "operations futex entrees avec un gros verrou herite de leur appelant",
    ),
    # --- ce qui doit rester A ZERO ------------------------------------------
    #
    # Ces trois-la ne sont pas des plafonds a resserrer : ce sont des fautes.
    # Un superbloc rejete hors coupure de courant veut dire qu'une ecriture a
    # ete dechiree ; une erreur de volume veut dire que le stockage ment ; une
    # relance refusee veut dire qu'un moteur de rendu plante en boucle.
    "persistance_superblocs_rejetes": (
        re.compile(r"\[PERSIST-COMMIT\].*?superblocs_rejetes=(\d+)"),
        lambda v: v,
        "superblocs rejetes au montage -- une ecriture de commit dechiree",
    ),
    "bloc_erreurs": (
        re.compile(r"\[BLOC-NG\].*?erreurs=(\d+)"),
        lambda v: v,
        "requetes bloc refusees ou echouees",
    ),
    "ladybird_relances_refusees": (
        re.compile(r"\[LADYBIRD-SUP\].*?relances_refusees=(\d+)"),
        lambda v: v,
        "relances de moteur de rendu refusees -- une boucle de plantage",
    ),
}


def mesures_execution(journal: Path) -> dict[str, float]:
    texte = journal.read_text(encoding="utf-8", errors="replace")
    mesures = {}
    for nom, (motif, converti, _) in EXECUTION.items():
        valeurs = [converti(float(m)) for m in motif.findall(texte)]
        if valeurs:
            # Le PIRE de la trace : un budget se juge sur le maximum, pas sur
            # une moyenne qui noierait justement le figement qu'on cherche.
            mesures[nom] = max(valeurs)
    return mesures


def main() -> int:
    parseur = argparse.ArgumentParser()
    parseur.add_argument("--journal", type=Path,
                         help="journal serie QEMU, pour les budgets d'execution")
    parseur.add_argument("--adopte", action="store_true",
                         help="ecrit les valeurs courantes comme nouvelle reference")
    options = parseur.parse_args()

    reference = json.loads(REFERENCE.read_text(encoding="utf-8"))
    courant = sites_par_domaine()

    if options.adopte:
        reference["architecture"]["sites_bkl_par_domaine"] = courant
        if options.journal and options.journal.exists():
            reference["execution"].update(mesures_execution(options.journal))
        REFERENCE.write_text(json.dumps(reference, indent=2, sort_keys=True) + "\n",
                             encoding="utf-8")
        print(f"reference adoptee : {REFERENCE.relative_to(RACINE)}")
        return 0

    fautes, gains, non_verifies = [], [], []

    # --- architecture -------------------------------------------------------
    attendu = {c: v for c, v in
               reference["architecture"]["sites_bkl_par_domaine"].items()
               if not c.startswith("_")}
    for domaine in sorted(set(attendu) | set(courant)):
        budget = attendu.get(domaine, 0)
        mesure = courant.get(domaine, 0)
        if mesure > budget:
            fautes.append(
                f"  sites BKL / {domaine} : {mesure} > {budget} (budget). "
                f"Un site rajoute dans un sous-systeme qu'on allege annule le "
                f"travail sans echouer a aucun test."
            )
        elif mesure < budget:
            gains.append(f"  sites BKL / {domaine} : {mesure} < {budget}")

    # --- execution ----------------------------------------------------------
    #
    # Une grandeur REQUISE absente est une FAUTE, pas une non-verification.
    # C'est la difference entre « le scenario n'a pas produit ce chiffre » et
    # « le scenario n'emet aucun releve, donc cette barriere ne peut pas
    # rougir ». Le second cas s'etait installe sans bruit : la trace ne portait
    # aucun releve periodique, les treize grandeurs sortaient « absent du
    # journal », et la barriere annoncait quand meme « budgets tenus ».
    #
    # Ne sont PAS requises les grandeurs qui dependent legitimement du
    # scenario : un compteur de navigateur quand aucun navigateur ne tourne,
    # un volume bloc absent, un commit de persistance jamais declenche.
    requis = set(reference["execution"].get("_requis", []))
    budgets = [(nom, valeur) for nom, valeur in reference["execution"].items()
               if not nom.startswith("_")]
    mesures: dict[str, float] = {}

    if options.journal and options.journal.exists():
        mesures = mesures_execution(options.journal)
        for nom, budget in budgets:
            if nom not in mesures:
                if nom in requis:
                    fautes.append(
                        f"  {nom} : REQUIS mais absent de {options.journal}. "
                        f"Le releve periodique n'a pas ete emis par ce scenario "
                        f"(`smpstat`), donc ce budget n'a rien verifie. Une "
                        f"barriere qui ne peut pas rougir ne protege rien."
                    )
                else:
                    non_verifies.append(f"  {nom} : absent du journal")
                continue
            mesure = mesures[nom]
            libelle = EXECUTION[nom][2] if nom in EXECUTION else nom
            if mesure > budget:
                fautes.append(f"  {nom} ({libelle}) : {mesure:.3f} > {budget} (budget)")
            elif mesure < budget:
                gains.append(f"  {nom} : {mesure:.3f} < {budget}")
    else:
        non_verifies = [f"  {nom} : aucun journal fourni" for nom, _ in sorted(budgets)]

    if gains:
        print("budgets ameliores (adopter avec --adopte) :")
        print("\n".join(gains))
    if non_verifies:
        # Annonce, jamais silence : un budget qu'on croit vert alors qu'il n'a
        # pas tourne est pire que pas de budget du tout.
        print("budgets NON VERIFIES (pas de mesure disponible) :")
        print("\n".join(non_verifies))
    if fautes:
        print("budgets depasses :")
        print("\n".join(fautes))
        return 1

    # Le compte des mesures fait PARTIE du verdict. « budgets tenus » sans lui
    # se lit comme « tout a ete verifie » alors que la trace pouvait n'avoir
    # rien porte du tout -- exactement ce que cette barriere doit empecher.
    total = sum(courant.values())
    mesures_faites = sum(1 for nom, _ in budgets if nom in mesures)
    print(f"ok  budgets tenus ; {total} site(s) d'acquisition du gros verrou "
          f"dans {len(courant)} domaine(s) ; execution : "
          f"{mesures_faites}/{len(budgets)} grandeur(s) mesuree(s)"
          + (f", {len(budgets) - mesures_faites} absente(s)"
             if mesures_faites < len(budgets) else ""))
    return 0


if __name__ == "__main__":
    sys.exit(main())
