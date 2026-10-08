# Réseau et stockage

QEMU : e1000 de référence CI.
Physique : RTL8111/8168.

Chaîne exercée : Ethernet → ARP/IP → UDP/DNS → TCP → services navigateur.

Le flush ATA après chaque write a été retiré ; les vraies barrières déclenchent
désormais `FLUSH CACHE`.

Le banc ATA écrit/relit/compare, injecte une erreur via `blkdebug` et vérifie
le fallback PIO sans corruption.

Le NVMe de la TRIGKEY est un chemin matériel distinct.
