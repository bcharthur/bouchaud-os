pub const MAX_CPUS: usize = 16;
const FREE: usize = 0;

// OWNER et DEPTH sont une SEULE machine d'etat atomique. Les huit bits bas
// portent le token CPU et les bits hauts la profondeur. `OWNER=local, DEPTH=0`
// n'est plus un etat publiable : acquisition, reentrance, suspension et
// liberation sont chacune un CAS sur ce mot.
const OWNER_BITS: u32 = 8;
const OWNER_MASK: u64 = (1u64 << OWNER_BITS) - 1;
static ETAT: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug)]
struct EtatBkl {
    owner: usize,
    depth: usize,
}

#[inline]
fn encode_etat(owner: usize, depth: usize) -> u64 {
    debug_assert!(owner <= OWNER_MASK as usize, "smp_lock: token OWNER hors format");
    debug_assert_eq!(owner == FREE, depth == 0,
        "smp_lock: etat OWNER/DEPTH incoherent a l'encodage");
    ((depth as u64) << OWNER_BITS) | owner as u64
}

#[inline]
fn decode_etat(raw: u64) -> EtatBkl {
    EtatBkl {
        owner: (raw & OWNER_MASK) as usize,
        depth: (raw >> OWNER_BITS) as usize,
    }
}

#[inline]
fn etat_charge(ordering: Ordering) -> EtatBkl {
    decode_etat(ETAT.load(ordering))
}

#[inline]
fn owner_load(ordering: Ordering) -> usize {
    etat_charge(ordering).owner
}

#[inline]
fn depth_load(cpu: usize, ordering: Ordering) -> usize {
    let etat = etat_charge(ordering);
    if etat.owner == token(cpu) { etat.depth } else { 0 }
}

#[inline]
fn essaie_acquerir_etat(
    cpu: usize,
    depth: usize,
    success: Ordering,
    failure: Ordering,
) -> Result<(), EtatBkl> {
    debug_assert!(depth > 0, "smp_lock: acquisition de profondeur nulle");
    ETAT.compare_exchange(
        encode_etat(FREE, 0),
        encode_etat(token(cpu), depth),
        success,
        failure,
    )
    .map(|_| ())
    .map_err(decode_etat)
}

#[inline]
fn remplace_profondeur_possedee(
    cpu: usize,
    avant: usize,
    apres: usize,
    success: Ordering,
) -> Result<(), EtatBkl> {
    debug_assert!(avant > 0, "smp_lock: transition depuis profondeur nulle");
    let nouveau = if apres == 0 {
        encode_etat(FREE, 0)
    } else {
        encode_etat(token(cpu), apres)
    };
    ETAT.compare_exchange(
        encode_etat(token(cpu), avant),
        nouveau,
        success,
        Ordering::Acquire,
    )
    .map(|_| ())
    .map_err(decode_etat)
}

#[inline]
fn augmente_profondeur(cpu: usize) -> Result<(usize, usize), EtatBkl> {
    loop {
        let courant = etat_charge(Ordering::Acquire);
        if courant.owner != token(cpu) || courant.depth == 0 {
            return Err(courant);
        }
        let apres = courant.depth.checked_add(1).expect("smp_lock: profondeur saturee");
        match remplace_profondeur_possedee(cpu, courant.depth, apres, Ordering::AcqRel) {
            Ok(()) => return Ok((courant.depth, apres)),
            Err(observe) if observe.owner == token(cpu) && observe.depth > 0 => continue,
            Err(observe) => return Err(observe),
        }
    }
}

#[inline]
fn cpu() -> usize {
    crate::arch::x86_64::smp::cpu_index().min(MAX_CPUS - 1)
}

#[inline]
fn token(cpu: usize) -> usize {
    cpu + 1
}

/// Serialise les transitions locales et leurs metriques contre les IRQ.
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
