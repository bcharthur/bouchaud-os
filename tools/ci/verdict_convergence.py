#!/usr/bin/env python3
"""Verdict de la campagne de convergence Ladybird (BOUCHAUD_CONVERGENCE_VERDICT_V1).

Lance par le job `convergence` du workflow ladybird-native-browser, APRES les
jobs qui jouent Ladybird sur Bouchaud. Chacun de ces jobs construit le noyau
depuis le MEME checkout et verifie que l'artefact Ladybird a ete produit par
le MEME HEAD (`artifact_manifest.py verify` : `artefact d'un autre HEAD`
sinon). Ce script ne rejoue rien : il rassemble les resultats et refuse le
faux vert.

    BOUCHAUD_TEST_HEAD=<sha>
    BOUCHAUD_LADYBIRD_CONVERGENCE_OK            si toutes les epreuves passent
    BOUCHAUD_LADYBIRD_CONVERGENCE_FAIL causes=  sinon, avec la liste

BOUCHAUD_CI_PALIERS_V1 : le verdict depend du PALIER du run (BOUCHAUD_PALIER).

  qualification   toutes les epreuves, primitives P13 comprises, artefact
                  CONSTRUIT dans ce run et lie a son HEAD. Seul ce palier
                  imprime BOUCHAUD_LADYBIRD_CONVERGENCE_OK.
  developpement   memes epreuves Ladybird, memes seuils ; les primitives P13
                  sont jouees par leur propre workflow (p13-primitives-noyau),
                  pas ici, et l'artefact peut etre REUTILISE d'un run aux
                  entrees de build identiques. Imprime
                  BOUCHAUD_LADYBIRD_INTEGRATION_OK -- jamais CONVERGENCE_OK :
                  un vert de developpement ne peut pas passer pour une
                  qualification (le job porte d'ailleurs un autre nom).

Palier absent ou inconnu, provenance incoherente (zero ou deux artefacts,
build saute en qualification, primitives sautees en qualification) : rouge.
"""
import os
import subprocess
import sys

EPREUVES = [
    ("R_PRIMITIVES", "os-primitives : TCG/KVM, horloges, stockage, services et sondes QEMU"),
    ("R_PERFORMANCE", "budgets de performance"),
    ("R_MEMOIRE", "memoire apres process swaps : deux repetitions, 20/20 swaps chacune"),
    ("R_ORDRE_WORKER", "ordre worker sur deux demarrages froids"),
    ("R_CRASH_SERVICES", "politique de crash des services"),
    ("R_COMPOSITOR_PIEGE", "piege Compositor : huit demarrages froids"),
    ("R_ENDURANCE_LONGUE", "endurance 20 min sous KVM"),
    ("R_BUILD", "build Ladybird (chaine canonique)"),
    ("R_SMOKE", "smoke BrowserHost (canvas, codecs, JS, workers x10, molette, audio, isolation, lien Compositor)"),
    ("R_CACHE", "cache HTTP et SQLite apres redemarrage du navigateur"),
    ("R_WPT", "WPT smoke (50 fichiers, compares a Linux)"),
    ("R_ENDURANCE", "endurance 10 min sous TCG : stabilite (cadres, workers, onglets cross-site, aucun crash)"),
    # BOUCHAUD_PORTE_PERF_SOUS_KVM_V1 : la cadence est jugee sous KVM (docs/ENDURANCE.md).
    ("R_ENDURANCE_KVM", "endurance 5 min sous KVM : stabilite des deux bras A/B et performance du bras par defaut"),
    ("R_SITES", "sites reels HTTPS (Example exige, Wikipedia mesure), pixels du Compositor"),
    ("R_CRASH_RENDU", "robustesse : crash d'un WebContent (reprise), isolation des cadres, cycle de vie des workers"),
]


# Provenance de l'artefact : exactement un producteur, et celui que la decision
# annonce. `R_BUILD` reste dans EPREUVES pour la qualification ; en
# developpement, il est juge ici avec `R_REUTILISE`.
PROVENANCES = {"construit": ("R_BUILD", "R_REUTILISE"), "reutilise": ("R_REUTILISE", "R_BUILD")}


def juge(env) -> tuple[list[str], list[str]]:
    """(lignes, causes). Pure : testable sans environnement."""
    lignes, causes = [], []
    palier = env.get("BOUCHAUD_PALIER", "absent")
    mode = env.get("PROVENANCE_MODE", "") or "absent"
    lignes.append(f"BOUCHAUD_PALIER={palier} provenance={mode}")
    if palier not in ("qualification", "developpement"):
        return lignes, [f"palier={palier}"]
    if mode not in PROVENANCES:
        causes.append(f"provenance={mode}")
    else:
        producteur, autre = PROVENANCES[mode]
        if env.get(producteur, "absent") != "success":
            causes.append(f"{producteur.lower()[2:]}={env.get(producteur, 'absent')}")
        if env.get(autre, "absent") != "skipped":
            causes.append(f"{autre.lower()[2:]}={env.get(autre, 'absent')} (deux producteurs)")
        lignes.append(f"  {'ok     ' if not causes else 'ECHEC  '} artefact Ladybird {mode} : "
                      f"{producteur.lower()[2:]}={env.get(producteur, 'absent')}")
    if palier == "qualification" and mode != "construit":
        causes.append("qualification sans build complet")
    for cle, nom in EPREUVES:
        if cle == "R_BUILD":
            continue  # juge avec la provenance ci-dessus
        resultat = env.get(cle, "absent")
        if cle == "R_PRIMITIVES" and palier == "developpement":
            # Jouees par p13-primitives-noyau.yml sur les chemins noyau. Ici :
            # sautees, ou reussies si on les a demandees ; jamais en echec.
            ok = resultat in ("skipped", "success")
            lignes.append(f"  {'hors-run ' if resultat == 'skipped' else ('ok     ' if ok else 'ECHEC  ')} {nom} : {resultat}")
            if not ok:
                causes.append(f"primitives={resultat}")
            continue
        lignes.append(f"  {'ok     ' if resultat == 'success' else 'ECHEC  '} {nom} : {resultat}")
        if resultat != "success":
            causes.append(f"{cle.lower()[2:]}={resultat}")
    return lignes, causes


def main() -> int:
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    print(f"BOUCHAUD_TEST_HEAD={head}")
    print("provenance : noyau construit dans chaque job depuis ce checkout ; artefact Ladybird du job build "
          "de ce run (liaison au HEAD) ou d'un run aux entrees identiques (liaison par empreinte), "
          "verifie par tools/ci/verifie_artefact_ladybird.sh dans chaque job")
    lignes, causes = juge(os.environ)
    for ligne in lignes:
        print(ligne)
    if causes:
        print(f"BOUCHAUD_LADYBIRD_CONVERGENCE_FAIL causes={','.join(causes)}")
        return 1
    if os.environ.get("BOUCHAUD_PALIER") == "qualification":
        print("BOUCHAUD_LADYBIRD_CONVERGENCE_OK")
    else:
        print(f"BOUCHAUD_LADYBIRD_INTEGRATION_OK palier=developpement artefact={os.environ.get('PROVENANCE_MODE')}")
        print("BOUCHAUD_LADYBIRD_CONVERGENCE_NON_QUALIFIANTE raison=palier developpement "
              "(primitives P13 hors run ; qualification = workflow_dispatch palier=qualification)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
