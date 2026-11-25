// Stable word identity and copyin.

#[inline]
fn wait_word_key(uaddr: u64) -> Option<CleMot> {
    // BOUCHAUD_PROCESSUS_COURANT_SANS_ARC_V1 : un `futex` par appel, sans part
    // d'`Arc<Process>` ni verrou du bloc par-CPU.
    crate::kernel::task::avec_processus_courant(|process| {
        // Garder explicitement le SpinLockGuard dans un scope local (V13.1) :
        // la cle est composee APRES sa liberation.
        let pml4;
        let translated = {
            let mut mm = process.mm.lock();
            pml4 = mm.space.pml4();
            match mm.space.translate(uaddr) {
                // Frame empruntee : identite physique, partagee entre
                // processus. La page zero n'est empruntee a personne : elle
                // est commune a TOUS les processus, une cle physique les
                // melangerait.
                Some(phys) if !mm.space.owns_frame(phys)
                    && !crate::kernel::vmm::est_page_zero(phys) => Some((0, phys)),
                _ => None,
            }
        };
        // BOUCHAUD_COW_V1 : frame privee, ou pas encore residente -- identite
        // VIRTUELLE (espace, adresse), stable a travers une casse de COW qui
        // change la frame sous le mot.
        translated.unwrap_or((pml4, uaddr))
    })
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
fn wait_word_bucket(key: CleMot) -> usize {
    let (espace, adresse) = key;
    let melange = adresse ^ espace.rotate_left(29);
    let mixed = melange ^ (melange >> 17) ^ (melange >> 33);
    (mixed as usize) & (WAIT_WORD_BUCKETS - 1)
}
