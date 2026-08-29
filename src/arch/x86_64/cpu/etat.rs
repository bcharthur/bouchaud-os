// Identité CPU commune à tous les fragments.

/// Logical Bouchaud CPU index used by per-CPU accounting arrays.
///
/// The historical name is kept for source compatibility during NG1.
pub fn hardware_cpu_index() -> usize {
    smp::cpu_index().min(smp::MAX_CPUS - 1)
}
