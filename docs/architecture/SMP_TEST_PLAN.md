# Plan de test SMP-NG3

## Build

```text
cargo check
cargo bootimage
```

## Windows/QEMU

Pour chaque 1, 2, 4 puis 8 CPU, booter, attendre le bureau, lancer Ladybird et
conserver le log série:

```powershell
.\run.ps1 -CpuCount 1
.\run.ps1 -CpuCount 2
.\run.ps1 -CpuCount 4
.\run.ps1 -CpuCount 8
```

Valider `example.com`, Google, recherche, clavier, clic et scroll pendant dix
minutes. Rechercher absence de stall BKL, progression des générations TLB et
présence de PERF_BROWSER_CLICK/PERF_EXEC_START/PERF_FIRST_PAINT.

## Stress MM

Un binaire userland doit créer 4/8 threads qui partagent un PID et répètent
mmap, faults simultanés sur une même page, mprotect et munmap. Vérifier qu'une
seule transition Loading→Present existe par page, qu'aucune frame n'est rendue
avant tous les ACK et qu'aucune Task n'est Running sur deux CPU.

## Reboot

Effectuer 20 boots SMP4 et 10 SMP8. Un seul succès ne valide pas une race.
Ces tests runtime n'ont pas été exécutés dans l'environnement Linux Codex sans
QEMU ni PowerShell.
