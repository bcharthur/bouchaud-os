//! BOUCHAUD_COMPOSITORD_PRESENTATION_V1 -- tranche 1 du compositeur ring 3.
//!
//! `bo_display_present(affichage, region, demande*)` : un processus ring 3
//! qui detient la CAPACITE D'AFFICHAGE (objet `Display`, droit
//! `DISPLAY_PRESENT`) fait poser un rectangle d'une de ses regions partagees
//! a l'ecran. C'est l'interface dont un `compositord` ring 3 a besoin pour
//! remplacer la boucle de composition du noyau : il ne manipule que des
//! identifiants (capacite, region) et des rectangles, jamais une adresse
//! physique ni une commande du pilote.
//!
//! Securite :
//!   - aucun appel ne cree d'objet `Display` ; le noyau en donne UN, au seul
//!     programme `/bin/composited` qu'il lance lui-meme (`lance_compositeur`) ;
//!   - la region doit etre lue avec le droit READ du processus appelant ;
//!   - les bornes (surface dans la region, rectangle dans la surface,
//!     destination dans l'ecran) sont jugees par `contrat::valide`, pure et
//!     testee sur l'hote, AVANT toute lecture ;
//!   - tant que le bureau NOYAU tient l'ecran, l'appel est refuse
//!     (`WouldBlock`) : deux compositeurs n'ecrivent jamais le meme tampon.
//!
//! Retour : l'empreinte des pixels RELUS a l'ecran apres presentation ; le
//! client la compare a celle qu'il a calculee (preuve de bout en bout).

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use super::abi::types::{Error, HandleId, ObjectKind, Result, Rights};
use super::handle;
use super::object::Object;

#[path = "affichage_contrat.rs"]
pub mod contrat;

static BUREAU_NOYAU: AtomicBool = AtomicBool::new(false);
static ECRAN_AU_COMPOSITEUR: AtomicBool = AtomicBool::new(false);

/// Le compositeur ring 3 a pris l'ecran (premiere presentation acceptee).
pub fn ecran_au_compositeur() -> bool {
    ECRAN_AU_COMPOSITEUR.load(Ordering::Acquire)
}
static PRESENTATIONS: AtomicU64 = AtomicU64::new(0);
static REFUS: AtomicU64 = AtomicU64::new(0);

/// Le fil du bureau noyau tient l'ecran a partir de maintenant.
pub fn bureau_noyau_demarre() {
    BUREAU_NOYAU.store(true, Ordering::Release);
}

/// Seul chemin qui fait naitre une capacite d'affichage.
pub const COMPOSITEUR: &str = "/bin/composited";

/// Droits de la capacite donnee au compositeur : presenter, inspecter. Ni
/// DUP ni TRANSFER : elle ne se multiplie pas et ne se passe pas.
pub const DROITS_COMPOSITEUR: Rights = Rights(Rights::DISPLAY_PRESENT.0 | Rights::INSPECT.0);

/// Lance `/bin/composited` avec la capacite d'affichage, s'il est present.
/// Absent (cas de toutes les images livrees aujourd'hui) : rien ne se passe.
pub fn lance_compositeur() -> Option<u32> {
    use alloc::string::{String, ToString};
    crate::fs::ramfs::fs().resolve(COMPOSITEUR, 0)?;
    let argv = [COMPOSITEUR.to_string(), "presentation".to_string()];
    let env = [String::from("PATH=/bin:/usr/bin")];
    let mut prepare = |process: &crate::kernel::task::Process| -> alloc::vec::Vec<String> {
        match handle::install_for(process.pid, Object::Display, DROITS_COMPOSITEUR) {
            Ok(h) => alloc::vec![alloc::format!("BO_DISPLAY={}", h.raw())],
            Err(_) => alloc::vec![],
        }
    };
    match crate::kernel::exec::lance_detache(COMPOSITEUR, &argv, &env, 0, &mut prepare) {
        Ok(pid) => {
            crate::serial_println!(
                "BOUCHAUD_COMPOSITEUR_LANCE pid={} capacite=affichage droits={:#x}",
                pid, DROITS_COMPOSITEUR.0
            );
            Some(pid)
        }
        Err(e) => {
            crate::serial_println!("BOUCHAUD_COMPOSITEUR_ECHEC raison={}", e);
            None
        }
    }
}

fn refuse(e: Error, raison: &str) -> Error {
    REFUS.fetch_add(1, Ordering::Relaxed);
    crate::serial_println!("BOUCHAUD_DISPLAY_PRESENT_REFUS raison={}", raison);
    e
}

pub fn present(args: [u64; 6]) -> Result<u64> {
    let table = handle::current_table();
    // 1. La capacite : bon type ET bon droit, sinon rien n'est lu.
    table.lookup_kind(HandleId::from_raw(args[0]), Rights::DISPLAY_PRESENT, ObjectKind::Display)
        .map_err(|e| refuse(e, "capacite"))?;
    // 2. La region, lue avec les droits de l'appelant.
    let region = table.lookup_kind(HandleId::from_raw(args[1]), Rights::READ, ObjectKind::SharedRegion)
        .map_err(|e| refuse(e, "region"))?;
    let region = match region.object.as_ref() {
        Object::SharedRegion(r) => r,
        _ => return Err(Error::WrongType),
    };
    // 3. La demande.
    let octets = super::abi::usercopy::read(args[2], contrat::DEMANDE_OCTETS)?;
    let mut fil = [0u8; contrat::DEMANDE_OCTETS];
    fil.copy_from_slice(&octets);
    let d = contrat::Demande::depuis_octets(&fil);
    // 4. L'ecran : refuse tant que le bureau noyau le tient.
    if BUREAU_NOYAU.load(Ordering::Acquire) {
        return Err(refuse(Error::WouldBlock, "bureau-noyau"));
    }
    crate::drivers::gfx::entre_si_inactif();
    if !crate::drivers::gfx::is_active() {
        return Err(refuse(Error::NotFound, "pas-d-ecran"));
    }
    ECRAN_AU_COMPOSITEUR.store(true, Ordering::Release);
    let (el, eh) = (crate::drivers::gfx::width() as u32, crate::drivers::gfx::height() as u32);
    contrat::valide(&d, region.len(), el, eh).map_err(|r| {
        let raison = match r {
            contrat::Refus::Vide => "vide",
            contrat::Refus::TropGrand => "trop-grand",
            contrat::Refus::SurfaceHorsRegion => "surface-hors-region",
            contrat::Refus::HorsSurface => "hors-surface",
            contrat::Refus::HorsEcran => "hors-ecran",
        };
        refuse(Error::InvalidArgument, raison)
    })?;
    // 5. Pose, ligne a ligne, puis presentation du seul rectangle.
    for r in 0..d.h {
        let ligne = region.read(d.decalage_ligne(r), d.l as usize * 4)?;
        let dst = crate::drivers::gfx::ligne_mut(d.dest_x as usize, (d.dest_y + r) as usize, d.l as usize);
        for (i, px) in dst.iter_mut().enumerate() {
            let o = 4 * i;
            *px = u32::from_le_bytes([ligne[o], ligne[o + 1], ligne[o + 2], ligne[o + 3]]) & 0x00ff_ffff;
        }
    }
    crate::drivers::gfx::present_rect(d.dest_x as usize, d.dest_y as usize, d.l as usize, d.h as usize);
    // 6. Relecture de ce qui est a l'ecran : l'empreinte rendue au client.
    let e = contrat::empreinte((0..d.h).flat_map(|r| {
        (0..d.l).map(move |c| crate::drivers::gfx::get_pixel_rgb((d.dest_x + c) as usize, (d.dest_y + r) as usize))
    }));
    let n = PRESENTATIONS.fetch_add(1, Ordering::Relaxed) + 1;
    crate::serial_println!(
        "BOUCHAUD_DISPLAY_PRESENT n={} rect={}x{}@{},{} ecran={}x{} empreinte={:#x} refus={}",
        n, d.l, d.h, d.dest_x, d.dest_y, el, eh, e, REFUS.load(Ordering::Relaxed)
    );
    Ok(e)
}
