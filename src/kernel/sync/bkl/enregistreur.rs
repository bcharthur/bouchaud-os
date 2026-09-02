// BOUCHAUD_P0_BKL_ENREGISTREUR_V1
//
// POURQUOI UN ENREGISTREUR DE VOL
// -------------------------------
// L'assertion `release par un CPU non proprietaire` dit QUE l'etat est casse
// (`DEPTH[cpu] > 0` alors que `OWNER == FREE`), jamais QUI l'a casse. Les
// sondes existantes ne gardent que le DERNIER evenement de chaque sorte : elles
// donnent une photo, pas la sequence. Or ce qu'il faut ici est precisement la
// sequence -- la transition qui a decouple les deux moities de l'etat.
//
// CE QU'IL N'A PAS LE DROIT DE FAIRE
// ----------------------------------
// Allouer -- l'allocateur est sous ce verrou. Prendre un verrou -- ce serait le
// verrou lui-meme. Formater -- `core::fmt` peut fauter et appelle du code
// arbitraire. Il ne fait donc que des `store` relaxes dans un anneau statique,
// et tout le texte est fabrique au moment du vidage, une seule fois.
//
// POURQUOI UN ANNEAU GLOBAL ET NON PAR CPU
// ----------------------------------------
// La question posee est un ENTRELACEMENT entre CPU. Un anneau par CPU donne
// quatre listes qu'on ne peut plus remettre dans l'ordre : c'est exactement
// l'information qui manque. Le `fetch_add` global coute une ligne de cache
// partagee, et c'est le prix de l'ordre total.
//
// POURQUOI IL N'EXISTE QU'EN `debug_assertions`
// ---------------------------------------------
// Il n'est la que pour expliquer un `debug_assert!`. Le compiler en release
// ferait payer une ligne de cache partagee a chaque prise du gros verrou pour
// une trace que personne ne lira jamais.
#[cfg(debug_assertions)]
pub mod enregistreur {
    use super::*;
    use core::sync::atomic::AtomicBool;

    /// Nature de la transition. Les valeurs sont stables : elles sont relues
    /// telles quelles dans le vidage.
    pub const ENTER: u8 = 1;
    pub const REENTER: u8 = 2;
    pub const TRY_ENTER: u8 = 3;
    pub const GUARD_DROP: u8 = 4;
    pub const RELEASE: u8 = 5;
    pub const SUSPEND: u8 = 6;
    pub const RESUME_BEGIN: u8 = 7;
    pub const RESUME_OK: u8 = 8;
    pub const SWITCH_BEFORE: u8 = 9;
    pub const SWITCH_AFTER: u8 = 10;
    pub const DETACHED_CHECK: u8 = 11;

    fn nom(kind: u8) -> &'static str {
        match kind {
            ENTER => "ENTER",
            REENTER => "REENTER",
            TRY_ENTER => "TRY_ENTER",
            GUARD_DROP => "GUARD_DROP",
            RELEASE => "RELEASE",
            SUSPEND => "SUSPEND",
            RESUME_BEGIN => "RESUME_BEGIN",
            RESUME_OK => "RESUME_OK",
            SWITCH_BEFORE => "SWITCH_BEFORE",
            SWITCH_AFTER => "SWITCH_AFTER",
            DETACHED_CHECK => "DETACHED_CHECK",
            _ => "?",
        }
    }

    /// 256 transitions gardees et videes. Sous SMP4, 64 transitions globales
    /// ne suffisaient pas toujours a conserver la derniere sequence complete
    /// du CPU fautif entre deux retours de `schedule()` detached.
    const TAILLE: usize = 256;
    /// Nombre de transitions imprimees sur violation.
    const VIDAGE: usize = TAILLE;

    /// Une case de l'anneau. Huit `u64` = 64 octets, soit une ligne de cache :
    /// deux cases voisines ne se disputent jamais la meme.
    struct Case {
        /// Numero d'ordre global. Ecrit en DERNIER : une case dont le `seq` est
        /// a jour a tous ses autres champs a jour.
        seq: AtomicU64,
        /// kind | cpu | owner_avant | owner_apres | depth_avant | depth_apres
        /// | cpu_du_garde | phase, un octet chacun.
        etat: AtomicU64,
        /// index de tache (32 bits bas) | tid (32 bits hauts).
        tache: AtomicU64,
        pid: AtomicU64,
        syscall: AtomicU64,
        /// Selon `kind` : profondeur sauvegardee, `from|to`, ou identite du garde.
        aux: AtomicU64,
        /// RSP au moment de la transition : identifie la CONTINUATION, ce qu'un
        /// numero de tache ne fait pas -- une meme tache a plusieurs cadres.
        rsp: AtomicU64,
        /// `TSS.rsp0` courant : la pile noyau que ce CPU est cense servir.
        /// Deux CPU qui affichent la meme valeur sont sur la meme pile.
        kstack: AtomicU64,
    }

    impl Case {
        const fn vide() -> Self {
            Self {
                seq: AtomicU64::new(0),
                etat: AtomicU64::new(0),
                tache: AtomicU64::new(0),
                pid: AtomicU64::new(0),
                syscall: AtomicU64::new(0),
                aux: AtomicU64::new(0),
                rsp: AtomicU64::new(0),
                kstack: AtomicU64::new(0),
            }
        }
    }

    static ANNEAU: [Case; TAILLE] = [const { Case::vide() }; TAILLE];
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    /// Gele l'anneau pendant le vidage. Sans lui, les autres CPU -- qui ne sont
    /// arretes qu'APRES le releve -- ecraseraient les cases qu'on est en train
    /// de lire, et le vidage montrerait la fin de l'histoire a la place du
    /// debut.
    static GEL: AtomicBool = AtomicBool::new(false);

    #[inline]
    fn rsp_courant() -> u64 {
        let rsp: u64;
        unsafe { core::arch::asm!("mov {}, rsp", out(reg) rsp, options(nomem, nostack)) };
        rsp
    }

    #[inline]
    fn octet(valeur: usize) -> u64 {
        (valeur & 0xFF) as u64
    }

    /// Enregistre une transition. Aucun format, aucune allocation, aucun verrou.
    ///
    /// `garde_cpu` vaut `usize::MAX` quand la notion n'a pas de sens (tout sauf
    /// `GUARD_DROP`), et le vidage l'imprime alors `-`.
    #[inline]
    pub fn note(
        kind: u8,
        cpu: usize,
        owner_avant: usize,
        owner_apres: usize,
        depth_avant: usize,
        depth_apres: usize,
        garde_cpu: usize,
        aux: u64,
    ) {
        if GEL.load(Ordering::Relaxed) {
            return;
        }
        let seq = SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1;
        let case = &ANNEAU[(seq as usize) % TAILLE];

        // Tout est lu POUR `cpu`, l'index que le verrou lui-meme utilise --
        // jamais via GS. Trois `rdmsr` par transition enregistree seraient
        // trois sorties de machine virtuelle, et l'effet Heisenberg suffirait a
        // deplacer la course qu'on cherche a observer.
        let (index, syscall_nr, phase, _site, _aux) =
            crate::kernel::task::stall_probe_context_pour(cpu);
        let (tid, kstack) = {
            let per_cpu = crate::arch::x86_64::usermode::per_cpu_for(cpu);
            (per_cpu.current, per_cpu.kernel_rsp)
        };

        case.etat.store(
            octet(kind as usize)
                | (octet(cpu) << 8)
                | (octet(owner_avant) << 16)
                | (octet(owner_apres) << 24)
                | (octet(depth_avant) << 32)
                | (octet(depth_apres) << 40)
                | (octet(garde_cpu) << 48)
                | (octet(phase as usize) << 56),
            Ordering::Relaxed,
        );
        case.tache
            .store((index as u64 & 0xFFFF_FFFF) | (tid << 32), Ordering::Relaxed);
        case.pid
            .store(crate::kernel::task::pid_pour_sonde(cpu), Ordering::Relaxed);
        case.syscall.store(syscall_nr, Ordering::Relaxed);
        case.aux.store(aux, Ordering::Relaxed);
        case.rsp.store(rsp_courant(), Ordering::Relaxed);
        case.kstack.store(kstack, Ordering::Relaxed);
        // En dernier, et en Release : un lecteur qui voit ce `seq` voit tout
        // le reste de la case.
        case.seq.store(seq, Ordering::Release);
    }

    /// Imprime les dernieres transitions et GELE l'anneau definitivement.
    ///
    /// Appele depuis le `panic_handler`, donc dans un contexte ou plus rien ne
    /// doit etre suppose valide : on ne lit que des atomiques et on n'appelle
    /// que la sortie serie.
    pub fn vide() {
        if GEL.swap(true, Ordering::AcqRel) {
            // Deja vide par un autre CPU : ne pas entrelacer deux vidages.
            return;
        }
        let derniere = SEQUENCE.load(Ordering::Acquire);
        if derniere == 0 {
            crate::serial_println_brut!("[BKL-FR] anneau vide");
            return;
        }
        let premiere = derniere.saturating_sub(VIDAGE as u64 - 1).max(1);
        crate::serial_println_brut!(
            "[BKL-FR] {} transitions (seq {}..{}), la plus recente en dernier",
            derniere - premiere + 1,
            premiere,
            derniere,
        );
        crate::serial_println_brut!(
            "[BKL-FR] seq kind cpu owner(av->ap) depth(av->ap) garde tache tid pid syscall/phase aux rsp kstack"
        );

        for seq in premiere..=derniere {
            let case = &ANNEAU[(seq as usize) % TAILLE];
            // Une case dont le `seq` ne correspond plus a ete recyclee entre
            // notre calcul et notre lecture. On le DIT au lieu d'imprimer des
            // champs qui appartiennent a une autre transition.
            if case.seq.load(Ordering::Acquire) != seq {
                crate::serial_println_brut!("[BKL-FR] {} <recyclee>", seq);
                continue;
            }
            let etat = case.etat.load(Ordering::Relaxed);
            let tache = case.tache.load(Ordering::Relaxed);
            let garde = ((etat >> 48) & 0xFF) as usize;
            crate::serial_println_brut!(
                "[BKL-FR] {} {} cpu={} owner={}->{} depth={}->{} garde={} tache={} tid={} pid={} sys={}/{} aux={:#x} rsp={:#x} kstack={:#x}",
                seq,
                nom((etat & 0xFF) as u8),
                (etat >> 8) & 0xFF,
                (etat >> 16) & 0xFF,
                (etat >> 24) & 0xFF,
                (etat >> 32) & 0xFF,
                (etat >> 40) & 0xFF,
                if garde == 0xFF { -1i64 } else { garde as i64 },
                (tache & 0xFFFF_FFFF) as u32,
                tache >> 32,
                case.pid.load(Ordering::Relaxed),
                case.syscall.load(Ordering::Relaxed),
                (etat >> 56) & 0xFF,
                case.aux.load(Ordering::Relaxed),
                case.rsp.load(Ordering::Relaxed),
                case.kstack.load(Ordering::Relaxed),
            );
        }
        crate::serial_println_brut!("[BKL-FR] fin");
    }
}

#[cfg(not(debug_assertions))]
pub mod enregistreur {
    pub const ENTER: u8 = 1;
    pub const REENTER: u8 = 2;
    pub const TRY_ENTER: u8 = 3;
    pub const GUARD_DROP: u8 = 4;
    pub const RELEASE: u8 = 5;
    pub const SUSPEND: u8 = 6;
    pub const RESUME_BEGIN: u8 = 7;
    pub const RESUME_OK: u8 = 8;
    pub const SWITCH_BEFORE: u8 = 9;
    pub const SWITCH_AFTER: u8 = 10;
    pub const DETACHED_CHECK: u8 = 11;

    #[inline(always)]
    pub fn note(
        _kind: u8, _cpu: usize, _owner_avant: usize, _owner_apres: usize,
        _depth_avant: usize, _depth_apres: usize, _garde_cpu: usize, _aux: u64,
    ) {}

    #[inline(always)]
    pub fn vide() {
        crate::serial_println_brut!("[BKL-FR] non compile (release)");
    }
}

/// Vide l'enregistreur de vol du gros verrou. Appele par le `panic_handler`.
pub fn vide_enregistreur() {
    enregistreur::vide();
}

/// Marque le point exact avant/apres `schedule()` d'une attente detached.
/// `phase`: 1=avant, 2=apres, 3=assertion finale. `aux` conserve la boucle.
pub fn note_detached_check(phase: u8, boucle: u64, depth: usize) {
    let _irq = LocalIrqGuard::acquire();
    let cpu = cpu();
    let owner = owner_load(Ordering::Acquire);
    enregistreur::note(
        enregistreur::DETACHED_CHECK,
        cpu,
        owner,
        owner,
        depth,
        depth,
        usize::MAX,
        (boucle << 8) | phase as u64,
    );
}
