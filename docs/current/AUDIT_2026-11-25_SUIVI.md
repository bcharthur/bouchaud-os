# Suivi de l'audit architectural du 25/11/2026 — chantiers A à O

Branche `claude/lucid-galileo-0g4429`, PR brouillon
[bcharthur/bouchaud-os#383](https://github.com/bcharthur/bouchaud-os/pull/383)
(aucune fusion sans accord). Point de départ : `main@5b6db15`.

**Niveaux de preuve** employés ici, et rien au-delà :
**IMPL** (écrit), **COMP** (compile), **HOST** (test hôte), **TCG** (banc QEMU
TCG local, avec témoin négatif quand il y en a un), **INTÉGRÉ** (exercé dans le
vrai bureau ou par un composant réel de l'OS). Aucun résultat KVM, WHPX ni
PHYSICAL dans ce document.

## 0. Point bloquant ouvert — incident de la tempête d'exec

**À lire avant toute fusion.** `tools/ci/run_tempete_exec.sh` (mode
`complet` : fork + execve + fils + mémoire, 3 lanceurs, SMP4) produit, sur les
images de la branche et **sous forte charge de l'hôte (≥ 4 QEMU parallèles)**,
des incidents : gel total (sortie série coupée en pleine ligne), arrêt muet de
QEMU (triple faute), ou panique `page fault @ <adresse utilisateur>` en mode
noyau, le CPU s'exécutant sur la pile noyau d'un fil mourant (`syscall_nr=60`)
alors que la tâche courante est un autre fil.

| Image | Charge | Passages | Incidents |
|---|---|---|---|
| référence `main@5b6db15` | ×2 | 5 | 0 |
| référence `main@5b6db15` | ×4 | 12 | **0** |
| branche (avant COW) | ×2 | 9 | 1 |
| branche (COW + page zéro) | ×4 | ~10 | plusieurs |
| branche, avec / sans libération à la mort (`banc-sans-espace-rendu`) | ×2 | 6 / 6 | 0 / 0 |

Conclusion à ce stade : **défaut introduit par la branche**, visible seulement
sous forte charge (cadence des vCPU), **non attribué**. Écartés par l'expérience
ou par lecture : la libération à la mort sous charge ×2 (A/B), la course TOCTOU
de la COW (corrigée, cause d'un autre symptôme), la réentrance de verrou dans
`MmCourant`, le recyclage d'un emplacement de tâche encore sur un cœur
(`on_cpu < 0` exigé). Outils en place : relevé des vCPU au gel par le moniteur
QEMU, `-d cpu_reset`, option témoin `banc-sans-espace-rendu`. L'A/B sous charge
×4 est la prochaine mesure.

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
- **État** : implémenté, qualifié TCG. Incident §0 ouvert.
- **Prochaine tranche** : attribuer l'incident §0 ; signaux pendant l'attente vfork.

### C — Mémoire virtuelle et récupération (Memory Fabric)
- **Initial** : `fork` recopiait tout ; pas de page zéro ; espace d'un mort retenu jusqu'à la récolte ; OOM tuant le fautif.
- **Réalisé** :
  - `Mm` remplaçable (`MmCourant`) ; références du cache rendues par le `Mm` ;
  - **fork Copy-on-Write** : table de propriétaires (`src/kernel/memory/cow.rs`, pure), bits `PTE_COW`/`PTE_COW_ECRITURE`, copie ou promotion à la faute, part rendue après l'invalidation TLB, `mprotect` cohérent, échec d'allocation → politique OOM ;
  - **écritures du noyau** (`user_write`, usercopy native, `clear_child_tid`, `AddressSpace::write`) : cassent la COW avant d'écrire ;
  - **page zéro partagée** pour les lectures anonymes ;
  - mémoire rendue à la mort (attente bornée, quarantaine si un cœur tient encore le CR3) ; OOM contrôlé (victime = plus gros consommateur) ;
  - `MAP_SHARED|MAP_ANONYMOUS` réellement partagé (défaut préexistant découvert par la sonde).
- **Mesures** (TCG) : fork d'un père de 64 Mio : **192 Kio** consommés contre **66 076 Kio** (référence) ; lire 128 Mio anonymes : **264 Kio** contre **131 332 Kio** ; `fork+execve` 30 → 19 ms ; zombie : mémoire rendue 14 ms après la mort, avant récolte.
- **Preuve** : HOST (`test_cow`, `test_oom`) ; TCG SMP1/2/4 avec témoins (`run_cow.sh`, `run_page_zero.sh`, `run_memoire_mort.sh`).
- **État** : implémenté et qualifié TCG **hors incident §0**, qui bloque la déclaration « stabilisé ».
- **Prochaine tranche** : attribuer §0 ; regrouper le shootdown du fork ; réclamation sous pression (pages COW) ; tables de pages vides (#379, non réimplémenté ici).

### D — Création, exécution, destruction des processus
- **Initial** : `posix_spawn` = fork à copie complète.
- **Réalisé** : vfork réel (père suspendu, `execve` installe un `Mm` neuf) ; fork COW.
- **Mesures** : `posix_spawn` 9,1 ms contre 30,3 ms (`fork+execve` avant COW), SMP2.
- **Preuve** : TCG avec témoins.
- **État** : implémenté, qualifié TCG.
- **Prochaine tranche** : cache des pages ELF entre processus.

### E — IPC natif et mémoire partagée
- **Initial** : `SharedRegion` lisible/écrivable seulement par appels (copie).
- **Réalisé** : ABI 1.7 `bo_shm_map`/`bo_shm_unmap` (droits MAP/READ/WRITE, W^X, région tenue par les projections, héritée au fork, rendue au munmap/exec/mort) ; stockage aligné page dans l'arène physique du tas (aucune autre donnée ne partage ses frames, arène d'amorçage refusée) ; **quota par processus créateur** ; **adoption par `composited`**.
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
- **Initial** : la Calculatrice était déjà ring 3 ; Notes, Fichiers, Terminal… dans le noyau.
- **Réalisé** : **Bouchaud Notes en application ring 3** (`/bin/notes-gui`), ouverte par l'icône, le menu et « ouvrir dans Notes » ; rendu identique via l'**atlas de police d'Atelier** publié par le noyau (`/usr/share/atelier/glyphes-*.bin`, `userland/libs/police/atelier.h`) ; enregistrement `write`+`fsync` sur `/persist`.
- **Preuve** : INTÉGRÉ, TCG (`banc_notes_bureau.py` : lancement, police, saisie, F2, F3, capture).
- **État** : intégré ; `gui/apps/notes.rs` ne reste qu'en repli si le binaire manque.
- **Prochaine tranche** : retirer le repli quand toutes les images l'embarquent ; migrer Fichiers ou Insights.

### I — Ladybird
- Exclu par la demande. Bénéficie sans modification de vfork/COW/page zéro.

### J — Stockage, VFS
- **Initial** : `/dev` et `/proc` = comparaisons de chaînes ; `/proc/meminfo` figé au démarrage.
- **Réalisé** : interface `SystemeFichiers`, **table de montages**, résolution par point de montage, `Vnode` ; **devfs** et **procfs** montés ; `/proc` vivant (`meminfo`, `uptime`, `mounts`, `bouchaud/memoire`, `<pid>/status`) ; union avec le RAMFS.
- **Preuve** : HOST (`tools/fs/test_vfs.rs`), TCG avec témoin (`run_vfs.sh`).
- **État** : première tranche intégrée au chemin `open` de tous les programmes.
- **Prochaine tranche** : adaptateur RAMFS derrière la même interface et descripteurs indépendants des indices ; `getdents` sur les montages ; puis système de fichiers disque inscriptible (BFS existant), cache de pages unifié, writeback/fsync.

### K — Réseau
- **État** : inchangé.

### L — Sécurité et isolation
- **Réalisé** : droits stricts et refus déterministes de `bo_shm_map` ; quota du tas noyau par processus ; W^X vérifié sur les projections.
- **Preuve** : TCG.

### M — Matériel physique
- **État** : inchangé ; aucune qualification PHYSICAL.

### N — Audio, périphériques
- **État** : inchangé.

### O — Observabilité
- **Réalisé** : `[PROC-CYCLE]`, `[MEM-COW]`, `/proc/bouchaud/memoire`, `COW_REFUS`, `OOM_VICTIME`, `ESPACE_RENDU_A_LA_MORT`, `ESPACE_QUARANTAINE`.

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
python3 tools/ci/banc_bureau_ring3_entrees.py --boot $B
tools/ci/run_tempete_exec.sh $B 30     # incident §0
```
