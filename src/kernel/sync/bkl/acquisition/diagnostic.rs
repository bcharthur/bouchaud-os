// Diagnostic ultra-fin du chemin non bloquant `try_enter`.
//
// Pourquoi ces sites existent : le run Google V10 a montré un CPU0 propriétaire
// avec le site IRQ timer 60 qui restait vivant pendant des dizaines de secondes.
// Le timer pose 60 AVANT `smp_lock::try_enter()` et 61 juste APRÈS. Si 60 reste
// affiché alors que OWNER appartient à CPU0, la panne est entre le CAS et le
// retour de `try_enter`. Ces marqueurs rendent cette fenêtre observable.
//
// Les codes 600..699 sont réservés à l'acquisition BKL V11A. Ils n'ont aucune
// influence sur l'arbitrage ; ce ne sont que des stores de diagnostic déjà
// compris par `[SMP-SNAPSHOT]` / `[SMP-STALL]`.

static TRY_DIAG_ACTIVE: [AtomicU32; MAX_CPUS] =
    [const { AtomicU32::new(0) }; MAX_CPUS];

#[inline]
fn try_diag_begin(cpu: usize, variante: u32) {
    TRY_DIAG_ACTIVE[cpu].store(variante.max(1), Ordering::Relaxed);
    crate::kernel::task::stall_site_set(600, variante as u64);
}

#[inline]
fn try_diag_step(cpu: usize, site: u32, aux: u64) {
    if TRY_DIAG_ACTIVE[cpu].load(Ordering::Relaxed) != 0 {
        crate::kernel::task::stall_site_set(site, aux);
    }
}

#[inline]
fn try_diag_end(cpu: usize, site: u32, aux: u64) {
    try_diag_step(cpu, site, aux);
    TRY_DIAG_ACTIVE[cpu].store(0, Ordering::Relaxed);
}
