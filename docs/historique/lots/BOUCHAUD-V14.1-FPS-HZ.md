# Bouchaud OS — V14.1 FPS / Hz telemetry

Patch drop-in à appliquer **par-dessus V14**. Aucun `git apply` n'est requis :
extraire le ZIP à la racine du dépôt en remplaçant les fichiers.

Nouveautés :

- `[FPS: XXX]` et `[Hz: XXX]` dans chaque préfixe série ;
- mesure globale finale au niveau du compositeur, pas seulement Ladybird ;
- hot path sans verrou et sans allocation ;
- ligne périodique `[FRAME-CLOCK]` avec valeurs décimales et cible théorique ;
- analyseur `tools/perf/analyse-fps-hz.py`.

Validation :

```powershell
python .\tools\dev\verifie-v14.1.py
cargo check
cargo bootimage
```

Run :

```powershell
.\tools\perf\run-ladybird-v14.ps1 -Url "https://www.google.com/" 2>&1 |
    Tee-Object -FilePath v14.1-fps.log
```

Puis :

```powershell
python .\tools\perf\analyse-fps-hz.py .\v14.1-fps.log
```
