# Architecture actuelle

```text
UEFI
  │
Bouchaud kernel
  ├─ SMP / scheduler / temps
  ├─ mémoire virtuelle / VMA / shared memory
  ├─ processus / threads / signaux / futex / poll
  ├─ VFS / persistance
  ├─ réseau
  ├─ sécurité
  ├─ drivers PCI / bloc / USB / Ethernet / affichage
  └─ WM / surfaces / damage
        │
        └─ ring 3
             ├─ applications
             └─ Bouchaud Navigateur
```

Bouchaud OS utilise son propre noyau. La compatibilité Linux/POSIX sert au
portage ; elle n'implique aucun noyau Linux au runtime.

Direction : développer progressivement des API Bouchaud natives sans casser les
ports existants.
