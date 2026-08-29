# Audit de fragmentation — Bouchaud OS — 2026-08-29

Cet audit distingue :
- **fragmenté** : responsabilité physiquement séparée en façade + sous-fichiers ;
- **partiel** : une partie a été extraite mais l'orchestrateur reste volumineux ;
- **à fragmenter** : responsabilité encore fortement concentrée.

## Déjà fragmenté

| Domaine | État | Structure / remarque |
|---|---|---|
| Big Kernel Lock | Fort | `src/kernel/sync/bkl/` : état, métriques, attente, acquisition, scheduler, diagnostic, flight recorder |
| Pont BKL / scheduler | Fort | `bkl/ordonnanceur/` : état, priorité, trace, suspend, resume |
| Waiter handoff V10 | Fort | `bkl/handoff/` : état, sélection/lease, acquisition, release/réveil, diagnostic |
| Performance Observatory | Fort | `src/kernel/debug/perf/` : types, flight recorder, browser, watchdog, report |
| CPU idle / HLT | Fort | `src/arch/x86_64/cpu/idle/` depuis V6 |
| Préemption / IRQ scheduler | Fort/partiel | `src/arch/x86_64/idt/` depuis V8 ; les autres classes d'IRQ/exceptions restent à extraire |
| Souris PS/2 | Fort | `src/drivers/input/mouse/` : état, PS/2, paquet, diagnostic ; façade `ps2_mouse.rs` |
| Desktop BKL scoped | Fort | `src/gui/desktop_bkl/` : état, scope, diagnostic |
| GUI générale | Déjà modulaire | nombreux modules dédiés : scene, damage, disposition, transition, windowing, theme, graphics, texte, widgets, etc. |

## Fragmenté partiellement

| Domaine | Pourquoi ce n'est pas fini |
|---|---|
| `src/kernel/process/thread.rs` | Le scheduler réel, table des tâches, current task, switch, idle, blocages, futex, kernel-thread et diagnostics cohabitent encore dans un gros fichier. |
| `src/compat/linux/mod.rs` | Le dispatcher et de nombreux syscalls restent centralisés malgré `bkl.rs`, `fd.rs`, `fichier.rs`, `horloge.rs`, `memoire.rs`, `net.rs`, `proc.rs`, `verrous.rs`. |
| GUI / `window_manager.rs` | Les briques autour sont bien séparées, mais l'orchestrateur concentre encore input, focus, z-order, drag/resize, composition et intégration client. |
| IDT x86_64 | Timer/resched/preempt ont progressé, mais exceptions, IRQ périphériques et routage peuvent encore devenir des sous-domaines. |
| Persistance | Le scope critique a été réduit, mais `persistance.rs` concentre encore index/snapshot/I/O/validation/diagnostic. |

## Priorités de fragmentation suivantes

### P0 — `kernel/process/thread.rs`

```text
src/kernel/process/thread/
├── etat.rs
├── table.rs
├── courant.rs
├── switch.rs
├── blocage.rs
├── futex.rs
├── kernel_thread.rs
├── idle.rs
└── diagnostic.rs
```

C'est la prochaine frontière structurante : elle permettra de découpler réellement
futex, WaitQueue et scheduler du BKL sans modifier un monolithe à chaque patch.

### P0 — futex

Aujourd'hui le syscall Linux et la mécanique d'attente sont répartis entre
`compat/linux/mod.rs` et `kernel/process/thread.rs`. La cible saine est :

```text
src/kernel/sync/futex/
├── cle.rs
├── table.rs
├── wait.rs
├── wake.rs
└── diagnostic.rs
```

Après cet isolement seulement, le syscall `futex` pourra être audité pour sortir
du BKL externe en sécurité.

### P0/P1 — poll / readiness

`poll` est déjà déclaré sans BKL externe, mais l'architecture readiness doit
devenir par objet / source d'attente avec générations. Ne pas remplacer
aveuglément un `wake_all` global par `wake_one` : cela peut réveiller le mauvais
poller et perdre un événement.

```text
src/kernel/object/readiness/
├── source.rs
├── registration.rs
├── poll.rs
├── wake.rs
└── diagnostic.rs
```

### P1 — réseau

```text
src/net/socket/
├── state.rs
├── rx.rs
├── tx.rs
├── wait.rs
└── diagnostic.rs
```

### P1 — mémoire / demand paging

Isoler registry de faults, clean cache, backing I/O, readahead, partage et
diagnostic. Ladybird produit une forte charge de page faults au démarrage.

### P1 — persistance

Cible :
`persistance/{index.rs,snapshot.rs,io.rs,validation.rs,diagnostic.rs}`.

### P1 — window manager

Cible :
`window_manager/{input.rs,focus.rs,zorder.rs,drag_resize.rs,composition.rs,clients.rs,diagnostic.rs}`.

### P2 — polices

Isoler chargement/cache/parsing/rasterisation pour pouvoir exécuter le travail
lourd hors BKL.

## Règle d'architecture à conserver

1. façade historique stable ;
2. fragments `include!()` dans le même module lorsque les éléments privés
   doivent rester dans le même scope ;
3. commentaires `//` dans les fragments inclus, jamais `//!` injecté après des
   items ;
4. `#[path = "..."] mod legacy;` lorsqu'un fichier historique possédant des
   `//!` doit devenir un vrai sous-module ;
5. logs fragmentés avec la fonctionnalité qu'ils observent ;
6. changement sémantique petit, borné et mesurable à chaque version.
