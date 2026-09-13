# Correctif Trigkey SMP double fault

Symptôme observé :

```text
BOUCHAUD_BOOT_POINT reseau
BOUCHAUD_STAGE2_SMP_PROBE_V31_BEGIN
SMP4_STAGE sipi-1
DOUBLE FAULT
RSP=0x10000014d70 HORS TAS NOYAU
CPU=0
```

Interprétation : le point `reseau` est le dernier point franchi, pas forcément la cause directe. Le crash arrive dans la fenêtre `smp::init_probe()`, juste après `sipi-1`.

Le correctif applique trois protections :

1. Guard SMP autour de `init_probe()` : IRQ désactivées sur le BSP pendant préparation trampoline/mailbox/INIT/SIPI et attente AP.
2. IRQ0 / IPI reschedule neutralisés si un handler était déjà entré pendant cette fenêtre.
3. `net-lien` est démarré après `BOUCHAUD_STAGE2_SMP_WIRED_V31`, pas avant le bring-up AP.

Objectif : ne pas désactiver SMP, mais empêcher le scheduler/réseau/timer de se mêler à la fenêtre matérielle INIT/SIPI.

## Application

Depuis la racine du repo :

```powershell
.\tools\apply-trigkey-smp-fix.ps1
cargo +nightly clean
cargo +nightly bootimage
```

Puis teste la clé Trigkey et vérifie que les logs dépassent :

```text
SMP4_STAGE sipi-1
SMP4_STAGE sipi-2
BOUCHAUD_STAGE2_SMP_WIRED_V31
```
