//! Pilote VGA texte (buffer memoire 0xb8000, 80x25).
//!
//! Fournit le writer global utilise par les macros `print!` / `println!` ainsi
//! que la gestion des couleurs et du defilement.

use core::fmt;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use crate::arch::x86_64::ports::outb;
use crate::kernel::sync::SpinLockIrq;

const VGA_BUFFER: usize = 0xb8000;
const VGA_WIDTH: usize = 80;
const VGA_HEIGHT: usize = 25;

/// Dimensions de l'ecran texte, exposees pour l'editeur plein ecran.
pub const WIDTH: usize = VGA_WIDTH;
pub const HEIGHT: usize = VGA_HEIGHT;

pub const COLOR_DEFAULT: u8 = 0x0f;
pub const COLOR_GREEN: u8 = 0x0a;
pub const COLOR_CYAN: u8 = 0x0b;
pub const COLOR_RED: u8 = 0x0c;
pub const COLOR_YELLOW: u8 = 0x0e;

const CELLULES: usize = VGA_WIDTH * VGA_HEIGHT;

// BOUCHAUD_VGA_TEXTE_OMBRE_V1 -- L'ECRAN TEXTE VIT EN RAM
//
// Chaque cellule s'ecrivait directement dans 0xB8000, et chaque defilement
// relisait puis recopiait les 1 920 cellules de l'ecran, octet par octet :
// 7 680 acces MMIO -- autant de sorties de VM sous KVM, autant d'appels au
// peripherique emule sous TCG --, une ligne de 1 800 octets en defilant 23.
// Le tout sous le verrou de la console, INTERRUPTIONS MASQUEES. Banc de
// saturation TCG SMP4 (port a 5 us/octet, six ecrivains) : verrou tenu
// 762 ms, attendu 2,68 s ; les ecrivains restaient en mode noyau (preemption
// differee, aucun point sur), le fil `console` pret dans la file de leur
// coeur n'etait plus elu pendant 2 s, et la console passait en repli.
//
// Desormais l'ecran est une ombre en RAM : ecrire et defiler n'y sont que
// des copies memoire, sous le verrou. La memoire texte n'est recopiee qu'APRES
// le verrou, interruptions telles que l'appelant les avait : une image de
// l'ombre est prise sous le verrou (4 Kio de RAM), puis ses lignes modifiees
// partent sur 0xB8000 en mots de 32 bits, sans aucune lecture -- au plus
// 1 000 ecritures au lieu de 7 680 acces par defilement. Un seul publieur a
// la fois (`PUBLICATION_EN_COURS`) : qui le trouve occupe laisse ses lignes
// sales et le lui signale, il repart avec une nouvelle image.
//
// Le defilement MATERIEL (registre d'origine du CRTC, memoire texte en
// anneau) a ete essaye : sous QEMU l'ecran restait celui du BIOS. Rendu
// verifie par `screendump` : ombre + recopie, identique au chemin d'avant.
pub struct VgaWriter {
    row: usize,
    col: usize,
    color: u8,
    /// L'ecran, en RAM : `couleur << 8 | caractere` par cellule.
    cellules: [u16; CELLULES],
    /// Lignes de l'ecran modifiees depuis la derniere image (bit = ligne).
    lignes_sales: u32,
    /// L'ombre a ete lue une fois sur l'ecran (ce que le BIOS y a laisse).
    chargee: bool,
}

const TOUTES_LES_LIGNES: u32 = (1 << VGA_HEIGHT) - 1;

/// L'image a recopier sur la memoire texte et ses lignes modifiees. Ecrite
/// sous le verrou de la console par le SEUL detenteur de
/// `PUBLICATION_EN_COURS`, lue par lui hors du verrou.
struct Image {
    cellules: core::cell::UnsafeCell<[u16; CELLULES]>,
    lignes: core::cell::UnsafeCell<u32>,
}
// SAFETY : un seul detenteur du jeton `PUBLICATION_EN_COURS` y accede.
unsafe impl Sync for Image {}
static IMAGE: Image = Image {
    cellules: core::cell::UnsafeCell::new([0; CELLULES]),
    lignes: core::cell::UnsafeCell::new(0),
};
static PUBLICATION_EN_COURS: AtomicBool = AtomicBool::new(false);
/// Pose AVANT d'essayer le jeton : un publieur qui le rend relit ce drapeau
/// (ordre sequentiel), aucune ligne sale n'est oubliee.
static PUBLICATION_DEMANDEE: AtomicBool = AtomicBool::new(false);
static PUBLICATION_PIRE_NS: AtomicU64 = AtomicU64::new(0);

// BOUCHAUD_CONSOLE_VERROU_V1
//
// L'ecrivain VGA et la pile de captures etaient deux `static mut`. Le gros
// verrou du noyau ne les protegeait qu'a moitie : `write(1)` d'un programme le
// prenait, le bureau le tenait, mais chaque fil noyau qui fait un `println!`
// (services reseau, echantillonneur, ...) ecrivait deja sans lui -- et pouvait
// pousser dans le `String` de capture pendant qu'un autre coeur le faisait
// grandir, ou `pop` la pile sous ses pieds.
//
// Les deux vivent maintenant sous un seul verrou a interruptions masquees
// (un `println!` peut partir d'un gestionnaire). La prise est BORNEE : un
// coeur qui panique en tenant la console, puis y reecrit, perdrait sinon la
// machine dans une attente sans fin ; il perd seulement la copie VGA de son
// message, que COM1 et l'ecran GOP recoivent de toute facon.
struct Console {
    vga: VgaWriter,
    /// Pile de tampons de capture. Quand elle n'est pas vide, la sortie texte
    /// est ecrite dans le tampon du sommet au lieu d'aller a l'ecran. La pile
    /// permet d'imbriquer redirections (`>`) et pipes (`|`).
    captures: alloc::vec::Vec<alloc::string::String>,
}

static CONSOLE: SpinLockIrq<Console> = SpinLockIrq::new(Console {
    vga: VgaWriter {
        row: 0,
        col: 0,
        color: COLOR_DEFAULT,
        cellules: [0; CELLULES],
        lignes_sales: 0,
        chargee: false,
    },
    captures: alloc::vec::Vec::new(),
});

/// Tentatives de prise avant d'abandonner une ecriture de console.
const PRISE_CONSOLE_TENTATIVES: u32 = 1 << 22;

/// Ecritures de console abandonnees faute d'avoir pu prendre le verrou.
static CONSOLE_ABANDONS: AtomicU64 = AtomicU64::new(0);

/// BOUCHAUD_VGA_TEXTE_MESURE_V1 : le verrou de la console est tenu
/// INTERRUPTIONS MASQUEES. Prises, pire detention, pire attente (ns).
static CONSOLE_PRISES: AtomicU64 = AtomicU64::new(0);
static CONSOLE_TENU_PIRE_NS: AtomicU64 = AtomicU64::new(0);
static CONSOLE_ATTENTE_PIRE_NS: AtomicU64 = AtomicU64::new(0);

/// Execute `f` sous le verrou de la console. `None` si la prise a echoue apres
/// `PRISE_CONSOLE_TENTATIVES` essais : voir BOUCHAUD_CONSOLE_VERROU_V1.
fn avec_console<R>(f: impl FnOnce(&mut Console) -> R) -> Option<R> {
    let debut = crate::kernel::timer::monotonic_ns();
    for _ in 0..PRISE_CONSOLE_TENTATIVES {
        if let Some(mut console) = CONSOLE.try_lock() {
            let pris = crate::kernel::timer::monotonic_ns();
            let r = f(&mut console);
            drop(console);
            let fin = crate::kernel::timer::monotonic_ns();
            CONSOLE_PRISES.fetch_add(1, Ordering::Relaxed);
            CONSOLE_TENU_PIRE_NS.fetch_max(fin.saturating_sub(pris), Ordering::Relaxed);
            CONSOLE_ATTENTE_PIRE_NS.fetch_max(pris.saturating_sub(debut), Ordering::Relaxed);
            return Some(r);
        }
        core::hint::spin_loop();
    }
    CONSOLE_ABANDONS.fetch_add(1, Ordering::Relaxed);
    None
}

/// `(prises, pire detention interruptions masquees, pire attente, abandons,
/// pire recopie sur la memoire texte hors du verrou)` de la console texte,
/// depuis le demarrage (ns).
pub fn mesures_console() -> (u64, u64, u64, u64, u64) {
    (
        CONSOLE_PRISES.load(Ordering::Relaxed),
        CONSOLE_TENU_PIRE_NS.load(Ordering::Relaxed),
        CONSOLE_ATTENTE_PIRE_NS.load(Ordering::Relaxed),
        CONSOLE_ABANDONS.load(Ordering::Relaxed),
        PUBLICATION_PIRE_NS.load(Ordering::Relaxed),
    )
}

/// Ecritures de console abandonnees depuis le demarrage (diagnostic).
pub fn abandons_console() -> u64 {
    CONSOLE_ABANDONS.load(Ordering::Relaxed)
}

impl VgaWriter {
    fn clear(&mut self) {
        self.charge();
        self.cellules.fill((self.color as u16) << 8 | b' ' as u16);
        self.lignes_sales = TOUTES_LES_LIGNES;
        self.row = 0;
        self.col = 0;
    }

    fn set_color(&mut self, color: u8) {
        self.color = color;
    }

    fn write_cell(&mut self, row: usize, col: usize, byte: u8, color: u8) {
        self.charge();
        self.cellules[row * VGA_WIDTH + col] = (color as u16) << 8 | byte as u16;
        self.lignes_sales |= 1 << row;
    }

    /// Une fois : l'ombre part de ce que l'ecran affiche deja.
    fn charge(&mut self) {
        if self.chargee {
            return;
        }
        self.chargee = true;
        #[cfg(not(feature = "reference-desktop"))]
        for (i, cellule) in self.cellules.iter_mut().enumerate() {
            // SAFETY : 0xB8000 + 2 * i est dans la memoire texte (80 x 25).
            *cellule = unsafe { core::ptr::read_volatile((VGA_BUFFER as *const u16).add(i)) };
        }
    }

    /// Sous le verrou de la console : s'il y a des lignes modifiees et que le
    /// jeton de publication est libre, le prend, copie l'ombre dans `IMAGE`
    /// et rend `true` -- l'appelant publiera HORS du verrou
    /// (`publie_hors_verrou`). Jeton tenu ailleurs : les lignes restent
    /// sales, son detenteur repartira avec une nouvelle image.
    fn prepare_publication(&mut self) -> bool {
        if self.lignes_sales == 0 {
            return false;
        }
        PUBLICATION_DEMANDEE.store(true, Ordering::SeqCst);
        if PUBLICATION_EN_COURS.swap(true, Ordering::SeqCst) {
            return false;
        }
        PUBLICATION_DEMANDEE.store(false, Ordering::SeqCst);
        // SAFETY : jeton tenu -- seul acces a `IMAGE`.
        unsafe {
            (*IMAGE.cellules.get()).copy_from_slice(&self.cellules);
            *IMAGE.lignes.get() = self.lignes_sales;
        }
        self.lignes_sales = 0;
        true
    }

    fn newline(&mut self) {
        self.col = 0;
        if self.row + 1 >= VGA_HEIGHT {
            self.scroll();
        } else {
            self.row += 1;
        }
    }

    fn scroll(&mut self) {
        self.charge();
        self.cellules.copy_within(VGA_WIDTH.., 0);
        let vide = (self.color as u16) << 8 | b' ' as u16;
        self.cellules[CELLULES - VGA_WIDTH..].fill(vide);
        self.lignes_sales = TOUTES_LES_LIGNES;
        self.row = VGA_HEIGHT - 1;
        self.col = 0;
    }

    fn backspace(&mut self) {
        if self.col > 0 {
            self.col -= 1;
            self.write_cell(self.row, self.col, b' ', self.color);
        }
    }

    fn byte(&mut self, byte: u8) {
        match byte {
            b'\n' => self.newline(),
            8 => self.backspace(),
            byte => {
                if self.col >= VGA_WIDTH {
                    self.newline();
                }
                self.write_cell(self.row, self.col, byte, self.color);
                self.col += 1;
            }
        }
    }
}

impl fmt::Write for VgaWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            match byte {
                0x08 | 0x20..=0x7e | b'\n' => self.byte(byte),
                _ => self.byte(b'?'),
            }
        }
        Ok(())
    }
}

/// Efface l'ecran et replace le curseur en haut a gauche.
pub fn clear() {
    let a_publier = avec_console(|console| {
        #[cfg(feature = "reference-desktop")]
        {
            console.vga.row = 0;
            console.vga.col = 0;
            false
        }

        #[cfg(not(feature = "reference-desktop"))]
        {
            console.vga.clear();
            console.vga.prepare_publication()
        }
    });
    if a_publier == Some(true) {
        publie_hors_verrou();
    }
}

/// BOUCHAUD_VGA_TEXTE_OMBRE_V1 : detenteur du jeton de publication, HORS du
/// verrou de la console : recopie les lignes modifiees de `IMAGE` sur la
/// memoire texte, par mots de 32 bits, sans aucune lecture ; puis rend le
/// jeton et repart si quelqu'un l'a trouve occupe entre-temps.
fn publie_hors_verrou() {
    loop {
        let debut = crate::kernel::timer::monotonic_ns();
        // BOUCHAUD_STAGE2_NO_LEGACY_VGA_MMIO : sous `reference-desktop`
        // (UEFI/GOP), 0xB8000 n'est pas un backend valide.
        #[cfg(not(feature = "reference-desktop"))]
        {
            // SAFETY : jeton tenu -- seul acces a `IMAGE`.
            let (cellules, mut sales) = unsafe { (&*IMAGE.cellules.get(), *IMAGE.lignes.get()) };
            while sales != 0 {
                let ligne = sales.trailing_zeros() as usize;
                sales &= sales - 1;
                let source = &cellules[ligne * VGA_WIDTH..(ligne + 1) * VGA_WIDTH];
                // 80 cellules par ligne : l'adresse reste alignee sur 4 octets.
                let base = (VGA_BUFFER as *mut u32).wrapping_add(ligne * VGA_WIDTH / 2);
                for (k, paire) in source.chunks_exact(2).enumerate() {
                    // SAFETY : ligne < 25, dans la memoire texte 80 x 25.
                    unsafe { core::ptr::write_volatile(base.add(k), paire[0] as u32 | (paire[1] as u32) << 16) };
                }
            }
        }
        PUBLICATION_PIRE_NS.fetch_max(
            crate::kernel::timer::monotonic_ns().saturating_sub(debut),
            Ordering::Relaxed,
        );
        PUBLICATION_EN_COURS.store(false, Ordering::SeqCst);
        if !PUBLICATION_DEMANDEE.load(Ordering::SeqCst) {
            return;
        }
        if avec_console(|console| console.vga.prepare_publication()) != Some(true) {
            return;
        }
    }
}

/// Change la couleur d'affichage courante.
pub fn set_color(color: u8) {
    avec_console(|console| console.vga.set_color(color));
}

/// Positionne le curseur (texte + curseur materiel) sur (row, col).
pub fn set_cursor(row: usize, col: usize) {
    avec_console(|console| {
        let vga = &mut console.vga;
        vga.row = if row >= VGA_HEIGHT { VGA_HEIGHT - 1 } else { row };
        vga.col = if col >= VGA_WIDTH { VGA_WIDTH - 1 } else { col };

        #[cfg(not(feature = "reference-desktop"))]
        {
            let pos = vga.row * VGA_WIDTH + vga.col;
            unsafe {
                outb(0x3D4, 0x0F);
                outb(0x3D5, (pos & 0xFF) as u8);
                outb(0x3D4, 0x0E);
                outb(0x3D5, ((pos >> 8) & 0xFF) as u8);
            }
        }
    });
}

/// Profondeur de journalisation d'une commande de terminal.
static TERMINAL_TRACE_DEPTH: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

pub fn terminal_trace_begin() {
    TERMINAL_TRACE_DEPTH.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
}

pub fn terminal_trace_end() {
    let _ = TERMINAL_TRACE_DEPTH.fetch_update(
        core::sync::atomic::Ordering::Relaxed,
        core::sync::atomic::Ordering::Relaxed,
        |depth| Some(depth.saturating_sub(1)),
    );
}

fn terminal_trace_active() -> bool {
    TERMINAL_TRACE_DEPTH.load(core::sync::atomic::Ordering::Relaxed) != 0
}


/// Demarre une capture (empile un tampon vide).
pub fn capture_start() {
    avec_console(|console| console.captures.push(alloc::string::String::new()));
}

/// Termine la capture courante et renvoie le texte accumule.
pub fn capture_take() -> Option<alloc::string::String> {
    avec_console(|console| console.captures.pop()).flatten()
}

/// Recopie sur COM1 tout ce qui part a l'ecran.
///
/// Sert au mode non interactif : la sortie des commandes doit atteindre l'hote
/// pour qu'il puisse l'analyser. La recopie se fait au fil de l'eau et non a la
/// fin, de sorte qu'une panique laisse quand meme voir tout ce qui l'a precedee.
static SERIAL_MIRROR: AtomicBool = AtomicBool::new(false);

/// Active ou coupe la recopie de la sortie texte vers COM1.
pub fn set_serial_mirror(on: bool) {
    SERIAL_MIRROR.store(on, Ordering::Relaxed);
}

/// La sortie texte est-elle recopiee sur COM1 ?
pub fn serial_mirror() -> bool {
    SERIAL_MIRROR.load(Ordering::Relaxed)
}

/// Implementation reelle derriere les macros `print!` / `println!`.
pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;

    // La blackbox voit la sortie AVANT la capture VGA/pipeline.
    if terminal_trace_active() {
        crate::kernel::blackbox::terminal_sortie(args.clone());
    }

    // Les captures du shell restent prioritaires : les commandes du terminal
    // graphique continuent a reutiliser println! sans toucher au VGA physique.
    // Le test et l'ecriture se font sous la meme prise : un `capture_take`
    // concurrent ne peut pas retirer le tampon entre les deux.
    let capturee = avec_console(|console| match console.captures.last_mut() {
        Some(top) => {
            let _ = top.write_fmt(args);
            true
        }
        None => false,
    });
    if capturee == Some(true) {
        return;
    }

    #[cfg(feature = "reference-desktop")]
    {
        // UEFI/GOP: sortie texte de secours vers COM1 uniquement.
        crate::drivers::serial::_print(args);
        return;
    }

    #[cfg(not(feature = "reference-desktop"))]
    {
        // COM1 hors du verrou de la console : l'emission serie dure 87 us par
        // octet, et elle a deja son propre jeton.
        if serial_mirror() {
            crate::drivers::serial::_print(args);
        }
        // BOUCHAUD_VGA_TEXTE_INVISIBLE_V1 : bureau graphique actif, la memoire
        // texte 0xB8000 n'est pas affichee -- et sous QEMU chaque octet ecrit
        // la-bas est une sortie MMIO (sortie de VM sous KVM), chaque
        // defilement en coute ~4 000, sous le verrou de la console que les
        // autres coeurs attendent. Endurance KVM 37742169261 : `avec_console`
        // porte 55 a 74 % des echantillons noyau de WebContent et du
        // BrowserHost (leur stderr passe par ici). Le texte reste sur COM1 et
        // dans les captures du terminal ; seule la copie invisible disparait.
        // (La Trigkey -- `reference-desktop`, GOP -- n'ecrit deja jamais ici.)
        if crate::drivers::gfx::is_active() {
            return;
        }
        let a_publier = avec_console(|console| {
            let _ = console.vga.write_fmt(args);
            console.vga.prepare_publication()
        });
        if a_publier == Some(true) {
            publie_hors_verrou();
        }
    }
}
