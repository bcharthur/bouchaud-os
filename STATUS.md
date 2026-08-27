# Bouchaud OS — Current Status

Dernière mise à jour : 27 août 2026

Ce document sépare le code présent, les validations hors cible et les résultats
observés sur la cible. La vision ne constitue jamais une preuve d'exécution.

## Legend

- ✅ **Proven** — test et exécution réelle associés à un checkpoint connu.
- 🟡 **Implemented / validation in progress** — code/tests présents, preuve runtime finale insuffisante.
- 🔵 **In progress** — chantier actif, sans checkpoint de clôture.
- ⚪ **Planned** — direction ou fonctionnalité non réalisée.

## Checkpoints de référence

| Checkpoint | Status | Evidence | Portée exacte |
|---|---:|---|---|
| Gate0 | ✅ | tag `gate0-complete-20260826`; commit `2ad9d39` | Trois boots QEMU SMP4 : 4 CPU actifs, WebContent/Ladybird, document chargé, stabilité observée, aucune panic BKL. `4aa9e76` fournit l'outil de résumé des logs, pas la preuve runtime principale. |
| Gate1A | ✅ | tag `gate1a-complete-20260827`; sparse damage `a70940d` | Plusieurs runs du damage sparse à capacité fixe, sans panic ni overflow observé. |
| Gate1B / Gate1C | 🔵 | event-driven `f69eaa1`/`17d098f`; culling `201f5ff`/`8691c3e`; audit `487658a` | Implémentation et tests progressent ; aucun tag runtime final ne clôt ces gates. |

## Kernel et exécution

| Area | Feature | Status | Evidence | Notes |
|---|---|---:|---|---|
| Platform | x86_64 bare metal sous QEMU | ✅ | Gate0; `targets/x86_64-bouchaud_os.json`; `src/arch/x86_64/` | Référence actuelle, pas une validation générale sur PC physiques. |
| Boot | Boot x86_64 / contrat de boot | ✅ | Gate0; `src/boot/`; `src/main.rs` | Boot répété dans le périmètre Gate0. |
| VM | Chemins pagination/VMA/shared memory utilisés par Ladybird | ✅ | jalons ring 3 M5–M13 dans `docs/ETAT_DES_LIEUX.md`; Gate0 | Ces chemins précis ont porté les processus et surfaces du workload validé. |
| VM | Robustesse générale pagination/VMA/shared memory | 🟡 | implémentation `src/kernel/memory/`; audits `MEMORY_FABRIC.md`, `VMA_ENGINE.md` | Couverture sous charges et géométries générales non établie par Gate0 ; optimisation/contention VM-BKL toujours travaillées. |
| Process | Processus, threads et ring 3 | ✅ | jalons M5–M13; `src/kernel/process/` | Utilisés par les services du checkpoint. |
| Exec | Chargement/exécution ELF64 | ✅ | jalons ring 3 M5–M13; `elf.rs`, `exec.rs` | La preuve concerne ELF, pas PE. |
| Exec | Parse PE32+ AMD64 | 🟡 | tests de module; `3c1423f`, `9d1f31c` | Analyse statique, aucune exécution `.exe`. |
| Exec | `PreparedImage` neutre | 🟡 | tests; `2a00701`; `loader/image.rs` | Description préparée, sans mapping ni transfert de contrôle. |
| Exec | Mapping/exécution PE | ⚪ | contrat `loader/mod.rs` | Projection, imports/runtime Windows et démarrage absents. |
| Syscalls | Sous-ensemble ABI requis par les jalons Ladybird | ✅ | témoins ring 3 M5–M13 dans `docs/ETAT_DES_LIEUX.md`; `SYSCALL_MATRIX.md` | Le workload prouve les appels effectivement exercés, pas toute l'ABI Linux. |
| Syscalls | Couverture générale de la compatibilité ABI Linux | 🟡 | inventaire `docs/linux-compat/SYSCALL_MATRIX.md`; dispatch `src/compat/linux/` | Routes présentes avec niveaux de support variables ; absence de preuve runtime exhaustive. Couche de compatibilité, pas architecture native finale. |
| SMP | Boot avec 4 CPU actifs | ✅ | Gate0 | Trois boots ; ni SMP8 ni matériel physique couverts. |
| Scheduler | Ordonnancement SMP4 du workload Gate0 | ✅ | tag `gate0-complete-20260826`; commit `2ad9d39` | Quatre CPU actifs pendant les trois boots et le workload Gate0. |
| Scheduler | Scheduler SMP/affinité, robustesse générale | 🟡 | `scheduler/core.rs`; `SMP_NG_FOUNDATION.md`; tests ciblés | Implémenté, mais Gate0 ne prouve ni toutes les affinités/charges, ni SMP8, ni l'endurance générale. |
| Scheduler | Work stealing | 🟡 | `SMP_ACCOUNTING_INVARIANTS.md`; `1d979f7` | Implémenté/instrumenté, sans checkpoint exhaustif autonome. |
| Sync | Big Kernel Lock | 🟡 | Gate0 sans panic; `ec6d307`, `34cad7f`; `BKL_LATENCY_AUDIT.md` | Toujours présent ; holds et poll/VM post-Gate1A en cours. |
| VM/SMP | TLB shootdown | 🟡 | `src/arch/x86_64/smp.rs`; `TLB_SHOOTDOWN.md` | Implémenté/mesuré, robustesse complète sous stress à valider. |
| Process | Signaux et timers | 🟡 | `signal.rs`, `timer.rs`; matrice syscall | Présents/utilisés, sans campagne autonome récente identifiée. |
| Waits | Wait queues et futex | 🟡 | `wait_queue.rs`, `thread.rs`; M5 | Utilisés ; couverture/contention générales non closes. |
| Waits | `poll` / `select` / `epoll` | 🟡 | self-test `exec.rs`; M5; `59b2575` | Réveil inter-thread ciblé prouvé ; travail poll/BKL actif. |

## GUI

| Area | Feature | Status | Evidence | Notes |
|---|---|---:|---|---|
| Display | Framebuffer/backbuffer logiciel | ✅ | Gate0; `framebuffer.rs`; `window_manager.rs` | Composition CPU ; aucun backend GPU exposé. |
| Damage | Sparse, capacité fixe | ✅ | Gate1A; `a70940d`; `degats.rs` | Aucun overflow observé, pas une preuve d'impossibilité. |
| Compositor | Réveil event-driven | 🟡 | `f69eaa1`, test `17d098f`, diagnostic `88f3208` | Code/test présents ; validation runtime/perf finale en cours. |
| Compositor | Scene culling | 🟡 | `201f5ff`; tests `8691c3e`, `2771d7c` | Équivalence testée ; Gate1B/1C non closes. |
| Damage | Transitions/diagnostics LFB | 🔵 | `384d541`, `12300cc`, audit `487658a` | Corrections récentes, campagne finale non checkpointée. |
| Desktop | Fenêtres, taskbar, menu, curseur | ✅ | Gate0; `window.rs`, `widgets.rs` | Fonctionnels dans le workload observé. |
| Fonts | Vectoriel via `fontdue` | 🟡 | `Cargo.toml`; `src/gui/font.rs` | Chemin primaire avec repli ; glyphes/scripts non validés exhaustivement. |
| Protocol | GUI protocol/clients ring 3 | ✅ | M8/M9; tests `protocole.rs`; `GUI_USERLAND_PROTOCOL.md` | Clients userland ; WM/compositeur reste noyau. |

## Réseau et applications

| Area | Feature | Status | Evidence | Notes |
|---|---|---:|---|---|
| Link | Ethernet/e1000, ARP, IPv4 | ✅ | M9; `e1000.rs`; `src/net/` | QEMU seulement. |
| Transport | TCP/sockets exercés par HTTP(S) et IPC Ladybird | ✅ | jalons M5, M9 et M12 dans `ETAT_DES_LIEUX.md` | Preuve limitée aux opérations et scénarios réellement exercés par ces témoins. |
| Transport | UDP exercé par DNS ring 3 | ✅ | jalon M13 dans `ETAT_DES_LIEUX.md` | Démultiplexage et attente DNS prouvés par la sonde ciblée sous QEMU. |
| Transport | Couverture générale TCP/UDP/sockets | 🟡 | implémentation `src/net/transport/`; compat `src/compat/linux/net.rs` | Variantes, erreurs, charge et endurance globales ne sont pas couvertes par les seuls jalons. |
| Configuration | DHCP | 🟡 | `src/net/application/dhcp.rs` | Implémenté, sans preuve récente distincte identifiée. |
| Naming | DNS | ✅ | M13 dans `ETAT_DES_LIEUX.md` | Sonde ring 3 et navigation publique par nom documentées. |
| Web | HTTP et HTTPS/TLS | ✅ | M9/M12 dans `ETAT_DES_LIEUX.md` | Fixtures/chemin public, pas tous les sites. |
| Apps | Terminal, Calculatrice, Fichiers, Rustpad | 🟡 | `src/gui/apps/`; `src/gui/window.rs` | Implémentées, chacune reste à qualifier par le DoD 0.1. |
| Browser | WebContent/Ladybird en fenêtre | ✅ | Gate0; M8–M13 | Moteur/services portés et document chargé ; pas frontend complet. |
| Browser | Chrome Bouchaud/input/scroll | 🟡 | `M11_NAVIGATEUR.md`; Gate0 | Utilisable dans ce périmètre ; endurance/compatibilité limitées. |
| Browser | Frontend Ladybird complet | ⚪ | `FRONTEND_BOUCHAUD.md` | Services, sandbox, onglets et intégration complète non acquis. |

## Multiplateforme

| Area | Feature | Status | Evidence | Notes |
|---|---|---:|---|---|
| Structure | Séparation arch/platform/drivers | 🟡 | `0b3eb17`; `MULTIPLATFORM_FOUNDATION.md` | Fondation ; migration des dépendances historiques progressive. |
| AArch64 | Façade/structure de portage | 🟡 | `src/arch/aarch64/`; `platform/qemu_virt/` | Structure, aucun backend bootable prouvé. |
| Raspberry Pi | Plateforme Pi | ⚪ | `platform/raspberry_pi/`; `AARCH64_RASPBERRY.md` | Squelette/vision, aucun boot revendiqué. |

## Limites importantes

- La preuve principale est QEMU x86_64 SMP4 ; elle ne généralise ni à tous les
  PC, ni à SMP8, ni à AArch64.
- Gate0 et Gate1A sont clos. Event-driven, culling, transitions, LFB, BKL/VM et
  poll/BKL restent 🟡 ou 🔵 faute de checkpoint runtime final.
- Sandbox Ladybird complète, GPU, exécution PE et window server ring 3 ne sont
  pas acquis.
