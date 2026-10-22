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
    /// BOUCHAUD_REPRISE_DECOUPEE_V1 : le retour de `switch_context` dans la
    /// tache elue (sa pile, son contexte), 0 si non note.
    pub commute_ns: u64,
    /// La fin de `complete_switch_handoff` (la passation de la tache
    /// sortante, faite par la tache entrante), 0 si non notee.
    pub passe_ns: u64,
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
    /// BOUCHAUD_REPRISE_DECOUPEE_V1 -- la reprise en trois : elu -> contexte
    /// repris (`install`, `switch_context`), contexte repris -> passation
    /// finie (`complete_switch_handoff`), passation -> retour dans
    /// `dort_jusqua`. Instants absents ou incoherents : tout dans `retour_ns`.
    pub commutation_ns: u64,
    pub passation_ns: u64,
    pub retour_ns: u64,
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
            commutation_ns: 0,
            passation_ns: 0,
            retour_ns: 0,
            dormi_ms,
            tics_vus,
            tics_manquants,
            sans_balayage: true,
        };
    }
    let elu = if m.elu_ns >= m.pret_ns && m.elu_ns <= m.reprise_ns { m.elu_ns } else { m.pret_ns };
    // Les instants de la reprise ne valent que dans l'ordre elu <= commute <=
    // passe <= reprise ; sinon (non notes, ou d'une autre commutation) toute
    // la reprise reste au retour, rien n'est invente.
    let (commutation_ns, passation_ns, retour_ns) =
        if m.commute_ns >= elu && m.passe_ns >= m.commute_ns && m.reprise_ns >= m.passe_ns && m.commute_ns != 0 {
            (m.commute_ns - elu, m.passe_ns - m.commute_ns, m.reprise_ns - m.passe_ns)
        } else {
            (0, 0, m.reprise_ns - elu)
        };
    Decomposition {
        total_ns,
        tardif_ns: m.pret_ns.saturating_sub(m.echeance_ns),
        attente_ns: elu - m.pret_ns,
        reprise_ns: m.reprise_ns - elu,
        commutation_ns,
        passation_ns,
        retour_ns,
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
    commutation_ns: AtomicU64,
    passation_ns: AtomicU64,
    retour_ns: AtomicU64,
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
            commutation_ns: AtomicU64::new(0),
            passation_ns: AtomicU64::new(0),
            retour_ns: AtomicU64::new(0),
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
            self.commutation_ns.store(d.commutation_ns, Ordering::Relaxed);
            self.passation_ns.store(d.passation_ns, Ordering::Relaxed);
            self.retour_ns.store(d.retour_ns, Ordering::Relaxed);
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
                commutation_ns: self.commutation_ns.swap(0, Ordering::Relaxed),
                passation_ns: self.passation_ns.swap(0, Ordering::Relaxed),
                retour_ns: self.retour_ns.swap(0, Ordering::Relaxed),
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
pub fn note_tic_tsc(cpu: usize, maintenant: u64, tsc: u64, rip: u64, ring3: bool, cycles_par_ms: u64, vole: Option<u64>, courant: Courant) {
    if cpu >= CPUS_SUIVIS {
        return;
    }
    let vole_avant = DERNIER_VOL_TIC[cpu].swap(vole.unwrap_or(u64::MAX), Ordering::Relaxed);
    let avant = ECART_PIRE_NS.load(Ordering::Relaxed);
    // Ecrit par ce seul coeur : relu avant `note_tic`, qui le remplace.
    let precedent = DERNIER_TIC_NS[cpu].load(Ordering::Relaxed);
    note_tic(cpu, maintenant, rip, ring3);
    // BOUCHAUD_ATTRIBUTION_REVEIL_V2 : chaque absence du PIT est journalisee.
    if precedent != 0 && maintenant > precedent + PERIODE_PIT_NS + TOLERANCE_BATTEMENT_NS {
        let (duree, _) = section_juste_avant(cpu, tsc, cycles_par_ms / 10);
        let masque_ns = if cycles_par_ms == 0 { 0 } else { duree.saturating_mul(1_000_000) / cycles_par_ms };
        let vole_ns = vol_entre(vole_avant, vole);
        if let Some(a) = absence_de(cpu as u32, SOURCE_PIT, precedent, maintenant, PERIODE_PIT_NS, masque_ns, vole_ns, courant) {
            journalise_absence(&a);
        }
    }
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

// ===========================================================================
// BOUCHAUD_ATTRIBUTION_REVEIL_V2 : la ventilation de CHAQUE reveil.
//
// # Pourquoi V1 ne suffisait pas
//
// V1 garde le PIRE reveil d'une phase. La sonde, elle, compare des quantiles
// (p95, p99, pire) de trois phases. Run 38569175760, KVM SMP1, trois passes
// « RESULTAT : 0 » -- et aucune n'avait rien compare : la phase « au repos »
// (p95 1 935 us, 34 absences du PIT de plus de 2 ms, toutes closes hors de
// toute section masquee) etait plus lente que la phase SOUS CHARGE (p95
// 1 127 us). La sonde concluait « charge non mesurable » et passait. Pour
// juger une comparaison, il faut savoir QUELS reveils ont ete retardes par
// le temps qui manquait hors de l'invite -- pas seulement le pire.
//
// # Une absence de battement
//
// Chaque coeur recoit des battements : le PIT (coeur zero, chaque
// milliseconde ; c'est lui qui balaie les echeances) et le quantum local
// (chaque coeur, toutes les `SCHED_QUANTUM_TICKS` ms). Un battement en
// retard de plus d'une milliseconde sur sa periode ouvre une ABSENCE : de
// l'instant ou il etait du a celui ou un battement est enfin arrive. Elle
// est journalisee (`journalise_absence`) avec la section masquee de
// l'invite qui venait de se terminer a son arrivee. Masquee sur au moins la
// moitie de l'absence : le noyau retenait les interruptions. Sinon : le temps
// a manque hors de l'invite (hote, emulateur) -- l'absence est EXTERNE.
//
// # La ventilation
//
// A la fin de chaque sommeil en retard de plus de `SEUIL_CONTAMINATION_NS`,
// `ventile` repartit le retard, etape par etape :
//
//   * livraison externe / masquee : la part de echeance -> pret couverte par
//     une absence du PIT (contention de l'emulateur / noyau masque) ;
//   * detection : le reste de echeance -> pret -- un battement etait la, et
//     le noyau n'a pas reveille la tache (retard de reveil) ;
//   * election : pret -> elu, hors absence externe du coeur qui l'a elue --
//     la tache prete attendait son tour (politique d'election) ;
//   * election externe : cette absence ;
//   * reprise / reprise externe : elu -> reprise, de meme.
//
// Un reveil dont la part externe depasse `SEUIL_CONTAMINATION_NS` est
// CONTAMINE. La sonde juge chaque comparaison deux fois : sur tous ses
// reveils, et sur les seuls reveils propres. Si les deux jugements different,
// le verdict est NON QUALIFIANT -- jamais une reussite.
//
// BOUCHAUD_PREUVE_HOTE_V1 (V3) : « externe » exige desormais une PREUVE --
// le temps vole publie par l'hyperviseur (`vol_hote`). Une absence ni
// masquee ni prouvee est INEXPLIQUEE et reste au compte du noyau ; sous TCG,
// ou aucune preuve n'existe, aucune absence n'est externe. Et la sonde ne
// lit plus une liste d'echeances a apparier : elle lit, apres CHAQUE reveil,
// les comptes cumules de son fil (`ComptesFil`, `/proc/self/fil`), qui
// couvrent aussi sa fenetre en ring 3.

/// Au-dela, une part EXTERNE rend le reveil contamine ; au-dela, une part
/// est comptee dans sa categorie.
pub const SEUIL_CONTAMINATION_NS: u64 = 1_000_000;
/// Le PIT bat chaque milliseconde ; au-dela de deux, un battement manque.
pub const PERIODE_PIT_NS: u64 = 1_000_000;
/// Tolerance au-dela de la periode avant de parler d'absence.
pub const TOLERANCE_BATTEMENT_NS: u64 = 1_000_000;

pub const SOURCE_PIT: u8 = 1;
pub const SOURCE_QUANTUM: u8 = 2;

/// La tache installee sur un coeur au moment d'un battement
/// (BOUCHAUD_PREUVE_HOTE_V1) : `tache` = indice + 1 (0 : aucune, coeur au
/// repos), `depuis_ns` le debut de sa tranche en cours.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Courant {
    pub tache: u32,
    pub depuis_ns: u64,
}

/// Un battement qui n'est pas venu a l'heure, sur un coeur.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Absence {
    pub cpu: u32,
    pub source: u8,
    /// L'instant ou le battement manquant etait du.
    pub debut_ns: u64,
    /// L'instant ou un battement est enfin arrive.
    pub fin_ns: u64,
    /// La section masquee de l'invite terminee juste avant ce battement.
    pub masque_ns: u64,
    /// Le temps que l'HOTE a vole a ce coeur pendant l'absence, prouve par
    /// l'hyperviseur (`vol_hote`) ; 0 sans preuve (TCG, materiel nu).
    pub vole_ns: u64,
    /// La tache installee sur ce coeur quand le battement est enfin arrive.
    pub courant: Courant,
}

/// Ce qui explique une absence (BOUCHAUD_PREUVE_HOTE_V1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    /// La section masquee de l'invite en couvre au moins la moitie : le
    /// noyau retenait les interruptions.
    Masquee,
    /// L'hyperviseur PROUVE que l'hote a retire le coeur au moins la moitie
    /// de l'absence.
    Externe,
    /// Ni l'un ni l'autre. Jamais mise au compte de l'hote : elle reste a
    /// celui du noyau.
    Inexpliquee,
}

impl Absence {
    pub fn duree_ns(&self) -> u64 {
        self.fin_ns.saturating_sub(self.debut_ns)
    }

    pub fn cause(&self) -> Cause {
        let d = self.duree_ns();
        if self.masque_ns.saturating_mul(2) >= d {
            Cause::Masquee
        } else if self.vole_ns.saturating_mul(2) >= d {
            Cause::Externe
        } else {
            Cause::Inexpliquee
        }
    }

    /// Le temps a manque hors de l'invite, PREUVE a l'appui.
    pub fn externe(&self) -> bool {
        self.cause() == Cause::Externe
    }
}

/// Le vol prouve entre deux battements, depuis les compteurs cumules lus a
/// chacun (`u64::MAX` : pas de lecture). Pure.
pub fn vol_entre(avant: u64, apres: Option<u64>) -> Option<u64> {
    match apres {
        Some(v) if avant != u64::MAX && v >= avant => Some(v - avant),
        _ => None,
    }
}

/// L'absence qu'un battement arrive `maintenant_ns` apres le precedent
/// ouvre, si elle existe. `vole_ns` : le vol de l'hote entre les DEUX
/// battements (`None` sans preuve). Seul ce qui depasse une periode peut
/// tomber dans l'absence : la preuve retenue est le vol moins une periode,
/// jamais plus que l'absence -- elle peut sous-estimer, jamais excuser trop.
/// Pure.
#[allow(clippy::too_many_arguments)]
pub fn absence_de(
    cpu: u32,
    source: u8,
    precedent_ns: u64,
    maintenant_ns: u64,
    periode_ns: u64,
    masque_ns: u64,
    vole_ns: Option<u64>,
    courant: Courant,
) -> Option<Absence> {
    if precedent_ns == 0 || maintenant_ns <= precedent_ns {
        return None;
    }
    if maintenant_ns - precedent_ns <= periode_ns + TOLERANCE_BATTEMENT_NS {
        return None;
    }
    let debut_ns = precedent_ns + periode_ns;
    let duree = maintenant_ns - debut_ns;
    let vole_ns = vole_ns.map_or(0, |v| v.saturating_sub(periode_ns).min(duree));
    Some(Absence { cpu, source, debut_ns, fin_ns: maintenant_ns, masque_ns, vole_ns, courant })
}

/// La part d'un retard, etape par etape.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ventilation {
    pub livraison_externe_ns: u64,
    pub livraison_masquee_ns: u64,
    pub livraison_inexpliquee_ns: u64,
    pub detection_ns: u64,
    pub election_ns: u64,
    pub election_externe_ns: u64,
    pub election_inexpliquee_ns: u64,
    pub reprise_ns: u64,
    pub reprise_externe_ns: u64,
    pub reprise_inexpliquee_ns: u64,
    /// Le journal ne couvrait pas tout l'intervalle (debordement, absence en
    /// cours d'ecriture, plus d'absences que de place) : la part externe est
    /// INCONNUE -- donc non prouvee.
    pub incomplete: bool,
}

impl Ventilation {
    /// La part PROUVEE retiree par l'hote.
    pub fn externe_ns(&self) -> u64 {
        self.livraison_externe_ns + self.election_externe_ns + self.reprise_externe_ns
    }

    /// La part des absences que ni le noyau ni l'hote n'expliquent.
    pub fn inexpliquee_ns(&self) -> u64 {
        self.livraison_inexpliquee_ns + self.election_inexpliquee_ns + self.reprise_inexpliquee_ns
    }

    /// Exclu de la comparaison de controle : retarde par l'hote, PREUVE a
    /// l'appui (BOUCHAUD_PREUVE_HOTE_V1). Un journal incomplet ne prouve
    /// rien : le reveil reste compte.
    pub fn contamine(&self) -> bool {
        self.externe_ns() > SEUIL_CONTAMINATION_NS
    }
}

const COUVERTURE_MAX: usize = 32;

/// La longueur de [a, b) couverte par la reunion des intervalles. Pure ; les
/// intervalles peuvent se chevaucher (PIT et quantum du meme coeur).
fn couverture(a: u64, b: u64, intervalles: &[(u64, u64)]) -> u64 {
    if b <= a {
        return 0;
    }
    let mut tries = [(0u64, 0u64); COUVERTURE_MAX];
    let mut n = 0;
    for &(c, d) in intervalles {
        let (x, y) = (c.max(a), d.min(b));
        if y > x && n < COUVERTURE_MAX {
            // Insertion triee par debut.
            let mut i = n;
            while i > 0 && tries[i - 1].0 > x {
                tries[i] = tries[i - 1];
                i -= 1;
            }
            tries[i] = (x, y);
            n += 1;
        }
    }
    let (mut total, mut fin) = (0u64, a);
    for &(x, y) in &tries[..n] {
        let x = x.max(fin);
        if y > x {
            total += y - x;
            fin = y;
        }
    }
    total
}

/// Repartit le retard d'un reveil. `cpu_reprise` : le coeur ou la tache a
/// repris. Pure.
pub fn ventile(m: &Mesure, d: &Decomposition, cpu_reprise: u32, absences: &[Absence], incomplete: bool) -> Ventilation {
    let pret = m.echeance_ns.saturating_add(d.tardif_ns);
    let elu = pret.saturating_add(d.attente_ns);
    let reprise = elu.saturating_add(d.reprise_ns);
    // Par cause : [Externe, Masquee, Inexpliquee], pour le balayage (PIT) et
    // pour le coeur de reprise.
    let mut pit = [[(0u64, 0u64); COUVERTURE_MAX]; 3];
    let mut coeur = [[(0u64, 0u64); COUVERTURE_MAX]; 3];
    let mut npit = [0usize; 3];
    let mut ncoeur = [0usize; 3];
    let mut incomplete = incomplete;
    let rang = |c: Cause| match c {
        Cause::Externe => 0,
        Cause::Masquee => 1,
        Cause::Inexpliquee => 2,
    };
    for a in absences {
        let intervalle = (a.debut_ns, a.fin_ns);
        let r = rang(a.cause());
        // Le balayage des echeances bat au PIT. Sans balayage (la tache
        // s'est reveillee elle-meme, sur son coeur), ce sont aussi les
        // battements de SON coeur qui comptaient.
        let balaie = (a.source == SOURCE_PIT && a.cpu == 0) || (d.sans_balayage && a.cpu == cpu_reprise);
        if balaie {
            if npit[r] < COUVERTURE_MAX {
                pit[r][npit[r]] = intervalle;
                npit[r] += 1;
            } else {
                incomplete = true;
            }
        }
        // Sur le coeur de reprise, une absence masquee est le noyau qui
        // retient ce coeur : elle reste dans l'election / la reprise.
        if a.cpu == cpu_reprise && r != 1 {
            if ncoeur[r] < COUVERTURE_MAX {
                coeur[r][ncoeur[r]] = intervalle;
                ncoeur[r] += 1;
            } else {
                incomplete = true;
            }
        }
    }
    // Ordre des parts : la preuve d'abord, puis le masquage, puis
    // l'inexplique -- chacune bornee par ce qui reste.
    let livraison_externe_ns = couverture(m.echeance_ns, pret, &pit[0][..npit[0]]).min(d.tardif_ns);
    let reste = d.tardif_ns - livraison_externe_ns;
    let livraison_masquee_ns = couverture(m.echeance_ns, pret, &pit[1][..npit[1]]).min(reste);
    let reste = reste - livraison_masquee_ns;
    let livraison_inexpliquee_ns = couverture(m.echeance_ns, pret, &pit[2][..npit[2]]).min(reste);
    let election_externe_ns = couverture(pret, elu, &coeur[0][..ncoeur[0]]).min(d.attente_ns);
    let election_inexpliquee_ns =
        couverture(pret, elu, &coeur[2][..ncoeur[2]]).min(d.attente_ns - election_externe_ns);
    let reprise_externe_ns = couverture(elu, reprise, &coeur[0][..ncoeur[0]]).min(d.reprise_ns);
    let reprise_inexpliquee_ns =
        couverture(elu, reprise, &coeur[2][..ncoeur[2]]).min(d.reprise_ns - reprise_externe_ns);
    Ventilation {
        livraison_externe_ns,
        livraison_masquee_ns,
        livraison_inexpliquee_ns,
        detection_ns: d.tardif_ns - livraison_externe_ns - livraison_masquee_ns - livraison_inexpliquee_ns,
        election_ns: d.attente_ns - election_externe_ns - election_inexpliquee_ns,
        election_externe_ns,
        election_inexpliquee_ns,
        reprise_ns: d.reprise_ns - reprise_externe_ns - reprise_inexpliquee_ns,
        reprise_externe_ns,
        reprise_inexpliquee_ns,
        incomplete,
    }
}

// ---------------------------------------------------------------------------
// Le journal des absences : un anneau global, ecrit depuis les gestionnaires
// de battement (un ecrivain par case, seqlock), lu a la fin des sommeils.

pub const JOURNAL_ABSENCES: usize = 256;

struct Case {
    seq: AtomicU64,
    /// `tache << 32 | cpu << 8 | source`.
    cpu_source: AtomicU64,
    debut: AtomicU64,
    fin: AtomicU64,
    masque: AtomicU64,
    vole: AtomicU64,
    depuis: AtomicU64,
}

static JOURNAL: [Case; JOURNAL_ABSENCES] = [const {
    Case {
        seq: AtomicU64::new(0),
        cpu_source: AtomicU64::new(0),
        debut: AtomicU64::new(0),
        fin: AtomicU64::new(0),
        masque: AtomicU64::new(0),
        vole: AtomicU64::new(0),
        depuis: AtomicU64::new(0),
    }
}; JOURNAL_ABSENCES];
static JOURNAL_ECRITS: AtomicU64 = AtomicU64::new(0);
static ABSENCES_EXTERNES: AtomicU64 = AtomicU64::new(0);
static ABSENCES_MASQUEES: AtomicU64 = AtomicU64::new(0);
static ABSENCES_INEXPLIQUEES: AtomicU64 = AtomicU64::new(0);

/// Journalise une absence. Appele en interruption : quelques ecritures
/// atomiques, rien d'autre.
pub fn journalise_absence(a: &Absence) {
    match a.cause() {
        Cause::Externe => ABSENCES_EXTERNES.fetch_add(1, Ordering::Relaxed),
        Cause::Masquee => ABSENCES_MASQUEES.fetch_add(1, Ordering::Relaxed),
        Cause::Inexpliquee => ABSENCES_INEXPLIQUEES.fetch_add(1, Ordering::Relaxed),
    };
    let n = JOURNAL_ECRITS.fetch_add(1, Ordering::Relaxed);
    let c = &JOURNAL[(n % JOURNAL_ABSENCES as u64) as usize];
    c.seq.store(2 * n + 1, Ordering::Relaxed);
    core::sync::atomic::fence(Ordering::Release);
    c.cpu_source.store(
        ((a.courant.tache as u64) << 32) | ((a.cpu as u64 & 0xff_ffff) << 8) | a.source as u64,
        Ordering::Relaxed,
    );
    c.debut.store(a.debut_ns, Ordering::Relaxed);
    c.fin.store(a.fin_ns, Ordering::Relaxed);
    c.masque.store(a.masque_ns, Ordering::Relaxed);
    c.vole.store(a.vole_ns, Ordering::Relaxed);
    c.depuis.store(a.courant.depuis_ns, Ordering::Relaxed);
    c.seq.store(2 * n + 2, Ordering::Release);
    // BOUCHAUD_PREUVE_HOTE_V1 : l'absence du fil arme, rangee pour lui.
    ENREGISTREUR.note_absence(a);
}

/// Les absences qui recouvrent [debut, fin), dans `sortie`. Rend leur nombre
/// et `true` si le journal ne permet pas de les connaitre toutes.
pub fn absences_entre(debut: u64, fin: u64, sortie: &mut [Absence]) -> (usize, bool) {
    parcourt_journal(debut, |a| a.fin_ns > debut && a.debut_ns < fin, sortie)
}

/// Parcourt le journal : les absences que `garde` retient, dans `sortie`.
/// `debut` : l'instant a partir duquel le journal doit etre complet. Rend
/// leur nombre et `true` si le journal ne permet pas de les connaitre toutes
/// (case en cours d'ecriture, plus d'absences que de place, anneau qui a
/// deja ecrase des absences closes apres `debut`).
fn parcourt_journal(debut: u64, garde: impl Fn(&Absence) -> bool, sortie: &mut [Absence]) -> (usize, bool) {
    let mut n = 0;
    let mut incomplet = false;
    let mut plus_ancienne_fin = u64::MAX;
    for c in JOURNAL.iter() {
        let s1 = c.seq.load(Ordering::Acquire);
        if s1 == 0 {
            continue;
        }
        if s1 & 1 == 1 {
            // En cours d'ecriture : peut-etre la notre.
            incomplet = true;
            continue;
        }
        let cs = c.cpu_source.load(Ordering::Relaxed);
        let a = Absence {
            cpu: ((cs >> 8) & 0xff_ffff) as u32,
            source: cs as u8,
            debut_ns: c.debut.load(Ordering::Relaxed),
            fin_ns: c.fin.load(Ordering::Relaxed),
            masque_ns: c.masque.load(Ordering::Relaxed),
            vole_ns: c.vole.load(Ordering::Relaxed),
            courant: Courant { tache: (cs >> 32) as u32, depuis_ns: c.depuis.load(Ordering::Relaxed) },
        };
        core::sync::atomic::fence(Ordering::Acquire);
        if c.seq.load(Ordering::Relaxed) != s1 {
            incomplet = true;
            continue;
        }
        plus_ancienne_fin = plus_ancienne_fin.min(a.fin_ns);
        if garde(&a) {
            if n < sortie.len() {
                sortie[n] = a;
                n += 1;
            } else {
                incomplet = true;
            }
        }
    }
    // L'anneau a deja tourne et sa plus ancienne absence finit apres le
    // debut de l'intervalle : des absences plus anciennes, peut-etre dedans,
    // ont ete ecrasees.
    if JOURNAL_ECRITS.load(Ordering::Relaxed) > JOURNAL_ABSENCES as u64 && plus_ancienne_fin > debut {
        incomplet = true;
    }
    (n, incomplet)
}

/// (prouvees externes, masquees, inexpliquees) depuis le demarrage.
pub fn absences_comptees() -> (u64, u64, u64) {
    (
        ABSENCES_EXTERNES.load(Ordering::Relaxed),
        ABSENCES_MASQUEES.load(Ordering::Relaxed),
        ABSENCES_INEXPLIQUEES.load(Ordering::Relaxed),
    )
}

static DERNIER_QUANTUM_NS: [AtomicU64; CPUS_SUIVIS] = [const { AtomicU64::new(0) }; CPUS_SUIVIS];
// BOUCHAUD_ATTENTE_INTERACTIVE_OCCUPANT_V1 : battements du vecteur de
// replanification par coeur (quantum ou tir anticipe), pour dire combien en
// sont passes pendant l'attente d'une interactive.
static BATTEMENTS: [AtomicU64; CPUS_SUIVIS] = [const { AtomicU64::new(0) }; CPUS_SUIVIS];

/// Battements du vecteur de replanification recus par `cpu` depuis le boot.
pub fn battements(cpu: usize) -> u64 {
    if cpu >= CPUS_SUIVIS {
        return 0;
    }
    BATTEMENTS[cpu].load(Ordering::Relaxed)
}

// Le dernier battement de chaque coeur : l'intervalle depuis le precedent, la
// part masquee juste avant (section qui l'a retenu) et le vol de l'hote sur
// cet intervalle. Lus a l'election d'une interactive restee longtemps prete :
// un battement elu en retard dit POURQUOI il etait en retard.
static DERNIER_INTERVALLE_NS: [AtomicU64; CPUS_SUIVIS] = [const { AtomicU64::new(0) }; CPUS_SUIVIS];
static DERNIER_MASQUE_NS: [AtomicU64; CPUS_SUIVIS] = [const { AtomicU64::new(0) }; CPUS_SUIVIS];
static DERNIER_VOLE_NS: [AtomicU64; CPUS_SUIVIS] = [const { AtomicU64::new(u64::MAX) }; CPUS_SUIVIS];

/// `(intervalle ns, masque ns, vol ns ou u64::MAX si inconnu)` du dernier
/// battement de `cpu`.
pub fn dernier_battement(cpu: usize) -> (u64, u64, u64) {
    if cpu >= CPUS_SUIVIS {
        return (0, 0, u64::MAX);
    }
    (
        DERNIER_INTERVALLE_NS[cpu].load(Ordering::Relaxed),
        DERNIER_MASQUE_NS[cpu].load(Ordering::Relaxed),
        DERNIER_VOLE_NS[cpu].load(Ordering::Relaxed),
    )
}
// BOUCHAUD_PREUVE_HOTE_V1 : le vol cumule lu au battement precedent, par
// source (u64::MAX : pas de lecture).
static DERNIER_VOL_TIC: [AtomicU64; CPUS_SUIVIS] = [const { AtomicU64::new(u64::MAX) }; CPUS_SUIVIS];
static DERNIER_VOL_QUANTUM: [AtomicU64; CPUS_SUIVIS] = [const { AtomicU64::new(u64::MAX) }; CPUS_SUIVIS];

/// A chaque battement du quantum local (vecteur de replanification) : une
/// absence au-dela de la periode, avec la section masquee qui la close, le
/// vol prouve de l'hote et la tache installee.
pub fn note_quantum(cpu: usize, maintenant: u64, tsc: u64, periode_ns: u64, cycles_par_ms: u64, vole: Option<u64>, courant: Courant) {
    if cpu >= CPUS_SUIVIS {
        return;
    }
    BATTEMENTS[cpu].fetch_add(1, Ordering::Relaxed);
    let precedent = DERNIER_QUANTUM_NS[cpu].swap(maintenant, Ordering::Relaxed);
    let vole_avant = DERNIER_VOL_QUANTUM[cpu].swap(vole.unwrap_or(u64::MAX), Ordering::Relaxed);
    let (duree, _) = section_juste_avant(cpu, tsc, cycles_par_ms / 10);
    let masque_ns = if cycles_par_ms == 0 { 0 } else { duree.saturating_mul(1_000_000) / cycles_par_ms };
    let vole_ns = vol_entre(vole_avant, vole);
    if precedent != 0 {
        DERNIER_INTERVALLE_NS[cpu].store(maintenant.saturating_sub(precedent), Ordering::Relaxed);
    }
    DERNIER_MASQUE_NS[cpu].store(masque_ns, Ordering::Relaxed);
    DERNIER_VOLE_NS[cpu].store(vole_ns.unwrap_or(u64::MAX), Ordering::Relaxed);
    if let Some(a) = absence_de(cpu as u32, SOURCE_QUANTUM, precedent, maintenant, periode_ns, masque_ns, vole_ns, courant) {
        journalise_absence(&a);
    }
}

/// Une part comptee : combien de reveils l'ont vue depasser le seuil, sa
/// somme et son pire.
pub struct Compteur {
    n: AtomicU64,
    somme_ns: AtomicU64,
    pire_ns: AtomicU64,
}

impl Compteur {
    pub const fn neuf() -> Self {
        Self { n: AtomicU64::new(0), somme_ns: AtomicU64::new(0), pire_ns: AtomicU64::new(0) }
    }

    fn note(&self, ns: u64) {
        if ns > SEUIL_CONTAMINATION_NS {
            self.n.fetch_add(1, Ordering::Relaxed);
        }
        self.somme_ns.fetch_add(ns, Ordering::Relaxed);
        if ns > self.pire_ns.load(Ordering::Relaxed) {
            self.pire_ns.store(ns, Ordering::Relaxed);
        }
    }

    fn releve(&self) -> ReleveCompteur {
        ReleveCompteur {
            n: self.n.swap(0, Ordering::Relaxed),
            somme_ns: self.somme_ns.swap(0, Ordering::Relaxed),
            pire_ns: self.pire_ns.swap(0, Ordering::Relaxed),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReleveCompteur {
    /// Reveils dont cette part depassait `SEUIL_CONTAMINATION_NS`.
    pub n: u64,
    pub somme_ns: u64,
    pub pire_ns: u64,
}

/// Les ventilations d'une tache depuis le dernier releve. Ecrit et relu par
/// la tache elle-meme, comme `Registre`.
pub struct RegistreVentilation {
    ventiles: AtomicU64,
    contamines: AtomicU64,
    inexpliques: AtomicU64,
    incompletes: AtomicU64,
    livraison_externe: Compteur,
    livraison_masquee: Compteur,
    livraison_inexpliquee: Compteur,
    detection: Compteur,
    election: Compteur,
    election_externe: Compteur,
    election_inexpliquee: Compteur,
    reprise: Compteur,
    reprise_externe: Compteur,
    reprise_inexpliquee: Compteur,
}

/// Ce que `RegistreVentilation::releve` rend.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReleveVentilation {
    /// Reveils ventiles (en retard de plus de `SEUIL_CONTAMINATION_NS`).
    pub ventiles: u64,
    /// Dont la part PROUVEE de l'hote depassait le seuil.
    pub contamines: u64,
    /// Dont la part inexpliquee depassait le seuil.
    pub inexpliques: u64,
    pub incompletes: u64,
    pub livraison_externe: ReleveCompteur,
    pub livraison_masquee: ReleveCompteur,
    pub livraison_inexpliquee: ReleveCompteur,
    pub detection: ReleveCompteur,
    pub election: ReleveCompteur,
    pub election_externe: ReleveCompteur,
    pub election_inexpliquee: ReleveCompteur,
    pub reprise: ReleveCompteur,
    pub reprise_externe: ReleveCompteur,
    pub reprise_inexpliquee: ReleveCompteur,
}

impl RegistreVentilation {
    pub const fn neuf() -> Self {
        Self {
            ventiles: AtomicU64::new(0),
            contamines: AtomicU64::new(0),
            inexpliques: AtomicU64::new(0),
            incompletes: AtomicU64::new(0),
            livraison_externe: Compteur::neuf(),
            livraison_masquee: Compteur::neuf(),
            livraison_inexpliquee: Compteur::neuf(),
            detection: Compteur::neuf(),
            election: Compteur::neuf(),
            election_externe: Compteur::neuf(),
            election_inexpliquee: Compteur::neuf(),
            reprise: Compteur::neuf(),
            reprise_externe: Compteur::neuf(),
            reprise_inexpliquee: Compteur::neuf(),
        }
    }

    pub fn note(&self, v: &Ventilation) {
        self.ventiles.fetch_add(1, Ordering::Relaxed);
        self.livraison_externe.note(v.livraison_externe_ns);
        self.livraison_masquee.note(v.livraison_masquee_ns);
        self.livraison_inexpliquee.note(v.livraison_inexpliquee_ns);
        self.detection.note(v.detection_ns);
        self.election.note(v.election_ns);
        self.election_externe.note(v.election_externe_ns);
        self.election_inexpliquee.note(v.election_inexpliquee_ns);
        self.reprise.note(v.reprise_ns);
        self.reprise_externe.note(v.reprise_externe_ns);
        self.reprise_inexpliquee.note(v.reprise_inexpliquee_ns);
        if v.incomplete {
            self.incompletes.fetch_add(1, Ordering::Relaxed);
        }
        if v.contamine() {
            self.contamines.fetch_add(1, Ordering::Relaxed);
        }
        if v.inexpliquee_ns() > SEUIL_CONTAMINATION_NS {
            self.inexpliques.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Rend les ventilations et remet tout a zero.
    pub fn releve(&self) -> ReleveVentilation {
        ReleveVentilation {
            ventiles: self.ventiles.swap(0, Ordering::Relaxed),
            contamines: self.contamines.swap(0, Ordering::Relaxed),
            inexpliques: self.inexpliques.swap(0, Ordering::Relaxed),
            incompletes: self.incompletes.swap(0, Ordering::Relaxed),
            livraison_externe: self.livraison_externe.releve(),
            livraison_masquee: self.livraison_masquee.releve(),
            livraison_inexpliquee: self.livraison_inexpliquee.releve(),
            detection: self.detection.releve(),
            election: self.election.releve(),
            election_externe: self.election_externe.releve(),
            election_inexpliquee: self.election_inexpliquee.releve(),
            reprise: self.reprise.releve(),
            reprise_externe: self.reprise_externe.releve(),
            reprise_inexpliquee: self.reprise_inexpliquee.releve(),
        }
    }
}

// ===========================================================================
// BOUCHAUD_PREUVE_HOTE_V1 : les comptes CUMULES d'un fil, pour juger chaque
// reveil de la sonde sur TOUTE sa fenetre.
//
// # La fenetre aveugle
//
// La ventilation couvre un sommeil : de l'echeance a la reprise dans le
// noyau. La sonde, elle, mesure de l'instant attendu a sa lecture d'horloge
// en ring 3 : entre les deux, le retour en ring 3, `clock_gettime`, et avant
// le sommeil, le calcul de sa duree. Endurance 1413289b SMP4 (TCG) : pire
// retard noyau 2 065 us, pire retard sonde 5 311 us -- au moins 1,2 ms de la
// queue tombait hors de tout instrument (arrondi a la milliseconde compris).
//
// # Les comptes
//
// Cumules depuis la naissance du fil, jamais remis a zero :
//
//   * externe_ns / inexplique_ns / masque_ns : les parts livraison + election
//     de chaque sommeil ventile ;
//   * pret_ns : le temps pret mais non elu (toute attente, preemption en
//     ring 3 comprise) -- la politique ;
//   * incompletes : ventilations dont le journal ne couvrait pas tout.
//
// S'y ajoute, a la lecture, chaque absence d'un battement de son coeur close
// pendant que le fil y etait installe, rognee au debut de sa tranche (jamais
// le temps d'un autre fil) : `/proc/self/fil` la liste avec l'instant ou
// elle s'est close. La part reprise de la ventilation N'EST PAS dans les
// cumuls : l'absence close pendant la reprise est dans cette liste, une
// seule fois.

pub struct ComptesFil {
    externe_ns: AtomicU64,
    inexplique_ns: AtomicU64,
    masque_ns: AtomicU64,
    pret_ns: AtomicU64,
    incompletes: AtomicU64,
}

/// Ce que `/proc/self/fil` rend en tete.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReleveFil {
    pub externe_ns: u64,
    pub inexplique_ns: u64,
    pub masque_ns: u64,
    pub pret_ns: u64,
    pub incompletes: u64,
}

/// La part de `a` qui retient le fil `tache` installe sur le coeur : rognee
/// au debut de sa tranche. Pure.
pub fn part_du_fil(a: &Absence, tache: u32) -> u64 {
    if tache == 0 || a.courant.tache != tache || a.courant.depuis_ns == 0 {
        return 0;
    }
    a.fin_ns.saturating_sub(a.debut_ns.max(a.courant.depuis_ns))
}

impl ComptesFil {
    pub const fn neuf() -> Self {
        Self {
            externe_ns: AtomicU64::new(0),
            inexplique_ns: AtomicU64::new(0),
            masque_ns: AtomicU64::new(0),
            pret_ns: AtomicU64::new(0),
            incompletes: AtomicU64::new(0),
        }
    }

    /// Fin d'un sommeil ventile : les parts livraison et election.
    pub fn note_ventilation(&self, v: &Ventilation) {
        self.externe_ns.fetch_add(v.livraison_externe_ns + v.election_externe_ns, Ordering::Relaxed);
        self.inexplique_ns.fetch_add(v.livraison_inexpliquee_ns + v.election_inexpliquee_ns, Ordering::Relaxed);
        self.masque_ns.fetch_add(v.livraison_masquee_ns, Ordering::Relaxed);
        if v.incomplete {
            self.incompletes.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Une attente pret -> elu (ecrit par le coeur qui elit).
    pub fn note_pret(&self, ns: u64) {
        self.pret_ns.fetch_add(ns, Ordering::Relaxed);
    }

    /// Les cumuls : quatre lectures, rien d'autre (appele a l'entree de
    /// chaque sommeil du fil arme). Les absences closes pendant que le fil
    /// tenait son coeur ne sont PAS ici : le journal se parcourt une fois, a
    /// la lecture de `/proc/self/fil` (`absences_closes_entre`).
    pub fn cumuls(&self) -> ReleveFil {
        ReleveFil {
            externe_ns: self.externe_ns.load(Ordering::Relaxed),
            inexplique_ns: self.inexplique_ns.load(Ordering::Relaxed),
            masque_ns: self.masque_ns.load(Ordering::Relaxed),
            pret_ns: self.pret_ns.load(Ordering::Relaxed),
            incompletes: self.incompletes.load(Ordering::Relaxed),
        }
    }
}

/// Les absences du fil `tache`, closes dans (debut, fin], dans `sortie`.
/// Rend leur nombre et `true` si le journal ne permet pas de les connaitre
/// toutes.
pub fn absences_closes_entre(debut: u64, fin: u64, tache: u32, sortie: &mut [Absence]) -> (usize, bool) {
    parcourt_journal(debut, |a| a.courant.tache == tache && a.fin_ns > debut && a.fin_ns <= fin, sortie)
}

// ---------------------------------------------------------------------------
// BOUCHAUD_PREUVE_HOTE_V1 -- l'enregistreur des sommeils d'UN fil.
//
// # Pourquoi pas une lecture apres chaque reveil
//
// Premiere version : la sonde relisait `/proc/self/fil` apres chaque reveil.
// Mesure locale (TCG SMP1) : la phase « sous charge (normal) » passait d'un
// p95 de 4 ms a 45-76 ms -- trois appels systeme de plus entre la lecture
// d'horloge et le sommeil suivant, autant de points de preemption : la sonde
// normale prenait du retard en cascade derriere les calculs, et la phase
// interactive, elle, ne bougeait pas. L'instrument changeait ce qu'il
// mesurait, et dans le sens qui facilite « la priorite raccourcit la queue ».
//
// Seconde version : le noyau rangeait les comptes a l'entree de chaque
// sommeil, mais en parcourant le journal (256 absences) a chaque fois. Meme
// mesure : mediane normale 7,8 ms contre 5,2-5,7 ms sans l'enregistreur --
// encore trop cher pour un instrument.
//
// # Ce qu'il fait
//
// Le fil qui ouvre `/proc/self/fil` arme l'enregistreur (un seul fil a la
// fois, le dernier qui arme gagne). A l'ENTREE de chacun de ses sommeils --
// echeance deja fixee --, le noyau range six entiers : l'echeance, l'instant
// d'entree et les cumuls du fil (`ComptesFil::cumuls`, quatre lectures). Les
// absences closes pendant que le fil arme tenait son coeur, le gestionnaire
// de battement les range lui-meme (`note_absence`, atomiques seules), avec
// l'instant ou chacune s'est close : rien ne depend de l'anneau global, qui
// debordait en une phase a SMP4 sous TCG. `/proc/self/fil` rend le tout. La sonde relit le tout UNE fois,
// apres sa serie : deux entrees de sommeil consecutives encadrent un
// reveil, retour en ring 3 et lecture d'horloge compris.

pub const SOMMEILS_ENREGISTRES: usize = 512;
/// Absences du fil arme, rangees par le gestionnaire de battement lui-meme :
/// le journal global (256 absences, tous les coeurs) debordait en une phase
/// de 5 s a SMP4 sous TCG.
pub const ABSENCES_ENREGISTREES: usize = 1024;

pub struct Enregistreur {
    /// Indice + 1 de la tache armee (0 : aucune).
    tache: AtomicU64,
    /// L'instant de l'armement : les absences closes depuis seront listees.
    arme_a_ns: AtomicU64,
    n: AtomicU64,
    cases: [[AtomicU64; 6]; SOMMEILS_ENREGISTRES],
    n_absences: AtomicU64,
    /// `[fin_ns, cause (1 E, 2 M, 3 I), part_ns, publie]`.
    absences: [[AtomicU64; 4]; ABSENCES_ENREGISTREES],
}

/// Une absence du fil arme, close pendant qu'il tenait son coeur.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbsenceDuFil {
    pub fin_ns: u64,
    pub cause: Cause,
    /// Rognee au debut de sa tranche (`part_du_fil`).
    pub part_ns: u64,
}

/// Un sommeil enregistre : son echeance et les comptes cumules du fil a son
/// entree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sommeil {
    pub echeance_ns: u64,
    /// L'entree dans le sommeil : borne de l'intervalle du reveil precedent.
    pub entree_ns: u64,
    pub comptes: ReleveFil,
}

impl Enregistreur {
    pub const fn neuf() -> Self {
        Self {
            tache: AtomicU64::new(0),
            arme_a_ns: AtomicU64::new(0),
            n: AtomicU64::new(0),
            cases: [const { [const { AtomicU64::new(0) }; 6] }; SOMMEILS_ENREGISTRES],
            n_absences: AtomicU64::new(0),
            absences: [const { [const { AtomicU64::new(0) }; 4] }; ABSENCES_ENREGISTREES],
        }
    }

    /// Arme pour `tache` a l'instant `maintenant_ns`, et efface. Le dernier
    /// qui arme gagne.
    pub fn arme(&self, tache: u32, maintenant_ns: u64) {
        self.tache.store(0, Ordering::Release);
        self.n.store(0, Ordering::Relaxed);
        for c in self.absences.iter() {
            c[3].store(0, Ordering::Relaxed);
        }
        self.n_absences.store(0, Ordering::Relaxed);
        self.arme_a_ns.store(maintenant_ns, Ordering::Relaxed);
        self.tache.store(tache as u64, Ordering::Release);
    }

    /// L'instant de l'armement pour `tache` (0 s'il n'est pas arme pour lui).
    pub fn arme_a(&self, tache: u32) -> u64 {
        if self.arme_pour(tache) { self.arme_a_ns.load(Ordering::Relaxed) } else { 0 }
    }

    pub fn arme_pour(&self, tache: u32) -> bool {
        tache != 0 && self.tache.load(Ordering::Acquire) == tache as u64
    }

    /// Range un sommeil du fil arme (ecrit par ce fil seul). Au-dela de la
    /// capacite, le sommeil est compte, pas range.
    pub fn note(&self, tache: u32, s: &Sommeil) {
        if !self.arme_pour(tache) {
            return;
        }
        let i = self.n.fetch_add(1, Ordering::Relaxed) as usize;
        if i < SOMMEILS_ENREGISTRES {
            let c = &self.cases[i];
            c[0].store(s.echeance_ns, Ordering::Relaxed);
            c[1].store(s.entree_ns, Ordering::Relaxed);
            c[2].store(s.comptes.externe_ns, Ordering::Relaxed);
            c[3].store(s.comptes.inexplique_ns, Ordering::Relaxed);
            c[4].store(s.comptes.masque_ns, Ordering::Relaxed);
            c[5].store(s.comptes.pret_ns, Ordering::Relaxed);
        }
    }

    /// Une absence vient d'etre journalisee : si elle s'est close pendant
    /// que le fil arme tenait son coeur, elle est rangee ici. Appele en
    /// interruption : atomiques seules.
    pub fn note_absence(&self, a: &Absence) {
        let tache = a.courant.tache;
        if tache == 0 || self.tache.load(Ordering::Acquire) != tache as u64 {
            return;
        }
        let part = part_du_fil(a, tache);
        if part == 0 {
            return;
        }
        let i = self.n_absences.fetch_add(1, Ordering::Relaxed) as usize;
        if i < ABSENCES_ENREGISTREES {
            let c = &self.absences[i];
            let cause = match a.cause() {
                Cause::Externe => 1,
                Cause::Masquee => 2,
                Cause::Inexpliquee => 3,
            };
            c[0].store(a.fin_ns, Ordering::Relaxed);
            c[1].store(cause, Ordering::Relaxed);
            c[2].store(part, Ordering::Relaxed);
            c[3].store(1, Ordering::Release);
        }
    }

    /// Les absences rangees pour `tache` ; `true` si elles n'ont pas toutes
    /// tenu (ou si une etait encore en cours d'ecriture).
    pub fn lis_absences(&self, tache: u32, mut sortie: impl FnMut(&AbsenceDuFil)) -> bool {
        if !self.arme_pour(tache) {
            return false;
        }
        let n = self.n_absences.load(Ordering::Acquire) as usize;
        let mut incomplet = n > ABSENCES_ENREGISTREES;
        for c in &self.absences[..n.min(ABSENCES_ENREGISTREES)] {
            if c[3].load(Ordering::Acquire) != 1 {
                incomplet = true;
                continue;
            }
            let cause = match c[1].load(Ordering::Relaxed) {
                1 => Cause::Externe,
                2 => Cause::Masquee,
                _ => Cause::Inexpliquee,
            };
            sortie(&AbsenceDuFil { fin_ns: c[0].load(Ordering::Relaxed), cause, part_ns: c[2].load(Ordering::Relaxed) });
        }
        incomplet
    }

    /// Les sommeils ranges pour `tache`, et combien n'ont pas tenu.
    pub fn lis(&self, tache: u32, mut sortie: impl FnMut(&Sommeil)) -> u64 {
        if !self.arme_pour(tache) {
            return 0;
        }
        let n = self.n.load(Ordering::Relaxed) as usize;
        for c in &self.cases[..n.min(SOMMEILS_ENREGISTRES)] {
            sortie(&Sommeil {
                echeance_ns: c[0].load(Ordering::Relaxed),
                entree_ns: c[1].load(Ordering::Relaxed),
                comptes: ReleveFil {
                    externe_ns: c[2].load(Ordering::Relaxed),
                    inexplique_ns: c[3].load(Ordering::Relaxed),
                    masque_ns: c[4].load(Ordering::Relaxed),
                    pret_ns: c[5].load(Ordering::Relaxed),
                    incompletes: 0,
                },
            });
        }
        n.saturating_sub(SOMMEILS_ENREGISTRES) as u64
    }
}

pub static ENREGISTREUR: Enregistreur = Enregistreur::neuf();
