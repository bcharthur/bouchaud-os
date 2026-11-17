# Audit technique complémentaire — 2026-11-02

Ce rapport complète [CLAUDE_NEXT_MISSION.md](CLAUDE_NEXT_MISSION.md). Il distingue **présence de code**, **preuve** et **expérience à mener**. Les références de code ont été lues sur `main=ba4244fea53434de84ce532bf9f6d7ae5a95342e`.

## Branches — relevé avant PR

| Branche | SHA | Décision |
|---|---|---|
| main | ba4244fe | KEEP, branche produit |
| chatgpt/active | 44fef2cb | KEEP, branche de travail |
| fix/nightly-ladybird-artifact-20261101 | ca4e8c04 | UNKNOWN : PR #231 fermée sans merge, contenu unique à comparer avant suppression |
| fix/proc-processus-snapshot-kvm-20261101 | 207717c3 | UNKNOWN : PR #232 fermée sans merge, contenu unique à comparer avant suppression |
| backup/main-before-c22-v29-20261102 | 788c5446 | KEEP temporaire, sauvegarde du fast-forward C22 |
| backup/main-before-branch-prune-20261102 | b8436617 | KEEP temporaire, sauvegarde CI avant nettoyage |

La branche `chatgpt/prep-claude-20261102` est une septième branche **temporaire de PR**, à supprimer après fusion, pas une nouvelle baseline. Un tag `archive/c22-v9-ci-forensic-20261102` conserve l'ancien diagnostic divergent. Aucun nettoyage supplémentaire sans preuve `merge-base/ahead/behind`.

## Scheduler — points d'entrée réels

| Transition / invariant | Fichiers et fonctions constatés | Expérience courte |
|---|---|---|
| Blocked → Ready et cible | `src/kernel/scheduler/reveil.rs` : `choisit_coeur`, `decide`, `demande_differee_posee` | tracer génération publication / CPU cible / décision |
| Ready → runqueue | `src/kernel/scheduler/runqueue.rs`, `src/kernel/process/thread/ordonnancement.rs` | détecter une mise en file non consommée ou perdue |
| CPU muet / sauvetage | `src/kernel/scheduler/secours.rs` : `coeur_muet`, `secours_permis`, `choisit_accueil`, `coeurs_muets` | corréler âge Ready / âge de l'occupant / CPU admissible |
| IPI / réveil des AP | `src/arch/x86_64/smp.rs` | comparer émission IPI, réception, élection et reprise |
| reprise / handoff | `src/kernel/process/thread/ordonnancement.rs`, `src/kernel/scheduler/continuation.rs` | ticket Ready → switch → reprise, avec timestamps |

Aucune race nouvelle n'a été **reproduite** dans ce relevé ; ces pistes ne justifient aucun correctif immédiat.

## C5 — revue du mécanisme demand paging

Le module principal de VMM est **`src/kernel/memory/virtual.rs`**, pas `src/kernel/memory/mod.rs` (ce dernier n'existe pas dans l'arbre constaté). On y trouve `alloc_frame_a_recouvrir`, `alloc_frame`, `free_frame`, les bits `PTE_PRESENT`, et le banc `attente_banc_c5`. L'ordre de publication sur tous les faults doit être relevé en partant de `src/kernel/process/` et des appels VMM. **Status VERIFIED_KVM C5 sur HEAD = UNKNOWN**.

Reproduction courte sans prétention de preuve : `git grep -n 'alloc_frame_a_recouvrir\|attente_banc_c5\|PTE_PRESENT\|banc-c5-' src tools Cargo.toml`. Validation ciblée existante : `.github/workflows/p13-cible-kvm.yml` (banc historique et corrigé). Demander des journaux de preuve exacts avant de réouvrir C5.

## P10+ GPU — pas seulement GET_DISPLAY_INFO

Le code existe dans `src/drivers/display/virtio_gpu.rs`, `virtio_gpu_dma.rs`, `virtio_gpu_file.rs`, `virtio_gpu_irq.rs`, `virtio_gpu_tampons.rs`, `virtio_gpu_trame.rs`. `virtio_gpu_irq.rs` documente MSI-X, index `used` et relecture ; `virtio_gpu_tampons.rs` possède `peut_ecrire`, `soumet`, `acheve`, `mode`, `a_rejouer`, `octets_copies`. Cela prouve une implémentation avec modèle d'ownership, **pas** une qualification matérielle.

Banc de 2D ciblé : `tools/ci/run_virtio_gpu_scanout.sh` et analyse `tools/ci/analyse_scanout.py`. Exiger une trace resource/backing/scanout/transfer/flush/réponse, y compris interruption vs polling et résultat de rollback. Virgl, Venus, Vulkan, WebGPU et GPU AMD : preuve PHYSICAL actuelle **UNKNOWN**.

## Ladybird — inventaire de risques

**25** fichiers `tools/ladybird/prepare-*.py` vérifiés dans le tree actuel. Gros scripts :
- `prepare-compositor-lien.py` (34 647 octets) : intégration/composition, ancres upstream à inventorier.
- `prepare-echange-processus.py` (30 698 octets) : cycle Page/Document/Request, correctif C22 V29 à préserver et tester à SHA upstream épinglé.
- `prepare-full-browser-host.py` (20 987 octets) : pont BrowserHost/runtime.
- `prepare-m9-diagnostics.py` (16 845 octets) : instrumentation M9 à isoler des modifications produit.
- `prepare-gc-retention-proof.py` (14 072 octets) : preuves et diagnostics GC.

Catégories provisoires à valider ligne par ligne : **portage durable**, **diagnostic temporaire**, **correctif Bouchaud**, **correctif upstreamable**, **obsolète**. Ne supprimer aucun `prepare` sur simple intuition.

## Branch protection — proposition solo viable

- Exiger PR pour `main`, refuser push direct / force push / suppression.
- Exiger les checks **toujours produits** `fast-gate`, `integration-gate`, `reliability-gate`; vérifier leurs noms réels dans les checks GitHub avant configuration.
- Checks IPC/security/browser : si déclenchés par chemins seulement, ne pas les rendre obligatoires universellement avant adoption d'un gate agrégateur.
- La qualification longue SMP8 reste un gate de **release/baseline**, pas nécessairement de chaque PR documentaire.
- Configurer ruleset et éventuelles exceptions administrateur volontairement, **jamais en silence depuis cette PR**.

## CI — inventaire compact des 17 familles conservées

| Famille | Workflows | Rôle |
|---|---|---|
| FAST | ci | gate court de PR/main |
| CORE | integration, reliability-v3, os-primitives, p13-primitives-noyau | noyau, KVM, matrice ; primitives P13 non strictes possibles |
| RUNTIME | native-ipc-runtime, security-runtime, userland | IPC, sécurité, legacy séparé |
| BROWSER | ladybird-native-browser, ladybird-sync | port native + provenance/épingle upstream |
| ENDURANCE | nightly, endurance, soak | validations différées/longues ; Nightly same-SHA exigé pour P13 |
| P13 ciblé | p13-cible-kvm, p13-dedicated-kvm, p13-final-candidate | qualification stricte et diagnostic localisé |
| RELEASE | release | artefacts + attestations |

Attention : `os-primitives` est à la fois manuel et réutilisable ; le supprimer rendrait Ladybird qualification et P13 final cassés.

## Artefacts et commandes

- C22 : [run 41518023294](https://github.com/bcharthur/bouchaud-os/actions/runs/41518023294), journal du job mémoire, preuve 6/6.
- P13 : `tools/ci/run_p13_campaigns.sh FULL_SHA` en hôte dédié KVM4 ; `tools/ci/run_p13_dedicated_kvm.sh BOOT smp8` et `tools/ci/p13_kvm_soak.py --minutes 30` sur KVM8.
- Contrôle court : `python3 tools/ci/check_pipeline_policy.py` et `python3 -m unittest discover -s tools/ci/reliability -p 'test_p13_final_gate.py' -v`.
- Vérification du snapshot : `python3 tools/ci/check_status_snapshot.py`. Le contrôle accepte le HEAD observé ou un descendant ne modifiant que `docs/current/**` et ce vérificateur ; toute modification runtime/CI hors de ce périmètre rend le snapshot obsolète (retour 2).

**Rapport à considérer comme audit STATIC et de CI, pas nouvelle preuve KVM/PHYSICAL.**