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

/// BOUCHAUD_C72_SCENARIO_G -- CHECKPOINT OK, PUIS EXTINCTION FORCEE EN ECHEC
///
/// Seconde a laquelle le banc pose un checkpoint volontaire. Zero = jamais.
///
/// Le checkpoint periodique tourne a 45 s ; un banc court ne l'atteindrait
/// pas, et un banc long pour cette seule raison coute du temps a chaque
/// execution. Ce reglage pose le checkpoint QUAND ON VEUT, ce qui est aussi
/// ce que fait `diag-save` a la main.
pub const CHECKPOINT_S: u64 = match option_env!("BOUCHAUD_BANC_CHECKPOINT_S") {
    Some(v) => match u64::from_str_radix(v, 10) {
        Ok(n) => n,
        _ => 0,
    },
    None => 0,
};

/// Le vidage FINAL doit-il etre force en echec ?
///
/// C'est le scenario qui compte : un checkpoint recuperable, PUIS une
/// extinction incomplete. Il prouve ce que le test physique du Trigkey a
/// coute -- qu'un shutdown rate ne doit plus emporter toute la session.
///
/// Les trois pannes sont armees juste avant l'extinction, donc apres le
/// checkpoint : elles tombent sur le vidage final, exactement ou on les veut.
pub const FINAL_KO: bool = match option_env!("BOUCHAUD_BANC_FINAL_KO") {
    Some(v) => !matches!(v.as_bytes(), b"0" | b"non" | b"off"),
    None => false,
};

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

/// BOUCHAUD_P0_REVEIL_CIBLE_V1 -- LA CHARGE QUI MANQUAIT
///
/// Nombre de fils NOYAU de calcul qui ne rendent jamais la main.
///
/// # Pourquoi ce banc n'a jamais vu le defaut d'ordonnancement
///
/// Le banc precedent tourne avec deux fils et seize coeurs virtuels : chaque
/// tache possede un coeur, ne se bloque jamais derriere une autre, et
/// `publish_ready` n'est appele qu'une poignee de fois en deux minutes. La
/// campagne du 17 septembre le dit en chiffres -- cinq reveils immediats pour
/// cent vingt secondes. Une machine ou personne ne se dispute un coeur ne peut
/// pas reproduire une attente de six secondes pour obtenir un coeur.
///
/// Ces fils-la fabriquent la situation physique : autant de calculs noyau que
/// de coeurs, de classe `Interactive`, qui n'appellent JAMAIS `schedule()`.
/// C'est exactement ce que le releve TRIGKEY montre autour de `usb-hid`.
///
/// Zero par defaut : le banc d'endurance mesure une duree, pas une latence, et
/// n'a rien a faire de seize fils qui tournent.
pub const FILS_CPU: usize = match option_env!("BOUCHAUD_BANC_CHARGE_CPU") {
    Some(v) => match usize::from_str_radix(v, 10) {
        Ok(n) if n <= 64 => n,
        _ => 0,
    },
    None => 0,
};

static TOURS_CPU: AtomicU64 = AtomicU64::new(0);
static FILS_CPU_VIVANTS: AtomicU64 = AtomicU64::new(0);

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
    // L'ARBITRE EST SENSIBLE A LA LATENCE, ET CE N'EST PAS UN ARTIFICE.
    //
    // Il se reveille une fois par periode pour lire une horloge et armer une
    // panne : quelques microsecondes de budget, et une exigence de reveil
    // stricte -- un arbitre qui se reveille avec six secondes de retard arme
    // ses pannes au mauvais moment et n'eteint jamais la machine.
    //
    // C'est aussi ce qui rend le banc CONCLUANT quand `FILS_CPU` occupe tous
    // les coeurs : sans cette propriete, l'arbitre subirait exactement la
    // famine qu'on mesure, et le banc ne rendrait aucun verdict du tout.
    // Deux taches sensibles en meme temps mettent de surcroit a l'epreuve la
    // tranche minimale qui les empeche de se couper l'une l'autre.
    crate::kernel::task::spawn_noyau_sensible(
        fil_arbitre,
        "banc-io-arbitre",
        crate::kernel::task::Priorite::Interactive,
    );
    for _ in 0..FILS_CPU {
        if crate::kernel::task::spawn_noyau_priorite(
            fil_cpu,
            "banc-io-cpu",
            crate::kernel::task::Priorite::Interactive,
        ) {
            FILS_CPU_VIVANTS.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Un calcul noyau qui ne rend JAMAIS la main.
///
/// Il n'appelle ni `sleep_ticks`, ni `schedule`, ni rien qui cede : c'est le
/// point. Avant ce lot, un tel fil rendait son coeur inatteignable -- aucun
/// IPI de publication (le coeur ne dort pas), aucun IPI de quantum (le masque
/// exclut les fils noyau), aucun point sur (il n'y en a pas dans un fil
/// noyau), aucun voleur (la pression ne comptait que le fond de file).
fn fil_cpu() -> ! {
    let mut graine: u64 = 0x9E37_79B9_7F4A_7C15;
    loop {
        // Un bloc de calcul, puis une lecture atomique. Pas de `sleep_ticks` :
        // ce fil doit ETRE le probleme, pas le contourner.
        for _ in 0..4096 {
            graine = graine
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            core::hint::black_box(graine);
        }
        TOURS_CPU.fetch_add(1, Ordering::Relaxed);
        if ARRET_DEMANDE.load(Ordering::Acquire) {
            crate::kernel::task::sleep_ticks(1000);
        }
    }
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
    let mut checkpoint = false;
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
        if !checkpoint && CHECKPOINT_S != 0 && ecoule >= CHECKPOINT_S * NS {
            checkpoint = true;
            let bilan = crate::kernel::blackbox::checkpoint("banc");
            crate::serial_println!(
                "BOUCHAUD_BANC_IO_CHECKPOINT ecoule_s={} ok={} seq={} records={} duree_ms={}",
                ecoule / NS, bilan.ok() as u8, bilan.seq, bilan.poses,
                bilan.duree_us / 1_000,
            );
        }
        if ecoule >= SECONDES * NS {
            // LE VIDAGE FINAL EST SABORDE ICI, ET PAS PLUS TOT.
            //
            // Arme apres le checkpoint, juste avant l'extinction : les trois
            // pannes tombent donc sur le vidage final. C'est le scenario G --
            // checkpoint recuperable, extinction incomplete.
            if FINAL_KO {
                // UNE PANNE QUI TIENT, ET PAS TROIS QUI SE RATTRAPENT.
                //
                // Premiere version : les trois injections BOT. Elles sont
                // CONSOMMEES a la premiere occasion, le chemin d'extinction
                // les rattrape par ses reprises, et le banc a rendu
                // `completude=COMPLETE` avec le sabotage pourtant arme. Il
                // avait raison de refuser : la condition n'etait pas exercee.
                //
                // Ce qu'on doit prouver n'est pas la reprise mais la SURVIE DU
                // CHECKPOINT quand le vidage final echoue vraiment. La panne
                // la plus fidele au cas physique est aussi la plus simple :
                // la cle ne repond plus.
                usb::coupe_le_support();
                crate::serial_println!(
                    "BOUCHAUD_BANC_IO_FINAL_KO arme=1 quoi=support_coupe ecoule_s={}",
                    ecoule / NS,
                );
            }
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
    crate::kernel::power::shutdown_avec_raison(0, "banc_io");
}

/// Imprime les mesures que le script relit, dans une forme stable.
fn verdict(depart: u64) {
    let ecoule = crate::kernel::timer::monotonic_ns().saturating_sub(depart);
    let (ecart_max, _) = crate::drivers::xhci_active::ecart_scrutation_hid();
    let verrou = crate::drivers::xhci_active::etat_du_verrou();
    let chrono = crate::drivers::xhci_active::chrono_hid();
    let tourniquet = crate::drivers::xhci_active::tourniquet_compteurs();
    let bot = crate::drivers::xhci_active::releve_bot();
    let tambour = crate::kernel::blackbox::tambour();
    let trames = crate::gui::frame_clock::snapshot();
    let (_, injections_consommees) = crate::drivers::xhci_active::injections();
    let reveil = crate::kernel::scheduler::preempt::stats_reveil();
    crate::serial_println!(
        "BOUCHAUD_BANC_IO_VERDICT ecoule_s={} lectures={} blocs_lus={} echecs_lecture={} \
hid_poll_gap_max_ms={} hid_poll_gap_verdict={} hid_poll_gap_cible_ms={} hid_poll_gap_defaut_ms={} \
hid_wake_to_run_max_us={} hid_wake_verdict={} hid_wake_cible_us={} hid_wake_defaut_us={} \
hid_run_to_lock_max_us={} hid_lock_starve_max_us={} hid_poll_body_max_us={} hid_responsable={} \
hid_lock_fail_total={} hid_lock_fail_streak_max={} hid_lock_fail_owner_fs={} hid_lock_fail_owner_repli={} \
tourniquet_reservations={} tourniquet_refus={} tourniquet_expirations={} \
charge_cpu_fils={} charge_cpu_tours={} \
reveil_immediats={} reveil_cibles={} reveil_differes={} reveil_ipi={} \
reveil_preempt_noyau={} reveil_refus={} reveil_deplaces={} \
runtime_owner={} runtime_max_hold_ns={} runtime_max_owner={} \
runtime_acquisitions={} runtime_contentions={} runtime_timeouts={} \
blackbox_ram_records={} blackbox_ram_overwrites={} blackbox_ram_lost={} blackbox_ram_refuses={} \
bot_etat={} bot_timeouts={} bot_recoveries={} bot_recovery_success={} bot_recovery_failure={} \
bot_refus={} bot_rejouees={} injections_consommees={} wm_heartbeat={} wm_frames={} scheduler_heartbeat={}",
        ecoule / NS,
        LECTURES.load(Ordering::Relaxed),
        BLOCS_LUS.load(Ordering::Relaxed),
        ECHECS.load(Ordering::Relaxed),
        ecart_max / 1_000_000,
        crate::drivers::xhci_active::verdict_ecart_hid(ecart_max / 1_000_000),
        crate::drivers::xhci_active::ECART_HID_CIBLE_MS,
        crate::drivers::xhci_active::ECART_HID_DEFAUT_MS,
        chrono.wake_to_run_max_us,
        crate::drivers::xhci_active::verdict_reveil_hid(chrono.wake_to_run_max_us),
        crate::drivers::xhci_active::REVEIL_HID_CIBLE_US,
        crate::drivers::xhci_active::REVEIL_HID_DEFAUT_US,
        chrono.run_to_lock_max_us,
        chrono.lock_starve_max_us, chrono.poll_body_max_us, chrono.responsable(),
        chrono.lock_fail_total, chrono.lock_fail_streak_max,
        chrono.lock_fail_owner[3], chrono.lock_fail_owner[2],
        tourniquet.0, tourniquet.1, tourniquet.2,
        FILS_CPU_VIVANTS.load(Ordering::Relaxed),
        TOURS_CPU.load(Ordering::Relaxed),
        reveil.immediats, reveil.cibles, reveil.differes, reveil.ipi_envoyes,
        reveil.preemptions_noyau, reveil.preemptions_noyau_refusees,
        reveil.placements_deplaces,
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
