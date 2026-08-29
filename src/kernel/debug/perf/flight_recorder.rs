// Enregistreur de vol performance.
// Aucun Vec, aucune allocation, aucun SpinLock/BKL : ce chemin reste lisible
// même pendant une panique ou un blocage du noyau.

static PERF_WRITE_SEQ: AtomicU64 = AtomicU64::new(0);
static PERF_SLOT_SEQ: [AtomicU64; PERF_RING_CAPACITY] =
    [const { AtomicU64::new(0) }; PERF_RING_CAPACITY];
static PERF_TS_NS: [AtomicU64; PERF_RING_CAPACITY] =
    [const { AtomicU64::new(0) }; PERF_RING_CAPACITY];
static PERF_KIND: [AtomicU32; PERF_RING_CAPACITY] =
    [const { AtomicU32::new(0) }; PERF_RING_CAPACITY];
static PERF_PID: [AtomicU32; PERF_RING_CAPACITY] =
    [const { AtomicU32::new(0) }; PERF_RING_CAPACITY];
static PERF_CPU: [AtomicU32; PERF_RING_CAPACITY] =
    [const { AtomicU32::new(0) }; PERF_RING_CAPACITY];
static PERF_A0: [AtomicU64; PERF_RING_CAPACITY] =
    [const { AtomicU64::new(0) }; PERF_RING_CAPACITY];
static PERF_A1: [AtomicU64; PERF_RING_CAPACITY] =
    [const { AtomicU64::new(0) }; PERF_RING_CAPACITY];

#[inline]
pub fn perf_record(kind: u32, pid: u32, a0: u64, a1: u64) -> u64 {
    let seq = PERF_WRITE_SEQ.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    let i = (seq as usize) % PERF_RING_CAPACITY;

    // Slot rendu invalide pendant l'écriture, puis publication Release.
    PERF_SLOT_SEQ[i].store(0, Ordering::Relaxed);
    PERF_TS_NS[i].store(crate::kernel::timer::monotonic_ns(), Ordering::Relaxed);
    PERF_KIND[i].store(kind, Ordering::Relaxed);
    PERF_PID[i].store(pid, Ordering::Relaxed);
    let cpu = crate::arch::x86_64::usermode::cpu_index();
    PERF_CPU[i].store(cpu.min(u32::MAX as usize) as u32, Ordering::Relaxed);
    PERF_A0[i].store(a0, Ordering::Relaxed);
    PERF_A1[i].store(a1, Ordering::Relaxed);
    PERF_SLOT_SEQ[i].store(seq, Ordering::Release);
    seq
}

/// Vidage panic-safe : lecture d'atomiques uniquement, sans BKL.
///
/// Une entrée dont `slot_seq != seq` a été écrasée ou était en cours d'écriture
/// et est simplement ignorée.
pub fn dump_flight_recorder() {
    let end = PERF_WRITE_SEQ.load(Ordering::Acquire);
    let start = end.saturating_sub(PERF_DUMP_EVENTS as u64).saturating_add(1);

    crate::serial_println!(
        "[PERF-FR] begin start={} end={} capacity={}",
        start, end, PERF_RING_CAPACITY
    );

    if end == 0 {
        crate::serial_println!("[PERF-FR] fin");
        return;
    }

    let mut seq = start.max(1);
    while seq <= end {
        let i = (seq as usize) % PERF_RING_CAPACITY;
        let published = PERF_SLOT_SEQ[i].load(Ordering::Acquire);
        if published == seq {
            crate::serial_println!(
                "[PERF-FR] seq={} t_ns={} cpu={} pid={} kind={}({}) a0={} a1={}",
                seq,
                PERF_TS_NS[i].load(Ordering::Relaxed),
                PERF_CPU[i].load(Ordering::Relaxed),
                PERF_PID[i].load(Ordering::Relaxed),
                PERF_KIND[i].load(Ordering::Relaxed),
                perf_event_name(PERF_KIND[i].load(Ordering::Relaxed)),
                PERF_A0[i].load(Ordering::Relaxed),
                PERF_A1[i].load(Ordering::Relaxed),
            );
        }
        seq = seq.wrapping_add(1);
        if seq == 0 { break; }
    }
    crate::serial_println!("[PERF-FR] fin");
}
