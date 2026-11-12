# Trigkey : installation native et mise à jour automatique — état réel

État au 2026-11-12. Cible : Trigkey Speed S5 (Ryzen 7 5700U, 32 Go, NVMe, RTL8168).

**Aucune installation ni mise à jour n'a été faite sur le Trigkey réel.**
Toutes les preuves ci-dessous sont **HOST** (tests sur la machine de construction) ou **QEMU TCG/OVMF** (CI GitHub).
Aucune ne vaut preuve **PHYSICAL**.

Aucun candidat installable n'est qualifié aujourd'hui. Il manque :

- une clé de production engagée ;
- un slot complet (Ladybird, DOOM, bureau) ;
- un agent de mise à jour réseau ;
- un installateur A/B audité.

Niveaux de preuve, à ne jamais confondre : HOST, QEMU TCG, QEMU KVM, OVMF, VIRGL, PHYSICAL.

## État par phase

| Phase | Objet | État | Preuve |
|---|---|---|---|
| 0 | Fusion #300/#301/#302/#304, main vérifiée | **FAIT** | CI au SHA exact |
| 0 | « Unix & Filesystem Reliability » en check requis | **ACTION HUMAINE** | API 403 (droits admin) |
| 1 | Disposition ESP / META / SYS-A / SYS-B / RECOVERY | **FAIT** (image) | QEMU TCG/OVMF (`trigkey-ab-preflight`, `run_ab_rollback.sh`) |
| 1 | Partition STATE | **FAIT** (image) | QEMU TCG/OVMF (`run_state_ab.sh`) |
| 2 | Installateur SSD dédié (hôte Linux) | **FAIT**, jamais lancé sur un vrai disque | HOST (13 tests) + QEMU TCG/OVMF (`run_installe_ssd_qemu.sh`) |
| 3 | Image physique unifiée | **PARTIEL** | voir ci-dessous |
| 4 | Release signée, canaux, promotion | **FAIT (outillage)**, bloqué par la clé | HOST (31 tests, dont témoins négatifs) + QEMU TCG (banc A/B) |
| 5 | Agent de mise à jour réseau | **NON FAIT** | — |
| 6 | Ed25519, anti-retour, refus sans clé | **FAIT** | HOST + QEMU TCG |
| 6 | Rotation et révocation de clé | **PROCÉDURE** | aucun banc |
| 7 | Rollback, recovery | **FAIT** | QEMU TCG (`run_ab_rollback.sh`, 11 scénarios) |
| 8 | Persistance à travers coupures et changement de slot | **FAIT** | QEMU TCG/OVMF (`run_state_ab.sh` : A, A, B ; compteur 1, 2, 3) |
| 9 | Réseau RTL8168, DHCP, DNS, TLS, horloge | **BLOCKED_PHYSICAL** | QEMU seulement (autres NIC) |
| 10 | `bo-update` | **NON FAIT** | — |
| 11 | Qualification physique | **BLOCKED_PHYSICAL** | procédures ci-dessous |
| 12 | Scripts Windows | **NON FAIT** | procédure manuelle ci-dessous |

Détail des lignes marquées « voir ci-dessous » :

- **Partition STATE (phase 1, `BOUCHAUD_STATE_AB_V1`).**
  - `build_trigkey_ab_image.sh` pose une 6e partition `BOUCHAUD-STATE` (256 Mio dans l'image, type `B0DC8A5D-0001`), la dernière du disque, à zéro.
  - Le noyau la monte comme volume persistant (`installation::monte_le_systeme_installe`) : `/persist` vit dessus. Une mise à jour réécrit un slot, jamais STATE.
  - Chaque démarrage d'un système installé incrémente `/persist/.bouchaud/demarrages`, puis synchronise et journalise `BOUCHAUD_PERSIST_DEMARRAGES n=… slot=…`. C'est le témoin du banc.
  - **Montage sain (`BOUCHAUD_MONTAGE_SAIN_V1`, revue #307).** Le montage distingue :
    - volume **neuf** : les deux superblocs entièrement nuls ;
    - **restauration complète** ;
    - **échec** : zone ou table illisible, table incohérente, superblocs abîmés mais non nuls, ou un seul fichier non relu.

    Après un échec, ni le compteur ni aucune synchronisation n'écrivent : `persistance::synchronise` rend `REFUS_MONTAGE_INCOMPLET` pour tout appelant, et le disque reste intact pour une réparation. Le journal le dit : `BOUCHAUD_PERSIST_MONTAGE etat=echec raison=… consequence=lecture_seule`.
  - **Défaut trouvé par ce banc et corrigé.** `persistance::monte()` tenait le verrou tournant du RAMFS pendant ses lectures disque. Sur NVMe, une lecture cède le processeur, et LOCKDEP paniquait (`held_rank=50 … acquiring=scheduler-transition`) au premier montage. Le disque est désormais lu sans verrou, et le dépôt dans le RAMFS se fait sous verrou, sans E/S. Ce défaut n'apparaissait qu'avec une persistance sur NVMe, que rien n'exerçait jusque-là.
- **Installateur SSD dédié (phase 2, `BOUCHAUD_INSTALL_SSD_V1`).**
  - `tools/install/installe_ssd.py` s'exécute **sur un hôte Linux** : le SSD dédié y est branché (adaptateur USB-NVMe ou baie) avant d'être monté dans le Trigkey.
  - `liste` montre les disques. `installe` exige la cible **et** son identité exacte `modèle|série|octets`, lue dans `/sys/block`. Il ne choisit jamais de disque.
  - Le mode par défaut est un dry-run. `--execute` demande en plus la phrase tapée `EFFACER <SÉRIE>`.
  - Refus :
    - une partition (et non le disque entier) ;
    - un disque utilisé **directement ou indirectement** (revue #307). Les dépendances de stockage sont remontées dans sysfs (`holders`/`slaves`, récursivement) depuis chaque système monté et chaque swap. Cela couvre LVM, dm-crypt, RAID md, tout device-mapper et les membres btrfs. Une pile ouverte mais non montée suffit à refuser (`detenu_par`). Une source `/dev/…` non résoluble, comme `/dev/root`, est refusée : en cas d'ambiguïté, on refuse ;
    - un numéro de série illisible ;
    - une image altérée ou plus grande que le disque ;
    - une image dont la dernière partition n'est pas STATE.
  - Après écriture : `fsync`, puis **relecture intégrale** comparée à l'empreinte. STATE est ensuite étendue jusqu'à la fin du disque : GPT de secours écrite avant la primaire, CRC recalculés, relecture des deux.
  - Sa **zone persistante finale** (les 128 derniers Mio, là où le noyau la cherche) et son premier Mio sont ensuite **mis à zéro, synchronisés et relus**. Rien d'une installation précédente ne peut réapparaître (revue #307).
  - Banc `run_installe_ssd_qemu.sh`, sur un fichier de 2 Gio, 6 démarrages QEMU :
    1. Un témoin vérifie qu'un refus de confirmation n'écrit rien.
    2. Installation, puis compteur 1 puis 2.
    3. **Réinstallation sur ce SSD déjà utilisé**, puis compteur 1 (volume neuf) et 2 : rien de l'installation précédente ne réapparaît.
    4. **Les deux superblocs abîmés** : montage `echec`, aucun compteur, zone **identique octet pour octet** après le démarrage.
    5. **Réparation** : compteur 3, l'état d'avant est intact.
  - La commande noyau historique `installer --go [--ecrase]` (ESP + SYSTEME, sans A/B, sans identité) **ne doit pas servir** pour le Trigkey.
  - **Jamais lancé sur un vrai disque** par cette session.
- **Image physique unifiée (phase 3).**
  - Les slots A/B portent le noyau et 4 services : serviced, healthd, updated et logd.
  - Ils ne portent ni Ladybird, ni DOOM, ni le bureau.

## Ce qui est en place

### Anti-retour (`BOUCHAUD_ANTI_RETOUR_V1`, PR #305)

Voir `UPDATE_RECOVERY.md` § « Authenticité ≠ anti-rollback ».

- **Manifeste v2 signé.**
  - Il porte une séquence monotone et une famille matérielle.
  - Le plancher est stocké dans META, blocs 3 et 4 (deux copies, CRC).
- **Refus.** Le noyau refuse :
  - une release plus ancienne que le plancher ;
  - un manifeste v1 après un plancher ;
  - une autre famille matérielle ;
  - un registre illisible.
- **Rollback local.** Il reste possible.
- **Preuves.**
  - HOST : `tools/update/test_anti_retour.rs`, 11 tests, dont une coupure à chaque octet.
  - QEMU TCG : `run_ab_rollback.sh mise-a-jour`. Le banc vérifie le refus d'une release signée plus ancienne et d'une release signée pour un autre matériel. Il vérifie aussi que le plancher monte au commit de santé.

### Clés de production (`BOUCHAUD_CLES_PRODUCTION_V1`)

- **Liste des clés.** `src/kernel/update/cles_production.rs` liste les moitiés publiques des clés de production.
  - Elle est décodée à la compilation : une clé mal formée casse la construction.
  - **Elle est vide** : un système de production refuse toute installation (`aucune_cle_de_confiance`).
- **Clé de banc.** Elle n'existe que dans une construction `banc-cle-dev` (feature + variable).
  - Elle naît et meurt dans un répertoire temporaire de `run_ab_rollback.sh`.
- **Garde `tools/verifie-cle-dev.py`.** Ce garde refuse :
  - la feature dans `default` ;
  - toute mention de la clé de dev dans un chemin de livraison (`release.yml`, `release-trigkey.yml`, les deux scripts d'image) ;
  - tout bloc PEM de clé privée commité ;
  - un `release-trigkey.yml` déclenchable autrement qu'à la main ;
  - un secret lu hors environnement protégé ;
  - un runner `self-hosted`.

### Chaîne de release (`BOUCHAUD_RELEASE_V1`)

| Pièce | Rôle |
|---|---|
| `tools/release/build_trigkey_ab_image.sh` | image disque A/B + **image d'un slot** (`system-slot.img`, la charge utile d'une mise à jour) ; refuse un arbre sale |
| `tools/release/smoke_trigkey_ab_qemu.sh` | démarre une **copie jetable** de cette image sous QEMU/OVMF et exige un vrai signal de santé ; échoue si l'empreinte de l'original a changé (revue #306, témoin `test_trigkey_ab_smoke.py`) |
| `tools/release/verifie_artefacts_release.py` | avant signature et avant publication : `SHA256SUMS` couvre **exactement** les fichiers publiés ; disque == empreinte de construction == `manifest-ab.json` ; image de slot == manifeste signé ; preuve QEMU du même commit ; signature par une clé engagée, jamais `developpement` |
| `tools/release/garde_release.py` | refuse : hors `main` ; qualification non verte **au SHA exact** (CI Fast, Integration, Non-regression Gate, Reliability V3, Unix & Filesystem Reliability) ; aucune clé de production engagée ; étiquette existante ; version réutilisée ; séquence non croissante |
| `tools/release/fabrique_release.py` | manifeste v2 canonique, signature Ed25519, `release.json`, `SHA256SUMS` ; refuse une signature qui ne vérifie avec aucune clé engagée (mauvais secret), une sortie non vide |
| `tools/release/cle_publique.py` | extrait la moitié publique d'une clé (hors ligne) |
| `.github/workflows/release-trigkey.yml` | **manuel seulement** : garde → image + QEMU → signature (environnement `trigkey-testing`) → prérelease immuable `trigkey-s<N>-<version>` ; promotion `testing → stable` sans nouvelle signature (environnement `trigkey-stable`) |

- **Ce qui ne publie rien.** Un push, même sur `main`, ne publie rien, et une pull request non plus.
- **Le secret.** Il n'est lu que dans le job `signe`, après approbation de l'environnement.
  - Il est écrit dans un fichier 0600, puis effacé.
  - Le job vérifie que la signature correspond à une clé **engagée dans le dépôt**.
- **Lien avec le banc A/B.** Le banc `run_ab_rollback.sh mise-a-jour` installe un manifeste produit par `fabrique_release.py`, avec la clé de banc.
  - Le format émis par la release est donc celui que le noyau installe (QEMU TCG).

**Aujourd'hui, toute exécution du workflow est refusée par la garde.** Le motif est `aucune_cle_de_production_engagee`, et ce refus est voulu.

## Actions humaines (rien de cela n'a été fait en ton nom)

### 1. Check requis (droits administrateur du dépôt)

Settings → Branches (ou Rules) → règle de `main` → « Require status checks to pass ». Ajouter les checks du workflow **Unix & Filesystem Reliability**.

L'API a répondu 403 à cette session : c'est une action d'administrateur.

### 2. Clé de signature de production (hors ligne)

1. Sur une machine hors ligne, jamais un runner, jamais le Trigkey :
   ```
   openssl genpkey -algorithm ed25519 -out release-2026-11.pem
   python3 tools/release/cle_publique.py release-2026-11.pem --nom release-2026-11
   ```
2. Coller la ligne affichée dans `src/kernel/update/cles_production.rs`.
   - Passer par une PR relue.
   - Seule la moitié **publique** va dans le dépôt.
3. Sauvegarder la clé privée hors ligne, en deux copies chiffrées.
   - Sa perte interdit toute nouvelle release vers les machines existantes, sauf réinstallation.

### 3. Environnements GitHub

Settings → Environments :

- **`trigkey-testing`**
  - Relecteur requis : toi.
  - Branches de déploiement : `main` seulement.
  - Secret `BOUCHAUD_RELEASE_ED25519_PEM` = contenu PEM de la clé privée.
- **`trigkey-stable`**
  - Relecteur requis : toi.
  - Branches : `main` seulement.
  - Aucun secret nécessaire : la promotion ne re-signe pas.

### 4. Rotation et révocation

- **Rotation.** Elle se fait en trois étapes :
  1. Engager la nouvelle clé publique à côté de l'ancienne.
  2. Publier une release signée par l'**ancienne**, qui fait confiance aux deux.
  3. Remplacer ensuite le secret par la nouvelle clé.
- **Révocation.** Retirer l'ancienne clé de `cles_production.rs` dans une release suivante.
  - Une machine qui n'a pas reçu cette release fait encore confiance à l'ancienne clé.
  - Aucun banc ne prouve encore ce scénario.

## Opérations physiques (BLOCKED_PHYSICAL)

Ces opérations sont à faire par toi, sur place.

**Aucune écriture NVMe sur un disque contenant des données importantes.**

1. **Clé USB d'essai (`cle-usb-test`), sans risque pour le NVMe.**
   - Procédure : `TRIGKEY_CLE_USB_TEST.md`.
   - Les écritures vers ATA et NVMe sont refusées au niveau du transport (`tools/verifie-ecriture-interne.py`, 16 mutations témoins).
   - L'installateur ne peut donc rien écrire en interne dans ce mode.
   - À relever :
     - démarrage UEFI ;
     - détection RTL8168, DHCP, DNS, TLS, horloge ;
     - écran, clavier et souris.
2. **NVMe de test vierge (ou disque dont la perte est acceptée).**
   - À faire seulement après les phases 2, 3 et 5.
   - L'installateur A/B à venir exigera l'identité exacte du disque (modèle, numéro de série, taille), tapée par toi.
3. **Cycle de mise à jour réel.**
   - Publier une release `testing`.
   - L'installer depuis le Trigkey et redémarrer, avec ton consentement.
   - Vérifier le commit de santé et la montée du plancher.
   - Provoquer un échec (coupure pendant l'écriture), puis vérifier le rollback.
4. **Coupures secteur.**
   - Pendant `write`, pendant `finish`, et entre le commit A/B et le plancher.
   - Le système attendu est l'ancien ou le nouveau, jamais un état intermédiaire (prouvé en HOST et QEMU, pas en PHYSICAL).

## Procédure Windows → Trigkey (aujourd'hui, manuelle)

Les scripts `Build-`, `Test-`, `Prepare-TrigkeyUSB.ps1` et `Release-BouchaudOS.ps1` **n'existent pas encore**.

1. RustRover : développer, pousser une branche, ouvrir une PR. La CI qualifie au SHA exact.
2. Fusionner sur `main` quand tout est vert. Rien n'est publié à ce stade.
3. Actions → « Trigkey release (manuelle, signée) » → `publier`, avec version et séquence.
   - Tu approuves l'environnement `trigkey-testing`.
   - Le workflow produit une prérelease `trigkey-s<N>-<version>`.
4. Après essai : `promouvoir`, avec l'étiquette. Tu approuves l'environnement `trigkey-stable`.
5. Côté Trigkey, l'agent qui télécharge et installe **n'existe pas encore** (phase 5).

## Risques de perte de données

- `installer --go --ecrase` (installateur noyau existant) détruit les partitions étrangères du NVMe. **Ne pas l'utiliser sur le Trigkey.**
- Une clé privée perdue empêche toute mise à jour signée.
- Une clé privée divulguée permet d'installer n'importe quoi au-dessus du plancher. Il faut alors la révoquer en urgence.
- Le plancher anti-retour ne descend jamais. Une release publiée par erreur avec une séquence très haute bloque toutes les suivantes.
  - Il faut alors republier avec une séquence encore plus haute.
  - D'où la garde de monotonie, et la saisie manuelle de la séquence.
