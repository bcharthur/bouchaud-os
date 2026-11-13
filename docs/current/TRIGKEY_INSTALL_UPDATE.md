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
| 3 | Image de livraison unique (A/B + STATE, services, compositeur, DOOM, Ladybird, /etc, OTA stable) | **FAIT** ; Ladybird prouvé seulement dans le job CI qui a l'artefact | QEMU TCG/OVMF, image exacte, gestes réels (`test_livraison_qemu.py`) |
| 4 | Release signée, canaux, promotion | **FAIT (outillage)**, bloqué par la clé | HOST (31 tests, dont témoins négatifs) + QEMU TCG (banc A/B) |
| 5 | Agent OTA : vérification signée périodique | **TRANCHE 1 FAITE** (vérifie, n'installe pas) | HOST (`test_ota.rs`) + QEMU TCG (`run_ab_rollback.sh ota-verifie`, serveur HTTP hôte) |
| 5 | Agent OTA : téléchargement HTTPS, reprise, installation, redémarrage consenti, santé, rollback | **DÉMONTRÉ EN QEMU** (non qualifié en physique) | QEMU TCG (`run_ab_rollback.sh ota-complet` et `ota-rollback`) |
| 6 | Ed25519, anti-retour, refus sans clé | **FAIT** | HOST + QEMU TCG |
| 6 | Rotation et révocation de clé | **PROCÉDURE** | aucun banc |
| 7 | Rollback, recovery | **FAIT** | QEMU TCG (`run_ab_rollback.sh`, 11 scénarios) |
| 8 | Persistance à travers coupures et changement de slot | **FAIT** | QEMU TCG/OVMF (`run_state_ab.sh` : A, A, B ; compteur 1, 2, 3) |
| 9 | Réseau RTL8168, DHCP, DNS, TLS, horloge | **BLOCKED_PHYSICAL** | QEMU seulement (autres NIC) |
| 10 | `bo-update status|historique|verifier|consentir` (Terminal) | **FAIT** | compile, garde des commandes ; exécution QEMU : voir ci-dessous |
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
- **Image de livraison unique (phase 3, `BOUCHAUD_LIVRAISON_V1`).**
  - **Une seule image** : `build_trigkey_ab_image.sh` (avec `BO_NATIF_LADYBIRD`). Le ramdisk de chaque slot est le système complet assemblé par `assemble_systeme.sh` :
    - `serviced`, `healthd`, `updated`, `logd` ;
    - `composited` ;
    - DOOM (doomgeneric épinglé, Freedoom 0.13.0 vérifié) ;
    - Ladybird et ses processus (artefact natif de la CI), avec ses certificats, ses polices et sa page d'accueil ;
    - `/etc/bouchaud/version` (version, commit, séquence, canal) ;
    - `/etc/bouchaud/ota.conf` : canal **stable**, téléchargement automatique, redémarrage **sur consentement** ;
    - `/etc/bouchaud/composants` : ce qui est embarqué et ce qui manque. Sans Ladybird, l'image est marquée `livrable=non`.
  - **Image de release.** Avec `BO_SEQUENCE > 0`, elle porte aussi son registre anti-retour (slot A = N, plancher N) : un système fraîchement installé ne se repropose pas sa propre release.
  - **Test sur l'image exacte** (`test_livraison_qemu.py`).
    - Il démarre une copie jetable sous QEMU/OVMF (NVMe, e1000) et utilise la **souris de QEMU**. La géométrie du bouton Démarrer et des lignes du menu est journalisée par l'Atelier lui-même.
    - Il ouvre le Terminal, DOOM puis Ladybird par le menu Démarrer, et exige pour chacun la fenêtre, le processus ring 3 et la **première trame livrée**. Pour Ladybird, il exige aussi la première trame de son interface.
    - Il exige enfin la santé des services, STATE montée, l'agent OTA actif, aucune panique, et l'image inchangée.
    - Verdicts : `LIVRAISON_QEMU_OK` (livrable) ou `PARTIEL` (Ladybird absent, dit).
    - Le job CI `ladybird / image Trigkey de livraison` le lance avec `--exige-ladybird`. Le workflow de release aussi : **une release sans Ladybird est refusée**.
  - **Preuve locale** (sans l'artefact Ladybird) : bureau, Terminal (invite `root:/$`), DOOM (écran titre Freedoom), STATE, OTA, aucune panique. Verdict `LIVRAISON_QEMU_PARTIEL` honnête.

## OTA actif sur l'image livrée (`BOUCHAUD_OTA_V1`)

- **Configuration embarquée** : `/etc/bouchaud/ota.conf`.
  - `canal=stable` ; la source est `releases/latest/download` du dépôt.
  - `telechargement=consenti` : vérification, téléchargement et préparation du slot inactif sont **automatiques**.
  - `redemarrage=demande` : **jamais** de redémarrage sans `bo-update consentir`.
  - Option `redemarrage=fenetre:HH-HH:MIN` : redémarrage automatique entre HH et HH (UTC), seulement après MIN minutes **sans aucune entrée** clavier ou souris (au moins 5). Il n'y a jamais de redémarrage pendant une session active.
- **Vérification** : au démarrage, **dès que le réseau a un bail** (sans délai d'abandon), quand il revient après une perte, toutes les 6 h, et à la demande (`bo-update verifier`).
- **État persistant dans STATE.**
  - `/persist/.bouchaud/ota-etat` contient : `version`, `commit`, `canal`, `slot_actif`, `release_slot_actif`, `release_slot_inactif`, `plancher`, `release_en_echec`, `dernier_demarrage`, `reseau`, `derniere_verification`, `dernier_telechargement`, `prochaine_installation`.
  - `/persist/.bouchaud/ota-historique` garde les 50 dernières lignes : refus, installations refusées, candidat prêt, redémarrage et son consentement, **rollback** (lu dans la raison du chargeur).
  - **Lecture** : `bo-update status` et `bo-update historique`, dans le Terminal de l'Atelier.
  - **Écriture** : en RAMFS sous verrou, sans E/S. C'est l'écriture différée qui synchronise, et avant un redémarrage OTA, une synchronisation explicite.
- **Canal `testing`** : non automatisé. GitHub n'offre pas d'URL « dernière prerelease », et un canal testing exige une étiquette précise. **BLOCKED** : à concevoir (pointeur de canal signé).

## Campagne D : réseau hostile (`BOUCHAUD_OTA_RESEAU_V1`, banc `ota-reseau`)

Un seul banc, `tools/ci/run_ab_rollback.sh ota-reseau` (QEMU TCG/OVMF, NVMe, e1000), sert la release 1001 depuis un réseau hostile. Le serveur HTTPS de banc est `tools/ci/serveur_ota_banc.py` : chaque défaut est volontaire, unique et journalisé côté serveur. Séquence attendue et obtenue en local : `A~ A* B+`.

| Défaut | Conséquence exigée | Preuve |
|---|---|---|
| Source au certificat d'une **CA inconnue** (même nom, même adresse) | Refus **avant toute requête** : `certificat_non_fiable` | `BOUCHAUD_OTA_TLS_REFUS … ancre de confiance inconnue` ; aucun `GET` côté serveur |
| Source en **HTTP 500** | Refus nommé : `manifeste_http_erreur` | `BOUCHAUD_OTA_HTTP_STATUT statut=500` |
| **Partie altérée** (bonne taille, un octet changé) | Contenu, pas transport : téléchargement refusé (`partie_empreinte_differente`), **non** repris sur place, relance du cycle à 60 s | historique `telechargement_refuse`, `BOUCHAUD_OTA_RELANCE dans_s=60` |
| **Partie tronquée** (Content-Length complet, moitié du corps) | Transport : **cette** partie reprise | `REPRISE … part002 essai=1 raison=partie_tronquee`, puis `essai=2 resultat=ok` |
| **Lien coupé 20 s** (`set_link`) après la partie 3 | Reprise de la partie 4 au retour du lien | `REPRISE … part004 essai=[2-4] resultat=ok` |
| **Alimentation coupée** après la partie 5 | Démarrage suivant : interruption **vue dans STATE**, rien d'écrit dans un slot ; tout recommence depuis la partie 0, sous signature | `BOUCHAUD_OTA_TELECHARGEMENT_INTERROMPU`, slot A `raison=actif`, puis installation, `B+` |

Changements de comportement apportés par cette campagne :

- **TLS strict pour l'OTA** (`BOUCHAUD_TLS_CONFIANCE_V1`).
  - Le **premier essai du banc a montré** que le noyau acceptait le certificat d'une CA inconnue et téléchargeait toute la release depuis ce serveur. Le client HTTPS du noyau, conçu pour le navigateur, signalait `[TLS !]` mais continuait.
  - Désormais, une source `https` exige à chaque redirection une chaîne de confiance, le nom couvert et un certificat non expiré, **avant** d'envoyer la requête. Une session keep-alive non fiable n'est pas réutilisée, et une redirection vers `http` est refusée.
  - Un miroir déclaré `http` reste permis : la décision repose sur la signature du manifeste.
  - Le navigateur ne change pas.
- **Relance bornée** : après un échec récupérable, 60 s, puis 120 s, … sans dépasser 6 h. Une réussite remet le compteur à zéro.
- **État relu sur STATE** au démarrage : le dernier téléchargement et la dernière vérification traversent les redémarrages, dans des bornes (32 champs, clés `[a-z0-9_]`).
- **Ce qui n'existe pas encore** : la reprise **d'octets** à travers un redémarrage. Les parties vivent en RAM. Une coupure fait tout recommencer, et rien n'est jamais installé à partir d'un téléchargement incomplet.

## Politique OTA en anneau 3 (P4, `BOUCHAUD_OTA_ANNEAU3_V1`)

**Fait (branche `claude/trigkey-ota-ring3`).** L'agent quitte le noyau.

- **L'agent** : `/bin/bo-ota` (`userland/services/ota/`, Rust std, musl statique).
  - C'est un service **non critique** de `serviced` ; le contrat de santé v3 est inchangé.
  - Il fait ce que faisait le fil noyau `ota`, avec les mêmes lignes de journal :
    - vérification signée et HTTPS strict ;
    - parties avec reprise, préparation dans `/update/`, `pret generation=N` ;
    - nouvel essai après un refus ;
    - état et historique dans STATE.
- **Le TLS** : la pile du **dépôt**, sans bibliothèque tierce. Le blocage « choix d'une pile TLS en anneau 3 » est donc levé sans import.
  - `system/tls-core` compile les **fichiers** TLS du noyau (`#[path]`) : une correction s'applique aux deux dans le même commit, avec les mêmes racines.
  - Le handshake est générique sur un `Transport` et une `Plateforme` (aléatoire, heure). `validate.rs` ne lit plus aucune horloge.
- **Ce que le noyau garde** :
  - **L'installation** : `updated` installe, et le noyau revérifie tout au `begin` et au `finish`.
  - **Son état d'autorité**, publié en lecture seule et régénéré à chaque ouverture : `/proc/bouchaud/ota`. On y trouve les clés de confiance, l'anti-retour, les releases des slots, le candidat, les refus, le bail réseau, le consentement et les demandes de vérification.
  - **Le redémarrage**. L'agent écrit `/update/redemarrage` ; le noyau ne le suit que si META porte un candidat, puis selon la politique ou le consentement.
- **Conséquence** : un agent compromis peut gaspiller de la bande passante ou se taire. Il ne peut ni installer une image non signée, ni redescendre de version, ni redémarrer sans candidat vérifié.

**Preuves.**
- `tools/ci/test_bo_ota_tls.sh` (CI Fast) : la pile du noyau sur un socket Linux, contre de vrais serveurs TLS 1.3.
  - CA de banc : acceptée, octets et empreinte exacts.
  - CA inconnue et nom non couvert : refusés **avant** toute requête (le serveur n'en reçoit aucune).
  - Serveur absent : refus de transport.
- `ota-complet` en local, sous QEMU TCG : `A* B+ B=` et `AB_OTA_COMPLET_OK`.
  - 24 parties en HTTPS strict depuis l'anneau 3 ;
  - partie 5 coupée, puis reprise ;
  - premier `finish` refusé volontairement, puis reproposé (génération 2) ;
  - candidat, redémarrage décidé par le noyau, B commité, puis `a_jour`.
- Les autres bancs `ota-*` sont qualifiés par la CI.

**Reste à faire.**
- Profil de sécurité dédié à l'agent. Il tourne aujourd'hui avec le profil `System` de `serviced`, comme les autres services.
- Reprise d'octets d'un téléchargement à travers un redémarrage.
- Chaîne TLS **réelle** de GitHub depuis l'anneau 3 : P11b, physique ; le proxy de la session intercepte TLS.

## Incident intermittent `ota-complet` (non résolu, surveillé)

- **L'échec.** `ab-rollback (ota-complet)` a échoué **une fois** en CI, au SHA ec0ea87, au démarrage 1 (verdict `A!`).
- **Cause inconnue.** À ce SHA, la trap du serveur de banc écrasait celle qui lance le diagnostic (corrigé dans d993d63).
- **Depuis** : succès en CI à d993d63 (×2), 6f44125 et `main@df30caa` ; 5 succès en local.
- **Statut.** Ce n'est **pas** considéré comme résolu. Tout nouvel échec doit être diagnostiqué : les lignes `AB_DIAG` OTA, `updated` et HTTPS figurent désormais dans le journal du job.
- **Nouvel indice** : `ota-rollback`, run 43301393229, SHA 260fc0c, même verdict `A!` au démarrage 1.
  - Ce que montre `AB_DIAG` :
    - la partie 13 attend 12,8 s puis revient vide ;
    - les trois essais suivants reviennent vides en 3 s, **sans poignée TLS terminée** ;
    - le téléchargement est refusé.
  - **Cause probable**, côté banc :
    - le serveur HTTPS de banc faisait sa poignée TLS dans sa boucle d'acceptation, sans délai ;
    - une connexion figée bloquait donc toutes les suivantes ;
    - le banc `ota-reseau`, avec sa coupure de lien, a reproduit ce blocage de façon déterministe.
  - **Correction** : `tools/ci/serveur_ota_banc.py` fait la poignée dans le fil de chaque connexion, avec un délai de 60 s.
  - **Statut inchangé** : ce n'est **pas** déclaré résolu tant que des runs ultérieurs n'ont pas été observés, et la cause du premier gel de connexion, côté invité, reste inconnue.

## Agent OTA, tranche 2 : de la release signée au slot actif (`BOUCHAUD_OTA_V1`)

**Validé sous QEMU TCG/OVMF, sur deux versions système réellement distinctes. Non testé sur le Trigkey.**

- **Consentement explicite**, dans la configuration :
  - `telechargement=consenti` : sans lui, l'agent signale `consentement_requis` et ne télécharge rien ;
  - `redemarrage=consenti` : sans lui, il installe puis attend (`BOUCHAUD_OTA_REDEMARRAGE_EN_ATTENTE`).
  - Il n'existe pas de mode implicite.
- **Téléchargement.** La première source dont le verdict est `disponible` est retenue, ce verdict reposant sur le manifeste signé seul.
  - La release est publiée **en parties de 4 Mio** (`fabrique_release.py --parties-mio 4`, ajouté au workflow `release-trigkey.yml`).
  - Deux bornes l'imposent : le client HTTPS du noyau plafonne une réponse à 8 000 000 octets, et un fichier RAMFS à 64 Mio.
  - Chaque partie est vérifiée contre l'index (taille, SHA-256), et **le flux entier contre le manifeste signé** (taille, SHA-256). Une partie non conforme est refusée, et rien n'est mis en place.
- **Reprise.** Une partie qui échoue côté réseau est retentée (4 essais, pause croissante). Les parties déjà vérifiées sont conservées. Une partie reçue mais différente de l'index n'est **pas** retentée.
- **Installation.** L'agent met la release en place dans `/update/` et pose `pret` en dernier. `updated` (ring 3, seul détenteur de la capacité d'installation) l'installe dans le slot **inactif**. Le noyau revérifie tout à `begin` et à `finish` : signature, matériel, plancher, empreinte relue sur disque.
- **Après installation.** Redémarrage consenti, puis le chargeur démarre le candidat, puis commit de santé ou rollback automatique.
- **Release en échec.** Une release qui a échoué sa santé sur cette machine (son slot est Mauvais) n'est **jamais reproposée** (`release_deja_en_echec_ici`). Une release plus récente l'est.
- **CA de banc.** Feature `banc-ca-dev` plus `BOUCHAUD_CA_BANC_DEV`, pour servir l'HTTPS de l'hôte sous QEMU. La variable seule fait échouer la construction. `tools/verifie-cle-dev.py` refuse la feature dans `default`, toute mention dans un chemin de livraison, et toute pose de la variable hors du banc A/B.

| Banc QEMU TCG | Ce qui est exigé | Résultat local |
|---|---|---|
| `ota-complet` | A (release 1000, noyau v1) télécharge en **TLS 1.3** (chaîne vérifiée, CA de banc) la release 1001 : 24 parties. La partie 5 est **coupée une fois** par le serveur et doit être reprise. `updated` installe dans B, puis redémarrage consenti, B démarre (**autre noyau** : `BOUCHAUD_BUILD commit=ota-v2-…`), commit de santé, plancher à 1001. Au 3ᵉ démarrage, l'agent est `a_jour`, sans nouveau téléchargement. STATE : compteur 1, 2, 3 | `A* B+ B=`, `AB_OTA_COMPLET_OK` |
| `ota-rollback` | Même chaîne, mais la release 1001 a un service critique défaillant : trois démarrages B refusés, puis **retour automatique à A**. STATE : compteur strictement croissant sur les 5 démarrages. Plancher **inchangé**. Au retour sur A : `release_deja_en_echec_ici`, **aucun** retéléchargement | `A* B- B- B- A=`, `AB_OTA_ROLLBACK_OK`, compteur 1 à 5 |

**Limites.**
- Les parties téléchargées vivent en RAM : la reprise vaut à l'intérieur d'un démarrage, pas à travers un redémarrage.
- Une source dont l'image ne vérifie pas n'entraîne pas de repli sur une autre source dans le même cycle : le cycle suivant réessaie.
- Le tout-automatique n'existe pas.
- L'image de production n'embarque pas encore `ota.conf`.
- **Physique** : non testé sur le Trigkey (RTL8168, horloge, racines publiques vers github.com, coupure secteur pendant l'écriture).

## Agent OTA, tranche 1 (`BOUCHAUD_OTA_V1`)

- **Fonctionnement.** Tâche noyau `ota`, lancée au démarrage. Elle attend un bail réseau, puis vérifie à chaque cycle (au démarrage, puis toutes les 6 h) chaque ligne `source=` de `/etc/bouchaud/ota.conf`.
  - Elle télécharge `<source>/manifest` et `<source>/manifest.sig`, en HTTPS (TLS et racines du noyau) ou en HTTP pour un miroir local.
  - Seul un statut **200** est accepté.
  - Elle décide **uniquement sur le manifeste signé**, jamais sur `release.json`, avec `update-core::ota::decide`. C'est la même politique que `begin` : clé de confiance, manifeste v2 canonique, matériel, plancher anti-retour, et release plus récente que le slot courant.
  - Elle journalise `BOUCHAUD_OTA_VERIFIE source=… resultat=disponible|a_jour|refuse raison=…`.
- **Canal stable.** La source est `https://github.com/<owner>/<repo>/releases/latest/download`. Le « latest » de GitHub est la dernière release **non** prerelease, c'est-à-dire une release promue par `release-trigkey.yml`.
- **Fail closed.** Sans clé de production engagée, chaque source est refusée (`aucune_cle_de_confiance`).
- **Ce que la tranche 1 ne fait pas.** Elle ne télécharge pas l'image, n'installe rien et ne redémarre pas : une installation demandera le consentement (tranche 2). Le mode tout-automatique n'existe pas. L'image de production n'embarque pas encore `ota.conf`, l'agent y est donc inactif (`BOUCHAUD_OTA_INACTIF`).
- **Preuves.**
  - HOST : `tools/update/test_ota.rs`. Il couvre les cas « disponible », « à jour » et « installation neuve », plus des témoins de refus : aucune clé, autre clé, octet ou bit altéré, rétrogradation signée, autre matériel, v1, non canonique, registre illisible.
  - QEMU TCG : `run_ab_rollback.sh ota-verifie`, avec une carte e1000 et le réseau utilisateur de QEMU. L'hôte sert six sources. Le banc exige exactement six verdicts : `ok` disponible (1001), `egal` à jour (1000), `vieux` rétrograde, `arm` matériel incompatible, `autre` signature non fiable, `absent` injoignable. Il exige aussi qu'**aucune** installation ne démarre et que le serveur ait réellement été sollicité.
  - **Défaut trouvé par ce banc et corrigé** : un 404 était lu comme un corps de manifeste.
- **Non qualifié** : TLS vers github.com depuis le Trigkey (RTL8168, horloge, racines) reste `BLOCKED_PHYSICAL`.

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

## Procédure Windows → Trigkey

La procédure complète est dans `TRIGKEY_PREMIERE_UTILISATION.md` : préparation du poste, clé de validation, installation sur SSD dédié, checklist physique, retour arrière, configuration GitHub, mise à jour au quotidien.

- **Scripts** (`tools/windows/`) :
  - `Build-BouchaudOS.ps1`, `Test-BouchaudOS.ps1`, `Prepare-TrigkeyUSB.ps1`, `Release-BouchaudOS.ps1` ;
  - ils orchestrent dans WSL 2 les mêmes outils Linux que la CI.
- **Niveau de preuve** : **non qualifiés sur Windows**.
  - La CI exécute sous `pwsh` les chemins de refus de `Prepare-TrigkeyUSB.ps1`, avec des disques simulés (`tools/ci/test-scripts-windows-trigkey.ps1`).
  - Aucune écriture physique n'a été faite.
- **Côté Trigkey** : l'agent OTA de l'image livrée vérifie, télécharge et prépare la release **stable**, puis attend `bo-update consentir`.
  - Prouvé en QEMU seulement (`ota-complet`, `ota-rollback`, test de livraison).
  - Rien n'est fait sur le matériel.

## Risques de perte de données

- `installer --go --ecrase` (installateur noyau existant) détruit les partitions étrangères du NVMe. **Ne pas l'utiliser sur le Trigkey.**
- Une clé privée perdue empêche toute mise à jour signée.
- Une clé privée divulguée permet d'installer n'importe quoi au-dessus du plancher. Il faut alors la révoquer en urgence.
- Le plancher anti-retour ne descend jamais. Une release publiée par erreur avec une séquence très haute bloque toutes les suivantes.
  - Il faut alors republier avec une séquence encore plus haute.
  - D'où la garde de monotonie, et la saisie manuelle de la séquence.
