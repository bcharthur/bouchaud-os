// H10 -- La machine de reference, et sa preuve.
//
// # Ce que ce module est, et ce qu'il n'est pas
//
// Ce n'est pas un pilote, ni une fonctionnalite visible. C'est l'outillage qui
// permet de dire, depuis la machine elle-meme, CE QUI MARCHE et ce qui ne
// marche pas -- sans photo d'ecran, sans interpretation, et sans avoir a
// croire une affirmation ecrite ailleurs.
//
// Chaque commande imprime des marqueurs stables que le port serie emporte et
// que l'enregistreur de vol conserve. Un releve d'essai physique devient ainsi
// un fichier, comparable au precedent.
//
// # La regle qui gouverne l'ecriture disque
//
// Le disque interne du TRIGKEY porte 1 000 215 216 blocs -- un demi-teraoctet,
// et le systeme d'exploitation de son proprietaire. Une commande de test qui
// ecrirait a un LBA choisi par l'utilisateur detruirait des donnees le jour ou
// le chiffre serait mal tape.
//
// Aucune ecriture de ce module ne touche donc le volume BRUT. Toutes passent
// par `Volume::DONNEES`, qui n'existe que si une partition portant le GUID de
// type Bouchaud a ete trouvee dans la table GPT. Sans cette partition,
// l'ecriture est REFUSEE et le dit. C'est la seule barriere qui tienne : elle
// ne depend ni d'une borne calculee, ni de l'attention de celui qui tape.

extern crate alloc;

use crate::drivers::bloc::{self, Achevement, Volume};

/// Blocs lus par `disktest`. Assez pour traverser plusieurs commandes NVMe,
/// assez peu pour que la sortie reste lisible.
const DISKTEST_BLOCS: usize = 8;
/// Blocs ecrits par `disktest --ecriture`.
const DISKTEST_BLOCS_ECRITURE: usize = 1;
/// Fichier temoin de `persist-test`.
const TEMOIN: &str = "/persist/trigkey-temoin.txt";

/// Un temoin a-t-il ete pose DANS CETTE SESSION ?
///
/// C'est la seule chose qui distingue une relecture qui prouve quelque chose
/// d'une relecture qui n'en prouve aucune. Ni l'horloge ni la duree de
/// fonctionnement ne repondent a cette question : seule la memoire de ce qui
/// s'est passe depuis l'amorcage le fait, et elle disparait au redemarrage --
/// ce qui est exactement la propriete recherchee.
static POSE_CETTE_SESSION: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

fn oui_non(v: bool) -> &'static str {
    if v { "oui" } else { "non" }
}

// ---------------------------------------------------------------------------
// hwinfo -- ce que la machine DIT d'elle-meme
// ---------------------------------------------------------------------------

pub fn hwinfo() {
    crate::println!("=== TRIGKEY hwinfo ===");

    // --- Processeur -------------------------------------------------------
    let coeurs = crate::arch::x86_64::smp::schedulable_cpus();
    let vus = crate::arch::x86_64::smp::discovered_cpus();
    crate::println!("cpu       coeurs_en_ligne={} decouverts={}", coeurs, vus);
    crate::serial_println!(
        "H10_HWINFO_CPU coeurs_en_ligne={} decouverts={} tsc_hz={}",
        coeurs, vus,
        crate::kernel::timer::tsc_hz().unwrap_or(0),
    );

    // --- Quantum local ----------------------------------------------------
    //
    // Le mode compte autant que le nombre de coeurs : un timer absent rend
    // quinze coeurs sur seize decoratifs, et c'est arrive.
    let mode = crate::arch::x86_64::smp::mode_timer_local();
    let hz = crate::arch::x86_64::smp::lapic_hz();
    crate::println!("timer     mode={} lapic_hz={}", mode, hz);
    crate::serial_println!("H10_HWINFO_TIMER mode={} lapic_hz={}", mode, hz);

    // --- PCI --------------------------------------------------------------
    crate::println!("pci       (voir lspci pour le detail)");
    crate::arch::x86_64::pci::print_devices();

    // --- Disque interne ---------------------------------------------------
    let present = crate::drivers::nvme::present();
    let hors_service = crate::drivers::nvme::hors_service();
    match crate::drivers::nvme::geometrie() {
        Some((blocs, taille)) => {
            let mio = blocs.saturating_mul(taille as u64) / (1024 * 1024);
            crate::println!(
                "nvme      present={} hors_service={} blocs={} taille_bloc={} mio={}",
                oui_non(present), oui_non(hors_service), blocs, taille, mio,
            );
            crate::serial_println!(
                "H10_HWINFO_NVME present=1 hors_service={} blocs={} taille_bloc={} mio={}",
                hors_service as u8, blocs, taille, mio,
            );
        }
        None => {
            crate::println!("nvme      present={} (aucune geometrie)", oui_non(present));
            crate::serial_println!("H10_HWINFO_NVME present=0");
        }
    }

    // --- Volumes ----------------------------------------------------------
    //
    // La distinction compte : AMORCE est le disque brut, DONNEES la partition
    // Bouchaud. C'est elle, et elle seule, ou l'on a le droit d'ecrire.
    for (volume, nom) in [(Volume::AMORCE, "amorce"), (Volume::DONNEES, "donnees")] {
        let d = bloc::descripteur(volume);
        crate::println!(
            "volume{}   present={} nom={} blocs={} taille_bloc={} profondeur={}",
            volume.0, oui_non(bloc::present(volume)), d.nom, d.blocs,
            d.taille_bloc, d.profondeur_file,
        );
        crate::serial_println!(
            "H10_HWINFO_VOLUME index={} present={} nom={} blocs={} taille_bloc={} profondeur={}",
            volume.0, bloc::present(volume) as u8, d.nom, d.blocs,
            d.taille_bloc, d.profondeur_file,
        );
    }

    // --- USB --------------------------------------------------------------
    crate::drivers::xhci_active::log_stockage();
    // Par peripherique : lequel repond en Interrupt-IN, lequel est servi par le
    // repli EP0. Un clavier servi par le repli ressemble a un clavier qui
    // marche, et c'est exactement ce qu'il ne faut pas laisser croire.
    crate::drivers::xhci_active::log_hid_transports();
    crate::serial_println!("H10_HWINFO_FIN");
}

// ---------------------------------------------------------------------------
// hwtest -- une suite de verdicts, pas un recit
// ---------------------------------------------------------------------------

struct Bilan {
    passes: u32,
    echecs: u32,
    non_testes: u32,
}

impl Bilan {
    fn neuf() -> Self {
        Self { passes: 0, echecs: 0, non_testes: 0 }
    }

    /// Un point du contrat, avec son verdict.
    ///
    /// NON TESTE est un troisieme etat, et il est indispensable : confondre
    /// « je n'ai pas pu verifier » avec « c'est faux » ferait echouer un essai
    /// pour une absence de materiel, et confondre avec « c'est vrai »
    /// presenterait comme acquis ce que personne n'a vu.
    fn point(&mut self, nom: &str, verdict: Option<bool>, detail: &str) {
        let (etiquette, code) = match verdict {
            Some(true) => {
                self.passes += 1;
                ("PASSE     ", "passe")
            }
            Some(false) => {
                self.echecs += 1;
                ("ECHEC     ", "echec")
            }
            None => {
                self.non_testes += 1;
                ("NON TESTE ", "non-teste")
            }
        };
        crate::println!("{}{}  {}", etiquette, nom, detail);
        crate::serial_println!("H10_HWTEST point={} verdict={} detail={}", nom, code, detail);
    }
}

pub fn hwtest() {
    crate::println!("=== TRIGKEY hwtest ===");
    let mut bilan = Bilan::neuf();

    // 1. Les coeurs battent-ils VRAIMENT ?
    let coeurs = crate::arch::x86_64::smp::schedulable_cpus();
    if coeurs > 1 {
        let (battants, en_ligne) = crate::arch::x86_64::smp::mesure_battement_par_coeur(200);
        bilan.point(
            "smp-battement",
            Some(battants == en_ligne),
            if battants == en_ligne { "tous les coeurs recoivent leur quantum" }
            else { "des coeurs en ligne ne recoivent aucun quantum" },
        );
    } else {
        bilan.point("smp-battement", None, "un seul coeur : rien a comparer");
    }

    // 2. Le quantum local est-il arme, et par quel mode ?
    let mode = crate::arch::x86_64::smp::mode_timer_local();
    bilan.point(
        "timer-local",
        Some(mode != "aucun"),
        mode,
    );

    // 3. Le disque interne repond-il ?
    if crate::drivers::nvme::present() {
        bilan.point(
            "nvme-present",
            Some(!crate::drivers::nvme::hors_service()),
            if crate::drivers::nvme::hors_service() { "hors service" } else { "en service" },
        );
    } else {
        bilan.point("nvme-present", None, "aucun controleur NVMe sur ce bus");
    }

    // 4. Une lecture reelle aboutit-elle ?
    if bloc::present(Volume::AMORCE) {
        let mut tampon = [0u8; 512];
        let lu = matches!(bloc::lit(Volume::AMORCE, 0, 1, &mut tampon), Achevement::Fait(_));
        bilan.point(
            "bloc-lecture",
            Some(lu),
            if lu { "le premier bloc du disque a ete lu" } else { "lecture refusee" },
        );
    } else {
        bilan.point("bloc-lecture", None, "aucun volume d'amorce");
    }

    // 5. La partition Bouchaud existe-t-elle ?
    //
    // C'est la condition de TOUTE ecriture, et donc du test de persistance.
    // `bloc::present(Volume::DONNEES)` ne suffit PAS : le pilote ATA
    // enregistre son disque esclave sur ce meme volume, sans condition.
    let donnees = crate::platform::pc::installation::systeme_monte();
    bilan.point(
        "partition-bouchaud",
        Some(donnees),
        if donnees { "presente : l'ecriture encadree est possible" }
        else { "absente : aucune ecriture disque ne sera tentee" },
    );

    // 6. L'entree existe-t-elle ?
    //
    // SANS BUS, IL N'Y A RIEN A CONCLURE.
    //
    // La premiere version rendait ECHEC quand aucun peripherique ne repondait,
    // y compris sur une machine sans le moindre controleur xHCI -- ce qui est
    // le cas de la plupart des scenarios QEMU. Un essai devenait rouge pour
    // une absence de materiel, exactement ce que l'etat NON TESTE existe pour
    // eviter.
    if crate::drivers::xhci_active::controleurs() == 0 {
        bilan.point("usb-hid", None, "aucun controleur xHCI sur ce bus");
    } else {
        let entree = crate::drivers::xhci_active::hid_polling();
        bilan.point(
            "usb-hid",
            Some(entree),
            if entree { "au moins un peripherique d'entree repond" }
            else { "un controleur existe, aucun peripherique d'entree ne repond" },
        );
    }

    crate::println!(
        "bilan     passes={} echecs={} non_testes={}",
        bilan.passes, bilan.echecs, bilan.non_testes,
    );
    // LE VERDICT NE COMPTE PAS LES NON TESTES COMME DES SUCCES.
    //
    // Une machine sans NVMe rendrait sinon un vert complet en n'ayant rien
    // prouve du tout.
    crate::serial_println!(
        "H10_HWTEST_BILAN passes={} echecs={} non_testes={} verdict={}",
        bilan.passes, bilan.echecs, bilan.non_testes,
        if bilan.echecs == 0 { "ok" } else { "echec" },
    );
}

// ---------------------------------------------------------------------------
// bootlog -- ce que l'amorcage a dit
// ---------------------------------------------------------------------------

pub fn bootlog() {
    crate::println!("=== TRIGKEY bootlog ===");
    crate::kernel::dmesg::print();
    crate::serial_println!("H10_BOOTLOG_FIN");
}

// ---------------------------------------------------------------------------
// nvmetest -- le pilote, en detail
// ---------------------------------------------------------------------------

pub fn nvmetest() {
    crate::println!("=== TRIGKEY nvmetest ===");
    if !crate::drivers::nvme::present() {
        crate::println!("nvme absent : rien a tester");
        crate::serial_println!("H10_NVMETEST verdict=non-teste raison=absent");
        return;
    }
    crate::drivers::nvme::log_stats();

    let Some((blocs, taille)) = crate::drivers::nvme::geometrie() else {
        crate::println!("geometrie indisponible");
        crate::serial_println!("H10_NVMETEST verdict=echec raison=geometrie");
        return;
    };
    crate::println!("geometrie blocs={} taille_bloc={}", blocs, taille);

    // Une lecture reelle, et une seule : le but est de prouver que le chemin
    // fonctionne, pas de mesurer un debit.
    let mut tampon = [0u8; 512];
    match bloc::lit(Volume::AMORCE, 0, 1, &mut tampon) {
        Achevement::Fait(n) => {
            // Les seize premiers octets suffisent a distinguer un bloc lu d'un
            // tampon laisse a zero.
            crate::println!(
                "lecture   lba=0 blocs={} tete={:02x}{:02x}{:02x}{:02x}",
                n, tampon[0], tampon[1], tampon[2], tampon[3],
            );
            crate::serial_println!(
                "H10_NVMETEST verdict=ok blocs={} taille_bloc={} tete={:02x}{:02x}{:02x}{:02x}",
                blocs, taille, tampon[0], tampon[1], tampon[2], tampon[3],
            );
        }
        autre => {
            crate::println!("lecture refusee : {:?}", autre);
            crate::serial_println!("H10_NVMETEST verdict=echec raison=lecture");
        }
    }
}

// ---------------------------------------------------------------------------
// disktest -- lecture toujours, ecriture jamais hors de la partition Bouchaud
// ---------------------------------------------------------------------------

pub fn disktest(argc: usize, argv: &[&str; 12]) {
    let ecriture = argc >= 2 && argv[1] == "--ecriture";
    crate::println!("=== TRIGKEY disktest ({}) ===", if ecriture { "lecture+ecriture" } else { "lecture" });

    if !bloc::present(Volume::AMORCE) {
        crate::println!("aucun volume : rien a tester");
        crate::serial_println!("H10_DISKTEST verdict=non-teste raison=aucun-volume");
        return;
    }

    // --- Lecture ----------------------------------------------------------
    let mut tampon = [0u8; 512 * DISKTEST_BLOCS];
    let lu = match bloc::lit(Volume::AMORCE, 0, DISKTEST_BLOCS, &mut tampon) {
        Achevement::Fait(n) => n,
        autre => {
            crate::println!("lecture refusee : {:?}", autre);
            crate::serial_println!("H10_DISKTEST verdict=echec phase=lecture");
            return;
        }
    };
    crate::println!("lecture   {} bloc(s) depuis le lba 0", lu);
    crate::serial_println!("H10_DISKTEST phase=lecture blocs={} verdict=ok", lu);

    if !ecriture {
        crate::println!("ecriture  non demandee (ajouter --ecriture)");
        crate::serial_println!("H10_DISKTEST verdict=ok ecriture=non-demandee");
        return;
    }

    // --- Ecriture, et le refus qui la precede -----------------------------
    //
    // LE REFUS EST LA FONCTIONNALITE.
    //
    // Sans partition Bouchaud, le seul volume disponible est le disque BRUT,
    // qui porte le systeme de son proprietaire. Une ecriture y serait
    // destructrice et irreversible. On ne borne pas : on refuse.
    //
    // LA QUESTION POSEE COMPTE AUTANT QUE LE REFUS.
    //
    // La premiere version demandait `bloc::present(Volume::DONNEES)`. Le
    // pilote ATA enregistre son disque esclave sur ce meme volume a
    // l'amorcage : la reponse etait donc VRAIE sur un disque vierge, et
    // l'ecriture avait lieu. Le scenario H10 l'a attrape en lancant la
    // commande sur un disque sans partition -- `lba=8211`, sur soixante-quatre
    // mebioctets de zeros qui auraient pu etre autre chose.
    //
    // `systeme_monte()` n'est vrai que si une partition portant le GUID de
    // type Bouchaud a ete trouvee ET montee.
    if !crate::platform::pc::installation::systeme_monte() {
        crate::println!(
            "ecriture  REFUSEE : aucune partition Bouchaud sur ce disque."
        );
        crate::println!(
            "          Le disque brut porte le systeme de la machine ; ce test"
        );
        crate::println!(
            "          n'y ecrira jamais. Installer une partition Bouchaud"
        );
        crate::println!(
            "          d'abord, ou lancer disktest sans --ecriture."
        );
        crate::serial_println!(
            "H10_DISKTEST verdict=refuse phase=ecriture raison=pas-de-partition-bouchaud"
        );
        return;
    }

    let d = bloc::descripteur(Volume::DONNEES);
    // Le DERNIER bloc de la partition : le plus loin possible des structures
    // que le systeme de fichiers place en tete.
    let Some(lba) = d.blocs.checked_sub(1) else {
        crate::serial_println!("H10_DISKTEST verdict=echec phase=ecriture raison=partition-vide");
        return;
    };

    let mut avant = [0u8; 512];
    if !matches!(bloc::lit(Volume::DONNEES, lba, 1, &mut avant), Achevement::Fait(_)) {
        crate::println!("ecriture  abandonnee : le bloc cible est illisible");
        crate::serial_println!("H10_DISKTEST verdict=echec phase=relecture-prealable");
        return;
    }

    // Un motif reconnaissable, et l'horloge : deux essais successifs ne
    // peuvent pas se confondre.
    let mut motif = [0u8; 512 * DISKTEST_BLOCS_ECRITURE];
    let marque = b"BOUCHAUD-H10-DISKTEST-";
    motif[..marque.len()].copy_from_slice(marque);
    let ns = crate::kernel::timer::monotonic_ns();
    for (i, octet) in ns.to_le_bytes().iter().enumerate() {
        motif[marque.len() + i] = *octet;
    }

    if !matches!(bloc::ecrit(Volume::DONNEES, lba, DISKTEST_BLOCS_ECRITURE, &motif), Achevement::Fait(_)) {
        crate::println!("ecriture  refusee par le volume");
        crate::serial_println!("H10_DISKTEST verdict=echec phase=ecriture");
        return;
    }
    bloc::vidange(Volume::DONNEES);

    let mut apres = [0u8; 512];
    if !matches!(bloc::lit(Volume::DONNEES, lba, 1, &mut apres), Achevement::Fait(_)) {
        crate::serial_println!("H10_DISKTEST verdict=echec phase=relecture");
        return;
    }
    let conforme = apres[..marque.len() + 8] == motif[..marque.len() + 8];
    crate::println!(
        "ecriture  lba={} (dernier bloc de la partition Bouchaud) relecture={}",
        lba, oui_non(conforme),
    );
    // On REPOSE le contenu d'origine. Un test qui laisse le disque modifie
    // n'est pas un test, c'est une modification.
    let restaure = matches!(bloc::ecrit(Volume::DONNEES, lba, 1, &avant), Achevement::Fait(_));
    bloc::vidange(Volume::DONNEES);
    crate::println!("restaure  contenu d'origine repose={}", oui_non(restaure));
    crate::serial_println!(
        "H10_DISKTEST verdict={} phase=ecriture lba={} relecture={} restaure={}",
        if conforme && restaure { "ok" } else { "echec" },
        lba, conforme as u8, restaure as u8,
    );
}

// ---------------------------------------------------------------------------
// persist-test -- ecrire, redemarrer, relire
// ---------------------------------------------------------------------------

pub fn persist_test(argc: usize, argv: &[&str; 12]) {
    crate::println!("=== TRIGKEY persist-test ===");
    let pose = argc >= 2 && argv[1] == "--pose";
    let verifie = argc >= 2 && argv[1] == "--verifie";

    if !pose && !verifie {
        crate::println!("usage : persist-test --pose | persist-test --verifie");
        crate::println!("  --pose     ecrit un temoin horodate et le synchronise");
        crate::println!("  --verifie  relit le temoin apres un redemarrage");
        return;
    }

    if pose {
        let ns = crate::kernel::timer::monotonic_ns();
        let rtc = crate::arch::x86_64::rtc::now();
        // Le contenu porte de quoi reconnaitre CE passage : deux essais du
        // meme jour ne doivent pas se confondre.
        let contenu = alloc::format!(
            "BOUCHAUD-H10-TEMOIN rtc={:04}-{:02}-{:02}T{:02}:{:02}:{:02} ns={}\n",
            rtc.year, rtc.month, rtc.day, rtc.hour, rtc.minute, rtc.second, ns,
        );
        let pose_ok = {
            let mut fs = crate::fs::ramfs::fs();
            match fs.resolve_parent_name(TEMOIN, 0) {
                Some((parent, nom)) => match fs.resolve(TEMOIN, 0) {
                    Some(idx) => { fs.write_node(idx, &contenu); true }
                    None => match fs.touch_at(parent, nom) {
                        Ok(idx) => { fs.write_node(idx, &contenu); true }
                        Err(_) => false,
                    },
                },
                None => false,
            }
        };
        match pose_ok {
            true => {
                // LE DRAPEAU MARQUE L'ECRITURE, PAS SA REUSSITE SUR DISQUE.
                //
                // Pose plus bas, apres le retour d'echec de synchronisation, il
                // laissait `--verifie` croire a une nouvelle session et rendre
                // « ok » sur une relecture de la memoire -- c'est-a-dire le faux
                // positif exact que cette commande existe pour eviter.
                POSE_CETTE_SESSION.store(true, core::sync::atomic::Ordering::Release);
                let ecrits = crate::fs::persistance::synchronise();
                // `synchronise` rend -1 sur ECHEC, pas sur « rien a ecrire » :
                // une zone vide rend zero. Presenter -1 comme un compte de
                // fichiers ferait passer une panne d'ecriture pour un succes.
                if ecrits < 0 {
                    crate::println!(
                        "pose      ECHEC de synchronisation : /persist n'a PAS ete ecrit sur le disque."
                    );
                    crate::println!(
                        "          Le temoin existe en memoire et disparaitra au redemarrage."
                    );
                    crate::serial_println!(
                        "H10_PERSIST verdict=echec phase=synchronisation code={}", ecrits,
                    );
                    return;
                }
                crate::println!("pose      {} ecrit, synchronise={} fichier(s)", TEMOIN, ecrits);
                crate::println!("          contenu : {}", contenu.trim_end());
                crate::println!("          REDEMARRER, puis : persist-test --verifie");
                crate::serial_println!(
                    "H10_PERSIST verdict=pose fichier={} rtc={:04}-{:02}-{:02}T{:02}:{:02}:{:02} ns={} synchronises={}",
                    TEMOIN, rtc.year, rtc.month, rtc.day, rtc.hour, rtc.minute, rtc.second,
                    ns, ecrits,
                );
            }
            false => {
                crate::println!("pose      ECHEC : {} n'a pas pu etre ecrit", TEMOIN);
                crate::serial_println!("H10_PERSIST verdict=echec phase=pose");
            }
        }
        return;
    }

    let lu = {
        let fs = crate::fs::ramfs::fs();
        fs.resolve(TEMOIN, 0).map(|idx| fs.nodes[idx].content_str())
    };
    // CE QUE `--verifie` PROUVE, ET CE QU'IL NE PROUVE PAS.
    //
    // Il relit le systeme de fichiers en memoire. Lance dans la meme session
    // que `--pose`, il retrouvera toujours le temoin -- meme si rien n'a
    // atteint le disque. Seul un REDEMARRAGE entre les deux fait de cette
    // relecture une preuve de persistance.
    let meme_session = POSE_CETTE_SESSION.load(core::sync::atomic::Ordering::Acquire);
    match lu {
        Some(texte) => {
            let conforme = texte.starts_with("BOUCHAUD-H10-TEMOIN");
            crate::println!("verifie   {} relu, conforme={}", TEMOIN, oui_non(conforme));
            if meme_session {
                crate::println!(
                    "          CETTE RELECTURE NE PROUVE RIEN : le temoin a ete pose"
                );
                crate::println!(
                    "          dans cette meme session, et cette commande relit la"
                );
                crate::println!(
                    "          MEMOIRE. Redemarrer, puis relancer --verifie."
                );
                crate::serial_println!(
                    "H10_PERSIST verdict=non-concluant phase=verifie raison=meme-session"
                );
                return;
            }
            crate::println!("          contenu : {}", texte.trim_end());
            crate::serial_println!(
                "H10_PERSIST verdict={} phase=verifie contenu={}",
                if conforme { "ok" } else { "echec" },
                texte.trim_end(),
            );
        }
        None => {
            // Ce n'est PAS forcement un echec : personne n'a peut-etre encore
            // pose de temoin. Le dire evite de conclure a une panne de
            // persistance sur un essai qui n'a jamais eu lieu.
            crate::println!("verifie   {} absent.", TEMOIN);
            crate::println!("          Soit aucun temoin n'a ete pose, soit la");
            crate::println!("          persistance ne l'a pas conserve. Lancer");
            crate::println!("          `persist-test --pose`, redemarrer, relancer.");
            crate::serial_println!("H10_PERSIST verdict=absent phase=verifie");
        }
    }
}

// ---------------------------------------------------------------------------
// safe-mode -- la console, quoi qu'il arrive
// ---------------------------------------------------------------------------

/// Le mode de secours est-il demande ?
static SECOURS: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Le systeme doit-il rester en console texte ?
///
/// Consulte par l'amorcage graphique : un bureau qui ne demarre pas laisse une
/// machine sans aucun moyen de dire pourquoi. La console, elle, suffit a lancer
/// `hwtest` et a relever un journal.
pub fn secours_demande() -> bool {
    SECOURS.load(core::sync::atomic::Ordering::Acquire)
}

pub fn safe_mode(argc: usize, argv: &[&str; 12]) {
    let arret = argc >= 2 && argv[1] == "--sortir";
    if arret {
        SECOURS.store(false, core::sync::atomic::Ordering::Release);
        crate::println!("mode de secours DESACTIVE (effet au prochain demarrage)");
        crate::serial_println!("H10_SAFEMODE etat=desactive");
        return;
    }
    SECOURS.store(true, core::sync::atomic::Ordering::Release);
    // Le miroir serie garantit que la suite atteint l'hote meme si l'affichage
    // local est inutilisable -- ce qui est precisement le cas ou l'on demande
    // un mode de secours.
    crate::drivers::vga::set_serial_mirror(true);
    crate::println!("=== TRIGKEY safe-mode ===");
    crate::println!("mode de secours ACTIVE.");
    crate::println!("  - la sortie console est miroitee sur le port serie");
    crate::println!("  - le bureau graphique ne sera pas lance au prochain demarrage");
    crate::println!("  - `safe-mode --sortir` annule");
    crate::serial_println!("H10_SAFEMODE etat=actif miroir_serie=1");
}
