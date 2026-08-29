# Bouchaud OS — V14 Fluidité / Performance

V14 vise le profil qui reste lent après V13.3.1 : Google fonctionne, le BKL
n'est plus bloqué plusieurs secondes, mais le navigateur paie encore énormément
de fautes de pages, de réveils et d'I/O TCG.

## Ce que V14 change

1. **Clustered demand paging** : après une faute propre read-only validée, jusqu'à
   8 pages voisines du même VMA sont acquises hors `Mm`, revalidées, puis mappées.
   C'est le changement qui peut réellement réduire le *nombre* de futures #PF.
2. **Read-ahead MM 2/4/8 pages** et télémétrie détaillée.
3. **Backing read-ahead 64/128/256 KiB**, cache 512 fenêtres.
4. **Clean page cache 64 MiB** (16 384 pages reclaimables) au lieu de 8 MiB.
5. **WRITE/WRITEV sans BKL externe** : les page faults de copyin ne possèdent
   plus le verrou global. Console/RAMFS/audio/inet gardent un BKL interne court.
6. **MUNMAP/MADVISE sans BKL externe** : `Mm`, caches et TLB ont déjà leurs
   domaines propres.
7. **Diagnostic moins intrusif** : détection des stalls toujours à 1 Hz,
   snapshots sains à 0,2 Hz et watchdog imprimé au plus toutes les 10 s.
8. **Profil UX WHPX1** : ne pas confondre 4 vCPU TCG avec 4 cœurs natifs. Le
   launcher V14 privilégie l'accélération matérielle pour l'usage interactif ;
   TCG4 reste le profil de validation SMP.

## Pourquoi la RAM ne monte pas par défaut

Le run V13.3.1 alloue déjà 12 GiB et le guest garde plusieurs GiB libres. La
pression est dans le coût des faults/I/O/BKL/émulation, pas dans un manque de
RAM. V14 utilise mieux cette RAM avec ~64 MiB de clean-cache et un backing cache
plus généreux au lieu de simplement passer QEMU à 16 GiB.

## Application

Extraire le ZIP à la racine du dépôt, puis :

```powershell
python .\tools\dev\verifie-v14.py
git apply --check .\V14-SOURCE.patch
git apply .\V14-SOURCE.patch
python .\tools\verifie-verrouillage.py
cargo check
cargo bootimage
```

Le ZIP remplace directement les fragments V13 qu'il possède. `V14-SOURCE.patch`
ne touche que les fichiers historiques non possédés par le drop-in V13.

## Exécution

Profil interactif rapide :

```powershell
.\tools\perf\run-ladybird-v14.ps1 -Url "https://www.google.com/"
```

Validation SMP TCG :

```powershell
.\tools\perf\run-ladybird-v14.ps1 -TcgSmp -Url "https://www.google.com/"
```

Analyse :

```powershell
python .\tools\perf\analyse-v14.py .\v14-google.log
```
