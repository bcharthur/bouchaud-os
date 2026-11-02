# Architecture vérifiée — 2026-11-02

```text
UEFI / bootimage
    └── Rust bare-metal x86_64
        ├── SMP / LAPIC / TSC-deadline / IPI / scheduler
        ├── VM / demand paging / process & thread / Linux compat
        ├── ABI native : handles, IPC, waitsets, shared memory, objects
        ├── network, storage, PCI, USB, security, GPU virtio 2D
        ├── GUI / WM / services encore partiellement dans noyau
        └── userland / services
              ├── composited (slice existante, pas preuve de migration totale)
              └── services Ladybird : BrowserHost, WebContent, RequestServer,
                  ImageDecoder, WebWorker, Compositor
```

## Scheduler — graphe de revue

```text
TASK BLOCKED
   └─ événement / attente / deadline
      └─ publication READY [état et ordre mémoire à contrôler]
         └─ file par CPU / classe de runqueue
            └─ IPI/reschedule éventuel
               └─ LAPIC/timer/CPU idle
                  └─ élection / rejet / remise en file
                     └─ commutation de contexte
                        └─ reprise / mesures de latence
```

Chemins constatés : `src/kernel/process/thread/ordonnancement.rs` (élection, remise des tâches rejetées, classes), `src/kernel/scheduler/runqueue` (file CPU), `src/arch/x86_64/smp.rs` (IPI, timer, AP), `src/kernel/process/thread/lifecycle.rs` (états et sortie), `src/kernel/process/thread/preemption.rs`. Pour les verrous/atomics précis, exiger une revue par fonction et les tests dédiés : **pas de garantie de linéarisation affirmée ici sans analyse exhaustive**.

Invariants à instrumenter : Ready sans CPU électeur ; Blocked sans source ; deadline effacée avant fin d'attente ; IPI sans réception ; `hlt` sans timer ; runqueue sans propriétaire ; migration bloquée. Commencer par les compteurs existants, sans refaire tout le scheduler.

## C5 — sûreté page fault (critère de revue)

Ordre de sûreté **attendu à confirmer dans l'ensemble des chemins** : allocation frame → remplissage complet → ordre/barrière adaptée → publication PTE → TLB/visibilité. Rollback et libération doivent être sûrs sous faults concurrents. Il existe des preuves et bancs C5 historiques, mais **VERIFIED_KVM actuel = UNKNOWN** faute de revue du chemin exact à ce SHA. Ne pas réécrire C5 sans contre-exemple.

## GPU P10+

Le code contient `src/drivers/display/virtio_gpu.rs` et `src/drivers/display/virtio_gpu_dma.rs`, avec des appels identifiables aux commandes `SET_SCANOUT`, transfert et réponses virtio, et des tests de trame via `tools/ci/run_virtio_gpu_scanout.sh` / `analyse_scanout.py`. **Plus avancé que GET_DISPLAY_INFO seul.** Le niveau de preuve complet de MSI-X/fences/double buffering et de la composition directe doit encore être établi par fonction ; virgl/Venus/Vulkan/WebGPU/AMD PHYSICAL = UNKNOWN, jamais « DONE » d'après l'existence de code.

## Stratégie ring3 (sans microkernel forcé)

Le premier candidat de démonstration est **un petit service d'observation/health non privilégié**, utilisant les handles/IPC natifs, plutôt que déplacer un pilote DMA/IRQ. Justifier isolation, coût IPC, reprise en cas de crash et frontières de permissions. `userland/services/composited/` contient un prototype à auditer ; ne pas prétendre que tous les services GUI sont déjà ring3.

## Dette du port Ladybird

Au relevé : 26 scripts `tools/ladybird/prepare-*.py` dans l'arbre. Parmi les plus volumineux : `prepare-compositor-lien.py`, `prepare-echange-processus.py`, `prepare-full-browser-host.py`, `prepare-m9-diagnostics.py`, `prepare-gc-retention-proof.py`. Risque : remplacements de chaînes dépendants des ancres upstream. Cible : SHA upstream épinglé + plate-forme Bouchaud + patch-stack minimale + instrumentation optionnelle. **Ne pas convertir mécaniquement les 26 scripts avant tests natifs.**
