//! Pilote clavier PS/2 en polling, mapping AZERTY-FR.
//!
//! Tant que les interruptions ne sont pas activees (voir
//! `arch::x86_64::interrupts`), le clavier est lu en interrogeant le controleur
//! PS/2. L'editeur de ligne gere Entree, Backspace, Suppr et Tab.

use crate::arch::x86_64::ports::inb;

/// Attend puis renvoie un scancode brut depuis le controleur PS/2.
fn read_scancode() -> u8 {
    loop {
        let status = unsafe { inb(0x64) };
        if status & 1 != 0 {
            return unsafe { inb(0x60) };
        }
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
/// Les caracteres accentues sont translitteres tant que l'affichage reste en
/// ASCII pur (ex. la touche `é` produit `e`).
fn scancode_to_char(sc: u8, shift: bool) -> Option<char> {
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
        _ => None,
    }
}

/// Lit une ligne complete au clavier dans `buf`, renvoie le nombre d'octets.
pub fn read_line(buf: &mut [u8]) -> usize {
    let mut len = 0usize;
    let mut shift = false;

    loop {
        let sc = read_scancode();

        // Certaines touches PS/2 sont envoyees avec le prefixe etendu 0xE0.
        // Exemple : Suppr/Delete = E0 53. Comme l'editeur de ligne n'a pas encore
        // de curseur horizontal, on mappe Suppr sur le meme comportement que Backspace.
        if sc == 0xe0 {
            let ext = read_scancode();
            if ext == 0x53 {
                if len > 0 {
                    len -= 1;
                    print!("\x08");
                }
            }
            continue;
        }

        match sc {
            0x2a | 0x36 => { shift = true; continue; }
            0xaa | 0xb6 => { shift = false; continue; }
            _ => {}
        }

        if sc & 0x80 != 0 {
            continue;
        }

        if let Some(ch) = scancode_to_char(sc, shift) {
            match ch {
                '\n' => {
                    println!("");
                    return len;
                }
                '\x08' => {
                    if len > 0 {
                        len -= 1;
                        print!("\x08");
                    }
                }
                '\t' => {
                    if len < buf.len() {
                        buf[len] = b' ';
                        len += 1;
                        print!(" ");
                    }
                }
                ch => {
                    if len < buf.len() && ch.is_ascii() {
                        buf[len] = ch as u8;
                        len += 1;
                        print!("{}", ch);
                    }
                }
            }
        }
    }
}
