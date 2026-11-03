# Mise à jour atomique, santé, rollback et recovery

**État (2026-11-03, branche `claude/atomic-platform-20261103`) : chaîne A/B complète prouvée sous QEMU/OVMF (TCG, NVMe), en local ET sur les runners hébergés (matrice `ab-rollback`, 11 scénarios). Signature des versions : NON vérifiée sur la machine. PHYSICAL : non démontré.**

## Architecture

```text
UEFI ─▶ BOOTX64.EFI = Bouchaud Loader (shim)            tools/reference/uefi-preboot-probe
          │ lit META (2 copies) ─ choisit_boot ─ consomme l'essai ─ ÉCRIT ─ FLUSH ─ RELIT
          │ compteur d'échecs du slot actif (bloc META 2) ─ touche R ─ RECOVERY
          │ dépose le PASSAGE (page mémoire type OS-loader 0x8000B0AB)
          ▼
        partition SYS-A | SYS-B | RECOVERY  →  bootloader 0.11 → noyau (+ ramdisk du slot)
          ▼
        noyau : kernel::update = MÉCANISME (aucune politique)
          │  passage → slot réellement booté, META identifiée par GUID unique
          │  capacités Update (objet natif, jamais créé par un appel système)
          ▼
        serviced system (ring 3)  ── santé : healthd + updated + logd Running 3 s ──▶ bo_update_commit_health
          ├─ healthd            (capacité : aucune)
          ├─ updated            (INSPECT|UPDATE_INSTALL : slot INACTIF seulement)
          └─ logd               (événements structurés, file bornée)
          ▼
        bureau / Ladybird  (aucune décision de mise à jour n'en dépend)
```

La **politique** (candidat, bon, mauvais, essais, rollback, échecs répétés, commit de santé) est dans `system/update-core` (`no_std`, sans allocation, sans dépendance noyau), partagée par le chargeur, le noyau, le constructeur d'images et les tests hôte. Elle n'est **pas** une responsabilité du noyau.

## Disposition disque (GPT, `disposition.rs::plan_ab` + `uefi-image-builder --disque-ab`)

| Partition | Type GUID | Contenu |
|---|---|---|
| BOUCHAUD-ESP | EFI | le shim seul (`BOOTX64.EFI`) |
| BOUCHAUD-META | `B0DC8A5D-0002-…` | 1 Mio brut : copie 0 (bloc 0), copie 1 (bloc 1), compteur d'échecs (bloc 2) |
| BOUCHAUD-SYS-A / SYS-B | `…-0003-` / `…-0004-` | FAT : `BOUCHAUD-LOADER.EFI`, noyau, `boot.json`, ramdisk (userland système) |
| BOUCHAUD-RECOVERY (option) | `…-0005-` | FAT : système minimal (même format qu'un slot) |
| STATE | `…-0001-` | persistance utilisateur (`fs::persistance`) |

Taille d'un slot : charge utile + `ESP_MARGE_MIO`, plancher `ESP_MINIMUM_MIO`. STATE ≥ 4 × la zone de persistance. Le chargeur et le noyau trouvent les partitions **par type GPT sur le disque de démarrage**, jamais par numéro.

## Contrats et invariants

- **I1–I5** (update-core) : actif toujours Bon ; essais bornés ; jamais de boot d'un slot Vide/Mauvais ; deux copies, la plus ancienne réécrite ; aucune installation sur l'actif.
- **I6** : `running_slot != update_target` (`commence_installation(cible, en_cours)`).
- **Chargeur** : un candidat n'est booté que si son essai consommé est durable (écriture, flush, relecture décodée) ; sinon l'actif boote. Deux copies invalides → RECOVERY (ou arrêt si pas de partition RECOVERY) ; jamais un boot deviné.
- **Passage** : le noyau ne déduit PAS le slot courant de META (faux dans le cas du repli) ; il lit le passage (slot, raison, essais restants, séquence, GUID unique de META, CRC).
- **Commit de santé** : seul le slot booté comme candidat consommé, sur l'enregistrement exact d'où il a booté, est promu ; confirmer l'actif déjà Bon n'écrit rien (même si une installation a réécrit META entre-temps) ; un boot RECOVERY ne confirme jamais rien. Écriture : copie la plus ancienne, **vidange réelle exigée**, relecture comparée.
- **Échecs répétés** : 3 boots consécutifs de l'actif sans santé confirmée → RECOVERY. Un rollback remet le compteur à 0 ; un compteur illisible vaut 0.
- **Installation** (`updated`) : session liée au pid qui a fait `begin` ; écritures alignées bornées à la partition du slot cible ; relecture DEPUIS LE DISQUE par morceaux (`verify`) ; `finish` exige toute la relecture et une empreinte SHA-256 égale ; refus = aucun candidat.

## Preuves

| Verdict | Niveau | Preuve |
|---|---|---|
| UPDATE_AB_CORE | HOST | `tools/update/test_ab.rs` (coupure à chaque octet, exploration), `test_durabilite.rs` (cache volatil, réordonnancement, secteurs déchirés, 16 frontières, mutations), `test_passage.rs` (passage, commit, échecs répétés, recovery) — CI Fast |
| LOADER_AB | TCG | `run_ab_rollback.sh` : `panique`/`gel` `B B B A A`, `meta-0`/`meta-1` (copie survivante), `meta-2` (arrêt sans RECOVERY) |
| BOOT_ATTEMPT_PERSISTENCE | HOST + TCG | mutation « boot avant flush » détectée (hôte) ; chaque boot QEMU est un SIGKILL sur le même disque |
| HEALTH_COMMIT | TCG | `sante` : `B+ B=` (actif=B, B=Bon relu par la capacité) |
| AUTO_ROLLBACK | TCG | `panique`, `gel`, `sante-ko` (`B- B- B- A= A=`) |
| UPDATED_RING3 | TCG | `mise-a-jour` : `A* B+ B=` — B mis à zéro, écrit par `updated`, relu, Candidat{3}, booté, confirmé |
| UPDATE_CAPABILITIES | TCG | 16 refus/contrôles observés en ring 3 (écriture avant begin, slot en cours, santé via capacité d'installation, dup, handles étrangers, table = 3 handles, `/dev/nvme0n1`/`/dev/sda` absents, hors slot, non aligné, finish avant relecture, empreinte fausse) |
| SIGNED_MANIFEST | **HASH VALID seulement** | le noyau recalcule le SHA-256 sur le disque ; **SIGNATURE TRUSTED : NOT_STARTED** sur la machine (les attestations Sigstore de `release.yml` restent une provenance vérifiée hors machine) |
| RECOVERY_BOOT | TCG | `recovery-requise` (`R`), `echecs-repetes` (`A- A- A- R`) ; aucun superviseur ni bureau |
| RECOVERY_ROLLBACK | TCG | `recovery-rollback` (`R B=`) : touche R, `verifie A/B` conformes, rollback refusé sans `--confirme`, puis fait |

Commande unique : `tools/ci/run_ab_rollback.sh <scénario>` (workflow `native-ipc-runtime`, job `ab-rollback`).

## Boîte noire par boot (§28)

Une ligne par boot, écrite par le noyau dès que le stockage est prêt :

```text
BOUCHAUD_BOOT_RECORD boot_id=0x… system_slot=A|B system_version=<8 premiers octets du SHA-256 que META enregistre pour ce slot>
                     boot_reason=… candidate_attempts_left=N previous_boot_result=… meta_sequence=N
```

`previous_boot_result` ne dit que ce que les enregistrements prouvent : `candidat_epuise`, `candidat_en_essai`, `non_confirme` / `confirme` (compteur d'échecs du slot actif), sinon `inconnu`. `logd` estampille chaque événement avec ce `boot_id` (transmis par serviced dès le `hello`, avant tout événement).

## Recovery (`kernel::update::recovery`)

Console texte (framebuffer + COM1), entrée clavier ou série. Lecture seule par défaut : `etat`, `manifestes`, `verifie A|B`, `boite-noire`, `panique`, `redemarre`, `arrete`. Écritures nommées, `--confirme` obligatoire : `rollback`, `remet-compteur`. `panique` dit honnêtement que la dernière panique n'est pas disponible ici (la boîte noire persistante est sur la clé USB, `tools/reference/extract-blackbox.py`).

## Limites connues

- Signature des versions non vérifiée sur la machine (seule l'empreinte l'est) ; clés de développement : aucune n'est commitée.
- `verifie` mesure le volume FAT (taille du BPB), pas un manifeste signé.
- Le commit exige un volume à vidange réelle : un NVMe sans cache volatil (VWC=0) est refusé (sûr, mais à assouplir en distinguant « pas de cache » de « inconnu »).
- META et slots doivent être sur un volume que le noyau sait piloter (NVMe interne, USB, ATA) ; prouvé sous QEMU NVMe seulement.
- Image Trigkey : le ramdisk Ladybird doit embarquer `serviced`, `healthd`, `updated`, `logd`, sinon tout candidat est retiré (comportement voulu, non encore intégré à `build_trigkey_image.sh`).
- PHYSICAL : protocole dans [TRIGKEY_CAMPAIGN.md](TRIGKEY_CAMPAIGN.md), non exécuté.
