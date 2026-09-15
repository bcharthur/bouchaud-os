# Bouchaud OS — Idle/IRQ Fragmentation V6

## Pourquoi ce patch

Le freeze du 28/08 à 23:26 apparaît **sans Ladybird**. Juste avant l'arrêt des
logs, le BKL est libre et `resume_after_schedule()` est sain. Le prochain domaine
à isoler est donc le sommeil BSP : scheduler idle, lock park, HLT et livraison
des interruptions.

## Ce que V6 change réellement

V6 fragmente `src/arch/x86_64/cpu.rs` en responsabilités :

```text
src/arch/x86_64/
├── cpu.rs
└── cpu/
    ├── etat.rs
    ├── accounting.rs
    ├── time.rs
    ├── info.rs
    └── idle/
        ├── etat.rs
        ├── scheduler.rs
        ├── lock_park.rs
        └── trace.rs
```

Tous les fragments sont `include!` dans le même module `cpu`, donc les API
existantes (`prepare_scheduler_idle`, `commit_scheduler_idle`,
`prepare_lock_park`, `commit_lock_park`, `wait_for_interrupt`, etc.) restent
aux mêmes chemins Rust.

## Test P0 : BSP sans HLT

`BSP_SAFE_IDLE_DIAGNOSTIC = true`.

Sur **CPU0 seulement** :
- `commit_scheduler_idle()` ne fait plus `sti; hlt`;
- `commit_lock_park()` ne fait plus `sti; hlt`;
- `wait_for_interrupt()` ne fait plus `sti; hlt`.

Le BSP réactive IF, exécute 64 `PAUSE`, puis revient au scheduler. CPU1-3 gardent
le vrai `sti; hlt`.

C'est un test diagnostique volontairement coûteux : la charge CPU affichée peut
être plus haute. Ne pas utiliser ce run pour juger les performances.

### Interprétation

- Si le freeze souris/clavier disparaît pendant plusieurs minutes :
  **le domaine BSP HLT / wakeup est confirmé**.
- Si le freeze persiste mais les logs série continuent :
  ce n'est pas HLT ; inspecter desktop/compositor/scheduler.
- Si le freeze persiste ET les logs série s'arrêtent malgré BSP no-HLT :
  inspecter PIT/PIC/APIC/IF ou une boucle noyau dure.

## Nouveaux logs

Le rapport BKL périodique appelle maintenant :

```text
[IDLE-DIAG]
[IDLE-CPU]
```

Exemple :

```text
[IDLE-DIAG] bsp_safe=1 ... pit_ticks=... pit_age_ns=... idle_mask=...
[IDLE-CPU] cpu=0 phase=running ... sched=100/100/0/safe100 ...
```

Le compteur `safe` doit monter fortement sur CPU0. C'est normal.

## Test recommandé

D'abord **sans ouvrir Ladybird** :

```powershell
.\run.ps1 -Ladybird -LadybirdUrl "https://www.google.com/" |
    Tee-Object -FilePath idle-v6-desktop.log
```

Laisser le bureau et le terminal ouverts 3 à 5 minutes. Bouger la souris,
taper dans le terminal, ouvrir/fermer des fenêtres.

Puis analyser :

```powershell
python .\tools\perf\analyse-idle-v6.py .\idle-v6-desktop.log
```

Ensuite seulement refaire Google.

## Retour au HLT normal

Dans :

```text
src/arch/x86_64/cpu/idle/etat.rs
```

passer :

```rust
pub const BSP_SAFE_IDLE_DIAGNOSTIC: bool = true;
```

à :

```rust
pub const BSP_SAFE_IDLE_DIAGNOSTIC: bool = false;
```

## Ce que le patch ne fait pas

Il ne change pas :
- la politique du BKL V5.1 ;
- `src/fs/persistance.rs` ;
- `src/gui/client.rs` ;
- `src/kernel/process/thread.rs` ;
- le protocole APIC/PIC lui-même.

Le but est d'obtenir une preuve avant de toucher aux contrôleurs
d'interruptions.
