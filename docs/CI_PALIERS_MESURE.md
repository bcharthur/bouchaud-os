# Mesure avant/après des paliers CI Ladybird

Runs comparés, mêmes entrées Ladybird (empreinte `e38206db`) :

- **avant** : 40528051026, `workflow_dispatch` sur e58e150c, workflow
  historique (build complet + os-primitives appelé + épreuves Ladybird) ;
- **après** : 40557293468, `workflow_dispatch palier=developpement` sur
  995d2d13 (artefact réutilisé de 40528051026, primitives hors run).

Durées relevées sur les horodatages `started_at` / `completed_at` des jobs
(API GitHub). « min-runner » = somme des durées des jobs exécutés.

## Répartition

| Poste | Avant (min-runner) | Après (min-runner) | Nature de l'écart |
|---|---|---|---|
| Production de l'artefact Ladybird | 139,9 (build) | 1,4 (provenance 0,3 + réutilisation 1,0 + rendez-vous 0,1) | **gain de réutilisation** |
| Sondes P13 (os-primitives : probes 47,0, ordonnanceur 12,7, gpu 13,8, réveil 6,3) | 79,8 | 0 | **changement de périmètre** (voir plus bas) |
| Épreuves Ladybird (15 jobs, mêmes scripts, mêmes seuils) | 138,1 | 149,1 | aucun gain : +11 min de variance d'installation apt (« QEMU + outils » jusqu'à 9 min dans certains jobs du run après) |
| Dédoublonnage + verdict | 0,3 | 0,2 | — |
| **Total** | **358,1** | **150,6** | −58 % |
| **Durée murale** | **163,6 min** | **31,4 min** | −81 % |

La durée murale ne dépend que du chemin critique. Avant : build (140 min)
puis épreuves ; les sondes P13 tournaient en parallèle et n'allongeaient pas
le run. Après : réutilisation (1 min) puis épreuves (la plus longue,
endurance 20 min sous KVM, 29,6 min). **Le gain mural (−132 min) est donc
entièrement dû à la réutilisation**, pas au retrait des sondes P13.

## Volumes

| | Avant | Après |
|---|---|---|
| Artefact Ladybird publié | 433,8 Mo (58 fichiers, 1,09 Go bruts) | 433,8 Mo (republication à l'identique, 18 s) |
| Corpus WPT publié | 0,4 Mo | 0,4 Mo |
| Caches sauvegardés | Ninja 1 030 Mo + ccache 137 Mo + Cargo 41 Mo + dépendances vcpkg | aucun |
| Téléchargements de l'artefact | 14 × 433,8 Mo | 15 × 433,8 Mo (réutilisation + 14 consommateurs) |

Empaquetage mesuré dans le job de réutilisation : un tar.zst (niveau 10,
fenêtre longue) ferait 160,6 Mo au lieu de 433,8 Mo, en 7,2 s. Non adopté :
le transfert coûte 14 à 18 s par job, et `run.ps1`, `nightly.yml`,
`ladybird-browser-host.yml` et `ladybird-platform-smp4.yml` consomment le
répertoire tel quel.

## Ce que le gain couvre, et ce qu'il ne couvre pas

| Situation | Avant | Après |
|---|---|---|
| Push noyau (chemins du déclencheur), entrées Ladybird inchangées | ~358 min-runner, ~164 min murales | Ladybird développement ~151 min-runner / ~31 min murales **+** `p13-primitives-noyau.yml` ~80 min-runner / ~47 min murales en parallèle ⇒ **~231 min-runner (−36 %), ~47 min murales (−71 %)** |
| Modification Ladybird seule (`tools/ladybird/**`) | ~358 min-runner | build froid inchangé (~140 min) + épreuves ; **les sondes P13 ne tournent plus (−80 min-runner)** |
| Qualification (`workflow_dispatch`, défaut) | ~358 min-runner | **inchangé** : build canonique dans le run, os-primitives, 13 épreuves, verdict de convergence |

Le retrait des sondes P13 du workflow Ladybird n'est un gain que pour les
modifications qui ne touchent pas le noyau. Sur un push noyau, elles tournent
toujours, dans leur propre workflow, avec le même `p13_strict=false`
qu'avant sur ces branches.

## Qualification : preuves obligatoires conservées

Vérifié sans relancer de build (simulation de l'ordonnancement GitHub sur le
workflow de 995d2d13, `tools/ci/simule_paliers.py`) :

- dispatch par défaut et branche d'audit → palier qualification, décision
  `construit`, `build` exécuté, `reutilise` sauté, os-primitives exécuté,
  les 13 épreuves Ladybird exécutées, job « ladybird / verdict de
  convergence », `CONVERGENCE_OK` seulement si tout est `success` ;
- pannes simulées (build, artefact, primitives en échec ou sautées, épreuve
  sautée ou annulée, décision de réutilisation en qualification) → rouge ;
- `tools/verifie-ci-paliers.py` : 26 mutations détectées, dont tout retrait
  d'une preuve du contrat et tout câblage qui la ferait sauter.

Le chemin qualification n'a pas été rejoué sur GitHub après ces
modifications (2 h 20 de build et ~80 min de P13) : sa validation est la
simulation, les tests et la garde. Il le sera au premier run strict de la
nouvelle baseline.
