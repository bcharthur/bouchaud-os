# SHA upstream epingles

Ce fichier est **la** source de verite du SHA upstream — il n'y en a pas d'autre.
`tools/ladybird/fetch.sh` le lit ; `--verifie` echoue si l'arbre recupere ne
correspond pas. Voir `docs/ladybird/DEPENDENCIES.md` §7 pour la raison du choix.

Regle : un SHA n'avance jamais sans que la CI ait construit **et** teste le port
Bouchaud a ce SHA. Voir `.github/workflows/ladybird-sync.yml`.

## Ladybird

    depot   https://github.com/LadybirdBrowser/ladybird
    sha     cdfe5f858eb5fc64a8d9d3fcc247d71b03fbd1f6
    date    2026-08-14
    licence BSD 2-Clause (THIRD_PARTY_LICENSES/ladybird-BSD-2-Clause.txt)

    raison  SHA initial du portage. Choisi comme etat de reference de l'analyse
            (docs/ladybird/DEPENDENCIES.md) : le graphe de dependances, la
            surface POSIX mesuree et la liste de tiers-parti de
            THIRD_PARTY_NOTICES.md decrivent cet arbre-la et aucun autre.
