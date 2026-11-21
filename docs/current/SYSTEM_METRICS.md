# Métriques système

Ouvrir **Moniteur** dans le menu du bureau, ou lire le même relevé dans un programme utilisateur. Dans le Terminal Bouchaud, la commande est `cat /proc/bouchaud/metrics`.

Le format texte est versionné. Chaque ouverture fige le contenu du descripteur. Un nouvel appel peut reprendre l'instantané monotone de moins d'une seconde. La collecte partage ce cache entre le Moniteur et les lecteurs de `/proc`. Aucun journal par trame n'est ajouté.

- `SYSTEM_METRICS` donne les cumuls globaux CPU utilisateur, noyau et inactif en nanosecondes. Le champ `cpu_busy_pct` utilise les différences entre deux relevés valides. La RAM est calculée à partir des cadres gérés par l'allocateur physique.
- `DISK_METRICS` distingue les lectures du backing ATA des opérations NVMe. `nvme_present` précise si un contrôleur NVMe est disponible. Les compteurs de barrière couvrent la persistance. Les métriques ATA couvrent les lectures du backing, avec débit récent et pire durée observée.
- `GPU_METRICS` expose la disponibilité de VIRGL, les trames et les pixels de texture montés. Les compteurs VirtIO décrivent le temps de présentation côté invité. Les percentiles du bureau sont les bornes hautes de l’histogramme cumulatif, tous chemins du bureau confondus. L'utilisation GPU est `N/A`, car aucun compteur hôte fiable ne remonte par cette interface.
- `SCHED_METRICS` donne la pire attente runnable observée depuis le démarrage. Une valeur `N/A` signifie que le débit, le delta ou la distribution n'est pas encore exploitable.

Les vues Linux existantes `/proc/stat` et `/proc/<pid>/stat` restent disponibles pour les cumuls de processus. Les IOPS globales, latences disque par percentile et attribution des écritures aux processus demandent une source de mesure supplémentaire.

## Journal série

Le préfixe affiche `[heure][CPU=… RAM=… DISK=… GPU=…][FPS: …]`.
`services-metrics` rafraîchit le préfixe à partir du cache commun et émet
`SYSTEM_METRICS`, `DISK_METRICS` et `GPU_METRICS` toutes les cinq secondes.
Chaque ligne structurée porte la même date monotone en nanosecondes.
Le préfixe lit un bloc préformaté avec des lectures atomiques bornées.
Un relevé indisponible ou une publication concurrente donne `N/A`.

- `RAM` résume la mémoire gérée par l’allocateur en Mio. Le relevé structuré
  fournit la précision à l’octet. Les nœuds RAMFS restent dans le diagnostic
  de capacité du système de fichiers.
- `DISK` privilégie le NVMe s’il est présent. `r` et `w` sont les débits
  récents en Kio/s. Côté ATA, `r` couvre la lecture du backing et `w` est
  indisponible. `p` compte les commits de persistance et `b` les barrières
  effectives depuis le démarrage. Le relevé structuré donne en plus les
  octets utiles de persistance écrits par seconde et les barrières indisponibles.
- `GPU` indique le dernier chemin de composition du bureau observé
  (`CPU` ou `VIRGL`), `f` les trames VIRGL cumulées et `p` le temps
  moyen récent de présentation VirtIO en microsecondes. Ce temps dépend des
  présentations de l’intervalle. L’occupation GPU (`u`) et le backend
  Vulkan des applications restent indisponibles dans le noyau. Une application
  peut employer Vulkan même quand le bureau compose en CPU.
