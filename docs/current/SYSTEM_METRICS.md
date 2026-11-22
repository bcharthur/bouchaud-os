# Métriques système

Ouvrir **Moniteur** dans le menu du bureau, ou lire le même relevé dans un programme utilisateur. Dans le Terminal Bouchaud, la commande est `cat /proc/bouchaud/metrics`.

Le format texte est versionné. Chaque ouverture fige le contenu du descripteur. Un nouvel appel peut reprendre l'instantané monotone de moins d'une seconde. La collecte partage ce cache entre le Moniteur et les lecteurs de `/proc`. Aucun journal par trame n'est ajouté.

- `SYSTEM_METRICS` donne les cumuls globaux CPU utilisateur, noyau et inactif en nanosecondes. Le champ `cpu_busy_pct` utilise les différences entre deux relevés valides. La RAM est calculée à partir des cadres gérés par l'allocateur physique.
- `DISK_METRICS` distingue les lectures du backing ATA des opérations NVMe. `nvme_present` précise si un contrôleur NVMe est disponible. Les compteurs de barrière couvrent la persistance. Les métriques ATA couvrent les lectures du backing, avec débit récent et pire durée observée.
- `GPU_METRICS` expose la disponibilité de VIRGL, les trames et les pixels de texture montés. Les compteurs VirtIO décrivent le temps de présentation côté invité. Les percentiles du bureau sont les bornes hautes de l’histogramme cumulatif, tous chemins du bureau confondus. L'utilisation GPU est `N/A`, car aucun compteur hôte fiable ne remonte par cette interface.
- `SCHED_METRICS` donne la pire attente runnable observée depuis le démarrage. Une valeur `N/A` signifie que le débit, le delta ou la distribution n'est pas encore exploitable.

Les vues Linux existantes `/proc/stat` et `/proc/<pid>/stat` restent disponibles pour les cumuls de processus. Les IOPS globales, latences disque par percentile et attribution des écritures aux processus demandent une source de mesure supplémentaire.
