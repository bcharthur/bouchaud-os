# Trigkey : première utilisation, installation, mises à jour

Toutes les commandes ci-dessous **existent dans le dépôt**.
Les scripts Windows sont dans `tools/windows/` ; ils orchestrent les outils Linux du dépôt, exécutés dans **WSL 2**.

**Niveau de preuve à ce jour : QEMU (TCG/OVMF) seulement.**
Rien n'a été exécuté sur le Trigkey, ni sur un vrai Windows. Chaque ligne de la checklist physique est `NOT_RUN` tant que tu ne l'as pas faite.

## 0. Préparer le poste Windows (une fois)

1. **WSL 2** avec Ubuntu : `wsl --install -d Ubuntu`.
2. **Dans Ubuntu (WSL)** :
   ```
   sudo apt install -y build-essential musl-tools binutils python3 git qemu-system-x86 ovmf
   ```
   Puis rustup, avec la toolchain du dépôt (`rust-toolchain.toml`).
3. **GitHub CLI** (`gh auth login`) et **QEMU pour Windows**, pour le bureau interactif.

## 1. Ce que tu peux faire MAINTENANT

| Étape | Commande Windows (PowerShell, à la racine du dépôt) | Résultat attendu |
|---|---|---|
| Construire l'image de livraison | `.\tools\windows\Build-BouchaudOS.ps1` | `BOUCHAUD_BUILD_WINDOWS_OK … livrable=oui` (avec Ladybird récupéré de la CI) |
| La tester comme un utilisateur (WSL) | `.\tools\windows\Test-BouchaudOS.ps1 -Mode Livraison -ExigeLadybird` | `LIVRAISON_QEMU_OK` |
| Ouvrir son bureau dans une fenêtre | `.\tools\windows\Test-BouchaudOS.ps1 -Mode Bureau` | bureau Atelier, souris et clavier, sur une copie de l'image |
| Lister les disques | `.\tools\windows\Prepare-TrigkeyUSB.ps1 -Mode Liste` | identités `Modèle|Série|Octets` |
| Clé de validation : simulation | `.\tools\windows\Prepare-TrigkeyUSB.ps1 -Mode Validation -Disque N -Identite '…'` | `PLAN …` puis `USB_DRY_RUN` |
| Clé de validation : écriture | même commande + `-Execute`, puis taper `ECRIRE <SÉRIE>` | `BOUCHAUD_USB_ECRITE … relu=ok` |

Remarques :
- **`Test-BouchaudOS.ps1`** : les scripts Windows ne sont **pas qualifiés sur Windows**. La CI les exécute sous `pwsh` Linux, refus compris.
- **Clé de validation (`-Execute`)** : l'écriture est **non qualifiée** (aucune écriture physique faite par la session).
- **Ce que contient la clé de validation** : le même système que l'image de livraison, démarré en live.
  - Elle est compilée avec `cle-usb-test` : aucune écriture, vidange, formatage ni effacement vers un disque **interne** (ATA, NVMe).
  - Seule la clé elle-même, reconnue par son GUID GPT, est inscriptible.
  - Détails : `TRIGKEY_CLE_USB_TEST.md`.

## 2. Ce qui exige ta présence physique

Checklist de qualification. Résultats : `PASS`, `FAIL` ou `NOT_RUN`.
Pour chaque ligne, conserve la **preuve** dans `target/physique/<date>/` : photo de l'écran, journal série si disponible, boîte noire extraite de la clé avec `tools/reference/extract-blackbox.py`.

| # | Vérification (clé de validation, disques internes en lecture seule) | Preuve à conserver | Résultat |
|---|---|---|---|
| P1 | Démarrage UEFI depuis la clé (menu de démarrage du Trigkey) | photo | NOT_RUN |
| P2 | CPU (8 cœurs Ryzen 7 5700U) et mémoire (32 Go) vus | photo de « Moniteur » | NOT_RUN |
| P3 | Pilotes PCI (`lspci` dans le Terminal) | photo | NOT_RUN |
| P4 | Affichage (résolution native, pas d'artefacts) | photo | NOT_RUN |
| P5 | Clavier et souris (USB) | vidéo courte | NOT_RUN |
| P6 | Ethernet RTL8168 : lien, DHCP (`ifconfig`), DNS (`nslookup example.com`) | photo | NOT_RUN |
| P7 | TLS et horloge (`https example.com`, `date`) | photo | NOT_RUN |
| P8 | Bureau Atelier, Terminal, DOOM | photos | NOT_RUN |
| P9 | Ladybird : page d'accueil, puis un site HTTPS | photo | NOT_RUN |
| P10 | `bo-update status` (version, commit, canal stable) | photo | NOT_RUN |
| P11 | Arrêt et redémarrage par le menu | constat | NOT_RUN |
| P11b | `bo-update verifier`, puis `bo-update status`. Chaîne TLS **réelle** de GitHub en mode strict : `derniere_verification` doit montrer `a_jour`, `disponible` ou `aucune_cle_de_confiance`, **jamais** `certificat_non_fiable`. Jamais vérifié depuis la session : son proxy intercepte TLS. | photo | NOT_RUN |
| P12 | Disques internes **intacts** après la session (ex. Windows du Trigkey démarre toujours) | constat | NOT_RUN |

| # | Installation sur SSD dédié | Preuve | Résultat |
|---|---|---|---|
| I1 | SSD **dédié** (aucune donnée importante), branché au PC Windows | — | NOT_RUN |
| I2 | `Prepare-TrigkeyUSB.ps1 -Mode InstallationSSD -Disque N -Identite '…'`, en simulation | sortie `PLAN` | NOT_RUN |
| I3 | Même commande + `-Execute`, puis phrase `EFFACER <SÉRIE>` | `INSTALL_OK … relu=ok state_mio=…` | NOT_RUN |
| I4 | SSD monté dans le Trigkey, **sans clé USB** : démarrage sur l'Atelier | photo | NOT_RUN |
| I5 | Créer un fichier dans « Fichiers » ou le Terminal, redémarrer, le retrouver | photos | NOT_RUN |
| I6 | `bo-update status` : `slot_actif=A`, état de `STATE` | photo | NOT_RUN |

Le Trigkey possède **un seul emplacement NVMe** : le SSD dédié remplace temporairement le disque d'origine. Garde ce dernier hors de la machine pendant les essais.

## 3. Ce qui est encore bloquant

| Blocage | Nature | Ce qui manque |
|---|---|---|
| Clé de **production** absente | `BLOCKED_HUMAN` | Tant qu'aucune clé n'est engagée, **toute** release est refusée par la CI, et **toute** installation OTA par le système (fail closed). Voir §5. |
| Environnements GitHub `trigkey-testing` et `trigkey-stable` | `BLOCKED_HUMAN` | À créer : §5. |
| Check requis « Unix & Filesystem Reliability » | `BLOCKED_HUMAN` | Droits administrateur du dépôt. |
| Matériel Trigkey (RTL8168, affichage, USB, NVMe réel, coupure secteur) | `BLOCKED_PHYSICAL` | §2. |
| Canal `testing` automatique | non fait | GitHub n'offre pas d'URL « dernière prerelease » : seul le canal **stable** est automatique. |
| Reprise d'un téléchargement à travers un redémarrage | partiel | L'interruption est détectée dans STATE (`bo-update historique` : `telechargement_interrompu`), et le téléchargement recommence depuis la première partie, sous signature (QEMU : `ota-reseau`). La reprise **d'octets** n'est pas faite : les parties vivent en RAM. |
| Politique OTA en ring 3 | en cours | L'agent est encore dans le noyau (`src/kernel/update/ota.rs`). Il installe via `updated`, en ring 3, seul détenteur de la capacité. |
| Incident intermittent `ota-complet` (ec0ea87) | surveillé | Voir `TRIGKEY_INSTALL_UPDATE.md`. |

## 4. Retour arrière

- **Mise à jour défaillante** : c'est **automatique**. Un candidat qui ne confirme pas sa santé en 3 démarrages est abandonné, et le chargeur revient au slot précédent. STATE est intacte, et la release en échec n'est plus jamais reproposée (prouvé en QEMU : `ota-rollback`).
- **Retour manuel** : au démarrage, maintenir `R`. On arrive dans la console RECOVERY ; y taper `rollback --confirme` (l'actif devient la version précédente bonne) puis `redemarre`.
- **Abandon de Bouchaud OS** : remettre le SSD d'origine du Trigkey. Le SSD dédié n'a jamais touché le disque d'origine.

## 5. Configuration GitHub (à faire toi-même ; rien n'a été fait en ton nom)

1. **Clé Ed25519 hors ligne**, sur une machine sans réseau :
   ```
   openssl genpkey -algorithm ed25519 -out release-2026-11.pem
   python3 tools/release/cle_publique.py release-2026-11.pem --nom release-2026-11
   ```
   - Mets la ligne affichée dans `src/kernel/update/cles_production.rs`, par une PR relue. **Seule la clé publique** va dans le dépôt.
   - Garde la clé privée en deux copies chiffrées, hors ligne. **Jamais** sur le Trigkey.
2. **Settings → Environments → `trigkey-testing`** :
   - relecteur requis : toi ;
   - branches de déploiement : `main` seulement ;
   - secret `BOUCHAUD_RELEASE_ED25519_PEM` : le contenu PEM de la clé privée.
3. **Settings → Environments → `trigkey-stable`** : relecteur requis toi, `main` seulement, aucun secret.
4. **Settings → Branches → règle de `main`** : ajouter les checks de « Unix & Filesystem Reliability » aux checks requis.
5. **Rotation**, en trois étapes :
   1. engager la nouvelle clé publique à côté de l'ancienne ;
   2. publier une release signée par l'**ancienne** ;
   3. remplacer le secret, puis retirer l'ancienne clé dans une release suivante.

## 6. Mise à jour au quotidien

1. Coder sous Windows (RustRover), pousser une branche, ouvrir une PR. La CI qualifie au SHA exact, image de livraison comprise.
2. Fusionner dans `main` quand tout est vert.
3. Publier une release testing :
   ```
   .\tools\windows\Release-BouchaudOS.ps1 -Publier -Version 2026.11.20-1 -Sequence 1
   ```
   - Approuve l'environnement `trigkey-testing` dans GitHub Actions.
   - La CI construit l'image de livraison (Ladybird compris, sinon refus), la teste, la signe et la publie en prerelease `trigkey-s1-2026.11.20-1`.
4. Vérifier les artefacts publiés :
   ```
   .\tools\windows\Release-BouchaudOS.ps1 -Verifier -Etiquette trigkey-s1-2026.11.20-1
   ```
5. Promouvoir en stable :
   ```
   .\tools\windows\Release-BouchaudOS.ps1 -Promouvoir -Etiquette trigkey-s1-2026.11.20-1
   ```
   Approuve `trigkey-stable`.
6. **Sur le Trigkey, sans le PC.**
   - Au démarrage, au retour du réseau, ou toutes les 6 h, l'agent voit la release stable, la télécharge, la vérifie (signature, matériel, anti-retour, empreintes) et l'installe dans le slot inactif.
   - Ensuite, `bo-update status` affiche `prochaine_installation=prete …`.
   - Tape `bo-update consentir` pour redémarrer, ou configure `redemarrage=fenetre:HH-HH:MIN`.
7. **Après le redémarrage** : `bo-update status` affiche la nouvelle `version` et le nouveau `commit` ; `bo-update historique` trace tout.
