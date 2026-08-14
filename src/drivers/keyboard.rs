//! Pilote clavier PS/2 pilote par interruptions, mapping AZERTY-FR.
//!
//! Le gestionnaire d'IRQ1 (voir `arch::x86_64::idt`) lit le scancode et l'empile
//! ici via `push_scancode`. L'editeur de ligne consomme la file et met le CPU en
//! veille (`hlt`) quand elle est vide. Gere Entree, Backspace, Suppr et Tab.

use x86_64::instructions::interrupts;

const QUEUE_SIZE: usize = 128;

/// File circulaire de scancodes alimentee par l'IRQ clavier.
static mut QUEUE: [u8; QUEUE_SIZE] = [0; QUEUE_SIZE];
static mut Q_HEAD: usize = 0;
static mut Q_TAIL: usize = 0;

/// Compteurs de diagnostic. La machine est mono-coeur et ces champs ne sont
/// modifies que depuis IRQ1 (interruptions coupees) ou pendant l'initialisation.
static mut IRQ_COUNT: u64 = 0;
static mut DROPPED_COUNT: u64 = 0;
static mut LAST_SCANCODE: u8 = 0;
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
    let (head, tail, irq, dropped, last, config, ack_defaults, ack_enable) = unsafe {
        (
            Q_HEAD,
            Q_TAIL,
            IRQ_COUNT,
            DROPPED_COUNT,
            LAST_SCANCODE,
            INIT_CONFIG,
            INIT_ACK_DEFAULTS,
            INIT_ACK_ENABLE,
        )
    };
    let pending = if tail >= head {
        tail - head
    } else {
        QUEUE_SIZE - head + tail
    };
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

        unsafe {
            Q_HEAD = 0;
            Q_TAIL = 0;
            IRQ_COUNT = 0;
            DROPPED_COUNT = 0;
            LAST_SCANCODE = 0;
            SHIFT = false;
            ALTGR = false;
            EXTENDED_PENDING = false;
        }

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
    unsafe {
        IRQ_COUNT = IRQ_COUNT.wrapping_add(1);
        LAST_SCANCODE = sc;
        let next = (Q_TAIL + 1) % QUEUE_SIZE;
        if next != Q_HEAD {
            QUEUE[Q_TAIL] = sc;
            Q_TAIL = next;
        } else {
            DROPPED_COUNT = DROPPED_COUNT.wrapping_add(1);
        }
    }
}

/// Retire un scancode si disponible (interruptions deja desactivees).
fn pop_scancode() -> Option<u8> {
    unsafe {
        if Q_HEAD == Q_TAIL {
            None
        } else {
            let sc = QUEUE[Q_HEAD];
            Q_HEAD = (Q_HEAD + 1) % QUEUE_SIZE;
            Some(sc)
        }
    }
}

/// Y a-t-il un scancode en attente ? (consultation sans consommation)
///
/// Necessaire a `poll`/`select` : signaler qu'un descripteur est lisible ne
/// doit pas retirer l'evenement de la file, sinon la lecture qui suit ne
/// trouverait plus rien.
pub fn has_pending() -> bool {
    unsafe { core::ptr::read_volatile(&Q_HEAD) != core::ptr::read_volatile(&Q_TAIL) }
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

fn ascii_letter(ch: u8, shift: bool) -> char {
    if shift && ch >= b'a' && ch <= b'z' {
        (ch - 32) as char
    } else {
        ch as char
    }
}

/// Traduit un scancode en caractere selon la disposition AZERTY-FR.
///
/// `altgr` active la 3e couche (AltGr) qui fournit les symboles indispensables
/// au shell ( | < > { } [ ] \ @ # ~ ` ^ ), utile notamment sur les claviers
/// portables depourvus de la touche ISO `<>` (ex. Dell a pave numerique).
///
/// Les caracteres accentues sont translitteres tant que l'affichage reste en
/// ASCII pur (ex. la touche `é` produit `e`).
fn scancode_to_char(sc: u8, shift: bool, altgr: bool) -> Option<char> {
    if altgr {
        // Couche AltGr (FR) + raccourcis Bouchaud OS pour < et > sans touche ISO.
        return match sc {
            0x03 => Some('~'),   // AltGr+2
            0x04 => Some('#'),   // AltGr+3
            0x05 => Some('{'),   // AltGr+4
            0x06 => Some('['),   // AltGr+5
            0x07 => Some('|'),   // AltGr+6
            0x08 => Some('`'),   // AltGr+7
            0x09 => Some('\\'),  // AltGr+8
            0x0a => Some('^'),   // AltGr+9
            0x0b => Some('@'),   // AltGr+0
            0x0c => Some(']'),   // AltGr+)
            0x0d => Some('}'),   // AltGr+=
            0x32 => Some('<'),   // AltGr+, (touche virgule)
            0x33 => Some('>'),   // AltGr+; (touche point-virgule)
            _ => None,
        };
    }
    match sc {
        0x01 => Some('\x1b'),
        0x0e => Some('\x08'),
        0x0f => Some('\t'),
        0x1c => Some('\n'),
        0x39 => Some(' '),

        // Ligne numerique AZERTY. Les accents sont translitteres pour l'instant.
        0x02 => Some(if shift { '1' } else { '&' }),
        0x03 => Some(if shift { '2' } else { 'e' }),
        0x04 => Some(if shift { '3' } else { '"' }),
        0x05 => Some(if shift { '4' } else { '\'' }),
        0x06 => Some(if shift { '5' } else { '(' }),
        0x07 => Some(if shift { '6' } else { '-' }),
        0x08 => Some(if shift { '7' } else { 'e' }),
        0x09 => Some(if shift { '8' } else { '_' }),
        0x0a => Some(if shift { '9' } else { 'c' }),
        0x0b => Some(if shift { '0' } else { 'a' }),
        0x0c => Some(if shift { ')' } else { ')' }),
        0x0d => Some(if shift { '+' } else { '=' }),

        // AZERTY lettres principales
        0x10 => Some(ascii_letter(b'a', shift)),
        0x11 => Some(ascii_letter(b'z', shift)),
        0x12 => Some(ascii_letter(b'e', shift)),
        0x13 => Some(ascii_letter(b'r', shift)),
        0x14 => Some(ascii_letter(b't', shift)),
        0x15 => Some(ascii_letter(b'y', shift)),
        0x16 => Some(ascii_letter(b'u', shift)),
        0x17 => Some(ascii_letter(b'i', shift)),
        0x18 => Some(ascii_letter(b'o', shift)),
        0x19 => Some(ascii_letter(b'p', shift)),
        0x1a => Some(if shift { '^' } else { '^' }),
        0x1b => Some(if shift { '*' } else { '$' }),

        0x1e => Some(ascii_letter(b'q', shift)),
        0x1f => Some(ascii_letter(b's', shift)),
        0x20 => Some(ascii_letter(b'd', shift)),
        0x21 => Some(ascii_letter(b'f', shift)),
        0x22 => Some(ascii_letter(b'g', shift)),
        0x23 => Some(ascii_letter(b'h', shift)),
        0x24 => Some(ascii_letter(b'j', shift)),
        0x25 => Some(ascii_letter(b'k', shift)),
        0x26 => Some(ascii_letter(b'l', shift)),
        0x27 => Some(ascii_letter(b'm', shift)),
        0x28 => Some(if shift { '%' } else { 'u' }),
        0x2b => Some(if shift { '|' } else { '*' }),

        0x2c => Some(ascii_letter(b'w', shift)),
        0x2d => Some(ascii_letter(b'x', shift)),
        0x2e => Some(ascii_letter(b'c', shift)),
        0x2f => Some(ascii_letter(b'v', shift)),
        0x30 => Some(ascii_letter(b'b', shift)),
        0x31 => Some(ascii_letter(b'n', shift)),
        0x32 => Some(if shift { '?' } else { ',' }),
        0x33 => Some(if shift { '.' } else { ';' }),
        0x34 => Some(if shift { '/' } else { ':' }),
        0x35 => Some(if shift { '/' } else { '!' }),

        // Touche ISO "<>" (a gauche de W) presente sur la plupart des AZERTY.
        0x56 => Some(if shift { '>' } else { '<' }),
        _ => None,
    }
}


/// Touche logique renvoyee par `read_key`.
#[derive(Clone, Copy, PartialEq)]
pub enum Key {
    Char(u8),
    Enter,
    Backspace,
    Tab,
    Up,
    Down,
    Left,
    Right,
    Other,
}

/// Etat persistant de la touche Shift entre deux appels a `read_key`.
static mut SHIFT: bool = false;
/// Etat persistant de la touche AltGr (Alt droit, sequence E0 38 / E0 B8).
static mut ALTGR: bool = false;
/// Prefixe E0 recu sans son second octet : `try_key` reste non bloquant.
static mut EXTENDED_PENDING: bool = false;

/// Decode un scancode (et son eventuel 2e octet etendu) en touche logique.
/// Renvoie None pour les codes qui ne font que modifier un etat (Shift/AltGr)
/// ou les relachements.

fn decode_from(sc: u8) -> Option<Key> {
    if unsafe { EXTENDED_PENDING } {
        unsafe { EXTENDED_PENDING = false; }
        return match sc {
            0x38 => { unsafe { ALTGR = true; } None }
            0xb8 => { unsafe { ALTGR = false; } None }
            0x48 => Some(Key::Up),
            0x50 => Some(Key::Down),
            0x4b => Some(Key::Left),
            0x4d => Some(Key::Right),
            0x53 => Some(Key::Backspace),
            _ => None,
        };
    }
    if sc == 0xe0 {
        unsafe { EXTENDED_PENDING = true; }
        return None;
    }
    match sc {
        0x2a | 0x36 => { unsafe { SHIFT = true; } None }
        0xaa | 0xb6 => { unsafe { SHIFT = false; } None }
        _ => {
            if sc & 0x80 != 0 { return None; }
            let shift = unsafe { SHIFT };
            let altgr = unsafe { ALTGR };
            scancode_to_char(sc, shift, altgr).map(|ch| match ch {
                '\n' => Key::Enter,
                '\x08' => Key::Backspace,
                '\t' => Key::Tab,
                '\x1b' => Key::Other,
                c => Key::Char(c as u8),
            })
        }
    }
}

/// Lit la prochaine touche logique (bloquant).
pub fn read_key() -> Key {
    loop {
        if let Some(k) = decode_from(read_scancode()) { return k; }
    }
}

/// Lecture non bloquante d'une touche logique (None si rien). Pour le GUI.
pub fn try_key() -> Option<Key> {
    loop {
        let sc = try_scancode()?;
        if let Some(k) = decode_from(sc) { return Some(k); }
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
