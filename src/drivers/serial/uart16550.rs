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
use core::sync::atomic::{AtomicU64, AtomicU8, AtomicUsize, Ordering};
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

    fn vide(&mut self) {
        if self.len == 0 {
            return;
        }
        ecris_octets(&self.donnees[..self.len]);
        self.len = 0;
    }
}

impl fmt::Write for TamponFormat {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let mut reste = s.as_bytes();
        while !reste.is_empty() {
            if self.len == self.donnees.len() {
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

/// Lignes emises alors que leur propre coeur tenait deja le jeton (reentrance
/// par exception ou NMI). Attendu : 0.
pub fn emissions_reentrees() -> u64 {
    EMISSIONS_REENTREES.load(Ordering::Relaxed)
}

/// Attentes du jeton allees jusqu'a la borne (ligne emise sans lui).
pub fn emissions_a_la_borne() -> u64 {
    EMISSIONS_A_LA_BORNE.load(Ordering::Relaxed)
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
fn attend_jeton_libre() {
    const BORNE_NS: u64 = 1_000_000_000;
    let debut = crate::kernel::timer::monotonic_ns();
    let mut tours = 0u32;
    while EMISSION.load(Ordering::Relaxed) != 0 {
        tours = tours.wrapping_add(1);
        if tours > 50_000_000
            || (tours % 256 == 0 && crate::kernel::timer::monotonic_ns().saturating_sub(debut) > BORNE_NS)
        {
            return;
        }
        core::hint::spin_loop();
    }
}

pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;
    if !is_ready() {
        return;
    }

    // Le formatage se fait HORS du jeton, interruptions ouvertes : il peut
    // etre long, et rien ne l'oblige a etre serialise.
    let mut sortie = TamponFormat::neuf();
    let _ = sortie.write_fmt(args);

    let ouvertes = x86_64::instructions::interrupts::are_enabled();
    // BOUCHAUD_SERIE_ATTENTE_DATEE_V1 : la file d'attente se fait AVANT de
    // masquer, sans rien tenir (voir `attend_jeton_libre`) ; la phase masquee
    // ne couvre plus que la course finale.
    if ouvertes {
        attend_jeton_libre();
    }
    x86_64::instructions::interrupts::disable();
    let moi = crate::arch::x86_64::smp::cpu_index() + 1;
    // Attente BORNEE : le port sert aussi aux paniques et aux interruptions ;
    // une ligne entrelacee vaut mieux qu'une machine qui se tait.
    let mut tours = 0u32;
    let pris = loop {
        match EMISSION.compare_exchange_weak(0, moi, Ordering::Acquire, Ordering::Relaxed) {
            Ok(_) => break true,
            Err(detenteur) if detenteur == moi => {
                EMISSIONS_REENTREES.fetch_add(1, Ordering::Relaxed);
                break false;
            }
            Err(_) => {}
        }
        // Interruptions masquees : servir soi-meme les shootdowns TLB qui
        // attendent ce coeur, comme pendant l'emission.
        if tours % 64 == 0 {
            crate::arch::x86_64::smp::sert_shootdowns_en_attente();
        }
        tours += 1;
        if tours > 100_000 {
            EMISSIONS_A_LA_BORNE.fetch_add(1, Ordering::Relaxed);
            break false;
        }
        core::hint::spin_loop();
    };
    sortie.vide();
    if pris {
        EMISSION.store(0, Ordering::Release);
    }
    if ouvertes {
        x86_64::instructions::interrupts::enable();
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
