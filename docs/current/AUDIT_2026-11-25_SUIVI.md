# Suivi de l'audit architectural du 25/11/2026 — chantiers A à O

Branche `claude/lucid-galileo-0g4429`, PR brouillon
[bcharthur/bouchaud-os#383](https://github.com/bcharthur/bouchaud-os/pull/383)
(aucune fusion sans accord). Point de départ : `main@5b6db15`.

**Niveaux de preuve** employés ici, et rien au-delà :
**IMPL** (écrit), **COMP** (compile), **HOST** (test hôte), **TCG** (banc QEMU
TCG local, avec témoin négatif quand il y en a un), **INTÉGRÉ** (exercé dans le
vrai bureau ou par un composant réel de l'OS). Aucun résultat KVM, WHPX ni
PHYSICAL dans ce document.

## 0. Incident de la tempête d'exec — cause établie, correctif mesuré

`tools/ci/run_tempete_exec.sh` (mode `complet` : fork + execve + fils +
mémoire, 3 lanceurs, SMP4) produisait, sur les images de la branche et **sous
forte charge de l'hôte (≥ 4 QEMU parallèles)**, des incidents : gel total,
arrêt muet de QEMU (triple faute), ou panique `page fault @ <adresse
utilisateur>` en mode noyau sur la pile d'un fil mourant.

**Cause (lecture du code, `f63dbe7`)** : `switch_to` efface le bit d'activité
du cœur sortant (`quitte_espace`) **avant** d'écrire le CR3 suivant
(`install`). Dans cette fenêtre, à IF=0, le cœur exécute encore sous
l'ancienne PML4. La branche rend désormais l'espace d'un mort **à sa mort**
(et `execve` l'ancien espace) sur la seule foi de ces bits : la PML4 et ses
tables pouvaient être réutilisées et réécrites sous un cœur qui les parcourait
encore. La référence ne rendait l'espace qu'à la récolte, bien plus tard :
d'où « défaut introduit par la branche, visible seulement sous charge ».

**Correctif** : `AddressSpaceIdentity::barriere_cr3` — un IPI de shootdown vers
**tous** les cœurs en ligne et l'attente de leurs ACK avant tout rendu d'espace
(mort, vidage de quarantaine, `execve`). Un cœur dans la fenêtre n'acquitte
qu'après avoir écrit son nouveau CR3.

| Image | Charge | Passages | Incidents |
|---|---|---|---|
| référence `main@5b6db15` | ×4 | 12 | 0 |
| branche avant correctif (A/B exit_mm) | ×4 | 6 / 6 | 1 triple faute / 0 |
| **branche avec barrière CR3** | ×4 (4 QEMU) + bancs | **32** | **0 au noyau** (31 terminés `echecs=0`, 2 000 à 5 000 barrières par run ; le 32e n'a jamais atteint le noyau : vCPU0 en mode réel à `0x7607`, chargeur affamé par 6 QEMU) |

Le taux initial était bas : la mesure **corrobore** le mécanisme, elle ne le
prouve pas seule. Statut : **corrigé, à confirmer** par une campagne plus
longue (et KVM) avant de déclarer la Memory Fabric stabilisée.

Deux défauts voisins trouvés en chemin, corrigés : l'OOM jugeait une victime
morte dès `zombie`, avant qu'elle ait rendu sa mémoire (`cffa6da`) ; la sonde
`memoire-mort` mesurait parfois un fils déjà mort.

### Coût mesuré de la barrière CR3

Deux paires TCG SMP2, QEMU 10.0.13, même tempête fork/exec, entre
`168ee875` et `f63dbe76`, avec ordre inversé pour la deuxième paire.
Avec un seul pair distant, les émissions de shootdown et leurs réémissions
représentent 14 828 → 32 010 puis 14 790 → 30 965 IPI. Sur environ 57 s
dans l'invité, le temps CPU noyau augmente de 62,56 → 63,32 s puis
62,89 → 64,34 s. Le p95 d'`execve` passe de 1,89 → 2,18 ms puis
2,18 → 2,42 ms. Le temps cumulé dans les barrières est de 2,73 et 3,16 s,
avec un maximum de 38 ms. Le débit de la phase complète varie entre les
paires. La mesure provient de noyaux instrumentés localement avec des
compteurs de durée, sans changement de la barrière livrée.

### Session manuelle WHPX et fermeture forcee

Sur `9f1014ca`, WHPX materiel confirme, SMP1, 8 192 Mio, le test utilisateur
a atteint le bureau en 2 642 ms depuis le noyau. Ladybird a presente sa
fenetre 17,33 ms apres le clic. Example.com a pris 873 ms et le delai depuis
le lancement du navigateur etait de 2 981 ms. Sept forks ont partage
16 346 pages COW sans copie immediate ni echec d'allocation. Les medianes
etaient de 3,957 ms pour fork et 1,176 ms pour execve. Le Mm du WebWorker
(20 105 pages) a ete rendu en 2 030 us. Aucun panic ni crash Ladybird
detecte, extinction QEMU avec le code 0.

Apres la fermeture forcee, le dernier releve WHPX indiquait 82 taches
zombies, 13 vivantes, 21 processus crees, aucun detruit et environ 1 700 Mio
de RAM. Sur `main@5b6db15` et `9f1014ca`, un controle TCG SMP1 avec un
client de test a trois processus et quinze fils reproduit la retention
apres le clic sur la croix. Les deux bras ont 15 taches zombies et aucun
processus detruit jusqu'a la reouverture, puis zero zombie et trois
processus detruits. Le chemin force recolte le registre de processus.
Les references des taches zombies, et donc leurs Mm, restent jusqu'au
recyclage des emplacements. Le chemin normal de dernier fil rend le Mm
plus tot sur le candidat. Le releve WHPX totalise les frames du systeme.

Observations de la meme session WHPX, a confirmer sur une reference
appariee si elles deviennent un sujet de travail. CPU proche de 99 % en
SMP1, 15 pics HID (maximum 233 845 us), 32 `PERSIST-SLOW` (mediane
environ 494 ms), attente ATA jusqu'a 341 ms, aucun GPU materiel et YouTube
charge en environ 30 s.

## 1. Commits de la branche

| SHA | Lot |
|---|---|
| `15a4800` | `Mm` remplaçable, vfork réel, `CLONE_CHILD_SETTID`, `getppid` |
| `8b043c2` | mémoire rendue à la mort, OOM contrôlé, récolte des orphelins |
| `d50b7d9` | suivi : bilan tempête, attente bornée |
| `4a3ec17` | revue : quarantaine des espaces de morts, sonde `CHILD_SETTID` brute, `meme_que` retiré |
| `33cd816` | fork Copy-on-Write, clés de futex stables, `MAP_SHARED` anonyme réellement partagé |
| `c64ad25` | course TOCTOU de la casse COW corrigée, page zéro partagée |
| `ca18346` | ABI native 1.7 : `bo_shm_map` / `bo_shm_unmap` |
| `b0a7144` | Notes en application ring 3, atlas de police d'Atelier |
| `cd76e1f` | `composited` projette l'image de l'écran |
| `7eb61a2` | VFS : table de montages, devfs/procfs montés, `/proc` vivant |
| `62ef599` | option témoin `banc-sans-espace-rendu` |
| `de7c67f` | quota des `SharedRegion` par processus créateur |
| `cc55dc0` | suivi d'audit A à O |
| `168ee87` | gardes d'architecture rétablies (8 régressions de la branche) ; ABI native sans compat Linux (`kernel::projection`) |
| `f63dbe7` | **barrière CR3** avant de rendre un espace (incident §0) |
| `cffa6da` | OOM : la victime tient sa mémoire jusqu'à l'avoir rendue |
| `f385bbb` + `ef72e1a` | VFS : répertoires des systèmes montés (`getdents`, `stat`, `/proc/<pid>`) — `f385bbb` seul ne compile pas (raté de découpage, corrigé par `ef72e1a`) |
| `a38db1c` | icône d'une application ring 3 choisie par son image (régression de Notes) |
| `c25b577` | Journal en application ring 3 ; trace réservée à `DEBUG` |
| `359b8d1` | sonde : refus de la trace sans `DEBUG` (témoin discriminant) |

## 2. Chantiers A à O

### A — Ordonnanceur, préemption, multicœur
- **Initial** : files par cœur, préemption, IPI, histogrammes de latence par classe.
- **Réalisé** : rien dans la politique de placement (verrouillée par un protocole témoin KVM).
- **Preuve** : —
- **État** : inchangé.
- **Prochaine tranche** : histogramme de latence ready→running par CPU, puis banc « tâche interactive prête derrière un calcul utilisateur, cœur voisin libre », politique isolée derrière une option.

### B — Synchronisation, futex, cycle de vie
- **Initial** : `getppid` constant, `CLONE_CHILD_SETTID` ignoré, orphelins zombies à vie ; clés de futex physiques.
- **Réalisé** : `getppid` réel ; `CHILD_SETTID`/`CLEARTID`/`SETTLS` ; récolte des orphelins (deux règles) ; **clés de futex `(PML4, adresse)` pour les pages privées**, `(0, physique)` pour les empruntées (indispensable à la COW : sinon réveils mêlés entre père et fils, et réveil perdu après une copie).
- **Preuve** : TCG avec témoins (`run_vfork.sh … tid`, `run_orphelins.sh`, `cow-probe` étape `futex`).
- **État** : implémenté, qualifié TCG. Incident §0 : cause établie, corrigé, à confirmer.
- **Prochaine tranche** : campagne longue tempête (et KVM) ; signaux pendant l'attente vfork.

### C — Mémoire virtuelle et récupération (Memory Fabric)
- **Initial** : `fork` recopiait tout ; pas de page zéro ; espace d'un mort retenu jusqu'à la récolte ; OOM tuant le fautif.
- **Réalisé** :
  - `Mm` remplaçable (`MmCourant`) ; références du cache rendues par le `Mm` ;
  - **fork Copy-on-Write** : table de propriétaires (`src/kernel/memory/cow.rs`, pure), bits `PTE_COW`/`PTE_COW_ECRITURE`, copie ou promotion à la faute, part rendue après l'invalidation TLB, `mprotect` cohérent, échec d'allocation → politique OOM ;
  - **écritures du noyau** (`user_write`, usercopy native, `clear_child_tid`, `AddressSpace::write`) : cassent la COW avant d'écrire ;
  - **page zéro partagée** pour les lectures anonymes ;
  - mémoire rendue à la mort (attente bornée, quarantaine si un cœur tient encore le CR3, **barrière CR3** avant tout rendu d'espace) ; OOM contrôlé (victime = plus gros consommateur, attendue jusqu'à ce qu'elle ait **rendu** sa mémoire ; la quarantaine est vidée avant de choisir) ;
  - `MAP_SHARED|MAP_ANONYMOUS` réellement partagé (défaut préexistant découvert par la sonde).
- **Mesures** (TCG) : fork d'un père de 64 Mio : **192 Kio** consommés contre **66 076 Kio** (référence) ; lire 128 Mio anonymes : **264 Kio** contre **131 332 Kio** ; `fork+execve` 30 → 19 ms ; zombie : mémoire rendue 14 ms après la mort, avant récolte.
- **Preuve** : HOST (`test_cow`, `test_oom`) ; TCG SMP1/2/4 avec témoins (`run_cow.sh`, `run_page_zero.sh`, `run_memoire_mort.sh` — 4/4 sous charge ×4) ; tempête ×4 : 31/31 runs au noyau sans incident (§0).
- **État** : implémenté et qualifié TCG ; la cause de l'incident §0 est corrigée, la stabilisation reste à confirmer sur une campagne longue.
- **Prochaine tranche** : campagne longue + KVM ; regrouper le shootdown du fork ; réclamation sous pression (pages COW) ; tables de pages vides (#379, non réimplémenté ici).

### D — Création, exécution, destruction des processus
- **Initial** : `posix_spawn` = fork à copie complète.
- **Réalisé** : vfork réel (père suspendu, `execve` installe un `Mm` neuf) ; fork COW.
- **Mesures** : `posix_spawn` 9,1 ms contre 30,3 ms (`fork+execve` avant COW), SMP2.
- **Preuve** : TCG avec témoins.
- **État** : implémenté, qualifié TCG.
- **Prochaine tranche** : cache des pages ELF entre processus.

### E — IPC natif et mémoire partagée
- **Initial** : `SharedRegion` lisible/écrivable seulement par appels (copie).
- **Réalisé** : ABI 1.7 `bo_shm_map`/`bo_shm_unmap` — la mécanique VM est dans un module neutre `kernel::projection`, partagé avec la compat Linux : le cœur natif ne dépend plus de `compat/linux` (garde `verifie-abi-native`) — (droits MAP/READ/WRITE, W^X, région tenue par les projections, héritée au fork, rendue au munmap/exec/mort) ; stockage aligné page dans l'arène physique du tas (aucune autre donnée ne partage ses frames, arène d'amorçage refusée) ; **quota par processus créateur** ; **adoption par `composited`**.
- **Mesures** : `composited` glisser une fenêtre : 32 → **0** appels de région par pas, 16,8 → 12,0 ms par pas (TCG, bruité).
- **Preuve** : TCG (`run_shm_map.sh`, témoin quota) ; INTÉGRÉ (`banc_bureau_ring3_entrees.py`).
- **État** : implémenté, intégré au compositeur ring 3.
- **Prochaine tranche** : projection des surfaces d'application (fenêtres) pour supprimer `FENETRE_LIT` ; files bornées et réveils ciblés par objet.

### F — Entrées HID → écran
- **État** : inchangé (non abordé).
- **Prochaine tranche** : identifiant de corrélation HID → présentation.

### G — Composition graphique
- **Réalisé** : réduction des appels/copies de `composited` (voir E).
- **Preuve** : INTÉGRÉ (banc bureau ring 3, TCG). Aucun GPU matériel en jeu.
- **Prochaine tranche** : surfaces clientes projetées des deux côtés, double tampon sans copie.

### H — Atelier en ring 3
- **Initial** : la Calculatrice était déjà ring 3 ; Notes, Journal, Fichiers, Terminal… dans le noyau.
- **Réalisé** : **Journal en application ring 3** (`/bin/journal-gui`) : le noyau ne fait plus que publier la trace (`/proc/bouchaud/journal`, réservée à `DEBUG`) ; même fenêtre au pixel près (capture comparée au témoin noyau), séquences ANSI retirées ; icône d'une application ring 3 choisie par son image (Notes avait perdu la sienne). **Bouchaud Notes en application ring 3** (`/bin/notes-gui`), ouverte par l'icône, le menu et « ouvrir dans Notes » ; rendu identique via l'**atlas de police d'Atelier** publié par le noyau (`/usr/share/atelier/glyphes-*.bin`, `userland/libs/police/atelier.h`) ; enregistrement `write`+`fsync` sur `/persist`.
- **Preuve** : INTÉGRÉ, TCG (`banc_notes_bureau.py` : lancement, police, saisie, F2, F3, capture ; `banc_journal_bureau.py` 7/7 : lancement, accès, police, filtre, recapture, défilement, capture + `--temoin`).
- **État** : intégré ; `gui/apps/notes.rs` et `gui/apps/journal.rs` ne restent qu'en repli si le binaire manque.
- **Prochaine tranche** : retirer les replis quand toutes les images embarquent les binaires ; migrer Fichiers (consommateur du VFS : `opendir`/`stat`) — demande de publier les icônes d'Atelier comme l'atlas de police.

### I — Ladybird
- Exclu par la demande. Bénéficie sans modification de vfork/COW/page zéro.

### J — Stockage, VFS
- **Initial** : `/dev` et `/proc` = comparaisons de chaînes ; `/proc/meminfo` figé au démarrage.
- **Réalisé** : interface `SystemeFichiers`, **table de montages**, résolution par point de montage, `Vnode` ; **devfs** et **procfs** montés ; `/proc` vivant (`meminfo`, `uptime`, `mounts`, `bouchaud/memoire`, `<pid>/status`) ; union avec le RAMFS.
- **Preuve** : HOST (`tools/fs/test_vfs.rs`, 5 tests), TCG avec témoin (`run_vfs.sh` : montages, `/proc` vivant, liste des pids, `/proc/<fils>` après `wait`, `/dev/input`, refus de la trace).
- **Réalisé (tranche 2)** : `SystemeFichiers::liste`, `Ouverture::Dossier`, `FdKind::DossierMonte` ; `getdents` fait l'**union** RAMFS + système monté (un nom une fois) ; `/proc` liste les **processus vivants** (`/proc/<pid>` disparaît à la récolte), devfs liste ses périphériques ; `stat`/`fstat`/`statx` : `S_IFDIR`. Limite : `chdir`/`openat` relatif sous un répertoire monté absent du RAMFS → `ENOTDIR`.
- **État** : deux tranches intégrées au chemin `open`/`getdents`/`stat` de tous les programmes.
- **Prochaine tranche** : adaptateur RAMFS derrière la même interface et descripteurs indépendants des indices (porteront le chemin : lève la limite ci-dessus) ; puis système de fichiers disque inscriptible (BFS existant), cache de pages unifié, writeback/fsync.

### K — Réseau
- **État** : inchangé.

### L — Sécurité et isolation
- **Réalisé** : droits stricts et refus déterministes de `bo_shm_map` ; quota du tas noyau par processus ; W^X vérifié sur les projections ; **la trace du noyau exige `DEBUG`** (`FsDeny::KernelTrace`, refus audité).
- **Preuve** : TCG ; refus de la trace : témoin discriminant (sans la garde, un processus uid 1000 l'ouvre).

### M — Matériel physique
- **État** : inchangé ; aucune qualification PHYSICAL.

### N — Audio, périphériques
- **État** : inchangé.

### O — Observabilité
- **Réalisé** : `[PROC-CYCLE]` (dont `barrieres_cr3=`), `[MEM-COW]`, `/proc/bouchaud/memoire`, `COW_REFUS`, `OOM_VICTIME`, `ESPACE_RENDU_A_LA_MORT`, `ESPACE_QUARANTAINE`.

## 3. Rejouer

```bash
cargo bootimage
B=target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin
tools/ci/run_cow.sh $B 50 4
tools/ci/run_page_zero.sh $B 2
tools/ci/run_vfork.sh $B 10 2
tools/ci/run_memoire_mort.sh $B "zombie oom" 2 1024
tools/ci/run_orphelins.sh $B 2
tools/ci/run_shm_map.sh $B 2
tools/ci/run_vfs.sh $B 2
python3 tools/ci/banc_notes_bureau.py --boot $B
python3 tools/ci/banc_journal_bureau.py --boot $B          # et --temoin
python3 tools/ci/banc_bureau_ring3_entrees.py --boot $B
tools/ci/run_tempete_exec.sh $B 30     # incident §0 (sous charge : 4 instances en parallèle)
```
