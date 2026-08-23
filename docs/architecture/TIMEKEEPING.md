# Timekeeping x86_64

`kernel::timer::monotonic_ns()` est l'horloge de deadline. Au boot, le noyau lit
la fréquence TSC via CPUID 0x15 (rapport crystal/compteur), puis CPUID 0x16 en
repli. La conversion cycles→ns utilise un intermédiaire `u128`; un `fetch_max`
global absorbe un éventuel faible décalage de TSC entre CPU.

Si le firmware/hyperviseur ne publie aucune fréquence, la calibration PIT
existante initialise le dernier repli. Tant que cette calibration n'est pas
terminée, les ticks PIT restent disponibles. Le PIT conserve donc les rôles de
bootstrap, calibration et tick scheduler transitoire, mais n'est plus la source
primaire de `monotonic_ms()` lorsqu'une fréquence TSC architecturale existe.

Cette séparation corrige le défaut TCG où 1000 IRQ attendues peuvent prendre de
nombreuses secondes murales : les délais réseau, `poll`, `timerfd` et les sleeps
qui appellent `monotonic_ms()` suivent désormais le TSC virtuel plutôt que le
nombre d'IRQ0 effectivement traitées.

## Limites et prochaine étape

La détection doit encore enregistrer explicitement les bits invariant-TSC et
RDTSCP, puis calibrer/valider les offsets AP au démarrage. Le scheduling dépend
toujours du PIT BSP et des IPI broadcast; SMP-NG3 doit le déplacer sur les timers
LAPIC locaux. Les tests runtime QEMU 1/2/4/8 CPU restent indispensables.
