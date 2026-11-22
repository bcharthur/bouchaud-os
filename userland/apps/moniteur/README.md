# Collecteur de métriques en espace utilisateur

Construire le collecteur et placer le binaire dans le répertoire `bin` de
l’image userland.

```sh
gcc -O2 -Wall -Wextra -static-pie userland/apps/moniteur/collecte.c -o collecte-metriques
```

Dans Bouchaud OS, `exec /bin/collecte-metriques 3` exporte trois instantanés
système à une seconde d’intervalle sur la sortie standard. Un second argument
sélectionne le PID suivi. Sans PID, le relevé porte sur le collecteur lui-même.

Le processus définit la cadence, lit de nouveaux descripteurs à chaque relevé
et publie le résultat. Le noyau fournit les instantanés bruts via
`/proc/bouchaud/metrics` et `/proc/<pid>/stat`. La ligne `PROCESS_METRICS`
indique les cumuls CPU en ticks et la mémoire résidente en pages. Les E/S
par processus sont indiquées comme indisponibles. Le collecteur quitte avec
un code non nul si le processus suivi disparaît ou devient illisible.

Ce binaire s’utilise à la demande. Le bureau conserve son Moniteur intégré.

## Fenêtre Moniteur en espace utilisateur

Construire le client graphique et placer `moniteur-gui` dans le répertoire
`bin` de l’image userland.

```sh
gcc -O2 -Wall -Wextra -static-pie -I userland/libs/police \
    userland/apps/moniteur/moniteur-gui.c -o moniteur-gui
```

Ajouter ces lignes à l’autorun de cette image, ou les saisir avant d’ouvrir
le bureau depuis le shell de Bouchaud OS.

```sh
export BO_AUTOSTART_APP=/bin/moniteur-gui
desktop
```

Le Moniteur est un client BOGU. Il lit l’instantané par `/proc`, dessine son
contenu dans la surface héritée et publie ses trames via le canal hérité.
Le gestionnaire de fenêtres continue de fournir la géométrie et les
décorations. Cette fenêtre indépendante est optionnelle.

Pour ouvrir ce client depuis **Démarrer → Moniteur**, exporter
`BO_MONITEUR_RING3=1` avant `desktop`. Le bureau ouvre son Moniteur intégré
si le lancement du programme échoue. Le même réglage s’applique à
`BO_ATELIER_OUVRE=3` pour les démarrages automatisés.
