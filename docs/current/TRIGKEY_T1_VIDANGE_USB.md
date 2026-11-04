# Trigkey T1 — vidange réelle du stockage USB (BOUCHAUD_USB_VIDANGE_REELLE_V1)

Branche `claude/trigkey-t1-vidange-usb-20261104`, issue de `main@69e179a4`. Premier point du plan T1 (`docs/current/TRIGKEY_PLATFORM_PLAN.md`, branche `claude/trigkey-physical-20261103`).

Le code de mise à jour A/B (`kernel/update`) **n'est pas modifié** : il refuse toujours un commit sur un volume sans vidange réelle. C'est le pilote USB qui, désormais, peut prouver qu'il en a une.

## Le verrou

Le volume USB (`usb-bot`) déclarait `vidange_reelle: false`, et une vidange rendait `Fait(0)` sans rien émettre. C'était honnête, puisqu'une clé avec un cache d'écriture peut accepter une écriture sans l'avoir posée. Mais cela interdisait tout commit A/B sur une clé : en QEMU (xHCI et usb-storage), le slot B démarrait et son commit de santé était refusé (`B- B-`, `volume_sans_vidange_reelle`).

## Ce qui change (`src/drivers/usb/xhci_active.rs`)

- **Sonde au montage.** Après `READ CAPACITY`, le pilote émet `SYNCHRONIZE CACHE (10)` (opcode `0x35`, sans données, LBA 0, longueur 0, c'est-à-dire tout le support). `vidange_reelle` ne devient vrai **que** si le support la termine avec un CSW de statut 0. En cas de refus, le sens en attente est lu pour l'effacer, et le volume reste utilisable sans promesse de durabilité. La ligne `BOUCHAUD_USB_STOCKAGE_VIDANGE sonde=… vidange_reelle=…` le dit.
- **Vidange réelle.** `Genre::Vidange` émet la commande et attend son CSW.
- **Perte de confiance.** Un support qui refuse plus tard repasse à `vidange_reelle=0`, la vidange échoue et le commit A/B est refusé (`BOUCHAUD_USB_STOCKAGE_VIDANGE_REFUSEE`).
- **Injection** : `usbfault vidange` (shell) et la feature de banc `banc-ab-sans-vidange` (jamais dans `default`), qui fait refuser la sonde, pour le témoin négatif.

## Correctifs de revue (Codex, PR #246)

### 1. La capacité appartient au support qui porte le volume (`BOUCHAUD_USB_LIAISON_SUPPORT_V1`)

**Défaut.** `USB_VIDANGE_REELLE`, `USB_BLOCS` et `USB_TAILLE_BLOC` étaient des atomiques globales que **chaque** support prêt réécrivait, alors que les E/S allaient au « premier support prêt ». Le défaut était plus large que la capacité de vidange. Démontré sur le binaire d'avant (`924521a2`, banc `run_usb_liaison.sh`, journaux bruts) :

| Situation | Ce que faisait l'ancien code |
|---|---|
| Clé 1 vide, la sonde de la clé 2 échoue | la clé 2 republie le volume avec **sa** géométrie (32 Mio au lieu de 64) et retire la promesse de la clé 1 |
| La sonde de la clé 1 échoue, clé 2 vide | la clé 2 **accorde** `vidange_reelle=1` à un volume servi par la clé 1 : fausse promesse de durabilité |
| Clé liée débranchée puis rebranchée | volume re-publié en silence, `vidange_reelle=1`, sur un support dont l'identité n'est plus certaine |

**Correction.**
- Chaque support reçoit au montage un **numéro** unique depuis le démarrage, jamais réutilisé (le `slot_id` xHCI et l'indice dans la table, eux, sont recyclés).
- `Stockage::vidange_sondee` porte la sonde de **ce** support.
- Le volume `Volume(3)` est **lié** au premier support prêt. Un mot atomique (`LIAISON_VOLUME`, logique pure dans `src/drivers/usb/stockage/liaison.rs`) dit quel numéro il désigne, si ce support a prouvé la vidange et s'il est encore là.
- Géométrie, lecture, écriture et vidange désignent ce numéro (`support_lie`), jamais « le premier prêt ». Un second support n'est pas publié (`BOUCHAUD_USB_STOCKAGE_NON_PUBLIE … raison=volume-deja-lie`) et ne change rien au volume.
- Si le support lié est retiré, le volume devient **orphelin** pour le reste du démarrage (`BOUCHAUD_USB_VOLUME_ORPHELIN`) : géométrie à 0, E/S `Absent`, aucune promesse. Une clé rebranchée (la même ou une autre, on ne peut pas le savoir) reçoit un nouveau numéro et n'est **pas** reliée. En effet, la table de partitions, la persistance et l'A/B ont lu l'ancien support. `trouve_meta` ne trouve plus le volume, et aucun commit n'a lieu.
- Toutes les écritures du mot de liaison se font sous le verrou du pilote. L'unique exception, une vidange abandonnée faute de verrou, ne fait que **retirer** la promesse, par échange-compare : elle n'écrase jamais un état orphelin publié entre-temps.
- **Transport partagé.** L'état `reprise_bot` est unique pour tous les supports. Une reprise en attente appartient désormais au support qui a eu l'incident (`Transport::concerne`, `remet_a_neuf_pour`). La mise en service d'une seconde clé n'efface plus la reprise que la première doit encore faire, et une clé ne se réinitialise plus à la place d'une autre. Sans cela, le TD abandonné resterait sur l'anneau du support fautif, et sa commande suivante prendrait un achèvement tardif pour le sien.

**Limite assumée.** Un seul volume USB est exposé (`Volume(3)`). Une seconde clé est vue, sondée et journalisée, mais pas publiée. Exposer plusieurs volumes USB est un autre chantier (allocation de volumes, montage à chaud dans le VFS).

### 2. Un refus de vidange est traité selon le protocole (`BOUCHAUD_USB_REFUS_SCSI_V1`)

**Défaut.** Un refus en cours d'utilisation retirait bien la promesse, mais laissait la condition de sens en attente. Certaines clés refusent alors toute commande suivante, lectures comprises.

**Correction** (`traite_refus_vidange`, appelée à la sonde comme en usage, **sous le même verrou** que la commande refusée, sans second verrou) :
- **CSW « Command Failed »** (refus de commande, transport sain) : `REQUEST SENSE`, qui lit et efface la condition. Le sens est journalisé (`sens=clé/ASC/ASCQ(nom)`).
- **Échec du transport** (échéance, STALL non récupéré, erreur de phase) : reprise Bulk-Only (réinitialisation de classe et déblocage des deux points), seulement sur le support fautif, puis `REQUEST SENSE`.
- **Aucune commande émise** (injection, support absent) : rien à lire ni à reprendre.
- Dans tous les cas, la promesse du support lié est retirée, et le commit A/B reste refusé.
- Les lectures et écritures ordinaires continuent.

## Preuves

Tout ce qui suit a été rejoué sur l'arbre final de la PR.

### Tests hôte

| Suite | Résultat | Ce qu'elle établit |
|---|---|---|
| `tools/platform/test_usb_liaison.rs` | 17/17 | tests 1 à 9 au niveau du modèle : promesse du volume = celle du seul support lié, quelle que soit la séquence. L'exploration exhaustive compare 59 049 séquences de 5 événements à un modèle de référence écrit à part. Elle couvre aussi : seconde clé sans effet dans les deux sens, refus en usage, volume orphelin, aucune reliaison, numéro 0, mot publié fidèle, classement des refus, sens fixe et descripteur, sens tronqué |
| `tools/platform/test_reprise_bot.rs` | 13/13 (dont 3 nouveaux) | une reprise en attente n'appartient qu'au support fautif ; la mise en service d'une seconde clé ne l'efface pas |
| `tools/ci/run_host_tests.sh` | 131 suites, toutes vertes | non-régression hôte |

### QEMU TCG : `tools/ci/run_usb_liaison.sh` (nouveau, image par défaut)

56 invariants sur 56, six démarrages en 3 min 27 s en local (`USB_LIAISON_OK scenarios=6`). Le banc tourne aussi dans le workflow Integration (job `usb-liaison`, requis par `integration-gate`).

| Test | Scénario | Ce qui est vérifié |
|---|---|---|
| 1 | `accepte` | sonde `ok`, volume lié au n° 1 avec `vidange_reelle=1`, E/S et vidange réussies |
| 2, 5 | `refuse-sonde` | sonde refusée, volume publié **sans** promesse, lecture, réécriture et relecture réussies deux fois, aucune vidange émise |
| 3 | `deux-cles` | clé 1 (64 Mio) vide, sonde de la clé 2 (32 Mio) refusée : la clé 2 n'est pas publiée, le volume garde la géométrie **et** la promesse de la clé 1, et les E/S portent sur la dernière LBA de 64 Mio |
| 3, 9 | `deux-cles-inverse` | sonde de la clé 1 refusée, clé 2 vide : la clé 2 **n'accorde pas** de promesse au volume de la clé 1 (commit A/B refusé) |
| 4, 5 | `refus-en-usage` | sonde passée, puis **refus SCSI réel** de QEMU après la première écriture (`blkdebug`, `iotype=flush`) : `REQUEST SENSE` lu sous le verrou (`sens=11/0x00/0x06`), promesse retirée, lectures et écritures encore bonnes ensuite |
| 6, 7 | `rebranchement` | débranchement vu, volume orphelin, plus d'E/S ni de vidange ; la clé rebranchée reçoit le n° 2, n'est pas reliée, et l'ancienne capacité n'est pas réutilisée |

**Témoin négatif.** Le même banc, joué sur le binaire d'avant (`924521a2`), montre les défauts dans ses journaux bruts. Le tableau des défauts ci-dessus en donne le détail : la clé 2 republie le volume, accorde ou retire la promesse de la clé 1, et une clé rebranchée est re-publiée avec `vidange_reelle=1`.

### QEMU TCG et OVMF : `tools/ci/run_ab_rollback.sh`

| Support | Scénario | Attendu | Obtenu |
|---|---|---|---|
| **USB** | `sante` | `B+ B=` | `B+ B=` |
| **USB, deux clés** (`BO_AB_SUPPORT=usb-deux`, nouveau) | `seconde-cle` : la clé A/B vide, la seconde refuse (feature de banc `banc-ab-seconde-cle-sans-vidange`) | `B+ B=` | `B+ B=` (`NON_PUBLIE numero=2`, `SERVICED_HEALTH_COMMIT result=committed`) — test 8 |
| **USB** | `sans-vidange` (témoin négatif) | `B- B- B- A= A=` | `B- B- B- A= A=` — test 9 |
| **USB** | `mise-a-jour` | `A* B+ B=` | `A* B+ B=` |
| **USB** | `panique` | `B B B A A` | `B B B A A` |
| **USB** | `recovery-rollback` | `R B=` | `R B=` |
| **USB** | `echecs-repetes` | `A- A- A- R` | `A- A- A- R` |
| NVMe | `sante` (non-régression) | `B+ B=` | `B+ B=` |

Les scénarios USB `sante`, `sans-vidange` et `seconde-cle` entrent dans la matrice `ab-rollback` du workflow Native IPC, dont le filtre de chemins couvre désormais `src/drivers/usb/**`. Avant, aucun scénario USB n'y figurait.

### Autres vérifications

- `tools/ci/run_usb_stockage.sh` : `USB_STOCKAGE_OK`.
- Garde-fous : 209/209.
- `CI_POLICY_OK`.
- 62 avertissements, comme sur `main`.

## Ce que cela ne prouve PAS

- **PHYSICAL reste NOT_STARTED.** Ces preuves sont QEMU : le usb-storage de QEMU accepte `SYNCHRONIZE CACHE`. Une vraie clé peut la refuser (le pilote le gère : pas de commit), ou l'accepter sans vider réellement son cache, ce qu'aucun logiciel ne peut détecter. La campagne Trigkey (T3, T4) doit le mesurer sur la clé choisie, avec une coupure secteur pendant l'écriture.
- Le reste du plan T1 n'est pas encore fait : image A/B physique traçable (`build_trigkey_ab_image.sh`), paquet de mise à jour hors image, garde `verifie-image-trigkey-ab.py`.
