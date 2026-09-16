//! Le banc de charge d'entree-sortie, pour la validation physique.
//!
//! # Pourquoi ce banc existe
//!
//! Les trois archives physiques s'arretent toutes a l'instant precis ou le
//! navigateur commence a lire ses quatre cents mebioctets sur la cle
//! d'amorcage. Aucune campagne ne pouvait le voir : sous QEMU, le chemin
//! d'ecriture de l'enregistreur sortait immediatement faute de partition
//! cible, et personne ne lisait le volume USB pendant ce temps. Le defaut
//! n'existait que sur la machine, ce qui est la pire facon d'exister.
//!
//! Ce banc reproduit ce qui manquait :
//!
//!   * une LECTURE SOUTENUE du volume USB, par le meme chemin bloc que le
//!     chargeur du navigateur -- c'est ce trafic-la qui tenait le verrou du
//!     pilote en continu ;
//!   * la scrutation HID qui tourne en meme temps, et dont on mesure le PIRE
//!     ecart entre deux tours servis ;
//!   * des pannes ARTIFICIELLES armees en cours de route -- une echeance de
//!     donnees, une echeance de statut, un verrou deja tenu --, parce que ces
//!     trois-la ne se fabriquent pas sur commande avec une vraie cle et sont
//!     pourtant exactement ce qu'il faut prouver ;
//!   * une extinction PROPRE a la fin, qui est le seul moment ou
//!     l'enregistreur touche desormais le support.
//!
//! # Pourquoi derriere un drapeau de compilation
//!
//! Il lit le volume en boucle et eteint la machine. Ni l'un ni l'autre n'a sa
//! place dans une image livree, et un reglage a l'execution laisserait la
//! possibilite de l'armer par accident. Le drapeau rend la chose impossible :
//! l'image physique ne contient pas ce code.

use core::sync::atomic::{AtomicU64, Ordering};

/// Duree du banc, en secondes. Le script la pose a la compilation.
pub const SECONDES: u64 = match option_env!("BOUCHAUD_BANC_SECONDES") {
    Some(v) => match u64::from_str_radix(v, 10) {
        Ok(n) if n >= 10 => n,
        _ => 120,
    },
    None => 120,
};

const NS: u64 = 1_000_000_000;

/// Quand chaque panne artificielle est armee, en secondes depuis le depart.
///
/// Elles sont ESPACEES, et c'est le point : une reprise doit avoir eu le temps
/// d'aboutir avant la panne suivante, sinon le banc ne mesurerait que le
/// cumul de trois pannes simultanees -- un cas qui n'arrive pas, et dont la
/// reussite ne prouverait pas celle des trois cas qui arrivent.
const INJECTION_DONNEES_S: u64 = 30;
const INJECTION_STATUT_S: u64 = 45;
const INJECTION_VERROU_S: u64 = 60;

/// Les pannes artificielles sont-elles armees ?
///
/// Le banc d'endurance mesure une DUREE, pas une reprise : il tourne sans
/// volume de charge, donc sans trafic pour consommer les pannes, et celles-ci
/// tomberaient sur le vidage final -- ou elles mesureraient la reprise au lieu
/// de l'endurance. Deux bancs, deux questions, un seul module.
pub const INJECTIONS_ARMEES: bool = match option_env!("BOUCHAUD_BANC_INJECTIONS") {
    Some(v) => !matches!(v.as_bytes(), b"0" | b"non" | b"off"),
    None => true,
};

static BLOCS_LUS: AtomicU64 = AtomicU64::new(0);
static LECTURES: AtomicU64 = AtomicU64::new(0);
static ECHECS: AtomicU64 = AtomicU64::new(0);
/// Le fil de charge doit-il s'arreter ?
static ARRET_DEMANDE: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);
/// Le fil de charge a-t-il vraiment rendu la main ?
static CHARGE_ARRETEE: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// Lance le banc : un fil de charge, un fil d'arbitrage.
pub fn demarre() {
    crate::serial_println!(
        "BOUCHAUD_BANC_IO_DEPART secondes={} injections={} injections_s={},{},{}",
        SECONDES, INJECTIONS_ARMEES as u8,
        INJECTION_DONNEES_S, INJECTION_STATUT_S, INJECTION_VERROU_S,
    );
    crate::kernel::task::spawn_noyau(fil_charge, "banc-io-charge");
    crate::kernel::task::spawn_noyau(fil_arbitre, "banc-io-arbitre");
}

/// Lit le volume USB en boucle, comme le chargeur du navigateur.
fn fil_charge() -> ! {
    use crate::drivers::bloc::{self, Achevement, Volume};
    // Trente-deux kibioctets par requete : l'ordre de grandeur d'une lecture
    // de binaire par le chargeur, et assez pour que chaque requete soit
    // decoupee en plusieurs commandes BOT par le pilote.
    const BLOCS: usize = 64;
    let volume = Volume(3);
    let mut tampon = [0u8; BLOCS * 512];
    let mut lba = 0u64;
    loop {
        if ARRET_DEMANDE.load(Ordering::Acquire) {
            CHARGE_ARRETEE.store(true, Ordering::Release);
            crate::kernel::task::sleep_ticks(1000);
            continue;
        }
        let total = bloc::descripteur(volume).blocs;
        if total <= BLOCS as u64 {
            crate::kernel::task::sleep_ticks(100);
            continue;
        }
        match bloc::lit(volume, lba, BLOCS, &mut tampon) {
            Achevement::Fait(n) => {
                LECTURES.fetch_add(1, Ordering::Relaxed);
                BLOCS_LUS.fetch_add(n as u64, Ordering::Relaxed);
            }
            _ => {
                ECHECS.fetch_add(1, Ordering::Relaxed);
            }
        }
        lba = (lba + BLOCS as u64) % (total - BLOCS as u64);
        // UN TICK ENTRE DEUX LECTURES, ET PAS ZERO.
        //
        // Sans lui, ce fil monopoliserait son coeur et le banc mesurerait sa
        // propre boucle plutot que le pilote. Un tick laisse passer la
        // scrutation HID et l'enregistreur -- ce que ferait un chargeur reel,
        // qui attend aussi son analyseur.
        crate::kernel::task::sleep_ticks(1);
    }
}

/// Arme les pannes, puis eteint proprement.
fn fil_arbitre() -> ! {
    use crate::drivers::xhci_active as usb;
    let depart = crate::kernel::timer::monotonic_ns();
    let mut donnees = false;
    let mut statut = false;
    let mut verrou = false;
    loop {
        let ecoule = crate::kernel::timer::monotonic_ns().saturating_sub(depart);
        if !donnees && INJECTIONS_ARMEES && ecoule >= INJECTION_DONNEES_S * NS {
            donnees = true;
            usb::arme_injection(usb::INJECTE_ECHEANCE_DONNEES);
            crate::serial_println!("BOUCHAUD_BANC_IO_INJECTION quoi=donnees ecoule_s={}", ecoule / NS);
        }
        if !statut && INJECTIONS_ARMEES && ecoule >= INJECTION_STATUT_S * NS {
            statut = true;
            usb::arme_injection(usb::INJECTE_ECHEANCE_STATUT);
            crate::serial_println!("BOUCHAUD_BANC_IO_INJECTION quoi=statut ecoule_s={}", ecoule / NS);
        }
        if !verrou && INJECTIONS_ARMEES && ecoule >= INJECTION_VERROU_S * NS {
            verrou = true;
            usb::arme_injection(usb::INJECTE_VERROU_TENU);
            crate::serial_println!("BOUCHAUD_BANC_IO_INJECTION quoi=verrou ecoule_s={}", ecoule / NS);
        }
        if ecoule >= SECONDES * NS {
            break;
        }
        crate::kernel::task::sleep_ticks(200);
    }
    // LA CHARGE S'ARRETE AVANT LE VERDICT, ET C'EST CE QUI REND LE CRITERE C
    // MESURABLE.
    //
    // « Le verrou revient toujours a `aucun` » veut dire : aucune fuite. Lire
    // le proprietaire pendant qu'un lecteur legitime tient le verrou mesure
    // autre chose -- qu'il y avait du trafic --, et le banc echouait sur ce
    // malentendu alors que `runtime_timeouts=0` disait deja qu'il n'y avait
    // aucune fuite.
    //
    // On arrete la charge, on attend qu'elle ait vraiment rendu la main, puis
    // on regarde. Un verrou encore tenu APRES la quiescence est une fuite, et
    // c'est la seule chose que ce critere doit attraper.
    ARRET_DEMANDE.store(true, Ordering::Release);
    let limite = crate::kernel::timer::monotonic_ns().saturating_add(5 * NS);
    while crate::kernel::timer::monotonic_ns() < limite {
        if CHARGE_ARRETEE.load(Ordering::Acquire)
            && crate::drivers::xhci_active::etat_du_verrou().libre()
            && crate::drivers::xhci_active::repare_le_transport_bot()
        {
            break;
        }
        crate::kernel::task::sleep_ticks(20);
    }
    verdict(depart);
    // L'EXTINCTION EST PROPRE, ET C'EST TOUT L'OBJET DU BANC.
    //
    // Le vidage de l'enregistreur n'a plus lieu qu'ici. Couper au bouton
    // laisserait le tambour intact en RAM -- et invalide, puisque la RAM
    // s'efface -- ce qui ne prouverait rien du chemin qu'on veut mesurer.
    crate::kernel::power::shutdown(0);
}

/// Imprime les mesures que le script relit, dans une forme stable.
fn verdict(depart: u64) {
    let ecoule = crate::kernel::timer::monotonic_ns().saturating_sub(depart);
    let (ecart_max, _) = crate::drivers::xhci_active::ecart_scrutation_hid();
    let verrou = crate::drivers::xhci_active::etat_du_verrou();
    let bot = crate::drivers::xhci_active::releve_bot();
    let tambour = crate::kernel::blackbox::tambour();
    let trames = crate::gui::frame_clock::snapshot();
    let (_, injections_consommees) = crate::drivers::xhci_active::injections();
    crate::serial_println!(
        "BOUCHAUD_BANC_IO_VERDICT ecoule_s={} lectures={} blocs_lus={} echecs_lecture={} \
hid_poll_gap_max_ms={} runtime_owner={} runtime_max_hold_ns={} runtime_max_owner={} \
runtime_acquisitions={} runtime_contentions={} runtime_timeouts={} \
blackbox_ram_records={} blackbox_ram_overwrites={} blackbox_ram_lost={} blackbox_ram_refuses={} \
bot_etat={} bot_timeouts={} bot_recoveries={} bot_recovery_success={} bot_recovery_failure={} \
bot_refus={} bot_rejouees={} injections_consommees={} wm_heartbeat={} wm_frames={} scheduler_heartbeat={}",
        ecoule / NS,
        LECTURES.load(Ordering::Relaxed),
        BLOCS_LUS.load(Ordering::Relaxed),
        ECHECS.load(Ordering::Relaxed),
        ecart_max / 1_000_000,
        verrou.proprietaire.nom(),
        verrou.tenue_max_ns,
        verrou.tenue_max_proprietaire.nom(),
        verrou.prises,
        verrou.contentions,
        verrou.expirations,
        tambour.poses,
        tambour.ecrases,
        tambour.perdus,
        tambour.refuses,
        bot.etat.nom(),
        bot.echeances,
        bot.reprises,
        bot.reprises_reussies,
        bot.reprises_echouees,
        bot.refus,
        crate::drivers::xhci_active::bot_reprises_utiles(),
        injections_consommees,
        crate::gui::reveil::tours(),
        trames.frames_useful,
        crate::kernel::task::quantums_recus(0),
    );
}
