# V14 — Architecture performance

Le chemin chaud devient : faute utilisateur -> clean cache -> read-ahead backing
64–256 KiB -> read-ahead clean 2/4/8 -> publication PTE courante -> publication
cluster revalidée. Aucun I/O n'est effectué sous le verrou `Mm`.

Le cluster ne traverse jamais un VMA, ne mappe jamais une page writable, ne
prépublie jamais un backing non disque et revalide le `pml4` ainsi que le
`MappingToken` après l'acquisition de chaque frame. Une course mmap/munmap ou
mprotect transforme donc le préchargement en abandon, pas en PTE obsolète.

La suppression du BKL externe pour WRITE/WRITEV sépare le copyin, qui peut
faulter, des sinks hérités. Les sinks dont l'état n'est pas encore nativement
SMP (console, RAMFS mutable, AC97, inet/e1000) reprennent un BKL local, après le
copyin. Pipe, eventfd et socketpair restent sur leurs verrous d'objet.
