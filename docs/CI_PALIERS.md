# Paliers CI et provenance de l'artefact Ladybird (BOUCHAUD_CI_PALIERS_V2)

Ce document explique d'où venait le coût des runs `ladybird-native-browser`,
ce qui a été changé, et ce qui n'a pas bougé. Les mesures avant/après sont
dans `docs/CI_PALIERS_MESURE.md`.

## Diagnostic (run 40528051026, job 122272968079, SHA e58e150c)

| Étape du job `ladybird / build once` | Durée | Volume |
|---|---|---|
| Toolchain C/C++ | 3 min 51 s | — |
| Dépendances navigateur (vcpkg, cache manqué) | 35 min 50 s | — |
| Construire Ladybird | 99 min 06 s | ccache : **0 succès sur 2710 compilations** |
| Publier Ladybird natif | **18 s** | 58 fichiers, 1,1 Gio bruts, **433 774 222 octets** compressés (niveau 1), 52 blocs de 8 Mio sans reprise |
| Publier le corpus WPT | 3 s | 290 fichiers, 415 Ko |
| Sauvegardes de cache (Ninja, ccache, Cargo) | 19 s | 1 030 Mo + 137 Mo + 41 Mo |
| **Job complet** | **2 h 20** | |

La publication n'était pas bloquée : elle a duré 18 s. Le job paraissait
figé sur elle parce que c'est la dernière étape visible d'un job de 2 h 20.
Les vrais coûts étaient ailleurs :

1. **Ladybird est reconstruit à chaque push noyau.** L'artefact ne dépend pas
   du noyau, mais le workflow se déclenche sur les chemins noyau et le
   manifeste liait l'artefact au HEAD Bouchaud. Sur les 60 derniers runs
   (13→27/10), il n'y a eu que **4 jeux d'entrées de build distincts**. Pour le
   seul jeu `e38206db`, au moins cinq builds complets de 100 à 136 min ont été
   vérifiés job par job (40143056163, 40186341104, 40189082195, 40210815280,
   40528051026), pour un résultat que le premier fournissait déjà.
2. **Les caches incrémentaux ne servent presque jamais.** Les caches GitHub
   sont cloisonnés par branche (une branche neuve ne voit que ceux de `main`,
   qui n'en a pas) et ne sont sauvegardés qu'à la fin d'un build de deux
   heures, alors que les runs d'une même branche se chevauchent. Même avec les
   dépendances restaurées (run 40189082195), la compilation a pris 100 min.
3. **Une vérification Ladybird rejoue les sondes P13** (`os-primitives`, ~80
   min-runner : probes 47 min, ordonnanceur 13, gpu 14, réveil 6), et une
   modification du noyau rejouait 2 h 15 de build pour les obtenir.
4. **Un dépassement du délai du job perd tous les caches.** À 180 min, GitHub
   annule le job ; `!cancelled()` saute alors chaque sauvegarde, et le run
   suivant repart de zéro.

## Ce qui change

### Trois paliers

| Palier | Déclencheur | Ce qui tourne | Verdict |
|---|---|---|---|
| Rapide | `ci.yml` (inchangé) | gardes d'architecture, suites hôte, politique CI | `fast-gate` |
| Intégration ciblée | push / PR | `p13-primitives-noyau.yml` sur les chemins noyau (os-primitives, `p13_strict=false` comme avant) ; `ladybird-native-browser` palier `developpement` : toutes les épreuves Ladybird, mêmes seuils, artefact réutilisé si les entrées sont identiques | job « ladybird / verdict integration developpement » : `BOUCHAUD_LADYBIRD_INTEGRATION_OK` + `CONVERGENCE_NON_QUALIFIANTE` |
| Qualification | `workflow_dispatch` (défaut `palier=qualification`), branche d'audit | build Ladybird complet dans le run, os-primitives appelé (strict sur l'audit), toutes les épreuves | job « ladybird / verdict de convergence » : `BOUCHAUD_LADYBIRD_CONVERGENCE_OK` |

Le dispatch par défaut reste une qualification : `run-bouchaud-ladybird-ci.ps1`
et les campagnes officielles ne changent pas de sens.

### Contrat des paliers (source unique : `tools/ci/contrat_paliers.py`)

`requis` : conclusion `success` exigée ; `failure`, `cancelled`, `skipped` ou
absente sont bloquants. `hors-run` : le job n'est pas joué dans ce palier et
doit être `skipped` (toute autre conclusion signale un câblage divergent).

| Preuve (job) | Qualification | Développement |
|---|---|---|
| provenance (garde d'empreinte, décision) | requis | requis |
| producteur de l'artefact | `build` requis, `reutilise` sauté | exactement un des deux, celui qu'annonce la décision |
| artefact (producteur unique) | requis | requis |
| primitives P13 (os-primitives) | requis | hors-run (joué par `p13-primitives-noyau.yml`) |
| performance, memoire-onglets, ordre-worker, crash-services, compositor-piege, endurance-longue, browser-host-smoke, cache, wpt, endurance, endurance-kvm, sites, crash-rendu | requis | requis |
| verdict | « ladybird / verdict de convergence », `CONVERGENCE_OK` | « ladybird / verdict integration developpement », `INTEGRATION_OK` + `CONVERGENCE_NON_QUALIFIANTE` |

Il n'y a pas de palier « provenance seule » : sans épreuve Ladybird jouée, il
n'y a pas de verdict Ladybird à rendre.

`verdict_convergence.py` juge avec ce contrat. `tools/verifie-ci-paliers.py`
vérifie que le workflow le câble à l'identique (chaque `R_X: ${{ needs.<job>.result }}`,
chaque besoin du verdict, l'exclusion des épreuves hors-run, la liste des
épreuves requises dans les deux paliers est épinglée) et **simule** le
workflow avec la sémantique d'ordonnancement de GitHub
(`tools/ci/simule_paliers.py`) : 6 scénarios nominaux doivent être verts, 12
scénarios de panne doivent être rouges.

### Incident du run 40554668634 (corrigé)

Premier run de développement : provenance, réutilisation et artefact verts,
puis les 15 jobs consommateurs **sautés** à 17:53:45 — câblage, pas
intention. Ils déclaraient `needs: artefact` sans condition ; GitHub les
enveloppe alors d'un `success()` implicite qui porte sur **tous les
ancêtres**, et `build` / `reutilise` sont exclusifs : l'un est toujours sauté.
Le défaut touchait aussi la qualification (`reutilise` y est sauté). Le
verdict a été rouge, comme il devait l'être.

Correctif : `needs: [dedoublonne, artefact]` et
`if: ${{ !cancelled() && needs.dedoublonne.outputs.reutilise != 'true' && needs.artefact.result == 'success' }}`
sur chaque consommateur (et l'équivalent sur `performance`, qui attend le
smoke). Le simulateur reproduit exactement les 15 sauts sur le workflow de
14f9bcfa ; la garde rejette ce câblage par mutation.

### Provenance de l'artefact

`tools/ci/ladybird_empreinte.py` calcule, **depuis git**, l'empreinte SHA-256
des entrées du build : `tools/ladybird/**` (contenu et mode), `third_party/UPSTREAM.md`,
`tools/ci/apt-installe.sh`, `tools/verifie-chain-ladybird.py`, et le texte des
étapes productrices du job `build` (hors clés d'orchestration : délais,
conditions). Sa garde suit la recette de façon transitive et refuse toute
lecture hors empreinte ou toute étape `run` non classée.

`tools/ci/ladybird_provenance.py` réutilise un artefact uniquement si :
palier développement ; run source de ce dépôt et de ce workflow, en `push` ou
`workflow_dispatch` (**jamais `pull_request`** : un fork peut publier un
artefact du même nom) ; job `build once` réussi ; artefact et corpus non
expirés ; empreinte identique. Dans le doute, on construit.

Le job `reutilise` télécharge l'artefact, puis le **revérifie** avant de le
republier dans le run (`verifie_artefact_ladybird.sh entrees` :
manifeste complet, SHA-256 de chaque binaire, `resources/`, absence de
`PT_INTERP`, puis empreinte recalculée au HEAD producteur et au HEAD courant).
Chaque job consommateur revérifie avec la liaison publiée par le rendez-vous
`artefact` (`tete` si construit, `entrees` si réutilisé). Le job
`ordre-worker`, qui ne vérifiait rien, vérifie désormais.

### Caches

- Délai d'étape de 150 min sur « Construire Ladybird » (job : 180 min) : un
  build trop long échoue l'étape au lieu d'annuler le job, et les
  sauvegardes passent.
- Sauvegardes Ninja et ccache déplacées **avant** les publications.
- Publications bornées (20 min pour l'artefact, 10 min pour le corpus).

### Empaquetage

L'artefact garde son format (répertoire, consommé tel quel par `run.ps1`,
`nightly.yml`, `ladybird-browser-host.yml`, `ladybird-platform-smp4.yml`).
Le job `reutilise` mesure ce qu'un tar.zst (niveau 10, fenêtre longue)
gagnerait ; voir la mesure.

## Ce qui ne change pas

Aucun seuil, aucune épreuve, aucune assertion. Les jobs Ladybird jouent les
mêmes scripts avec les mêmes budgets. `os-primitives.yml` est intact et
reste manuel/réutilisable. La qualification construit toujours l'artefact
dans le run et le lie à son HEAD.

## Ce qui empêche un faux vert

- `verdict_convergence.py` : palier absent ou inconnu, provenance absente,
  invalide ou en échec, deux producteurs, réutilisation en qualification,
  build absent, toute épreuve requise non `success`, primitives jouées ou en
  échec en développement : rouge.
  `CONVERGENCE_OK` n'est imprimé qu'en qualification.
- Le job de verdict change de **nom** hors qualification.
- `tools/verifie-ci-paliers.py` (26 mutations, simulation des paliers) et
  `tools/ci/check_pipeline_policy.py` vérifient la structure ; tests :
  `test_ladybird_empreinte.py` (14), `test_ladybird_provenance.py` (10),
  `test_verdict_convergence_paliers.py` (11 tests, dont chaque épreuve requise
  × {failure, cancelled, skipped, absente} × chaque palier, et la reproduction
  du run 40554668634).

## Qualification stricte (P13_QUALIFICATION_STRICTE_V1, 28/10)

Une qualification (`workflow_dispatch`, palier `qualification`) appelle
`os-primitives` avec `p13_strict: true`, comme la branche d'audit. Avant, un
dispatch de qualification sur un candidat jouait les sondes P13 NON strictes :
le critere de sortie de la decision du 24/10 (« les 15 jobs de convergence et
`P13_STRICT_OK` SMP1/2/4 a chaque campagne ») n'etait satisfaisable sur aucune
branche candidate. `tools/verifie-ci-paliers.py` le verrouille (mutation
« qualification non stricte »).
