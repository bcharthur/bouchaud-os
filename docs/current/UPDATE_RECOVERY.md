# Mise à jour atomique, santé, rollback et recovery — conception et frontières

Ce document décrit **comment** la plateforme atomique fonctionne, **ce qui la prouve** et **où elle s'arrête**. Les verdicts, les SHA et les runs vivent dans un seul endroit : [status.json](status.json) (`atomic_platform`). Niveau de preuve de toute cette chaîne : **HOST + TCG**. Rien n'est KVM ni PHYSICAL.

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

La **politique** (candidat, bon, mauvais, essais, rollback, échecs répétés, commit de santé, manifeste, signature) est dans `system/update-core` (`no_std`, sans allocation, sans dépendance). Le noyau n'en porte que le mécanisme.

## Disposition disque (GPT, `disposition.rs::plan_ab` + `uefi-image-builder --disque-ab`)

| Partition | Type GUID | Contenu |
|---|---|---|
| BOUCHAUD-ESP | EFI | le shim seul (`BOOTX64.EFI`) |
| BOUCHAUD-META | `B0DC8A5D-0002-…` | 1 Mio brut : copie 0 (bloc 0), copie 1 (bloc 1), compteur d'échecs (bloc 2) |
| BOUCHAUD-SYS-A / SYS-B | `…-0003-` / `…-0004-` | FAT : `BOUCHAUD-LOADER.EFI`, noyau, `boot.json`, ramdisk (userland système) |
| BOUCHAUD-RECOVERY (option) | `…-0005-` | FAT : système minimal (même format qu'un slot) |
| STATE | `…-0001-` | persistance utilisateur (`fs::persistance`) |

Partitions trouvées **par type GPT sur le disque de démarrage**, jamais par numéro.

## Invariants

- **I1–I6** (update-core) : actif toujours Bon ; essais bornés ; jamais de boot d'un slot Vide/Mauvais ; deux copies, la plus ancienne réécrite ; aucune installation sur l'actif ni sur le slot en cours (`running_slot != update_target`).
- **Chargeur** : un candidat n'est booté que si son essai consommé est durable (écriture, flush, relecture décodée) ; sinon l'actif. Deux copies invalides → RECOVERY, ou arrêt sans partition RECOVERY ; jamais un boot deviné.
- **Passage** : le noyau lit le slot réellement booté dans le passage ; il ne le déduit pas de META.
- **Commit de santé** : seul le candidat booté comme tel, sur l'enregistrement exact d'où il a booté, est promu ; vidange réelle exigée ; un boot RECOVERY ne confirme rien.
- **Échecs répétés** : 3 boots de l'actif sans santé → RECOVERY ; rollback = compteur à 0 ; compteur illisible = 0.

## Chaîne d'installation et frontière TOCTOU

```text
updated lit /update/manifest + .sig      (fichiers : données NON fiables)
  ↓ bo_update_install_begin(slot, octets, sig)
noyau : copie les octets (usercopy) → verifie_signe(cles_de_confiance)      SIGNATURE TRUSTED
        → session = { pid, slot, image_sha256, image_octets, essais } capturés ICI
  ↓ write(off, buf)   borné au slot cible, aligné, ≤ 1 Mio
  ↓ verify(len)       vidange réelle, puis relecture DU DISQUE par morceaux → SHA-256 courant
  ↓ finish(sha, len, essais)   doit égaler la session ; tout relu ; SHA relu = SHA signé     HASH VALID
noyau : marque_candidat(slot, sha) → META persistée (copie ancienne, vidange, relecture)
```

- **Source de vérité** : ce que le noyau a capturé à `begin` (octets signés, puis valeurs analysées), puis les octets **relus du slot**. Modifier `/update/manifest` après `begin` n'a aucun effet (le noyau a sa copie). Modifier `/update/system.img` change ce qui est écrit, donc ce qui est relu : l'empreinte relue ne correspond plus à l'empreinte signée, et `finish` refuse.
- **Session liée au pid** : un autre processus — y compris un `updated` relancé — doit refaire `begin` ; il ne reprend aucune autorité de la session précédente.
- **Sérialisation** (`BOUCHAUD_INSTALL_SERIALISE_V1`) : `begin`, `write`, `verify` et `finish` s'exécutent entièrement sous un mutex dormant, E/S comprises. Sans lui, une écriture encore en vol d'un `updated` tué pouvait atterrir après le `begin` de son successeur, au pire sur une zone déjà relue, et `finish` aurait accepté une empreinte que le disque ne porte plus. Un détenteur ne peut pas mourir en le tenant : les attentes d'E/S cèdent le processeur sans parquer, et le mutex attend en `wait_noyau`, si bien que la mort prend effet à la frontière de l'appel.
- **Battements** : `updated` répond à `serviced` avant chaque lecture, entre chaque morceau de 64 Kio, et entre chaque étape longue avant l'écriture (`BOUCHAUD_UPDATED_BATTEMENT_ETAPES_V1`) : lecture du manifeste, chaque `begin` (une vérification Ed25519 dans le noyau), auto-tests de refus. Sans ce dernier point, sous TCG, cette séquence enchaînait environ 1,4 s sans service, pour une tolérance de 4 × 250 ms. Une demande d'arrêt reçue pendant l'une de ces réponses est retenue et termine le travail. La taille vient d'une mesure (lecture paresseuse du ramdisk > 1 s pour 256 Kio sous TCG), pas d'un seuil. Le banc publie les durées de lecture, d'écriture et de relecture, ainsi que l'écart maximal entre deux services (`UPDATED_MESURE`), et échoue si un battement est manqué.

## Racine de confiance : Ed25519 (`system/update-core/src/ed25519.rs`)

### Provenance

| Élément | Origine | Différence |
|---|---|---|
| `car`, `sel`, `pack25519`, `unpack25519`, `fa/fz/fm/fs`, `inv25519`, `pow2523`, `add`, `cswap`, `pack`, `scalarmult`, `scalarbase`, `mod_l`, `reduce`, `unpackneg` | TweetNaCl 20140427 (`car25519`, `sel25519`, `pack25519`, `unpack25519`, `A/Z/M/S`, `inv25519`, `pow2523`, `add`, `cswap`, `pack`, `scalarmult`, `scalarbase`, `modL`, `reduce`, `unpackneg`) — domaine public | traduction 1-pour-1 ; l'index `(i+1)*(i<15)` de `car25519` devenu un `if` (même arithmétique : `o[0] += 38·(c−1)`) ; tableaux Rust bornés |
| constantes `D`, `D2`, `X`, `Y`, `I`, `L` | TweetNaCl | identiques |
| `neq` | `neq25519` | `!=` au lieu de `crypto_verify_32` : temps variable, voulu (entrées publiques) |
| SHA-512 | **pas** TweetNaCl : FIPS 180-4, constantes **calculées** (racines cubiques / carrées des premiers) | vérifié contre OpenSSL à chaque frontière de bloc |
| `verifie` | `crypto_sign_open`, forme détachée | hache `R‖A‖M` en flux ; **ajouts** : S < L (RFC 8032 §5.1.7) ; décodage strict de A et R (y < p, pas de zéro négatif, RFC 8032 §5.1.3) ; refus de A et R de petit ordre (règle libsodium) |

Vérification seulement (jamais de signature sur la machine) ; temps variable accepté car clé, message et signature sont publics.

### Défaut trouvé par la revue et corrigé

Avant `BOUCHAUD_ED25519_STRICT_V1`, le port se comportait **exactement comme OpenSSL 3** : il acceptait une clé de petit ordre (identité comprise), avec laquelle `R = [r]B, S = r` vérifie **n'importe quel message**, ainsi que les encodages non canoniques de clé (`y = 1 + p`, zéro négatif). L'attaquant ne choisit pas la clé de confiance, donc ce n'était pas exploitable avec une vraie clé. Mais une clé mal configurée (`0100…00`) aurait fait confiance à tout. Le correctif n'ajoute que des refus ; les équations sont inchangées.

### Preuves (HOST)

- `tools/update/test_signature.rs` (CI Fast) : vecteurs RFC 8032 §7.1 ; SHA-512 contre OpenSSL ; signatures OpenSSL acceptées et 768 inversions de bits refusées par message ; S + L ; clé et R pathologiques ; manifeste canonique et entrées hostiles.
- `tools/update/ed25519_adversarial.py --strict` (CI Fast) : 332 cas construits avec l'arithmétique de référence RFC 8032 §6, comparés à **OpenSSL** et **libsodium** :
  - les 8 points de torsion, en clé et en R ;
  - S ∈ {0, L, > L, 2²⁵⁶−1} ;
  - encodages limites de y et points hors courbe ;
  - contrefaçon universelle sur clé de petit ordre ;
  - signature du **détenteur de la clé** avec nonce r = 0 (R = identité, S = k·a), où l'équation sans cofacteur est vraie ;
  - 64 signatures aléatoires et leurs altérations.

  Résultat : Bouchaud = libsodium sur les 332 cas. Bouchaud ≠ OpenSSL sur 16 cas, tous des entrées de petit ordre ou non canoniques qu'OpenSSL accepte. Le script échoue si une référence manque.
- Contrôles de débordement actifs (comme le profil `dev` du noyau) : corpus entier + 20 000 entrées arbitraires, aucune panique.
- Preuve par mutation (chaque refus retiré séparément) :
  - refus de clé de petit ordre : 5 divergences, et le test hôte échoue ;
  - [8]P calculé en deux doublements au lieu de trois : 2 divergences ;
  - refus de R de petit ordre : 2 divergences, grâce aux cas r = 0, ajoutés après qu'une première mutation était passée inaperçue ;
  - refus du zéro négatif : **mutation équivalente**. Les seuls points d'abscisse nulle sont l'identité et le point d'ordre 2, déjà refusés comme petit ordre. Ce refus est une défense en profondeur ; aucun test ne peut le distinguer.

### Coût dans le noyau

La vérification s'exécute dans l'appel système `begin`, pendant que `serviced` attend le battement de `updated` (contrat v3 : 4 battements manqués de 250 ms, soit environ 1 s, puis mort).

La première version stricte calculait [8]P avec l'échelle complète de 256 bits, plus deux inversions. La vérification en devenait environ 2× plus lente sur l’hôte. Sous TCG, cela a suffi pour que `serviced` tue `updated` pendant ses auto-tests de refus : `heartbeat_missed`, trouvé par le scénario `mise-a-jour-morts`.

`BOUCHAUD_ED25519_PETIT_ORDRE_V2` calcule [8]P en trois doublements et teste l'identité en coordonnées projectives (X = 0, Y = Z), sans inversion. Coût mesuré sur l'hôte, `-C opt-level=2`, 200 vérifications du vecteur RFC 1 :

| Version | µs par vérification |
|---|---:|
| `21fca026` (non stricte) | 1465 |
| stricte v1 | 2868 |
| stricte v2 | 1539 |

Le seuil de battement n'a pas été touché.

### Décision : `KEEP_INTERNAL_IMPLEMENTATION`

`ed25519-dalek` (avec `curve25519-dalek`, `sha2`, `digest`, `signature`, `subtle`, `zeroize`) est `no_std`, maintenu et largement audité. Son intégration casserait pourtant trois contraintes actuelles :
- `update-core` est compilé tel quel par le shim UEFI, par le noyau en `build-std`, et par les tests hôte en `rustc --test #[path]`, sans cargo ;
- elle ajouterait sept crates à la base de confiance du chargeur et du noyau ;
- il faudrait vendre les dépendances pour une construction reproductible.

Pour une vérification seule, sur entrées publiques, le port de 250 lignes est plus petit et intégralement relu. Il est adossé à une suite adverse contre deux implémentations reconnues, exécutée à chaque modification. **À réévaluer** si la machine doit un jour signer, ou si une exigence de temps constant apparaît.

## Clés de confiance

| Clé | État | Comment |
|---|---|---|
| Production | **NOT_CONFIGURED** — toute installation est refusée (`aucune_cle_de_confiance`) | à créer hors dépôt sur décision humaine (hors ligne / HSM, rotation, sauvegarde, révocation) ; seule la moitié publique entre dans `cles_de_confiance` |
| Développement | banc seulement | exige **à la fois** la feature `banc-cle-dev` et `BOUCHAUD_CLE_VERSION_DEV`. La variable seule fait **échouer la construction** (`BOUCHAUD_CLE_DEV_GARDE_V1`). `tools/verifie-cle-dev.py` vérifie que ni la release, ni l'image Trigkey, ni `default` ne la posent, et qu'aucune clé privée n'est commitée. La clé du banc naît et meurt dans un répertoire temporaire. Le noyau imprime au boot `BOUCHAUD_UPDATE_CLES confiance= production= developpement=`. |

Le scénario QEMU `sans-cle` prouve une image normale : `confiance=0`, `begin` refusé même pour un manifeste correctement signé, B reste vide, la clé publique du banc est absente de l'ELF noyau et aucune clé privée n'apparaît dans les sorties du banc.

## Authenticité ≠ anti-rollback

Le manifeste signé prouve **qui** a publié une image, pas **qu'elle est plus récente** que celle installée. Une ancienne version correctement signée peut aujourd'hui être réinstallée : **`ANTI_ROLLBACK_POLICY = NOT_IMPLEMENTED`**.

Contrainte pour le futur mécanisme : distinguer

- le **rollback local** (candidat B mauvais → retour à A ; `rollback --confirme` en recovery vers la version précédente Bonne), qui doit rester possible sans réseau ni signature nouvelle ;
- le **downgrade hostile** (un attaquant fournit une ancienne release signée → installée), qui doit être refusé : numéro de version monotone signé dans le manifeste, plancher persisté et avancé seulement après un commit de santé, révocation de versions vulnérables.

## Recovery : frontière gelée

`kernel::update::recovery` est un **mécanisme minimal de secours** dans le noyau : lecture seule par défaut (`etat`, `manifestes`, `verifie A|B`, `boite-noire`, `panique`, `redemarre`, `arrete`), deux écritures nommées avec `--confirme` (`rollback`, `remet-compteur`). Il ne doit pas devenir un second système dans le noyau. Toute fonctionnalité plus riche (réseau, réinstallation, diagnostic) ira dans un environnement de recovery **ring 3**.

## Boîte noire par boot

`BOUCHAUD_BOOT_RECORD boot_id system_slot system_version boot_reason candidate_attempts_left previous_boot_result meta_sequence`, imprimé dès que le stockage est prêt. `previous_boot_result` ne dit que ce que les enregistrements prouvent. `logd` estampille chaque événement avec ce `boot_id`.

## Limites connues

- Image Trigkey : le ramdisk doit embarquer `serviced`, `healthd`, `updated`, `logd`, sinon tout candidat est retiré (voulu). Ce n'est pas encore intégré à `build_trigkey_image.sh`, donc **aucun test physique A/B n'est prêt à réussir**.
- Le commit exige un volume à vidange réelle : un NVMe sans cache volatil (VWC=0) est refusé (sûr ; à assouplir en distinguant « pas de cache » de « inconnu »).
- `verifie` (recovery) mesure le volume FAT (taille du BPB).
- META et slots : prouvés sous QEMU NVMe seulement.
