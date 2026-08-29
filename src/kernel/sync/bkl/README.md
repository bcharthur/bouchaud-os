# BKL fragmenté — V3

Cette archive est une refactorisation **structurelle uniquement** du BKL P0 V3.

## Pourquoi `include!` plutôt que des sous-modules Rust ?

Les fichiers sont physiquement séparés par responsabilité mais sont compilés
dans le **même module `bkl`**. Cela évite de modifier la visibilité des statiques,
les chemins publics et les invariants internes pendant qu'on stabilise encore
le P0.

Cette étape doit donc être nettement moins risquée qu'une transformation
simultanée en `mod etat; mod attente; ...`.

## Répartition

- `etat.rs` : `OWNER`, `DEPTH`, identité CPU/token et `LocalIrqGuard`.
- `metriques.rs` : compteurs par syscall, provenance, max hold/wait, snapshots.
- `attente.rs` : `PARKED`, réveil ciblé, round-robin, `RESUME_WAITERS`,
  anti-barging V3 et protocole anti-réveil-perdu.
- `acquisition.rs` : `KernelGuard`, `enter`, `try_enter`,
  `try_enter_depuis_zero`, `release_one`.
- `ordonnanceur.rs` : `suspend_for_schedule` et `resume_after_schedule`.
- `enregistreur.rs` : flight recorder debug, vidage et `note_switch`.

## Important

Le patch ne touche pas `src/fs/persistance.rs` et conserve donc les modifications
P1 déjà présentes dans ton arbre.

Il ne modifie volontairement aucune logique du BKL V3 : les corps de code des
six fragments sont extraits tels quels du fichier V3, puis réassemblés par
`include!`.

Après extraction à la racine :

    git diff --stat
    git diff --check

Puis compile/teste exactement comme avant.

## V3.1 — correction de fragmentation

La première archive de fragmentation avait inclus par erreur `cpu()` et
`token()` à la fois dans `etat.rs` et `attente.rs`. Comme tous les fragments
sont volontairement inclus dans le même module Rust, cela provoquait E0428.

V3.1 garde ces deux helpers uniquement dans `etat.rs`. Aucun comportement BKL
n'est modifié.

## V4 diagnostic

Voir `BKL-V4-DIAGNOSTIC.txt` à la racine du patch.
