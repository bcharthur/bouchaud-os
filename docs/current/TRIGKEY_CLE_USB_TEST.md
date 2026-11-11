# Image d'essai sur clé USB — disques internes en lecture seule

`BOUCHAUD_CLE_USB_TEST_V1`. État : **IMPLÉMENTÉ, qualifié QEMU (TCG, KVM en CI) ; PHYSICAL NON QUALIFIÉ.**
Aucun essai sur le Trigkey n'a été fait avec cette image. Rien dans ce document ne vaut preuve matérielle.

## But

Démarrer Bouchaud OS depuis une clé USB sur une machine qui porte déjà un système, sans **jamais** modifier ses disques internes.
La seule écriture permise vise la clé de démarrage elle-même, reconnue sans ambiguïté.

## Modèle de confiance

| Support | Lecture, identification | Écriture, vidange | Formatage, effacement, TRIM, micrologiciel, passage ATA, inconnu |
|---|---|---|---|
| ATA (IDE/SATA) | permis | **refusé** (`disque-interne`) | **refusé** |
| NVMe | permis | **refusé** (`disque-interne`) | **refusé** |
| USB, clé autorisée | permis | permis | **refusé** (`commande-interdite`) |
| USB, autre clé ou disque USB | permis | **refusé** (`support-non-autorise`) | **refusé** |
| USB, identité non établie | permis | **refusé** (`identite-incertaine`) | **refusé** |
| USB, deux supports portent le GUID | permis | **refusé** (`identite-ambigue`) | **refusé** |

- **La clé autorisée n'est pas reconnue au nom de son pilote.** Elle porte, dans son en-tête GPT primaire, un GUID de disque.
  - Ce GUID est tiré au hasard à la construction (`os.urandom`) et gravé dans le noyau (`BOUCHAUD_MEDIA_AUTORISE`).
  - Il est ensuite posé sur l'image, puis relu.
  - L'identité n'est accordée que si toutes ces conditions sont réunies : signature exacte, somme CRC32 exacte, `MyLBA == 1` et GUID identique.
  - Le moindre écart donne `inconnue` ou `etrangere`.
- **Le GUID est absent ou invalide dans le noyau ?** Aucun support n'est autorisé, et l'image est entièrement en lecture seule.
- **Ambiguïté.** Dès qu'un second emplacement xHCI présente le GUID, plus aucune écriture USB n'est permise jusqu'à l'extinction.
  - Une clé retirée puis rebranchée sur un autre emplacement compte pour deux. Le refus est le repli.
- **Listes blanches, pas listes noires.** Une commande inconnue est refusée.
  - Sur NVMe, `Set Features` n'est permis que pour le nombre de files (FID 7, sans « save »).
  - En SCSI, seul `READ CAPACITY (16)` est permis parmi les `SERVICE ACTION IN`.

## Où la règle s'applique

Elle s'applique aux seuls points où une commande quitte la machine. `tools/verifie-ecriture-interne.py` le vérifie à chaque CI, avec 16 mutations témoins.

| Transport | Point unique | Refus |
|---|---|---|
| ATA | `ata::emet_commande`, seule écriture du registre de commande | `false`, la commande n'est pas émise |
| NVMe | `File::pose`, seule écriture d'une entrée de soumission, files admin et E/S | `Err("refus-cle-usb-test")`, sonnette non sonnée |
| USB (volume) | `bot_commande_une_fois`, seul codage d'un CBW | `Err("refus-cle-usb-test")`, rien sur le bus |
| USB (boîte noire) | `blackbox_bot_en_place`, seul codage d'un CBW | idem |

- Sur NVMe, `admin` est déduit de l'identifiant de la file (`file == 0`), le même qui choisit la sonnette.
  - Une file d'E/S ne peut donc pas être prise pour la file admin, où `0x01` (écriture) serait jugé comme `Create I/O SQ`.
- La règle elle-même est pure et sans allocation : `src/drivers/api/stockage_regle.rs`. Elle est testée sur l'hôte par `tools/securite/test_stockage_regle.rs` (15 tests).
  - Ces tests incluent une table GPT écrite par un outil indépendant du noyau.

### Défaut corrigé au passage

`BOUCHAUD_NVME_CID_JAMAIS_POSE_V1` : une commande NVMe refusée avant la file restait `EnVol` dans le suivi des identifiants.
- C'était déjà le cas pour « file pleine ».
- Après 255 refus, plus aucune entrée-sortie ne partait, lectures comprises.
- Correctif : `Suivi::jamais_pose`.
- Preuves :
  - tests hôte `des_refus_avant_la_file_n_epuisent_pas_le_pot` et `jamais_pose_ne_libere_ni_un_achevement_ni_une_quarantaine` ;
  - banc QEMU : 300 écritures refusées, puis une lecture qui aboutit.

## Limite connue

L'énumération USB est séquentielle. Avec deux clés portant le GUID, la première est la seule autorisée tant que la seconde n'est pas arrivée.
- La sonde de montage lui envoie alors `SYNCHRONIZE CACHE`. Cette commande est sans données et ne suit aucune écriture, donc le support n'est pas modifié.
- Le banc mesure cette limite : zéro écriture de données sur l'une ou l'autre clé, au plus une vidange, et les deux images intactes.

## Preuves (QEMU uniquement)

`tools/ci/run_cle_usb_test.sh BOOT_CLE BOOT_TEMOIN GUID` est exécuté dans Security Runtime, job `cle-usb-test`, sous TCG puis KVM.

**Disques :**
- IDE maître (démarrage) ;
- IDE esclave (données, interne) ;
- NVMe GPT (interne) ;
- une ou deux clés USB GPT.

**Charge :** `disktest --ecriture`, `ata-integrite`, `persist-test --pose`, `installer --go --ecrase`, `usbvolume --io`, puis la sonde.
- La sonde `sonde-ecriture-interne` existe seulement avec `banc-ecriture-interne`. Elle refuse de tourner hors d'une VM QEMU (signature CPUID TCG/KVM).
- Elle émet de vraies commandes :
  - ATA : WRITE, FLUSH, DSM, SECURITY ERASE ;
  - NVMe : Write, Flush, 300 écritures, Write Zeroes, DSM, Set Features (VWC), Format NVM ;
  - USB : WRITE(10), SYNCHRONIZE CACHE, FORMAT UNIT, UNMAP, ATA PASS-THROUGH(16), WRITE SAME(16).

**Trois juges, dont deux hors de la machine :**
1. Empreinte SHA-256 de chaque image disque, avant et après.
2. Trace de QEMU (`ide_bus_exec_cmd`, `pci_nvme_io_cmd`, `pci_nvme_admin_cmd`, `scsi_req_parsed`) : les commandes que les périphériques émulés ont réellement reçues.
3. Journal du noyau : refus, identité, bilan.

| Scénario | Attendu |
|---|---|
| clé autorisée | images boot/scenario/nvme **intactes** ; aucune commande interdite dans la trace ; la clé est écrite et la marque de la sonde est relue ; commandes interdites refusées même sur elle |
| clé étrangère | toutes les images intactes ; aucune écriture ni vidange USB dans la trace |
| deux clés, même GUID | ambiguïté détectée ; aucune écriture de données ; au plus une vidange (limite ci-dessus) |
| **témoin** (sans `cle-usb-test`) | les images **changent**, la trace **voit** les écritures, Format NVM, FORMAT UNIT… Sans ce témoin, un banc aveugle serait vert |

Mesuré localement, TCG : 89 vérifications, `CLE_USB_TEST_OK`. KVM : exécuté en CI uniquement, car la machine de développement n'a pas `/dev/kvm`.

**L'image UEFI réelle**, essai local unique sous OVMF + TCG, non automatisé en CI :
- **Configuration** : `bouchaud-trigkey.img`, construite avec `BO_CLE_USB_TEST=1`, démarrée comme clé `usb-storage` (bootindex 0), avec un NVMe GPT interne et 240 s de fonctionnement.
- **Journal** :
  - `lecture_seule=1 media_autorise=guid-gpt` ;
  - `identite=autorisee ambigu=0`, la même clé étant vue par la boîte noire puis par le volume, sur le même emplacement ;
  - `vidange_reelle=1`.
- **Trace QEMU** :
  - NVMe : seulement `Identify`, `Create SQ/CQ`, `Set Features` (une fois, le nombre de files) et 33 lectures ;
  - clé : lectures, `SYNCHRONIZE CACHE` du montage, aucune écriture.
- **Empreinte du NVMe** : inchangée.

## Construire l'image

```sh
BO_CLE_USB_TEST=1 tools/release/build_trigkey_image.sh OUT_DIR [RAMDISK]
```

Le script :
- tire le GUID, ou prend `BO_MEDIA_AUTORISE=HEX32` ;
- compile avec `cle-usb-test` ;
- pose le GUID sur l'image (`tools/release/pose_guid_disque_gpt.py`, en-têtes primaire et de secours, sommes recalculées) ;
- le relit ;
- vérifie qu'il est dans `kernel.elf` ;
- l'écrit dans `manifest.json` (`cle_usb_test.media_guid_gpt`, `proof_level: BUILD_ONLY`).

Une image dont le GUID relu diffère, ou dont le noyau ne porte pas le GUID, n'est pas produite.

Au démarrage, l'image affiche `BOUCHAUD_DISQUES_INTERNES lecture_seule=1 image=cle-usb-test media_autorise=guid-gpt`. La commande `disques-internes` affiche le bilan des refus.

## Ce qui reste à faire avant tout essai physique

- **Essai matériel**, uniquement sur autorisation explicite et sur une machine dont les disques internes sont sauvegardés :
  - relever l'empreinte des disques internes avant et après, depuis un autre système ;
  - vérifier le journal série.
- Avant cet essai, aucune validation PHYSICAL ne peut être annoncée.
- AHCI natif (SATA en mode AHCI) et virtio-blk : Bouchaud OS n'a **aucun** de ces pilotes aujourd'hui (vérifié dans `src/`). Un disque SATA en AHCI n'est donc ni lu ni écrit.
  - Un futur pilote de stockage devra passer par la même règle, et `verifie-ecriture-interne.py` devra l'imposer.
