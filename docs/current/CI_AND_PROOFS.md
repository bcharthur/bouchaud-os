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

## Un build par arbre (BOUCHAUD_CI_UN_BUILD_PAR_SHA_V1)

Un push sur `claude/**` ou `feat/**` qui porte une PR déclenche deux runs de
chaque workflow : `push` (la tête) et `pull_request` (le commit de fusion).
Sur ed5602f4, `ladybird-native-browser` a ainsi construit Ladybird deux fois
(runs 39479633757 et 39479638849, même arbre).

`ladybird-native-browser` et `native-ipc-runtime` ont un job `dedoublonne`
(`tools/ci/dedoublonne_pr.py`) dont dépendent leurs jobs racines. Le run PR
n'est sauté que si TOUT tient :

* PR de ce dépôt, pas un `labeled` ;
* un run `push` du même workflow existe sur la même tête, ni annulé ni sauté
  (les filtres de chemins diffèrent d'un déclencheur à l'autre) ;
* l'arbre de fusion est identique à l'arbre de tête : si `main` a avancé, la
  fusion est un autre arbre et le run PR tourne.

Dans le doute (API muette, erreur), tout tourne. Le résumé du job nomme le run
push qui fait foi.

CI Fast, Integration et Reliability V3 ne sont PAS dédoublonnés : leurs gates
(`fast-gate`, `integration-gate`, `reliability-gate`) sont les checks requis de
`main` (`tools/ci/configure_protection.ps1`), et GitHub compte un job sauté
comme réussi. Les dédoublonner demande une décision humaine sur la protection
de branche. `check_pipeline_policy.py` tient les deux règles (5 négatifs).
