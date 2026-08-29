pub const MAX_CPUS: usize = 16;
const FREE: usize = 0;

static OWNER: AtomicUsize = AtomicUsize::new(FREE);
static DEPTH: [AtomicUsize; MAX_CPUS] = [const { AtomicUsize::new(0) }; MAX_CPUS];
#[inline]
fn cpu() -> usize {
    crate::arch::x86_64::usermode::cpu_index().min(MAX_CPUS - 1)
}

#[inline]
fn token(cpu: usize) -> usize {
    cpu + 1
}

/// Serialise les transitions OWNER/DEPTH contre les IRQ du CPU courant.
/// L'etat IF precedent est restaure exactement au Drop.
struct LocalIrqGuard {
    restore_enabled: bool,
}

impl LocalIrqGuard {
    #[inline]
    fn acquire() -> Self {
        let restore_enabled = interrupts::are_enabled();
        interrupts::disable();
        Self { restore_enabled }
    }
}

impl Drop for LocalIrqGuard {
    #[inline]
    fn drop(&mut self) {
        if self.restore_enabled {
            interrupts::enable();
        }
    }
}

