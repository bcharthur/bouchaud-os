//! L'ecran de faute noyau, ecrit directement dans le framebuffer GOP.
//!
//! # Ce qu'une faute ressemblait a, et pourquoi c'etait le pire des cas
//!
//! Les gestionnaires d'exception relevaient un contexte tres complet -- et
//! l'ecrivaient sur COM1. La panique, elle, ecrit dans le tampon texte VGA a
//! `0xB8000`. Sur la machine de reference, demarree en UEFI derriere un
//! framebuffer GOP, ces deux sorties n'existent pas : il n'y a pas de cable
//! serie, et le mode texte VGA n'est pas affiche.
//!
//! Une faute de page dans le noyau produisait donc exactement la meme chose
//! qu'une boucle infinie : un ecran arrete. Impossible de distinguer « le
//! noyau a saute dans le vide » de « le noyau attend un peripherique », et
//! ce sont deux enquetes qui n'ont rien a voir.
//!
//! # Les contraintes du chemin de faute
//!
//! Ce module est appele depuis un gestionnaire d'exception, parfois apres une
//! double faute. Il n'a donc le droit ni d'allouer, ni de prendre un verrou,
//! ni de rasteriser une police vectorielle -- un `Font::from_bytes` sur ce
//! chemin transformerait une faute diagnosticable en triple faute muette.
//!
//! Il n'utilise que : des atomiques, la police bitmap 8x8 deja embarquee, et
//! des ecritures volatiles dans le framebuffer.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering};

use crate::boot::{FramebufferInfo, FramebufferPixelFormat};

// ---------------------------------------------------------------------------
// Le framebuffer, retenu au plus tot
// ---------------------------------------------------------------------------

static FB_ADRESSE: AtomicU64 = AtomicU64::new(0);
static FB_OCTETS: AtomicUsize = AtomicUsize::new(0);
static FB_LARGEUR: AtomicU32 = AtomicU32::new(0);
static FB_HAUTEUR: AtomicU32 = AtomicU32::new(0);
static FB_FOULEE: AtomicU32 = AtomicU32::new(0);
static FB_BPP: AtomicU32 = AtomicU32::new(0);
/// Vrai quand le format est BGR (l'octet de poids faible porte le bleu).
static FB_BGR: AtomicBool = AtomicBool::new(false);

/// Retient le framebuffer que le chemin de faute utilisera.
///
/// A appeler des que Stage 2 en connait un, et AVANT tout pilote susceptible
/// de fauter : un ecran de faute qui n'a pas d'ecran ne sert a rien.
pub fn installe_framebuffer(info: FramebufferInfo) {
    FB_LARGEUR.store(info.width, Ordering::Release);
    FB_HAUTEUR.store(info.height, Ordering::Release);
    FB_FOULEE.store(info.stride, Ordering::Release);
    FB_BPP.store(info.bytes_per_pixel as u32, Ordering::Release);
    FB_BGR.store(
        matches!(info.pixel_format, FramebufferPixelFormat::Bgr),
        Ordering::Release,
    );
    FB_OCTETS.store(info.byte_len, Ordering::Release);
    // L'adresse en DERNIER : c'est elle que le chemin de faute teste pour
    // decider si le reste est utilisable.
    FB_ADRESSE.store(info.address, Ordering::Release);
}

/// Un ecran est-il disponible pour le chemin de faute ?
pub fn ecran_disponible() -> bool {
    FB_ADRESSE.load(Ordering::Acquire) != 0
}

// ---------------------------------------------------------------------------
// Le dernier point de controle du demarrage
// ---------------------------------------------------------------------------

const POINT_MAX: usize = 40;
static POINT_TEXTE: [AtomicU8; POINT_MAX] = [const { AtomicU8::new(0) }; POINT_MAX];
static POINT_LONGUEUR: AtomicUsize = AtomicUsize::new(0);
static POINTS_FRANCHIS: AtomicU32 = AtomicU32::new(0);

/// Note l'etape de demarrage en cours.
///
/// Le nom est RECOPIE dans un tampon fixe plutot que retenu par reference :
/// le chemin de faute ne doit jamais dereferencer un pointeur dont il ne peut
/// pas prouver la validite, et une paire (pointeur, longueur) lue en deux
/// temps peut se lire dechiree.
pub fn point(nom: &str) {
    let octets = nom.as_bytes();
    let n = octets.len().min(POINT_MAX);
    // La longueur passe a zero d'abord : un lecteur concurrent voit alors un
    // point vide, jamais un melange de deux noms.
    POINT_LONGUEUR.store(0, Ordering::Release);
    for i in 0..n {
        POINT_TEXTE[i].store(octets[i], Ordering::Relaxed);
    }
    POINT_LONGUEUR.store(n, Ordering::Release);
    POINTS_FRANCHIS.fetch_add(1, Ordering::Relaxed);
    crate::serial_println!("BOUCHAUD_BOOT_POINT {}", nom);
}

fn ecrit_dernier_point(mut sortie: impl FnMut(u8)) {
    let n = POINT_LONGUEUR.load(Ordering::Acquire).min(POINT_MAX);
    if n == 0 {
        for b in b"(aucun)" {
            sortie(*b);
        }
        return;
    }
    for i in 0..n {
        sortie(POINT_TEXTE[i].load(Ordering::Relaxed));
    }
}

// ---------------------------------------------------------------------------
// Dessin
// ---------------------------------------------------------------------------

const FOND: u32 = 0x0016_0A0A;
const CADRE: u32 = 0x00C8_3232;
const TITRE: u32 = 0x00FF_6B6B;
const TEXTE: u32 = 0x00EF_F3F8;
const ETIQUETTE: u32 = 0x009D_A8B8;

#[inline]
unsafe fn pose_pixel(x: u32, y: u32, couleur: u32) {
    let base = FB_ADRESSE.load(Ordering::Relaxed);
    let bpp = FB_BPP.load(Ordering::Relaxed);
    let foulee = FB_FOULEE.load(Ordering::Relaxed);
    if x >= FB_LARGEUR.load(Ordering::Relaxed) || y >= FB_HAUTEUR.load(Ordering::Relaxed) {
        return;
    }
    let decalage = (y as usize) * (foulee as usize) * (bpp as usize) + (x as usize) * (bpp as usize);
    if decalage + (bpp as usize) > FB_OCTETS.load(Ordering::Relaxed) {
        return;
    }
    let (r, v, b) = (
        ((couleur >> 16) & 0xFF) as u8,
        ((couleur >> 8) & 0xFF) as u8,
        (couleur & 0xFF) as u8,
    );
    let pixel = (base as *mut u8).add(decalage);
    if bpp >= 3 {
        if FB_BGR.load(Ordering::Relaxed) {
            core::ptr::write_volatile(pixel, b);
            core::ptr::write_volatile(pixel.add(1), v);
            core::ptr::write_volatile(pixel.add(2), r);
        } else {
            core::ptr::write_volatile(pixel, r);
            core::ptr::write_volatile(pixel.add(1), v);
            core::ptr::write_volatile(pixel.add(2), b);
        }
    } else if bpp == 1 {
        let gris = ((r as u32 * 30 + v as u32 * 59 + b as u32 * 11) / 100) as u8;
        core::ptr::write_volatile(pixel, gris);
    }
}

unsafe fn remplit(x0: u32, y0: u32, largeur: u32, hauteur: u32, couleur: u32) {
    for y in y0..y0.saturating_add(hauteur) {
        for x in x0..x0.saturating_add(largeur) {
            pose_pixel(x, y, couleur);
        }
    }
}

/// Dessine un caractere ASCII a l'echelle demandee.
unsafe fn glyphe(x: u32, y: u32, c: u8, echelle: u32, couleur: u32) {
    let motif = crate::drivers::gfx::font::glyph(c);
    for (ligne, bits) in motif.iter().enumerate() {
        for colonne in 0..8u32 {
            if bits & (1 << colonne) == 0 {
                continue;
            }
            for dy in 0..echelle {
                for dx in 0..echelle {
                    pose_pixel(
                        x + colonne * echelle + dx,
                        y + ligne as u32 * echelle + dy,
                        couleur,
                    );
                }
            }
        }
    }
}

unsafe fn texte(x: u32, y: u32, s: &str, echelle: u32, couleur: u32) -> u32 {
    let mut curseur = x;
    for c in s.bytes() {
        glyphe(curseur, y, c, echelle, couleur);
        curseur += 8 * echelle;
    }
    curseur
}

unsafe fn hexa(x: u32, y: u32, valeur: u64, echelle: u32, couleur: u32) -> u32 {
    const CHIFFRES: &[u8; 16] = b"0123456789ABCDEF";
    let mut curseur = texte(x, y, "0x", echelle, couleur);
    let mut commence = false;
    for quartet in (0..16).rev() {
        let d = ((valeur >> (quartet * 4)) & 0xF) as usize;
        if d == 0 && !commence && quartet != 0 {
            continue;
        }
        commence = true;
        glyphe(curseur, y, CHIFFRES[d], echelle, couleur);
        curseur += 8 * echelle;
    }
    curseur
}

// ---------------------------------------------------------------------------
// L'ecran
// ---------------------------------------------------------------------------

/// Un seul ecran de faute. Le second CPU qui faute n'ecrase pas le premier
/// releve -- c'est celui-la qui porte la cause.
static DEJA_AFFICHE: AtomicBool = AtomicBool::new(false);

/// Affiche l'ecran de faute fatale.
///
/// `cr2` n'est renseigne que pour une faute de page ; ailleurs, le registre ne
/// dit rien de la faute en cours et l'afficher induirait en erreur.
pub fn affiche(
    vecteur: u8,
    nom: &str,
    rip: u64,
    rsp: u64,
    rflags: u64,
    code: u64,
    cr2: Option<u64>,
) {
    if !ecran_disponible() {
        return;
    }
    if DEJA_AFFICHE.swap(true, Ordering::AcqRel) {
        return;
    }

    let largeur = FB_LARGEUR.load(Ordering::Acquire);
    let hauteur = FB_HAUTEUR.load(Ordering::Acquire);
    // Une echelle par tranche de 640 pixels : lisible en 800x600 comme en
    // 1920x1080, sans calcul de mise en page.
    let echelle = (largeur / 640).clamp(1, 3);
    let marge = 16 * echelle;
    let pas = 12 * echelle;

    unsafe {
        remplit(0, 0, largeur, hauteur, FOND);
        remplit(0, 0, largeur, 2 * echelle, CADRE);
        remplit(0, hauteur.saturating_sub(2 * echelle), largeur, 2 * echelle, CADRE);

        let mut y = marge;
        texte(marge, y, "BOUCHAUD KERNEL FAULT", echelle * 2, TITRE);
        y += 20 * echelle + pas;

        texte(marge, y, "VECTEUR   ", echelle, ETIQUETTE);
        let x = marge + 10 * 8 * echelle;
        let fin = hexa(x, y, vecteur as u64, echelle, TEXTE);
        texte(fin + 8 * echelle, y, nom, echelle, TEXTE);
        y += pas;

        texte(marge, y, "RIP       ", echelle, ETIQUETTE);
        hexa(x, y, rip, echelle, TEXTE);
        y += pas;

        texte(marge, y, "RSP       ", echelle, ETIQUETTE);
        hexa(x, y, rsp, echelle, TEXTE);
        y += pas;

        texte(marge, y, "RFLAGS    ", echelle, ETIQUETTE);
        hexa(x, y, rflags, echelle, TEXTE);
        y += pas;

        texte(marge, y, "CODE      ", echelle, ETIQUETTE);
        hexa(x, y, code, echelle, TEXTE);
        y += pas;

        texte(marge, y, "CR2       ", echelle, ETIQUETTE);
        match cr2 {
            Some(adresse) => {
                hexa(x, y, adresse, echelle, TEXTE);
            }
            None => {
                texte(x, y, "-", echelle, ETIQUETTE);
            }
        }
        y += pas;

        texte(marge, y, "CPU       ", echelle, ETIQUETTE);
        hexa(x, y, crate::arch::x86_64::smp::cpu_index() as u64, echelle, TEXTE);
        y += pas + pas;

        texte(marge, y, "DERNIER POINT DE DEMARRAGE", echelle, ETIQUETTE);
        y += pas;
        let mut curseur = marge;
        ecrit_dernier_point(|octet| {
            glyphe(curseur, y, octet, echelle, TEXTE);
            curseur += 8 * echelle;
        });
        y += pas;
        texte(marge, y, "POINTS FRANCHIS ", echelle, ETIQUETTE);
        hexa(
            marge + 16 * 8 * echelle,
            y,
            POINTS_FRANCHIS.load(Ordering::Relaxed) as u64,
            echelle,
            TEXTE,
        );
        y += pas + pas;

        texte(
            marge,
            y,
            "LA MACHINE EST ARRETEE. COUPEZ L'ALIMENTATION POUR REDEMARRER.",
            echelle,
            ETIQUETTE,
        );
    }
}

/// Affiche l'ecran d'une panique noyau -- une faute que le noyau a DECIDEE,
/// pas une exception materielle.
///
/// Il n'y a pas de registres a montrer ici : le contexte utile est le lieu de
/// la panique et l'etape de demarrage franchie. Afficher un `RIP` a zero pour
/// remplir la mise en page serait pire que de ne rien afficher.
pub fn affiche_panique(fichier: &str, ligne: u32) {
    if !ecran_disponible() {
        return;
    }
    if DEJA_AFFICHE.swap(true, Ordering::AcqRel) {
        return;
    }

    let largeur = FB_LARGEUR.load(Ordering::Acquire);
    let hauteur = FB_HAUTEUR.load(Ordering::Acquire);
    let echelle = (largeur / 640).clamp(1, 3);
    let marge = 16 * echelle;
    let pas = 12 * echelle;

    unsafe {
        remplit(0, 0, largeur, hauteur, FOND);
        remplit(0, 0, largeur, 2 * echelle, CADRE);
        remplit(0, hauteur.saturating_sub(2 * echelle), largeur, 2 * echelle, CADRE);

        let mut y = marge;
        texte(marge, y, "BOUCHAUD KERNEL PANIC", echelle * 2, TITRE);
        y += 20 * echelle + pas;

        texte(marge, y, "LIEU      ", echelle, ETIQUETTE);
        let x = marge + 10 * 8 * echelle;
        // Le chemin est tronque par la GAUCHE : la fin d'un chemin identifie
        // le fichier, son debut ne fait que repeter l'arborescence du depot.
        let colonnes = ((largeur.saturating_sub(x + marge)) / (8 * echelle)) as usize;
        let court = if fichier.len() > colonnes {
            &fichier[fichier.len() - colonnes..]
        } else {
            fichier
        };
        texte(x, y, court, echelle, TEXTE);
        y += pas;

        texte(marge, y, "LIGNE     ", echelle, ETIQUETTE);
        hexa(x, y, ligne as u64, echelle, TEXTE);
        y += pas;

        texte(marge, y, "CPU       ", echelle, ETIQUETTE);
        hexa(x, y, crate::arch::x86_64::smp::cpu_index() as u64, echelle, TEXTE);
        y += pas + pas;

        texte(marge, y, "DERNIER POINT DE DEMARRAGE", echelle, ETIQUETTE);
        y += pas;
        let mut curseur = marge;
        ecrit_dernier_point(|octet| {
            glyphe(curseur, y, octet, echelle, TEXTE);
            curseur += 8 * echelle;
        });
        y += pas + pas;

        texte(
            marge,
            y,
            "LE DETAIL COMPLET EST SUR COM1.",
            echelle,
            ETIQUETTE,
        );
    }
}
