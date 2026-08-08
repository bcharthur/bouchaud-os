#!/bin/bash
# Tableau de bord du navigateur : tout ce qui se verifie, et son evolution.
#
#   ./suivi.sh                # tout : moteur, hote Qt, temoins, sites reels
#   ./suivi.sh --rapide       # sans reseau ni Qt, quelques secondes
#   ./suivi.sh --sans-corpus  # temoins locaux, mais garde l'hote Qt
#   ./suivi.sh --histoire     # la trajectoire, sans rien relancer
#   ./suivi.sh --strict       # sort en erreur si une mesure surveillee recule
#   ./suivi.sh --bavard       # montre la sortie brute de chaque verification
#
# Les trois verifications du depot ne se parlaient pas : `test-moteur.sh`
# comptait des assertions, `verifie-hote.sh` des pixels, `compatibilite.py` des
# manques. Aucune ne disait si le navigateur allait mieux qu'hier. Celle-ci les
# lance toutes, range leurs nombres au meme endroit, et les compare a la
# derniere execution de meme portee.
#
# L'historique vit dans `navigateur/tests/suivi.jsonl`, versionne avec le code :
# `git log -p` sur ce fichier raconte l'histoire de la compatibilite mieux qu'un
# journal ecrit a la main, parce qu'il ne peut pas mentir.

set -e
cd "$(dirname "$0")"
exec python3 navigateur/suivi.py "$@"
