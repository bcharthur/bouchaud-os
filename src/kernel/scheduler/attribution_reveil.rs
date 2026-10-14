//! D'ou vient le retard d'un reveil de sommeil.
//!
//! BOUCHAUD_ATTRIBUTION_REVEIL_V1
//!
//! # Pourquoi
//!
//! L'endurance hebdomadaire rougit sur `ordonnanceur-probe` UN reveil sur
//! plusieurs centaines : SMP1, run 38533463034, cycle 5, interface
//! interactive -- p99 1 855 us (mieux que la classe normale, 4 816 us), mais
//! un pire de 9 333 us, au-dela du pire normal (5 284 us) plus le bruit au
//! repos (2 134 us). Le seul releve qui nommait un pire, `[SCHED-NG-PIRE]`,
//! est un maximum sur tout le demarrage : il designe le boot (54 ms a
//! 1,1 s), jamais la fenetre de la sonde.
//!
//! Un retard de reveil a quatre causes possibles, que rien ne separait :
//!
//!   1. le balayage des echeances est passe tard (`tardif` : echeance ->
//!      pret). Sur un coeur occupe par du calcul ring 3, il passe au plus tard
//!      au prochain quantum (4 tics) ; au-dela, c'est qu'un quantum n'a pas
//!      reveille -- contexte noyau, transition deja ouverte ;
//!   2. les tics ne sont pas arrives (`tics_manquants`) : interruptions
//!      masquees, ou processeur virtuel retire par l'hote ;
//!   3. la tache prete a attendu son election (`attente` : pret -> elu) ;
//!   4. le retour au fil apres l'election (`reprise` : elu -> reprise).
//!
//! # Ce que ce module fait
//!
//! `dort_jusqua` releve ses instants, `decompose` (pure, testee sur l'hote)
//! en tire les quatre parts, et la tache garde son PIRE reveil avec sa
//! decomposition (`Registre`). `/proc/self/reveil` le rend et le remet a
//! zero : la sonde le lit a la fin de chaque phase, trois lectures en tout --
//! rien qui perturbe la mesure.
//!
//! Les tics du PIT (IRQ0, coeur zero, 1 ms) gardent leur pire ECART et le
//! RIP qu'interrompt le tic qui le termine (`note_tic`). Un tic qui tombe
//! en ring 3 apres un long ecart dit : les interruptions etaient ouvertes,
//! le temps a manque au processeur virtuel lui-meme (hote). Un tic qui tombe
//! dans le noyau, a la sortie d'une section masquee, nomme cette section.

use core::sync::atomic::{AtomicU64, Ordering};

/// Les instants d'un sommeil, en nanosecondes monotones, et les tics du PIT.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mesure {
    /// L'instant ou la tache devait repartir.
    pub echeance_ns: u64,
    /// L'entree dans le sommeil.
    pub entree_ns: u64,
    /// L'instant ou le balayage des echeances l'a rendue prete ; 0 si elle a
    /// ete rendue prete par autre chose (signal) ou jamais garee.
    pub pret_ns: u64,
    /// Sa premiere election apres `pret_ns` ; 0 si aucune n'a ete notee.
    pub elu_ns: u64,
    /// Le retour dans `dort_jusqua`, apres la boucle.
    pub reprise_ns: u64,
    pub tics_entree: u64,
    pub tics_reprise: u64,
}

/// Les parts d'un retard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Decomposition {
    /// `reprise - echeance`.
    pub total_ns: u64,
    /// echeance -> pret (ou tout le retard si `sans_balayage`).
    pub tardif_ns: u64,
    /// pret -> elu.
    pub attente_ns: u64,
    /// elu -> reprise.
    pub reprise_ns: u64,
    /// Duree du sommeil entier, en millisecondes.
    pub dormi_ms: u64,
    /// Tics du PIT vus pendant le sommeil.
    pub tics_vus: u64,
    /// Millisecondes sans tic, au-dela d'un tic de tolerance.
    pub tics_manquants: u64,
    /// Rendue prete sans le balayage des echeances.
    pub sans_balayage: bool,
}

/// La decomposition d'un reveil. Pure.
pub fn decompose(m: &Mesure) -> Decomposition {
    let total_ns = m.reprise_ns.saturating_sub(m.echeance_ns);
    let dormi_ms = m.reprise_ns.saturating_sub(m.entree_ns) / 1_000_000;
    let tics_vus = m.tics_reprise.saturating_sub(m.tics_entree);
    // Un tic de tolerance : le premier et le dernier peuvent tomber juste
    // hors de l'intervalle mesure.
    let tics_manquants = dormi_ms.saturating_sub(tics_vus.saturating_add(1));
    if m.pret_ns == 0 || m.pret_ns < m.entree_ns || m.pret_ns > m.reprise_ns {
        return Decomposition {
            total_ns,
            tardif_ns: total_ns,
            attente_ns: 0,
            reprise_ns: 0,
            dormi_ms,
            tics_vus,
            tics_manquants,
            sans_balayage: true,
        };
    }
    let elu = if m.elu_ns >= m.pret_ns && m.elu_ns <= m.reprise_ns { m.elu_ns } else { m.pret_ns };
    Decomposition {
        total_ns,
        tardif_ns: m.pret_ns.saturating_sub(m.echeance_ns),
        attente_ns: elu - m.pret_ns,
        reprise_ns: m.reprise_ns - elu,
        dormi_ms,
        tics_vus,
        tics_manquants,
        sans_balayage: false,
    }
}

/// Au-dela : un quantum (4 ms) n'a pas suffi a voir l'echeance.
pub const TARDIF_SIGNALE_NS: u64 = 5_000_000;
/// Au-dela : la tache prete a attendu plus d'un tic son election.
pub const ATTENTE_SIGNALEE_NS: u64 = 1_000_000;

/// Le pire reveil d'une tache depuis le dernier releve, et des compteurs.
///
/// Ecrit par la tache elle-meme (a la fin de son sommeil), relu par elle-meme
/// (`/proc/self/reveil`) : aucune course a craindre au-dela de l'atomicite de
/// chaque champ.
pub struct Registre {
    reveils: AtomicU64,
    total_ns: AtomicU64,
    tardif_ns: AtomicU64,
    attente_ns: AtomicU64,
    reprise_ns: AtomicU64,
    dormi_ms: AtomicU64,
    tics_vus: AtomicU64,
    sans_balayage: AtomicU64,
    cpu: AtomicU64,
    a_ns: AtomicU64,
    par: AtomicU64,
    tardifs_signales: AtomicU64,
    attentes_signalees: AtomicU64,
    sommeils_tics_manquants: AtomicU64,
}

/// Ce que `Registre::releve` rend.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Releve {
    pub reveils: u64,
    pub pire: Decomposition,
    pub cpu: u64,
    pub a_ns: u64,
    /// Qui tenait le coeur quand le balayage l'a enfin reveillee : voir
    /// `empaquete_par`.
    pub par: u64,
    pub tardifs_signales: u64,
    pub attentes_signalees: u64,
    pub sommeils_tics_manquants: u64,
}

impl Registre {
    pub const fn neuf() -> Self {
        Self {
            reveils: AtomicU64::new(0),
            total_ns: AtomicU64::new(0),
            tardif_ns: AtomicU64::new(0),
            attente_ns: AtomicU64::new(0),
            reprise_ns: AtomicU64::new(0),
            dormi_ms: AtomicU64::new(0),
            tics_vus: AtomicU64::new(0),
            sans_balayage: AtomicU64::new(0),
            cpu: AtomicU64::new(0),
            a_ns: AtomicU64::new(0),
            par: AtomicU64::new(0),
            tardifs_signales: AtomicU64::new(0),
            attentes_signalees: AtomicU64::new(0),
            sommeils_tics_manquants: AtomicU64::new(0),
        }
    }

    /// Compte un reveil ; le garde s'il est le pire depuis le dernier releve.
    pub fn note(&self, d: &Decomposition, a_ns: u64, cpu: u64, par: u64) {
        let n = self.reveils.fetch_add(1, Ordering::Relaxed);
        if d.tardif_ns > TARDIF_SIGNALE_NS {
            self.tardifs_signales.fetch_add(1, Ordering::Relaxed);
        }
        if d.attente_ns > ATTENTE_SIGNALEE_NS {
            self.attentes_signalees.fetch_add(1, Ordering::Relaxed);
        }
        if d.tics_manquants > 0 {
            self.sommeils_tics_manquants.fetch_add(1, Ordering::Relaxed);
        }
        if n == 0 || d.total_ns > self.total_ns.load(Ordering::Relaxed) {
            self.total_ns.store(d.total_ns, Ordering::Relaxed);
            self.tardif_ns.store(d.tardif_ns, Ordering::Relaxed);
            self.attente_ns.store(d.attente_ns, Ordering::Relaxed);
            self.reprise_ns.store(d.reprise_ns, Ordering::Relaxed);
            self.dormi_ms.store(d.dormi_ms, Ordering::Relaxed);
            self.tics_vus.store(d.tics_vus, Ordering::Relaxed);
            self.sans_balayage.store(d.sans_balayage as u64, Ordering::Relaxed);
            self.cpu.store(cpu, Ordering::Relaxed);
            self.a_ns.store(a_ns, Ordering::Relaxed);
            self.par.store(par, Ordering::Relaxed);
        }
    }

    /// Rend le pire et les compteurs, et remet tout a zero.
    pub fn releve(&self) -> Releve {
        let dormi_ms = self.dormi_ms.swap(0, Ordering::Relaxed);
        let tics_vus = self.tics_vus.swap(0, Ordering::Relaxed);
        Releve {
            reveils: self.reveils.swap(0, Ordering::Relaxed),
            pire: Decomposition {
                total_ns: self.total_ns.swap(0, Ordering::Relaxed),
                tardif_ns: self.tardif_ns.swap(0, Ordering::Relaxed),
                attente_ns: self.attente_ns.swap(0, Ordering::Relaxed),
                reprise_ns: self.reprise_ns.swap(0, Ordering::Relaxed),
                dormi_ms,
                tics_vus,
                tics_manquants: dormi_ms.saturating_sub(tics_vus.saturating_add(1)),
                sans_balayage: self.sans_balayage.swap(0, Ordering::Relaxed) != 0,
            },
            cpu: self.cpu.swap(0, Ordering::Relaxed),
            a_ns: self.a_ns.swap(0, Ordering::Relaxed),
            par: self.par.swap(0, Ordering::Relaxed),
            tardifs_signales: self.tardifs_signales.swap(0, Ordering::Relaxed),
            attentes_signalees: self.attentes_signalees.swap(0, Ordering::Relaxed),
            sommeils_tics_manquants: self.sommeils_tics_manquants.swap(0, Ordering::Relaxed),
        }
    }
}

/// Qui tenait le coeur au moment du balayage qui reveille : pid (24 bits),
/// tid (32 bits), fil noyau, et balayage fait depuis une interruption
/// (quantum) plutot que depuis un `schedule()` volontaire. Zero : inconnu.
pub fn empaquete_par(pid: u64, tid: u64, noyau: bool, en_irq: bool) -> u64 {
    ((pid & 0xff_ffff) << 40) | ((tid & 0xffff_ffff) << 8) | ((noyau as u64) << 1) | en_irq as u64 | (1 << 2)
}

/// `(pid, tid, noyau, en_irq)` ; `None` si `par` est vide.
pub fn depaquete_par(par: u64) -> Option<(u64, u64, bool, bool)> {
    if par & (1 << 2) == 0 {
        return None;
    }
    Some(((par >> 40) & 0xff_ffff, (par >> 8) & 0xffff_ffff, par & 2 != 0, par & 1 != 0))
}

// ---------------------------------------------------------------------------
// Les tics du PIT.

/// Un ecart de tic au-dela de deux periodes est un tic perdu.
pub const ECART_TIC_SIGNALE_NS: u64 = 2_000_000;
const CPUS_SUIVIS: usize = 64;

static DERNIER_TIC_NS: [AtomicU64; CPUS_SUIVIS] = [const { AtomicU64::new(0) }; CPUS_SUIVIS];
static ECART_PIRE_NS: AtomicU64 = AtomicU64::new(0);
static ECART_RIP: AtomicU64 = AtomicU64::new(0);
static ECART_RING3: AtomicU64 = AtomicU64::new(0);
static ECART_CPU: AtomicU64 = AtomicU64::new(0);
static ECART_A_NS: AtomicU64 = AtomicU64::new(0);
static ECARTS_SIGNALES: AtomicU64 = AtomicU64::new(0);

/// Le pire ecart entre deux tics du PIT depuis le dernier releve.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReleveTics {
    pub pire_ns: u64,
    /// Le RIP qu'interrompait le tic qui a TERMINE ce pire ecart.
    pub rip: u64,
    /// Ce tic interrompait-il le ring 3 (interruptions forcement ouvertes) ?
    pub ring3: bool,
    pub cpu: u64,
    pub a_ns: u64,
    /// Ecarts au-dela de `ECART_TIC_SIGNALE_NS`.
    pub signales: u64,
}

/// A chaque tic du PIT : l'ecart depuis le precedent sur ce coeur.
pub fn note_tic(cpu: usize, maintenant: u64, rip: u64, ring3: bool) {
    if cpu >= CPUS_SUIVIS {
        return;
    }
    let precedent = DERNIER_TIC_NS[cpu].swap(maintenant, Ordering::Relaxed);
    if precedent == 0 || maintenant <= precedent {
        return;
    }
    let ecart = maintenant - precedent;
    if ecart <= ECART_TIC_SIGNALE_NS {
        return;
    }
    ECARTS_SIGNALES.fetch_add(1, Ordering::Relaxed);
    if ecart > ECART_PIRE_NS.load(Ordering::Relaxed) {
        ECART_PIRE_NS.store(ecart, Ordering::Relaxed);
        ECART_RIP.store(rip, Ordering::Relaxed);
        ECART_RING3.store(ring3 as u64, Ordering::Relaxed);
        ECART_CPU.store(cpu as u64, Ordering::Relaxed);
        ECART_A_NS.store(maintenant, Ordering::Relaxed);
    }
}

/// Rend le pire ecart et remet le releve a zero.
pub fn releve_tics() -> ReleveTics {
    ReleveTics {
        pire_ns: ECART_PIRE_NS.swap(0, Ordering::Relaxed),
        rip: ECART_RIP.swap(0, Ordering::Relaxed),
        ring3: ECART_RING3.swap(0, Ordering::Relaxed) != 0,
        cpu: ECART_CPU.swap(0, Ordering::Relaxed),
        a_ns: ECART_A_NS.swap(0, Ordering::Relaxed),
        signales: ECARTS_SIGNALES.swap(0, Ordering::Relaxed),
    }
}

// ---------------------------------------------------------------------------
// Les sections ou ce coeur a masque les interruptions.
//
// Un tic perdu a deux causes possibles : le noyau masquait les interruptions,
// ou le processeur virtuel n'avancait pas du tout (hote, traduction TCG). Le
// RIP du tic qui termine l'ecart ne les separe pas : un gestionnaire long,
// masque, qui rend la main au ring 3 livre le tic en attente EN ring 3. Ce
// qui les separe : la duree de la section masquee qui VIENT de se terminer
// quand le tic tombe. Egale a l'ecart : le noyau. Tres inferieure : le temps
// a manque hors de l'invite.
//
// Mesurees : les sections `SpinLockIrq` les plus externes (celles qui
// rouvrent IF, avec leur site `#[track_caller]`) et les gestionnaires du
// tic et du quantum (entree -> sortie, commutation comprise).

/// Un site est un `&'static Location` (verrou) ou, bit de poids fort pose, un
/// vecteur d'interruption.
pub const SITE_VECTEUR: u64 = 1 << 63;

static ENTREE_IRQ_TSC: [AtomicU64; CPUS_SUIVIS] = [const { AtomicU64::new(0) }; CPUS_SUIVIS];
static FIN_MASQUEE_TSC: [AtomicU64; CPUS_SUIVIS] = [const { AtomicU64::new(0) }; CPUS_SUIVIS];
static DUREE_MASQUEE_CYC: [AtomicU64; CPUS_SUIVIS] = [const { AtomicU64::new(0) }; CPUS_SUIVIS];
static SITE_MASQUE: [AtomicU64; CPUS_SUIVIS] = [const { AtomicU64::new(0) }; CPUS_SUIVIS];
static PIRE_MASQUE_CYC: AtomicU64 = AtomicU64::new(0);
static PIRE_MASQUE_SITE: AtomicU64 = AtomicU64::new(0);
static MASQUEES_SIGNALEES: AtomicU64 = AtomicU64::new(0);
static ECART_SECTION_CYC: AtomicU64 = AtomicU64::new(0);
static ECART_SECTION_SITE: AtomicU64 = AtomicU64::new(0);

/// Une section masquee vient de se terminer sur `cpu`.
#[inline]
pub fn note_section_masquee(cpu: usize, debut_tsc: u64, fin_tsc: u64, site: u64, cycles_par_ms: u64) {
    if cpu >= CPUS_SUIVIS || debut_tsc == 0 || fin_tsc <= debut_tsc {
        return;
    }
    let duree = fin_tsc - debut_tsc;
    DUREE_MASQUEE_CYC[cpu].store(duree, Ordering::Relaxed);
    SITE_MASQUE[cpu].store(site, Ordering::Relaxed);
    FIN_MASQUEE_TSC[cpu].store(fin_tsc, Ordering::Relaxed);
    if cycles_par_ms != 0 && duree > cycles_par_ms {
        MASQUEES_SIGNALEES.fetch_add(1, Ordering::Relaxed);
    }
    if duree > PIRE_MASQUE_CYC.load(Ordering::Relaxed) {
        PIRE_MASQUE_CYC.store(duree, Ordering::Relaxed);
        PIRE_MASQUE_SITE.store(site, Ordering::Relaxed);
    }
}

/// Entree d'un gestionnaire d'interruption (IF vient d'etre baisse).
#[inline]
pub fn entree_irq(cpu: usize, tsc: u64) {
    if cpu < CPUS_SUIVIS {
        ENTREE_IRQ_TSC[cpu].store(tsc, Ordering::Relaxed);
    }
}

/// Sortie d'un gestionnaire (juste avant `iretq`) : la section masquee
/// depuis l'entree la plus recente sur ce coeur.
#[inline]
pub fn sortie_irq(cpu: usize, tsc: u64, vecteur: u8, cycles_par_ms: u64) {
    if cpu >= CPUS_SUIVIS {
        return;
    }
    let debut = ENTREE_IRQ_TSC[cpu].swap(0, Ordering::Relaxed);
    note_section_masquee(cpu, debut, tsc, SITE_VECTEUR | vecteur as u64, cycles_par_ms);
}

/// Commutation faite HORS d'une interruption (`schedule()` volontaire) : la
/// tache entrante peut reprendre au milieu d'un gestionnaire suspendu sans
/// qu'aucune entree d'interruption n'ait eu lieu sur ce coeur depuis. Son
/// `sortie_irq` lirait une entree perimee ; on l'efface, et cette sortie-la
/// n'est pas mesuree plutot que mal mesuree.
#[inline]
pub fn commutation_hors_irq(cpu: usize) {
    if cpu < CPUS_SUIVIS {
        ENTREE_IRQ_TSC[cpu].store(0, Ordering::Relaxed);
    }
}

/// La section masquee terminee sur `cpu` au plus `fenetre_cyc` avant `tsc`.
fn section_juste_avant(cpu: usize, tsc: u64, fenetre_cyc: u64) -> (u64, u64) {
    let fin = FIN_MASQUEE_TSC[cpu].load(Ordering::Relaxed);
    if fin == 0 || tsc < fin || tsc - fin > fenetre_cyc {
        return (0, 0);
    }
    (DUREE_MASQUEE_CYC[cpu].load(Ordering::Relaxed), SITE_MASQUE[cpu].load(Ordering::Relaxed))
}

/// `note_tic`, avec la section masquee qui vient de se terminer quand
/// l'ecart est le pire (fenetre : un dixieme de milliseconde).
pub fn note_tic_tsc(cpu: usize, maintenant: u64, tsc: u64, rip: u64, ring3: bool, cycles_par_ms: u64) {
    if cpu >= CPUS_SUIVIS {
        return;
    }
    let avant = ECART_PIRE_NS.load(Ordering::Relaxed);
    note_tic(cpu, maintenant, rip, ring3);
    if ECART_PIRE_NS.load(Ordering::Relaxed) != avant && ECART_A_NS.load(Ordering::Relaxed) == maintenant {
        let (duree, site) = section_juste_avant(cpu, tsc, cycles_par_ms / 10);
        ECART_SECTION_CYC.store(duree, Ordering::Relaxed);
        ECART_SECTION_SITE.store(site, Ordering::Relaxed);
    }
}

/// Le pire masquage depuis le dernier releve, et la section terminee juste
/// avant le tic du pire ecart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReleveMasquage {
    pub pire_cyc: u64,
    pub pire_site: u64,
    /// Sections de plus d'une milliseconde.
    pub signalees: u64,
    /// La section masquee terminee juste avant le tic du pire ecart (0 :
    /// aucune -- le tic est tombe dans du code a IF ouvert).
    pub avant_ecart_cyc: u64,
    pub avant_ecart_site: u64,
}

pub fn releve_masquage() -> ReleveMasquage {
    ReleveMasquage {
        pire_cyc: PIRE_MASQUE_CYC.swap(0, Ordering::Relaxed),
        pire_site: PIRE_MASQUE_SITE.swap(0, Ordering::Relaxed),
        signalees: MASQUEES_SIGNALEES.swap(0, Ordering::Relaxed),
        avant_ecart_cyc: ECART_SECTION_CYC.swap(0, Ordering::Relaxed),
        avant_ecart_site: ECART_SECTION_SITE.swap(0, Ordering::Relaxed),
    }
}
