# CI et preuves

Référence unique de la CI depuis le 17/11/2026 (`BOUCHAUD_CI_TROIS_NIVEAUX_V1`). L'ancien relevé du 2/11 est conservé dans [docs/historique/notes/CI_AND_PROOFS_2026-11-02.md](../historique/notes/CI_AND_PROOFS_2026-11-02.md).

## Contrat de preuve

Les niveaux de preuve sont, dans l'ordre : STATIC → HOST → TCG → KVM → PHYSICAL. Aucune preuve ne se déduit d'un autre niveau.

Ne valent **pas** PASS :

- `skipped` ;
- `cancelled` ;
- `NON_QUALIFIANT` ou `INCONCLUSIF` ;
- une preuve absente.

Un verdict vaut pour **un SHA** :

- un run `pull_request` teste le commit de fusion ;
- un run `push` teste la tête de branche.

## Trois niveaux

| Niveau | Quand | Workflows | Objectif |
|---|---|---|---|
| **A — développement** | chaque `push`, sur toute branche | `ci.yml` (CI Fast) | premier verdict en quelques minutes : compilation, garde-fous, tests hôte, contrats de CI |
| **B — avant fusion** | chaque `pull_request`, puis le `push` sur `main` après fusion | `integration.yml`, `reliability-v3.yml`, `non-regression.yml`, et selon les chemins modifiés `ladybird-native-browser.yml`, `native-ipc-runtime.yml`, `p13-primitives-noyau.yml`, `gpu-composition.yml`, `security-runtime.yml`, `unix-fs-reliability.yml`, `trigkey-ab-preflight.yml`, `doom-port-windows.yml`, `ladybird-run-selector-v7.yml`, `ladybird-sync.yml` | QEMU, SMP, stockage, réseau, USB, Ladybird, IPC, GPU, sécurité, persistance, sur le commit de fusion |
| **C — qualification** | nuit, semaine ou manuel | `nightly.yml`, `endurance.yml`, `reliability-v3.yml` (planifié, soak de 20 min), `soak.yml`, `release*.yml`, `p13-*.yml`, `os-primitives.yml`, `governance-audit.yml`, `release-evidence.yml` | endurance, charges combinées, fuites, release, matériel |

**Un SHA de branche n'est testé qu'une fois par niveau** :

- A tourne sur le push de la branche.
- B tourne sur la PR.
- Aucun workflow ne se déclenche à la fois sur `pull_request` et sur le push d'une branche de travail.

`tools/ci/check_pipeline_policy.py` l'exige en lisant les déclencheurs, et `tools/ci/reliability/test_politique_ci_niveaux.py` vérifie que chaque régression le fait échouer.

Mesure au 17/11, avant ce découpage :

- **15 runs par SHA de branche en médiane** (maximum 18) sur 24 SHA ;
- 7 workflows exécutés en double, push et PR, sur le même arbre ;
- CI Fast : 18 min en médiane avant son verdict, dont l'attente de runners.

### Checks requis sur `main`

Les quatre checks requis sont `fast-gate`, `integration-gate`, `reliability-gate` et `non-regression-gate`. Ce sont des noms inchangés, des jobs `if: always()`, dont la liste est dans `tools/ci/configure-protection.sh`.

- **`fast-gate`** est publié sur la tête de la PR par le run `push` de CI Fast. Son workflow se déclenche sur toutes les branches, sans filtre (`tools/ci/declencheurs.py`).
- **Les trois autres** sont publiés par les runs `pull_request`.

> **`main` est protégé sur GitHub.** La lecture de la branche le 26 novembre 2026 confirme les quatre contextes ci-dessus. L’accès au détail administratif nécessite les permissions du dépôt. Le [bilan de migration CI/CD Fabric](CI_CD_FABRIC.md) décrit les contrôles à préserver.

Une **PR ouverte depuis un fork** ne reçoit pas `fast-gate`, puisque le push a lieu dans le fork. Elle reste donc bloquée : c'est un refus, pas un faux vert. Le dépôt n'a qu'un contributeur. Pour qu'une telle PR soit testée, sa branche est poussée dans ce dépôt, comme le fait déjà la production Ladybird, qui refuse les forks.

CI Fast classe **le delta cumulé de la branche** depuis sa bifurcation de `main`, jamais le seul dernier push (`BOUCHAUD_CI_DELTA_CUMULE_V1`). Sinon, un push documentaire après un push noyau rouge sauterait les contrôles du noyau sur la tête de la PR.

### Fusionner

1. La branche contient le dernier `main`. Sinon : fusionner `main` et pousser. A retourne alors sur l'arbre fusionné, et B sur la PR.
2. Les quatre checks requis sont `success` au **SHA de tête**. Un `skipped` ou un `cancelled` n'en est pas un.
3. Les workflows B déclenchés par les chemins modifiés sont `success` au même SHA.
4. Après la fusion : la CI de `main` est vérifiée au SHA fusionné.

## Ce que fait chaque niveau

### A — CI Fast (`ci.yml`)

Jobs parallèles, chacun décidé par `classify-changes` :

| Job | Contenu |
|---|---|
| `kernel / compile + invariants` | P10 avec et sans VIRGL ; noyau par défaut **en dernier**, car le garde de profondeur de pile lit ce binaire ; noyau UEFI, sonde ; 216 garde-fous d'architecture ; budgets ; frontend UI |
| `kernel / clippy + variantes` | clippy (lints `deny`), jeton BRDP armé et désarmé, compositeur ring 3 freestanding, SDK natif et contrat d'ABI |
| `host / tests + preuves de mise a jour` | noyau construit, puis `run_host_tests.sh` avec `BOUCHAUD_EXIGE_NOYAU=1` : la symbolisation échoue au lieu de se sauter (tests Rust `test_*.rs`, unittest de `tools/ci/reliability`), preuves A/B, anti-retour et OTA, Ed25519 contre OpenSSL/libsodium, agent OTA et TLS strict |
| `browser / engine tests` | QuickJS, tests du moteur, jalons sans régression |
| `windows / scripts` | exécution des scripts PowerShell de sélection et Trigkey |
| `ci / policy` | `check_pipeline_policy.py` |

Correction de classement (`BOUCHAUD_CI_BASE_PREMIER_PUSH_V1`) : au premier push d'une branche (`before = 0…0`), le classificateur compare désormais à la bifurcation de `main`. Avant, il ne classait que le dernier commit, et une série dont le dernier commit ne touchait que la documentation sautait la compilation. Le test associé est `tools/ci/reliability/test_classe_premier_push.py`.

### B — avant fusion

- **Integration** exécute ses suites sur **chaque** PR. Avant ce découpage, son run PR n'en lançait aucune sans le libellé `ci:integration`, et `integration-gate` y était vert sans rien avoir exécuté. Seule une PR strictement documentaire n'en lance aucune, et le verdict l'écrit : `INTEGRATION_GATE_DOCS_ONLY`.
- **Reliability V3** n'a plus le job `reliability / host`. Il rejouait exactement les scripts de CI Fast sur le même SHA, ce qui faisait quatre exécutions des tests hôte par commit. Restent la propriété (64 graines), le bootimage et la matrice SMP1/2/4/8.
- **Workflows filtrés par chemins** : Ladybird, Native IPC, P13 primitives, GPU, sécurité, Unix et FS, Trigkey, DOOM et sélecteur Windows ne tournent que si leurs chemins changent.
  - Le job `dedoublonne` de Ladybird, Native IPC et P13 décide « aucun run push sur la tête », puis exécute ses tests.
  - Il ne reflète plus rien, puisque ces workflows ne tournent plus sur les branches.

### C — qualification

- `nightly.yml` (02:30) : suites QEMU noyau, DNS, produit Ladybird, dette Windows.
- `reliability-v3.yml` planifié (01:20) : matrice complète et soak hébergé de 20 min.
- `endurance.yml` : chaque dimanche.
- Manuels :
  - `soak.yml` (8 à 72 h) et `p13-final-candidate.yml` demandent des runners auto-hébergés **non enregistrés**. Ils n'ont jamais tourné : `soak` a 0 run, et les 4 runs de `p13-final` ont été annulés faute de runner.
  - `release.yml` (étiquettes `v*`), `release-trigkey.yml` (environnement protégé), `os-primitives.yml` et `p13-cible-kvm.yml`.
- Aucune campagne de release, d'écriture disque physique ou destructive n'est déclenchée automatiquement.

## Inventaire des 26 workflows (17/11/2026)

Les durées sont mesurées sur 400 runs du 14 au 17/11, file d'attente comprise, en médiane / p95, en minutes.

| Workflow | Avant | Après | Décision | Justification |
|---|---|---|---|---|
| `ci.yml` CI Fast | push branches + PR ; 18,4 / 64,3 | **A** : push toute branche | CONSERVER, refondu | barrière requise ; doublon PR retiré ; un job de 12 min scindé en 3 jobs parallèles |
| `integration.yml` | push branches + PR (PR sans suites sauf libellé) ; 28,6 / 79,0 | **B** : PR + push `main` | CONSERVER, refondu | barrière requise ; le run PR exécute réellement les suites |
| `reliability-v3.yml` | push branches + PR + nuit ; 34,6 / 74,1 | **B** + **C** (nuit) | CONSERVER, job `host` FUSIONNÉ dans CI Fast | `host` = mêmes scripts que CI Fast |
| `non-regression.yml` | PR + push `main` ; 0,8 / 26,0 | **B** | CONSERVER | barrière requise, sans doublon |
| `ladybird-native-browser.yml` | push branches + PR ; 58,3 / 123,4 | **B** (chemins) + `main` | DÉPLACER (hors push de branche) | doublon push/PR sur 15 SHA |
| `native-ipc-runtime.yml` | push branches + PR ; 26,5 / 93,0 | **B** (chemins) + `main` | DÉPLACER | doublon sur 14 SHA |
| `p13-primitives-noyau.yml` | push branches + PR ; 50,8 / 93,1 | **B** (chemins) + `main` | DÉPLACER | doublon sur 14 SHA |
| `gpu-composition.yml` | push `main`/`claude/**` + PR ; 17,7 / 52,7 | **B** (chemins) + `main` | DÉPLACER | doublon sur 6 SHA |
| `security-runtime.yml` | PR + push `main` ; 9,1 / 29,1 | **B** | CONSERVER | sans doublon |
| `unix-fs-reliability.yml` | PR + push `main` ; 6,5 / 18,3 | **B** | CONSERVER | sans doublon |
| `trigkey-ab-preflight.yml` | PR (chemins) ; 6,7 / 9,2 | **B** | CONSERVER | image A/B sous QEMU, non physique |
| `doom-port-windows.yml` | PR (chemins) ; 9,2 / 9,7 | **B** | CONSERVER | seul build Windows du port DOOM |
| `ladybird-run-selector-v7.yml` | PR (chemins) + push d'une branche supprimée ; 0 run | **B** | CONSERVER, déclencheur mort retiré | seule exécution réelle sous Windows PowerShell 5.1 |
| `ladybird-sync.yml` | PR + push `main` + nuit ; 1,3 / 7,7 | **B** / **C** | CONSERVER | épinglage amont |
| `nightly.yml` | nuit ; 4,7 / 68,1 | **C** | CONSERVER | |
| `endurance.yml` | semaine ; 89 (1 run) | **C** | CONSERVER | |
| `release-evidence.yml` | nuit + PR (chemins) ; 0,2 / 0,3 | **C** | CONSERVER | rapport, jamais une barrière |
| `governance-audit.yml` | manuel | **C** | CONSERVER | audit à la demande, coût nul |
| `os-primitives.yml` | manuel + `workflow_call` | **C** (appelé par Ladybird, P13) | CONSERVER | workflow réutilisé, ne peut pas être supprimé |
| `p13-cible-kvm.yml` | manuel + push d'une branche supprimée | **C** | CONSERVER, déclencheur mort retiré | témoins négatifs ciblés KVM |
| `p13-final-candidate.yml` | manuel (runners dédiés) | **C** | CONSERVER, absorbe `p13-dedicated-kvm` | entrée `perimetre` (complet, baseline, smp8, smp8-soak) ; seul `complet` rend le verdict final |
| `p13-dedicated-kvm.yml` | manuel ; **0 run** | — | **SUPPRIMÉ** (FUSIONNÉ) | mêmes scripts et mêmes runners que `p13-final-candidate` ; durées d'endurance reprises (`soak_minutes`) |
| `soak.yml` | manuel ; 0 run | **C** | CONSERVER | seule endurance 8 à 72 h ; runner `bouchaud-soak` à enregistrer |
| `userland.yml` | push `main` (chemins) ; 22,4 / 38,4 | **C** | CONSERVER | fabrique du userland historique, téléchargé par `tools/userland.ps1` |
| `release.yml` | étiquette `v*` | **C** | CONSERVER | release attestée |
| `release-trigkey.yml` | manuel, environnement protégé | **C** | CONSERVER | release signée |

Résultat : 26 → **25** workflows.

Aucune assertion ni aucun seuil n'a été modifié. Aucune suite n'a été retirée : une seule exécution l'a été, celle qui était refaite sur le même arbre.

## Points de surveillance des faux verts

- Un `skipped` n'est jamais un PASS. Les gates écrivent pourquoi une suite n'a pas tourné : `INTEGRATION_GATE_DOCS_ONLY`, `RELIABILITY_GATE_DOCS_ONLY`, `P13_PERIMETRE_PARTIEL`.
- Une recherche de texte dans un workflow est satisfaite par un commentaire. Les contrats de déclencheurs lisent le YAML : `check_pipeline_policy.py` et `declencheurs.py`. `verifie-protection-main.py` cherchait `pull_request` comme sous-chaîne, et un commentaire le satisfaisait.
- Pour un `pull_request`, distinguer la tête de branche du commit de fusion testé.
- Un `|| true` peut servir à recueillir des journaux, jamais à convertir une erreur.
- Ne jamais requalifier P13 avec `P13_STRICT=0`, ni avec un runner KVM4 hébergé utilisé comme SMP8.
