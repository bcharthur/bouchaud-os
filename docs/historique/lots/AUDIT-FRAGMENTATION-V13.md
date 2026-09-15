# Audit de fragmentation après V13

## Fortement fragmenté

- BKL / handoff / scheduler bridge
- CPU idle / IRQ timer / souris
- thread scheduler (V11C)
- WaitQueue / WaitSource
- **wait-word natif (V13)** : types / état / clé / buckets / wait / wake / cleanup / diagnostic
- ReadinessSource
- **persistance (V13)** : format / transaction / snapshot / index / I/O / sync / diagnostic / montage / collecte / codec
- **memory readahead (V13)** : état / politique / observation / prefetch / diagnostic
- desktop BKL : politique / état / scope / diagnostic
- Performance Observatory + profile V13
- frontière native réseau : types / readiness / diagnostic

## Restent structurellement incomplets après V13

- migration de CHAQUE `FdKind` vers sa propre `ReadinessSource` ;
- migration complète des sockets inet hors `compat/linux/net.rs` vers un cœur socket natif ;
- extraction physique complète du gros `window_manager.rs` ;
- découpage du driver Bochs en scanout/backbuffer/present/drawing ;
- page-fault loader encore dans le domaine `thread` malgré sa fragmentation ;
- `compat/linux/mod.rs` reste trop gros : il doit finir en dispatcher fin.

V13 corrige tous les axes de performance immédiats listés pour le run Google ;
les éléments ci-dessus sont le chantier architectural suivant, pas des patches
magiques qu'il serait sûr de fusionner sans nouveau runtime.
