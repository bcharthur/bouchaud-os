# Bouchaud OS — V11C Final Deep Fragmentation

V11C est le **dernier V11**.

Il part de l'état réellement validé : V11B appliqué directement sur V10/V9.1,
sans exiger l'ancien V11A. Il absorbe donc le travail utile de V11A et termine
le grand découpage de `kernel/process/thread.rs`.

## Ce que V11C absorbe de V11A

- acquisition BKL -> `bkl/acquisition/`
- diagnostic `try_enter` sites 600..699
- IDT restante -> exceptions / clavier / stockage / souris
- persistance -> format / arbre / index / montage / sync / collecte / codec

L'ancien ZIP V11A ne doit donc **plus être appliqué séparément**.

## Thread/scheduler

Avant :

```text
src/kernel/process/thread.rs
  ~ plusieurs milliers de lignes
  mémoire + process + Task + scheduler + switch + blocage + futex + métriques
```

Après :

```text
src/kernel/process/
├── thread.rs
└── thread/
    ├── modeles.rs
    ├── faute_memoire.rs
    ├── processus.rs
    ├── tache.rs
    ├── etat_global.rs
    ├── diagnostic_stall.rs
    ├── courant.rs
    ├── creation.rs
    ├── commutation.rs
    ├── comptabilite.rs
    ├── ordonnancement.rs
    ├── lifecycle.rs
    ├── blocage.rs
    ├── preemption.rs
    ├── metriques.rs
    ├── sommeil.rs
    ├── futex.rs
    └── diagnostic.rs
```

Chaque fragment est `include!()` dans le même module `kernel::task`.

Conséquences :
- aucune nouvelle ABI ;
- aucune nouvelle visibilité Rust ;
- mêmes statiques ;
- même ordre lexical ;
- un bug futex se lit dans `thread/futex.rs` ;
- un bug de switch dans `thread/commutation.rs` ;
- un bug de wake dans `thread/blocage.rs` ;
- les logs SMP/BKL sont dans `thread/metriques.rs` et
  `thread/diagnostic_stall.rs`.

## Indépendance du noyau

V11C conserve la frontière V11B :

```text
compat/linux
      ↓ traduction seulement
kernel/native
      ↓
primitives Bouchaud OS
```

Le `thread/futex.rs` de V11C est encore l'implémentation historique nécessaire à
la compatibilité actuelle. V11C ne prétend pas qu'elle est la primitive finale.

À partir de V12, le chantier sera sémantique :
- attente sur mot native Bouchaud à buckets/verrous locaux ;
- adaptateur Linux futex vers cette primitive ;
- migration poll/readiness objet par objet ;
- réseau event-driven/readiness ;
- réduction réelle du BKL dans les chemins chauds.

## Test

```powershell
git diff --check
python .\tools\dev\verifie-fragmentation-v11c.py

.\run.ps1 -Ladybird -LadybirdUrl "https://www.google.com/" |
    Tee-Object -FilePath final-v11c-google.log

python .\tools\perf\analyse-v11c.py .\final-v11c-google.log
```

V11C est volontairement principalement structurel + observabilité. Le but est
que le prochain changement de politique commence directement en **V12** sur des
frontières propres.
