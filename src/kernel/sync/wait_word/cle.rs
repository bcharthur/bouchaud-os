// Stable word identity and copyin.

#[inline]
fn wait_word_key(uaddr: u64) -> Option<u64> {
    let process = crate::kernel::task::current_process_local()?;

    // Garder explicitement le SpinLockGuard dans un scope local.
    //
    // Avec l'expression chaînée directement en fin de fonction, Rust prolonge
    // la durée de vie du temporaire `SpinLockGuard` jusqu'à la destruction de
    // l'expression de retour. `process` était alors détruit avant le guard,
    // ce qui déclenchait E0597.
    let translated = {
        let mut mm = process.mm.lock();
        mm.space.translate(uaddr)
    };

    translated.or(Some(uaddr))
}

#[inline]
fn wait_word_read(uaddr: u64) -> Option<u32> {
    let process = crate::kernel::task::current_process_local()?;
    let mut raw = [0u8; 4];
    if !process.mm.lock().space.read(uaddr, &mut raw) {
        return None;
    }
    Some(u32::from_le_bytes(raw))
}

#[inline]
fn wait_word_bucket(key: u64) -> usize {
    let mixed = key ^ (key >> 17) ^ (key >> 33);
    (mixed as usize) & (WAIT_WORD_BUCKETS - 1)
}
