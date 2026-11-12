#!/usr/bin/env python3
"""BOUCHAUD_LIVRAISON_V1 -- l'artefact Ladybird natif d'une RELEASE, en lecture seule.

    DEPOT=owner/repo GH_TOKEN=... artefact_ladybird_release.py

Cherche, parmi les runs SURS du workflow ladybird-native-browser (push ou
workflow_dispatch de ce depot, jamais pull_request), un artefact
`bouchaud-ladybird-native-browser` de MEME empreinte que HEAD -- la regle de
tools/ci/ladybird_provenance.py, reutilisee telle quelle. Ne declenche AUCUNE
production : une release ne lance pas de build de navigateur.

Sortie : `source_run=<id>` dans $GITHUB_OUTPUT et code 0 ; sinon code 2 et
`ARTEFACT_LADYBIRD_ABSENT` avec la commande a lancer -- la release est
refusee plutot que publiee sans navigateur.
"""
import os
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "ci"))
import ladybird_empreinte as le  # noqa: E402
import ladybird_provenance as lp  # noqa: E402


def main() -> int:
    depot = os.environ.get("DEPOT", "")
    if not depot:
        print("ARTEFACT_LADYBIRD_REFUSE DEPOT absent", file=sys.stderr)
        return 2
    courant = le.empreinte("HEAD")
    api = lp.gh_api(depot)

    def empreinte_de(sha):
        le.assure_commit(sha)
        return le.empreinte(sha)["empreinte"]

    rid, raison = lp.cherche_artefact(0, courant["empreinte"], lp.liste_artefacts(api), api,
                                      lp.Empreintes(empreinte_de), depot)
    if rid is None:
        print(f"ARTEFACT_LADYBIRD_ABSENT empreinte={courant['empreinte'][:16]} ({raison}). "
              "Produire l'artefact : Actions > ladybird-native-browser > Run workflow (palier=artefact, "
              f"empreinte={courant['empreinte']}), puis relancer la release.", file=sys.stderr)
        return 2
    print(f"ARTEFACT_LADYBIRD_OK source_run={rid} empreinte={courant['empreinte'][:16]} ({raison})")
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as f:
            f.write(f"source_run={rid}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
