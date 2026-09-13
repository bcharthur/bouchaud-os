//! Prechauffage du cache de pages propres pour le navigateur.
//!
//! # Le defaut, tel que l'utilisateur le decrit
//!
//! « Sur le deuxieme demarrage Ladybird a demarre bien plus vite. »
//!
//! Les deux demarrages executent le meme binaire depuis le meme disque
//! memoire. Ce qui change entre eux, c'est le cache de pages propres :
//! `[BACKING-CACHE] clean_hit=5408 clean_miss=17346` au premier lancement,
//! et `[MM-NG6] fault_resolved=6878`. Chaque defaut de page manque, alloue
//! une trame, recopie quatre kibioctets, publie une entree de table -- et il
//! y en a plusieurs milliers avant que la premiere fenetre n'apparaisse.
//!
//! Le deuxieme lancement retrouve ces memes trames dans le cache et n'a plus
//! qu'a les rattacher. C'est cette difference-la qu'on peut offrir au premier.
//!
//! # Ce que ce module fait, et ce qu'il ne fait pas
//!
//! Il PRECHARGE : il demande au cache de pages propres les pages des binaires
//! du navigateur, puis les relache aussitot. Elles restent recuperables -- la
//! pression memoire peut les reprendre a tout moment -- mais tant que
//! personne n'en a besoin, elles sont la.
//!
//! Il NE LANCE AUCUN PROCESSUS. Demarrer les moteurs Web a l'amorcage
//! couterait leur memoire et leur processeur en permanence, pour un
//! utilisateur qui n'ouvrira peut-etre jamais le navigateur ; et il faudrait
//! decider quoi faire du processus deja lance quand il en demande un. Le
//! cache donne l'essentiel du gain sans aucune de ces questions.

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use crate::kernel::clean_page_cache::Key;
use crate::kernel::vmm::PAGE_SIZE;

/// Ce qu'on prechauffe, dans l'ordre ou le navigateur en a besoin.
///
/// L'hote d'abord -- c'est lui qu'`execve` charge --, puis les services qu'il
/// lance dans la seconde qui suit. Un chemin absent est saute sans bruit :
/// cette liste doit pouvoir survivre a une reorganisation de l'image.
const BINAIRES: &[&str] = &[
    "/bo-navigateur",
    "/usr/libexec/ladybird/WebContent",
    "/usr/libexec/ladybird/Compositor",
    "/usr/libexec/ladybird/RequestServer",
    "/usr/libexec/ladybird/ImageDecoder",
];

/// Plafond de pages prechauffees, toutes cibles confondues.
///
/// Soixante-quatre mebioctets. Le cache de pages propres en retient au plus
/// seize mille trois cent quatre-vingt-quatre (`MAX_RECLAIMABLE_PAGES`) : en
/// demander davantage ferait chasser par la fin ce qu'on vient de charger au
/// debut, ce qui est pire que de ne rien faire.
const PAGES_MAX: usize = 16_384;

/// Pages chargees entre deux pauses.
///
/// Le prechauffage ne doit jamais se voir. Mille pages -- quatre mebioctets --
/// prennent quelques millisecondes, apres quoi le fil rend la main.
const PAGES_PAR_TRANCHE: usize = 1_024;

static PAGES_PRECHAUFFEES: AtomicUsize = AtomicUsize::new(0);
static FICHIERS_PRECHAUFFES: AtomicUsize = AtomicUsize::new(0);
static DUREE_NS: AtomicU64 = AtomicU64::new(0);
static TERMINE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Pages prechauffees, fichiers couverts, duree totale.
pub fn compteurs() -> (usize, usize, u64) {
    (
        PAGES_PRECHAUFFEES.load(Ordering::Relaxed),
        FICHIERS_PRECHAUFFES.load(Ordering::Relaxed),
        DUREE_NS.load(Ordering::Relaxed),
    )
}

/// Le prechauffage a-t-il fini ?
pub fn termine() -> bool {
    TERMINE.load(Ordering::Acquire)
}

/// Charge dans le cache les pages d'UN fichier. Rend le nombre de pages.
fn prechauffe_un(chemin: &str, budget: usize) -> usize {
    // Le verrou du systeme de fichiers est pris pour la RESOLUTION seule, et
    // rendu avant la moindre page : `acquire` peut lire le support, et tenir
    // le systeme de fichiers pendant une lecture bloquerait tout le monde.
    let (node, taille) = {
        let fs = crate::fs::ramfs::fs();
        let Some(node) = fs.resolve(chemin, 0) else { return 0 };
        drop(fs);
        (node, crate::fs::backing::logical_len(node))
    };
    let Some(generation) = crate::fs::backing::generation(node) else {
        // Pas d'etendue enregistree : le contenu vit deja dans le noeud, il
        // n'y a rien a precharger et rien a dire.
        return 0;
    };
    if taille == 0 {
        return 0;
    }

    let pages_du_fichier = taille.div_ceil(PAGE_SIZE as usize);
    let mut faites = 0usize;
    let mut depuis_la_pause = 0usize;
    for index in 0..pages_du_fichier.min(budget) {
        let key = Key {
            node,
            offset: (index as u64).saturating_mul(PAGE_SIZE),
            generation,
        };
        // Prendre PUIS rendre : la page reste dans le cache et redevient
        // recuperable. On ne retient rien, on rechauffe.
        if crate::kernel::clean_page_cache::acquire(key).is_some() {
            crate::kernel::clean_page_cache::release(key);
            faites += 1;
        } else {
            // Un echec n'est pas une panne : la memoire peut etre sous
            // pression, et le prechauffage est precisement ce qu'on abandonne
            // en premier dans ce cas.
            break;
        }
        depuis_la_pause += 1;
        if depuis_la_pause >= PAGES_PAR_TRANCHE {
            depuis_la_pause = 0;
            crate::kernel::task::sleep_ticks(1);
        }
    }
    faites
}

/// Le fil de prechauffage. Une seule passe, puis il se termine.
fn fil_prechauffage() -> ! {
    // LAISSER LE BUREAU S'INSTALLER D'ABORD.
    //
    // Les premieres secondes sont les plus chargees -- polices, composition,
    // enumeration USB -- et ce sont aussi celles ou l'utilisateur regarde
    // l'ecran. Le prechauffage sert un clic qui n'aura pas lieu avant
    // plusieurs secondes ; il peut attendre.
    crate::kernel::task::sleep_ticks(3_000);

    let debut = crate::kernel::timer::monotonic_ns();
    let mut budget = PAGES_MAX;
    for chemin in BINAIRES.iter() {
        if budget == 0 {
            break;
        }
        let faites = prechauffe_un(chemin, budget);
        if faites != 0 {
            FICHIERS_PRECHAUFFES.fetch_add(1, Ordering::Relaxed);
            PAGES_PRECHAUFFEES.fetch_add(faites, Ordering::Relaxed);
            budget = budget.saturating_sub(faites);
        }
    }
    let duree = crate::kernel::timer::monotonic_ns().saturating_sub(debut);
    DUREE_NS.store(duree, Ordering::Relaxed);
    TERMINE.store(true, Ordering::Release);

    let (pages, fichiers, _) = compteurs();
    crate::serial_println!(
        "BOUCHAUD_PRECHAUFFAGE_NAVIGATEUR fichiers={} pages={} mio={} duree_ms={}",
        fichiers,
        pages,
        (pages as u64).saturating_mul(PAGE_SIZE) / (1024 * 1024),
        duree / 1_000_000,
    );

    // Le travail est fait une fois pour toutes. Le fil ne peut pas se rendre
    // lui-meme : il dort, et ne consomme plus rien.
    loop {
        crate::kernel::task::sleep_ticks(60_000);
    }
}

/// Lance le prechauffage en tache de fond.
pub fn demarre() -> bool {
    // Priorite BASSE au sens ou elle existe : `Normale` est la plus basse des
    // deux classes. Le prechauffage ne defend aucune latence -- il en offre.
    if crate::kernel::task::spawn_noyau_priorite(
        fil_prechauffage,
        "prechauffage",
        crate::kernel::task::Priorite::Normale,
    ) {
        return true;
    }
    crate::serial_println!(
        "BOUCHAUD_PRECHAUFFAGE_REFUSE raison=tache-non-creee \
consequence=premier-lancement-du-navigateur-plus-lent"
    );
    false
}
