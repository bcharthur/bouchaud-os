# Ordre des verrous SMP

Ordre autorisé pendant la migration :

1. BKL legacy (uniquement anciens chemins);
2. RunQueue CpuLocal;
3. verrou court de ressource/SpinLockIrq.

Une WaitQueue n'est jamais attendue avec un SpinLock détenu. SleepMutex est
interdit depuis IRQ. Le handler TLB ne prend aucun verrou.

Exception volontaire au classement: une attente de shootdown suspend entièrement
le BKL avant IPI/ACK, puis le reprend. Aucune frame n'est libérée dans cette
fenêtre. Il est interdit d'ajouter BKL, MM lock, allocation, I/O ou schedule au
handler TLB.

Pour plusieurs RunQueue, ne jamais en détenir deux : lire la longueur, relâcher,
puis voler une entrée. La validation Task/on_cpu se fait ensuite sous le BKL de
migration, ce qui interdit une double exécution.
