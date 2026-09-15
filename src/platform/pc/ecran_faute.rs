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
//! ni d'initialiser un rasteriseur TTF -- un `Font::from_bytes` sur ce
//! chemin transformerait une faute diagnosticable en triple faute muette.
//!
//! Les glyphes DejaVu Sans sont donc rasterises et anticreneles a la
//! compilation, puis inclus comme atlas alpha immuables. Le chemin de faute
//! n'utilise que des atomiques, ces octets statiques et des ecritures
//! volatiles dans le framebuffer.

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
fn memorise_point(nom: &str) {
    let octets = nom.as_bytes();
    let n = octets.len().min(POINT_MAX);
    POINT_LONGUEUR.store(0, Ordering::Release);
    for i in 0..n {
        POINT_TEXTE[i].store(octets[i], Ordering::Relaxed);
    }
    POINT_LONGUEUR.store(n, Ordering::Release);
    POINTS_FRANCHIS.fetch_add(1, Ordering::Relaxed);
}

pub fn point(nom: &str) {
    memorise_point(nom);
    crate::serial_println!("BOUCHAUD_BOOT_POINT {}", nom);
}

/// Jalon utilisable au milieu d'une commutation de pile : atomiques seulement.
pub fn point_silencieux(nom: &str) {
    memorise_point(nom);
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

/// Le dernier jalon est-il exactement `nom` ?
fn dernier_point_est(nom: &str) -> bool {
    let octets = nom.as_bytes();
    let n = POINT_LONGUEUR.load(Ordering::Acquire).min(POINT_MAX);
    if n != octets.len() { return false; }
    (0..n).all(|i| POINT_TEXTE[i].load(Ordering::Relaxed) == octets[i])
}

// Premiere exception entree avant une eventuelle double faute. Les donnees
// sont publiees avant le vecteur, qui sert de drapeau Release/Acquire.
const AUCUNE_EXCEPTION: u8 = 0xFF;
static PREMIERE_RESERVEE: AtomicBool = AtomicBool::new(false);
static PREMIER_VECTEUR: AtomicU8 = AtomicU8::new(AUCUNE_EXCEPTION);
static PREMIER_RIP: AtomicU64 = AtomicU64::new(0);
static PREMIER_CODE: AtomicU64 = AtomicU64::new(0);
static PREMIER_CR2: AtomicU64 = AtomicU64::new(0);
static PREMIER_CR2_VALIDE: AtomicBool = AtomicBool::new(false);

pub fn entre_exception(vecteur: u8, rip: u64, code: u64, cr2: Option<u64>) {
    if PREMIERE_RESERVEE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    PREMIER_RIP.store(rip, Ordering::Relaxed);
    PREMIER_CODE.store(code, Ordering::Relaxed);
    if let Some(adresse) = cr2 {
        PREMIER_CR2.store(adresse, Ordering::Relaxed);
        PREMIER_CR2_VALIDE.store(true, Ordering::Relaxed);
    }
    PREMIER_VECTEUR.store(vecteur, Ordering::Release);
}

/// Une faute de page resolue n'est plus une exception active.
pub fn sort_exception_resolue() {
    PREMIER_VECTEUR.store(AUCUNE_EXCEPTION, Ordering::Release);
    PREMIER_CR2_VALIDE.store(false, Ordering::Relaxed);
    PREMIERE_RESERVEE.store(false, Ordering::Release);
}

fn premiere_exception_lue() -> (u8, u64, u64, Option<u64>) {
    let vecteur = PREMIER_VECTEUR.load(Ordering::Acquire);
    (
        vecteur,
        PREMIER_RIP.load(Ordering::Relaxed),
        PREMIER_CODE.load(Ordering::Relaxed),
        PREMIER_CR2_VALIDE.load(Ordering::Relaxed)
            .then(|| PREMIER_CR2.load(Ordering::Relaxed)),
    )
}

// ---------------------------------------------------------------------------
// Dessin
// ---------------------------------------------------------------------------

const FOND: u32 = 0x0016_0A0A;
const CADRE: u32 = 0x00C8_3232;
const TITRE: u32 = 0x00FF_6B6B;
const TEXTE: u32 = 0x00EF_F3F8;
const ETIQUETTE: u32 = 0x009D_A8B8;
const MUTED: u32 = 0x0078_8496;
const WARN: u32 = 0x00F5_BE4A;

/// Lignes de trace noyau affichees sous le releve, au plus.
const TRACE_LIGNES: usize = 12;

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

// Atlas DejaVu Sans generes par build.rs. Chaque entree couvre les 95
// caracteres ASCII imprimables dans une cellule fixe 8s x 10s.
const FAULT_FONT_1: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/fault-font-1.bin"));
const FAULT_FONT_2: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/fault-font-2.bin"));
const FAULT_FONT_3: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/fault-font-3.bin"));
const FAULT_FONT_4: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/fault-font-4.bin"));
const FAULT_FONT_5: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/fault-font-5.bin"));
const FAULT_FONT_6: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/fault-font-6.bin"));

#[inline]
fn atlas_faute(echelle: u32) -> (&'static [u8], usize) {
    match echelle.clamp(1, 6) {
        1 => (FAULT_FONT_1, 1),
        2 => (FAULT_FONT_2, 2),
        3 => (FAULT_FONT_3, 3),
        4 => (FAULT_FONT_4, 4),
        5 => (FAULT_FONT_5, 5),
        _ => (FAULT_FONT_6, 6),
    }
}

#[inline]
unsafe fn lit_pixel(x: u32, y: u32) -> u32 {
    let base = FB_ADRESSE.load(Ordering::Relaxed);
    let bpp = FB_BPP.load(Ordering::Relaxed);
    let foulee = FB_FOULEE.load(Ordering::Relaxed);
    if bpp < 3 || x >= FB_LARGEUR.load(Ordering::Relaxed) || y >= FB_HAUTEUR.load(Ordering::Relaxed) {
        return 0;
    }
    let decalage = y as usize * foulee as usize * bpp as usize + x as usize * bpp as usize;
    if decalage + bpp as usize > FB_OCTETS.load(Ordering::Relaxed) { return 0; }
    let pixel = (base as *const u8).add(decalage);
    let a = core::ptr::read_volatile(pixel) as u32;
    let b = core::ptr::read_volatile(pixel.add(1)) as u32;
    let c = core::ptr::read_volatile(pixel.add(2)) as u32;
    if FB_BGR.load(Ordering::Relaxed) { (c << 16) | (b << 8) | a } else { (a << 16) | (b << 8) | c }
}

#[inline]
unsafe fn pose_pixel_alpha(x: u32, y: u32, couleur: u32, alpha: u8) {
    if alpha == 0 { return; }
    if alpha == 255 { pose_pixel(x, y, couleur); return; }
    let fond = lit_pixel(x, y);
    let a = alpha as u32;
    let inv = 255 - a;
    let melange = |decalage: u32| {
        ((((couleur >> decalage) & 0xff) * a + ((fond >> decalage) & 0xff) * inv + 127) / 255) & 0xff
    };
    pose_pixel(x, y, (melange(16) << 16) | (melange(8) << 8) | melange(0));
}

/// Dessine un caractere ASCII DejaVu Sans sans allocation ni verrou.
unsafe fn glyphe(x: u32, y: u32, c: u8, echelle: u32, couleur: u32) {
    let (atlas, scale) = atlas_faute(echelle);
    let ascii = if (32..=126).contains(&c) { c } else { b'?' };
    let largeur = 8 * scale;
    let hauteur = 10 * scale;
    let base = (ascii - 32) as usize * largeur * hauteur;
    for ligne in 0..hauteur {
        for colonne in 0..largeur {
            let alpha = atlas[base + ligne * largeur + colonne];
            pose_pixel_alpha(x + colonne as u32, y + ligne as u32, couleur, alpha);
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
    cs: u64,
    ss: u64,
) {
    if !ecran_disponible() || DEJA_AFFICHE.swap(true, Ordering::AcqRel) {
        return;
    }

    let largeur = FB_LARGEUR.load(Ordering::Acquire);
    let hauteur = FB_HAUTEUR.load(Ordering::Acquire);
    // A 1080p, 16 px suffit et laisse plus de vingt lignes au diagnostic.
    let echelle = (largeur / 960).clamp(1, 2);
    let marge = 12 * echelle;
    let pas = 12 * echelle;
    let x = marge + 18 * 8 * echelle;
    let tache = crate::kernel::task::identite_pour_faute();
    let nom_tache = if tache.is_some() {
        crate::kernel::task::nom_pour_faute()
    } else {
        "<aucune>"
    };
    let premiere = premiere_exception_lue();

    unsafe {
        remplit(0, 0, largeur, hauteur, FOND);
        remplit(0, 0, largeur, 2 * echelle, CADRE);
        remplit(0, hauteur.saturating_sub(2 * echelle), largeur, 2 * echelle, CADRE);

        let mut y = marge;
        texte(marge, y, "BOUCHAUD KERNEL FAULT", echelle * 2, TITRE);
        y += 20 * echelle + pas;

        texte(marge, y, "VECTEUR", echelle, ETIQUETTE);
        let fin = hexa(x, y, vecteur as u64, echelle, TEXTE);
        texte(fin + 8 * echelle, y, nom, echelle, TEXTE);
        y += pas;

        texte(marge, y, "RIP", echelle, ETIQUETTE);
        hexa(x, y, rip, echelle, TEXTE);
        y += pas;

        texte(marge, y, "RSP", echelle, ETIQUETTE);
        let fin = hexa(x, y, rsp, echelle, TEXTE);
        if !canonique(rsp) {
            texte(fin + 8 * echelle, y, "NON CANONIQUE", echelle, TITRE);
        } else if let Some((_, _, _, sommet, base, _)) = tache {
            texte(
                fin + 8 * echelle,
                y,
                if rsp >= base && rsp <= sommet { "DANS PILE" } else { "HORS PILE" },
                echelle,
                if rsp >= base && rsp <= sommet { TEXTE } else { TITRE },
            );
        } else {
            // AUCUNE TACHE : DIRE CE QU'ON SAIT, ET RIEN DE PLUS.
            //
            // Le noyau ne connait au runtime que les bornes des piles de
            // taches. La pile d'amorcage est allouee par le chargeur
            // `bootloader_api`, qui ne transmet ni sa base ni son sommet :
            // inventer une borne donnerait une precision que personne n'a
            // mesuree. « Hors piles taches » est exact ; « pile corrompue »
            // ne l'est pas, et le releve du 14 septembre l'a prouve -- deux
            // fautes successives y donnent `0x10000014d50` et
            // `0x10000014d70`, trente-deux octets d'ecart, soit deux
            // profondeurs d'appel de la MEME pile parfaitement valide.
            let fin = texte(
                fin + 8 * echelle,
                y,
                "HORS PILES TACHES",
                echelle,
                ETIQUETTE,
            );
            texte(fin + 8 * echelle, y, "(amorcage probable)", echelle, ETIQUETTE);
        }
        y += pas;

        texte(marge, y, "RSP MOD16", echelle, ETIQUETTE);
        hexa(x, y, rsp & 0xF, echelle, if rsp & 0xF == 8 { TEXTE } else { WARN });
        y += pas;

        texte(marge, y, "CS / SS", echelle, ETIQUETTE);
        let fin = hexa(x, y, cs, echelle, TEXTE);
        let fin = texte(fin + 8 * echelle, y, "/", echelle, ETIQUETTE);
        hexa(fin + 8 * echelle, y, ss, echelle, TEXTE);
        y += pas;

        texte(marge, y, "ANNEAU", echelle, ETIQUETTE);
        let fin = hexa(x, y, cs & 3, echelle, TEXTE);
        texte(fin + 8 * echelle, y, if cs & 3 == 3 { "UTILISATEUR" } else { "NOYAU" }, echelle, TEXTE);
        y += pas;

        texte(marge, y, "RFLAGS", echelle, ETIQUETTE);
        hexa(x, y, rflags, echelle, TEXTE);
        y += pas;

        texte(marge, y, "CODE", echelle, ETIQUETTE);
        hexa(x, y, code, echelle, TEXTE);
        y += pas;

        texte(marge, y, if vecteur == 8 { "CR2 (INDICE)" } else { "CR2" }, echelle, ETIQUETTE);
        match cr2 {
            Some(adresse) => { hexa(x, y, adresse, echelle, TEXTE); }
            None => { texte(x, y, "-", echelle, ETIQUETTE); }
        }
        y += pas;

        texte(marge, y, "CPU", echelle, ETIQUETTE);
        hexa(x, y, crate::arch::x86_64::smp::cpu_index() as u64, echelle, TEXTE);
        y += pas;

        texte(marge, y, "TACHE", echelle, ETIQUETTE);
        if let Some((index, pid, tid, _, _, in_kernel)) = tache {
            let fin = hexa(x, y, index as u64, echelle, TEXTE);
            let fin = texte(fin + 8 * echelle, y, "PID", echelle, ETIQUETTE);
            let fin = hexa(fin + 8 * echelle, y, pid as u64, echelle, TEXTE);
            let fin = texte(fin + 8 * echelle, y, "TID", echelle, ETIQUETTE);
            let fin = hexa(fin + 8 * echelle, y, tid as u64, echelle, TEXTE);
            texte(fin + 8 * echelle, y, if in_kernel { "KERNEL" } else { "USER" }, echelle, TEXTE);
        } else {
            texte(x, y, "<aucune>", echelle, ETIQUETTE);
        }
        y += pas;

        texte(marge, y, "NOM TACHE", echelle, ETIQUETTE);
        texte(x, y, nom_tache, echelle, TEXTE);
        y += pas;

        if let Some((_, _, _, sommet, base, _)) = tache {
            texte(marge, y, "PILE BASE", echelle, ETIQUETTE);
            hexa(x, y, base, echelle, TEXTE);
            y += pas;
            texte(marge, y, "PILE SOMMET", echelle, ETIQUETTE);
            hexa(x, y, sommet, echelle, TEXTE);
            y += pas;
            texte(marge, y, "PILE LIBRE", echelle, ETIQUETTE);
            hexa(x, y, rsp.saturating_sub(base), echelle, TEXTE);
            y += pas;
        }

        // LA PORTE ABSENTE, SI LE PROCESSEUR L'A NOMMEE.
        //
        // Un #NP sur porte IDT absente est le SEUL mecanisme qui dit quel
        // vecteur a ete demande. Tant qu'il n'etait pas installe, une porte
        // manquante se lisait « DOUBLE FAULT » sans autre information.
        if let Some((vecteur_absent, code_np, rip_np, rsp_np)) =
            crate::arch::x86_64::idt::premiere_porte_absente()
        {
            texte(marge, y, "PORTE IDT ABSENTE", echelle, ETIQUETTE);
            let fin = hexa(x, y, vecteur_absent as u64, echelle, TITRE);
            let fin = texte(fin + 8 * echelle, y, "CODE", echelle, ETIQUETTE);
            hexa(fin + 8 * echelle, y, code_np, echelle, TEXTE);
            y += pas;
            texte(marge, y, "RIP #NP", echelle, ETIQUETTE);
            let fin = hexa(x, y, rip_np, echelle, TEXTE);
            let fin = texte(fin + 8 * echelle, y, "RSP", echelle, ETIQUETTE);
            hexa(fin + 8 * echelle, y, rsp_np, echelle, TEXTE);
            y += pas;
        }

        texte(marge, y, "EXCEPTION INITIALE", echelle, ETIQUETTE);
        if premiere.0 == AUCUNE_EXCEPTION {
            texte(x, y, "<non entree dans un gestionnaire>", echelle, ETIQUETTE);
        } else {
            let fin = hexa(x, y, premiere.0 as u64, echelle, TITRE);
            texte(fin + 8 * echelle, y, "AVANT DOUBLE FAUTE", echelle, TITRE);
        }
        y += pas;
        if premiere.0 != AUCUNE_EXCEPTION {
            texte(marge, y, "RIP INITIAL", echelle, ETIQUETTE);
            hexa(x, y, premiere.1, echelle, TEXTE);
            y += pas;
            texte(marge, y, "CODE INITIAL", echelle, ETIQUETTE);
            hexa(x, y, premiere.2, echelle, TEXTE);
            if let Some(adresse) = premiere.3 {
                let fin = texte(x + 20 * 8 * echelle, y, "CR2", echelle, ETIQUETTE);
                hexa(fin + 8 * echelle, y, adresse, echelle, TEXTE);
            }
            y += pas;
        }

        texte(marge, y, "PISTE PRINCIPALE", echelle, ETIQUETTE);
        let piste = if vecteur != 8 {
            "EXCEPTION FATALE DIRECTE"
        } else if premiere.0 != AUCUNE_EXCEPTION {
            "LE GESTIONNAIRE INITIAL A REFAUTE"
        } else if dernier_point_est("run-noyau-switch") || dernier_point_est("fil-noyau-trampoline") {
            "CADRE INITIAL / ALIGNEMENT DE PILE"
        } else {
            "LIVRAISON D'EXCEPTION IMPOSSIBLE"
        };
        texte(x, y, piste, echelle, TITRE);
        y += pas + pas;

        texte(marge, y, "DERNIER POINT", echelle, ETIQUETTE);
        let mut curseur = x;
        ecrit_dernier_point(|octet| {
            glyphe(curseur, y, octet, echelle, TEXTE);
            curseur += 8 * echelle;
        });
        y += pas;

        texte(marge, y, "POINTS / REFUS / PILES", echelle, ETIQUETTE);
        let fin = hexa(x, y, POINTS_FRANCHIS.load(Ordering::Relaxed) as u64, echelle, TEXTE);
        let refuses = crate::kernel::heap::liens_refuses();
        let fin = hexa(fin + 8 * echelle, y, refuses, echelle, if refuses == 0 { TEXTE } else { TITRE });
        hexa(
            fin + 8 * echelle,
            y,
            crate::kernel::task::piles_corrompues(),
            echelle,
            if crate::kernel::task::piles_corrompues() == 0 { TEXTE } else { TITRE },
        );
        y += pas + pas;

        dessine_trace(marge, y, largeur, hauteur, echelle, pas);
    }
}

/// Une adresse est-elle canonique en x86-64 ?
///
/// Les bits 63:47 doivent tous valoir le bit 47. Une valeur qui viole cette
/// regle ne peut PAS etre une adresse : le processeur la refuse avant meme de
/// consulter la pagination. Le dire nommement evite de chercher une page
/// manquante pour une valeur qui n'a jamais designe de page.
fn canonique(adresse: u64) -> bool {
    let haut = adresse >> 47;
    haut == 0 || haut == 0x1FFFF
}

/// Les dernieres lignes de la trace noyau, lues SANS allouer.
///
/// # Pourquoi elles valent le detour
///
/// Les registres disent ou la machine est tombee. Ils ne disent pas ce
/// qu'elle FAISAIT. Sur une machine sans cable serie, ces quelques lignes sont
/// la seule facon de savoir si la faute suit une scrutation USB, un
/// achevement NVMe ou un lancement de processus.
///
/// La lecture se fait octet par octet dans l'anneau atomique du port serie :
/// aucune allocation, aucun verrou. `trace_snapshot` aurait alloue soixante-
/// quatre kilooctets sur un tas peut-etre deja corrompu.
fn dessine_trace(marge: u32, mut y: u32, largeur: u32, hauteur: u32, echelle: u32, pas: u32) {
    let colonnes = ((largeur.saturating_sub(marge * 2)) / (8 * echelle)) as usize;
    if colonnes == 0 {
        return;
    }
    // Ce qui reste de hauteur decide du nombre de lignes : mieux vaut en
    // montrer moins que d'ecrire hors de l'ecran.
    let disponibles = hauteur.saturating_sub(y + marge) / pas;
    let lignes = (disponibles as usize).min(TRACE_LIGNES);
    if lignes == 0 {
        return;
    }

    let (debut, fin) = crate::drivers::serial::trace_bornes();
    if fin == debut {
        return;
    }

    // Remonter l'anneau a l'envers jusqu'a avoir compte `lignes` retours a la
    // ligne, puis reafficher dans l'ordre.
    let mut depart = fin;
    let mut comptees = 0usize;
    while depart > debut && comptees <= lignes {
        depart -= 1;
        if crate::drivers::serial::trace_octet(depart) == b'\n' {
            comptees += 1;
        }
    }

    unsafe {
        texte(marge, y, "DERNIERES LIGNES DU NOYAU", echelle, ETIQUETTE);
        y += pas;

        let mut colonne = 0usize;
        let mut restantes = lignes;
        let mut etat_ansi = 0u8;
        for sequence in depart..fin {
            if restantes == 0 { break; }
            let octet = crate::drivers::serial::trace_octet(sequence);
            match etat_ansi {
                1 => {
                    etat_ansi = if octet == b'[' { 2 } else { 0 };
                    continue;
                }
                2 => {
                    if (0x40..=0x7e).contains(&octet) { etat_ansi = 0; }
                    continue;
                }
                _ => {}
            }
            match octet {
                0x1b => etat_ansi = 1,
                b'\n' => {
                    y += pas;
                    colonne = 0;
                    restantes -= 1;
                }
                b'\r' => {}
                0x20..=0x7E => {
                    if colonne < colonnes {
                        glyphe(marge + colonne as u32 * 8 * echelle, y, octet, echelle, MUTED);
                        colonne += 1;
                    }
                }
                _ => {
                    if colonne < colonnes {
                        glyphe(marge + colonne as u32 * 8 * echelle, y, b'?', echelle, MUTED);
                        colonne += 1;
                    }
                }
            }
        }
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
    let echelle = (largeur / 960).clamp(1, 2);
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
