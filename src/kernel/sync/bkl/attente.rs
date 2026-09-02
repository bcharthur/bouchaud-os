// BOUCHAUD_P1_BKL_PARK_WAKE_V1
//
// Masque des CPU arretes en attendant ce verrou, et de quoi le mesurer.
// `PARKED` est la seule donnee que le liberateur consulte : tant qu'il vaut
// zero, une liberation ne coute rien de plus qu'avant.
static PARKED: AtomicU64 = AtomicU64::new(0);
static TOTAL_PARKS: AtomicU64 = AtomicU64::new(0);
static TOTAL_WAKE_IPIS: AtomicU64 = AtomicU64::new(0);

// BOUCHAUD_P0_BKL_TARGETED_WAKE_V2
//
// Le V1 reveillait TOUS les CPU gares a chaque liberation. Le runtime Google
// montre que cela cree un troupeau : plusieurs milliers d'IPI, puis des milliers
// de reveils qui n'aboutissent a aucune acquisition. Un BKL n'a pourtant qu'un
// seul gagnant possible. On ne reveille donc qu'un seul waiter par liberation,
// avec un curseur round-robin pour eviter de toujours favoriser le meme CPU.
//
// Le curseur n'est qu'un mecanisme d'equite : la correction ne depend pas de sa
// valeur exacte. `fetch_add` evite qu'une liberation tres rapide sur un autre CPU
// ecrase le progres d'un liberateur precedent.
static WAKE_CURSOR: AtomicUsize = AtomicUsize::new(0);

// La priorite des continuations scheduler vit maintenant dans
// `ordonnanceur/etat.rs` + `ordonnanceur/priorite.rs`.

// BOUCHAUD_BKL_ADAPTIVE_IDLE
//
// Sous Ladybird plusieurs processus entrent en noyau en parallele. Le BKL
// serialise encore ces passages : un spin pur faisait alors consommer un vCPU
// entier a chaque contender. On garde un court spin actif pour les sections
// critiques tres breves, puis on parque le CPU avec HLT si IF est actif.
const BKL_ACTIVE_SPINS: usize = 64;

// BOUCHAUD_P1_BKL_PARK_WAKE_V1
//
// CE QUI A CHANGE, ET POURQUOI
// ----------------------------
// Ce parking datait d'un noyau ou un IPI de quantum partait toutes les 4 ms a
// TOUS les AP. Le dormeur n'avait donc pas besoin d'etre reveille : il l'etait
// de toute facon. BOUCHAUD_P0_TARGETED_SCHED_IPI_V1 a supprime cette diffusion
// -- a juste titre, elle coutait ~250 IPI/s par coeur inutilement -- et le
// parking s'est retrouve sans reveilleur.
//
// Ce qui reste ne suffit pas :
//   * le PIT ne bat que sur le BSP ;
//   * l'IPI de quantum ne vise que les AP qui executent une tache UTILISATEUR
//     (`running_user_cpu_mask`), donc jamais un AP dont `CURRENT` vaut NO_TASK
//     -- exactement l'etat de sa boucle idle, qui appelle pourtant `enter()` ;
//   * `publish_ready` ne reveille que le CPU auquel il destine une tache, et
//     seulement s'il l'a vu `is_idle` -- un CPU peut donc s'arreter juste
//     apres ce test.
//
// Un AP pouvait ainsi s'arreter sur un verrou libre. Les autres chemins le
// rattrapaient en pratique, mais aucun ne le garantissait : c'est la
// definition d'un reveil perdu.
//
// LE PROTOCOLE
// ------------
// Symetrique de celui du scheduler (V14), avec le liberateur pour reveilleur :
//
//     dormeur                          liberateur
//     -------                          ----------
//     CLI                              OWNER <- FREE      (SeqCst)
//     PARKED |= bit    (SeqCst)        lire PARKED        (SeqCst)
//     relire OWNER     (SeqCst)        reveiller ce qui y est
//     libre ? -> repartir sans dormir
//     STI; HLT
//
// AUCUN REVEIL PERDU
// ------------------
// Les quatre acces sont SeqCst, donc totalement ordonnes. Supposons que le
// dormeur s'arrete (il n'a pas vu FREE) et qu'un liberateur R ne voie pas son
// bit. Alors la lecture de PARKED par R precede la pose du bit, et comme R
// ecrit FREE avant de lire PARKED :
//
//     R.store(FREE) < R.load(PARKED) < dormeur.pose(bit) < dormeur.load(OWNER)
//
// La relecture du dormeur voit donc FREE -- il ne dort pas -- ou un
// proprietaire O acquis APRES. Dans ce second cas O relira PARKED apres sa
// propre acquisition, donc apres la pose du bit, et le reveillera. Un
// `Release`/`Acquire` ne suffirait pas ici : c'est un motif de tampon
// d'ecriture, que seul l'ordre total interdit.
//
// Le `sti; hlt` ferme la derniere fenetre : un IPI arrive entre la pose du bit
// et le `hlt` reste pendant dans l'APIC local, et l'ombre du `sti` garantit que
// le `hlt` le prend au lieu de le perdre.
///
/// # Le CPU est relu ICI, apres le masquage
///
/// L'appelant capture son index de CPU avant sa boucle d'attente. Entre deux
/// tours, les interruptions sont ACTIVES : une IPI de preemption peut commuter,
/// et la pile noyau reprendre ailleurs. L'index capture designe alors un AUTRE
/// coeur -- et poser le bit d'un autre coeur dans `PARKED` avant de s'arreter
/// est un reveil perdu par construction : le liberateur reveille celui dont le
/// bit est pose, jamais celui qui dort. Le dormeur ne repart qu'a la prochaine
/// interruption sans rapport, ce qui se compte en secondes.
///
/// `prepare_lock_park` commence par un `cli`. Apres lui, ce CPU ne peut plus
/// changer sous nos pieds : c'est le seul endroit ou lire l'index soit sur.
#[inline]
fn wait_for_owner_change(active_spins: &mut usize, reprise_prioritaire: bool) {
    if *active_spins < BKL_ACTIVE_SPINS {
        *active_spins += 1;
        COMPTES.note_spin();
        spin_loop();
        return;
    }

    *active_spins = 0;

    // Ne jamais faire STI depuis un contexte qui avait IF=0 (ex. IRQ).
    // Dans ce cas rare on conserve le spin actif : un tel contexte ne peut pas
    // dormir, et il n'a donc pas besoin d'etre reveille.
    if !interrupts::are_enabled() {
        // Compte a part : ce repli est le SEUL chemin qui tourne encore a vide,
        // et il se lit comme un spin ordinaire dans tout autre compteur.
        COMPTES.note_spin_irq_masquees();
        spin_loop();
        return;
    }

    crate::arch::x86_64::cpu::prepare_lock_park();
    let cpu = cpu();
    let bit = 1u64 << cpu;
    PARKED.fetch_or(bit, Ordering::SeqCst);

    let owner = owner_load(Ordering::SeqCst);
    let reprises = RESUME_WAITERS.load(Ordering::SeqCst);
    let priorite_bloque = !reprise_prioritaire && reprises != 0;
    let handoff_bloque = !reprise_prioritaire
        && reprises == 0
        && owner == FREE
        && handoff_bloque_waiter(cpu);

    if owner == FREE && !priorite_bloque && !handoff_bloque {
        // Libre ET autorise pour ce waiter : repartir tout de suite plutot que
        // dormir en attendant un reveil qui n'aura plus lieu.
        //
        // V10 ajoute une exception : si un AUTRE CPU est explicitement la
        // cible du handoff ordinaire, on reste gare. La cible, elle, repart.
        PARKED.fetch_and(!bit, Ordering::SeqCst);
        crate::arch::x86_64::cpu::abort_lock_park();
        return;
    }
    if owner == FREE && priorite_bloque {
        PRIORITY_PARK_FREE_OWNER.fetch_add(1, Ordering::Relaxed);
    }

    TOTAL_PARKS.fetch_add(1, Ordering::Relaxed);
    // Sur QUI on s'arrete. Un proprietaire qui domine cette ventilation est le
    // detenteur a instrumenter ; une repartition plate est de la contention.
    COMPTES.note_park(owner.wrapping_sub(1));
    PARKS_DEPUIS_ACQUISITION[cpu].fetch_add(1, Ordering::Relaxed);
    crate::arch::x86_64::cpu::commit_lock_park();
    PARKED.fetch_and(!bit, Ordering::SeqCst);
}

/// Parkings subis par ce CPU depuis sa derniere acquisition reussie.
///
/// Au-dela du premier, chacun est un reveil qui n'a servi a rien : le CPU a
/// ete rappele, n'a pas obtenu le verrou, et s'est rendormi. Avec le reveil
/// cible V2 ce compteur mesure surtout les courses perdues face a un nouveau
/// demandeur, et non plus un reveil en troupeau volontaire.
static PARKS_DEPUIS_ACQUISITION: [AtomicU64; MAX_CPUS] =
    [const { AtomicU64::new(0) }; MAX_CPUS];

/// Solde les parkings de ce CPU au moment ou il obtient enfin le verrou.
///
/// Le PREMIER parking d'une attente a servi : il a mene a cette acquisition.
/// Chacun des suivants est un reveil qui n'a rien produit -- le CPU a ete
/// rappele, un autre a gagne, il s'est rendormi.
#[inline]
fn solde_parkings(cpu: usize) {
    let parks = PARKS_DEPUIS_ACQUISITION[cpu].swap(0, Ordering::Relaxed);
    COMPTES.note_reveils_improductifs(parks.saturating_sub(1));
}

/// Choisit un seul CPU gare, de facon round-robin.
///
/// La selection porte sur l'instantane `parked`. La valeur peut devenir obsolete
/// avant l'IPI (le waiter peut avoir avorte son parking), ce qui ne compromet
/// pas la correction : au pire l'IPI est superflu. La vivacite reste garantie
/// par le protocole SeqCst de publication/relecture de `OWNER`.
#[inline]
fn choisit_waiter(parked: u64, releasing_cpu: usize) -> Option<usize> {
    if parked == 0 {
        return None;
    }

    let depart = WAKE_CURSOR.fetch_add(1, Ordering::Relaxed) % MAX_CPUS;
    for decalage in 0..MAX_CPUS {
        let target = (depart + decalage) % MAX_CPUS;
        if target == releasing_cpu {
            continue;
        }
        if parked & (1u64 << target) != 0 {
            return Some(target);
        }
    }
    None
}

/// Rappelle UN CPU arrete sur ce verrou. A appeler APRES `OWNER <- FREE`.
///
/// Pourquoi un seul suffit : si la cible se reveille mais perd la course, c'est
/// qu'un autre CPU a acquis le BKL ; ce nouveau proprietaire executera a son tour
/// ce reveil cible lors de sa liberation. Si personne n'a acquis entre-temps,
/// la cible relit `OWNER == FREE` et ne peut pas se rendormir sur ce meme etat.
/// On conserve ainsi la vivacite du protocole V1 sans reveiller N concurrents
/// pour une ressource qui ne peut avoir qu'un gagnant.
#[inline]
fn wake_parked_waiters(releasing_cpu: usize) {
    let parked = PARKED.load(Ordering::SeqCst);
    let eligibles = parked & !(1u64 << releasing_cpu);
    if eligibles == 0 {
        return;
    }

    // Une continuation qui restaure sa profondeur passe avant un nouvel
    // entrant. Si elle est publiee mais pas encore garee, elle tourne deja :
    // reveiller un waiter normal ne ferait que recreer la course que V3
    // cherche precisement a supprimer.
    let reprises = RESUME_WAITERS.load(Ordering::SeqCst) & !(1u64 << releasing_cpu);
    let reprises_garees = eligibles & reprises;
    let candidats = if reprises != 0 {
        if reprises_garees == 0 {
            // Une reprise est publiee mais tourne encore entre deux essais :
            // reveiller un waiter normal recreerait le barging V2.
            PRIORITY_WAKE_SUPPRESSED.fetch_add(1, Ordering::Relaxed);
            return;
        }
        reprises_garees
    } else {
        eligibles
    };

    // Une réservation ordinaire déjà fraîche survit à une libération
    // intermédiaire. C'est ce qui ferme la course où un barger a obtenu OWNER
    // juste avant que le libérateur précédent publie sa cible.
    if reprises == 0 && handoff_reveille_reserve_si_gare(releasing_cpu) {
        return;
    }

    let Some(target) = choisit_waiter(candidats, releasing_cpu) else {
        return;
    };

    if reprises_garees & (1u64 << target) != 0 {
        // Les reprises scheduler ont leur propre bitmap prioritaire et ne
        // doivent jamais être retardées par une réservation ordinaire.
        handoff_cancel_for_resume();
        PRIORITY_WAKEUPS.fetch_add(1, Ordering::Relaxed);
        TOTAL_WAKE_IPIS.fetch_add(1, Ordering::Relaxed);
        COMPTES.note_wake(target);
        crate::arch::x86_64::cpu::wake_parked_cpu(target);
        return;
    }

    // V10 : le CPU choisi devient le prochain entrant ordinaire privilégié
    // AVANT l'IPI. Les autres entrants le voient et se retirent.
    handoff_prepare_and_wake(target);
}
