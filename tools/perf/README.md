# Bouchaud Performance Observatory V1

Analyse locale :

```powershell
python .\tools\perf\analyse-bouchaud-perf.py .\perf-google.log
```

Le parseur comprend les anciens logs (`BKL-*`, `MM-NG6`, `PROC-SAMPLE`,
`GUI-PRESENT`) et les nouveaux blocs `PERF-BROWSER` / `PERF-WATCHDOG`.

Aucun accès réseau, aucune dépendance Python externe.
