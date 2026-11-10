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

## 5. Matrice avant / après (runs réels du 10/11)

| Scénario | Avant | Après | Preuves |
|---|---|---|---|
| **Qualification du noyau**, recette Ladybird inchangée | 166,9 et 171,4 min ; build 143,3 et 143,6 min (runs 42784700689 à 540fdfb, 42815443584 à 8e3c777, même empreinte `6199f460`) | **55,0 min, 0 build** : provenance 25 s, réutilisation revérifiée 49 s, artefact disponible à +1 min 29. Épreuves Ladybird terminées à +24 min 21. Chemin critique : `primitives / probes` (P13 strict, 54 min 43 s), inchangé | 42895151951 (aafe24e) : `PROVENANCE_DECISION mode=reutilise source_run=42209695755`, `LADYBIRD_EMPREINTE_LIEE empreinte=4e3f2384… producteur=f212bd0`, `BOUCHAUD_PALIER=qualification provenance=reutilise`, `BOUCHAUD_LADYBIRD_CONVERGENCE_OK`, `P13_STRICT_OK` SMP1/2/4, `C22_CONTRACTUEL_6X6_OK` (g2 120, 148, 36, 136, 140, 148 Kio) |
| **Intégration navigateur**, artefact valide existant | provenance puis réutilisation déjà possibles en développement, mais candidat de `main` hors limite (12) | artefact disponible à +1 min 25 ; épreuves en 23 à 26 min hors incident | 42875324047 (bf7db3d) : 15ᵉ candidat retenu ; run de 43 min, **rouge** : piège du Compositor et robustesse, voir « Incidents » |
| **Changement de recette Ladybird** | build dans le run (143 min), annulé par le push suivant | production séparée : build 80 min (runner plus rapide, caches toujours froids : 0/5 restaurés, ccache 0/2 710) ; **un push noyau annule le consommateur, pas la production** ; le run suivant la **rejoint** sans relancer de build, réutilise en 47 s, 16 épreuves vertes | 42875682665 (production, `verdict de production` vert), 42875546372 (consommateur annulé), 42876943129 (`production run=42875682665 … SUIVIE`, `mode=attendu`, intégration verte) |
| **Builds identiques** | `6199f460` : 3 builds complets + 1 en cours + 116 min partiels ; `4e3f2384` : 2 builds parallèles | 0 sur les 4 runs consommateurs de cette PR (3 réutilisations, 1 production rejointe) | ci-dessus |
| **CI Fast** | — | 9,1 min (commit de code), 2,7 min (doc seule) | 42875324110, run sur aafe24e |
| **Volumes** | artefact 58 fichiers, 1 090 575 475 octets bruts (≈ 434 Mo compressés) | identique, republié dans chaque run consommateur | `ARTEFACT_VOLUME` du job de réutilisation |

Cibles demandées, mesurées : CI Fast 9,1 min sur un commit de code (pas
« quelques minutes » : non traité dans cette PR) ; intégration navigateur
moins de 30 min avec un artefact valide (23 à 26 min hors incident) ;
compilation Ladybird uniquement sur changement d'empreinte.

### Incidents observés (non causés par la refonte)

- **Piège du Compositor, 1 démarrage froid sur 16 :** `LADYBIRD_FUNCTIONAL_SMOKE
  fail raison=jalons manquants=7`, système figé environ 20 min (FPS 0,
  `fault_resolved` constant), 0 faute Compositor. Vu sur le noyau de `main`
  (42875324047) ET sur la branche c22/p13/p10 avec l'ancienne CI (42851644175).
  C'est un blocage intermittent du produit, antérieur à cette PR. La même
  épreuve est verte dans 42876943129 et 42895151951.
- **Robustesse, cycle des workers (42875324047) :** aucun WebWorker créé
  (`webworkers_crees=0`), 11 assertions en cascade. Verte dans 42876943129 et
  42895151951 sur le même noyau. Intermittent, cause non établie.
- **Build tué par le runner hébergé** (42851644175 tentative 1, exit 143, à
  51 min sur une limite d'étape de 150) : une production séparée, non
  annulable par un push, n'empêche pas ce genre d'arrêt ; le consommateur
  qui l'attend échoue alors (`LADYBIRD_ATTENTE_ECHEC`) et le suivant relance
  la production.
