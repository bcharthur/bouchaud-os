#!/usr/bin/env python3
"""Garde-fou : le prechauffage aide le navigateur sans jamais se voir.

# Ce qu'il corrige

« Sur le deuxieme demarrage Ladybird a demarre bien plus vite. » Les deux
demarrages executent le meme binaire depuis le meme disque memoire ; ce qui
change entre eux est le cache de pages propres. Le releve du 12 septembre le
chiffre : `[BACKING-CACHE] clean_hit=5408 clean_miss=17346` au premier
lancement, et `[MM-NG6] fault_resolved=6878` defauts de page resolus avant que
la premiere fenetre n'apparaisse.

# Les trois choses qu'il n'a PAS le droit de faire

1. Lancer les moteurs Web. Ils couteraient leur memoire et leur processeur en
   permanence, pour un utilisateur qui n'ouvrira peut-etre jamais le
   navigateur -- et il faudrait decider quoi faire du processus deja lance
   quand il en demande un.
2. Demander plus de pages que le cache n'en retient. Au-dela, on chasse par la
   fin ce qu'on vient de charger au debut, ce qui est pire que de ne rien
   faire.
3. Se voir. Il part apres le bureau, travaille par tranches et rend la main
   entre chacune.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
SOURCE = RACINE / "src/kernel/memory/prechauffage.rs"
CACHE = RACINE / "src/kernel/memory/page_cache.rs"
STAGE2 = RACINE / "src/platform/pc/stage2.rs"
MAIN = RACINE / "src/main.rs"


def sans_commentaires(texte):
    return "\n".join(
        l for l in texte.splitlines() if not l.lstrip().startswith("//")
    )


def corps(source, entete):
    debut = source.find(entete)
    if debut < 0:
        return None
    i = source.find("{", debut)
    if i < 0:
        return None
    profondeur = 0
    for j in range(i, len(source)):
        if source[j] == "{":
            profondeur += 1
        elif source[j] == "}":
            profondeur -= 1
            if profondeur == 0:
                return source[i:j + 1]
    return None


def main():
    fautes = []
    for chemin in (SOURCE, CACHE, STAGE2, MAIN):
        if not chemin.exists():
            print("  - fichier absent : %s" % chemin)
            return 1

    source = sans_commentaires(SOURCE.read_text(encoding="utf-8"))
    cache = sans_commentaires(CACHE.read_text(encoding="utf-8"))

    # 1. IL NE LANCE RIEN.
    #
    # Les chemins de `BINAIRES` contiennent « libexec » : on ne cherche donc
    # pas une sous-chaine, mais un APPEL -- le nom suivi d'une parenthese.
    for interdit in ("sys_execve", "spawn_utilisateur", "exec_path", "fork"):
        if re.search(r"\b%s\s*\(" % re.escape(interdit), source):
            fautes.append(
                "prechauffage.rs : il LANCE quelque chose (`%s`). Demarrer les "
                "moteurs Web a l'amorcage couterait leur memoire et leur "
                "processeur en permanence, pour un utilisateur qui n'ouvrira "
                "peut-etre jamais le navigateur." % interdit
            )

    # 2. IL REND CE QU'IL PREND.
    un = corps(source, "fn prechauffe_un(")
    if un is None:
        fautes.append("prechauffage.rs : `prechauffe_un` a disparu.")
    else:
        if "clean_page_cache::acquire(" not in un:
            fautes.append(
                "prechauffage.rs : il ne remplit plus le cache de pages "
                "propres ; il ne prechauffe donc rien."
            )
        if "clean_page_cache::release(" not in un:
            fautes.append(
                "prechauffage.rs : il PREND sans RENDRE. Les pages resteraient "
                "attachees pour toujours, et la pression memoire ne pourrait "
                "plus les reprendre -- un prechauffage devenu une fuite."
            )
        if "sleep_ticks" not in un:
            fautes.append(
                "prechauffage.rs : il ne rend plus la main entre deux "
                "tranches ; il se verrait."
            )
        if "drop(fs)" not in un:
            fautes.append(
                "prechauffage.rs : le verrou du systeme de fichiers est tenu "
                "pendant le chargement des pages. `acquire` peut lire le "
                "support : tout le monde attendrait derriere."
            )

    # 3. SON PLAFOND NE DEPASSE PAS CE QUE LE CACHE RETIENT.
    m_pages = re.search(r"const PAGES_MAX: usize = ([0-9_]+);", source)
    m_cache = re.search(r"const MAX_RECLAIMABLE_PAGES: usize = ([0-9_]+);", cache)
    if m_pages is None:
        fautes.append("prechauffage.rs : le plafond de pages n'est plus lisible.")
    elif m_cache is not None:
        plafond = int(m_pages.group(1).replace("_", ""))
        capacite = int(m_cache.group(1).replace("_", ""))
        if plafond > capacite:
            fautes.append(
                "prechauffage.rs : le plafond (%d pages) depasse ce que le "
                "cache retient (%d). On chasserait par la fin ce qu'on vient "
                "de charger au debut." % (plafond, capacite)
            )

    # 4. IL PART APRES LE BUREAU.
    fil = corps(source, "fn fil_prechauffage()")
    if fil is None:
        fautes.append("prechauffage.rs : le fil a disparu.")
    else:
        m = re.search(r"sleep_ticks\((\d[0-9_]*)\)", fil)
        if m is None or int(m.group(1).replace("_", "")) < 1_000:
            fautes.append(
                "prechauffage.rs : le fil ne laisse plus le bureau s'installer "
                "avant de travailler. Les premieres secondes sont les plus "
                "chargees, et ce sont celles ou l'utilisateur regarde l'ecran."
            )

    # 5. IL EST LANCE SUR LES DEUX CHEMINS DE DEMARRAGE.
    for chemin, nom in ((STAGE2, "stage2.rs"), (MAIN, "main.rs")):
        texte = sans_commentaires(chemin.read_text(encoding="utf-8"))
        if "prechauffage::demarre()" not in texte:
            fautes.append("%s : le prechauffage n'est plus lance." % nom)

    if fautes:
        print("prechauffage : %d probleme(s)\n" % len(fautes))
        for f in fautes:
            print("  - %s\n" % f)
        return 1
    print(
        "prechauffage : aucun processus lance, pages prises et rendues, "
        "plafond sous la capacite du cache, main rendue entre les tranches, "
        "verrou du systeme de fichiers relache, lance sur les deux chemins"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
