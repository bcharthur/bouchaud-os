# Noyau et runtime

## SMP / scheduler
Multi-CPU, migrations, comptabilité, diagnostics et TLB shootdowns.

## Temps
TSC, contrôle PIT, horloge monotone et audits 1/10/60 s.

## Signaux
Masque par fil sur les chemins nécessaires ; SIGCHLD utilisé pour la
supervision navigateur.

## Mémoire
VMA, mappings paresseux, shared memory, page faults, RSS et surfaces.

## ABI
Compatibilité POSIX/Linux pour les ports, distincte de l'architecture native.
