# Production de l'artefact Ladybird séparée de la qualification du noyau

`BOUCHAUD_LADYBIRD_PRODUCTION_V1`, 10/11/2026. Ce document mesure d'abord,
puis décrit ce qui a changé et ce qui n'a pas bougé. Toutes les durées viennent
des horodatages `started_at` / `completed_at` des jobs (API GitHub) ; tous les
volumes viennent des journaux des jobs (`Cache saved`, `Sent N of N`,
`ccache -s`). Le proxy de cette session refuse l'API `actions/caches` (HTTP
403) : l'inventaire des caches ne peut pas être lu directement, il est
reconstitué depuis les journaux.

## 1. Audit (06 → 10/11, 40 derniers runs `ladybird-native-browser`)

### Où passe le temps d'un build complet

| Run (SHA) | Job build | Préparation (toolchain + vcpkg) | Compilation | Run complet |
|---|---|---|---|---|
| 42815443584 (8e3c777, qualification) | 143,6 min | 38,8 min (vcpkg) | 103,2 min | 171,4 min |
| 42784700689 (540fdfb, qualification) | 143,3 min | 39,3 min | 102,1 min | 166,9 min |
| 42691346286 (c188155) | 140,7 min | 37,6 min | 100,7 min | 167,9 min |
| 42694957199 (1c51a58) | 99,0 min | 29,3 min | 68,4 min | 123,2 min |
| 42209695755 (f212bd0, main) | 116,4 min | 32,7 min | 70,9 min | 150,0 min |

Publication de l'artefact : 18 s pour 434 Mo (mesure antérieure,
`docs/CI_PALIERS.md`), négligeable.

### Caches : restaurations, volumes, évictions

| Cache | Volume sauvegardé par build | Restaurations observées |
|---|---|---|
| vcpkg (dépendances prêtes) | 1 125,8 Mo | **0 / 7** |
| Ninja (build incrémental) | 1 028,2 Mo | **0 / 7** |
| ccache | 137,4 Mo (0,1 Gio sur 5 Gio) | **0 / 7** — 0 succès sur 2 710 compilations, à chaque build |
| Sources Ladybird | 97,0 Mo | **0 / 7** |
| Cargo Ladybird | 41 Mo | 1 / 7 |

Sept builds complets (06/11 → 10/11), et pas une seule restauration des quatre
caches qui comptent, alors que chaque sauvegarde réussit. Exemple : les caches
sauvegardés à 14:51 par le build de 8e3c777 étaient introuvables à 16:47 pour
le build de 93d9421, sur la MÊME branche, alors que leurs `restore-keys`
correspondaient. Explication compatible avec toutes les observations
(l'inventaire direct étant refusé par le proxy) : **éviction LRU de la limite
de 10 Go du dépôt**. Chaque build sauvegarde environ 2,4 Go, et 33 clés de
cache indexées par SHA, réparties sur 10 workflows, en écrivent d'autres à
chaque push (`native-ipc` seul : 411 Mo par push).

### Builds redondants (même empreinte, déjà produite)

| Empreinte | Builds complets | Redondants |
|---|---|---|
| `6199f460` (branche c22/p13/p10) | 1c51a58 99 min, 540fdfb 143 min, 8e3c777 144 min, 93d9421 (en cours) | **2 terminés + 1 en cours**, plus 116 min de builds partiels annulés ou tués (27 et 89 min) |
| `4e3f2384` (main) | f212bd0 116 min, 7412d94 105 min — **en parallèle, à 2 min d'écart** | **1** |

Cause : `ladybird_provenance.py` forçait `mode=construit` au palier
`qualification`, et le build vivait dans un run groupé par ref, annulé par le
push suivant.

## 2. Ce qui a changé

| Rôle | Où | Concurrence |
|---|---|---|
| **Production de l'artefact Ladybird** | run `ladybird-native-browser` palier `artefact` (workflow_dispatch, titre `ladybird-artefact <empreinte>`) : provenance + build + verdict de production | groupe `ladybird-artefact-<empreinte>`, **jamais annulé** |
| **Production de l'image Bouchaud OS** | inchangée : chaque job d'épreuve construit le bootimage de son SHA (1 à 2 min, cache Cargo) | — |
| **Qualifications C22 / P13 / P10** | runs consommateurs (`qualification`, `developpement`), `p13-primitives-noyau`, `reliability-v3`, `gpu-composition` | groupés par ref, annulés par le push suivant |

Décision de provenance d'un run consommateur :

1. un artefact de **même empreinte**, issu d'un **build réel** (job
   `ladybird / build once` conclu `success`), de ce dépôt, de ce workflow,
   événement push ou dispatch, artefacts non expirés : **réutilisé**, puis
   revérifié (manifeste + liaison par les entrées) avant republication.
   L'empreinte est recalculée depuis git pour le `head_sha` du run source ;
   jamais lue dans un titre ou un nom ;
2. sinon, une production de cette empreinte **en cours** : le run la
   **rejoint** (`attendu`) sans en relancer une ;
3. sinon, il **déclenche** la production et l'attend (`attendu`) ;
4. si la production ne peut pas être déclenchée (pull_request d'un fork,
   jeton en lecture) : **build dans le run** (`construit`), comme avant.

`reconstruire: true` (dispatch) : audit, un build **neuf** est exigé,
postérieur au début du run ; aucun artefact antérieur n'est accepté.

Ce qui n'a pas bougé : les 13 épreuves Ladybird et leurs seuils, les six essais
C22 et le seuil `g2 > 1024 Kio`, les primitives P13 strictes en qualification,
la garde d'empreinte, la revérification de tout artefact non construit dans le
run, CONVERGENCE_OK réservé à la qualification. Le job `build` n'a pas été
déplacé : l'empreinte de `main` reste `4e3f2384`, les artefacts existants
restent valides.

## 3. Ce qui empêche un faux vert

- `contrat_paliers.py` : modes `construit | reutilise | attendu` ; `existe`
  (aucun artefact dans le run) n'est permis à aucun palier consommateur ;
  `reutilise` exige le job de revérification, `attendu` exige l'attente ET la
  revérification ;
- `simule_paliers.py` : 9 scénarios nominaux verts, 16 pannes rouges, 2
  scénarios de production (build seul, ou rien si l'artefact existe ; verdict
  de production rendu) ;
- `verifie-ci-paliers.py` : 40 mutations détectées (27 avant) ;
- `verifie-verdicts-negatifs.py` (nouveau) : `| tee` sans pipefail,
  `|| true` derrière un vérificateur, `continue-on-error` non recensé (6
  emplacements recensés, tous diagnostics). Il aurait refusé le faux vert
  `LADYBIRD_BENCH_ECHEC` du run 42842860256 ;
- `test_ladybird_reutilisation_bout_en_bout.py` : vrai dépôt git, vraie
  empreinte, vraie décision. Un commit noyau réutilise en qualification et en
  développement ; une modification de source (SHA upstream), de patch,
  d'outil (apt), de dépendance (vcpkg), de script de build ou d'étape
  productrice invalide l'artefact.

## 4. Checkpoints de compilation et runner persistant (évaluation)

Une fois la réutilisation en place, un build complet n'est nécessaire que
lorsque l'empreinte change. Fréquence mesurée : 5 empreintes distinctes du
06 au 10/11 (`4e3f2384`, `fb7528c2`, `91562d3e`, `6199f460`, `8620fb0d`),
soit environ une par jour. Le build froid restant coûte environ 143 min, dont
environ 39 min de dépendances vcpkg et environ 103 min de compilation.

| Option | Ce que les mesures disent | Décision |
|---|---|---|
| Caches GitHub (actuels) | 0 restauration sur 7 builds : éviction | conservés (inoffensifs), mais **ne sont pas un levier** tant que la limite de 10 Go est saturée par les caches par SHA |
| Checkpoint vcpkg en **artefact** (1,13 Go, hors éviction LRU, clé = identité toolchain + entrées vcpkg) | supprimerait environ 39 min/build ; un téléchargement d'artefact de 434 Mo prend moins d'une minute aujourd'hui | **recommandé, prochaine étape** ; gain à mesurer sur deux builds successifs |
| Checkpoint ccache en artefact (137 Mo) | jamais mesuré au-dessus de 0 % dans ce dépôt : le gain sur la compilation est inconnu | à mesurer après le checkpoint vcpkg ; ne pas annoncer de gain |
| Runner persistant (`tools/ci/runner/provision_kvm_runner.sh`) | garderait ccache, vcpkg et Ninja chauds sur disque ; supprime l'éviction | **BLOCKED_EXTERNAL** : aucun matériel provisionné. Pertinent seulement si les empreintes changent plus souvent qu'une fois par jour |
| Réduire les caches par SHA des autres workflows (33 clés) | libérerait la limite de 10 Go | à traiter séparément : change le temps de build Cargo de 10 workflows |

## 5. Matrice avant / après

Remplie avec les runs réels de la PR, voir la section suivante.
