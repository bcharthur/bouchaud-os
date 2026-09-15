# Bouchaud OS V13 — Grand Saut Native Performance

Base attendue : Event-Driven Core + V1.1 compile fix.

V13 regroupe les axes de correction qui avaient été identifiés séparément, sans
introduire de noyau Linux/Windows/macOS :

1. conserve BSP `sti; hlt` et attente desktop INTERFACE détachée ;
2. supprime le busy-handoff V9 devenu obsolète depuis le handoff BKL V10 ;
3. remplace le futex stocké dans la table globale des tâches par `wait_word`
   natif, bucketisé, avec façade fragmentée ;
4. suspend le BKL autour de l'attente/réveil wait-word ;
5. transforme la persistance en transaction : snapshot RAMFS court, hash + ATA
   PIO à depth=0, index disque sous verrou local ;
6. ajoute un readahead adaptatif qui préchauffe le clean-page cache ;
7. conserve les copies framebuffer `present*` hors BKL ;
8. ajoute une frontière native de readiness réseau fragmentée ;
9. fournit profiling V13 + comparaison avant/après ;
10. fournit un runner WHPX mono-vCPU pour mesurer l'UX sans payer le coût TCG
    SMP, tout en gardant le run TCG 4-vCPU pour valider SMP.

## Test recommandé

Validation structure / build :

```powershell
python .\tools\dev\verifie-v13.py
git diff --check
.\run.ps1 -Ladybird -LadybirdUrl "https://www.google.com/" 2>&1 |
    Tee-Object -FilePath v13-tcg-smp.log
python .\tools\perf\profile-v13.py .\v13-tcg-smp.log
```

Test UX accéléré :

```powershell
.\tools\perf\run-ladybird-fast.ps1 -Url "https://www.google.com/"
```

Le test WHPX mono-vCPU n'est PAS un remplacement du test SMP TCG : c'est un
second profil, destiné à séparer le coût de l'émulation TCG du coût réel du
noyau/navigateur.
