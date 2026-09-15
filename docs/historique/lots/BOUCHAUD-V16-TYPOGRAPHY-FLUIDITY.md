# Bouchaud OS — V16 Typography / Fluidity / SMP

V16 est incremental sur V15.1 + V14.

## Pourquoi le chrome V15 semblait ne rien changer

Le run V15 indique explicitement `native-browser-m9` deja present. Le code C++
modernise vit dans l'artefact Ladybird, pas dans le kernel : reutiliser un
binaire M9 ancien garde donc la barre URL bitmap et les glyphes historiques.
V16 rend ce cas impossible : le runner exige `V16_UI_CAPABLE`.

Un workflow V16 dedie reconstruit maintenant sur `perf/gui-event-driven`, applique
le transform V15 puis V16, force FontConfig/FreeType et publie le marqueur.

## Typographie

- URL : DejaVu Sans 16 px, Skia + FontConfig/FreeType, AA + subpixel.
- boutons back/forward/reload/stop : sources SVG V15, masque antialiase compile.
- chargement : stop + ligne/point d'etat V15, enfin dans le binaire reel.
- page Web : FontConfig/FreeType force + alias directement dans PathFontProvider;
  Google Sans / Roboto / Arial / Helvetica / SerenitySans -> DejaVu seulement
  si la vraie famille est absente; les poids CSS 500/600 choisissent la face
  DejaVu compatible la plus proche au lieu de retomber sur la police bootstrap.

## Fluidite

1. l'attente INTERFACE desktop se detache maintenant a depth=1, 2 ou plus et
   restaure EXACTEMENT la profondeur initiale;
2. les scopes explicitement locaux `present*` / rapport peuvent suspendre une
   profondeur imbriquee, alors que les checkpoints generiques restent depth=1;
3. clustered demand paging fichier monte de 8 a 16 pages;
4. les VMA Zero gagnent un cluster adaptatif 0/2/4/8 pages seulement apres une
   vraie sequence de faults; les acces aleatoires ne sont pas pre-peuples;
5. readahead propre monte progressivement jusqu'a 16 pages.

## Application

Extraire a la racine, sans script d'application :

    python .\tools\dev\verifie-v16.py
    cargo check
    cargo bootimage

Le chrome exige un nouvel artefact CI. Commit/push des fichiers V16, attendre le
workflow `ladybird-native-browser-v16`, puis :

    .\tools\perf\run-ladybird-v16.ps1 -CpuCount 4 -Accel tcg -RefreshLadybird

Le runner REFUSE un artefact sans `V16_UI_CAPABLE` au lieu de demarrer avec un
vieux chrome bitmap.

Pour l'usage interactif, essayer ensuite :

    .\tools\perf\run-ladybird-v16.ps1 -CpuCount 4 -Accel whpx

WHPX SMP reste experimental tant que le bring-up APIC n'expose pas toujours les
4 CPU. TCG4 reste la reference de correction SMP.

Analyse :

    python .\tools\perf\analyse-v16.py .\v16-smp4.log

## Cibles

- INTERFACE depth_violations = 0
- BKL max hors boot < 250 ms, aucun plateau ~5 s site 770
- browser silence < 1 s en interaction
- frame gap < 250 ms apres stabilisation
- FPS actif proche de 60 sur TCG4 avant de chercher 120 Hz
