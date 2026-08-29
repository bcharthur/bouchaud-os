// Politique idle native Bouchaud OS.
//
// V6 avait volontairement désactivé HLT sur CPU0 pour isoler un éventuel lost
// wakeup. Les jalons suivants ont séparé IRQ souris, handshake scheduler,
// WaitSource et WaitQueue. Garder ce mode diagnostic actif coûte maintenant
// énormément : le BSP tourne en PAUSE/scheduler alors qu'il n'a rien à faire.
//
// Politique finale : CPU0 utilise le même handshake `cli -> publish idle ->
// release BKL -> sti; hlt` que les AP.

/// Vrai HLT événementiel sur le BSP.
pub const BSP_HLT_ENABLED: bool = true;

/// Alias de compatibilité avec les logs/branches V6 historiques.
/// `false` signifie que le mode "BSP safe busy-return" est désactivé.
pub const BSP_SAFE_IDLE_DIAGNOSTIC: bool = !BSP_HLT_ENABLED;

/// Repli uniquement si BSP_HLT_ENABLED est remis à false pour diagnostic.
const BSP_SAFE_IDLE_PAUSES: usize = 64;

#[inline]
fn bsp_safe_relax() {
    for _ in 0..BSP_SAFE_IDLE_PAUSES {
        core::hint::spin_loop();
    }
}
