//! Deux horloges qui doivent se confirmer, pas s'excuser l'une l'autre.
//!
//! BOUCHAUD_ACCORD_HORLOGES_V1
//!
//! # Le defaut que ceci corrige
//!
//! La sonde de temps du demarrage rendait son verdict ainsi :
//!
//! ```text
//! (dt, dm, dt != 0 || dm != 0)
//! ```
//!
//! Un OU. Il suffisait qu'UNE des deux horloges bouge pour que la ligne
//! annonce `progress=1`. Une horloge morte passait ; deux horloges en
//! desaccord passaient aussi.
//!
//! Releve du 16 septembre 2026, QEMU Stage 2 sur un portable :
//!
//! ```text
//! BOUCHAUD_HWPROBE_TIMER progress=1 ticks_delta=23 ms_delta=14
//! BOUCHAUD_HWPROBE_CPU   ... tsc_mhz=43989
//! BOUCHAUD_SMP_TIMER_LOCAL ... lapic_hz=888654262
//! ```
//!
//! Vingt-trois tics pour quatorze millisecondes : un rapport de 1,64 entre le
//! compteur d'IRQ0 et l'horloge monotone. La meme image sur une autre machine
//! donnait `90 / 90`, soit 1,00, avec `tsc_mhz=2096` et `lapic_hz=62433112`.
//!
//! La consequence n'est pas cosmetique. `monotonic_ms` sert de base a
//! `BOUCHAUD_BOOT_POINT ... t_ms= delta_ms=`, au releve periodique, aux
//! echeances de l'ordonnanceur. Une base fausse d'un ordre de grandeur rend
//! faux tout ce qui s'y adosse -- et la ligne censee l'annoncer disait vert.
//!
//! # Ce que « d'accord » veut dire
//!
//! Les deux compteurs mesurent la meme duree par deux chemins independants :
//! le compteur de tics est incremente par IRQ0 (mille par seconde, donc un
//! tic vaut une milliseconde), l'horloge monotone descend du HPET ou du TSC.
//! Sur une fenetre d'attente quelconque, leurs deltas doivent etre du meme
//! ordre.
//!
//! La tolerance est LARGE a dessein. On ne cherche pas a mesurer une derive
//! de quelques pour cent -- une fenetre d'attente par boucle vide n'a pas
//! cette precision -- mais a attraper l'ordre de grandeur : une horloge
//! arretee, une calibration qui se trompe d'un facteur dix.
//!
//! Et quand la fenetre est trop courte pour conclure, le verdict le DIT au
//! lieu de choisir. Ne pas avoir pu verifier n'est pas avoir verifie.

/// Ce que deux deltas d'horloge disent l'un de l'autre.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Accord {
    /// Aucune des deux n'a bouge : le temps ne passe pas du tout.
    AucuneNAvance = 0,
    /// L'une avance, l'autre est arretee. C'est le cas que le OU laissait
    /// passer, et le plus grave : la moitie du systeme lit une horloge morte.
    UneSeuleAvance = 1,
    /// Les deux avancent, mais pas du meme ordre. La calibration est fausse.
    Desaccord = 2,
    /// La fenetre est trop courte pour conclure.
    ///
    /// Ce n'est PAS un verdict favorable. Ne pas avoir pu verifier et avoir
    /// verifie sont deux choses differentes, et les confondre est exactement
    /// la faute que le OU commettait.
    FenetreTropCourte = 3,
    /// Les deux avancent et se confirment.
    Accord = 4,
}

impl Accord {
    /// Peut-on se fier a l'horloge ?
    pub fn fiable(self) -> bool {
        matches!(self, Accord::Accord)
    }

    pub fn nom(self) -> &'static str {
        match self {
            Accord::AucuneNAvance => "aucune-n-avance",
            Accord::UneSeuleAvance => "une-seule-avance",
            Accord::Desaccord => "desaccord",
            Accord::FenetreTropCourte => "fenetre-trop-courte",
            Accord::Accord => "accord",
        }
    }
}

/// Relit un verdict range dans un octet.
///
/// Toute valeur inconnue est traitee comme un DESACCORD : sur une base de
/// temps, le doute doit pencher du cote qui refuse de garantir.
pub fn depuis_octet(code: u8) -> Accord {
    match code {
        0 => Accord::AucuneNAvance,
        1 => Accord::UneSeuleAvance,
        3 => Accord::FenetreTropCourte,
        4 => Accord::Accord,
        _ => Accord::Desaccord,
    }
}

/// Ecart TOLERE, en pour cent du PLUS PETIT des deux compteurs.
///
/// Vingt-cinq pour cent. Ce n'est pas une mesure de derive : c'est un filet
/// contre l'ordre de grandeur. Le releve fautif du 16 septembre est a
/// vingt-trois tics pour quatorze millisecondes, soit un rapport de 1,64 ; le
/// releve sain est a 90 pour 90, soit 1,00.
pub const TOLERANCE_POURCENT: u64 = 25;

/// Duree minimale, dans le plus grand des deux compteurs, pour conclure.
///
/// Sous huit tics, un ecart d'une unite pese plus de douze pour cent : la
/// quantification a elle seule produirait des desaccords. On ne conclut pas,
/// et surtout on ne conclut pas « tout va bien ».
pub const FENETRE_MINIMALE: u64 = 8;

/// Les deux horloges se confirment-elles ?
///
/// `tics` est le nombre d'IRQ0 comptees sur la fenetre -- une par
/// milliseconde. `millisecondes` est ce que l'horloge monotone dit de la meme
/// fenetre. Les deux sont donc censes etre proches.
pub fn accord(tics: u64, millisecondes: u64) -> Accord {
    if tics == 0 && millisecondes == 0 {
        return Accord::AucuneNAvance;
    }
    if tics == 0 || millisecondes == 0 {
        return Accord::UneSeuleAvance;
    }
    let grand = tics.max(millisecondes);
    let petit = tics.min(millisecondes);
    if grand < FENETRE_MINIMALE {
        return Accord::FenetreTropCourte;
    }
    // L'arithmetique passe par `u128` : `(grand - petit) * 100` deborde des
    // que l'un des compteurs approche `u64::MAX`, et un debordement rendrait
    // ici « tout va bien » sur la pire des entrees.
    let ecart = (grand - petit) as u128;
    let seuil = petit as u128 * TOLERANCE_POURCENT as u128 / 100;
    if ecart > seuil {
        Accord::Desaccord
    } else {
        Accord::Accord
    }
}
