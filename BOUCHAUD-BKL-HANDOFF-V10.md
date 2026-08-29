# Bouchaud OS — BKL Waiter Handoff V10

## Pourquoi

Le run Google V9.1 valide les safe points du desktop, mais sous charge Ladybird
le BKL subit encore une tempête de contention : beaucoup de parkings, d'IPI de
réveil et de réveils qui ne mènent à aucune acquisition.

V10 ne touche pas encore à la sémantique de futex/poll. Il corrige d'abord le
mécanisme commun qui amplifie leur contention : le waiter ordinaire choisi par
`wake_parked_waiters()` pouvait être réveillé puis se faire doubler par un CPU
déjà runnable avant même d'obtenir OWNER.

## Nouveau protocole

```text
OWNER -> FREE
    |
    +-- reprise scheduler publiée ?
    |      -> priorité V3 existante
    |
    +-- handoff ordinaire déjà actif ?
    |      -> le conserver / réveiller sa cible
    |
    `-- choisir UN PARKED
           -> HANDOFF_TARGET = cpu
           -> HANDOFF_SINCE = now
           -> IPI cible

nouvel entrant
    -> contrôle RESUME
    -> contrôle HANDOFF
    -> CAS OWNER
    -> recontrôle RESUME
    -> recontrôle HANDOFF
       -> si barger : rollback OWNER
       -> si cible : claim et efface HANDOFF
```

La seconde vérification après CAS ferme la course publication/CAS.

Une lease de 50 ms borne une réservation devenue orpheline. Une reprise
scheduler annule toujours un handoff ordinaire : V3 garde donc la priorité
absolue.

## Fragmentation

```text
src/kernel/sync/bkl/
├── handoff.rs
└── handoff/
    ├── etat.rs
    ├── selection.rs
    ├── acquisition.rs
    ├── release.rs
    └── diagnostic.rs
```

Fichiers existants modifiés :
- `src/kernel/sync/bkl.rs`
- `src/kernel/sync/bkl/attente.rs`
- `src/kernel/sync/bkl/ordonnanceur/priorite.rs`
- `src/kernel/sync/bkl/ordonnanceur/resume.rs`
- `src/kernel/sync/bkl/diagnostic.rs`

Aucun changement dans `thread.rs`, persistance, GUI, souris, IDT ou ABI futex/poll.

## Test

```powershell
git diff --check

.\run.ps1 -Ladybird -LadybirdUrl "https://www.google.com/" |
    Tee-Object -FilePath bkl-handoff-v10-google.log

python .\tools\perf\analyse-bkl-handoff-v10.py .\bkl-handoff-v10-google.log
```

Critère principal : faire baisser `reveils_sans_acq`, `parks` et `wake_ipis`
par rapport à V9.1 sans régression de `reprise_max_ns`.

Validation locale de l'archive : substitutions exactes, UTF-8/LF, aucun `//!`
dans les nouveaux fragments `include!`, syntaxe Python de l'analyseur.
La compilation Rust réelle reste à valider avec `cargo bootimage`.
