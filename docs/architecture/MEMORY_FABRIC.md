# Bouchaud Memory Fabric

## Statut

Ce document distingue **ce qui est implemente maintenant** de la cible long
terme. L'ambition ne doit jamais etre confondue avec une garantie du noyau.

## Etape 1 — ce patch

### Fichiers de boot paresseux

Le namespace historique reste temporairement RAMFS, mais les gros fichiers de
l'archive USTAR ne sont plus recopies dans `Node::content`.

```text
node RAMFS
    |
    +-- DiskExtent { drive, data_lba, size }
```

Les lecteurs passent par `fs::backing::read_at()`. Un fichier de 300 Mio peut
donc exister dans le namespace sans occuper 300 Mio de RAM.

### ELF file-backed demand paging

`exec()` ne clone plus l'image complete pour les programmes ordinaires. Le
chargeur lit seulement l'en-tete ELF et les program headers, puis transforme les
`PT_LOAD` en promesses file-backed.

```text
exec
  -> metadata ELF
  -> promesses PT_LOAD
  -> RIP
  -> #PF
  -> lecture de la page demandee
  -> mapping
  -> reprise
```

Les autotests ELF generes directement en memoire conservent le chargeur eager.

### `mmap(MAP_PRIVATE)` file-backed

Un mapping prive de fichier est reserve sans recopier son contenu. La premiere
faute lit la plage correspondante depuis le backing.

### Couche bloc

`drivers::block::BlockDevice` separe le consommateur de blocs du pilote ATA.
ATA reste le backend actuel ; `virtio-blk` pourra etre ajoute sans faire remonter
ATA dans les couches fichiers/memoire.

## Invariants

1. Un gros fichier immutable de boot ne doit pas etre copie integralement pour
   apparaitre dans `/`.
2. `exec()` d'un fichier de N octets ne doit pas exiger un buffer noyau de N
   octets avant la premiere instruction.
3. Une page file-backed propre doit etre rechargeable depuis son backing.
4. Les zones anonymes `MAP_NORESERVE` restent zero-backed et paresseuses.
5. Le chemin `MAP_SHARED` continue d'utiliser des frames partagees.
6. Les etendues disk-backed de l'archive sont read-only dans cette etape.

## Etapes suivantes

### VFS + BFS

Remplacer les indices RAMFS exposes partout par des vnode/filesystem handles :

```text
VFS
 +-- BFS
 +-- tmpfs
 +-- devfs
 +-- procfs
```

Le RAMFS actuel devient principalement tmpfs (`/tmp`, `/run`, `/dev/shm`).

### Page cache unifie / MemoryObject

Cible :

```text
MemoryObject(file/inode)
        |
        +-- page -> resident / disk / compressed / dirty
```

`read`, `mmap`, `exec` et IPC doivent converger vers les memes pages physiques.

### Copy-on-write

`fork()` et `MAP_PRIVATE` partageront les frames read-only et ne copieront qu'a
la premiere ecriture.

### Zero page et allocation vraiment lazy

Une grande zone zero peut initialement pointer vers une page zero globale,
read-only, puis se materialiser uniquement a l'ecriture.

### Compression / reclaim / swap

Ordre cible sous pression :

1. abandonner les pages file-backed propres ;
2. compresser les pages anonymes froides ;
3. swapper ce qui ne peut ni etre relu ni reconstruit.

### Memory intents

Future API : hot/cold, sequential/random/streaming, discardable,
reconstructible, realtime, immutable/shareable.

### Working-set profiles

Observer les faults de demarrage et precharger seulement le working set au
lancement suivant.

### Snapshots COW

Un processus proprement initialise peut devenir un template clone en COW pour
accelerer les applications lourdes.

### Zero-copy IPC / page loaning

Pixels, paquets, audio et video doivent transferer des references sur des
MemoryObjects plutot que des octets recopies.

### Reseau zero-copy

DMA NIC -> pages -> MemoryObject -> userspace, puis le chemin inverse a
l'emission lorsque le materiel le permet.

## Observabilite

Les compteurs de backing et de demand-paging sont la premiere base. La cible est
une commande `memtop` distinguant virtuel, resident, file-backed, shared, dirty,
compressed, faults et working set.


## VMA Engine v3

Le modele `promesses` append-only est remplace par une carte d'intervalles
split-safe (`kernel::vma`). Les reservations `PROT_NONE` existent dans la
metadata meme sans page residente. `munmap`, `mprotect`, `MAP_FIXED` et
`madvise` operent maintenant sans supprimer les fragments voisins.
Voir `docs/architecture/VMA_ENGINE.md`.
