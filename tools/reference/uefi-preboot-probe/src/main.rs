#![no_main]
#![no_std]
extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;

use uefi::boot::LoadImageSource;
use uefi::fs::FileSystem;
use uefi::proto::console::gop::{BltOp, BltPixel, BltRegion, GraphicsOutput};
use uefi::proto::console::pointer::Pointer;
use uefi::proto::console::text::Input;
use uefi::proto::device_path::LoadedImageDevicePath;
use uefi::proto::unsafe_protocol;
use uefi::{boot, cstr16, entry, Handle, Status};

#[repr(C)]
#[unsafe_protocol("bd8c1056-9f36-44ec-92a8-a6337f817986")]
struct EdidActive {
    size_of_edid: u32,
    edid: *const u8,
}

fn read_edid(handle: Handle) -> Vec<u8> {
    let Ok(proto) = boot::open_protocol_exclusive::<EdidActive>(handle) else { return Vec::new() };
    if proto.size_of_edid == 0 || proto.size_of_edid > 4096 || proto.edid.is_null() { return Vec::new(); }
    unsafe { core::slice::from_raw_parts(proto.edid, proto.size_of_edid as usize).to_vec() }
}

fn edid_native(edid: &[u8]) -> Option<(usize, usize)> {
    const MAGIC: [u8; 8] = [0x00,0xff,0xff,0xff,0xff,0xff,0xff,0x00];
    if edid.len() < 72 || edid[0..8] != MAGIC { return None; }
    let d = &edid[54..72];
    if d[0] == 0 && d[1] == 0 { return None; }
    let h = d[2] as usize | (((d[4] >> 4) as usize) << 8);
    let v = d[5] as usize | (((d[7] >> 4) as usize) << 8);
    (h != 0 && v != 0).then_some((h, v))
}

// ===========================================================================
// BOUCHAUD_ECRAN_PREBOOT_V1 : le demarrage dit ce qu'il fait, des le firmware
// ===========================================================================
//
// Ce prechargeur peignait un « B » bleu centre, puis le laissait FIGE pendant
// qu'il selectionnait le mode video, ecrivait son rapport, lisait le chargeur
// et lui passait la main. Le noyau, lui, avait deja son ecran de progression
// -- titre, barre a seize segments, ligne d'etat -- mais il ne l'ouvre qu'a
// `stage2::run`, donc APRES tout cela.
//
// D'ou le releve de l'utilisateur : « le B est toujours la au demarrage ».
// L'ecran du noyau fonctionnait. Il arrivait simplement trop tard pour etre
// celui qu'on regarde.
//
// Le prechargeur porte donc le MEME ecran, avec la meme palette et la meme
// barre, et il le passe au noyau en cours de route. Les deux moities ne
// pretendent pas etre une seule mesure : chacune compte ses propres etapes,
// et une barre qui repartirait a zero en mentant serait pire que pas de barre
// du tout.

/// Palette : celle de `ecran_faute::FOND_DEMARRAGE` et suivantes, a
/// l'identique. Un ecart de teinte au handoff se verrait comme un clignotement.
const FOND: BltPixel = BltPixel::new(0x0D, 0x11, 0x17);
const TEXTE: BltPixel = BltPixel::new(0xEF, 0xF3, 0xF8);
const ACCENT: BltPixel = BltPixel::new(0x44, 0xA8, 0xFF);
const BARRE: BltPixel = BltPixel::new(0x2B, 0x32, 0x3F);
const ETIQUETTE: BltPixel = BltPixel::new(0x6B, 0x76, 0x86);

/// Segments de la barre, comme `ecran_faute::DEMARRAGE_SEGMENTS`.
const SEGMENTS: usize = 16;
/// Etapes que CE prechargeur franchit avant de passer la main.
const ETAPES: usize = 5;

/// Police 8x8, generee par tools/reference/genere-police-preboot.py.
///
/// L'ordre suit `INDEX`; un caractere absent se rend comme une espace.
const INDEX: &[u8] = b" ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.-:/%";
const POLICE: [[u8; 8]; 42] = [
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // espace
    [0x38, 0x44, 0x82, 0x82, 0xfe, 0x82, 0x82, 0x00], // A
    [0xfc, 0x82, 0x82, 0xfc, 0x82, 0x82, 0xfc, 0x00], // B
    [0x3c, 0x42, 0x80, 0x80, 0x80, 0x42, 0x3c, 0x00], // C
    [0xf8, 0x84, 0x82, 0x82, 0x82, 0x84, 0xf8, 0x00], // D
    [0xfe, 0x80, 0x80, 0xf8, 0x80, 0x80, 0xfe, 0x00], // E
    [0xfe, 0x80, 0x80, 0xf8, 0x80, 0x80, 0x80, 0x00], // F
    [0x3c, 0x42, 0x80, 0x9e, 0x82, 0x42, 0x3e, 0x00], // G
    [0x82, 0x82, 0x82, 0xfe, 0x82, 0x82, 0x82, 0x00], // H
    [0x7c, 0x10, 0x10, 0x10, 0x10, 0x10, 0x7c, 0x00], // I
    [0x0e, 0x04, 0x04, 0x04, 0x84, 0x84, 0x78, 0x00], // J
    [0x84, 0x88, 0x90, 0xe0, 0x90, 0x88, 0x84, 0x00], // K
    [0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0xfe, 0x00], // L
    [0x82, 0xc6, 0xaa, 0x92, 0x82, 0x82, 0x82, 0x00], // M
    [0x82, 0xc2, 0xa2, 0x92, 0x8a, 0x86, 0x82, 0x00], // N
    [0x38, 0x44, 0x82, 0x82, 0x82, 0x44, 0x38, 0x00], // O
    [0xfc, 0x82, 0x82, 0xfc, 0x80, 0x80, 0x80, 0x00], // P
    [0x38, 0x44, 0x82, 0x82, 0x8a, 0x44, 0x3a, 0x00], // Q
    [0xfc, 0x82, 0x82, 0xfc, 0x88, 0x84, 0x82, 0x00], // R
    [0x3c, 0x42, 0x80, 0x38, 0x02, 0x84, 0x78, 0x00], // S
    [0xfe, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x00], // T
    [0x82, 0x82, 0x82, 0x82, 0x82, 0x44, 0x38, 0x00], // U
    [0x82, 0x82, 0x82, 0x44, 0x44, 0x28, 0x10, 0x00], // V
    [0x82, 0x82, 0x82, 0x92, 0xaa, 0xc6, 0x82, 0x00], // W
    [0x82, 0x44, 0x28, 0x10, 0x28, 0x44, 0x82, 0x00], // X
    [0x82, 0x44, 0x28, 0x10, 0x10, 0x10, 0x10, 0x00], // Y
    [0xfe, 0x02, 0x04, 0x18, 0x20, 0x40, 0xfe, 0x00], // Z
    [0x38, 0x44, 0x86, 0x8a, 0xb2, 0x44, 0x38, 0x00], // 0
    [0x10, 0x30, 0x50, 0x10, 0x10, 0x10, 0x7c, 0x00], // 1
    [0x38, 0x44, 0x02, 0x0c, 0x30, 0x40, 0xfe, 0x00], // 2
    [0x38, 0x44, 0x02, 0x18, 0x02, 0x44, 0x38, 0x00], // 3
    [0x0c, 0x14, 0x24, 0x44, 0xfe, 0x04, 0x0e, 0x00], // 4
    [0xfe, 0x80, 0xf8, 0x02, 0x02, 0x84, 0x78, 0x00], // 5
    [0x38, 0x44, 0x80, 0xfc, 0x82, 0x44, 0x38, 0x00], // 6
    [0xfe, 0x02, 0x04, 0x08, 0x10, 0x20, 0x20, 0x00], // 7
    [0x38, 0x44, 0x44, 0x38, 0x44, 0x44, 0x38, 0x00], // 8
    [0x38, 0x44, 0x82, 0x7e, 0x02, 0x44, 0x38, 0x00], // 9
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x18, 0x00], // .
    [0x00, 0x00, 0x00, 0x7c, 0x00, 0x00, 0x00, 0x00], // -
    [0x00, 0x18, 0x18, 0x00, 0x18, 0x18, 0x00, 0x00], // :
    [0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x00], // /
    [0xc2, 0xc4, 0x08, 0x10, 0x20, 0x46, 0x86, 0x00], // %
];

fn glyphe(caractere: u8) -> [u8; 8] {
    let caractere = caractere.to_ascii_uppercase();
    match INDEX.iter().position(|&connu| connu == caractere) {
        Some(rang) => POLICE[rang],
        None => POLICE[0],
    }
}

/// Une echelle lisible quelle que soit la definition, comme cote noyau.
fn echelle_pour(largeur: usize) -> usize {
    if largeur >= 2560 { 3 } else if largeur >= 1280 { 2 } else { 1 }
}

/// L'ecran de demarrage du prechargeur.
struct Ecran {
    largeur: usize,
    hauteur: usize,
    echelle: usize,
}

impl Ecran {
    fn nouveau(largeur: usize, hauteur: usize) -> Option<Self> {
        let echelle = echelle_pour(largeur);
        // En dessous, la barre et le titre ne tiennent pas : mieux vaut un
        // fond uni qu'une mise en page qui deborde de l'ecran.
        if largeur < 320 || hauteur < 64 * echelle {
            return None;
        }
        Some(Self { largeur, hauteur, echelle })
    }

    /// Largeur de la zone d'etat, comme `ecran_faute::DEMARRAGE_LARGEUR`.
    fn zone(&self) -> usize {
        (560 * self.echelle / 2).min(self.largeur)
    }

    fn marge(&self) -> usize {
        (self.largeur - self.zone()) / 2
    }

    /// UNE LIGNE DE TEXTE EN UN SEUL TRANSFERT.
    ///
    /// Un `VideoFill` par pixel allume tiendrait des dizaines de milliers
    /// d'appels de protocole pour un seul titre. Le bloc est donc compose en
    /// memoire, puis envoye d'un coup.
    fn texte(
        &self,
        gop: &mut GraphicsOutput,
        x: usize,
        y: usize,
        ligne: &str,
        echelle: usize,
        couleur: BltPixel,
    ) {
        let largeur = ligne.len() * 8 * echelle;
        let hauteur = 8 * echelle;
        // Deborder l'ecran fait echouer le blt en silence, et la ligne
        // disparait entierement. La tronquer garde l'essentiel visible.
        if largeur == 0 || hauteur == 0 || y + hauteur > self.hauteur || x >= self.largeur {
            return;
        }
        let visibles = ((self.largeur - x) / (8 * echelle)).min(ligne.len());
        if visibles == 0 {
            return;
        }
        let largeur = visibles * 8 * echelle;
        let mut tampon = alloc::vec![FOND; largeur * hauteur];
        for (rang, caractere) in ligne.bytes().take(visibles).enumerate() {
            let motif = glyphe(caractere);
            for (haut, bits) in motif.iter().enumerate() {
                for colonne in 0..8usize {
                    if bits & (0x80 >> colonne) == 0 {
                        continue;
                    }
                    for dy in 0..echelle {
                        for dx in 0..echelle {
                            let px = (rang * 8 + colonne) * echelle + dx;
                            let py = haut * echelle + dy;
                            tampon[py * largeur + px] = couleur;
                        }
                    }
                }
            }
        }
        let _ = gop.blt(BltOp::BufferToVideo {
            buffer: &tampon,
            src: BltRegion::Full,
            dest: (x, y),
            dims: (largeur, hauteur),
        });
    }

    fn ouvre(&self, gop: &mut GraphicsOutput) {
        let _ = gop.blt(BltOp::VideoFill {
            color: FOND,
            dest: (0, 0),
            dims: (self.largeur, self.hauteur),
        });
        let titre = "BOUCHAUD OS";
        let echelle = self.echelle * 2;
        let largeur_titre = titre.len() * 8 * echelle;
        let haut = self.hauteur / 2;
        if largeur_titre < self.largeur && haut > 96 {
            self.texte(gop, (self.largeur - largeur_titre) / 2, haut - 96, titre, echelle, TEXTE);
        }
    }

    /// Avance la barre et nomme ce qui commence.
    fn etape(&self, gop: &mut GraphicsOutput, numero: usize, libelle: &str) {
        let zone = self.zone();
        let x = self.marge();
        let y = self.hauteur / 2;
        // Effacer AVANT d'ecrire : une etape au nom court laisserait sinon
        // trainer la fin de la precedente.
        let _ = gop.blt(BltOp::VideoFill {
            color: FOND,
            dest: (x, y),
            dims: (zone, 12 * self.echelle),
        });
        let largeur_libelle = libelle.len() * 8 * self.echelle;
        if largeur_libelle <= zone {
            self.texte(gop, x + (zone - largeur_libelle) / 2, y, libelle, self.echelle, ETIQUETTE);
        }
        let atteints = numero.min(ETAPES) * SEGMENTS / ETAPES;
        let pas = zone / SEGMENTS;
        let plein = pas.saturating_sub(4 * self.echelle).max(1);
        for segment in 0..SEGMENTS {
            let couleur = if segment < atteints { ACCENT } else { BARRE };
            let _ = gop.blt(BltOp::VideoFill {
                color: couleur,
                dest: (x + segment * pas, y + 16 * self.echelle),
                dims: (plein, 2 * self.echelle),
            });
        }
    }

    /// La ligne du bas : ce que le firmware a reellement donne.
    fn bas(&self, gop: &mut GraphicsOutput, ligne: &str) {
        let y = self.hauteur.saturating_sub(24 * self.echelle);
        let _ = gop.blt(BltOp::VideoFill {
            color: FOND,
            dest: (0, y),
            dims: (self.largeur, 8 * self.echelle),
        });
        self.texte(gop, self.marge(), y, ligne, self.echelle, ETIQUETTE);
    }
}

fn run() -> uefi::Result {
    uefi::helpers::init()?;
    let gop_handle = boot::get_handle_for_protocol::<GraphicsOutput>()?;
    let edid = read_edid(gop_handle.clone());
    let native = edid_native(&edid);

    let mut gop = boot::open_protocol_exclusive::<GraphicsOutput>(gop_handle)?;
    let modes: Vec<_> = gop.modes().collect();

    let mut report = String::new();
    let _ = writeln!(report, "BOUCHAUD UEFI PREBOOT PROBE V2");
    let _ = writeln!(report, "edid.bytes={}", edid.len());
    let _ = writeln!(report, "edid.native={}", native.map(|(w,h)| alloc::format!("{}x{}", w, h)).unwrap_or_else(|| String::from("unavailable")));

    for (index, mode) in modes.iter().enumerate() {
        let info = mode.info();
        let (w, h) = info.resolution();
        let _ = writeln!(report, "gop.mode.{}={}x{} stride={} format={:?}", index, w, h, info.stride(), info.pixel_format());
    }

    let selected = native
        .and_then(|wanted| modes.iter().copied().find(|m| m.info().resolution() == wanted))
        .or_else(|| modes.iter().copied().filter(|m| { let (w,h)=m.info().resolution(); h != 0 && w.saturating_mul(9) == h.saturating_mul(16) }).max_by_key(|m| { let (w,h)=m.info().resolution(); w.saturating_mul(h) }))
        .or_else(|| modes.iter().copied().max_by_key(|m| { let (w,h)=m.info().resolution(); w.saturating_mul(h) }));

    if let Some(mode) = selected {
        let (w, h) = mode.info().resolution();
        if gop.set_mode(&mode).is_ok() { let _ = writeln!(report, "gop.selected={}x{}", w, h); }
        else { let _ = writeln!(report, "gop.selected=SET_MODE_FAILED"); }
    }

    let current = gop.current_mode_info();
    let (cw, ch) = current.resolution();
    let _ = writeln!(report, "gop.current={}x{} stride={} format={:?}", cw, ch, current.stride(), current.pixel_format());

    let pointer_count = boot::find_handles::<Pointer>().map(|v| v.len()).unwrap_or(0);
    let keyboard_count = boot::find_handles::<Input>().map(|v| v.len()).unwrap_or(0);
    let _ = writeln!(report, "firmware.pointer_handles={}", pointer_count);
    let _ = writeln!(report, "firmware.keyboard_handles={}", keyboard_count);
    let _ = writeln!(report, "note=written before ExitBootServices; kernel xHCI report follows after handoff");

    // L'ECRAN S'OUVRE ICI, PAS PLUS TOT.
    //
    // Avant `set_mode` la resolution n'est pas encore la bonne : peindre
    // avant obligerait a tout repeindre, et l'utilisateur verrait la bascule.
    let ecran = Ecran::nouveau(cw, ch);
    if let Some(ecran) = ecran.as_ref() {
        ecran.ouvre(&mut gop);
        ecran.etape(&mut gop, 1, "MODE VIDEO");
        // Ce que le firmware a REELLEMENT donne, et non ce qu'on esperait.
        // C'est la ligne qui distingue « l'ecran est noir » de « l'ecran est
        // dans un mode que le noyau ne sait pas reprendre ».
        let mut etat = String::new();
        let _ = write!(
            etat,
            "{}X{} CLAVIERS:{} POINTEURS:{}",
            cw, ch, keyboard_count, pointer_count
        );
        ecran.bas(&mut gop, &etat);
        ecran.etape(&mut gop, 2, "SONDE FIRMWARE");
    }

    let fs_proto = boot::get_image_file_system(boot::image_handle())?;
    let mut fs = FileSystem::new(fs_proto);
    let _ = fs.write(cstr16!("\\BOUCHAUD-PREBOOT.TXT"), report.as_bytes());
    if let Some(ecran) = ecran.as_ref() {
        ecran.etape(&mut gop, 3, "RAPPORT ECRIT");
        ecran.etape(&mut gop, 4, "LECTURE DU CHARGEUR");
    }

    let loader = match fs.read(cstr16!("\\EFI\\BOOT\\BOUCHAUD-LOADER.EFI")) {
        Ok(bytes) => bytes,
        Err(_) => {
            // UN ECHEC QUI SE VOIT. Rendre NOT_FOUND laissait un ecran fige
            // et aucune explication : la machine paraissait simplement lente.
            if let Some(ecran) = ecran.as_ref() {
                ecran.etape(&mut gop, 4, "CHARGEUR INTROUVABLE");
                ecran.bas(&mut gop, "EFI/BOOT/BOUCHAUD-LOADER.EFI ABSENT DE LA CLE");
            }
            return Err(Status::NOT_FOUND.into());
        }
    };
    drop(fs);

    if let Some(ecran) = ecran.as_ref() {
        ecran.etape(&mut gop, 5, "DEMARRAGE DU NOYAU");
    }
    // Le protocole se rend AVANT `start_image` : le chargeur enfant en a
    // besoin, et un GOP encore ouvert en exclusif le lui refuserait.
    drop(gop);

    // Preserve a full device path for the child image. The bootloader then
    // keeps a valid DeviceHandle and can reopen the same FAT to load
    // kernel-x86_64, boot.json and /ramdisk.
    let loaded_path = boot::open_protocol_exclusive::<LoadedImageDevicePath>(
        boot::image_handle(),
    )?;
    let child = boot::load_image(
        boot::image_handle(),
        LoadImageSource::FromBuffer {
            buffer: &loader,
            file_path: Some(&*loaded_path),
        },
    )?;
    drop(loaded_path);
    boot::start_image(child)
}

#[entry]
fn main() -> Status {
    match run() { Ok(()) => Status::SUCCESS, Err(error) => error.status() }
}
