//! Couche evdev : les peripheriques d'entree tels que Linux les expose.
//!
//! Une pile graphique (Qt `evdevkeyboard`/`evdevmouse`, SDL, libinput) ne lit
//! pas des caracteres : elle ouvre `/dev/input/event*`, interroge les capacites
//! du peripherique par ioctl, puis consomme un flux de `struct input_event`
//! contenant des **codes de touches Linux** et des **deltas** de souris. Ce
//! module fait la conversion depuis les pilotes PS/2 maison.
//!
//! ## Pourquoi les scancodes passent presque tels quels
//!
//! Le jeu de scancodes 1 du clavier PC et les codes de touches Linux
//! coincident sur toute la plage de base : `KEY_ESC` = 1 = scancode 0x01,
//! `KEY_Q` = 16 = scancode 0x10, et ainsi de suite jusqu'a 0x58. La conversion
//! se reduit donc a retirer le bit de relachement, et a traduire les seules
//! sequences etendues (prefixe 0xE0 : fleches, pave numerique, touches
//! systeme), qui elles sont renumerotees a partir de 96.
//!
//! Consequence a connaitre : ce sont des codes de **position physique**, pas
//! des caracteres. Le noyau applique une disposition AZERTY-FR pour son propre
//! shell, mais un programme evdev applique la sienne — par defaut `us` chez Qt.
//! Pour retrouver l'AZERTY il faut lui fournir un keymap
//! (`QT_QPA_EVDEV_KEYBOARD_PARAMETERS=/dev/input/event0:keymap=fr.qmap`).
//!
//! ## Souris : relative, pas absolue
//!
//! Le pilote PS/2 maintient une position absolue (utile au bureau du noyau),
//! mais un programme evdev attend `REL_X`/`REL_Y` — c'est ainsi qu'il
//! distingue une souris d'un ecran tactile. On derive donc les deltas de la
//! position, et on annonce `EV_REL` dans les capacites.

use alloc::vec::Vec;

use crate::drivers::{keyboard, mouse};

// --- Types d'evenements evdev ------------------------------------------------

pub const EV_SYN: u16 = 0x00;
pub const EV_KEY: u16 = 0x01;
pub const EV_REL: u16 = 0x02;
pub const EV_ABS: u16 = 0x03;
pub const EV_MSC: u16 = 0x04;

pub const SYN_REPORT: u16 = 0;

pub const REL_X: u16 = 0x00;
pub const REL_Y: u16 = 0x01;
pub const REL_WHEEL: u16 = 0x08;

pub const BTN_LEFT: u16 = 0x110;
pub const BTN_RIGHT: u16 = 0x111;
pub const BTN_MIDDLE: u16 = 0x112;

/// Taille d'un `struct input_event` sur x86-64 : deux `long` de temps, puis
/// type, code et valeur.
pub const EVENT_SIZE: usize = 24;

/// Plus grand code de touche annonce (borne des bitmaps `EVIOCGBIT`).
pub const KEY_MAX: u16 = 0x2FF;
pub const REL_MAX: u16 = 0x0F;

/// Compose un `struct input_event`.
fn event(kind: u16, code: u16, value: i32) -> [u8; EVENT_SIZE] {
    let mut buffer = [0u8; EVENT_SIZE];
    // Horodatage : les clients ne s'en servent que par differences (vitesse de
    // deplacement, double-clic), la base importe peu mais la monotonie si.
    let ms = crate::kernel::timer::monotonic_ms();
    buffer[0..8].copy_from_slice(&(ms / 1000).to_le_bytes());
    buffer[8..16].copy_from_slice(&((ms % 1000) * 1000).to_le_bytes());
    buffer[16..18].copy_from_slice(&kind.to_le_bytes());
    buffer[18..20].copy_from_slice(&code.to_le_bytes());
    buffer[20..24].copy_from_slice(&value.to_le_bytes());
    buffer
}

// --- Clavier -----------------------------------------------------------------

/// Un octet 0xE0 a ete lu : le scancode suivant appartient au jeu etendu.
// BOUCHAUD_C1_ENTREES_SANS_GROS_VERROU_V1
//
// Ces etats etaient des `static mut`. Rien ne les protegeait : c'est le gros
// verrou, pris par les descripteurs `/dev/input/*`, qui les rendait surs -- et
// c'est de lui qu'on sort. Deux CPU lisant la file de scancodes en meme temps
// la corrompraient.
//
// Un verrou tournant PROPRE a ce sous-systeme suffit, et sa section critique
// est minuscule : une traduction de scancode, un ajout en file. C'est tout
// l'ecart avec le verrou global, qui immobilisait la machine entiere pour la
// meme chose.
static ETAT_CLAVIER: crate::kernel::sync::SpinLock<EtatClavier> =
    crate::kernel::sync::SpinLock::new(EtatClavier {
        prefixe_etendu: false,
        en_attente: Vec::new(),
    });

struct EtatClavier {
    /// Le scancode precedent etait le prefixe 0xE0.
    prefixe_etendu: bool,
    /// Evenements evdev deja traduits, pas encore lus.
    en_attente: Vec<u8>,
}

/// Traduit un scancode etendu (prefixe 0xE0) en code de touche Linux.
fn extended_keycode(scancode: u8) -> Option<u16> {
    Some(match scancode {
        0x1C => 96,  // KEY_KPENTER
        0x1D => 97,  // KEY_RIGHTCTRL
        0x35 => 98,  // KEY_KPSLASH
        0x38 => 100, // KEY_RIGHTALT (AltGr)
        0x47 => 102, // KEY_HOME
        0x48 => 103, // KEY_UP
        0x49 => 104, // KEY_PAGEUP
        0x4B => 105, // KEY_LEFT
        0x4D => 106, // KEY_RIGHT
        0x4F => 107, // KEY_END
        0x50 => 108, // KEY_DOWN
        0x51 => 109, // KEY_PAGEDOWN
        0x52 => 110, // KEY_INSERT
        0x53 => 111, // KEY_DELETE
        0x5B => 125, // KEY_LEFTMETA
        0x5C => 126, // KEY_RIGHTMETA
        0x5D => 127, // KEY_COMPOSE
        _ => return None,
    })
}

/// Evenements clavier deja traduits, en attente de lecture.
///
/// Ce tampon intermediaire n'est pas un confort : tous les scancodes ne
/// produisent pas d'evenement (prefixe 0xE0, accuses du controleur 8042,
/// touches hors table). Sans lui, « la file de scancodes est non vide » ne
/// voudrait pas dire « il y a quelque chose a lire », et un client verrait son
/// `poll` revenir aussitot puis son `read` echouer en `EAGAIN` — une boucle
/// qui consomme un cœur entier sans rien faire.


/// Traduit tous les scancodes disponibles en evenements evdev.
fn drain_scancodes() {
    // Un seul verrouillage pour toute la vidange : le prendre par scancode
    // ferait entrelacer deux lecteurs au milieu d'une sequence 0xE0, et le
    // prefixe se retrouverait applique a la mauvaise touche.
    let mut etat = ETAT_CLAVIER.lock();
    while let Some(scancode) = keyboard::try_scancode() {
        if scancode == 0xE0 {
            etat.prefixe_etendu = true;
            continue;
        }
        let released = scancode & 0x80 != 0;
        let base = scancode & 0x7F;
        let extended = core::mem::replace(&mut etat.prefixe_etendu, false);

        let keycode = if extended {
            match extended_keycode(base) {
                Some(code) => code,
                None => continue,
            }
        } else if base >= 1 && base <= 0x58 {
            // Jeu de base : le code de touche Linux est le scancode lui-meme.
            base as u16
        } else {
            continue;
        };

        let evenement = event(EV_KEY, keycode, if released { 0 } else { 1 });
        let synchro = event(EV_SYN, SYN_REPORT, 0);
        etat.en_attente.extend_from_slice(&evenement);
        etat.en_attente.extend_from_slice(&synchro);
    }
}

/// Y a-t-il un evenement clavier a lire ? (pour `poll`/`select`)
pub fn keyboard_pending() -> bool {
    drain_scancodes();
    !ETAT_CLAVIER.lock().en_attente.is_empty()
}

/// Retire jusqu'a `max` octets d'evenements clavier.
///
/// Non bloquant : renvoie un tampon vide s'il n'y a rien a lire (l'appelant
/// traduit cela en `EAGAIN`, ce qu'attend un descripteur non bloquant).
pub fn read_keyboard(max: usize) -> Vec<u8> {
    drain_scancodes();
    let mut etat = ETAT_CLAVIER.lock();
    // On ne coupe jamais un evenement en deux.
    let len = core::cmp::min(etat.en_attente.len(), max - max % EVENT_SIZE);
    etat.en_attente.drain(..len).collect()
}

// --- Souris ------------------------------------------------------------------

/// Dernier etat de souris publie. Meme raison que pour le clavier : c'etait un
/// `static mut` que seul le gros verrou rendait sur.
static ETAT_SOURIS: crate::kernel::sync::SpinLock<EtatSouris> =
    crate::kernel::sync::SpinLock::new(EtatSouris { x: 0, y: 0, boutons: 0 });

#[derive(Clone, Copy)]
struct EtatSouris {
    x: i32,
    y: i32,
    boutons: u8,
}

/// Prend un instantane de l'etat de la souris sans produire d'evenement.
///
/// Appele a l'ouverture de `/dev/input/event1`. Sans cela, la premiere lecture
/// signalerait comme un mouvement l'ecart entre l'etat initial du module et
/// celui du pilote — et surtout, `poll` verrait le descripteur pret en
/// permanence, ce qui ferait tourner a vide la boucle d'evenements du client
/// au lieu de la laisser dormir.
pub fn sync_mouse() {
    let (x, y) = mouse::pos();
    *ETAT_SOURIS.lock() = EtatSouris { x: x as i32, y: y as i32, boutons: mouse::buttons() };
    let _ = mouse::take_wheel();
}

/// Produit les evenements de souris depuis le dernier appel : deltas de
/// position, changements d'etat des boutons, molette.
pub fn read_mouse(max: usize) -> Vec<u8> {
    let (x, y) = mouse::pos();
    let (x, y) = (x as i32, y as i32);
    let buttons = mouse::buttons();

    // Le verrou couvre la comparaison ET la publication : deux lecteurs
    // simultanes produiraient sinon deux fois le meme deplacement, ou aucun.
    let mut etat = ETAT_SOURIS.lock();
    let dx = x - etat.x;
    let dy = y - etat.y;
    let wheel = mouse::take_wheel();
    let changed = buttons ^ etat.boutons;

    if dx == 0 && dy == 0 && wheel == 0 && changed == 0 {
        return Vec::new();
    }

    *etat = EtatSouris { x, y, boutons: buttons };
    drop(etat);

    let mut out: Vec<u8> = Vec::new();
    let push = |bytes: [u8; EVENT_SIZE], out: &mut Vec<u8>| {
        if out.len() + EVENT_SIZE <= max {
            out.extend_from_slice(&bytes);
        }
    };

    if dx != 0 {
        push(event(EV_REL, REL_X, dx), &mut out);
    }
    if dy != 0 {
        push(event(EV_REL, REL_Y, dy), &mut out);
    }
    if wheel != 0 {
        // La molette PS/2 compte a l'envers de la convention evdev.
        push(event(EV_REL, REL_WHEEL, -wheel), &mut out);
    }
    for (mask, code) in [(0x01u8, BTN_LEFT), (0x02, BTN_RIGHT), (0x04, BTN_MIDDLE)] {
        if changed & mask != 0 {
            push(event(EV_KEY, code, if buttons & mask != 0 { 1 } else { 0 }), &mut out);
        }
    }
    if !out.is_empty() {
        push(event(EV_SYN, SYN_REPORT, 0), &mut out);
    }
    out
}

/// Y a-t-il des evenements souris en attente ? (pour `poll`/`select`)
pub fn mouse_pending() -> bool {
    let (x, y) = mouse::pos();
    let etat = *ETAT_SOURIS.lock();
    x as i32 != etat.x
        || y as i32 != etat.y
        || mouse::buttons() != etat.boutons
        || mouse::wheel_pending()
}

// --- Capacites (EVIOCGBIT) ---------------------------------------------------

/// Peripherique d'entree expose.
#[derive(Clone, Copy, PartialEq)]
pub enum Device {
    Keyboard,
    Mouse,
}

/// Positionne un bit dans un bitmap little-endian d'octets.
fn set_bit(bitmap: &mut [u8], bit: u16) {
    let index = (bit / 8) as usize;
    if index < bitmap.len() {
        bitmap[index] |= 1 << (bit % 8);
    }
}

/// Remplit le bitmap demande par `EVIOCGBIT(kind, len)`.
///
/// `kind` a 0 designe la liste des types d'evenements supportes ; sinon c'est
/// le bitmap des codes de ce type. C'est ce que lit un client pour decider si
/// le peripherique est un clavier, une souris ou un ecran tactile — un bitmap
/// vide le ferait ignorer le peripherique.
pub fn capability_bits(device: Device, kind: u16, len: usize) -> Vec<u8> {
    let mut bitmap = alloc::vec![0u8; len];
    match (device, kind) {
        (Device::Keyboard, 0) => {
            set_bit(&mut bitmap, EV_SYN);
            set_bit(&mut bitmap, EV_KEY);
            set_bit(&mut bitmap, EV_MSC);
        }
        (Device::Keyboard, EV_KEY) => {
            // Toutes les touches du jeu de base plus les etendues.
            for code in 1..=0x58u16 {
                set_bit(&mut bitmap, code);
            }
            for code in 96..=127u16 {
                set_bit(&mut bitmap, code);
            }
        }
        (Device::Mouse, 0) => {
            set_bit(&mut bitmap, EV_SYN);
            set_bit(&mut bitmap, EV_KEY);
            set_bit(&mut bitmap, EV_REL);
        }
        (Device::Mouse, EV_KEY) => {
            set_bit(&mut bitmap, BTN_LEFT);
            set_bit(&mut bitmap, BTN_RIGHT);
            set_bit(&mut bitmap, BTN_MIDDLE);
        }
        (Device::Mouse, EV_REL) => {
            set_bit(&mut bitmap, REL_X);
            set_bit(&mut bitmap, REL_Y);
            set_bit(&mut bitmap, REL_WHEEL);
        }
        _ => {}
    }
    bitmap
}

/// Nom lisible du peripherique (`EVIOCGNAME`).
pub fn device_name(device: Device) -> &'static [u8] {
    match device {
        Device::Keyboard => b"Bouchaud OS PS/2 Keyboard\0",
        Device::Mouse => b"Bouchaud OS PS/2 Mouse\0",
    }
}

/// Chemin physique du peripherique (`EVIOCGPHYS`).
pub fn device_phys(device: Device) -> &'static [u8] {
    match device {
        Device::Keyboard => b"isa0060/serio0/input0\0",
        Device::Mouse => b"isa0060/serio1/input0\0",
    }
}

/// `struct input_id` : bus, constructeur, produit, version.
pub fn device_id(device: Device) -> [u8; 8] {
    let mut buffer = [0u8; 8];
    buffer[0..2].copy_from_slice(&0x0011u16.to_le_bytes()); // BUS_I8042
    buffer[2..4].copy_from_slice(&0x0001u16.to_le_bytes()); // vendor
    let product: u16 = if device == Device::Keyboard { 0x0001 } else { 0x0002 };
    buffer[4..6].copy_from_slice(&product.to_le_bytes());
    buffer[6..8].copy_from_slice(&0x0001u16.to_le_bytes()); // version
    buffer
}
