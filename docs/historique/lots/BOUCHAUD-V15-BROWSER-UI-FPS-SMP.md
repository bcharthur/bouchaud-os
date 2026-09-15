# Bouchaud OS — V15 Browser UI / FPS / SMP

V15 est incremental sur V14.1.

## Interface navigateur

- barre URL: rendu Skia/DejaVu Sans 16 px, anti-aliasé; l'atlas historique reste seulement un fallback;
- retour / avance / recharger / stop: sources SVG dans `tools/ladybird/chrome/assets/`, pre-rasterisees en masque antialiase pour le chrome embarque;
- pendant un chargement, le bouton reload devient Stop, un point bleu et une ligne bleue apparaissent sous le champ URL;
- page stabilisee: point vert, aucune fausse progression en pourcentage.

Le patch navigateur est applique à l'arbre Ladybird jetable par `modernise-v15.py`, via le hook existant `verifie-chrome.sh`, juste avant le build CMake. Il faut donc reconstruire/rafraichir l'artefact Ladybird pour voir ces changements.

## FPS

Le pseudo-Hz V14.1 disparait. Le prefixe devient:

    [20:52:48][ 97%:  5%:  6%][FPS: 35] ...

`FPS` = trames utiles du compositeur Bouchaud, hors rafraichissement de l'horloge. Une page immobile peut donc afficher 0 FPS sans que ce soit une panne.

La ligne detaillee devient `[FRAME-PERF]` avec `useful_gap_max_ms`. Pour juger la navigation, il faut la lire avec `[PERF-BROWSER] frame_gap_max_ms` et `input_to_frame_max_ms`.

## Barre du haut

Le FPS est dessine a droite de la barre systeme avec la police proportionnelle native du bureau. Il se rafraichit au prochain redraw de la topbar (typiquement une fois par seconde via l'horloge).

## SMP

Le noyau courant declare `MAX_CPUS=16` et `run.ps1` accepte 1..16 vCPU. V15 fournit un runner borne aux profils utiles 1/4/8.

Valider d'abord 4 vCPU en TCG:

    .\tools\perf\run-ladybird-v15.ps1 -CpuCount 4 -Accel tcg -Url "https://www.google.com/"

Puis 8:

    .\tools\perf\run-ladybird-v15.ps1 -CpuCount 8 -Accel tcg -Url "https://www.google.com/"

WHPX SMP4 peut ensuite etre essaye pour la vitesse hote:

    .\tools\perf\run-ladybird-v15.ps1 -CpuCount 4 -Accel whpx -Url "https://www.google.com/"

Ne comparer des performances qu'apres avoir confirme dans les logs plusieurs CPU online et des `cpu_map=[...]` repartis.

## Installation / validation

Extraire le ZIP a la racine du depot, sans script d'application.

    python .\tools\dev\verifie-v15.py
    cargo check
    cargo bootimage

Pour la nouvelle UI Ladybird, pousser les fichiers du patch, attendre le workflow `ladybird-native-browser.yml`, puis:

    .\tools\perf\run-ladybird-v15.ps1 -CpuCount 4 -Accel tcg -RefreshLadybird

Analyse:

    python .\tools\perf\analyse-v15.py .\v15.log

## Cibles de performance

Ne viser 120 FPS que pendant une animation/scroll continu. Au repos, 0 FPS utile est souhaitable. Les cibles plus pertinentes sont: absence de frame gaps multi-secondes, faible input->frame, queue runnable maitrisee, et repartition reelle sur plusieurs vCPU.
