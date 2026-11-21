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
