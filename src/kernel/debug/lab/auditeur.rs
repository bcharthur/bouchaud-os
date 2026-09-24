//! L'auditeur : la regle qui dit « ceci est une panne », et rien d'autre.
//!
//! # Pourquoi ce module ne touche a aucun materiel
//!
//! Le releve TRIGKEY de la campagne `1179cdd` est une panne parfaitement
//! reguliere :
//!
//! ```text
//! tour 1 :  rx_packets 0 -> 64,  rx_cur 0 -> 63 -> 0
//! ensuite : rx_packets=64  rx_cur=0  desc_nic=64  desc_cpu=0   (151 s)
//!           isr_rx_ok croissant   rx_missed=0   chip_cmd=RX_ENB|TX_ENB
//! ```
//!
//! Aucun bit d'erreur, aucune trame manquee, le moteur arme, et la carte qui
//! continue d'annoncer des trames qu'elle n'ecrit plus. Rien, dans cet etat,
//! ne se declare tout seul en panne : c'est une CONJONCTION de compteurs qui
//! le dit, et une conjonction se prouve par arithmetique.
//!
//! D'ou ce module : des structures, des seuils, et des verdicts. Pas un accces
//! MMIO, pas une horloge propre, pas un etat global. `auditd` lui apporte ce
//! qu'il a lu ; il rend ce qu'il en conclut. Toute la regle se contredit en
//! test hote, sans demarrer la machine.
//!
//! # Les quatre verdicts, et ce qui les separe
//!
//! | verdict                        | ce qu'il affirme                        |
//! |--------------------------------|-----------------------------------------|
//! | `RxDmaStall`                   | la carte interrompt et n'ecrit plus     |
//! | `SecondTourAbsent`             | l'anneau a boucle et ne circule pas     |
//! | `AnneauInvariant`              | materiel et pilote ne parlent plus du meme objet |
//! | `BlackboxPersistenceStall`     | la RAM avance, le support est fige      |
//!
//! Les deux premiers se ressemblent et ne se confondent pas. `RxDmaStall` est
//! une observation de compteurs, valable a tout moment ; `SecondTourAbsent`
//! est propre au releve TRIGKEY -- un premier tour parfait suivi de rien --
//! et c'est celui qui nomme la panne physique.

/// Ce que l'auditeur regarde, a un instant.
///
/// Un champ qu'une plateforme ne sait pas remplir reste a zero. Les regles ne
/// comparent que des ECARTS et des conjonctions : un zero constant ne peut
/// declencher aucun verdict, et c'est voulu.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Observation {
    pub t_ns: u64,
    pub lien: bool,

    // --- l'anneau de reception --------------------------------------------
    pub rx_paquets: u64,
    pub rx_cur: u32,
    pub rx_tours_cpu: u64,
    pub rx_rendus_tour1: u64,
    pub rx_rendus_tour2: u64,
    pub rx_rearmes_tour1: u64,
    pub rx_reutilises_tour2: u64,
    pub desc_materiel: u32,
    pub desc_processeur: u32,
    pub rx_own_rendus: u64,

    // --- ce que la carte annonce ------------------------------------------
    pub isr_rx_ok: u64,
    pub isr_rx_err: u64,
    pub isr_rx_overflow: u64,
    pub isr_rx_fifo_over: u64,
    pub isr_system_error: u64,
    pub rx_missed: u32,
    pub chip_cmd: u8,

    /// L'invariant de l'anneau, casse ou non. Voir `anneau_rx::invariant_casse`.
    pub invariant_casse: bool,
    pub invariant_code: u32,

    // --- la boite noire ---------------------------------------------------
    pub bb_storage_ready: bool,
    pub bb_records_ram: u64,
    pub bb_records_persistes: u64,
}

/// Ce que l'auditeur conclut.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// La carte interrompt, et plus un descripteur ne revient au processeur.
    RxDmaStall {
        silence_ns: u64,
        isr_delta: u64,
        desc_processeur: u32,
    },
    /// L'anneau a boucle `63 -> 0` et le second tour n'a jamais commence.
    SecondTourAbsent {
        rendus_tour1: u64,
        rendus_tour2: u64,
        attente_ns: u64,
    },
    /// Materiel et pilote ne parlent plus du meme objet.
    AnneauInvariant { code: u32, rx_cur: u32 },
    /// La RAM avance, le support est fige.
    BlackboxPersistenceStall {
        records_ram: u64,
        records_persistes: u64,
        fige_depuis_ns: u64,
    },
    /// `RxEnb` est tombe. Le moteur n'ecrira plus rien et rien ne le relancera.
    MoteurRxArrete { chip_cmd: u8 },
}

impl Verdict {
    /// Un verdict merite-t-il une capture materielle complete ?
    ///
    /// Seuls ceux qui accusent la reception : capturer soixante-dix
    /// evenements de registres RTL8168 pour une panne de boite noire
    /// noierait l'anneau sans rien apprendre.
    pub fn demande_capture(&self) -> bool {
        matches!(
            self,
            Verdict::RxDmaStall { .. }
                | Verdict::SecondTourAbsent { .. }
                | Verdict::AnneauInvariant { .. }
                | Verdict::MoteurRxArrete { .. }
        )
    }
}

/// Au plus autant de verdicts par tour. Il n'y a que cinq regles.
pub const VERDICTS_MAX: usize = 5;

/// Ce qu'un tour d'audit produit.
#[derive(Clone, Copy, Debug)]
pub struct Rapport {
    verdicts: [Option<Verdict>; VERDICTS_MAX],
    n: usize,
    /// Cadence a tenir jusqu'au prochain tour, en hertz.
    pub cadence_hz: u32,
    /// La cadence vient-elle de changer ? Pour ne tracer que les transitions.
    pub cadence_changee: bool,
}

impl Rapport {
    fn vide(cadence_hz: u32) -> Self {
        Self { verdicts: [None; VERDICTS_MAX], n: 0, cadence_hz, cadence_changee: false }
    }

    fn pousse(&mut self, v: Verdict) {
        if self.n < VERDICTS_MAX {
            self.verdicts[self.n] = Some(v);
            self.n += 1;
        }
    }

    pub fn verdicts(&self) -> impl Iterator<Item = Verdict> + '_ {
        self.verdicts[..self.n].iter().filter_map(|v| *v)
    }

    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    pub fn demande_capture(&self) -> bool {
        self.verdicts().any(|v| v.demande_capture())
    }
}

// ---------------------------------------------------------------------------
// LES SEUILS, ET CE QUI LES FONDE
// ---------------------------------------------------------------------------

/// Silence de reception avant de conclure a un arret DMA.
///
/// La meme valeur que `anneau_rx::SILENCE_RX_NS`, et pour la meme raison : un
/// reseau domestique au repos peut rester une seconde sans une trame de
/// diffusion. Ce qui distingue l'anomalie, ce n'est pas la duree seule --
/// c'est qu'`isr_rx_ok` monte PENDANT ce silence.
pub const SEUIL_STALL_NS: u64 = 3_000_000_000;

/// Attente accordee au second tour apres le premier bouclage.
///
/// Le releve physique montre cent cinquante et une secondes sans un seul
/// descripteur de second tour. Trois secondes suffisent donc largement a
/// distinguer « la carte prend son temps » de « le second tour n'aura pas
/// lieu », et laissent la capture se faire pres de l'evenement.
pub const SEUIL_TOUR2_NS: u64 = 3_000_000_000;

/// Immobilite de la persistance toleree pendant que la RAM avance.
///
/// L'enonce physique : session reelle 395 s, records persistes 193 s. Cinq
/// secondes d'ecart sont deja anormales pour une boite noire qui purge a
/// chaque enregistrement ; deux cents secondes ne devaient pas passer
/// inapercues.
pub const SEUIL_BB_STALL_NS: u64 = 5_000_000_000;

/// Cadence de croisiere, en hertz.
pub const CADENCE_NORMALE_HZ: u32 = 1;
/// Cadence autour d'une anomalie, en hertz.
pub const CADENCE_ANOMALIE_HZ: u32 = 10;
/// Duree pendant laquelle la cadence rapide est tenue apres une anomalie.
///
/// # Pourquoi elle est BORNEE
///
/// Une panne durable -- et celle du RTL8168 l'est : cent cinquante et une
/// secondes -- ferait tourner l'auditeur a dix hertz pour toujours, sur une
/// machine dont on essaie justement de mesurer l'ordonnancement. La cadence
/// rapide sert a capturer les ABORDS d'une anomalie, pas a l'accompagner.
pub const FENETRE_ANOMALIE_NS: u64 = 5_000_000_000;

/// L'auditeur. Un etat, des temoins, et aucune dependance.
#[derive(Clone, Copy, Debug, Default)]
pub struct Auditeur {
    tours: u64,

    // --- temoin de progression reseau -------------------------------------
    temoin_pose: bool,
    temoin_ns: u64,
    temoin_rx_paquets: u64,
    temoin_isr_rx_ok: u64,
    temoin_desc_processeur: u32,
    temoin_own_rendus: u64,

    // --- le premier bouclage ----------------------------------------------
    //
    // Un drapeau EXPLICITE, et non « l'instant vaut zero ». L'horloge
    // monotone rend zero tant qu'elle n'est pas calibree, et le bouclage
    // `63 -> 0` peut survenir tot : avec zero pour sentinelle, un premier
    // bouclage horodate zero se ferait redecouvrir a chaque tour, l'attente
    // resterait nulle, et le verdict ne tomberait JAMAIS. C'est exactement le
    // genre de panne que ce module existe pour ne pas avoir.
    bouclage_vu: bool,
    premier_bouclage_ns: u64,

    // --- temoin de persistance --------------------------------------------
    bb_temoin_pose: bool,
    bb_temoin_ns: u64,
    bb_temoin_ram: u64,
    bb_temoin_persistes: u64,

    // --- anti-repetition ---------------------------------------------------
    //
    // Un verdict qui se repete a chaque tour noierait l'anneau : la panne dure
    // cent cinquante secondes, soit cent cinquante emissions identiques a un
    // hertz. On signale la TRANSITION, et on se rearme quand l'etat redevient
    // sain -- une panne qui revient est un fait neuf.
    stall_signale: bool,
    tour2_signale: bool,
    invariant_signale: bool,
    moteur_signale: bool,
    bb_signale: bool,

    // --- cadence -----------------------------------------------------------
    cadence_hz: u32,
    cadence_jusqu_a_ns: u64,
}

impl Auditeur {
    pub const fn nouveau() -> Self {
        Self {
            tours: 0,
            temoin_pose: false,
            temoin_ns: 0,
            temoin_rx_paquets: 0,
            temoin_isr_rx_ok: 0,
            temoin_desc_processeur: 0,
            temoin_own_rendus: 0,
            bouclage_vu: false,
            premier_bouclage_ns: 0,
            bb_temoin_pose: false,
            bb_temoin_ns: 0,
            bb_temoin_ram: 0,
            bb_temoin_persistes: 0,
            stall_signale: false,
            tour2_signale: false,
            invariant_signale: false,
            moteur_signale: false,
            bb_signale: false,
            cadence_hz: CADENCE_NORMALE_HZ,
            cadence_jusqu_a_ns: 0,
        }
    }

    pub fn tours(&self) -> u64 {
        self.tours
    }

    pub fn cadence_hz(&self) -> u32 {
        self.cadence_hz
    }

    /// Un tour d'audit.
    pub fn examine(&mut self, obs: &Observation) -> Rapport {
        self.tours += 1;
        let cadence_avant = self.cadence_hz;
        let mut rapport = Rapport::vide(self.cadence_hz);

        self.juge_le_moteur(obs, &mut rapport);
        self.juge_l_invariant(obs, &mut rapport);
        self.juge_le_second_tour(obs, &mut rapport);
        self.juge_le_stall_dma(obs, &mut rapport);
        self.juge_la_persistance(obs, &mut rapport);

        // La cadence monte sur anomalie, et RETOMBE d'elle-meme. Voir
        // `FENETRE_ANOMALIE_NS`.
        if !rapport.is_empty() {
            self.cadence_hz = CADENCE_ANOMALIE_HZ;
            self.cadence_jusqu_a_ns = obs.t_ns.saturating_add(FENETRE_ANOMALIE_NS);
        } else if self.cadence_hz != CADENCE_NORMALE_HZ && obs.t_ns >= self.cadence_jusqu_a_ns {
            self.cadence_hz = CADENCE_NORMALE_HZ;
        }
        rapport.cadence_hz = self.cadence_hz;
        rapport.cadence_changee = self.cadence_hz != cadence_avant;
        rapport
    }

    /// `RxEnb` est-il tombe ?
    fn juge_le_moteur(&mut self, obs: &Observation, rapport: &mut Rapport) {
        const RX_ENB: u8 = 0x08;
        // Une carte qu'on n'a pas encore programmee a `chip_cmd = 0`, et ce
        // n'est pas une panne : sans lien, on n'accuse personne.
        if !obs.lien || obs.chip_cmd == 0 {
            self.moteur_signale = false;
            return;
        }
        if obs.chip_cmd & RX_ENB == 0 {
            if !self.moteur_signale {
                self.moteur_signale = true;
                rapport.pousse(Verdict::MoteurRxArrete { chip_cmd: obs.chip_cmd });
            }
        } else {
            self.moteur_signale = false;
        }
    }

    fn juge_l_invariant(&mut self, obs: &Observation, rapport: &mut Rapport) {
        if obs.invariant_casse {
            if !self.invariant_signale {
                self.invariant_signale = true;
                rapport.pousse(Verdict::AnneauInvariant {
                    code: obs.invariant_code,
                    rx_cur: obs.rx_cur,
                });
            }
        } else {
            self.invariant_signale = false;
        }
    }

    /// LE VERDICT QUI NOMME LA PANNE PHYSIQUE.
    ///
    /// Un premier tour complet, puis plus un seul descripteur rendu au second.
    /// Ce n'est pas « la reception est lente » : c'est « l'anneau ne circule
    /// pas », et les deux ne se reparent pas pareil.
    fn juge_le_second_tour(&mut self, obs: &Observation, rapport: &mut Rapport) {
        if obs.rx_tours_cpu == 0 {
            // Le processeur n'a pas encore boucle : rien a dire.
            self.bouclage_vu = false;
            self.premier_bouclage_ns = 0;
            self.tour2_signale = false;
            return;
        }
        if !self.bouclage_vu {
            self.bouclage_vu = true;
            self.premier_bouclage_ns = obs.t_ns;
        }
        if obs.rx_rendus_tour2 > 0 {
            // L'anneau circule. Un retour de la panne serait un fait neuf.
            self.tour2_signale = false;
            return;
        }
        let attente = obs.t_ns.saturating_sub(self.premier_bouclage_ns);
        if attente >= SEUIL_TOUR2_NS && !self.tour2_signale {
            self.tour2_signale = true;
            rapport.pousse(Verdict::SecondTourAbsent {
                rendus_tour1: obs.rx_rendus_tour1,
                rendus_tour2: obs.rx_rendus_tour2,
                attente_ns: attente,
            });
        }
    }

    /// LES TROIS CONDITIONS ENSEMBLE, ET PAS DEUX.
    ///
    /// `rx_paquets` immobile seul, c'est un reseau calme. `isr_rx_ok` qui monte
    /// seul, c'est du trafic normal. Aucun descripteur rendu seul, c'est un
    /// anneau plein. Les trois ensemble, c'est une carte qui annonce des
    /// trames qu'elle n'ecrit pas -- et cela n'a pas d'autre explication.
    fn juge_le_stall_dma(&mut self, obs: &Observation, rapport: &mut Rapport) {
        if !self.temoin_pose {
            self.pose_temoin(obs);
            return;
        }
        let progres_reception = obs.rx_paquets > self.temoin_rx_paquets;
        let progres_descripteurs = obs.desc_processeur > self.temoin_desc_processeur
            || obs.rx_own_rendus > self.temoin_own_rendus;
        if progres_reception || progres_descripteurs {
            // Quelque chose entre : le temoin se repose, et l'ardoise
            // s'efface.
            self.pose_temoin(obs);
            self.stall_signale = false;
            return;
        }
        let silence = obs.t_ns.saturating_sub(self.temoin_ns);
        if silence < SEUIL_STALL_NS {
            return;
        }
        let isr_delta = obs.isr_rx_ok.saturating_sub(self.temoin_isr_rx_ok);
        if isr_delta == 0 {
            // Silence complet : la carte n'annonce rien non plus. C'est un
            // reseau muet, pas une carte qui ment. On ne l'accuse pas, et on
            // ne repose pas le temoin -- si `RxOK` repart sans que la
            // reception suive, le silence deja ecoule compte.
            return;
        }
        if !self.stall_signale {
            self.stall_signale = true;
            rapport.pousse(Verdict::RxDmaStall {
                silence_ns: silence,
                isr_delta,
                desc_processeur: obs.desc_processeur,
            });
        }
    }

    fn pose_temoin(&mut self, obs: &Observation) {
        self.temoin_pose = true;
        self.temoin_ns = obs.t_ns;
        self.temoin_rx_paquets = obs.rx_paquets;
        self.temoin_isr_rx_ok = obs.isr_rx_ok;
        self.temoin_desc_processeur = obs.desc_processeur;
        self.temoin_own_rendus = obs.rx_own_rendus;
    }

    /// La RAM avance-t-elle pendant que le support ne bouge plus ?
    ///
    /// La conjonction, encore. Une boite noire dont RIEN n'avance est une
    /// machine au repos ; ce qui accuse, c'est l'ECART qui se creuse.
    fn juge_la_persistance(&mut self, obs: &Observation, rapport: &mut Rapport) {
        if !self.bb_temoin_pose {
            self.pose_temoin_bb(obs);
            return;
        }
        if obs.bb_records_persistes > self.bb_temoin_persistes {
            self.pose_temoin_bb(obs);
            self.bb_signale = false;
            return;
        }
        if obs.bb_records_ram <= self.bb_temoin_ram {
            // La RAM n'avance pas non plus : rien n'est en retard.
            self.pose_temoin_bb(obs);
            return;
        }
        let fige = obs.t_ns.saturating_sub(self.bb_temoin_ns);
        if fige >= SEUIL_BB_STALL_NS && !self.bb_signale {
            self.bb_signale = true;
            rapport.pousse(Verdict::BlackboxPersistenceStall {
                records_ram: obs.bb_records_ram,
                records_persistes: obs.bb_records_persistes,
                fige_depuis_ns: fige,
            });
        }
    }

    fn pose_temoin_bb(&mut self, obs: &Observation) {
        self.bb_temoin_pose = true;
        self.bb_temoin_ns = obs.t_ns;
        self.bb_temoin_ram = obs.bb_records_ram;
        self.bb_temoin_persistes = obs.bb_records_persistes;
    }
}
