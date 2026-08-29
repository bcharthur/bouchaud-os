# Bouchaud OS — V16.2 Kernel Fluidity / Zombie Retirement

Ce hotfix est **kernel-side** et peut être testé avec l'ancien artefact Ladybird pendant que la CI V16 continue.

## Pourquoi

Le run V16.1 SMP4 a montré deux problèmes indépendants :

1. un vrai panic de cycle de vie : `zombie task resumed after exec quiescence` ;
2. des gels tardifs classés `kernel-bkl`, avec des tenues hors syscall de plusieurs secondes sur le chemin du rapport périodique.

## Corrections

- Retraite exec-zombie non retournante : la tâche zombie quitte définitivement son CPU sans appeler le `schedule()` qui pouvait revenir.
- UART COM1 à 115200 bauds.
- `serial_print!` agrège maintenant tout un formatage dans un tampon de pile de 2 Kio avant de descendre vers le FIFO.
- Le préfixe `[heure][CPU:RAM:FS][FPS]` est construit dans un tampon fixe puis émis en un bloc, au lieu de réentrer une quinzaine de fois dans le pilote série.
- Le relevé très détaillé passe de 5 s à 30 s. L'horloge et le compteur FPS restent inchangés ; seuls les gros dumps de diagnostic sont espacés.

## Test

```powershell
python .\tools\dev\verifie-v16.2.py
cargo check
cargo bootimage

.\run.ps1 `
    -Ladybird `
    -LadybirdInteractif `
    -LadybirdUrl "https://www.google.com/" `
    -CpuCount 4 `
    -RamMiB 12288 `
    -Accel tcg `
    -Audio none 2>&1 |
    Tee-Object -FilePath v16.2-kernel-old-browser-smp4.log

python .\tools\perf\analyse-v16.2.py .\v16.2-kernel-old-browser-smp4.log
```

## Critères

- aucun `zombie task resumed after exec quiescence` ;
- `depth_violations=0` ;
- disparition des tenues BKL multi-seconde causées par le reporting ;
- `frame_gap_max_ms` et `browser silence` doivent fortement baisser une fois le chargement initial stabilisé.

V16.2 ne modifie pas le binaire Ladybird : la nouvelle typographie / le chrome SVG V16 attend toujours le nouvel artefact CI.
