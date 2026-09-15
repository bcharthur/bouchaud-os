#!/usr/bin/env python3
"""Garde-fou : deux chemins d'amorcage qui doivent demarrer les memes services.

# Le defaut, et pourquoi il ne se voit pas

Sur la machine de reference -- UEFI, `reference-bringup`, `reference-desktop` --
`kernel_main` appelle `platform::pc::stage2::run(boot_info)` et **cet appel ne
rend jamais la main**. Tout ce que `main.rs` fait APRES est donc du code mort
sur la TRIGKEY, et vivant partout ailleurs.

Ce n'est pas theorique. La configuration de la PAT vivait dans
`arch::x86_64::init()`, jamais atteint : le releve physique portait
`coeurs_pat=0 pat=0x0007040600070406` -- la valeur de sortie d'usine -- et
personne ne l'a lu pendant des semaines. La correction a consiste a la
deplacer vers le seul endroit que TOUS les chemins traversent.

`stage2.rs` reproduit donc, dans le meme ordre, la liste des services que
`main.rs` demarre. Deux listes tenues a la main finissent toujours par
diverger, et une divergence ici ne fait rougir aucun test : la machine
demarre, le bureau s'affiche, et un service manque en silence.

# Ce qui est verifie

Chaque service demarre par `main.rs` apres la branche `stage2::run` est
demarre par `stage2.rs` AUSSI -- ou figure dans la liste des ecarts
DELIBERES ci-dessous, avec sa raison. Ajouter un service a `main.rs` sans
l'ajouter a `stage2.rs` rend donc ce garde-fou rouge, et le rendre vert
oblige a ecrire pourquoi.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
MAIN = RACINE / "src/main.rs"
STAGE2 = RACINE / "src/platform/pc/stage2.rs"

# Les appels qui DEMARRENT quelque chose de durable : un fil, un service, un
# sous-systeme qui continue de tourner. Un `log` ou un `banner` n'en est pas.
SERVICES = re.compile(
    r"\b((?:crate::)?(?:kernel|drivers|net|platform|arch|fs)::[A-Za-z0-9_:]*"
    r"(?:demarre|demarre_le_fil_[a-z0-9_]+|enable_scheduler|lance_le_montage_differe)"
    r"[A-Za-z0-9_]*)\s*\("
)

# Ecarts DELIBERES. Chaque entree porte la raison pour laquelle le chemin
# TRIGKEY ne demarre pas ce service -- et cette raison doit etre verifiable
# dans le code, pas seulement plausible.
ECARTS_ASSUMES = {
    "kernel::prechauffage::demarre": (
        "Sur ce chemin, le navigateur demarre tout seul cinq cents "
        "millisecondes apres la premiere trame du bureau (window_manager, "
        "`services_initialises`). Son vrai jeu de travail remplit le cache de "
        "pages propres ; un prechauffage speculatif en plus lui ferait "
        "CONCURRENCE pour les memes seize mille pages. La note en tete de la "
        "section correspondante de stage2.rs porte ce raisonnement."
    ),
}


def appels(source):
    return {m.group(1).removeprefix("crate::") for m in SERVICES.finditer(source)}


def main():
    for chemin in (MAIN, STAGE2):
        if not chemin.exists():
            print("  - fichier absent : %s" % chemin)
            return 1

    main_src = MAIN.read_text(encoding="utf-8")
    stage2_src = STAGE2.read_text(encoding="utf-8")

    branche = main_src.find("platform::pc::stage2::run(")
    if branche < 0:
        print(
            "  - main.rs : l'appel a `stage2::run` a disparu. Ce garde-fou "
            "repose sur le fait que ce chemin ne rend jamais la main ; si la "
            "structure de l'amorcage a change, c'est ce garde-fou qu'il faut "
            "revoir, pas le contourner."
        )
        return 1

    apres_branche = main_src[branche:]
    attendus = appels(apres_branche)
    presents = appels(stage2_src)

    fautes = []
    for service in sorted(attendus - presents):
        raison = ECARTS_ASSUMES.get(service)
        if raison is None:
            fautes.append(
                "`%s` est demarre par main.rs APRES la branche stage2, et "
                "stage2.rs ne le demarre pas.\n\n    Sur la machine de "
                "reference, `stage2::run` ne rend jamais la main : ce service "
                "n'existe donc que sur les autres chemins. Rien ne rougira, "
                "le bureau s'affichera, et il manquera en silence -- "
                "exactement ce qui est arrive a la configuration de la PAT.\n\n"
                "    Deux issues, et aucune n'est de supprimer ce test : "
                "ajouter l'appel a stage2.rs, ou inscrire l'ecart dans "
                "ECARTS_ASSUMES de ce fichier avec la raison, verifiable dans "
                "le code, pour laquelle ce chemin s'en passe." % service
            )

    # Un ecart assume qui a cesse d'en etre un doit sortir de la liste :
    # sinon elle devient une liste de dispenses perimees.
    for service, _ in sorted(ECARTS_ASSUMES.items()):
        if service in presents:
            fautes.append(
                "`%s` figure dans ECARTS_ASSUMES alors que stage2.rs le "
                "demarre desormais. La dispense n'a plus d'objet : la retirer "
                "rend au garde-fou sa valeur sur ce service." % service
            )
        elif service not in attendus:
            fautes.append(
                "`%s` figure dans ECARTS_ASSUMES alors que main.rs ne le "
                "demarre plus. Une dispense sans service a couvrir ne protege "
                "plus rien et masque le prochain oubli." % service
            )

    if fautes:
        print("services d'amorcage : %d probleme(s)" % len(fautes))
        for faute in fautes:
            print("\n  - %s" % faute)
        return 1

    print(
        "services d'amorcage : %d services communs aux deux chemins, %d ecart(s) "
        "assume(s) et justifie(s)"
        % (len(attendus & presents), len(ECARTS_ASSUMES))
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
