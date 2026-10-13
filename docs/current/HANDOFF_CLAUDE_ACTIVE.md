# Handoff actif — Claude (session du 13 octobre 2026)

> Source vivante de reprise. Les handoffs datés (`HANDOFF_CLAUDE_P19_2026-10-12.md`,
> `HANDOFF_CODEX_2026-10-08.md`) restent l'historique ; ce fichier dit où en
> est le travail **maintenant**. Rien ici n'annonce un résultat futur.

## 1. Git

| | |
|---|---|
| Dépôt / branche | `bcharthur/bouchaud-os` / `claude/ladybird-observability-performance` |
| Base de session | `0a83f182` (remote au démarrage) ; `main` = `788c5446` |
| Commits de session | `56f12ef7`, `9728a4d9`, `bf4a40b5`, `c9572100`, `ab9c4dda`, `ba8c68ff`, puis le commit docs qui ajoute ce fichier |
| Worktrees | principal ; `scratchpad/wt-neg` (contrôle négatif, à jeter) ; `scratchpad/wt-origin` (ancien, détaché) |
| Interdits respectés | aucun reset/clean/force-push/rebase ; jamais `main` |

## 2. Ce qui a été corrigé, avec la preuve

| Bug | Cause racine | Avant → après | Niveau |
|---|---|---|---|
| Garde préemption 166/167 | réouverture d'IF P19 non bornée au ring 3 ; garde qui ne lisait que la 1re réouverture | 166/167 → 167/167, 11 tests négatifs | STATIC |
| Crash Guard multi-fils | `exit_group` avec IF=0 → `schedule()` | contrôle négatif : KERNEL PANIC `ordonnancement.rs:277` → 16/16 | TCG SMP4 |
| Fil frère en `pause()` immortel | `sys_sigsuspend` ne relisait pas la condamnation | 8/8 non terminés → 16/16 | TCG SMP4 |
| CPU0 ne préempte pas l'utilisateur | report diagnostic V8 jamais retiré | SMP1 : gel ; SMP8 : immortel → 24/24 et 24/24 | TCG SMP1/SMP8 |
| Nightly `wm_vers_navigateur` | banc sans socat/nc | molette jamais envoyée → banc refuse + Nightly installe | HOST |
| Endurance hebdo illisible | rapport sans détail | nomme la vérification rouge | HOST |

Locaux, même noyau (`c9572100`) : `OS_PRIMITIVES_OK` (TCG SMP4, sonde
multi-fils en invité) ; endurance hebdo SMP1 1/1 (`ordonnanceur-probe` 4/4) ;
167/167 garde-fous ; `verifie-readme-commandes` OK.

## 3. Rouges restants (avec dernière trace)

1. **Panique lockdep `courant.rs:276`** (Reliability V3 38217842305, main
   `788c5446`, soak cycle 6/6) : `before_acquire(SchedulerTransition)` avec un
   verrou rangé tenu (DEPTH>0) sur CPU1. Message lockdep complet non lu
   (artefact inaccessible au proxy). Non reproduit : 11/11 cycles locaux.
2. **`ordonnanceur-probe` 1/4** (endurance hebdo 38130602802) : CI seulement.
3. **ONE-LOADER** : pas de preuve de rendu.
4. **LibJS `Value.cpp:484`** : ouvert.
5. **P13** : la campagne same-HEAD complète (CI Fast + Reliability + Integration
   + Ladybird + os-primitives) n'a pas encore été lue pour ce HEAD.

## 4. Prochaine commande exacte

Après le push de la session : lire la campagne déclenchée par le push, puis
déclencher sur la branche

```text
Reliability V3      (workflow_dispatch, ref=claude/ladybird-observability-performance)  -> soak 20 min
Endurance hebdo     (workflow_dispatch, même ref)                                      -> SMP1/2/4/8
```

Reproduction locale du Crash Guard (≈ 30 s par topologie, TCG) :

```bash
cargo bootimage
BO_SMP=1 tools/ci/run_faute_multifil.sh target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin 2
BO_SMP=8 tools/ci/run_faute_multifil.sh target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin 2
```

## 5. Hypothèse principale restante

La panique lockdep de transition : un chemin appelle `schedule()` (ou une IRQ
ouvre une transition) pendant qu'un `RankedSpinLock` est tenu sur le cœur
(fenêtre entre la libération du `SpinLock` interne, qui rétablit IF, et
`lockdep::released` dans `RankedSpinLockGuard::drop`, ou un vrai sommeil sous
verrou). Expérience falsifiable : faire imprimer par la panique la classe
tenue (déjà dans le message `LOCKDEP inversion ... held_rank=`) et rendre ce
message lisible dans le journal du job ; puis un banc qui exerce les cinq
classes rangées sous préemption.

## 6. Physique

Aucun test TRIGKEY. Aucune image physique produite dans cette session.
`PHYSICAL_STATUS=BLOCKED` (machine et flash appartiennent à Arthur).
