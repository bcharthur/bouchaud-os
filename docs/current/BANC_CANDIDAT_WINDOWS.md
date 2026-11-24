# Banc main contre candidat sur PC Windows (BOUCHAUD_BANC_CANDIDAT_V1)

Script unique : `tools\windows\Banc-Candidat.ps1` (PowerShell 5.1 ou 7).

```powershell
# 1. Deux bootimages : main et candidat (cargo bootimage sur chaque arbre, puis copie).
# 2. ladybird-browser.img present a la racine (run.ps1 une fois).
.\tools\windows\Banc-Candidat.ps1 -Main main.bin -Candidat cand.bin -Paires 3 -Accel whpx -Smp 2
```

Ce que fait le banc :

* Il utilise le meme disque Ladybird pour les deux bras. C'est une copie de banc dont l'autorun lance le navigateur ; elle est montee en `snapshot=on`, et `ladybird-browser.img` n'est jamais modifie.
* Les bras alternent par paire (M C, C M, ...).
* Il prend les jalons sur l'horloge de l'hote : QEMU -> noyau, QEMU -> bureau, ouverture de Ladybird (lancement -> premiere trame) et premiere page (lancement -> document charge).
* Une fois la page chargee, il injecte la molette pendant `-DefilementS` secondes par le moniteur QEMU (`mouse_move 0 0 dz`).
* Il releve ensuite, sur l'horloge de l'invite :
  * dans `[GPU-TRAMES]` : dessin et presentation p50, p95 et p99 ;
  * dans `[GPU-COMPOSITION]` : FPS du bureau, trames GPU, pixels poses par le GPU, pixels clients copies par le CPU et charge CPU de l'invite ;
  * dans `[LB:PERF]` : `copie_px` ;
  * le backend reel (`GPU=<mode>:`).
* Cote hote, il releve le temps CPU et la memoire de pointe de QEMU.

Ce qu'il produit, dans le dossier `banc-candidat-<date>\` :

* `banc.json` : metadonnees, SHA-256 des images, diagnostic de l'hote, passages et verdicts ;
* `banc.csv` et `verdicts.csv` ;
* `serie.log` et `qemu.err` pour chaque passage.

Regles :

* **Validite.** Un passage est valide seulement si l'accelerateur reel lu dans `BOUCHAUD_CPU_ISA` est celui demande, si la page a charge et si la molette a ete injectee. Un passage invalide est publie avec sa cause, n'entre pas dans les medianes, et le script sort alors avec le code 2.
* **Verdicts.** `CANDIDAT_MEILLEUR` ou `MAIN_MEILLEUR` seulement si les intervalles [min, max] des deux bras ne se recouvrent pas ; sinon `NON_CONCLUANT`.
* **GPU.** WHPX n'a pas de passage PCI : la carte graphique de l'hote n'est jamais vue nue par l'invite. Le banc affiche `VIRGL_POSSIBLE` seulement si ce binaire QEMU embarque `virtio-vga-gl` et un affichage GL. C'est alors l'OpenGL de l'hote, paravirtualise, sous `-Gpu virgl` ; sinon il affiche `NON_EXPOSABLE`.
* **Latence entree -> image.** Elle est `NON_MESURABLE` : aucun marqueur n'horodate les entrees dans le journal serie.

Pour simuler sans rien lancer : `-DryRun`. Pour analyser un journal seul : `-AnalyserJournal serie.log`.
Controle en CI : `tools/ci/test-scripts-windows-trigkey.ps1`.
