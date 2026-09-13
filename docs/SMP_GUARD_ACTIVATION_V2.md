# Correctif SMP guard activation V2

Le correctif précédent déclarait `SmpBootstrapGuard`, mais ne l'instanciait pas dans
`init_probe()`. Le code compilait donc, mais `SMP_BOOTSTRAP_GUARD` ne passait jamais
à `true` et les interruptions du BSP n'étaient jamais désactivées pendant INIT/SIPI.

Ce correctif ajoute:

```rust
let _bootstrap_guard = SmpBootstrapGuard::enter();
```

juste après `SMP4_STAGE bootstrap-identity-ok` et avant l'activation LAPIC / INIT / SIPI.

Le guard reste vivant jusqu'à la fin de `init_probe()` et restaure l'état IRQ via `Drop`.
