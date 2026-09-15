# Bouchaud OS — Final V12 Blocking / Readiness

Ce patch clôt le cycle de stabilisation BKL/GUI.

## Changement principal

Les appels sans BKL externe (`poll` / `ppoll`, puis toute primitive native
`WaitSource`) n'emportent plus un `KernelGuard` de `WaitQueue` à travers
`schedule()`.

Le chemin depth=0 devient :

```text
ticket -> BKL court -> inscription -> recheck -> Blocked
       -> DROP BKL -> schedule depth=0
       -> BKL court de nettoyage -> retour depth=0
```

Le chemin depth>0 reste historique pour ne pas casser les kernel threads legacy.

## Diagnostic

Le contexte syscall était CPU-local. Un kernel thread reprenant un CPU après un
user `poll` était donc faussement affiché comme `poll`. V12 masque ce contexte
quand `CURRENT_IS_KERNEL=1`.

## Fragmentation

```text
src/kernel/sync/wait_queue.rs
src/kernel/sync/wait_queue/
├── etat.rs
├── ticket.rs
├── attente.rs
├── reveil.rs
└── diagnostic.rs
```

## Test

```powershell
git diff --check
python .\tools\dev\verifie-fragmentation-v11c.py

.\run.ps1 -Ladybird -LadybirdUrl "https://www.google.com/" |
    Tee-Object -FilePath final-v12-google.log

python .\tools\perf\analyse-final-v12.py .\final-v12-google.log
```

`depth_violations` dans `[WAITQ-DETACHED]` doit rester strictement à zéro.
L'analyseur détecte UTF-8 et UTF-16LE.
