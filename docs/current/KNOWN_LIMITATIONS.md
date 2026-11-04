# Limites connues et chantiers ouverts — relevé 2026-11-04 (`main@69e179a4`)

Chaque statut se rapporte aux preuves disponibles, et non à une impossibilité architecturale. Ce tableau n'invalide pas les implémentations existantes. Il interdit seulement de les présenter comme utilisables au quotidien sans protocole mesuré.

Les colonnes distinguent ce qui est **sur `main`**, ce qui est **dans une PR ouverte** (rien n'en est disponible sur `main`) et ce qui reste **à faire**. Les SHA et verdicts sont dans [status.json](status.json).

| Prio | Domaine | Sur `main` | Dans une PR ouverte | Reste à faire |
|---|---|---|---|---|
| P0 | Qualification P13 stricte | SMP1/2/4 PASS (TCG et KVM hébergés) | — | runners dédiés `bouchaud-kvm4` et `bouchaud-kvm8` (action humaine), SMP8, endurance de 30 min sur un SHA unique |
| P0 | Protection de `main` | ruleset prêt (`tools/github/ruleset-main.json`) | — | application par un administrateur |
| P0 | Matériel physique (Trigkey) | pilotes RTL8168, NVMe, xHCI et GOP présents ; image traçable `tools/release/build_trigkey_image.sh` | #246 : vidange USB réelle, commit A/B sur clé | campagne PHYSICAL (`NOT_STARTED`), image A/B physique avec les services natifs, paquet de mise à jour hors image |
| P0 | Stockage généraliste | RAMFS vivant, `/persist` transactionnel A/B résistant aux coupures (TCG), GPT, FAT32 | #245 : dossiers personnels, écriture différée, métadonnées | VFS disque (inodes, montages), cache de pages unifié, `mmap` en écriture sur fichier, journal, volumes externes ; `/persist` réécrit encore toute sa table à chaque transaction |
| P0 | Stockage USB | un volume USB exposé | #246 : capacité de vidange par support, volume orphelin au débranchement | plusieurs volumes USB, remontage à chaud dans le VFS |
| P1 | Mises à jour | A/B de bout en bout sous TCG (chargeur, essai, commit de santé, retour arrière, recovery) ; manifeste Ed25519 | — | clé de version de production (décision humaine : hors ligne, HSM, rotation, révocation), politique anti-retour arrière, preuve KVM et PHYSICAL |
| P1 | Isolation | 4 services natifs en ring 3 (`serviced`, `healthd`, `updated`, `logd`), navigateur en ring 3 | — | bureau, gestionnaire de fenêtres, applications natives, réseau et TLS encore en ring 0 ; pilotes à privilèges minimaux, isolation DMA |
| P1 | Graphique | composition logicielle par régions, présentation GOP, BGA ou virtio-gpu 2D | #242 à #244 : Atelier ; #247 : première trame −1,9 s | accélération GPU 2D, 3D, pilote AMD, multi-écran, HiDPI |
| P1 | Latence IPC native SMP1 | mesurée et attribuée (`file→élu`) | — | correction |
| P1 | Audio, énergie | AC'97 sous QEMU | — | HDA, batterie, suspend et reprise, thermique : non pris en charge |
| P1 | Périphériques | xHCI (HID, stockage), PS/2 | — | Wi-Fi, Bluetooth, touchpad : non pris en charge |
| P2 | Distribution | — | — | installateur sur disque interne, gestion de paquets, examen LGPL de l'image userland statique |
| P3 | Projets futurs | — | — | IA locale comme service isolé, conteneurs, modes d'usage : **projets, non commencés** |

## Anomalies ouvertes, non attribuées

| Anomalie | Vue | État |
|---|---|---|
| `SIGCHLD_MULTIFIL_ECHEC` | une fois, `main@992e67e1`, passe TCG | 125 reproductions ciblées sans échec ; le journal du banc publie désormais le cas fautif |
| Panique SMP4 « correctif » du banc de comptabilité | une fois sur 41 démarrages locaux | 36 démarrages isolés sans panique ; le banc publie désormais le site et le message |

Une anomalie non attribuée n'est ni effacée ni requalifiée en problème d'infrastructure sans preuve. Si elle réapparaît sur une PR, celle-ci n'est pas prête à fusionner tant que l'anomalie n'est pas classée ([STATUS_CURRENT.md](STATUS_CURRENT.md)).
