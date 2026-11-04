# Stockage S1 — dossiers personnels persistants, écriture différée, métadonnées

Branche `claude/stockage-s1-20261104`, issue de la tête de la PR #244. P13, Atomic A/B et l'ordonnanceur ne sont pas touchés. Le protocole de commit de `/persist` (deux demi-zones, deux superblocs, barrières) est inchangé.

## Ce qui manquait

| Manque | Conséquence |
|---|---|
| Rien n'atteignait le disque sans `fsync` ou `sync` | Un fichier créé par le shell, par Fichiers ou par une application qui oublie `fsync` était perdu à la première coupure. |
| La table ne retenait que les **fichiers non vides** | Dossiers, fichiers vides, droits et propriétaires disparaissaient au redémarrage : un dossier personnel ne pouvait pas être persistant. |
| Pas de dossier personnel durable | L'accueil de la session de bureau était `/`, en RAMFS. |
| Les applications n'ouvraient pas un fichier choisi | Notes n'éditait que `bouchaud-notes.txt`. |

## Ce qui est livré

### 1. Métadonnées dans la table (`BOUCHAUD_PERSIST_META_V1`, `persistance/meta.rs`, pur)

Une entrée fait 256 octets : chemin (0..240), taille (240..248), et 8 octets jamais écrits. Les métadonnées y tiennent exactement :

| Octet | Contenu |
|---|---|
| 248 | Genre : `0` (fichier d'avant), `F` (fichier) ou `D` (dossier) |
| 249 | Version |
| 250..256 | `mode`, `uid`, `gid` (trois u16) |

La compatibilité vaut **dans les deux sens** :
- un disque d'avant se lit comme avant ;
- un disque de ce noyau, relu par un noyau d'avant (retour A/B), restaure les mêmes fichiers non vides, car l'ancien montage saute les entrées de taille nulle ;
- la somme de contrôle du superbloc couvre la table entière, métadonnées comprises.

La collecte retient maintenant chaque dossier, **avant** son contenu, ainsi que les fichiers vides. Le montage recrée les dossiers et applique droits et propriétaire.

### 2. Écriture différée (`BOUCHAUD_ECRITURE_DIFFEREE_V1`)

- `ramfs::touche` (point de passage de toute mutation) et `libere_noeud` incrémentent une génération quand le nœud est sous `/persist`.
- Le fil noyau `ecriture-differee` (une observation toutes les 250 ms, sans verrou) lance `persistance::synchronise`, la même transaction que `fsync` :
  - après **1,5 s de calme**, si bien qu'une rafale donne une seule transaction ;
  - ou au plus tard **8 s** après la première modification non écrite ;
  - après un échec, le délai double jusqu'à 60 s.
- La référence du fil est la génération figée à la fin du montage : les écritures faites avant son lancement, celles d'un autorun par exemple, sont écrites elles aussi.
- La politique est pure (`fs/ecriture_differee_politique.rs`).

**Borne de perte après une coupure** : environ 8 s pour ce qui n'a pas été `fsync`é, et zéro pour ce qui l'a été, comme avant.

### 3. Dossiers personnels (`BOUCHAUD_DOSSIERS_PERSONNELS_V1`)

- Quand la machine a une zone de persistance, le dossier personnel est `/persist/home/<compte>`. Sinon, c'est l'accueil déclaré du compte, comme avant.
- À l'ouverture du bureau, il est créé s'il manque, ainsi que **Documents, Images et Téléchargements** : mode `0700`, au nom du compte. Rien d'existant n'est écrasé.
- Le nom de compte est validé : aucune évasion par `..` ou `/`.

### 4. Ouvrir et enregistrer depuis les applications (`BOUCHAUD_NOTES_OUVRIR_V1`)

- Fichiers s'ouvre sur le dossier personnel ; Accueil le désigne, et Documents, Images et Téléchargements n'y sont plus « absents ».
- Un double-clic sur un fichier texte, code ou sans extension (≤ 256 Kio), ou l'entrée **Ouvrir dans Notes** du menu contextuel, ouvre une fenêtre « Notes — nom ». L'application dépose une demande que le bureau prend au tour suivant.
- Notes ouvre et enregistre **n'importe quel chemin**. Sous `/persist`, F2 est une transaction disque immédiate (sémantique `fsync`), et la marque PERSIST suit le fichier réellement édité.
- La barre d'état de Fichiers indique « Sur le disque · persistant » ou « RAMFS · non persistant », selon le dossier courant.

## Preuves

| Preuve | Résultat |
|---|---|
| `tools/ci/run_dossiers_persistants.sh` (**nouveau**) : 2 démarrages sur un disque ; le premier écrit sans `fsync`, crée un dossier vide et un fichier vide en `0600`, lance le bureau, puis est **tué par SIGKILL** | `DOSSIERS_PERSISTANTS_OK`. D1 fichier relu, D2 dossier vide et fichier vide relus, D3 droits `0600` relus, D4 `~/Documents`, `Images`, `Téléchargements` relus, D5 une transaction d'écriture différée (8 entrées, 143 ms sous TCG) |
| `run_coupure_persistance.sh` 6 cycles (écrire, fsync, renommer, fsync du dossier, coupure) | `COUPURE_PERSISTANCE_OK`, 0 incohérence, 0 régression, 0 invention |
| `run_persist_cache.sh` | `PERSIST_CACHE_OK`. L'arbre de cache écarté compte 2103 entrées, soit 2101 fichiers et ses 2 dossiers, qui sont maintenant des entrées. L'assertion reste exacte et l'explication est en commentaire. |
| `run_faute_persistance.sh` 3 répétitions | `FAUTE_PERSISTANCE_OK`, 0 échec, instantané 64/64 |
| Tests hôte | `tools/fs/test_persist_meta.rs` (6 : aller-retour, disque d'avant, genre et version inconnus, bornes) ; `tools/fs/test_ecriture_differee.rs` (8 : rien à écrire, calme, rafale, plafond, échec et repli, écriture pendant la transaction, chemins) ; `test_atelier` 88 |
| Capture QEMU réelle | `atelier/capture-s1-fichiers-notes-persistants.png` : Fichiers dans `~/Documents` (« Sur le disque · persistant »), double-clic sur `lisez-moi.txt`, Notes ouvert en PERSIST |

## Limites (prochains incréments du chantier stockage)

- Une transaction réécrit la **table entière** et les fichiers modifiés, et l'instantané copie le contenu de `/persist` sous le verrou du RAMFS. C'est adapté à quelques mégaoctets, pas à un disque de travail de plusieurs gigaoctets : un vrai système de fichiers à blocs (allocation, journal) reste à faire.
- La zone est limitée à 2048 entrées et 64 Mio par demi-zone.
- `mmap` en écriture s'appuie sur `msync`, et le cache de pages ne sert que les mappings en lecture seule adossés au disque, comme avant.
- Pas de montage de volumes externes (FAT32 sur USB) dans Fichiers.
- Pas d'« Enregistrer sous » dans Notes.
- `tools/fs/verifie_collecte_ramfs.py` échouait **déjà avant** ce changement (même résultat sur l'arbre non modifié). Il n'est pas dans la liste des garde-fous de la CI ; il est signalé ici et laissé tel quel.
