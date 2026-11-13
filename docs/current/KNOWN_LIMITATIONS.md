# Limites connues et chantiers ouverts — relevé 2026-11-10 (`main@6bdcb36`, branche `claude/c22-p13-p10-gpu-kernel-0y2ucg`)

Chaque statut se rapporte aux preuves disponibles, et non à une impossibilité architecturale. Ce tableau n'invalide pas les implémentations existantes. Il interdit seulement de les présenter comme utilisables au quotidien sans protocole mesuré.

Les colonnes distinguent ce qui est **sur `main`**, ce qui est **dans une PR ouverte** (rien n'en est disponible sur `main`) et ce qui reste **à faire**. Les SHA et verdicts sont dans [status.json](status.json).

| Prio | Domaine | Sur `main` | Dans une PR ouverte | Reste à faire |
|---|---|---|---|---|
| P0 | Qualification P13 stricte | SMP1/2/4 PASS (TCG et KVM hébergés) | branche du 10/11 : `P13_STRICT_OK` SMP1/2/4 KVM (`540fdfb`) ; correctif #264 (passation sous IF coupé) | runners dédiés `bouchaud-kvm4` et `bouchaud-kvm8` (action humaine) : SMP8 et `P13_FINAL_STRICT_OK` = **BLOCKED_EXTERNAL** |
| P0 | Protection de `main` | ruleset prêt (`tools/github/ruleset-main.json`) | — | application par un administrateur |
| P0 | Matériel physique (Trigkey) | pilotes RTL8168, NVMe, xHCI et GOP présents ; image traçable `tools/release/build_trigkey_image.sh` ; vidange USB réelle et commit A/B sur clé (TCG) | — | campagne PHYSICAL (`NOT_STARTED`), image A/B physique avec les services natifs, paquet de mise à jour hors image |
| P0 | Stockage généraliste | RAMFS vivant, `/persist` transactionnel A/B résistant aux coupures (TCG), GPT, FAT32 | #245 : dossiers personnels, écriture différée, métadonnées | VFS disque (inodes, montages), cache de pages unifié, `mmap` en écriture sur fichier, journal, volumes externes ; `/persist` réécrit encore toute sa table à chaque transaction |
| P0 | Stockage USB | un volume USB exposé ; capacité de vidange liée au support, volume orphelin au débranchement | — | plusieurs volumes USB, remontage à chaud dans le VFS ; commit sur clé fiable quand l'hôte est lent (anomalie attribuée ci-dessous) |
| P1 | Mises à jour | A/B de bout en bout sous TCG (chargeur, essai, commit de santé, retour arrière, recovery) ; manifeste Ed25519 | — | clé de version de production (décision humaine : hors ligne, HSM, rotation, révocation), politique anti-retour arrière, preuve KVM et PHYSICAL |
| P1 | Isolation | 4 services natifs en ring 3 (`serviced`, `healthd`, `updated`, `logd`), navigateur en ring 3 | — | bureau, gestionnaire de fenêtres, applications natives, réseau et TLS encore en ring 0 ; pilotes à privilèges minimaux, isolation DMA |
| P1 | Graphique | composition logicielle par régions, présentation GOP, BGA ou virtio-gpu 2D ; bureau Atelier (design system, dock, centre de contrôle, Fichiers) | branche du 10/11 : **composition GPU des fenêtres sous QEMU VIRGL** (KVM CI, mire au pixel, A/B, repli 2D, panne puis reprise) | raster GPU de Skia/Ladybird (BLOCKED_EXTERNAL : QEMU ≥ 9.2 Venus + uAPI DRM invitée), pilote AMD (Trigkey), compositeur en ring 3, multi-écran, HiDPI |
| P1 | Latence IPC native SMP1 | mesurée et attribuée (`file→élu`) | — | correction |
| P1 | Audio, énergie | AC'97 sous QEMU | — | HDA, batterie, suspend et reprise, thermique : non pris en charge |
| P1 | Périphériques | xHCI (HID, stockage), PS/2 | — | Wi-Fi, Bluetooth, touchpad : non pris en charge |
| P2 | Distribution | — | — | installateur sur disque interne, gestion de paquets, examen LGPL de l'image userland statique |
| P3 | Projets futurs | — | — | IA locale comme service isolé, conteneurs, modes d'usage : **projets, non commencés** |

## Anomalies ouvertes, non attribuées

- **#283 — Ladybird, démarrage à froid figé (~20 min) après le fork des rendus.**
  `LADYBIRD_FUNCTIONAL_SMOKE fail raison=jalons manquants=7`, 2 démarrages sur
  96 (épreuve « piège du Compositor »), sur `main` ET sur la branche candidate.
  Cause non établie (piste : ordonnancement / réveil perdu après fork-exec).
- **#288 — Banc de coupure : faux `INVENTION` possible.** Une occurrence sur
  ~180 coupures ; mécanisme probable (acquittements encore dans l'anneau de la
  console asynchrone, tolérance du juge +1) lu dans le code, non prouvé par une
  trace. Seuil non modifié.

| Anomalie | Vue | État |
|---|---|---|
| `SIGCHLD_MULTIFIL_ECHEC` | deux fois, en passe TCG : `main@992e67e1`, puis `main@b3d4077` (#311) | 125 reproductions ciblées sans échec après la première. La seconde nomme le cas `boucle` : enfant zombie récoltable, mais SIGCHLD jamais livré au parent (réveil seulement par la borne de 10 s). |
| Panique SMP4 « correctif » du banc de comptabilité | une fois sur 41 démarrages locaux | 36 démarrages isolés sans panique ; le banc publie désormais le site et le message |
| #264 panique SMP4 de passation pendant le soak de 20 min | soak `reliability-v3`, intermittente | **Attribuée le 10/11** : `switch_to` publiait la passation (`SWITCH_PENDING`) avec IF ouvert ; une IRQ de l'horloge entre cette publication et `switching_out` terminait une passation à moitié publiée, ou publiait la tâche sortante depuis sa propre pile. Correctif : passation publiée et terminée IF coupé, assertion dans `prepare_switch_handoff`, test structurel + 4 mutations (`tools/smp/test_passation_if_coupe.rs`). Soak SMP4 TCG local 9/9. **Close seulement après le soak hébergé de 20 min (`reliability-v3`, QEMU TCG `-cpu max`, SMP4) vert au SHA final.**<br>**Récidive le 12/11** (piège du Compositor, 3fd7ada, cpu=3, juste après un fork ; `tid=100`, la tâche SORTANTE) : #285 coupait IF dans `switch_to`, mais `kernel_task_trampoline` et `idle_bsp_trampoline` (pile initiale RFLAGS=0x202), ainsi que l'entrée de `schedule()`, terminaient une passation **interruptions ouvertes**. Une IRQ de quantum entre la lecture de `SWITCH_PENDING` et le contrôle de `switching_out` terminait la même passation dans `preempt_from_irq`. Correctif : `complete_switch_handoff` atomique vis-à-vis des IRQ (`without_interrupts`), garde `debug_assert` ; règle structurelle + 2 mutations (`tools/smp/test_passation_if_coupe.rs`). #264 rouverte jusqu'à sa qualification. |

Une anomalie non attribuée n'est ni effacée ni requalifiée en problème d'infrastructure sans preuve. Si elle réapparaît sur une PR, celle-ci n'est pas prête à fusionner tant que l'anomalie n'est pas classée ([STATUS_CURRENT.md](STATUS_CURRENT.md)).

## Anomalies attribuées (cause établie, correctif à décider)

| Anomalie | Vue | Cause établie | État |
|---|---|---|---|
| `USB_AB_COMMIT_REFUSE_HOTE_LENT` : `ab-rollback (sante \| sans-vidange \| seconde-cle, usb)` rend `B- B+` (ou `A-`) au lieu de `B+ B=` | CI, depuis l'ajout des scénarios sur clé (#246) : environ 5 passages USB sur 11 le 2026-11-04 ; jamais sur NVMe | Sur la clé émulée, deux échéances CSW consécutives (`bot-csw-echeance`, budget runtime BOT de 60 ms, antérieur à #246) malgré la reprise et le rejeu unique : la lecture de la META ou la relecture après écriture échoue, et le noyau **refuse le commit** (`SERVICED_UPDATE_STATE rc=-13`, `UPDATE_SANTE_REFUS raison=relecture`). Reproduit en local 2 fois sur 3 sous contention CPU de l'hôte, 0 sur 3 sans, même arbre que `main`. | Comportement **sûr** : aucun faux commit, le démarrage suivant committe. Le correctif touche soit le seuil BOT, soit la politique de commit de `serviced` (Atomic A/B) : décision humaine. En attendant, une relance unique, et toute autre signature reste rouge. |
| `#247` comptabilité SMP2 `COMPTA_INJECTION_NON_QUALIFIANT` | P13 de #247 (run 41809544156), reproduit en local à chaque passage | **Manque de fenêtres**, pas une violation : bras correctif 0 à 3 fenêtres avec la précharge de police, 13, 16 et 18 sans (même noyau, appel neutralisé), `main` 8 à 27 ; zéro chevauchement partout. Le mécanisme exact (la précharge se termine vers 3,3 s) reste à établir. | #247 non fusionnable ; le banc n'est pas modifié. |
| `#245` et `#247` scanout virtio `SCANOUT_VIVANT_ECHEC pixels_changes=0` | P13 de #245 et #247 | Le banc regardait les secondes de l'ancienne horloge ; la barre Atelier n'en affiche plus. Le framebuffer n'est pas figé. | **Corrigé sur `main`** par #242 (`BOUCHAUD_SCANOUT_VIVANT_POINTEUR_V1`, avec témoin négatif VM arrêtée). #245 et #247 l'obtiendront en intégrant `main`. |
| `#247` console série (étape 21) | P13 de #247 (run 41809544156) | Non reproduit : les deux variantes TCG passent en local sur le même noyau ; les variantes KVM ne peuvent pas tourner ici. | Non attribué ; à relire au prochain P13 de #247. |

