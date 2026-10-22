# CI et preuves

Niveaux :
STATIC → HOST → QEMU-TCG → QEMU-KVM → PHYSICAL.

Workflows :
CI Fast, Reliability V3, Integration, os-primitives,
ladybird-native-browser.

TCG juge la stabilité et mesure la performance.
KVM juge stabilité + performance.

Le projet a déjà rencontré un faux vert d'horloge dû à une mesure manquante ;
les tests actuels exigent chaque mesure intermédiaire.

## Un build par arbre, sans faux vert (BOUCHAUD_CI_UN_BUILD_PAR_SHA_V2)

Un push sur `claude/**` ou `feat/**` qui porte une PR déclenche deux runs de
chaque workflow : `push` (la tête) et `pull_request` (le commit de fusion).

**V1 (3c92d7fe) était dangereuse** : le run PR sautait ses jobs dès qu'un run
push existait, même en échec ou en cours, et sautait aussi son verdict — une
PR pouvait paraître verte sans test exécuté ni réussi.

**V2** sépare la décision de la preuve (`tools/ci/dedoublonne_pr.py`) :

* `decide` (job `dedoublonne`) : réutiliser est permis si la PR vient de ce
  dépôt (pas `labeled`), si l'arbre de fusion est identique à l'arbre de tête,
  et si un run `push` du même workflow sur la même tête est en cours ou terminé
  en `success`/`failure`. Annulé, sauté, expiré, neutre, périmé : pas une
  preuve, les tests tournent. Dans le doute (API, champ absent), ils tournent.
* `attend` (job de verdict, `if: always()`, jamais sauté) : relit le run source
  jusqu'à sa conclusion FINALE et exige le run `success` ET son job de verdict
  `success` (ni sauté, ni absent), même tête, même workflow, événement `push`.
  Échec du push : rouge (bloquant). Annulé : rouge, avec la consigne de relancer
  le workflow PR (qui exécutera alors ses propres tests). Délai ou API
  indisponible jusqu'au bout : rouge. Le résumé du job nomme le run source, son
  SHA, son arbre et son workflow.

`ladybird-native-browser` (verdict : `ladybird / verdict de convergence`) et
`native-ipc-runtime` (verdict : `native-ipc / verdict`) seulement. CI Fast,
Integration et Reliability V3 portent les checks requis de `main` : non
dédoublonnés. Contrats dans `check_pipeline_policy.py` (6 négatifs), 20 tests
hôte de tous les états du run source (en cours, succès, échec, annulé, sauté,
expiré, neutre, relance en échec après la décision, job de verdict sauté ou
absent, source incohérente, erreurs d'API passagères ou permanentes,
pagination sans fin).

## Preuves indépendantes (BOUCHAUD_PREUVE_INDEPENDANTE_V1)

Dans `os-primitives`, chaque étape après le noyau construit tourne même si une
étape précédente a échoué (le job reste rouge). Run 39503091750 : un témoin TCG
rouge avait fait sauter l'accès à `/dev/kvm` et toute l'attribution P13 KVM.

## Scripts sous `pipefail` / `set -e` (BOUCHAUD_PIPEFAIL_SANS_SIGPIPE_V1)

Aucun tube vers un lecteur qui s'arrête tôt (`grep -q`, `head`) sous
`pipefail` (faux rouge par SIGPIPE), aucune `! commande` seule sous `set -e`
(faux vert : errexit l'ignore). Garde `tools/verifie-pipefail-sigpipe.py`.
