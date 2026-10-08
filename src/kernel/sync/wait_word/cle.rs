// Stable word identity and copyin.

#[inline]
fn wait_word_key(uaddr: u64) -> Option<u64> {
    // BOUCHAUD_PROCESSUS_COURANT_SANS_ARC_V1 : un `futex` par appel, sans part
    // d'`Arc<Process>` ni verrou du bloc par-CPU.
    let translated = crate::kernel::task::avec_processus_courant(|process| {
        // Garder explicitement le SpinLockGuard dans un scope local (V13.1).
        let translated = {
            let mut mm = process.mm.lock();
            mm.space.translate(uaddr)
        };
        translated
    })?;

    translated.or(Some(uaddr))
}

#[inline]
fn wait_word_read(uaddr: u64) -> Option<u32> {
    let mut raw = [0u8; 4];
    let lu = crate::kernel::task::avec_processus_courant(|process| process.mm.lock().space.read(uaddr, &mut raw))?;
    if !lu {
        return None;
    }
    Some(u32::from_le_bytes(raw))
}

#[inline]
fn wait_word_bucket(key: u64) -> usize {
    let mixed = key ^ (key >> 17) ^ (key >> 33);
    (mixed as usize) & (WAIT_WORD_BUCKETS - 1)
}
