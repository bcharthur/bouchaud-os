# SMP-NG3 TLB shootdown

## Protocole

Chaque CPU logique possède une slot d'émission atomique. Une génération non
nulle publie, avec sémantique release, la PML4, la plage, le masque des CPU
cibles et un masque d'ACK initialement vide. Des émetteurs différents utilisent
des slots différentes et ne peuvent donc plus écraser une mailbox globale.

Le masque cible est le snapshot `AddressSpace.active_cpus`, intersecté avec les
CPU online et privé du CPU local. `AddressSpace::activate` publie son bit avant
le chargement CR3 : un unmap concurrent voit donc le CPU comme cible, ou bien le
chargement CR3 intervient après la mutation PTE et ne peut importer l'ancienne
traduction.

Le handler IPI parcourt les slots publiées. Pour chaque slot qui cible le CPU,
il compare CR3, invalide la plage localement, puis publie l'ACK avec AcqRel. Il
ne prend ni BKL ni verrou MM, n'alloue pas, ne dort pas et termine par l'EOI.

## Retraite de mapping

`munmap` suit trois phases :

1. sous l'emprunt exclusif du processus, retirer les VMA/PTE et collecter les
   frames dans `UnmapRetirement`;
2. abandonner l'emprunt puis suspendre le BKL, envoyer les IPI et attendre tous
   les ACK;
3. reprendre le BKL et l'emprunt, retirer les frames de l'ownership de l'espace
   et les rendre à l'allocateur.

Ainsi aucun borrow `RefCell` ne traverse l'attente et aucune frame n'est libérée
avant la barrière d'ACK. `mprotect` sépare de même mutation et invalidation. Une
assertion debug interdit d'appeler l'attente distante lorsque le CPU détient le
BKL.

Les fautes de page utilisateur réactivent IF avant toute attente du verrou
legacy. Même durant la migration du #PF vers un verrou MM fin, un CPU cible peut
donc toujours traiter immédiatement le vecteur TLB.

## Frontière `Process` / BKL

Le retour runtime SMP4 a montré qu'une API `AddressSpace::unmap()` ne peut pas
suspendre le BKL de façon cachée : son appelant peut encore détenir un
`RefMut<Process>`. Toutes les mutations actives utilisent désormais uniquement
les phases explicites `prepare_unmap`/`prepare_protect`, abandon de l'emprunt,
`TlbInvalidation::execute`, puis `finish_unmap`. `brk` shrink, `MAP_FIXED`,
`madvise`, `munmap`, `mprotect` et le nettoyage d'un fault fichier suivent cette
frontière.

En debug, toute suspension effective du BKL vérifie que chaque `Process` peut
être emprunté exclusivement. L'assertion signale ainsi le callsite qui laisse
fuir un `Ref`/`RefMut`, avant qu'un autre CPU ne panique plus tard dans
`scheduler::install`.
