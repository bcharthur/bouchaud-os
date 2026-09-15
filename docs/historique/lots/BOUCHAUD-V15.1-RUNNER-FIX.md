# Bouchaud OS — V15.1 Runner Fix

Correctif ciblé du lanceur `tools/perf/run-ladybird-v15.ps1`.

## Symptôme corrigé

PowerShell signalait :

`Impossible de convertir la valeur «-LadybirdUrl» en type «System.Int32»` pour `Gate0SerialPort`.

## Cause

V15 construisait un tableau de chaînes contenant `-Ladybird`, `-LadybirdUrl`, etc. puis le splattait vers `run.ps1`. Le splatting d'un tableau PowerShell est positionnel ; les chaînes qui ressemblent à des noms de paramètres ne sont pas rebindées comme paramètres nommés. Les valeurs se décalaient donc dans la signature de `run.ps1`.

Le nom `$Args` était en plus à éviter car PowerShell est insensible à la casse et `$args` est une variable automatique.

## Correctif

V15.1 utilise un hashtable `$RunParams` splatté par nom. `Ladybird`, `LadybirdUrl`, `RamMiB`, `CpuCount`, `Accel`, `Audio` et `RefreshLadybird` arrivent désormais dans les bons paramètres de `run.ps1`.

## Relance

```powershell
.\tools\perf\run-ladybird-v15.ps1 `
    -CpuCount 4 `
    -Accel tcg `
    -Url "https://www.google.com/" 2>&1 |
    Tee-Object -FilePath v15-smp4.log
```
