# CLAUDE — MISSION IMMÉDIATE (2026-11-02)

**Lire ce fichier seulement pour démarrer.** Aucun chantier spéculatif, aucun refactor de confort, aucun seuil ou quorum diminué.

## A. HEAD et source

- `main` observé avant cette PR : `ba4244fea53434de84ce532bf9f6d7ae5a95342e`
- Ladybird upstream **pinned** : `cdfe5f858eb5fc64a8d9d3fcc247d71b03fbd1f6` (provenance confirmée dans le build V29 ; à revérifier après pull).
- Date : 2026-11-02. **Refais `git rev-parse HEAD`** : cette note est datée.
- Branche préparation : `chatgpt/prep-claude-20261102` (docs + scripts, pas runtime).
- `main` non protégée au relevé : **P0 processus**, ne jamais pousser directement avant durcissement.

## B. Baseline réellement démontrée

```text
FAST=PASS sur 5c3cd43a et b8436617; IN_PROGRESS sur ba4244fe au relevé
RUNTIME=PASS Native IPC 41530447729, Security Runtime 41530447803, sur 5c3cd43a
BROWSER=PASS developpement sur 5c3cd43a / run 41518023294; qualification complète UNKNOWN
C22=CLOSED_AT_KVM sur 5c3cd43a / 41518023294 / 6x6, 144..164 KiB < 1024 KiB
P13=PARTIAL (pas de P13_FINAL_STRICT_OK same-SHA)
PHYSICAL=UNKNOWN sur HEAD actuel
NIGHTLY=UNKNOWN sur HEAD actuel; ancien main rouge 41424125876
```

## C. Véritablement fermé

- Correctif C22 V29 intégré à main le 02/11, contrat produit 6/6 KVM validé sur SHA précis `5c3cd43a7a2a1c258388f9a81967abecdce51068`. Deux séries locales 6/6 supplémentaires signalées.
- 11 workflows C22 obsolètes retirés du code puis désactivés ; branches expérimentales C22 anciennes supprimées avec tag d'archive `archive/c22-v9-ci-forensic-20261102`.
- **Ne PAS rouvrir le témoin négatif intermittent comme gate bloquant.**

## D. Bloquants réellement ouverts (maximum 10)

1. `main` non protégée : risque de push non qualifié.
2. P13 : absence de gate `P13_FINAL_STRICT_OK` sur SHA unique et hôtes KVM4/KVM8.
3. Nightly Full : aucun succès confirmé sur le HEAD courant.
4. Tests PHYSICAL Trigkey actuels et stockage résilient non démontrés.
5. Plusieurs workflows/sources historiques restent lisibles mais pas normatifs.
6. Cohérence du noyau/ABI/service ring3 à éprouver avant migration importante.
7. Qualification GPU virtio 2D avancée vs GPU PHYSICAL non démontrée.

## E. Ordre des cinq chantiers pour Claude

### 1 — P13 et baseline finale (IMPACT P0, changement minimum)

- **Symptôme :** anciens rapports P13 contradictoires et absence de `P13_FINAL_STRICT_OK`.
- **Cause connue :** preuves KVM4, KVM8, Nightly et primitives pas consolidées sur même SHA ; des échecs anciens SMP4 / réveil / console existent.
- **Ne pas refaire :** enquête C22 V20–V29 ; restructuration du scheduler ; abaissement des seuils ; campagnes concurrentes sans runners.
- **Code :** `.github/workflows/p13-final-candidate.yml`, `tools/ci/p13_final_gate.py`, `tools/ci/run_p13_campaigns.sh`, `p13_kvm_host_preflight.py`, `src/kernel/process/thread/ordonnancement.rs`, `src/arch/x86_64/smp.rs`.
- **Local court :** `python3 -m unittest discover -s tools/ci/reliability -p 'test_p13_final_gate.py' -v`.
- **KVM long (seulement hôte dédié prêt) :** `BOUCHAUD_DEDICATED_HOST=YES bash tools/ci/run_p13_campaigns.sh "$(git rev-parse HEAD)"`.
- **CI :** dispatch `p13-final-candidate.yml` après Nightly+Ladybird qualifiés. **Sortie :** `P13_FINAL_STRICT_OK` même SHA ; sinon publier UNE cause et sa reproduction.

### 2 — C5 sûreté demand paging (P0, revue ciblée sans réécriture)

- **Hypothèse :** publication PTE peut être observée avant que la frame soit remplie dans un chemin concurrent ; ne présume pas qu'elle l'est.
- **Code :** `src/kernel/memory/`, `src/kernel/process/`, sondes `tools/ci/run_*c5*.sh` (localiser avec `git grep -n 'C5\|publication.*PTE'`).
- **Court :** `git grep -n 'C5\|PTE' src/kernel tools/ci | head -100`.
- **CI :** `p13-cible-kvm.yml` pour témoin/candidat ; avant exécution vérifier le runner et le SHA.
- **Vert :** allocation→copie→barrière→PTE, aucun frame doublement libéré ; preuves KVM confirmées. **Rouge :** UAF, frame partagée avant copie, fault concurrent non sérialisé.
- **Ne pas refaire :** correctif C5 sur intuition ; se limiter à un contre-exemple concret.

### 3 — Première frontière ABI native/ring3 (P1, expérience unique)

- **Symptôme :** services GUI et logiques de runtime encore dans le noyau.
- **Hypothèse :** service d'observation non privilégié isolable sans DMA, IRQ ni coût IPC critique.
- **Code :** `userland/services/composited/`, `src/kernel/native/`, `src/gui/`.
- **Court :** `git grep -n 'waitset\|HandleId\|shm' userland/services src/kernel/native | head -100`.
- **Vert :** service récupérable après crash, autorisations minimales et IPC mesuré, sans régression.
- **Ne pas refaire :** « transformer l'OS en microkernel » ou déplacer `composited` entier d'un coup.

### 4 — GPU P10+ état 2D et preuve (P1)

- **Symptôme :** docs sous-décrivent transport virtio, ressources, scanout/flush.
- **Code :** `src/drivers/display/virtio_gpu.rs`, `virtio_gpu_dma.rs`, `tools/ci/run_virtio_gpu_scanout.sh`, `docs/ladybird/P10_GPU_AUDIT.md`.
- **Court :** `git grep -n 'RESOURCE_CREATE_2D\|SET_SCANOUT\|TRANSFER_TO_HOST_2D\|RESOURCE_FLUSH' src/drivers/display`.
- **Vert :** repro KVM/TCG 2D avec métriques, ownership et IRQ/fence ; ne pas proclamer Vulkan/AMD avant PHYSICAL.
- **Ne pas refaire :** GPU 3D complet avant avoir figé la baseline.

### 5 — Dette Ladybird et mise à jour (P1)

- **Symptôme :** 26 `prepare-*.py`, ancres texte fragiles.
- **Code :** `tools/ladybird/prepare-echange-processus.py`, `prepare-compositor-lien.py`, `prepare-full-browser-host.py`, `prepare-gc-retention-proof.py`.
- **Court :** `find tools/ladybird -name 'prepare-*.py' | sort`.
- **Vert :** matrice ancres/fichiers/symboles/catégories, une conversion de patch isolée seulement si preuve d'équivalence, sans changement d'upstream au même moment.
- **Ne pas refaire :** remplacer la forêt Python par une réécriture large.

## F. Pièges à éviter

- Une PR peut être validée sur **merge SHA** ; vérifier le SHA de source et celui de l'artefact.
- C22 témoin historique non reproductible : absence de rouge témoin ≠ fuite produit.
- `P13_STRICT=0` ne qualifie pas P13. `NON_QUALIFIANT` reste un échec de preuve.
- TCG ≠ KVM ≠ Trigkey. Dossier physique obligatoire avant promesse daily-driver.
- Cache et manifeste ne sont pas des preuves d'exécution si les jobs ont été skipped.
- Ne pas merger un document daté comme preuve d'un CI plus récent.

## G. Première action Claude — copier-coller

```bash
git fetch origin main
git switch main && git pull --ff-only origin main
git rev-parse HEAD
python3 tools/ci/check_pipeline_policy.py
python3 -m unittest discover -s tools/ci/reliability -p 'test_p13_final_gate.py' -v
git grep -n 'BOUCHAUD_C22_V29\|P13_FINAL_STRICT_OK' tools .github/workflows
```

Puis ouvrir `docs/current/CI_AND_PROOFS.md` et le run `41518023294`. **Première modification runtime seulement après un résultat rouge attribué à un fichier et une expérience qui le reproduit.**
