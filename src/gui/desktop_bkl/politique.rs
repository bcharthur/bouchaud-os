// Politique V13 du desktop BKL.
//
// V9 injectait jusqu'à 4096 PAUSE après chaque libération pour laisser un
// waiter BKL avancer. V10 possède maintenant un handoff explicite ciblé et le
// desktop V12/V13 sait dormir à depth=0. Garder cette temporisation artificielle
// sous TCG brûle du CPU sans améliorer la correction.

/// Les checkpoints purement coopératifs sont espacés : les scopes lourds
/// `present*`, le wait INTERFACE et le handoff V10 restent indépendants.
const CHECKPOINT_MIN_NS: u64 = 8_000_000;

/// Plus de busy-window quand le système est calme.
const HANDOFF_SPINS_CALME: usize = 0;
/// Filet minuscule sous contention ; V10 fait le vrai arbitrage.
const HANDOFF_SPINS_CONTENTION: usize = 64;
