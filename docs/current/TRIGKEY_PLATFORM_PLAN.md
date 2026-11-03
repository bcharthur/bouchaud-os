# Trigkey Speed S5 — plan de qualification physique de l'Atomic Platform

**Statut : PHYSICAL = NOT_STARTED.** Ce plan dit quoi construire et comment le prouver. Aucune ligne ne vaut preuve tant qu'un journal réel de la Trigkey, rattaché au `manifest.json` de l'image, ne la porte pas.

## 0. Ce qui est établi avant le premier branchement (QEMU/hôte, 03/11)

| Fait | Source | Conséquence |
|---|---|---|
| La chaîne A/B est prouvée sur **NVMe** QEMU seulement | `run_ab_rollback.sh` : `-device nvme` | rien n'est prouvé sur un autre support |
| Sur **clé USB** (QEMU xHCI + usb-storage), le slot boote mais la santé n'est jamais commise : `B- B-` | essai local du 03/11, `SERVICED_HEALTH_COMMIT rc=-5`, `SERVICED_UPDATE_STATE rc=-13` | le protocole AB1–AB8 « flash sur clé USB » ne peut pas passer |
| Cause : le pilote USB déclare `vidange_reelle: false` ; `kernel::update` refuse une META sur un volume sans vidange réelle (`volume_sans_vidange_reelle`) | `xhci_active.rs:5702`, `update/mod.rs:137` | refus **correct** (un commit non vidé n'est pas durable) ; il manque SYNCHRONIZE CACHE |
| L'image Trigkey actuelle est mono-partition, sans `serviced/healthd/updated/logd` | `build_trigkey_image.sh` | aucun commit de santé possible |
| Le NVMe interne (Windows) n'est écrit qu'à travers une partition au GUID de type Bouchaud | `trigkey.rs`, `installation.rs` | barrière à **vérifier** sur la machine, en lecture seule |
| CPU de référence contradictoire (5700U déclaré, 5800H relevé) | `TRIGKEY_REFERENCE.md` | l'identification matérielle est l'étape 1, pas une hypothèse |

## 1. Incréments

### T1 — Image physique A/B réelle + vidange USB (premier incrément)

But : une image unique, traçable, qui porte la vraie Atomic Platform et peut être démarrée **depuis une clé USB** sans toucher au NVMe interne.

1. **Vidange réelle USB** : SYNCHRONIZE CACHE(10) (opcode 0x35) existe déjà pour la boîte noire (`blackbox_storage.rs:612`) mais n'est pas branché sur `Genre::Vidange` du volume générique (`xhci_active.rs`, qui rend `Fait(0)` sans rien émettre) ; le brancher ; `vidange_reelle = true` seulement si la commande aboutit (CSW status 0) au montage, sinon `false` et la META reste refusée. Refus conservé pour un support qui ne la prend pas.
   - Preuve QEMU : `run_ab_rollback.sh` paramétré `BO_AB_SUPPORT=usb` (xHCI + usb-storage), scénarios `sante`, `mise-a-jour`, `mise-a-jour-morts`, `recovery-rollback`, `echecs-repetes` verts sur USB **et** NVMe ; témoin négatif : support USB qui refuse SYNCHRONIZE CACHE → `volume_sans_vidange_reelle`, aucun commit.
2. **`tools/release/build_trigkey_ab_image.sh`** : même chaîne que le banc (`uefi-image-builder --disque-ab`), shim, META `a-bon`, slot A = noyau + ramdisk (`serviced`, `healthd`, `updated`, `logd`, archive Ladybird optionnelle), slot B vide, partition RECOVERY, manifeste traçable (commit, arbre propre, empreintes de chaque partition, `proof_level: BUILD_ONLY`). Clé de version : **dev** uniquement, sous `banc-cle-dev`, jamais dans une image marquée release.
3. **Paquet de mise à jour hors image** : slot B + manifeste signé (clé dev de la campagne), déposé dans `/update/` de A — pour AB4 sur la machine.
4. **Garde** : `verifie-image-trigkey-ab.py` — l'image contient les quatre services, la partition RECOVERY, une META valide ; aucune clé privée ; `manifest.json` cohérent.
5. **Doc** : `TRIGKEY_CAMPAIGN.md` §2bis corrigé (support = clé USB avec vidange réelle prouvée, ou disque externe ; jamais le NVMe interne).

Sortie de T1 : `HOST + TCG` sur USB, image prête. **Toujours PHYSICAL = NOT_STARTED.**

### T2 — Identification et inventaire (première session physique, lecture seule)

- Windows, avant tout : `tools/ci/collecte-trigkey.ps1` (lecture seule) → `trigkey-inventaire.json`.
- Boot de l'image T1 sur clé : `BOUCHAUD_HWPROBE_CPUID`, liste PCI (vendor/device/class), GOP, RAM, identité NVMe/xHCI/RTL8168/HDA/iGPU → nouvelle source de vérité dans `TRIGKEY_REFERENCE.md`.
- NVMe interne : `disktest` en **lecture** seule ; preuve que `Volume::DONNEES` n'existe pas (pas de GUID Bouchaud) et que toute écriture est refusée.

### T3 — Démarrage, services, stockage, pannes

AB1 (boot A, `SERVICED_HEALTH_COMMIT … already_good`), services natifs supervisés (crash/hang injectés → redémarrage, isolation), persistance sur la clé (fichier relu après reboot), coupure secteur pendant écriture `/persist`, 10 reboots consécutifs.

### T4 — Mise à jour A/B réelle et rollback

AB3/AB4 (installation sur le slot inactif, relecture disque, `Candidate`, reboot, commit de santé), AB2 (candidat qui panique → rollback après 3 essais), AB8 (coupure secteur pendant l'écriture du slot), mesures `UPDATED_MESURE` sur la vraie clé.

### T5 — Recovery

AB5 (touche R), AB6 (`verifie A/B`, `rollback --confirme`), AB7 (META corrompue → recovery, aucun boot deviné). Données de la clé préservées hors slots ; NVMe interne intact (empreinte avant/après depuis Windows).

### Séparé — P13 strict

Runners `bouchaud-kvm4` / `bouchaud-kvm8` (action humaine, `provision_kvm_runner.sh`), puis `p13-final-candidate.yml`. Indépendant du Trigkey : aucune preuve physique ne remplace P13 strict, et inversement.

## 2. Outils côté PC (à fournir avec T1)

| Outil | Rôle |
|---|---|
| `collecte-trigkey.ps1` (existe) | inventaire Windows en lecture seule |
| `flash-cle-trigkey.ps1` (à écrire) | écrit l'image sur un disque **amovible** désigné par son numéro ET sa taille ; refuse tout disque système/non amovible ; vérifie le SHA-256 relu |
| capture série | adaptateur USB-série si la carte en expose un ; sinon boîte noire sur la clé |
| `extract-blackbox.py` / `symbolise-blackbox.py` (existent) | lecture de la boîte noire, symbolisation avec le `kernel.elf` du manifeste |
| `trigkey_evidence.py` (existe) | contrôle des marqueurs réels ; ne valide jamais un champ vide |
| rapport | copie datée de `TRIGKEY_CRITERIA_EXAMPLE.json` : PASS/FAIL/NOT_RUN par étape + preuve |

## 3. Règles

- Une campagne = un `manifest.json`. Jamais deux images mélangées.
- Jamais le NVMe interne comme cible de flash ou d'A/B.
- Machine figée sans panique = FAIL. Étape non jouée = NOT_RUN.
- Aucune preuve QEMU ne remplit une case PHYSICAL.
