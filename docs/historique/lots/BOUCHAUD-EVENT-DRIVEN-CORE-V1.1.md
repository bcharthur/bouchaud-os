# Bouchaud OS — Event-Driven Core V1.1 compile fix

Le premier Event-Driven Core retirait par erreur, en même temps que l'ancienne
politique BSP V6, les helpers d'état idle qui se trouvaient entre les constantes
et `bsp_safe_relax()`.

Cela explique les erreurs E0425 sur :

- `is_idle`
- `idle_ns_at`
- `note_pit_tick`
- `idle_mask`
- `idle_next_seq`
- `idle_trace_phase`
- `idle_enter`
- `idle_exit`

V1.1 restaure tous ces helpers et garde uniquement la politique BSP dans le
nouveau fichier `cpu/idle/politique.rs`.

Aucun changement de comportement supplémentaire.

Appliquer ce ZIP PAR-DESSUS le patch Event-Driven Core précédent, puis relancer :

```powershell
git diff --check

.\run.ps1 -Ladybird -LadybirdUrl "https://www.google.com/" |
    Tee-Object -FilePath event-driven-core-v1.1.log
```
