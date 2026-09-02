//! Pilote clavier PS/2 pilote par interruptions, mapping AZERTY-FR.
//!
//! Le gestionnaire d'IRQ1 (voir `arch::x86_64::idt`) lit le scancode et l'empile
//! ici via `push_scancode`. L'editeur de ligne consomme la file et met le CPU en
//! veille (`hlt`) quand elle est vide. Gere Entree, Backspace, Suppr et Tab.

use x86_64::instructions::interrupts;

const QUEUE_SIZE: usize = 128;

/// File circulaire de scancodes alimentee par l'IRQ clavier.
// BOUCHAUD_C1_FILE_SCANCODES_SANS_GROS_VERROU_V1
//
// La file etait trois `static mut` : un tableau, une tete, une queue. Rien ne
// les protegeait. Le producteur est le gestionnaire d'IRQ1, les consommateurs
// sont des taches sur n'importe quel coeur -- et c'est le gros verrou, pris
// par le gestionnaire, qui empechait les deux de se marcher dessus.
//
// Un `SpinLockIrq` propre a la file remplace cela. Le choix d'IRQ plutot que
// d'un verrou tournant nu est OBLIGATOIRE ici : le producteur s'execute en
// contexte d'interruption. Si un consommateur tenait le verrou quand l'IRQ
// arrive sur le meme coeur, le gestionnaire tournerait a vide en attendant un
// verrou que seule la tache interrompue peut rendre -- un interblocage
// definitif. Masquer les interruptions le temps de la section critique, qui
// fait quelques instructions, ferme cela.
static FILE: crate::kernel::sync::SpinLockIrq<FileScancodes> =
    crate::kernel::sync::SpinLockIrq::new(FileScancodes {
        tampon: [0; QUEUE_SIZE],
        tete: 0,
        queue: 0,
    });

struct FileScancodes {
    tampon: [u8; QUEUE_SIZE],
    tete: usize,
    queue: usize,
}

impl FileScancodes {
    fn en_attente(&self) -> usize {
        if self.queue >= self.tete {
            self.queue - self.tete
        } else {
            QUEUE_SIZE - self.tete + self.queue
        }
    }
}

/// Compteurs de diagnostic. La machine est mono-coeur et ces champs ne sont
/// modifies que depuis IRQ1 (interruptions coupees) ou pendant l'initialisation.
// Compteurs de diagnostic : atomiques, sans verrou. Ils ne participent a
// aucune decision, seulement au journal.
static IRQ_COUNT: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static DROPPED_COUNT: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static LAST_SCANCODE: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);
static mut INIT_CONFIG: u8 = 0;
static mut INIT_ACK_DEFAULTS: u8 = 0;
static mut INIT_ACK_ENABLE: u8 = 0;

const WAIT_8042: usize = 100_000;

#[derive(Clone, Copy, Debug)]
pub struct Stats {
    pub irq: u64,
    pub dropped: u64,
    pub pending: usize,
    pub last_scancode: u8,
    pub controller_status: u8,
    pub controller_config: u8,
    pub pic_master_mask: u8,
    pub ack_defaults: u8,
    pub ack_enable: u8,
}

pub fn stats() -> Stats {
    use crate::arch::x86_64::ports::inb;
    let (head, tail, pending) = {
        let file = FILE.lock();
        (file.tete, file.queue, file.en_attente())
    };
    let irq = IRQ_COUNT.load(core::sync::atomic::Ordering::Relaxed);
    let dropped = DROPPED_COUNT.load(core::sync::atomic::Ordering::Relaxed);
    let last = LAST_SCANCODE.load(core::sync::atomic::Ordering::Relaxed);
    let (config, ack_defaults, ack_enable) =
        unsafe { (INIT_CONFIG, INIT_ACK_DEFAULTS, INIT_ACK_ENABLE) };
    let (master, _) = crate::arch::x86_64::interrupts::mask_snapshot();
    Stats {
        irq,
        dropped,
        pending,
        last_scancode: last,
        controller_status: unsafe { inb(0x64) },
        controller_config: config,
        pic_master_mask: master,
        ack_defaults,
        ack_enable,
    }
}

fn wait_input_empty() -> bool {
    use crate::arch::x86_64::ports::inb;
    for _ in 0..WAIT_8042 {
        if unsafe { inb(0x64) } & 0x02 == 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

fn wait_output_full() -> bool {
    use crate::arch::x86_64::ports::inb;
    for _ in 0..WAIT_8042 {
        if unsafe { inb(0x64) } & 0x01 != 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

fn controller_write(value: u8) -> bool {
    use crate::arch::x86_64::ports::outb;
    if !wait_input_empty() {
        return false;
    }
    unsafe { outb(0x64, value) };
    true
}

fn data_write(value: u8) -> bool {
    use crate::arch::x86_64::ports::outb;
    if !wait_input_empty() {
        return false;
    }
    unsafe { outb(0x60, value) };
    true
}

fn data_read() -> Option<u8> {
    use crate::arch::x86_64::ports::inb;
    if !wait_output_full() {
        return None;
    }
    Some(unsafe { inb(0x60) })
}

/// Envoie une commande au clavier et attend son ACK. Un `RESEND` (0xFE) est
/// rejoue une fois.
fn keyboard_command(command: u8) -> Option<u8> {
    for _ in 0..2 {
        if !data_write(command) {
            return None;
        }
        let reply = data_read()?;
        if reply == 0xFE {
            continue;
        }
        return Some(reply);
    }
    None
}

/// Prepare le controleur 8042 pour le clavier.
///
/// Deux choses indispensables, et une seule est evidente.
///
/// L'evidente : reactiver l'interface clavier et l'IRQ1 dans la configuration
/// du controleur, au cas ou le BIOS les aurait laissees fermees.
///
/// La moins evidente : **vider le tampon de sortie du controleur**. Le 8042
/// n'emet une nouvelle IRQ1 que lorsque son tampon a ete lu. Si le BIOS y
/// laisse un octet — un accuse de commande, une reponse d'autotest — et que
/// personne ne le lit, le controleur reste bloque sur cet octet : plus aucune
/// touche n'arrivera jamais, alors que tout le reste du systeme fonctionne.
/// Que cet octet soit present ou non depend du chemin exact suivi par le BIOS,
/// ce qui rend la panne intermittente d'un demarrage a l'autre.


pub fn init() {
    // Les interruptions sont deja actives a ce stade du boot. Une reponse a
    // F6/F4 ou au controleur arrive sur le meme port 0x60 que les scancodes :
    // IRQ1 ne doit pas pouvoir la consommer avant data_read().
    interrupts::without_interrupts(|| {
        use crate::arch::x86_64::ports::inb;

        let mut config = 0u8;

        {
            let mut file = FILE.lock();
            file.tete = 0;
            file.queue = 0;
        }
        IRQ_COUNT.store(0, core::sync::atomic::Ordering::Relaxed);
        DROPPED_COUNT.store(0, core::sync::atomic::Ordering::Relaxed);
        LAST_SCANCODE.store(0, core::sync::atomic::Ordering::Relaxed);
        // Reconfigurer le 8042 fait perdre des octets : un Shift enfonce
        // avant la reconfiguration n'aura jamais son relachement. Repartir
        // d'un etat neuf est la seule position honnete.
        *ETAT.lock() = EtatClavier::neuf();

        for _ in 0..64 {
            if unsafe { inb(0x64) } & 0x01 == 0 {
                break;
            }
            let _ = data_read();
        }

        let _ = controller_write(0xAE);

        if controller_write(0x20) {
            if let Some(current) = data_read() {
                config = current | 0x01 | 0x40;
                config &= !0x10;
                if controller_write(0x60) {
                    let _ = data_write(config);
                }
            }
        }

        let ack_defaults = keyboard_command(0xF6).unwrap_or(0);
        let ack_enable = keyboard_command(0xF4).unwrap_or(0);

        crate::arch::x86_64::interrupts::unmask_irq(1);
        let (master_mask, _) = crate::arch::x86_64::interrupts::mask_snapshot();

        unsafe {
            INIT_CONFIG = config;
            INIT_ACK_DEFAULTS = ack_defaults;
            INIT_ACK_ENABLE = ack_enable;
        }

        crate::kernel::dmesg::log_fmt(format_args!(
            "keyboard: 8042 pret config={:#04x}, ACK F6={:#04x} F4={:#04x}, PIC1 mask={:#04x}",
            config, ack_defaults, ack_enable, master_mask
        ));
    });
}

/// Rearme le clavier apres qu'un autre pilote a reconfigure le controleur 8042.
pub fn rearm_after_8042_reconfigure() {
    interrupts::without_interrupts(|| {
        let _ = controller_write(0xAE);

        let mut config = unsafe { INIT_CONFIG };
        if controller_write(0x20) {
            if let Some(current) = data_read() {
                config = current | 0x01 | 0x40;
                config &= !0x10;
                if controller_write(0x60) {
                    let _ = data_write(config);
                }
            }
        }

        let ack_enable = keyboard_command(0xF4).unwrap_or(0);
        crate::arch::x86_64::interrupts::unmask_irq(1);

        unsafe {
            INIT_CONFIG = config;
            INIT_ACK_ENABLE = ack_enable;
        }

        let (master_mask, _) = crate::arch::x86_64::interrupts::mask_snapshot();
        crate::kernel::dmesg::log_fmt(format_args!(
            "keyboard: rearme apres souris config={:#04x}, ACK F4={:#04x}, PIC1 mask={:#04x}",
            config, ack_enable, master_mask
        ));
    });
}

/// Empile un scancode. Appele depuis le gestionnaire d'interruption clavier.

pub fn push_scancode(sc: u8) {
    let mut range = false;
    IRQ_COUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    LAST_SCANCODE.store(sc, core::sync::atomic::Ordering::Relaxed);
    {
        let mut file = FILE.lock();
        let suivant = (file.queue + 1) % QUEUE_SIZE;
        if suivant != file.tete {
            let position = file.queue;
            file.tampon[position] = sc;
            file.queue = suivant;
            range = true;
        } else {
            DROPPED_COUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        }
    }
    // BOUCHAUD_GUI_EVENT_DRIVEN_V1
    //
    // Signaler seulement ce qui est REELLEMENT entre en file. Un scancode
    // perdu faute de place ne donne aucun travail a personne ; le compter
    // comme invalidation ferait croire a une activite qui n'existe pas, et
    // reveillerait le compositeur pour rien.
    //
    // Le pilote ne connait pas le compositeur : il parle au noyau, qui detient
    // le reveil. Aucune dependance de `drivers/` vers `gui/`.
    if range {
        crate::kernel::sync::signale_interface(crate::kernel::sync::SourceReveil::Clavier);
    }
}

/// Retire un scancode si disponible (interruptions deja desactivees).
fn pop_scancode() -> Option<u8> {
    let mut file = FILE.lock();
    if file.tete == file.queue {
        None
    } else {
        let sc = file.tampon[file.tete];
        file.tete = (file.tete + 1) % QUEUE_SIZE;
        Some(sc)
    }
}

/// Y a-t-il un scancode en attente ? (consultation sans consommation)
///
/// Necessaire a `poll`/`select` : signaler qu'un descripteur est lisible ne
/// doit pas retirer l'evenement de la file, sinon la lecture qui suit ne
/// trouverait plus rien.
pub fn has_pending() -> bool {
    let file = FILE.lock();
    file.tete != file.queue
}

/// Lecture non bloquante d'un scancode brut (None si rien). Utile pour le GUI.

pub fn try_scancode() -> Option<u8> {
    // Preserve l'etat IF de l'appelant.
    interrupts::without_interrupts(pop_scancode)
}

/// Attend le prochain scancode, en mettant le CPU en veille si la file est vide.
fn read_scancode() -> u8 {
    loop {
        interrupts::disable();
        if let Some(sc) = pop_scancode() {
            interrupts::enable();
            return sc;
        }
        // Active les interruptions puis halt de facon atomique : l'IRQ clavier
        // reveillera le CPU, qui rebouclera et trouvera le scancode.
        interrupts::enable_and_hlt();
    }
}

#[path = "clavier_decodeur.rs"]
mod decodeur;
pub use decodeur::{EtatClavier, Key, KeyEvent, Modificateurs};

/// L'unique etat du decodeur pour le clavier physique de la machine.
///
/// Le decodage lui-meme vit dans `clavier_decodeur.rs`, sans etat global, ce
/// qui le rend exercable sur l'hote. Ici il n'y a plus qu'un porteur.
///
/// Ce porteur etait un `static mut`, et le gros verrou etait tout ce qui le
/// serialisait : `read` s'executait sous BKL, donc deux CPU ne decodaient
/// jamais en meme temps. Liberer `read` retire cette protection accidentelle
/// -- les modificateurs (shift, ctrl, alt, prefixe 0xE0) sont un etat REL,
/// pas un compteur, et deux decodages entrelaces produisent une touche fausse
/// plutot qu'un chiffre approximatif. Il porte donc maintenant son verrou.
///
/// Un `SpinLock` ordinaire suffit : l'IRQ clavier ne fait que deposer des
/// scancodes dans `FILE` (un `SpinLockIrq`), elle ne decode jamais. Aucun
/// porteur de ce verrou-ci n'est interrompu par un autre qui le voudrait.
static ETAT: crate::kernel::sync::SpinLock<EtatClavier> =
    crate::kernel::sync::SpinLock::new(EtatClavier::neuf());

/// Lit la prochaine touche logique (bloquant).
pub fn read_key() -> Key {
    loop {
        let sc = read_scancode();
        if let Some(k) = ETAT.lock().decode_touche(sc) { return k; }
    }
}

/// Lecture non bloquante d'une touche logique (None si rien). Pour le GUI.
pub fn try_key() -> Option<Key> {
    loop {
        let sc = try_scancode()?;
        if let Some(k) = ETAT.lock().decode_touche(sc) { return Some(k); }
    }
}

/// Lecture non bloquante d'une transition de touche. Pour le bureau.
///
/// Contrairement a [`try_key`], les relachements sortent aussi : c'est toute
/// la raison d'etre de cette fonction.
pub fn try_key_event() -> Option<KeyEvent> {
    loop {
        let sc = try_scancode()?;
        if let Some(evenement) = ETAT.lock().decode(sc) { return Some(evenement); }
    }
}


/// Lit une ligne complete au clavier dans `buf`, renvoie le nombre d'octets.
pub fn read_line(buf: &mut [u8]) -> usize {
    read_into(buf, true)
}

/// Lit un secret (mot de passe) : seul `*` est affiche, jamais recopie ailleurs.
pub fn read_secret(buf: &mut [u8]) -> usize {
    read_into(buf, false)
}

/// Editeur de ligne minimal (login, nano, mot de passe). Le shell utilise un
/// editeur plus riche avec historique et completion (voir `shell`).
fn read_into(buf: &mut [u8], echo: bool) -> usize {
    let mut len = 0usize;
    loop {
        match read_key() {
            Key::Enter => { println!(""); return len; }
            Key::Backspace => {
                if len > 0 { len -= 1; print!("\x08"); }
            }
            Key::Char(c) => {
                if len < buf.len() {
                    buf[len] = c;
                    len += 1;
                    if echo { print!("{}", c as char); } else { print!("*"); }
                }
            }
            _ => {}
        }
    }
}
