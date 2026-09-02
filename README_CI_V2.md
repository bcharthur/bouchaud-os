# Bouchaud OS — CI/CD V2

Ce pack est prevu pour etre extrait **a la racine du depot** `bouchaud-os` avec
remplacement des fichiers existants.

Il ne supprime aucun scenario de preuve : les gros workflows historiques restent
accessibles en `workflow_dispatch`, mais ne sont plus tous lances sur chaque
commit.

## Graphe

```text
PR / push main
    |
    +--> CI Fast
    |      +-- classification des fichiers
    |      +-- kernel compile + invariants
    |      +-- browser-engine
    |      +-- PowerShell
    |      `-- fast-gate
    |
    +--> Integration (main automatiquement)
    |      +-- bootimage construit UNE fois
    |      +-- qemu smoke
    |      +-- system-health
    |      +-- os-primitives si concerne
    |      +-- mm-ng6 si concerne
    |      +-- DNS
    |      `-- Qt pixel si concerne
    |
    +--> Ladybird natif (seulement si Ladybird change)
    |      +-- vcpkg / source / ccache
    |      +-- build unique
    |      +-- artefact canonique
    |      `-- BrowserHost smoke sur Bouchaud OS
    |
    `--> Nightly Full
           +-- noyau construit UNE fois
           +-- health / primitives / mm en parallele
           +-- DNS
           `-- dernier artefact Ladybird de main
```

## Deduplication push / PR

Les workflows automatiques de validation utilisent :

- `pull_request` sur les branches de travail ;
- `push` uniquement sur `main`.

Un SHA appartenant a une PR ne lance donc plus simultanement un pipeline `push`
et un pipeline `pull_request` equivalent.

## Workflows lourds devenus manuels

Ces fichiers restent presents pour isoler une panne et reproduire un scenario :

- `system-health.yml`
- `os-primitives.yml`
- `mm-ng6-smp4.yml`
- `ladybird-browser-host.yml`
- `ladybird-platform-smp4.yml`

`ladybird-native-browser-v16.yml` est volontairement transforme en pointeur
manuel : V16 est une capacite de l'artefact produit par
`ladybird-native-browser.yml`, plus un second build complet.

## Integration sur une PR avant merge

Par defaut une PR ne paie que `CI Fast`. Pour demander la campagne Integration
sur une PR, ajouter le label GitHub :

```text
ci:integration
```

Le label reste actif sur les nouveaux commits de la PR jusqu'a son retrait.

## Scripts reproductibles localement

La logique QEMU n'est plus enterree dans le YAML :

```text
tools/ci/build_kernel.sh
tools/ci/run_qemu_smoke.sh
tools/ci/run_system_health.sh
tools/ci/run_os_primitives.sh
tools/ci/run_mm_ng6.sh
tools/ci/run_ladybird_browser_host.sh
tools/ci/run_platform_boot.sh
```

Cela permet de relancer exactement une etape sur une machine Linux sans relancer
un workflow GitHub entier.

## Compatibilite avec run.ps1

Le workflow canonique conserve exactement :

```text
.github/workflows/ladybird-native-browser.yml
artifact: bouchaud-ladybird-native-browser
```

`run.ps1` peut donc continuer a rechercher le dernier build reussi de ce
workflow sur la branche courante.

Le userland historique est conserve sous `userland.yml`, mais il n'est plus
construit sur chaque branche : il tourne automatiquement sur `main` uniquement
quand ses propres entrees changent, ou manuellement via `workflow_dispatch`.

## Branch protection conseillee

Le check stable a rendre obligatoire sur les PR est :

```text
CI Fast / fast-gate
```

`Integration / integration-gate` peut rester optionnel tant que la CI V2 est en
observation, puis devenir obligatoire sur les PR sensibles si souhaite.

## Premier deploiement conseille

1. Extraire le ZIP a la racine et accepter les remplacements.
2. `git diff -- .github tools/ci`
3. Committer sur une branche `ci/v2`.
4. Ouvrir une PR : seul `CI Fast` doit partir automatiquement, plus Ladybird si
   cette PR modifie effectivement `tools/ladybird/**`.
5. Ajouter le label `ci:integration` pour tester le second niveau.
6. Une fois vert, merger sur `main` : Integration part automatiquement.
7. Laisser Nightly tourner au moins une nuit avant de supprimer d'anciens caches
   GitHub Actions si besoin.

## Point volontairement non change

Le userland legacy garde un manifeste lie au SHA du commit. Le passage a un
`USERLAND_FINGERPRINT` partage entre plusieurs commits est une etape suivante,
car `run.ps1` doit etre modifie en meme temps pour ne pas creer un contrat de
compatibilite incomplet.
