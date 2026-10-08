# État courant

HEAD technique testé : `3658af04f05e04edf732cc086ddc890faaf88c52`, 8 octobre 2026.
Le commit de cette mise à jour est documentaire. [Handoff complet et preuves](HANDOFF_CODEX_2026-10-08.md).

## Campagne du même HEAD

| Workflow / groupe | Résultat |
| --- | --- |
| CI Fast | SUCCESS |
| Reliability V3 | SUCCESS |
| Integration | SUCCESS |
| os-primitives | FAILURE : préfixe de pile corrompu par entrelacement série |
| Ladybird build, smoke, cache/SQL, WPT, sites, robustesse | SUCCESS |
| crash services, ordre worker, piège Compositor, performance | SUCCESS |
| endurance TCG 10 min | SUCCESS : 108 cycles, 36/36 swaps |
| endurance KVM 20 min | SUCCESS : 230 cycles, 76/76 swaps |
| endurance KVM A/B | FAILURE : vrai remplacement du Compositor sur A ; B vert |
| mémoire après onglets fermés | FAILURE : 20/20 swaps x2, croissance RSS WebContent |
| convergence P13 | FAILURE : primitives, mémoire, KVM A/B |

## Compositor et A/B

Le rouge historique « un seul Compositor (0) » était une ligne PERF_EXECVE
entrelacée, alors que le PID 18 avait bien été créé. Pas de collecte tardive,
de logs A/B mélangés ni de seuil à relâcher.

La preuve repose maintenant sur des compteurs runtime cumulatifs répétés.
Au nouveau run 37800200518, elle détecte un vrai remplacement PID 18 → 28 :
sortie 0 autour de 40 s invité sur le bras A. Aucune assertion ou faute CPU
lisible ; la fermeture du canal de contrôle est une hypothèse à diagnostiquer.
Les 19/19 swaps et la performance réussissent sur les deux bras.

## P2/P9 mémoire

Les deux répétitions prouvent chacune 20/20 swaps distincts, fermetures et
collecte des nouveaux WebContent. M1/M2 du Compositor : 2 → 2 contextes,
2 → 2 surfaces, 4623 → 4623 KiB. Le RSS Compositor est stable en seconde phase.

Le WebContent principal reste en croissance :
69528 → 90820 → 102232 KiB, puis 69536 → 90876 → 102332 KiB.
M0 reste inconclusif faute de trois relevés périodiques stables après le
redimensionnement initial. L'ancien banc acceptait aussi des valeurs tronquées
et des événements postérieurs au repère dans la même seconde ; ces erreurs
sont corrigées, et une preuve absente ne donne plus de vert.

**P2/P9 n'est pas DONE.** Les diagnostics PageHost/GC préparés mais non publiés
comme code sont conservés avec le handoff. Le terminal est tombé hors ligne
avant leur compilation et avant l'analyse complémentaire du vrai remplacement.

## Suite

Priorité : sortie du canal de contrôle Compositor, preuve os-primitives,
puis lifecycle WebContent/GC et plateau mémoire reproductible.
Rejouer toute la campagne au même HEAD avant P10.

P10 inchangé : virtio-PCI moderne → control virtqueue → VERSION_1 →
GET_DISPLAY_INFO. Ce n'est pas de l'accélération GPU.
Aucun test TRIGKEY n'a été réalisé pendant cette reprise.
