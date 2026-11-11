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
/// BOUCHAUD_BUREAU_RING3_2A : le bureau de cette machine est le compositeur
/// ring 3 ; le bureau noyau ne doit pas demarrer.
static BUREAU_RING3: AtomicBool = AtomicBool::new(false);

/// L'ecran appartient-il au compositeur ring 3 (bureau configure, ou ecran
/// deja pris) ? Le bureau noyau refuse alors de demarrer.
pub fn ecran_reserve_au_compositeur() -> bool {
    BUREAU_RING3.load(Ordering::Acquire) || ECRAN_AU_COMPOSITEUR.load(Ordering::Acquire)
}
static ECRAN_AU_COMPOSITEUR: AtomicBool = AtomicBool::new(false);

/// Le compositeur ring 3 a pris l'ecran (premiere presentation acceptee).
pub fn ecran_au_compositeur() -> bool {
    ECRAN_AU_COMPOSITEUR.load(Ordering::Acquire)
}
static PRESENTATIONS: AtomicU64 = AtomicU64::new(0);
static REFUS: AtomicU64 = AtomicU64::new(0);

/// Le fil du bureau noyau demande l'ecran. Rend `false` -- et le bureau noyau
/// ne demarre pas -- si le compositeur ring 3 l'a deja, ou s'il est le bureau
/// configure de cette machine (BOUCHAUD_BUREAU_RING3_2A).
///
/// Exclusion sans verrou, ordre sequentiel : chaque cote PUBLIE sa prise
/// puis LIT celle de l'autre (`present` fait de meme). Au moins l'un des deux
/// voit l'autre ; deux compositeurs n'ecrivent jamais le meme ecran.
pub fn bureau_noyau_demarre() -> bool {
    BUREAU_NOYAU.store(true, Ordering::SeqCst);
    if BUREAU_RING3.load(Ordering::SeqCst) || ECRAN_AU_COMPOSITEUR.load(Ordering::SeqCst) {
        BUREAU_NOYAU.store(false, Ordering::SeqCst);
        REFUS.fetch_add(1, Ordering::Relaxed);
        crate::serial_println!("BOUCHAUD_BUREAU_NOYAU_REFUSE raison=ecran-au-compositeur");
        return false;
    }
    true
}

/// Seul chemin qui fait naitre une capacite d'affichage.
pub const COMPOSITEUR: &str = "/bin/composited";

/// Droits de la capacite donnee au compositeur : presenter, inspecter. Ni
/// DUP ni TRANSFER : elle ne se multiplie pas et ne se passe pas.
pub const DROITS_COMPOSITEUR: Rights = Rights(Rights::DISPLAY_PRESENT.0 | Rights::INSPECT.0);

/// BOUCHAUD_BUREAU_RING3_2A : configuration du bureau ring 3, posee dans
/// l'image. Premiere ligne : `CHEMIN [VAR=VALEUR ...]` -- le client confie au
/// compositeur et les variables qu'il recoit en plus de celles du shell.
pub const CONFIG_BUREAU: &str = "/bureau-ring3";

/// Zone de la fenetre de l'application (allocation = zone en 2A).
pub const ZONE_FENETRE: (usize, usize) = (640, 400);

/// Lit la configuration du bureau ring 3 : (client, variables).
fn config_bureau() -> Option<(alloc::string::String, alloc::vec::Vec<alloc::string::String>)> {
    use alloc::string::ToString;
    let texte = {
        let fs = crate::fs::ramfs::fs();
        let idx = fs.resolve(CONFIG_BUREAU, 0)?;
        if fs.nodes[idx].kind != crate::fs::ramfs::NodeKind::File {
            return None;
        }
        fs.nodes[idx].content_str()
    };
    let ligne = texte.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#'))?;
    let mut mots = ligne.split_whitespace();
    let chemin = mots.next()?.to_string();
    // Seules des affectations de variables `BO_*` passent : la configuration
    // ne sert pas a injecter un environnement arbitraire.
    let vars = mots
        .filter(|m| m.starts_with("BO_") && m.contains('='))
        .map(|m| m.to_string())
        .collect();
    Some((chemin, vars))
}

/// Lance `/bin/composited` avec la capacite d'affichage, s'il est present.
/// Absent (cas de toutes les images livrees aujourd'hui) : rien ne se passe.
///
/// BOUCHAUD_BUREAU_RING3_2A : si l'image porte `CONFIG_BUREAU`, le noyau
/// lance d'abord le client qui y est nomme -- un client BOGU ordinaire, avec
/// sa surface et son canal, comme sous le bureau noyau -- et donne au
/// compositeur, en plus de l'affichage, la capacite `Fenetre` sur ce client
/// (`BO_FENETRE=<handle>`, mode `bureau`). Le bureau noyau, lui, ne demarre
/// plus : l'ecran a un seul compositeur.
pub fn lance_compositeur() -> Option<u32> {
    use alloc::string::{String, ToString};
    crate::fs::ramfs::fs().resolve(COMPOSITEUR, 0)?;
    let fenetre = match config_bureau() {
        Some((chemin, vars)) => {
            match crate::gui::client::Client::lance_avec(
                &chemin, "application", 0, ZONE_FENETRE, ZONE_FENETRE, &vars,
            ) {
                Ok(client) => {
                    crate::serial_println!(
                        "BOUCHAUD_BUREAU_RING3_CLIENT pid={} chemin={} zone={}x{} variables={}",
                        client.pid, chemin, ZONE_FENETRE.0, ZONE_FENETRE.1, vars.len()
                    );
                    Some(super::fenetre::Fenetre::neuve(client))
                }
                Err(e) => {
                    crate::serial_println!("BOUCHAUD_BUREAU_RING3_ECHEC raison=client chemin={} detail={}", chemin, e);
                    None
                }
            }
        }
        None => None,
    };
    let mode = if fenetre.is_some() { "bureau" } else { "presentation" };
    if fenetre.is_some() {
        BUREAU_RING3.store(true, Ordering::Release);
    }
    let argv = [COMPOSITEUR.to_string(), mode.to_string()];
    let env = [String::from("PATH=/bin:/usr/bin")];
    let mut fenetre = fenetre;
    let mut prepare = |process: &crate::kernel::task::Process| -> alloc::vec::Vec<String> {
        let mut vars = alloc::vec::Vec::new();
        if let Ok(h) = handle::install_for(process.pid, Object::Display, DROITS_COMPOSITEUR) {
            vars.push(alloc::format!("BO_DISPLAY={}", h.raw()));
        }
        if let Some(f) = fenetre.take() {
            if let Ok(h) = handle::install_for(process.pid, Object::Fenetre(f), super::fenetre::DROITS_COMPOSITEUR) {
                vars.push(alloc::format!("BO_FENETRE={}", h.raw()));
            }
        }
        vars
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

/// `bo_display_info(affichage)` : (largeur << 32) | hauteur de l'ecran.
pub fn info(args: [u64; 6]) -> Result<u64> {
    handle::current_table()
        .lookup_kind(HandleId::from_raw(args[0]), Rights::DISPLAY_PRESENT, ObjectKind::Display)
        .map_err(|e| refuse(e, "capacite"))?;
    if BUREAU_NOYAU.load(Ordering::Acquire) {
        return Err(refuse(Error::WouldBlock, "bureau-noyau"));
    }
    let deja = crate::drivers::gfx::is_active();
    crate::drivers::gfx::entre_si_inactif();
    if !crate::drivers::gfx::is_active() {
        return Err(Error::NotFound);
    }
    let (l, h) = (crate::drivers::gfx::width() as u64, crate::drivers::gfx::height() as u64);
    if !deja && !ECRAN_AU_COMPOSITEUR.load(Ordering::Acquire) {
        crate::drivers::gfx::leave();
    }
    Ok((l << 32) | h)
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
    let deja_graphique = crate::drivers::gfx::is_active();
    crate::drivers::gfx::entre_si_inactif();
    if !crate::drivers::gfx::is_active() {
        return Err(refuse(Error::NotFound, "pas-d-ecran"));
    }
    // Revue de la PR #286 : l'ecran n'est au compositeur qu'une fois la
    // demande ACCEPTEE ; un refus rend le mode texte si cet appel l'avait quitte.
    let rend = |e| {
        if !deja_graphique && !ECRAN_AU_COMPOSITEUR.load(Ordering::Acquire) {
            crate::drivers::gfx::leave();
        }
        e
    };
    let (el, eh) = (crate::drivers::gfx::width() as u32, crate::drivers::gfx::height() as u32);
    contrat::valide(&d, region.len(), el, eh).map_err(|r| {
        let raison = match r {
            contrat::Refus::Vide => "vide",
            contrat::Refus::TropGrand => "trop-grand",
            contrat::Refus::SurfaceHorsRegion => "surface-hors-region",
            contrat::Refus::HorsSurface => "hors-surface",
            contrat::Refus::HorsEcran => "hors-ecran",
        };
        rend(refuse(Error::InvalidArgument, raison))
    })?;
    // Prise de l'ecran, puis relecture de celle du bureau noyau (voir
    // `bureau_noyau_demarre`) : si les deux se croisent, l'un renonce.
    ECRAN_AU_COMPOSITEUR.store(true, Ordering::SeqCst);
    if BUREAU_NOYAU.load(Ordering::SeqCst) {
        return Err(refuse(Error::WouldBlock, "bureau-noyau"));
    }
    // 5. Pose, ligne a ligne, puis presentation du seul rectangle.
    for r in 0..d.h {
        let ligne = region.read(d.decalage_ligne(r), d.l as usize * 4)?;
        let dst = crate::drivers::gfx::ligne_mut(d.dest_x as usize, (d.dest_y + r) as usize, d.l as usize);
        for (i, px) in dst.iter_mut().enumerate() {
            let o = 4 * i;
            *px = u32::from_le_bytes([ligne[o], ligne[o + 1], ligne[o + 2], ligne[o + 3]]) & 0x00ff_ffff;
        }
    }
    // Presentation JUGEE : un refus du peripherique (transfert ou flush virtio,
    // LFB absent...) est rendu au client, jamais une empreinte du tampon arriere.
    if let Err(raison) = crate::drivers::gfx::present_rect_verifie(
        d.dest_x as usize, d.dest_y as usize, d.l as usize, d.h as usize) {
        return Err(refuse(Error::DeviceError, raison));
    }
    // 6. Relecture du tampon AFFICHE (pas du tampon arriere) : l'empreinte
    // rendue au client est celle de ce que l'ecran montre.
    let mut illisible = false;
    let e = contrat::empreinte((0..d.h).flat_map(|r| (0..d.l).map(move |c| (c, r))).map(|(c, r)| {
        crate::drivers::gfx::pixel_affiche((d.dest_x + c) as usize, (d.dest_y + r) as usize).unwrap_or_else(|| {
            illisible = true;
            0
        })
    }));
    if illisible {
        return Err(refuse(Error::DeviceError, "relecture"));
    }
    let n = PRESENTATIONS.fetch_add(1, Ordering::Relaxed) + 1;
    crate::serial_println!(
        "BOUCHAUD_DISPLAY_PRESENT n={} rect={}x{}@{},{} ecran={}x{} empreinte={:#x} refus={}",
        n, d.l, d.h, d.dest_x, d.dest_y, el, eh, e, REFUS.load(Ordering::Relaxed)
    );
    Ok(e)
}
