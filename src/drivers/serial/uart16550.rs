//! Pilote série UART 16550 sur COM1 (port 0x3F8).
//!
//! V16.2 garde COM1 comme sortie de diagnostic mais retire son coût du chemin
//! critique autant que possible :
//! - 115200 bauds au lieu de 38400 ;
//! - FIFO 16 octets ;
//! - un tampon de formatage sur pile pour que `write_fmt` n'attende pas THRE à
//!   chaque fragment de `fmt` ;
//! - le préfixe de journal peut être émis d'un seul bloc sans réentrer dans le
//!   formateur série.

use core::fmt;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use crate::arch::x86_64::ports::{inb, outb};

#[path = "sonde.rs"]
pub mod sonde;

/// Verdict de la sonde, encode : 0 bus flottant, 1 muet, 2 present.
///
/// Avant la sonde, la valeur est 2 : tant qu'on n'a pas demande, on ecrit.
/// Un demarrage qui echouerait AVANT `init()` garde ainsi la sortie serie
/// qu'il a toujours eue sous QEMU.
static PRESENCE: AtomicU8 = AtomicU8::new(2);

const COM1: u16 = 0x3F8;
const PROFONDEUR_FIFO: usize = 16;
const FORMAT_BUFFER_SIZE: usize = 2048;

// BOUCHAUD_PHYSICAL_SERIAL_TRACE_V31
// A mini-PC rarely exposes COM1. Keep the serial stream, but mirror the most
// recent bytes into a bounded atomic ring so the GOP diagnostic screen can show
// exactly the same xHCI/SMP markers after ExitBootServices.
// BOUCHAUD_TAMBOUR_SERIE_1MIO_V1
//
// Soixante-quatre kilooctets, c'etait moins qu'UN demarrage. Le scenario
// `run_os_primitives`, sans bureau ni navigateur, produit 75 852 octets de
// journal ; un releve periodique du gestionnaire de fenetres en fait 18 426 a
// lui seul. L'enregistreur de vol ne sait ecrire qu'une fois la cle USB
// enumeree, c'est-a-dire APRES la partie du demarrage qu'on cherche le plus
// souvent a relire -- carte memoire, ACPI, demarrage des coeurs, PCI.
//
// Ce qui manquait aux archives n'etait donc pas de la verbosite : c'etait de
// la PLACE pour la garder jusqu'a ce que quelqu'un sache l'ecrire.
//
// Un mebioctet tient seize demarrages. Il coute un mebioctet de `.bss` sur
// une machine qui en a trente-deux mille, et rien du tout dans l'image : le
// tampon est entierement a zero.
const TRACE_BYTES: usize = 1024 * 1024;
static TRACE_WRITE: AtomicUsize = AtomicUsize::new(0);
static TRACE: [AtomicU8; TRACE_BYTES] = [const { AtomicU8::new(0) }; TRACE_BYTES];

#[inline]
fn trace_capture(bytes: &[u8]) {
    for &byte in bytes {
        let sequence = TRACE_WRITE.fetch_add(1, Ordering::AcqRel);
        TRACE[sequence % TRACE_BYTES].store(byte, Ordering::Release);
    }
}

pub fn trace_total_bytes() -> usize {
    TRACE_WRITE.load(Ordering::Acquire)
}

pub fn trace_snapshot() -> alloc::vec::Vec<u8> {
    let end = TRACE_WRITE.load(Ordering::Acquire);
    let len = end.min(TRACE_BYTES);
    let start = end.saturating_sub(len);
    let mut out = alloc::vec::Vec::with_capacity(len);
    for sequence in start..end {
        out.push(TRACE[sequence % TRACE_BYTES].load(Ordering::Acquire));
    }
    out
}

/// Bornes de la trace : premiere et derniere sequence encore presentes.
///
/// Les sequences sont des compteurs ABSOLUS, pas des indices : elles ne
/// bouclent pas, et l'appelant les convertit avec [`trace_octet`]. Rendre des
/// indices obligerait chaque lecteur a refaire le modulo, et l'un d'eux le
/// referait mal.
pub fn trace_bornes() -> (usize, usize) {
    let fin = TRACE_WRITE.load(Ordering::Acquire);
    (fin.saturating_sub(fin.min(TRACE_BYTES)), fin)
}

/// Un octet de la trace, par sequence absolue.
///
/// # Pourquoi cet acces existe a cote de `trace_snapshot`
///
/// `trace_snapshot` alloue un `Vec` de soixante-quatre kilooctets. C'est le
/// bon outil pour une fenetre du bureau ; c'en est le pire pour un
/// gestionnaire de double faute, ou le tas peut etre precisement ce qui vient
/// d'etre corrompu -- et ou une allocation transformerait un diagnostic en
/// triple faute muette.
///
/// Cet acces-ci ne fait qu'une lecture atomique. Il n'alloue pas, ne prend
/// aucun verrou, et reste utilisable quand plus rien d'autre ne l'est.
pub fn trace_octet(sequence: usize) -> u8 {
    TRACE[sequence % TRACE_BYTES].load(Ordering::Acquire)
}

/// État global du port série, pour éviter d'écrire avant l'init.
static mut INITIALISED: bool = false;

/// Le prochain octet normal écrit commence-t-il une ligne ?
static mut DEBUT_LIGNE: bool = true;
/// Garde de réentrance pendant la génération du préfixe.
static mut DANS_PREFIXE: bool = false;

pub struct SerialPort;
pub struct SerialBrut;

static mut SERIAL_BRUT: SerialBrut = SerialBrut;

/// Initialise COM1 : 115200 bauds, 8N1, FIFO active.
///
/// QEMU émule un 16550 ; un diviseur de 1 est le débit standard maximal du
/// périphérique et divise par trois la durée de vidage par rapport à l'ancien
/// diviseur 3 (38400 bauds).
pub fn init() {
    unsafe {
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x80);
        outb(COM1 + 0, 0x01); // diviseur bas : 1 -> 115200
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x03); // 8N1
        outb(COM1 + 2, 0xC7); // FIFO active, purge, seuil RX 14
        outb(COM1 + 4, 0x0B);
        INITIALISED = true;
    }
    let verdict = sonde_com1();
    PRESENCE.store(verdict as u8, Ordering::Release);
}

/// Interroge COM1 : y a-t-il un 16550 derriere ce port ?
///
/// La sonde laisse le controleur dans l'etat ou elle l'a trouve (MCR a 0x0B,
/// mode normal). Le verdict lui-meme est pris par [`sonde::verdict`], qui ne
/// touche a rien et se verifie sur l'hote.
fn sonde_com1() -> sonde::Presence {
    unsafe {
        let lsr = inb(COM1 + 5);
        let iir = inb(COM1 + 2);
        outb(COM1 + 4, 0x1E); // boucle locale, RTS/DTR/OUT1/OUT2
        outb(COM1, sonde::MOTIF_BOUCLAGE);
        let echo = inb(COM1);
        outb(COM1 + 4, 0x0B); // retour au mode normal
        sonde::verdict(lsr, iir, echo, sonde::MOTIF_BOUCLAGE)
    }
}

/// Ce que la sonde a conclu au demarrage.
pub fn presence_com1() -> sonde::Presence {
    sonde::depuis_octet(PRESENCE.load(Ordering::Acquire))
}

/// Capacite du tambour de trace, en octets.
///
/// L'enregistreur en a besoin pour dire de combien il est en retard AVANT
/// d'avoir perdu quoi que ce soit.
pub fn trace_capacite() -> usize {
    TRACE_BYTES
}

pub fn is_ready() -> bool {
    unsafe { INITIALISED }
}

#[inline]
fn transmit_empty() -> bool {
    unsafe { inb(COM1 + 5) & 0x20 != 0 }
}

#[inline]
fn attends_place() {
    let mut spin = 0u32;
    while !transmit_empty() {
        spin = spin.wrapping_add(1);
        if spin > 100_000 {
            break;
        }
        core::hint::spin_loop();
    }
}

/// BOUCHAUD_SERIE_LENTE_BANC_V1 -- banc seulement (`serie-lente NS`, 0 par
/// defaut) : chaque octet ecrit sur COM1 coute en plus NS nanosecondes
/// d'attente active, pour reproduire sous TCG le cout du port sous KVM (une
/// sortie de VM par octet, quelques microsecondes) et les flots qui le
/// saturent (run 39602362503 : 6 ecrivains, lignes de 1 800 octets).
static SERIE_LENTE_NS_PAR_OCTET: AtomicU64 = AtomicU64::new(0);

pub fn regle_serie_lente(ns_par_octet: u64) {
    SERIE_LENTE_NS_PAR_OCTET.store(ns_par_octet.min(1_000_000), Ordering::Relaxed);
}

fn write_lot(octets: &[u8]) {
    // Capture at the actual UART sink so prefixes and payload remain in the
    // same order as the bytes emitted on COM1.
    trace_capture(octets);

    // LE TAMBOUR N'EST PAS CONDITIONNEL, LE PORT L'EST.
    //
    // Sans COM1, la boucle ci-dessous coute une attente de THRE et seize
    // `outb` par lot de seize octets, pour une destination que personne ne
    // decode. Un releve periodique de dix-huit kilooctets, c'est mille cent
    // cinquante attentes : la verbosite devenait une taxe sur la machine
    // qu'elle devait decrire.
    //
    // Le tambour, lui, est ecrit dans tous les cas. C'est lui que
    // l'enregistreur de vol pose sur la cle USB, et c'est donc lui -- et non
    // COM1 -- qui porte le journal physique.
    if !presence_com1().ecrire() {
        return;
    }

    // BOUCHAUD_TLB_POINT_DE_SERVICE_V1 -- interruptions masquees (releve de
    // l'IRQ du minuteur, panique), chaque octet coute le temps du port et
    // aucun IPI n'est pris : le coeur sert lui-meme, a chaque lot, les
    // shootdowns qui l'attendent. Sinon l'emetteur atteint sa borne de deux
    // secondes et arrete la machine pour une ligne de diagnostic.
    let masquees = !x86_64::instructions::interrupts::are_enabled();
    let lent = SERIE_LENTE_NS_PAR_OCTET.load(Ordering::Relaxed);
    let mut pose = 0usize;
    while pose < octets.len() {
        if masquees {
            crate::arch::x86_64::smp::sert_shootdowns_en_attente();
        }
        attends_place();
        let fin = (pose + PROFONDEUR_FIFO).min(octets.len());
        while pose < fin {
            unsafe { outb(COM1, octets[pose]); }
            pose += 1;
            if lent != 0 {
                let jusqua = crate::kernel::timer::monotonic_ns().saturating_add(lent);
                while crate::kernel::timer::monotonic_ns() < jusqua {
                    core::hint::spin_loop();
                }
            }
        }
    }
}

/// Écrit directement sur COM1 sans déclencher de préfixe de journal.
///
/// Utilisé par `journal::ecris_prefixe`: le préfixe est déjà entièrement
/// construit sur pile, le refaire passer par `ecris_octets` récursivement
/// recréerait précisément le coût que V16.2 veut supprimer.
pub fn ecris_octets_sans_prefixe(octets: &[u8]) {
    if !is_ready() || octets.is_empty() {
        return;
    }
    let mut lot = [0u8; 64];
    super::lots::en_lots(octets, &mut lot, write_lot);
    if let Some(&dernier) = octets.last() {
        unsafe { DEBUT_LIGNE = dernier == b'\n'; }
    }
}

/// Émet des octets normaux, préfixe de journal compris.
pub fn ecris_octets(octets: &[u8]) {
    if !is_ready() || octets.is_empty() {
        return;
    }

    let mut lot = [0u8; 64];
    let mut debut = 0usize;

    for (indice, &byte) in octets.iter().enumerate() {
        unsafe {
            if DEBUT_LIGNE && byte != b'\n' && !DANS_PREFIXE {
                super::lots::en_lots(&octets[debut..indice], &mut lot, write_lot);
                debut = indice;

                DANS_PREFIXE = true;
                crate::kernel::journal::ecris_prefixe();
                DANS_PREFIXE = false;
            }
            DEBUT_LIGNE = byte == b'\n';
        }
    }

    super::lots::en_lots(&octets[debut..], &mut lot, write_lot);
}

// BOUCHAUD_V16_2_SERIAL_FORMAT_BUFFER
//
// `fmt::write` appelle `write_str` plusieurs fois pour une seule ligne
// formatée. L'ancien `SerialPort::write_str` descendait jusqu'au UART à chaque
// fragment ; sous TCG, chaque vérification THRE est un I/O émulé très coûteux.
// Ce tampon sur pile transforme une ligne ordinaire en un ou quelques gros
// envois, sans allocation et sans état global supplémentaire.
struct TamponFormat {
    donnees: [u8; FORMAT_BUFFER_SIZE],
    len: usize,
}

impl TamponFormat {
    const fn neuf() -> Self {
        Self { donnees: [0; FORMAT_BUFFER_SIZE], len: 0 }
    }

    /// Emet le contenu sous le jeton serie, puis vide le tampon.
    fn vide(&mut self) {
        if self.len == 0 {
            return;
        }
        emet(&self.donnees[..self.len]);
        self.len = 0;
    }
}

impl fmt::Write for TamponFormat {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let mut reste = s.as_bytes();
        while !reste.is_empty() {
            if self.len == self.donnees.len() {
                // Ligne plus longue que le tampon : le formatage se fait hors
                // du jeton, le debordement ne doit pas sortir sans lui
                // (BOUCHAUD_SERIE_ECRITURE_ATOMIQUE_V1). La ligne sort en
                // morceaux de 2 Kio, chacun entier.
                self.vide();
            }
            let place = self.donnees.len() - self.len;
            let n = place.min(reste.len());
            self.donnees[self.len..self.len + n].copy_from_slice(&reste[..n]);
            self.len += n;
            reste = &reste[n..];
        }
        Ok(())
    }
}

impl fmt::Write for SerialPort {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        ecris_octets(s.as_bytes());
        Ok(())
    }
}

/// Implémentation derrière `serial_print!` / `serial_println!`.
/// Jeton d'emission d'une ligne serie.
///
/// # Le defaut qu'il corrige
///
/// `_print` formatait dans un tampon local puis le vidait sans aucune
/// serialisation. Deux ecrivains simultanes -- un fil de service et le shell,
/// ou deux coeurs -- entrelacaient leurs octets, prefixe de journal compris :
///
///     raison=pas-de-par[00:21titi:17on-boucha][ 25ud%:  0%:  1
///
/// Une ligne de diagnostic entrelacee est une ligne PERDUE. Elle a fait echouer
/// un garde-fou qui avait pourtant raison, et elle rendrait inexploitable un
/// releve d'essai physique -- ce pour quoi la console serie existe.
///
/// BOUCHAUD_JETON_SERIE_CONTEXTE_V1 -- une ligne s'emet interruptions
/// masquees, jeton tenu par le CONTEXTE qui l'emet.
///
/// BOUCHAUD_JETON_SERIE_PROPRIETAIRE_V1 marquait le jeton du numero de coeur
/// et laissait passer sans attendre tout ecrivain du meme coeur. Faux deux
/// fois :
///
///   * un coeur n'est pas un contexte. Une tache qui tient le jeton peut etre
///     preemptee (fil noyau sur demande ciblee) ; la tache
///     suivante sur ce coeur passait, et la ligne de l'une se melait a celle
///     de l'autre (`CHILD_AFTER_FORK t=164[...] CHILD_AFTER_FORK ...`) ;
///   * meme pour une vraie reentrance d'IRQ, passer sans attendre insere le
///     texte de l'interruption AU MILIEU de la ligne interrompue
///     (`PROCESS_DEATH ... image=/bin[...][SMP-SNAPSHOT]...`).
///
/// Et sa priorite (un booleen) etait rendue par le premier de deux ecrivains
/// prioritaires pendant que le second attendait encore.
///
/// Desormais : interruptions masquees du debut de l'attente a la fin de la
/// ligne. Le detenteur ne peut alors etre ni interrompu ni preempte : une
/// ligne est atomique, et seul un AUTRE coeur peut attendre -- le temps des
/// lignes qui le precedent, borne par une duree (BOUCHAUD_SERIE_ATTENTE_DATEE_V1). Les gros releves ne s'impriment plus depuis le hard
/// IRQ (BOUCHAUD_RELEVES_HORS_IRQ_V1), aucune priorite n'est donc utile. Le
/// jeton garde son coeur (`cpu_index() + 1`) pour une seule chose :
/// reconnaitre une reentrance impossible par construction (exception ou NMI
/// pendant l'emission), la compter (`serie_reentrees`, attendu 0) et emettre
/// quand meme plutot que de s'interbloquer. La panique garde son chemin
/// synchrone distinct (`_print_raw`, sans jeton).
static EMISSION: AtomicUsize = AtomicUsize::new(0);
static EMISSIONS_REENTREES: AtomicU64 = AtomicU64::new(0);
static EMISSIONS_A_LA_BORNE: AtomicU64 = AtomicU64::new(0);
// BOUCHAUD_SERIE_ATTENTE_PROGRES_V1 : les lignes emises sous le jeton (le
// PROGRES que voit un appelant qui attend), et pourquoi chaque emission a la
// borne a renonce.
static EMISSIONS_FAITES: AtomicU64 = AtomicU64::new(0);
static RENONCES_MASQUEES: AtomicU64 = AtomicU64::new(0);
static RENONCES_SANS_PROGRES: AtomicU64 = AtomicU64::new(0);
static RENONCES_PLAFOND: AtomicU64 = AtomicU64::new(0);
static RENONCES_RELANCES: AtomicU64 = AtomicU64::new(0);
static ATTENTE_PIRE_NS: AtomicU64 = AtomicU64::new(0);
static ECHEANCES_RENOUVELEES: AtomicU64 = AtomicU64::new(0);

// BOUCHAUD_SERIE_JAMAIS_SANS_JETON_V1 : hors panique, une ligne sans jeton
// n'est jamais emise -- elle est rangee entiere dans l'anneau de son coeur,
// et le prochain detenteur du jeton la vide avant sa propre ligne.
const DIFFEREES_OCTETS: usize = 4096;
static DIFFEREES: [super::differees::Anneau<DIFFEREES_OCTETS>; crate::arch::x86_64::smp::MAX_CPUS] =
    [const { super::differees::Anneau::neuf() }; crate::arch::x86_64::smp::MAX_CPUS];
static LIGNES_DIFFEREES: AtomicU64 = AtomicU64::new(0);
static LIGNES_PERDUES: AtomicU64 = AtomicU64::new(0);
static LIGNES_VIDANGEES: AtomicU64 = AtomicU64::new(0);
static EN_PANIQUE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// `(differees, perdues faute de place, vidangees)` depuis le demarrage.
pub fn lignes_differees() -> (u64, u64, u64) {
    (
        LIGNES_DIFFEREES.load(Ordering::Relaxed),
        LIGNES_PERDUES.load(Ordering::Relaxed),
        LIGNES_VIDANGEES.load(Ordering::Relaxed),
    )
}

/// Le gestionnaire de panique bascule le journal : desormais la
/// disponibilite prime sur l'integrite (`attente::repli`).
/// `cpu_index() + 1` du coeur qui panique (0 : aucun).
static PANIQUE_COEUR: AtomicUsize = AtomicUsize::new(0);

/// En panique, SEUL le coeur qui panique emet sans jeton : un autre coeur qui
/// journalise pendant ce temps differe sa ligne (il va etre arrete). Avant,
/// tout coeur emettait sans jeton a sa borne -- et une ligne d'un autre coeur
/// passee par le chemin synchrone s'ecrivait EN MEME TEMPS que la vidange de
/// l'anneau par le coeur qui panique (banc run_console_panique.sh SMP4 :
/// `NET_BOOT` mele, par lots, aux lignes de la rafale).
fn panique_ici() -> bool {
    EN_PANIQUE.load(Ordering::Acquire)
        && PANIQUE_COEUR.load(Ordering::Acquire) == crate::arch::x86_64::smp::cpu_index() + 1
}

pub fn passe_en_panique() {
    PANIQUE_COEUR.store(crate::arch::x86_64::smp::cpu_index() + 1, Ordering::SeqCst);
    EN_PANIQUE.store(true, Ordering::SeqCst);
    // BOUCHAUD_CONSOLE_ASYNCHRONE_V1 : les lignes qui attendaient le fil de
    // vidange sortent AVANT le message de panique -- c'est souvent ce qui
    // explique la panique. Sans jeton : la disponibilite prime. Lecteur
    // unique : le fil publie `VIDANGE_EN_COURS` PUIS relit `EN_PANIQUE`, la
    // panique publie `EN_PANIQUE` PUIS relit `VIDANGE_EN_COURS` (SeqCst) --
    // l'un des deux voit l'autre. Un fil qui emet sur un autre coeur finit
    // son enregistrement ; arrete au milieu, on n'attend pas plus de 20 ms
    // (au pire, un morceau de ligne repete).
    let moi = crate::arch::x86_64::smp::cpu_index() + 1;
    // Datee (l'horloge est atomique), avec un plafond de tours si elle est figee.
    let echeance = crate::kernel::timer::monotonic_ns().saturating_add(20_000_000);
    let mut tours = 0u32;
    loop {
        let fil = VIDANGE_EN_COURS.load(Ordering::SeqCst);
        if fil == 0 || fil == moi || tours >= 50_000_000 {
            break;
        }
        tours += 1;
        if tours % 1024 == 0 && crate::kernel::timer::monotonic_ns() > echeance {
            break;
        }
        core::hint::spin_loop();
    }
    // Le jeton, pris et GARDE : plus aucun autre coeur n'ecrit sur le port
    // (son chemin synchrone attend, puis differe). Un detenteur legitime finit
    // d'abord sa ligne (et l'enregistrement entame) ; introuvable en 200 ms
    // (detenteur arrete au milieu d'une ligne) : on vide quand meme -- la
    // disponibilite prime.
    let echeance = crate::kernel::timer::monotonic_ns().saturating_add(200_000_000);
    let mut tours = 0u32;
    loop {
        match EMISSION.compare_exchange(0, moi, Ordering::Acquire, Ordering::Relaxed) {
            Ok(_) => break,
            Err(tenu) if tenu == moi => break,
            Err(_) => {}
        }
        tours += 1;
        if tours >= 50_000_000 || (tours % 1024 == 0 && crate::kernel::timer::monotonic_ns() > echeance) {
            break;
        }
        core::hint::spin_loop();
    }
    vide_console_entiere();
}

/// Vide, sous le jeton, les lignes differees de tous les coeurs (fin
/// d'autorun, avant le bilan). Sans jeton a la borne, elles restent rangees.
pub fn vidange_differees() {
    if is_ready() {
        attend_console_vide(5_000_000_000);
        sous_jeton(&[]);
    }
}

/// Detenteur du jeton (ou panique) : les lignes differees MURES, chacune
/// entiere, la plus ancienne d'abord (BOUCHAUD_CONSOLE_ORDRE_DIFFEREES_V1).
/// Les autres attendent que l'anneau de la console ait atteint leur repere.
fn vide_differees() {
    while vide_differee_mure() {}
}

/// Le repere et l'enregistrement de tete d'un anneau de lignes differees :
/// `(position, longueur, repere)`.
/// `(position, longueur, repere, numero d'ordre)` de la tete d'un anneau de
/// lignes differees.
fn tete_differee(anneau: &super::differees::Anneau<DIFFEREES_OCTETS>) -> Option<(usize, usize, u64, u64)> {
    let (t, n) = anneau.premier()?;
    let mut entete = [0u8; DIFFEREE_REPERE];
    let mut lus = 0usize;
    anneau.lis(t, 0, DIFFEREE_REPERE.min(n), |m| {
        entete[lus..lus + m.len()].copy_from_slice(m);
        lus += m.len();
    });
    let mut repere = [0u8; 8];
    let mut numero = [0u8; 8];
    repere.copy_from_slice(&entete[..8]);
    numero.copy_from_slice(&entete[8..]);
    Some((t, n, u64::from_le_bytes(repere), u64::from_le_bytes(numero)))
}

/// Detenteur du jeton (ou panique) : la ligne differee la plus ancienne dont
/// le repere est atteint par la lecture de l'anneau de la console -- emise
/// entiere, sans son repere, et retiree. `false` s'il n'y en a pas.
fn vide_differee_mure() -> bool {
    let lus = CONSOLE.lus() as u64;
    // BOUCHAUD_CONSOLE_ORDRE_DIFFEREES_V2 : plus petit repere, puis plus
    // petit numero d'ordre -- jamais l'ordre des coeurs.
    let tetes = DIFFEREES
        .iter()
        .enumerate()
        .filter_map(|(cpu, anneau)| tete_differee(anneau).map(|(_, _, repere, numero)| (cpu, repere, numero)));
    let Some(cpu) = super::differees::plus_ancienne_mure(tetes, lus) else { return false };
    let Some((t, n, _, _)) = tete_differee(&DIFFEREES[cpu]) else { return false };
    DIFFEREES[cpu].lis(t, DIFFEREE_REPERE.min(n), n, ecris_octets);
    DIFFEREES[cpu].retire(t, n);
    LIGNES_VIDANGEES.fetch_add(1, Ordering::Relaxed);
    true
}

/// BOUCHAUD_SERIE_JAMAIS_SANS_JETON_V1 -- banc : un detenteur SUSPENDU.
///
/// Prend le jeton comme `sous_jeton`, ecrit la moitie d'une ligne, puis le
/// garde `duree_ms` interruptions masquees sans rien emettre -- ce qu'est,
/// vu de l'invite, un processeur virtuel que l'hote a retire au milieu d'une
/// ligne --, en servant les shootdowns TLB qui l'attendent. Puis il finit sa
/// ligne et rend le jeton. Pendant ce temps, aucune autre ligne ne doit sortir
/// (`tools/ci/run_serie_entrelacement.sh`, mode `suspendu`). Rend `false` si le
/// jeton n'a pas pu etre pris en une seconde.
pub fn banc_detenteur_suspendu(duree_ms: u64) -> bool {
    if !is_ready() {
        return false;
    }
    let ouvertes = x86_64::instructions::interrupts::are_enabled();
    let echeance = crate::kernel::timer::monotonic_ns().saturating_add(BORNE_SANS_PROGRES_NS_BANC);
    loop {
        attend_jeton_libre(echeance);
        x86_64::instructions::interrupts::disable();
        let moi = crate::arch::x86_64::smp::cpu_index() + 1;
        if EMISSION.compare_exchange(0, moi, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            break;
        }
        if ouvertes {
            x86_64::instructions::interrupts::enable();
        }
        if crate::kernel::timer::monotonic_ns() > echeance {
            return false;
        }
    }
    finis_entame();
    vide_differees();
    ecris_octets(b"SERIE_DETENTEUR_SUSPENDU debut ");
    let fin = crate::kernel::timer::monotonic_ns().saturating_add(duree_ms.saturating_mul(1_000_000));
    while crate::kernel::timer::monotonic_ns() < fin {
        crate::arch::x86_64::smp::sert_shootdowns_en_attente();
        core::hint::spin_loop();
    }
    let mut ligne = [0u8; 64];
    let n = {
        use core::fmt::Write;
        let mut t = TamponPile { octets: &mut ligne, n: 0 };
        let _ = write!(t, "et fin, tenu {} ms\n", duree_ms);
        t.n
    };
    ecris_octets(&ligne[..n]);
    EMISSIONS_FAITES.fetch_add(1, Ordering::Relaxed);
    EMISSION.store(0, Ordering::Release);
    if ouvertes {
        x86_64::instructions::interrupts::enable();
    }
    true
}

const BORNE_SANS_PROGRES_NS_BANC: u64 = 1_000_000_000;

/// BOUCHAUD_CONSOLE_VERROU_BORNE_V1 -- banc : prend le verrou de l'anneau de
/// la console et le GARDE `duree_ms` interruptions masquees, comme un
/// detenteur dont le processeur virtuel serait retire par l'hote au milieu
/// d'un ajout. Les autres coeurs ne doivent pas tourner plus de la borne :
/// ils renoncent (`verrou_abandons`) et leurs lignes attendent dans l'anneau
/// de leur coeur, que le fil vide sans ce verrou. Rend `false` si le verrou
/// n'a pas pu etre pris en une seconde.
/// Banc : le verrou de l'anneau est-il tenu, en ce moment, par
/// `banc_verrou_console_suspendu` (P13_CONSOLE_DIFFEREE_CONTRE_PRESSION_V1) ?
static BANC_VERROU_TENU: AtomicBool = AtomicBool::new(false);

pub fn banc_verrou_tenu() -> bool {
    BANC_VERROU_TENU.load(Ordering::Acquire)
}

pub fn banc_verrou_console_suspendu(duree_ms: u64) -> bool {
    let ouvertes = x86_64::instructions::interrupts::are_enabled();
    let echeance = crate::kernel::timer::monotonic_ns().saturating_add(BORNE_SANS_PROGRES_NS_BANC);
    let moi = crate::arch::x86_64::smp::cpu_index() + 1;
    loop {
        x86_64::instructions::interrupts::disable();
        if CONSOLE_VERROU.compare_exchange(0, moi, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            break;
        }
        if ouvertes {
            x86_64::instructions::interrupts::enable();
        }
        if crate::kernel::timer::monotonic_ns() > echeance {
            return false;
        }
        core::hint::spin_loop();
    }
    CONSOLE_VERROU_GEN.fetch_add(1, Ordering::AcqRel);
    BANC_VERROU_TENU.store(true, Ordering::Release);
    let fin = crate::kernel::timer::monotonic_ns().saturating_add(duree_ms.saturating_mul(1_000_000));
    while crate::kernel::timer::monotonic_ns() < fin {
        crate::arch::x86_64::smp::sert_shootdowns_en_attente();
        core::hint::spin_loop();
    }
    BANC_VERROU_TENU.store(false, Ordering::Release);
    CONSOLE_VERROU.store(0, Ordering::Release);
    if ouvertes {
        x86_64::instructions::interrupts::enable();
    }
    true
}

struct TamponPile<'a> {
    octets: &'a mut [u8],
    n: usize,
}

impl fmt::Write for TamponPile<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let b = s.as_bytes();
        let k = b.len().min(self.octets.len() - self.n);
        self.octets[self.n..self.n + k].copy_from_slice(&b[..k]);
        self.n += k;
        Ok(())
    }
}

/// BOUCHAUD_CONSOLE_ORDRE_DIFFEREES_V1 -- UNE LIGNE DIFFEREE NE DOUBLE PAS
/// L'ANNEAU
///
/// Une ligne differee passait devant l'anneau de la console « une fois tous
/// les 32 morceaux » : elle sortait AVANT des lignes plus anciennes du meme
/// ecrivain encore dans l'anneau. Banc panique, rafale de 2 000 lignes, TCG
/// SMP4 : la 1 615 entre la 1 281 et la 1 282 ; banc de saturation (avant
/// la contre-pression) : 92 inversions. Desormais chaque ligne differee porte
/// son REPERE -- la position d'ecriture de l'anneau de la console quand elle
/// a ete differee -- et ne sort que lorsque la lecture de l'anneau l'a
/// atteint : tout ce qui etait range avant elle est sorti, rien de ce qui l'a
/// ete apres. Fusion exacte, en asynchrone, en synchrone et en panique.
/// Repere (position d'ecriture de l'anneau, 8 octets) puis numero d'ordre
/// global (8 octets) : BOUCHAUD_CONSOLE_ORDRE_DIFFEREES_V2.
const DIFFEREE_REPERE: usize = 16;
static DIFFEREE_NUMERO: AtomicU64 = AtomicU64::new(0);

/// Range la ligne dans l'anneau de CE coeur, precedee de son repere.
/// Interruptions masquees : un seul ecrivain par anneau.
fn differe(octets: &[u8]) {
    if octets.is_empty() {
        return;
    }
    let cpu = crate::arch::x86_64::smp::cpu_index().min(DIFFEREES.len() - 1);
    let repere = (CONSOLE.ecrits() as u64).to_le_bytes();
    let numero = DIFFEREE_NUMERO.fetch_add(1, Ordering::AcqRel).to_le_bytes();
    if DIFFEREES[cpu].range_morceaux(DIFFEREE_REPERE + octets.len(), |pose| {
        pose(&repere);
        pose(&numero);
        pose(octets);
    }) {
        LIGNES_DIFFEREES.fetch_add(1, Ordering::Relaxed);
    } else {
        LIGNES_PERDUES.fetch_add(1, Ordering::Relaxed);
    }
}

/// Pourquoi les emissions a la borne ont renonce au jeton :
/// `(contexte masque, sans progres, plafond absolu, relances)`, la pire
/// attente d'un appelant interruptible (ns) et le nombre d'echeances
/// renouvelees parce que d'autres emettaient.
pub fn renonces() -> (u64, u64, u64, u64, u64, u64) {
    (
        RENONCES_MASQUEES.load(Ordering::Relaxed),
        RENONCES_SANS_PROGRES.load(Ordering::Relaxed),
        RENONCES_PLAFOND.load(Ordering::Relaxed),
        RENONCES_RELANCES.load(Ordering::Relaxed),
        ATTENTE_PIRE_NS.load(Ordering::Relaxed),
        ECHEANCES_RENOUVELEES.load(Ordering::Relaxed),
    )
}

/// Lignes emises alors que leur propre coeur tenait deja le jeton (reentrance
/// par exception ou NMI). Attendu : 0.
pub fn emissions_reentrees() -> u64 {
    EMISSIONS_REENTREES.load(Ordering::Relaxed)
}

/// Attentes du jeton allees jusqu'a la borne (ligne emise sans lui).
pub fn emissions_a_la_borne() -> u64 {
    EMISSIONS_A_LA_BORNE.load(Ordering::Relaxed)
}

/// Une emission plus longue que ceci (interruptions masquees) est comptee.
const EMISSION_LONGUE_NS: u64 = 2_000_000;
static EMISSION_PIRE_NS: AtomicU64 = AtomicU64::new(0);
static EMISSIONS_LONGUES: AtomicU64 = AtomicU64::new(0);

/// `(pire duree d'une emission, interruptions masquees, en ns ; nombre
/// d'emissions de plus de 2 ms)` depuis le demarrage.
pub fn emissions_masquees() -> (u64, u64) {
    (EMISSION_PIRE_NS.load(Ordering::Relaxed), EMISSIONS_LONGUES.load(Ordering::Relaxed))
}

/// BOUCHAUD_SERIE_ATTENTE_DATEE_V1 -- faire la queue SANS masquer.
///
/// Le jeton attendait 100 000 `pause` interruptions masquees : 1 a 4 ms. Une
/// ligne de 200 octets coute ~0,6 ms sous KVM (une sortie de VM par octet) et
/// ~17 ms a 115 200 bauds sur la Trigkey : des que deux ou trois coeurs
/// faisaient la queue, la borne expirait et les lignes se melaient octet par
/// octet (run 37667817559, KVM : `origine=\x1b[9h0mt[`, un [PERF-RIP] illisible
/// qui a fait planter profil_rip.py, « Compositor lances : 0 »).
///
/// Un appelant interruptible attend ici que le jeton paraisse libre, sans le
/// prendre et sans rien masquer -- il ne tient rien ; preempte, il reprend sa
/// place --, au plus 1 s (DATEE, pas comptee en tours), avec un plafond de tours
/// pour finir meme si l'horloge n'avance pas encore. `_print` masque ensuite
/// et prend le jeton comme avant : la phase masquee ne couvre plus que la
/// course finale, et la ligne reste atomique.
fn attend_jeton_libre(echeance: u64) {
    let mut tours = 0u32;
    while EMISSION.load(Ordering::Relaxed) != 0 {
        tours = tours.wrapping_add(1);
        if tours > 50_000_000
            || (tours % 256 == 0 && crate::kernel::timer::monotonic_ns() > echeance)
        {
            return;
        }
        core::hint::spin_loop();
    }
}

// L'attente du jeton d'un appelant interruptible -- file non masquee ET
// courses masquees -- est bornee par `super::attente` : une seconde SANS
// PROGRES, renouvelee tant que d'autres emettent, et un plafond absolu
// (BOUCHAUD_SERIE_ATTENTE_PROGRES_V1).

/// Course masquee d'un appelant interruptible, avant de retourner dans la file.
///
/// BOUCHAUD_SERIE_ECRITURE_ATOMIQUE_V1. Quand le jeton parait libre, plusieurs
/// coeurs masquent et courent ; un seul gagne. Les perdants attendaient
/// ensuite, MASQUES, jusqu'a 100 000 tours -- quelques millisecondes -- puis
/// emettaient SANS le jeton. Sous KVM chaque octet sur COM1 est une sortie de
/// VM : une ligne de 2 Kio du gagnant dure plus que cette borne. Le perdant
/// ecrivait alors en meme temps que lui, octet contre octet (run 38482972584 :
/// `pid=1\x1b6[ 90kmi[l...`). Desormais le perdant rouvre les interruptions
/// et refait la queue tant que l'echeance n'est pas passee.
const COURSE_MASQUEE: u32 = 256;

// JAMAIS EN LIGNE. Le tampon de formatage fait 2 Kio : en ligne, il entrait
// dans la trame de CHAQUE appelant de `serial_println!`. Une fois `_print`
// reduit a formater puis `sous_jeton`, LLVM l'a integre partout -- +2 Kio par
// fonction qui journalise, +20 Kio sur la chaine `window_manager::boucle`
// (61 288 a 63 320 octets pour une pile de 61 440 ; 43 272 a 43 880 avant).
// Hors ligne, le tampon n'existe que dans la trame de `_print`, le temps de
// l'appel (tools/verifie-pile-noyau-profondeur.py).
#[inline(never)]
pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;
    if !is_ready() {
        return;
    }

    // Le formatage se fait HORS du jeton, interruptions ouvertes : il peut
    // etre long, et rien ne l'oblige a etre serialise.
    let mut sortie = TamponFormat::neuf();
    let _ = sortie.write_fmt(args);

    sortie.vide();
}

/// BOUCHAUD_SERIE_ECRITURE_ATOMIQUE_V1 -- la sortie d'un `write(2)` userland,
/// ligne par ligne, chaque ligne sous le jeton.
///
/// `console_write` appelait `ecris_octets` SANS le jeton. Ses lots de 64
/// octets se melaient donc aux lignes du noyau emises par un autre coeur :
/// `[LB] SERVICE_CRASH_REQUEST ... pid=1<ESC>...6`, la ligne d'un processus
/// coupee par le `PROCESS_DEATH` du processus qu'il venait de tuer. Le banc
/// des services y lisait « pid ? » -- un rouge que le code ne meritait pas,
/// et un journal qui mentait.
///
/// Le jeton est rendu entre deux lignes : une ligne du noyau passe entre
/// elles, jamais au milieu. Une ligne plus longue que le tampon de formatage
/// est coupee a sa taille, ce qui borne la phase interruptions masquees comme
/// pour `_print`.
pub fn ecris_lignes(octets: &[u8]) {
    if !is_ready() {
        return;
    }
    let mut reste = octets;
    while !reste.is_empty() {
        let n = super::lots::longueur_ligne(reste, FORMAT_BUFFER_SIZE);
        let (ligne, suite) = reste.split_at(n);
        emet(ligne);
        reste = suite;
    }
}

/// Emet `octets` sous le jeton serie : interruptions masquees, jeton tenu par
/// ce contexte du debut a la fin de la ligne (voir EMISSION). Sans le jeton,
/// la ligne est differee entiere -- jamais emise hors panique
/// (BOUCHAUD_SERIE_JAMAIS_SANS_JETON_V1). `octets` vide : vidange seule.
fn sous_jeton(octets: &[u8]) {
    use super::attente::{a_l_echeance, repli, Echeance, Renonce, Repli, BORNE_SANS_PROGRES_NS};
    let ouvertes = x86_64::instructions::interrupts::are_enabled();
    let debut_attente = crate::kernel::timer::monotonic_ns();
    let mut echeance = debut_attente.saturating_add(BORNE_SANS_PROGRES_NS);
    let mut emissions_vues = EMISSIONS_FAITES.load(Ordering::Relaxed);
    let mut renonce: Option<Renonce> = None;
    // Plafond de retours dans la file, pour finir meme si l'horloge n'avance
    // pas encore (meme raison que le plafond de tours de la file).
    let mut relances = 0u32;
    let issue = loop {
        // BOUCHAUD_SERIE_ATTENTE_DATEE_V1 : la file d'attente se fait AVANT de
        // masquer, sans rien tenir (voir `attend_jeton_libre`) ; la phase
        // masquee ne couvre que la course finale.
        if ouvertes {
            attend_jeton_libre(echeance);
        }
        x86_64::instructions::interrupts::disable();
        // Relu a chaque tour : interruptions ouvertes, la tache a pu migrer.
        let moi = crate::arch::x86_64::smp::cpu_index() + 1;
        // Attente BORNEE : le port sert aussi aux paniques et aux
        // interruptions. Un appelant interruptible ne renonce qu'a
        // l'echeance datee : avant, une course perdue le renvoie dans la
        // file (COURSE_MASQUEE). Renoncer ne veut plus dire emettre sans le
        // jeton : voir `attente::repli`.
        let maintenant = crate::kernel::timer::monotonic_ns();
        let mut requeue = ouvertes && relances < 100_000 && maintenant < echeance;
        // BOUCHAUD_SERIE_ATTENTE_PROGRES_V1 : echeance passee. Si d'autres ont
        // emis depuis la precedente, le detenteur n'est pas perdu : nouvelle
        // echeance, l'appelant refait la queue. Sinon (ou au plafond
        // absolu), il renonce.
        if ouvertes && relances < 100_000 && maintenant >= echeance && renonce.is_none() {
            match a_l_echeance(maintenant, debut_attente, emissions_vues, EMISSIONS_FAITES.load(Ordering::Relaxed)) {
                Echeance::Renouvelee { echeance_ns, emissions_vues: vues } => {
                    echeance = echeance_ns;
                    emissions_vues = vues;
                    ECHEANCES_RENOUVELEES.fetch_add(1, Ordering::Relaxed);
                    requeue = true;
                }
                Echeance::Renonce(r) => renonce = Some(r),
            }
        }
        let borne = if requeue { COURSE_MASQUEE } else { 100_000 };
        let mut tours = 0u32;
        let issue = loop {
            match EMISSION.compare_exchange_weak(0, moi, Ordering::Acquire, Ordering::Relaxed) {
                Ok(_) => break Some(true),
                Err(detenteur) if detenteur == moi => {
                    // BOUCHAUD_CONSOLE_ASYNCHRONE_V1 : le fil de vidange tient
                    // le jeton INTERRUPTIONS OUVERTES ; une IRQ qui journalise
                    // sur son coeur est attendue -- sa ligne est differee, et
                    // ce n'est pas une reentrance (attendue nulle).
                    if VIDANGE_EN_COURS.load(Ordering::Relaxed) == moi {
                        CONSOLE_DIFFEREES_VIDANGE.fetch_add(1, Ordering::Relaxed);
                    } else {
                        EMISSIONS_REENTREES.fetch_add(1, Ordering::Relaxed);
                    }
                    break Some(false);
                }
                Err(_) => {}
            }
            // Interruptions masquees : servir soi-meme les shootdowns TLB qui
            // attendent ce coeur, comme pendant l'emission.
            if tours % 64 == 0 {
                crate::arch::x86_64::smp::sert_shootdowns_en_attente();
            }
            tours += 1;
            if tours > borne {
                break None;
            }
            core::hint::spin_loop();
        };
        match issue {
            Some(pris) => break Some(pris),
            // Course perdue, rien n'est tenu : rouvrir et refaire la queue.
            None if requeue => {
                relances += 1;
                x86_64::instructions::interrupts::enable();
                continue;
            }
            None => {
                let cause = match (ouvertes, renonce) {
                    (false, _) => &RENONCES_MASQUEES,
                    (true, Some(Renonce::SansProgres)) => &RENONCES_SANS_PROGRES,
                    (true, Some(Renonce::Plafond)) => &RENONCES_PLAFOND,
                    (true, None) => &RENONCES_RELANCES,
                };
                cause.fetch_add(1, Ordering::Relaxed);
                break None;
            }
        }
    };
    if ouvertes {
        let attente = crate::kernel::timer::monotonic_ns().saturating_sub(debut_attente);
        ATTENTE_PIRE_NS.fetch_max(attente, Ordering::Relaxed);
    }
    // Interruptions masquees jusqu'a la fin : rien ne s'intercale.
    match issue {
        Some(true) => {
            // BOUCHAUD_SERIE_DUREE_MASQUEE_V1 : la ligne s'emet interruptions
            // masquees. A SMP1 rien ne peut la preempter : sa duree est un
            // retard impose a toute tache prete. On la mesure (pire, et
            // combien > 2 ms). Les lignes differees passent AVANT : l'ordre de
            // chaque ecrivain est garde.
            let debut_emission = crate::kernel::timer::monotonic_ns();
            // BOUCHAUD_CONSOLE_ASYNCHRONE_V1 : une ligne de la console entamee
            // par un morceau precedent se termine AVANT tout autre octet.
            finis_entame();
            vide_differees();
            // BOUCHAUD_CONSOLE_ASYNCHRONE_V1 : console coupee ou repliee, les
            // lignes encore rangees passent avant celle-ci. Active, c'est au
            // fil de les emettre (une a la fois, interruptions ouvertes). En
            // panique, c'est au seul coeur qui panique (`passe_en_panique`) :
            // un second lecteur, ici sous le jeton, doublait et melait les
            // lignes (banc run_console_panique.sh SMP4 : 22 doublons, 285
            // lignes abimees).
            if !console_active() && !EN_PANIQUE.load(Ordering::Acquire) {
                vide_console_entiere();
            }
            ecris_octets(octets);
            let duree = crate::kernel::timer::monotonic_ns().saturating_sub(debut_emission);
            EMISSION_PIRE_NS.fetch_max(duree, Ordering::Relaxed);
            if duree > EMISSION_LONGUE_NS {
                EMISSIONS_LONGUES.fetch_add(1, Ordering::Relaxed);
            }
            // BOUCHAUD_CONSOLE_ASYNCHRONE_V1 : ce qui reste synchrone une fois
            // le fil `console` pret (repli, reentrance, console coupee).
            if CONSOLE_FIL_PRET.load(Ordering::Relaxed) && !octets.is_empty() {
                CONSOLE_SYNCHRONES.fetch_add(1, Ordering::Relaxed);
                CONSOLE_SYNCHRONE_PIRE_NS.fetch_max(duree, Ordering::Relaxed);
            }
            EMISSIONS_FAITES.fetch_add(1, Ordering::Relaxed);
            EMISSION.store(0, Ordering::Release);
        }
        // BOUCHAUD_SERIE_JAMAIS_SANS_JETON_V1 : sans le jeton -- borne d'un
        // contexte masque, renoncement d'un appelant interruptible, ou
        // reentrance sur le coeur qui le tient (la ligne en cours serait
        // coupee) --, la ligne n'est JAMAIS emise hors panique.
        Some(false) | None => match repli(panique_ici()) {
            Repli::Differe => differe(octets),
            Repli::EmetSansJeton => {
                if !octets.is_empty() {
                    EMISSIONS_A_LA_BORNE.fetch_add(1, Ordering::Relaxed);
                }
                vide_differees();
                ecris_octets(octets);
            }
        },
    }
    if ouvertes {
        x86_64::instructions::interrupts::enable();
    }
}

// ===========================================================================
// BOUCHAUD_CONSOLE_ASYNCHRONE_V1 -- LA LIGNE ATTEND DANS UN ANNEAU, PAS LE PORT
//
// Une ligne s'emettait sous le jeton INTERRUPTIONS MASQUEES, precedee de la
// vidange des lignes differees de tous les coeurs. Sous KVM imbrique, chaque
// octet ecrit sur COM1 est une sortie de VM ; sur le materiel, 87 us a
// 115 200 bauds. Mesure : `serie_pire_us` 15 a 20 ms par bras de l'A/B KVM
// SMP4 (run 39503091750, c5fce1fd) -- vingt millisecondes sans battement, sans
// IPI, sans election sur le coeur emetteur. C'est la cause mesuree des echecs
// stricts P13 a SMP1/SMP4 (d02e8294 : detection en retard de 4,2 ms, reprise
// en retard de 16,6 ms).
//
// Desormais, hors demarrage, panique et repli :
//   * l'appelant copie sa ligne ENTIERE, prefixe compris (heure de
//     l'evenement), dans un anneau global, sous un verrou tenu le temps d'une
//     copie memoire, interruptions masquees -- quelques centaines de
//     nanosecondes, sans aucun acces au port ;
//   * le fil `console` vide l'anneau par morceaux de 16 octets, chacun sous
//     le jeton, preemption coupee, INTERRUPTIONS OUVERTES -- battements, IPI
//     et reveils precis sont servis pendant l'emission --, jeton rendu et
//     point cooperatif entre deux : une interactive n'attend pas la fin d'une
//     ligne de 2 Kio, et le jeton n'est jamais tenu par une tache sortie du
//     processeur. L'enregistrement de tete est REPRENABLE (`CONSOLE_ENTAME`) :
//     tout detenteur suivant le termine avant d'ecrire quoi que ce soit.
//
// Garanties gardees : une ligne est entiere ou absente (l'enregistrement
// n'est publie que complet, et repris la ou il en etait) ;
// l'ordre est celui des ajouts (un seul anneau, un verrou) ; un seul emetteur
// sur le port (le jeton) ; aucune attente illimitee ni a vide -- anneau
// plein, le producteur emet lui-meme les lignes les plus anciennes (la
// contre-pression du chemin synchrone) ; un appelant synchrone laisse finir
// la ligne du fil preempte sur son coeur ; un contexte qui ne peut pas ceder
// n'attend jamais le fil, il emet ; fil muet plus de deux secondes : repli
// synchrone definitif --; la panique, la double faute, l'arret d'un coeur et
// l'extinction vident l'anneau. Le demarrage, avant le fil, garde le chemin
// synchrone. `console-async on|off` bascule a chaud, pour l'A/B.

const CONSOLE_OCTETS: usize = 65536;
const CONSOLE_HORODATAGE: usize = 8;
const CONSOLE_FIL_MUET_NS: u64 = 2_000_000_000;
const CONSOLE_SOMME_NS: u64 = 50_000_000;
static CONSOLE: super::differees::Anneau<CONSOLE_OCTETS> = super::differees::Anneau::neuf();
/// 0 libre ; `cpu_index() + 1` de l'ajouteur, interruptions masquees.
static CONSOLE_VERROU: AtomicUsize = AtomicUsize::new(0);
/// BOUCHAUD_CONSOLE_VERROU_BORNE_V1 : l'attente du verrou de l'anneau,
/// interruptions masquees, est BORNEE. Un detenteur legitime ne le tient que
/// le temps d'une copie memoire (quelques microsecondes) ; au-dela, il est
/// suspendu (processeur virtuel retire par l'hote, coeur arrete par NMI) et
/// l'attendant renonce : sa ligne va dans l'anneau de son coeur.
const CONSOLE_VERROU_BORNE_NS: u64 = 1_000_000;
/// Un ecrivain qui PEUT CEDER le processeur (tache utilisateur, aucun verrou,
/// interruptions ouvertes) attend interruptions OUVERTES, en cedant, jusqu'a
/// cette borne : renoncer des la milliseconde remplissait l'anneau de 4 Kio
/// de son coeur (banc verrou suspendu 3 s, SMP4 : 145 lignes perdues). A 5 s,
/// une suspension de 12 s perdait encore 417 lignes (TCG SMP4) : passe la
/// borne, tous les suivants renoncent, et leurs lignes de 1 800 octets
/// debordent l'anneau de leur coeur. L'attente ne coute que la latence de
/// l'ecrivain ; la borne reste finie -- un detenteur suspendu plus d'une
/// minute est une machine perdue, pas une contention.
const CONSOLE_VERROU_BORNE_CEDEE_NS: u64 = 60_000_000_000;
/// Incrementee par chaque detenteur, sous le verrou : distingue une detention
/// d'une autre.
static CONSOLE_VERROU_GEN: AtomicU64 = AtomicU64::new(0);
/// La generation dont un attendant a deja renonce : tant que le verrou est
/// encore tenu par CETTE detention, les suivants renoncent sans attendre --
/// une detention suspendue ne coute qu'une seule attente bornee.
static CONSOLE_VERROU_GEN_ABANDONNEE: AtomicU64 = AtomicU64::new(u64::MAX);
/// Idem pour les ecrivains qui cedent (borne longue depassee).
static CONSOLE_VERROU_GEN_ABANDONNEE_CEDEE: AtomicU64 = AtomicU64::new(u64::MAX);
static CONSOLE_VERROU_ABANDONS: AtomicU64 = AtomicU64::new(0);
/// Attente la plus longue INTERRUPTIONS MASQUEES (bornee a la milliseconde).
static CONSOLE_VERROU_ATTENTE_PIRE_NS: AtomicU64 = AtomicU64::new(0);
static CONSOLE_VERROU_CEDEES: AtomicU64 = AtomicU64::new(0);
static CONSOLE_VERROU_CEDEE_PIRE_NS: AtomicU64 = AtomicU64::new(0);
/// Sous `CONSOLE_VERROU` : le dernier octet range etait-il un saut de ligne ?
static mut CONSOLE_DEBUT_LIGNE: bool = true;
static CONSOLE_VOULUE: AtomicBool = AtomicBool::new(true);
static CONSOLE_FIL_PRET: AtomicBool = AtomicBool::new(false);
static CONSOLE_FIL_LANCE: AtomicBool = AtomicBool::new(false);
static CONSOLE_FIL_ENDORMI: AtomicBool = AtomicBool::new(false);
static CONSOLE_REPLI_SECURITE: AtomicBool = AtomicBool::new(false);
static CONSOLE_DERNIER_PROGRES_NS: AtomicU64 = AtomicU64::new(0);
/// BOUCHAUD_CONSOLE_REPLI_EXPLIQUE_V1 : le dernier tour du fil (qu'il ait
/// emis ou non) et l'etat au moment du repli de securite -- un repli n'est
/// jamais silencieux.
static CONSOLE_FIL_VU_NS: AtomicU64 = AtomicU64::new(0);
static CONSOLE_REPLI_MUET_NS: AtomicU64 = AtomicU64::new(0);
static CONSOLE_REPLI_FIL_VU_NS: AtomicU64 = AtomicU64::new(0);
static CONSOLE_REPLI_OCCUPATION: AtomicUsize = AtomicUsize::new(0);
static CONSOLE_REPLI_JETON: AtomicUsize = AtomicUsize::new(0);
static CONSOLE_REPLI_ENDORMI: AtomicBool = AtomicBool::new(false);
static CONSOLE_REPLI_A_NS: AtomicU64 = AtomicU64::new(0);
static CONSOLE_FIL_TACHE: AtomicUsize = AtomicUsize::new(usize::MAX);
/// Ou en est chaque coeur dans le chemin d'un producteur (diagnostic du
/// repli) : 0 hors console, 1 attente du verrou en cedant, 2 attente du
/// verrou masquee, 3 aide a vider (cede), 4 aide a vider (ne cede pas),
/// 5 jeton occupe pendant l'aide, 6 chemin synchrone, 7 tete de la file de
/// place (vide et retente), 8 dans la file de place, pas en tete.
static CONSOLE_PHASE: [AtomicU8; crate::arch::x86_64::smp::MAX_CPUS] =
    [const { AtomicU8::new(0) }; crate::arch::x86_64::smp::MAX_CPUS];
static CONSOLE_REPLI_PHASES: [AtomicU8; 4] = [const { AtomicU8::new(0) }; 4];
/// Ou en est le fil `console` (diagnostic du repli) : 1 vide un morceau,
/// 2 point cooperatif, 3 dort 1 ms (jeton occupe), 4 dort (anneau vide),
/// 5 gele (banc de la vraie panne).
static CONSOLE_FIL_PHASE: AtomicU8 = AtomicU8::new(0);
static CONSOLE_REPLI_FIL_PHASE: AtomicU8 = AtomicU8::new(0);
/// Au repli : `(etat, on_cpu, file)` du fil (`etat_tache_brut`).
static CONSOLE_REPLI_FIL_ETAT: AtomicU8 = AtomicU8::new(9);
static CONSOLE_REPLI_FIL_ON_CPU: AtomicU8 = AtomicU8::new(0xFF);
static CONSOLE_REPLI_FIL_FILE: AtomicU8 = AtomicU8::new(0xFF);
/// Duree REELLE du sommeil d'une milliseconde du fil sur jeton occupe, et
/// d'un tour complet de la boucle interne (pire).
static CONSOLE_FIL_SOMME_JETON_PIRE_NS: AtomicU64 = AtomicU64::new(0);
static CONSOLE_FIL_TOUR_PIRE_NS: AtomicU64 = AtomicU64::new(0);

// BOUCHAUD_CONSOLE_FIL_SECOURS_V1 -- UN FIL VIVANT N'EST PAS UN FIL MORT.
//
// KVM SMP4, verrou de l'anneau tenu 3 s (run 39667691144) : le fil a dormi
// une milliseconde sur le jeton occupe, et ce sommeil a dure 3 001 297 us --
// reveille, il attendait dans la file du coeur que le banc masquait, et un
// fil noyau n'est jamais vole. A 2 s, le chien de garde a pris ce fil vivant
// pour un fil mort : repli synchrone, `synchrones=1582`.
//
// Trois situations, trois reponses :
//   * contention normale -- le fil progresse (un morceau, ou le jeton vu
//     occupe) : rien ;
//   * coeur suspendu -- le fil est PRET, en file sur un coeur qui ne commute
//     plus depuis `SECOURS_SEUIL_NS` : il est reloge sur un coeur vivant
//     (`kernel::task::secours_tache_coeur_muet`) ;
//   * vrai blocage -- le fil ne progresse plus depuis `CONSOLE_FIL_MUET_NS` :
//     soupcon, le fil est reveille ; toujours muet `CONSOLE_FIL_CONFIRMATION_NS`
//     plus tard, repli synchrone definitif. La duree du silence reste bornee
//     (2,5 s), le chien de garde n'est jamais desarme.
//
// Le soupcon distingue aussi une machine entiere suspendue (SMP1, ou tous
// les processeurs virtuels retires) : a la reprise, le premier producteur
// voit un progres vieux de plusieurs secondes ; le fil, pret, progresse dans
// la milliseconde et leve le soupcon.
const CONSOLE_FIL_CONFIRMATION_NS: u64 = 500_000_000;
/// `console-async secours on|off` : le secours, pour l'A/B (actif par defaut).
static CONSOLE_SECOURS_VOULU: AtomicBool = AtomicBool::new(true);
static CONSOLE_FIL_SECOURS: AtomicU64 = AtomicU64::new(0);
/// Silence du fil au moment ou il a ete reloge (pire).
static CONSOLE_FIL_SECOURS_PIRE_NS: AtomicU64 = AtomicU64::new(0);
/// Debut du soupcon en cours (0 : aucun). Leve des que le progres le depasse.
static CONSOLE_SOUPCON_NS: AtomicU64 = AtomicU64::new(0);
static CONSOLE_SOUPCONS: AtomicU64 = AtomicU64::new(0);
/// Au repli : delai entre le soupcon et le repli.
static CONSOLE_REPLI_SOUPCON_NS: AtomicU64 = AtomicU64::new(0);
/// Banc de la vraie panne (`console-async gele-fil APRES DUREE`) : le fil
/// cesse de progresser DUREE ms a partir de l'instant DEBUT.
static CONSOLE_FIL_GEL_DEBUT_NS: AtomicU64 = AtomicU64::new(0);
static CONSOLE_FIL_GEL_DUREE_NS: AtomicU64 = AtomicU64::new(0);
static CONSOLE_FIL_GELS: AtomicU64 = AtomicU64::new(0);

#[inline]
fn phase(p: u8) {
    let cpu = crate::arch::x86_64::smp::cpu_index();
    if cpu < CONSOLE_PHASE.len() {
        CONSOLE_PHASE[cpu].store(p, Ordering::Relaxed);
    }
}
/// Au repli : la tache qui occupait chaque coeur (indices), pour nommer qui
/// empechait le fil de tourner.
static CONSOLE_REPLI_OCCUPANTS: [AtomicUsize; 4] = [const { AtomicUsize::new(usize::MAX) }; 4];
/// Au repli, par coeur : depuis combien de temps l'occupant tient le coeur
/// (ns), son appel systeme en cours, les dernieres adresses vues par le
/// battement (toute, et en mode noyau), la file (interactives, normales).
static CONSOLE_REPLI_TENU_NS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static CONSOLE_REPLI_APPEL: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static CONSOLE_REPLI_RIP: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static CONSOLE_REPLI_RIP_NOYAU: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static CONSOLE_REPLI_FILE: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
/// Au repli, par coeur : IPI de quantum recues, adresse et mode de la
/// derniere, derniere raison de ne pas commuter (`RENONCE_*`).
static CONSOLE_REPLI_IPI: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static CONSOLE_REPLI_IPI_RIP: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static CONSOLE_REPLI_IPI_MODE: [AtomicU8; 4] = [const { AtomicU8::new(0) }; 4];
/// `cpu_index() + 1` du fil de vidange pendant qu'il tient le jeton.
static VIDANGE_EN_COURS: AtomicUsize = AtomicUsize::new(0);
static CONSOLE_ATTENTE: crate::kernel::sync::WaitQueue = crate::kernel::sync::WaitQueue::new();
static DANS_REVEIL_CONSOLE: [AtomicBool; crate::arch::x86_64::smp::MAX_CPUS] =
    [const { AtomicBool::new(false) }; crate::arch::x86_64::smp::MAX_CPUS];

static CONSOLE_LIGNES: AtomicU64 = AtomicU64::new(0);
static CONSOLE_EMISES: AtomicU64 = AtomicU64::new(0);
static CONSOLE_PLEINES: AtomicU64 = AtomicU64::new(0);
static CONSOLE_REPLIS_DIFFERES: AtomicU64 = AtomicU64::new(0);
static CONSOLE_REENTREES: AtomicU64 = AtomicU64::new(0);
static CONSOLE_DIFFEREES_VIDANGE: AtomicU64 = AtomicU64::new(0);
static CONSOLE_REVEILS: AtomicU64 = AtomicU64::new(0);
static CONSOLE_JETON_OCCUPE: AtomicU64 = AtomicU64::new(0);
static CONSOLE_VIDANGES_AIDEES: AtomicU64 = AtomicU64::new(0);
static CONSOLE_LIGNE_PIRE_NS: AtomicU64 = AtomicU64::new(0);
static CONSOLE_MORCEAU_PIRE_NS: AtomicU64 = AtomicU64::new(0);
static CONSOLE_SYNCHRONES: AtomicU64 = AtomicU64::new(0);
static CONSOLE_SYNCHRONE_PIRE_NS: AtomicU64 = AtomicU64::new(0);
static CONSOLE_RETARD_PIRE_NS: AtomicU64 = AtomicU64::new(0);
static CONSOLE_OCCUPATION_PIRE: AtomicUsize = AtomicUsize::new(0);
static CONSOLE_AJOUT_PIRE_NS: AtomicU64 = AtomicU64::new(0);

/// BOUCHAUD_CONSOLE_CONTRE_PRESSION_V1 -- anneau plein : qui peut ceder
/// ATTEND sa place, dans l'ordre d'arrivee (`super::file_place`), sans
/// echeance tant que la console avance ; qui ne le peut pas (interruption,
/// verrou tenu, fil noyau) trouve la reserve d'urgence, que les premiers ne
/// consomment jamais. KVM SMP4, flot sature (run 39602362503) : 135 lignes
/// perdues ; TCG, port a 5 us/octet : 323 -- chaque producteur retentait pour
/// son compte, un autre prenait la place qu'il venait de liberer, il renoncait
/// a une seconde et sa ligne de 1 800 octets debordait l'anneau de 4 Kio de
/// son coeur.
const CONSOLE_RESERVE_URGENCE: usize = 8192;
static CONSOLE_FILE_PLACE: super::file_place::FilePlace = super::file_place::FilePlace::neuve();
/// Les attendants qui ne sont pas en tete dorment ici (2 ms au plus) ; chaque
/// sortie de la file les reveille.
static CONSOLE_PLACE_ATTENTE: crate::kernel::sync::WaitQueue = crate::kernel::sync::WaitQueue::new();
static CONSOLE_CONTRE_PRESSIONS: AtomicU64 = AtomicU64::new(0);
static CONSOLE_CONTRE_PRESSION_PIRE_NS: AtomicU64 = AtomicU64::new(0);
static CONSOLE_CONTRE_PRESSION_TOTALE_NS: AtomicU64 = AtomicU64::new(0);
/// Tete de file figee plus de `CONSOLE_FIL_MUET_NS`, passee (attendu 0).
static CONSOLE_PLACE_SAUTS: AtomicU64 = AtomicU64::new(0);
/// Sorties de la file parce que la console est devenue synchrone (la ligne
/// passe alors par le chemin synchrone, elle n'est pas perdue).
static CONSOLE_PLACE_SYNCHRONES: AtomicU64 = AtomicU64::new(0);
/// Contextes qui ne peuvent pas ceder et ont trouve l'anneau plein, reserve
/// comprise (ils vident eux-memes, puis differeront).
static CONSOLE_URGENCES: AtomicU64 = AtomicU64::new(0);
/// Pourquoi une ligne de la console a fini dans l'anneau de son coeur.
static CONSOLE_DIFFEREES_OCCUPE: AtomicU64 = AtomicU64::new(0);
static CONSOLE_DIFFEREES_URGENCE: AtomicU64 = AtomicU64::new(0);

#[inline]
fn console_active() -> bool {
    CONSOLE_VOULUE.load(Ordering::Acquire)
        && CONSOLE_FIL_PRET.load(Ordering::Acquire)
        && !CONSOLE_REPLI_SECURITE.load(Ordering::Acquire)
        && !EN_PANIQUE.load(Ordering::Acquire)
}

enum RefusConsole {
    Plein,
    Reentree,
    /// Verrou tenu au-dela de `CONSOLE_VERROU_BORNE_NS` (detenteur suspendu).
    Occupe,
}

/// Point d'entree de toute ligne normale (`_print`, `ecris_lignes`).
fn emet(octets: &[u8]) {
    if !octets.is_empty() && console_active() {
        surveille_fil_console();
        if console_active() {
            // BOUCHAUD_CONSOLE_CONTRE_PRESSION_V1 : qui peut ceder laisse la
            // reserve d'urgence, et ne double pas ceux qui attendent deja.
            let cedable = x86_64::instructions::interrupts::are_enabled()
                && crate::kernel::scheduler::preempt::cession_volontaire_sure();
            let essai = if !cedable {
                range_console(octets, 0)
            } else if CONSOLE_FILE_PLACE.vide() {
                range_console(octets, CONSOLE_RESERVE_URGENCE)
            } else {
                Err(RefusConsole::Plein)
            };
            match essai {
                Ok(()) => {
                    reveille_vidange();
                    return;
                }
                // Il attend sa place, dans l'ordre : jamais d'echeance, jamais
                // l'anneau de son coeur. Console devenue synchrone pendant
                // l'attente : le chemin synchrone ci-dessous emet la ligne.
                Err(RefusConsole::Plein) if cedable => {
                    CONSOLE_PLEINES.fetch_add(1, Ordering::Relaxed);
                    match attend_place(octets) {
                        Ok(()) => {
                            reveille_vidange();
                            return;
                        }
                        Err(RefusConsole::Plein) => {}
                        Err(RefusConsole::Reentree) => {
                            CONSOLE_REENTREES.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(RefusConsole::Occupe) => {
                            x86_64::instructions::interrupts::without_interrupts(|| differe(octets));
                            CONSOLE_DIFFEREES_OCCUPE.fetch_add(1, Ordering::Relaxed);
                            CONSOLE_REPLIS_DIFFERES.fetch_add(1, Ordering::Relaxed);
                            reveille_vidange();
                            return;
                        }
                    }
                }
                Err(RefusConsole::Plein) => {
                    CONSOLE_PLEINES.fetch_add(1, Ordering::Relaxed);
                    CONSOLE_URGENCES.fetch_add(1, Ordering::Relaxed);
                    if aide_a_vider(octets) {
                        return;
                    }
                    // Jeton introuvable (le fil, interrompu sur CE coeur au
                    // milieu d'une ligne, ou un detenteur bloque) : la ligne
                    // attend, entiere, dans l'anneau de son coeur.
                    x86_64::instructions::interrupts::without_interrupts(|| differe(octets));
                    CONSOLE_DIFFEREES_URGENCE.fetch_add(1, Ordering::Relaxed);
                    CONSOLE_REPLIS_DIFFERES.fetch_add(1, Ordering::Relaxed);
                    reveille_vidange();
                    return;
                }
                // Le verrou est tenu par CE coeur (exception ou NMI au milieu
                // d'un ajout) : le chemin synchrone differe la ligne.
                Err(RefusConsole::Reentree) => {
                    CONSOLE_REENTREES.fetch_add(1, Ordering::Relaxed);
                }
                // BOUCHAUD_CONSOLE_VERROU_BORNE_V1 : detenteur du verrou
                // suspendu. La ligne attend, entiere, dans l'anneau de son
                // coeur ; le fil la videra (il ne depend pas de ce verrou).
                // Jamais sur le port sans le jeton.
                Err(RefusConsole::Occupe) => {
                    x86_64::instructions::interrupts::without_interrupts(|| differe(octets));
                    CONSOLE_DIFFEREES_OCCUPE.fetch_add(1, Ordering::Relaxed);
                    CONSOLE_REPLIS_DIFFERES.fetch_add(1, Ordering::Relaxed);
                    reveille_vidange();
                    return;
                }
            }
        }
    }
    // BOUCHAUD_CONSOLE_REPLI_COOPERATIF_V1 : en repli (fil muet, ou console
    // coupee pendant que l'anneau se vide), les lignes restees dans l'anneau
    // passent avant -- par morceaux, interruptions OUVERTES, si l'appelant
    // peut ceder. Sinon le chemin synchrone les videra toutes sous le jeton,
    // interruptions masquees (banc verrou suspendu 3 s, SMP2 : 1,26 s).
    if !octets.is_empty() && !console_active() && !EN_PANIQUE.load(Ordering::Acquire) && !CONSOLE.est_vide()
        && crate::kernel::scheduler::preempt::cession_volontaire_sure()
    {
        vide_anneau_en_cedant();
    }
    sous_jeton(octets);
}

/// Range `octets` dans l'anneau, un prefixe a chaque debut de ligne, en UN
/// enregistrement precede de son horodatage. Interruptions masquees le temps
/// de la copie. `reserve` octets doivent rester libres apres lui
/// (BOUCHAUD_CONSOLE_CONTRE_PRESSION_V1 : la reserve d'urgence, laissee par
/// qui peut attendre).
fn range_console(octets: &[u8], reserve: usize) -> Result<(), RefusConsole> {
    // Hors verrou : l'heure, les charges, les FPS (lus comme avant).
    let prefixe = crate::kernel::journal::prefixe();
    // Lu interruptions encore ouvertes : peut-on attendre en cedant ?
    let peut_ceder = crate::kernel::scheduler::preempt::cession_volontaire_sure();
    let ouvertes = x86_64::instructions::interrupts::are_enabled();
    x86_64::instructions::interrupts::disable();
    let debut = crate::kernel::timer::monotonic_ns();
    let mut moi = crate::arch::x86_64::smp::cpu_index() + 1;
    let mut tours = 0u32;
    loop {
        match CONSOLE_VERROU.compare_exchange_weak(0, moi, Ordering::Acquire, Ordering::Relaxed) {
            Ok(_) => break,
            Err(tenu) if tenu == moi => {
                if ouvertes {
                    x86_64::instructions::interrupts::enable();
                }
                return Err(RefusConsole::Reentree);
            }
            Err(_) => {}
        }
        let gen = CONSOLE_VERROU_GEN.load(Ordering::Acquire);
        // BOUCHAUD_CONSOLE_VERROU_BORNE_V1 : qui peut ceder attend
        // interruptions OUVERTES en cedant (borne longue) ; seul l'essai du
        // verrou se fait interruptions masquees.
        if peut_ceder && ouvertes {
            phase(1);
            let attente = crate::kernel::timer::monotonic_ns().saturating_sub(debut);
            if (tours == 0 && CONSOLE_VERROU_GEN_ABANDONNEE_CEDEE.load(Ordering::Acquire) == gen)
                || attente > CONSOLE_VERROU_BORNE_CEDEE_NS
            {
                CONSOLE_VERROU_GEN_ABANDONNEE_CEDEE.store(gen, Ordering::Release);
                CONSOLE_VERROU_ABANDONS.fetch_add(1, Ordering::Relaxed);
                x86_64::instructions::interrupts::enable();
                return Err(RefusConsole::Occupe);
            }
            if tours == 0 {
                CONSOLE_VERROU_CEDEES.fetch_add(1, Ordering::Relaxed);
            }
            tours = tours.wrapping_add(1);
            x86_64::instructions::interrupts::enable();
            // Tous les producteurs peuvent attendre ici pendant que le
            // coeur du fil console est muet. Sans surveillance dans cette
            // boucle, ni le secours ni la confirmation du repli ne sont
            // servis avant la liberation du verrou. Aucun verrou n'est
            // acquis ici, et le contexte cedable a ete verifie plus haut.
            surveille_fil_console();
            if !console_active() {
                return Err(RefusConsole::Plein);
            }
            crate::kernel::task::yield_now();
            x86_64::instructions::interrupts::disable();
            // Elle a pu migrer : le detenteur s'identifie par SON coeur.
            moi = crate::arch::x86_64::smp::cpu_index() + 1;
            continue;
        }
        phase(2);
        // Interruptions masquees : une detention dont un autre a deja
        // renonce ne se re-attend pas ; sinon, au plus la borne.
        let abandonnee = tours == 0 && CONSOLE_VERROU_GEN_ABANDONNEE.load(Ordering::Acquire) == gen;
        if abandonnee
            || (tours % 64 == 63
                && crate::kernel::timer::monotonic_ns().saturating_sub(debut) > CONSOLE_VERROU_BORNE_NS)
        {
            CONSOLE_VERROU_GEN_ABANDONNEE.store(gen, Ordering::Release);
            CONSOLE_VERROU_ABANDONS.fetch_add(1, Ordering::Relaxed);
            if ouvertes {
                x86_64::instructions::interrupts::enable();
            }
            return Err(RefusConsole::Occupe);
        }
        tours = tours.wrapping_add(1);
        if tours % 64 == 0 {
            crate::arch::x86_64::smp::sert_shootdowns_en_attente();
        }
        core::hint::spin_loop();
    }
    CONSOLE_VERROU_GEN.fetch_add(1, Ordering::AcqRel);
    phase(0);
    let attendu = crate::kernel::timer::monotonic_ns().saturating_sub(debut);
    if peut_ceder && ouvertes {
        CONSOLE_VERROU_CEDEE_PIRE_NS.fetch_max(attendu, Ordering::Relaxed);
    } else {
        CONSOLE_VERROU_ATTENTE_PIRE_NS.fetch_max(attendu, Ordering::Relaxed);
    }
    // SAFETY : CONSOLE_DEBUT_LIGNE n'est lu et ecrit que sous CONSOLE_VERROU.
    let debut_ligne = unsafe { CONSOLE_DEBUT_LIGNE };
    let total = CONSOLE_HORODATAGE
        + super::differees::longueur_prefixee(octets, prefixe.octets().len(), debut_ligne);
    let mut suivant = debut_ligne;
    let fait = super::file_place::tient(CONSOLE_OCTETS - CONSOLE.occupation(), total, reserve)
        && CONSOLE.range_morceaux(total, |pose| {
            pose(&debut.to_le_bytes());
            suivant = super::differees::pose_prefixee(octets, prefixe.octets(), debut_ligne, pose);
        });
    if fait {
        unsafe { CONSOLE_DEBUT_LIGNE = suivant; }
        CONSOLE_LIGNES.fetch_add(1, Ordering::Relaxed);
        CONSOLE_OCCUPATION_PIRE.fetch_max(CONSOLE.occupation(), Ordering::Relaxed);
    }
    CONSOLE_VERROU.store(0, Ordering::Release);
    let duree = crate::kernel::timer::monotonic_ns().saturating_sub(debut);
    CONSOLE_AJOUT_PIRE_NS.fetch_max(duree, Ordering::Relaxed);
    if ouvertes {
        x86_64::instructions::interrupts::enable();
    }
    if fait { Ok(()) } else { Err(RefusConsole::Plein) }
}

/// Reveille le fil s'il dort -- seulement d'un contexte qui peut publier une
/// tache (aucun verrou tournant, aucune IRQ, interruptions ouvertes). Les
/// autres comptent sur la borne de sommeil du fil (`CONSOLE_SOMME_NS`).
fn reveille_vidange() {
    // Pendant de la barriere du fil (publier l'ajout, PUIS lire s'il dort).
    core::sync::atomic::fence(Ordering::SeqCst);
    if !CONSOLE_FIL_ENDORMI.load(Ordering::SeqCst) {
        return;
    }
    let cpu = crate::arch::x86_64::smp::cpu_index().min(DANS_REVEIL_CONSOLE.len() - 1);
    if !crate::kernel::scheduler::preempt::cession_volontaire_sure()
        || DANS_REVEIL_CONSOLE[cpu].swap(true, Ordering::AcqRel)
    {
        return;
    }
    CONSOLE_ATTENTE.wake_one();
    CONSOLE_REVEILS.fetch_add(1, Ordering::Relaxed);
    DANS_REVEIL_CONSOLE[cpu].store(false, Ordering::Release);
}

/// Anneau plein : le PRODUCTEUR paie l'emission -- il emet lui-meme les
/// lignes les plus anciennes, sous le jeton, une a la fois, jusqu'a ce que la
/// sienne tienne ; c'est la contre-pression du chemin synchrone, dans le meme
/// ordre. S'il peut ceder, par morceaux avec un point cooperatif entre deux
/// (comme le fil) ; sinon (contexte de demarrage, verrou tenu, IRQ) d'un
/// trait. Attendre le fil ne suffisait pas : a part egale avec 120 processus
/// il n'avancait pas, et un contexte qui ne peut pas ceder rangeait ses lignes
/// dans l'anneau de son coeur (4 Kio) puis les perdait (banc
/// `run_console_panique.sh` : 190 lignes sur 600). Rend `false` si le jeton
/// reste introuvable : tenu par CE coeur sans pouvoir ceder (une IRQ qui a
/// interrompu le detenteur), ou au-dela de l'echeance (1 s interruptions
/// ouvertes ; 20 ms masquees). Depuis BOUCHAUD_CONSOLE_CONTRE_PRESSION_V1, le
/// chemin des seuls contextes qui ne peuvent pas attendre (les autres font la
/// queue dans `attend_place`) ; il dispose de la reserve d'urgence.
fn aide_a_vider(octets: &[u8]) -> bool {
    let peut_ceder = crate::kernel::scheduler::preempt::cession_volontaire_sure();
    // Interruptions ouvertes (contexte de demarrage, verrou dormant) : on peut
    // attendre la place longtemps sans rien retarder ; masquees, 20 ms.
    let ouvertes = x86_64::instructions::interrupts::are_enabled();
    let mut depuis = crate::kernel::timer::monotonic_ns();
    let echeance = depuis.saturating_add(if ouvertes { 1_000_000_000 } else { 20_000_000 });
    loop {
        phase(if peut_ceder { 3 } else { 4 });
        if EN_PANIQUE.load(Ordering::Acquire) || crate::kernel::timer::monotonic_ns() > echeance {
            phase(0);
            return false;
        }
        let emis = match vide_un_morceau() {
            Vidange::Emise | Vidange::Rien => {
                CONSOLE_VIDANGES_AIDEES.fetch_add(1, Ordering::Relaxed);
                true
            }
            Vidange::JetonOccupe => {
                phase(5);
                if peut_ceder {
                    crate::kernel::task::yield_now();
                } else if EMISSION.load(Ordering::Relaxed) == crate::arch::x86_64::smp::cpu_index() + 1 {
                    phase(0);
                    return false;
                } else {
                    core::hint::spin_loop();
                }
                false
            }
        };
        // Retente APRES CHAQUE ESSAI, jeton obtenu ou non : le detenteur du
        // jeton -- le fil, sur un autre coeur -- libere justement de la place.
        // Attendre le jeton seul ne suffisait pas : le fil le reprend sitot
        // rendu, l'appelant tournait 20 ms pour rien puis differait sa ligne,
        // qui doublait ensuite des lignes plus anciennes (banc
        // run_console_panique.sh SMP4 : 543 avant 197).
        // Reserve d'urgence comprise : ce contexte ne peut pas attendre
        // (BOUCHAUD_CONSOLE_CONTRE_PRESSION_V1).
        match range_console(octets, 0) {
            Ok(()) => {
                phase(0);
                reveille_vidange();
                return true;
            }
            // Verrou tenu par ce coeur, ou par un detenteur suspendu
            // (BOUCHAUD_CONSOLE_VERROU_BORNE_V1) : l'appelant differe.
            Err(RefusConsole::Reentree) | Err(RefusConsole::Occupe) => {
                phase(0);
                return false;
            }
            Err(RefusConsole::Plein) => {}
        }
        if emis && peut_ceder {
            crate::kernel::task::point_cooperatif(&mut depuis);
        }
    }
}

/// BOUCHAUD_CONSOLE_CONTRE_PRESSION_V1 : anneau plein -- ou d'autres attendent
/// deja -- pour un producteur qui PEUT CEDER (tache utilisateur, aucun
/// verrou, interruptions ouvertes). Il prend un ticket. A son tour, il vide
/// lui-meme l'anneau par morceaux (sous le jeton, interruptions ouvertes, un
/// point cooperatif entre deux) jusqu'a ce que sa ligne tienne en laissant la
/// reserve d'urgence ; avant, il dort (2 ms au plus, reveille par chaque
/// sortie de la file). Aucune echeance tant que la console est asynchrone :
/// le fil ou la tete font avancer le port, la ligne attend au lieu de se
/// perdre. `Ok` : rangee. `Err(Plein)` : la console est devenue synchrone
/// (repli, arret, panique), l'appelant prend le chemin synchrone.
/// `Err(Occupe)` : verrou de l'anneau tenu au-dela de sa borne.
fn attend_place(octets: &[u8]) -> Result<(), RefusConsole> {
    let debut = crate::kernel::timer::monotonic_ns();
    let ticket = CONSOLE_FILE_PLACE.prend();
    CONSOLE_CONTRE_PRESSIONS.fetch_add(1, Ordering::Relaxed);
    let mut depuis = debut;
    let mut vu = CONSOLE_FILE_PLACE.servi();
    let mut vu_depuis = debut;
    let issue = loop {
        if !console_active() {
            CONSOLE_PLACE_SYNCHRONES.fetch_add(1, Ordering::Relaxed);
            break Err(RefusConsole::Plein);
        }
        surveille_fil_console();
        if CONSOLE_FILE_PLACE.a_son_tour(ticket) {
            phase(7);
            match range_console(octets, CONSOLE_RESERVE_URGENCE) {
                Err(RefusConsole::Plein) => {}
                autre => break autre,
            }
            match vide_un_morceau() {
                Vidange::Emise | Vidange::Rien => {
                    CONSOLE_VIDANGES_AIDEES.fetch_add(1, Ordering::Relaxed);
                    crate::kernel::task::point_cooperatif(&mut depuis);
                }
                // Le fil emet sur un autre coeur : la place vient.
                Vidange::JetonOccupe => crate::kernel::task::yield_now(),
            }
            continue;
        }
        // Une tete figee (sa tache detruite sans sortir de la file) ne bloque
        // pas les suivants au-dela du delai d'un fil muet.
        let maintenant = crate::kernel::timer::monotonic_ns();
        let servi = CONSOLE_FILE_PLACE.servi();
        if servi != vu {
            vu = servi;
            vu_depuis = maintenant;
        } else if maintenant.saturating_sub(vu_depuis) > CONSOLE_FIL_MUET_NS
            && CONSOLE_FILE_PLACE.saute(vu, ticket)
        {
            CONSOLE_PLACE_SAUTS.fetch_add(1, Ordering::Relaxed);
            vu_depuis = maintenant;
            continue;
        }
        phase(8);
        let t = CONSOLE_PLACE_ATTENTE.ticket();
        if !CONSOLE_FILE_PLACE.a_son_tour(ticket) {
            let _ = CONSOLE_PLACE_ATTENTE.wait_until_noyau(t, maintenant.saturating_add(2_000_000));
        }
        depuis = crate::kernel::timer::monotonic_ns();
    };
    // Toute sortie, rangee ou non, fait avancer les suivants.
    phase(0);
    CONSOLE_FILE_PLACE.sort(ticket);
    CONSOLE_PLACE_ATTENTE.wake_all();
    let attendu = crate::kernel::timer::monotonic_ns().saturating_sub(debut);
    CONSOLE_CONTRE_PRESSION_PIRE_NS.fetch_max(attendu, Ordering::Relaxed);
    CONSOLE_CONTRE_PRESSION_TOTALE_NS.fetch_add(attendu, Ordering::Relaxed);
    issue
}

/// Le chien de garde du fil (BOUCHAUD_CONSOLE_FIL_SECOURS_V1) : un fil pret
/// sur un coeur muet est reloge ; un fil muet plus de deux secondes est
/// soupconne, reveille, puis -- toujours muet une demi-seconde plus tard --
/// la console se replie sur le chemin synchrone, et le detenteur suivant du
/// jeton videra l'anneau.
fn surveille_fil_console() {
    if CONSOLE.est_vide() {
        return;
    }
    let maintenant = crate::kernel::timer::monotonic_ns();
    let progres = CONSOLE_DERNIER_PROGRES_NS.load(Ordering::Relaxed);
    if progres == 0 {
        return;
    }
    let muet = maintenant.saturating_sub(progres);
    if muet > crate::kernel::scheduler::secours::SECOURS_SEUIL_NS && CONSOLE_SECOURS_VOULU.load(Ordering::Relaxed) {
        let fil = CONSOLE_FIL_TACHE.load(Ordering::Relaxed);
        if fil != usize::MAX
            && crate::kernel::task::secours_tache_coeur_muet(fil, crate::kernel::scheduler::secours::SECOURS_SEUIL_NS).is_some()
        {
            CONSOLE_FIL_SECOURS.fetch_add(1, Ordering::Relaxed);
            CONSOLE_FIL_SECOURS_PIRE_NS.fetch_max(muet, Ordering::Relaxed);
        }
    }
    if muet <= CONSOLE_FIL_MUET_NS {
        return;
    }
    // Soupcon : aucun en cours, ou le fil a progresse depuis le dernier.
    let soupcon = CONSOLE_SOUPCON_NS.load(Ordering::Acquire);
    if soupcon <= progres {
        if CONSOLE_SOUPCON_NS.compare_exchange(soupcon, maintenant, Ordering::AcqRel, Ordering::Relaxed).is_ok() {
            CONSOLE_SOUPCONS.fetch_add(1, Ordering::Relaxed);
            reveille_vidange();
        }
        return;
    }
    if maintenant.saturating_sub(soupcon) < CONSOLE_FIL_CONFIRMATION_NS {
        return;
    }
    if !CONSOLE_REPLI_SECURITE.swap(true, Ordering::AcqRel) {
        CONSOLE_REPLI_A_NS.store(maintenant, Ordering::Relaxed);
        CONSOLE_REPLI_SOUPCON_NS.store(maintenant.saturating_sub(soupcon), Ordering::Relaxed);
        CONSOLE_REPLI_MUET_NS.store(maintenant.saturating_sub(progres), Ordering::Relaxed);
        CONSOLE_REPLI_FIL_VU_NS.store(
            maintenant.saturating_sub(CONSOLE_FIL_VU_NS.load(Ordering::Relaxed)),
            Ordering::Relaxed,
        );
        CONSOLE_REPLI_OCCUPATION.store(CONSOLE.occupation(), Ordering::Relaxed);
        CONSOLE_REPLI_JETON.store(EMISSION.load(Ordering::Relaxed), Ordering::Relaxed);
        CONSOLE_REPLI_ENDORMI.store(CONSOLE_FIL_ENDORMI.load(Ordering::Relaxed), Ordering::Relaxed);
        CONSOLE_REPLI_FIL_PHASE.store(CONSOLE_FIL_PHASE.load(Ordering::Relaxed), Ordering::Relaxed);
        let (etat, on_cpu, file) = crate::kernel::task::etat_tache_brut(CONSOLE_FIL_TACHE.load(Ordering::Relaxed));
        CONSOLE_REPLI_FIL_ETAT.store(etat, Ordering::Relaxed);
        CONSOLE_REPLI_FIL_ON_CPU.store(on_cpu as u8, Ordering::Relaxed);
        CONSOLE_REPLI_FIL_FILE.store(file, Ordering::Relaxed);
        for (cpu, case) in CONSOLE_REPLI_OCCUPANTS.iter().enumerate() {
            case.store(crate::kernel::task::occupant_coeur(cpu), Ordering::Relaxed);
            let (_, _, depuis) = crate::kernel::task::profil_occupant(cpu);
            CONSOLE_REPLI_TENU_NS[cpu].store(
                if depuis == 0 { 0 } else { maintenant.saturating_sub(depuis) },
                Ordering::Relaxed,
            );
            let (_, appel, _, _, _) = crate::kernel::task::stall_probe_context_pour(cpu);
            CONSOLE_REPLI_APPEL[cpu].store(appel, Ordering::Relaxed);
            let (rip, rip_noyau) = crate::kernel::task::rips_timer(cpu);
            CONSOLE_REPLI_RIP[cpu].store(rip, Ordering::Relaxed);
            CONSOLE_REPLI_RIP_NOYAU[cpu].store(rip_noyau, Ordering::Relaxed);
            let (i, n) = crate::arch::x86_64::cpu_local::CpuId::from_index(cpu)
                .map(|id| crate::arch::x86_64::cpu_local::local(id).attente_file())
                .unwrap_or((0, 0));
            CONSOLE_REPLI_FILE[cpu].store(((i as u64) << 32) | n as u64, Ordering::Relaxed);
            let (ipi, ipi_rip, utilisateur, raison) = crate::kernel::task::dernier_battement(cpu);
            CONSOLE_REPLI_IPI[cpu].store(ipi, Ordering::Relaxed);
            CONSOLE_REPLI_IPI_RIP[cpu].store(ipi_rip, Ordering::Relaxed);
            CONSOLE_REPLI_IPI_MODE[cpu].store(((utilisateur as u8) << 4) | (raison & 0xF), Ordering::Relaxed);
            CONSOLE_REPLI_PHASES[cpu].store(CONSOLE_PHASE[cpu].load(Ordering::Relaxed), Ordering::Relaxed);
        }
    }
}

/// Le fil de vidange : une ligne, interruptions ouvertes, puis la main.
fn fil_console() -> ! {
    CONSOLE_FIL_TACHE.store(crate::kernel::task::indice_tache_courante(), Ordering::Relaxed);
    CONSOLE_DERNIER_PROGRES_NS.store(crate::kernel::timer::monotonic_ns(), Ordering::Relaxed);
    CONSOLE_FIL_PRET.store(true, Ordering::Release);
    let mut depuis = crate::kernel::timer::monotonic_ns();
    let mut precedent = 0u64;
    loop {
        let ticket = CONSOLE_ATTENTE.ticket();
        loop {
            // BOUCHAUD_CONSOLE_FIL_SECOURS_V1 -- banc de la vraie panne : le
            // fil cesse de progresser (`console-async gele-fil`).
            let gel = CONSOLE_FIL_GEL_DEBUT_NS.load(Ordering::Relaxed);
            if gel != 0
                && crate::kernel::timer::monotonic_ns() >= gel
                && CONSOLE_FIL_GEL_DEBUT_NS.compare_exchange(gel, 0, Ordering::AcqRel, Ordering::Relaxed).is_ok()
            {
                gele_fil_pour_banc(CONSOLE_FIL_GEL_DUREE_NS.load(Ordering::Relaxed));
                precedent = 0;
            }
            let tour = crate::kernel::timer::monotonic_ns();
            CONSOLE_FIL_VU_NS.store(tour, Ordering::Relaxed);
            if precedent != 0 {
                CONSOLE_FIL_TOUR_PIRE_NS.fetch_max(tour.saturating_sub(precedent), Ordering::Relaxed);
            }
            precedent = tour;
            CONSOLE_FIL_PHASE.store(1, Ordering::Relaxed);
            match vide_un_morceau() {
                Vidange::Emise => {
                    CONSOLE_FIL_PHASE.store(2, Ordering::Relaxed);
                    crate::kernel::task::point_cooperatif(&mut depuis);
                }
                // Un emetteur synchrone (repli, banc du detenteur suspendu)
                // tient le port : le fil ne l'attend pas en tournant, il dort
                // une milliseconde et laisse le coeur.
                Vidange::JetonOccupe => {
                    CONSOLE_JETON_OCCUPE.fetch_add(1, Ordering::Relaxed);
                    // Vivant, mais le port est pris : pas un fil muet (le
                    // repli synchrone attendrait le meme jeton).
                    CONSOLE_DERNIER_PROGRES_NS.store(crate::kernel::timer::monotonic_ns(), Ordering::Relaxed);
                    let t = CONSOLE_ATTENTE.ticket();
                    let avant = crate::kernel::timer::monotonic_ns();
                    let borne = avant.saturating_add(1_000_000);
                    CONSOLE_FIL_PHASE.store(3, Ordering::Relaxed);
                    let _ = CONSOLE_ATTENTE.wait_until_noyau(t, borne);
                    depuis = crate::kernel::timer::monotonic_ns();
                    CONSOLE_FIL_SOMME_JETON_PIRE_NS.fetch_max(depuis.saturating_sub(avant), Ordering::Relaxed);
                }
                Vidange::Rien => break,
            }
        }
        CONSOLE_DERNIER_PROGRES_NS.store(crate::kernel::timer::monotonic_ns(), Ordering::Relaxed);
        CONSOLE_FIL_ENDORMI.store(true, Ordering::SeqCst);
        // Pendant de `reveille_vidange` : se declarer endormi, PUIS relire.
        core::sync::atomic::fence(Ordering::SeqCst);
        if CONSOLE.est_vide() && differees_vides() {
            let borne = crate::kernel::timer::monotonic_ns().saturating_add(CONSOLE_SOMME_NS);
            CONSOLE_FIL_PHASE.store(4, Ordering::Relaxed);
            let _ = CONSOLE_ATTENTE.wait_until_noyau(ticket, borne);
        }
        // Le sommeil anneau vide ne compte pas comme un tour muet.
        precedent = 0;
        CONSOLE_FIL_ENDORMI.store(false, Ordering::SeqCst);
        depuis = crate::kernel::timer::monotonic_ns();
    }
}

/// Banc de la vraie panne : le fil dort par tranches de 10 ms sans toucher
/// l'anneau ni ses temoins de progres -- vivant pour l'ordonnanceur, muet
/// pour la console. Le chien de garde doit replier, borne.
fn gele_fil_pour_banc(duree_ns: u64) {
    CONSOLE_FIL_GELS.fetch_add(1, Ordering::Relaxed);
    CONSOLE_FIL_PHASE.store(5, Ordering::Relaxed);
    let fin = crate::kernel::timer::monotonic_ns().saturating_add(duree_ns);
    while crate::kernel::timer::monotonic_ns() < fin {
        crate::kernel::task::sleep_ticks(crate::kernel::timer::ms_to_ticks(10).max(1));
    }
}

/// BOUCHAUD_CONSOLE_FIL_SECOURS_V1 -- banc : le coeur dans la file duquel le
/// fil `console` attend son tour (`None` avant le fil).
pub fn coeur_du_fil_console() -> Option<usize> {
    let index = CONSOLE_FIL_TACHE.load(Ordering::Relaxed);
    if index == usize::MAX {
        return None;
    }
    let (etat, _, file) = crate::kernel::task::etat_tache_brut(index);
    if etat == 9 { None } else { Some(file as usize) }
}

/// Banc de la vraie panne : dans `apres_ms`, le fil cesse de progresser
/// `duree_ms`.
pub fn banc_gele_fil_console(apres_ms: u64, duree_ms: u64) {
    CONSOLE_FIL_GEL_DUREE_NS.store(duree_ms.saturating_mul(1_000_000), Ordering::Relaxed);
    let debut = crate::kernel::timer::monotonic_ns().saturating_add(apres_ms.saturating_mul(1_000_000));
    CONSOLE_FIL_GEL_DEBUT_NS.store(debut.max(1), Ordering::Release);
}

/// `console-async secours on|off` (A/B) : relogement du fil pret sur un coeur
/// muet.
pub fn choisit_secours_fil(oui: bool) {
    CONSOLE_SECOURS_VOULU.store(oui, Ordering::Relaxed);
}

fn differees_vides() -> bool {
    DIFFEREES.iter().all(|a| a.est_vide())
}

/// Octets emis d'un seul tenant, jeton tenu, preemption coupee. Sous KVM
/// imbrique, ~3 us par octet (une sortie de VM) ; sur le materiel, 87 us a
/// 115 200 bauds -- seize octets, la FIFO du 16550, font 1,4 ms au pire.
const CONSOLE_MORCEAU: usize = 16;

/// Octets deja emis de l'enregistrement de tete de `CONSOLE` (0 : aucun
/// entame). Lu et ecrit sous le jeton seulement : le detenteur suivant -- le
/// fil, un producteur qui aide, le chemin synchrone, la panique -- reprend LA
/// ou le precedent s'est arrete, avant tout autre octet. Le jeton n'est donc
/// jamais tenu par une tache sortie du processeur, et la ligne reste entiere.
static CONSOLE_ENTAME: AtomicUsize = AtomicUsize::new(0);
/// Debut d'emission de l'enregistrement entame (`ligne_pire_us`).
static CONSOLE_ENTAME_DEPUIS_NS: AtomicU64 = AtomicU64::new(0);

enum Vidange {
    Emise,
    Rien,
    JetonOccupe,
}

/// UN morceau, sous le jeton, preemption coupee, INTERRUPTIONS OUVERTES --
/// battements, IPI et reveils precis sont servis -- puis jeton rendu :
/// l'appelant (le fil, un producteur) passe un point cooperatif entre deux.
/// L'enregistrement entame d'abord ; puis l'anneau ; une ligne differee
/// seulement anneau vide, ou une fois tous les 32 morceaux, et jamais au
/// milieu d'un enregistrement entame. Le jeton n'est essaye que quelques
/// tours : la preemption n'est jamais coupee le temps d'une attente.
fn vide_un_morceau() -> Vidange {
    if CONSOLE.est_vide() && differees_vides() {
        return Vidange::Rien;
    }
    crate::kernel::scheduler::preempt::disable();
    let moi = crate::arch::x86_64::smp::cpu_index() + 1;
    let mut tours = 0u32;
    while EMISSION.compare_exchange_weak(0, moi, Ordering::Acquire, Ordering::Relaxed).is_err() {
        tours += 1;
        if tours > 64 {
            crate::kernel::scheduler::preempt::enable();
            return Vidange::JetonOccupe;
        }
        core::hint::spin_loop();
    }
    VIDANGE_EN_COURS.store(moi, Ordering::SeqCst);
    let mut fait = false;
    if !EN_PANIQUE.load(Ordering::SeqCst) {
        let debut = crate::kernel::timer::monotonic_ns();
        // BOUCHAUD_CONSOLE_ORDRE_DIFFEREES_V1 : une ligne differee passe
        // devant la tete de l'anneau des que la lecture a atteint son repere
        // -- ni avant (elle doublerait des lignes plus anciennes), ni apres
        // (des lignes plus recentes la doubleraient) --, et jamais au milieu
        // d'un enregistrement entame.
        let entame = CONSOLE_ENTAME.load(Ordering::Relaxed) != 0;
        if !entame {
            fait = vide_differee_mure();
        }
        if !fait {
            fait = avance_tete(CONSOLE_MORCEAU, debut).is_some();
        }
        let fin = crate::kernel::timer::monotonic_ns();
        CONSOLE_MORCEAU_PIRE_NS.fetch_max(fin.saturating_sub(debut), Ordering::Relaxed);
        CONSOLE_DERNIER_PROGRES_NS.store(fin, Ordering::Relaxed);
    }
    VIDANGE_EN_COURS.store(0, Ordering::SeqCst);
    // Progres vu des emetteurs synchrones : le jeton a ete rendu.
    EMISSIONS_FAITES.fetch_add(1, Ordering::Relaxed);
    EMISSION.store(0, Ordering::Release);
    crate::kernel::scheduler::preempt::enable();
    if fait { Vidange::Emise } else { Vidange::Rien }
}

/// Sous le jeton (ou en panique) : au plus `budget` octets de
/// l'enregistrement de tete, repris a `CONSOLE_ENTAME` (l'horodatage de 8
/// octets n'est jamais emis), et retire s'il est fini. `Some(fini)` s'il y en
/// avait un, `None` si l'anneau est vide.
fn avance_tete(budget: usize, maintenant: u64) -> Option<bool> {
    let (t, n) = CONSOLE.premier()?;
    let deja = CONSOLE_ENTAME.load(Ordering::Relaxed);
    if deja == 0 {
        CONSOLE_ENTAME_DEPUIS_NS.store(maintenant, Ordering::Relaxed);
    }
    let de = deja.max(CONSOLE_HORODATAGE).min(n);
    let a = de.saturating_add(budget).min(n);
    CONSOLE.lis(t, de, a, ecris_octets_sans_prefixe);
    if a < n {
        CONSOLE_ENTAME.store(a, Ordering::Relaxed);
        return Some(false);
    }
    let mut horodatage = [0u8; CONSOLE_HORODATAGE];
    let mut lus = 0usize;
    CONSOLE.lis(t, 0, CONSOLE_HORODATAGE.min(n), |m| {
        horodatage[lus..lus + m.len()].copy_from_slice(m);
        lus += m.len();
    });
    CONSOLE.retire(t, n);
    CONSOLE_ENTAME.store(0, Ordering::Relaxed);
    CONSOLE_EMISES.fetch_add(1, Ordering::Relaxed);
    let fin = crate::kernel::timer::monotonic_ns();
    CONSOLE_RETARD_PIRE_NS.fetch_max(fin.saturating_sub(u64::from_le_bytes(horodatage)), Ordering::Relaxed);
    CONSOLE_LIGNE_PIRE_NS.fetch_max(
        fin.saturating_sub(CONSOLE_ENTAME_DEPUIS_NS.load(Ordering::Relaxed)),
        Ordering::Relaxed,
    );
    Some(true)
}

/// Detenteur du jeton : termine l'enregistrement entame par un morceau
/// precedent, s'il y en a un -- avant tout autre octet sur le port.
fn finis_entame() {
    // Toujours, meme si la panique vient de commencer : l'appelant TIENT le
    // jeton, le coeur qui panique l'attend (`passe_en_panique`). L'ecarter ici
    // faisait ecrire la ligne de ce detenteur au milieu de l'enregistrement
    // entame (banc SMP4 : `DHCP_RX_ANNEAU` dans la rafale 238).
    if CONSOLE_ENTAME.load(Ordering::Relaxed) != 0 {
        let _ = avance_tete(usize::MAX, crate::kernel::timer::monotonic_ns());
    }
}

/// Tout l'anneau, d'un coup, fusionne avec les lignes differees par leur
/// repere (BOUCHAUD_CONSOLE_ORDRE_DIFFEREES_V1) : detenteur synchrone du
/// jeton (console coupee ou repliee) et panique (sans jeton).
fn vide_console_entiere() {
    let maintenant = crate::kernel::timer::monotonic_ns();
    loop {
        if CONSOLE_ENTAME.load(Ordering::Relaxed) == 0 && vide_differee_mure() {
            continue;
        }
        if avance_tete(usize::MAX, maintenant).is_none() {
            break;
        }
    }
}

/// Attend (au plus `borne_ns`) que le fil ait tout emis : fin d'autorun, avant
/// le bilan, et bascule `console-async off`.
fn attend_console_vide(borne_ns: u64) {
    if !console_active() {
        return;
    }
    let echeance = crate::kernel::timer::monotonic_ns().saturating_add(borne_ns);
    let peut_ceder = crate::kernel::scheduler::preempt::cession_volontaire_sure();
    while !(CONSOLE.est_vide() && differees_vides()) && crate::kernel::timer::monotonic_ns() < echeance {
        if peut_ceder {
            reveille_vidange();
            crate::kernel::task::yield_now();
            continue;
        }
        // Ne peut pas ceder (contexte de demarrage de l'autorun, IRQ) : sur un
        // seul coeur, le fil ne tournerait jamais pendant cette attente -- elle
        // tournait a vide 5 s et declenchait le repli (SMP1). L'appelant emet
        // lui-meme, sous le jeton ; s'il le tient deja sur ce coeur (le fil,
        // interrompu), il ne peut rien.
        match vide_un_morceau() {
            Vidange::Emise | Vidange::Rien => {}
            Vidange::JetonOccupe => {
                if EMISSION.load(Ordering::Relaxed) == crate::arch::x86_64::smp::cpu_index() + 1 {
                    return;
                }
                core::hint::spin_loop();
            }
        }
    }
}

/// Lance le fil `console`. Idempotent ; sans lui, tout reste synchrone.
pub fn demarre_console_asynchrone() -> bool {
    if CONSOLE_FIL_LANCE.swap(true, Ordering::AcqRel) {
        return true;
    }
    if crate::kernel::task::spawn_noyau(fil_console, "console") {
        return true;
    }
    CONSOLE_FIL_LANCE.store(false, Ordering::Release);
    false
}

/// Extinction, redemarrage : tout ce que le fil n'a pas encore emis sort, dans
/// l'ordre, puis la console reste synchrone jusqu'au bout (les dernieres
/// lignes avant de couper le courant ne dependent d'aucun fil).
pub fn arrete_console_asynchrone() {
    attend_console_vide(2_000_000_000);
    CONSOLE_VOULUE.store(false, Ordering::Release);
    if is_ready() {
        // Console desormais inactive : le detenteur vide l'anneau avant de
        // rendre le jeton (`sous_jeton`).
        sous_jeton(&[]);
    }
}

/// Un coeur qui s'arrete pour toujours (faute fatale hors panique) : les
/// lignes en attente sortent sous le jeton -- sur un seul coeur, plus
/// personne ne les emettrait. Pendant une panique, c'est elle qui vide.
pub fn vide_console_avant_arret() {
    if EN_PANIQUE.load(Ordering::SeqCst) || !is_ready() {
        return;
    }
    let echeance = crate::kernel::timer::monotonic_ns().saturating_add(1_000_000_000);
    for _ in 0..(CONSOLE_OCTETS + 4096 * crate::arch::x86_64::smp::MAX_CPUS) {
        match vide_un_morceau() {
            Vidange::Emise => {}
            Vidange::Rien => return,
            Vidange::JetonOccupe => {
                if crate::kernel::timer::monotonic_ns() > echeance {
                    return;
                }
                core::hint::spin_loop();
            }
        }
    }
}

/// BOUCHAUD_CONSOLE_REPLI_COOPERATIF_V1 : vide l'anneau par morceaux, sous le
/// jeton, interruptions ouvertes, un point cooperatif entre deux -- pour un
/// appelant qui peut ceder. Borne : 2 s.
fn vide_anneau_en_cedant() {
    let mut depuis = crate::kernel::timer::monotonic_ns();
    let echeance = depuis.saturating_add(2_000_000_000);
    while crate::kernel::timer::monotonic_ns() < echeance && !EN_PANIQUE.load(Ordering::Acquire) {
        match vide_un_morceau() {
            Vidange::Emise => {
                crate::kernel::task::point_cooperatif(&mut depuis);
            }
            Vidange::Rien => return,
            Vidange::JetonOccupe => crate::kernel::task::yield_now(),
        }
    }
}

/// `console-async on|off` : bascule a chaud (A/B). Couper attend que l'anneau
/// soit vide ; les lignes suivantes sortent par le chemin synchrone.
pub fn choisit_console_asynchrone(oui: bool) {
    if oui {
        // `on` explicite : un repli de securite anterieur est leve. Si le fil
        // est vraiment muet, `surveille_fil_console` le reposera dans 2 s.
        CONSOLE_DERNIER_PROGRES_NS.store(crate::kernel::timer::monotonic_ns(), Ordering::Relaxed);
        CONSOLE_REPLI_SECURITE.store(false, Ordering::Release);
    } else {
        attend_console_vide(5_000_000_000);
    }
    CONSOLE_VOULUE.store(oui, Ordering::Release);
}

/// La console est-elle asynchrone en ce moment (voulue, fil pret, pas de repli) ?
pub fn console_asynchrone_active() -> bool {
    console_active()
}

/// Compteurs de la console asynchrone, publies par le bilan d'autorun.
pub struct CompteursConsole {
    pub active: bool,
    pub repli_securite: bool,
    pub lignes: u64,
    pub emises: u64,
    pub pleines: u64,
    pub replis_differes: u64,
    pub reentrees: u64,
    pub differees_pendant_vidange: u64,
    pub reveils: u64,
    pub jeton_occupe: u64,
    pub vidanges_aidees: u64,
    pub ligne_pire_ns: u64,
    pub morceau_pire_ns: u64,
    pub synchrones: u64,
    pub synchrone_pire_ns: u64,
    pub retard_pire_ns: u64,
    pub ajout_pire_ns: u64,
    pub occupation_pire: usize,
    pub verrou_abandons: u64,
    pub verrou_attente_pire_ns: u64,
    pub verrou_cedees: u64,
    pub verrou_cedee_pire_ns: u64,
}

/// `BOUCHAUD_CONSOLE_BILAN` : une ligne, lue par les bancs (fin d'autorun,
/// `console-async etat`).
pub fn journalise_bilan_console() {
    let c = compteurs_console();
    // BOUCHAUD_CONSOLE_CONTRE_PRESSION_V1 : le cout de l'attente (producteurs
    // qui ont fait la queue, attente pire et moyenne) et chaque chemin qui
    // peut finir dans l'anneau d'un coeur. Avant le bilan : les bancs
    // arretent la machine deux secondes apres lui.
    let attentes = CONSOLE_CONTRE_PRESSIONS.load(Ordering::Relaxed);
    // BOUCHAUD_VGA_TEXTE_MESURE_V1 : la copie texte d'un `write(2)` et de
    // chaque `println!`, interruptions masquees.
    let (prises, tenu, attente, abandons, recopie) = crate::drivers::vga::mesures_console();
    crate::serial_println!(
        "BOUCHAUD_VGA_TEXTE prises={} tenu_pire_us={} attente_pire_us={} abandons={} recopie_pire_us={}",
        prises, tenu / 1000, attente / 1000, abandons, recopie / 1000,
    );
    crate::serial_println!(
        "BOUCHAUD_CONSOLE_CONTRE_PRESSION attentes={} attente_pire_us={} attente_moyenne_us={} sauts={} \
         sorties_synchrones={} urgences={} differees_occupe={} differees_urgence={} reserve={} \
         fil_tour_pire_us={} fil_somme_jeton_pire_us={}",
        attentes,
        CONSOLE_CONTRE_PRESSION_PIRE_NS.load(Ordering::Relaxed) / 1000,
        CONSOLE_CONTRE_PRESSION_TOTALE_NS.load(Ordering::Relaxed) / 1000 / attentes.max(1),
        CONSOLE_PLACE_SAUTS.load(Ordering::Relaxed),
        CONSOLE_PLACE_SYNCHRONES.load(Ordering::Relaxed),
        CONSOLE_URGENCES.load(Ordering::Relaxed),
        CONSOLE_DIFFEREES_OCCUPE.load(Ordering::Relaxed),
        CONSOLE_DIFFEREES_URGENCE.load(Ordering::Relaxed),
        CONSOLE_RESERVE_URGENCE,
        CONSOLE_FIL_TOUR_PIRE_NS.load(Ordering::Relaxed) / 1000,
        CONSOLE_FIL_SOMME_JETON_PIRE_NS.load(Ordering::Relaxed) / 1000,
    );
    crate::serial_println!(
        "BOUCHAUD_CONSOLE_BILAN active={} repli_securite={} lignes={} emises={} pleines={} \
         replis_differes={} vidanges_aidees={} reentrees={} differees_pendant_vidange={} reveils={} jeton_occupe={} ligne_pire_us={} \
         morceau_pire_us={} retard_pire_us={} ajout_pire_us={} occupation_pire={} synchrones={} synchrone_pire_us={} \
         verrou_abandons={} verrou_cedees={} verrou_cedee_pire_us={} verrou_attente_pire_us={} \
         repli_a_ms={} repli_muet_ms={} repli_fil_vu_ms={} repli_occupation={} repli_jeton={} repli_fil_endormi={} \
         secours_voulu={} secours={} secours_pire_ms={} relogees={} refus_entree={} soupcons={} repli_soupcon_ms={} gels={}",
        c.active as u8, c.repli_securite as u8, c.lignes, c.emises, c.pleines,
        c.replis_differes, c.vidanges_aidees, c.reentrees, c.differees_pendant_vidange, c.reveils, c.jeton_occupe,
        c.ligne_pire_ns / 1000, c.morceau_pire_ns / 1000, c.retard_pire_ns / 1000, c.ajout_pire_ns / 1000, c.occupation_pire,
        c.synchrones, c.synchrone_pire_ns / 1000,
        c.verrou_abandons, c.verrou_cedees, c.verrou_cedee_pire_ns / 1000, c.verrou_attente_pire_ns / 1000,
        CONSOLE_REPLI_A_NS.load(Ordering::Relaxed) / 1_000_000,
        CONSOLE_REPLI_MUET_NS.load(Ordering::Relaxed) / 1_000_000,
        CONSOLE_REPLI_FIL_VU_NS.load(Ordering::Relaxed) / 1_000_000,
        CONSOLE_REPLI_OCCUPATION.load(Ordering::Relaxed),
        CONSOLE_REPLI_JETON.load(Ordering::Relaxed),
        CONSOLE_REPLI_ENDORMI.load(Ordering::Relaxed) as u8,
        CONSOLE_SECOURS_VOULU.load(Ordering::Relaxed) as u8,
        CONSOLE_FIL_SECOURS.load(Ordering::Relaxed),
        CONSOLE_FIL_SECOURS_PIRE_NS.load(Ordering::Relaxed) / 1_000_000,
        crate::kernel::task::compteurs_secours_coeur_muet().0,
        crate::kernel::task::compteurs_secours_coeur_muet().1,
        CONSOLE_SOUPCONS.load(Ordering::Relaxed),
        CONSOLE_REPLI_SOUPCON_NS.load(Ordering::Relaxed) / 1_000_000,
        CONSOLE_FIL_GELS.load(Ordering::Relaxed),
    );
    if CONSOLE_REPLI_A_NS.load(Ordering::Relaxed) != 0 {
        crate::serial_println!(
            "BOUCHAUD_CONSOLE_REPLI phases={}{}{}{} fil_phase={} fil_etat={} fil_on_cpu={} fil_file={} \
             fil=[{}] occupants=[{}] [{}] [{}] [{}]",
            CONSOLE_REPLI_PHASES[0].load(Ordering::Relaxed),
            CONSOLE_REPLI_PHASES[1].load(Ordering::Relaxed),
            CONSOLE_REPLI_PHASES[2].load(Ordering::Relaxed),
            CONSOLE_REPLI_PHASES[3].load(Ordering::Relaxed),
            CONSOLE_REPLI_FIL_PHASE.load(Ordering::Relaxed),
            CONSOLE_REPLI_FIL_ETAT.load(Ordering::Relaxed),
            CONSOLE_REPLI_FIL_ON_CPU.load(Ordering::Relaxed) as i8,
            CONSOLE_REPLI_FIL_FILE.load(Ordering::Relaxed),
            crate::kernel::task::decrit_tache(CONSOLE_FIL_TACHE.load(Ordering::Relaxed)),
            crate::kernel::task::decrit_tache(CONSOLE_REPLI_OCCUPANTS[0].load(Ordering::Relaxed)),
            crate::kernel::task::decrit_tache(CONSOLE_REPLI_OCCUPANTS[1].load(Ordering::Relaxed)),
            crate::kernel::task::decrit_tache(CONSOLE_REPLI_OCCUPANTS[2].load(Ordering::Relaxed)),
            crate::kernel::task::decrit_tache(CONSOLE_REPLI_OCCUPANTS[3].load(Ordering::Relaxed)),
        );
        for cpu in 0..4 {
            let file = CONSOLE_REPLI_FILE[cpu].load(Ordering::Relaxed);
            crate::serial_println!(
                "BOUCHAUD_CONSOLE_REPLI_COEUR cpu={} tenu_ms={} appel={} rip={:#x} rip_noyau={:#x} file_interactives={} file_normales={} \
                 ipi={} ipi_rip={:#x} ipi_utilisateur={} renonce={}",
                cpu,
                CONSOLE_REPLI_TENU_NS[cpu].load(Ordering::Relaxed) / 1_000_000,
                CONSOLE_REPLI_APPEL[cpu].load(Ordering::Relaxed) as i64,
                CONSOLE_REPLI_RIP[cpu].load(Ordering::Relaxed),
                CONSOLE_REPLI_RIP_NOYAU[cpu].load(Ordering::Relaxed),
                file >> 32,
                file & 0xFFFF_FFFF,
                CONSOLE_REPLI_IPI[cpu].load(Ordering::Relaxed),
                CONSOLE_REPLI_IPI_RIP[cpu].load(Ordering::Relaxed),
                CONSOLE_REPLI_IPI_MODE[cpu].load(Ordering::Relaxed) >> 4,
                CONSOLE_REPLI_IPI_MODE[cpu].load(Ordering::Relaxed) & 0xF,
            );
        }
    }
}

pub fn compteurs_console() -> CompteursConsole {
    CompteursConsole {
        active: console_active(),
        repli_securite: CONSOLE_REPLI_SECURITE.load(Ordering::Relaxed),
        lignes: CONSOLE_LIGNES.load(Ordering::Relaxed),
        emises: CONSOLE_EMISES.load(Ordering::Relaxed),
        pleines: CONSOLE_PLEINES.load(Ordering::Relaxed),
        replis_differes: CONSOLE_REPLIS_DIFFERES.load(Ordering::Relaxed),
        reentrees: CONSOLE_REENTREES.load(Ordering::Relaxed),
        differees_pendant_vidange: CONSOLE_DIFFEREES_VIDANGE.load(Ordering::Relaxed),
        reveils: CONSOLE_REVEILS.load(Ordering::Relaxed),
        jeton_occupe: CONSOLE_JETON_OCCUPE.load(Ordering::Relaxed),
        vidanges_aidees: CONSOLE_VIDANGES_AIDEES.load(Ordering::Relaxed),
        ligne_pire_ns: CONSOLE_LIGNE_PIRE_NS.load(Ordering::Relaxed),
        morceau_pire_ns: CONSOLE_MORCEAU_PIRE_NS.load(Ordering::Relaxed),
        synchrones: CONSOLE_SYNCHRONES.load(Ordering::Relaxed),
        synchrone_pire_ns: CONSOLE_SYNCHRONE_PIRE_NS.load(Ordering::Relaxed),
        retard_pire_ns: CONSOLE_RETARD_PIRE_NS.load(Ordering::Relaxed),
        ajout_pire_ns: CONSOLE_AJOUT_PIRE_NS.load(Ordering::Relaxed),
        occupation_pire: CONSOLE_OCCUPATION_PIRE.load(Ordering::Relaxed),
        verrou_abandons: CONSOLE_VERROU_ABANDONS.load(Ordering::Relaxed),
        verrou_attente_pire_ns: CONSOLE_VERROU_ATTENTE_PIRE_NS.load(Ordering::Relaxed),
        verrou_cedees: CONSOLE_VERROU_CEDEES.load(Ordering::Relaxed),
        verrou_cedee_pire_ns: CONSOLE_VERROU_CEDEE_PIRE_NS.load(Ordering::Relaxed),
    }
}

impl fmt::Write for SerialBrut {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        ecris_octets_sans_prefixe(s.as_bytes());
        Ok(())
    }
}

/// Sortie brute de forensic/panique : aucun préfixe.
///
/// Le chemin de panique privilégie la simplicité : il ne dépend ni du journal
/// ni du tampon de formatage normal.
pub fn _print_raw(args: fmt::Arguments) {
    use core::fmt::Write;
    if !is_ready() {
        return;
    }
    unsafe {
        let _ = SERIAL_BRUT.write_fmt(args);
    }
}

#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => {{
        $crate::drivers::serial::_print(format_args!($($arg)*))
    }};
}

#[macro_export]
macro_rules! serial_print_brut {
    ($($arg:tt)*) => {{
        $crate::drivers::serial::_print_raw(format_args!($($arg)*))
    }};
}

#[macro_export]
macro_rules! serial_println_brut {
    () => {{ $crate::serial_print_brut!("\n") }};
    ($fmt:expr) => {{ $crate::serial_print_brut!(concat!($fmt, "\n")) }};
    ($fmt:expr, $($arg:tt)*) => {{
        $crate::serial_print_brut!(concat!($fmt, "\n"), $($arg)*)
    }};
}

#[macro_export]
macro_rules! serial_println {
    () => {{ $crate::serial_print!("\n") }};
    ($fmt:expr) => {{ $crate::serial_print!(concat!($fmt, "\n")) }};
    ($fmt:expr, $($arg:tt)*) => {{
        $crate::serial_print!(concat!($fmt, "\n"), $($arg)*)
    }};
}
