// La priorite d'ordonnancement change-t-elle une DECISION ?
//
// # Ce que cette sonde existe pour repondre
//
// `Priorite::Interactive` et `Priorite::Normale` existent, la latence
// reveil->execution est mesuree depuis longtemps, et les centiles sont publies
// par classe. Rien de tout cela ne dit si l'etiquette sert a quelque chose.
//
// Sur une machine au repos, les deux classes se reveillent en quelques
// microsecondes et leurs centiles se ressemblent. La difference n'apparait que
// SOUS CONTENTION : quand plus de taches sont pretes qu'il n'y a de coeurs, et
// que l'ordonnanceur doit choisir. Une priorite qui ne se voit pas la ne se
// voit nulle part.
//
// La sonde fabrique donc cette contention, puis compare.
//
// # Ce qu'elle ne prouve pas
//
// Elle mesure une machine emulee, sur une charge synthetique. Elle ne dit rien
// des latences sous Ladybird, ni sur le materiel de reference.

use crate::kernel::scheduler::latency;
use crate::kernel::task::{spawn_noyau_priorite, Priorite};
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// Bruleurs de temps processeur par defaut. Il en faut PLUS que de coeurs :
/// sinon chacun trouve un coeur libre, personne n'attend, et la priorite n'a
/// rien a arbitrer.
const BRULEURS: usize = 8;
/// Dormeurs par classe. Chaque reveil produit un echantillon de latence.
const DORMEURS_PAR_CLASSE: usize = 2;
/// Cycles sommeil/reveil par dormeur.
const CYCLES: u32 = 40;

static ARRET: AtomicBool = AtomicBool::new(false);
static BRULEURS_VIVANTS: AtomicU32 = AtomicU32::new(0);
static DORMEURS_VIVANTS: AtomicU32 = AtomicU32::new(0);
/// Somme de controle des bruleurs. Sans elle, l'optimiseur a le droit de
/// supprimer la boucle entiere : une charge qui ne charge rien mesurerait une
/// machine au repos.
static TEMOIN: AtomicU64 = AtomicU64::new(0);

fn bruleur() -> ! {
    let mut somme = 0u64;
    while !ARRET.load(Ordering::Relaxed) {
        for i in 0..50_000u64 {
            somme = somme.wrapping_add(i ^ (somme >> 3));
        }
        TEMOIN.fetch_add(somme & 1, Ordering::Relaxed);
    }
    BRULEURS_VIVANTS.fetch_sub(1, Ordering::Release);
    crate::kernel::task::exit_current(0)
}

fn dormeur() -> ! {
    for _ in 0..CYCLES {
        // Un tick : le plus court sommeil qui passe par le chemin de reveil
        // complet -- echeance armee, tache bloquee, reveil par le timer,
        // remise en file, election. C'est ce trajet que la latence mesure.
        crate::kernel::task::sleep_ticks(1);
    }
    DORMEURS_VIVANTS.fetch_sub(1, Ordering::Release);
    crate::kernel::task::exit_current(0)
}

/// Met les deux classes en concurrence et compare leurs centiles.
pub fn execute() {
    execute_avec(BRULEURS)
}

/// Meme sonde, avec un nombre de bruleurs choisi.
///
/// Le parametre n'est pas un confort : la charge est la variable de
/// l'experience. Zero bruleur mesure la machine au repos et sert de reference ;
/// quatre remplissent les coeurs sans les surcharger ; huit forcent la file
/// d'attente. Un resultat qui ne tiendrait qu'a une seule valeur ne dirait
/// rien du systeme.
pub fn execute_avec(bruleurs_demandes: usize) {
    latency::remise_a_zero();
    ARRET.store(false, Ordering::Release);
    TEMOIN.store(0, Ordering::Relaxed);
    BRULEURS_VIVANTS.store(0, Ordering::Release);
    DORMEURS_VIVANTS.store(0, Ordering::Release);

    // La contention d'abord : les dormeurs doivent trouver la machine occupee.
    for _ in 0..bruleurs_demandes {
        BRULEURS_VIVANTS.fetch_add(1, Ordering::Release);
        if !spawn_noyau_priorite(bruleur, "lat-brule", Priorite::Normale) {
            BRULEURS_VIVANTS.fetch_sub(1, Ordering::Release);
        }
    }
    let bruleurs = BRULEURS_VIVANTS.load(Ordering::Acquire);
    if bruleurs_demandes != 0 && bruleurs == 0 {
        crate::serial_println!("SCHED_LATENCE_ECHEC raison=aucun-bruleur");
        return;
    }

    for (priorite, nom) in [
        (Priorite::Interactive, "lat-inter"),
        (Priorite::Normale, "lat-normal"),
    ] {
        for _ in 0..DORMEURS_PAR_CLASSE {
            DORMEURS_VIVANTS.fetch_add(1, Ordering::Release);
            if !spawn_noyau_priorite(dormeur, nom, priorite) {
                DORMEURS_VIVANTS.fetch_sub(1, Ordering::Release);
            }
        }
    }
    let dormeurs = DORMEURS_VIVANTS.load(Ordering::Acquire);
    if dormeurs < 2 {
        ARRET.store(true, Ordering::Release);
        crate::serial_println!("SCHED_LATENCE_ECHEC raison=dormeurs-insuffisants");
        return;
    }

    let fini = crate::kernel::timer::attente_bornee(30_000, || {
        DORMEURS_VIVANTS.load(Ordering::Acquire) == 0
    });
    ARRET.store(true, Ordering::Release);
    let calme = crate::kernel::timer::attente_bornee(10_000, || {
        BRULEURS_VIVANTS.load(Ordering::Acquire) == 0
    });

    let inter = latency::centiles(latency::INTERACTIVE);
    let normale = latency::centiles(latency::NORMALE);
    crate::serial_println!(
        "SCHED_LATENCE bruleurs={} dormeurs={} fini={} calme={} temoin={}",
        bruleurs, dormeurs, fini as u8, calme as u8,
        TEMOIN.load(Ordering::Relaxed),
    );
    crate::serial_println!(
        "SCHED_LATENCE_INTERACTIVE count={} p50_ns={} p95_ns={} p99_ns={} max_ns={}",
        inter.count, inter.p50_ns, inter.p95_ns, inter.p99_ns, inter.max_ns,
    );
    crate::serial_println!(
        "SCHED_LATENCE_NORMALE count={} p50_ns={} p95_ns={} p99_ns={} max_ns={}",
        normale.count, normale.p50_ns, normale.p95_ns, normale.p99_ns, normale.max_ns,
    );

    if !fini {
        crate::serial_println!("SCHED_LATENCE_ECHEC raison=echeance-dormeurs");
        return;
    }
    // Des echantillons dans LES DEUX classes : comparer une classe pleine a
    // une classe vide donnerait un verdict qui ne compare rien.
    if inter.count == 0 || normale.count == 0 {
        crate::serial_println!(
            "SCHED_LATENCE_ECHEC raison=classe-sans-echantillon inter={} normale={}",
            inter.count, normale.count,
        );
        return;
    }
    // LE VERDICT EST UNE COMPARAISON, PAS UN SEUIL.
    //
    // Un seuil en nanosecondes decrirait la machine qui l'a mesure : sous QEMU
    // sans KVM, les valeurs absolues ne veulent rien dire. Ce qui doit tenir
    // sur n'importe quelle machine, c'est l'ORDRE : la classe interactive ne
    // doit pas etre servie plus tard que la normale, sous la meme charge et
    // sur la meme periode.
    if inter.p95_ns > normale.p95_ns {
        crate::serial_println!(
            "SCHED_LATENCE_INVERSEE p95_interactive={} p95_normale={} \
consequence=la-priorite-ne-change-aucune-decision",
            inter.p95_ns, normale.p95_ns,
        );
        return;
    }
    crate::serial_println!(
        "SCHED_LATENCE_OK p95_interactive={} p95_normale={}",
        inter.p95_ns, normale.p95_ns,
    );
}
