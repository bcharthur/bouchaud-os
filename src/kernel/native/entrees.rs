//! BOUCHAUD_ENTREES_RING3_V1 -- tranche 3 du bureau hors du noyau : les
//! ENTREES (souris, clavier) confiees au compositeur ring 3.
//!
//! Jusqu'ici `composited` composait des fenetres, mais personne en ring 3 ne
//! recevait la souris ni le clavier : ses deplacements etaient scriptes. Le
//! noyau garde le PILOTE (IRQ, decodage, disposition) ; il ne donne au
//! compositeur qu'une LECTURE, sur la capacite d'affichage qu'il est seul a
//! detenir :
//!
//!   `bo_display_entrees(affichage, tampon*, touches_max)` -> touches lues
//!
//!   en-tete (24 octets) : x, y, boutons (u32), molette (i32, crans depuis la
//!   lecture precedente), touches (u32), changements souris (u32, compteur) ;
//!   puis `touches` transitions de 16 octets : code, unicode, modificateurs,
//!   appui -- deja traduites comme le bureau noyau les traduit pour un client
//!   (`gui::window_manager::traduit_touche`).
//!
//! Le compositeur route ensuite chaque evenement vers UNE fenetre par
//! `bo_fenetre_entree` (fenetre.rs) : le noyau ecrit le message du protocole
//! GUI, le client ne voit jamais le compositeur.
//!
//! Proprietaire unique : a la premiere lecture, le clavier est CONFIE au
//! compositeur (`keyboard::confie_au_compositeur`). Le shell noyau, qui lisait
//! la meme file, ne vole plus les frappes -- il attend.
//!
//! Reveil : l'IRQ souris/clavier publie (`activite::publie_differe`, atomique,
//! sans ordonnanceur) ; la moitie basse du minuteur reveille les attentes
//! natives. La capacite d'affichage est LISIBLE (READABLE) tant qu'une entree
//! attend : `composited` la met dans son waitset et dort entre deux gestes.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use super::abi::types::{Error, HandleId, ObjectKind, Result, Rights, Signals};
use super::handle;

/// Le compositeur lit les entrees : il en est le proprietaire.
static ACTIF: AtomicBool = AtomicBool::new(false);
/// Compteur de changements souris deja rendus au compositeur.
static VU_SOURIS: AtomicU64 = AtomicU64::new(0);
static LECTURES: AtomicU64 = AtomicU64::new(0);
static TOUCHES: AtomicU64 = AtomicU64::new(0);

pub const ENTETE_OCTETS: usize = 24;
pub const TOUCHE_OCTETS: usize = 16;
pub const TOUCHES_MAX: usize = 64;

/// Le compositeur ring 3 est-il proprietaire des entrees ?
pub fn actif() -> bool {
    ACTIF.load(Ordering::Acquire)
}

/// Appele par le chemin de reveil des IRQ d'entree (souris, clavier), en
/// contexte d'interruption : atomiques seulement.
#[inline]
pub fn irq() {
    if ACTIF.load(Ordering::Relaxed) {
        super::activite::publie_differe();
    }
}

/// Une entree attend-elle le compositeur ?
pub fn en_attente() -> bool {
    if !ACTIF.load(Ordering::Acquire) {
        return false;
    }
    crate::drivers::mouse::changements() != VU_SOURIS.load(Ordering::Acquire)
        || crate::drivers::mouse::wheel_pending()
        || crate::drivers::keyboard::has_pending()
}

/// Signaux de la capacite d'affichage (`Object::Display`).
pub fn signaux() -> Signals {
    if en_attente() { Signals::READABLE } else { Signals::NONE }
}

/// `bo_display_entrees(affichage, tampon*, touches_max)`.
pub fn lit(args: [u64; 6]) -> Result<u64> {
    handle::current_table()
        .lookup_kind(HandleId::from_raw(args[0]), Rights::DISPLAY_PRESENT, ObjectKind::Display)?;
    // Le bureau noyau tient l'ecran : c'est lui qui consomme les entrees.
    if super::affichage::bureau_noyau_actif() {
        return Err(Error::WouldBlock);
    }
    let max = (args[2] as usize).min(TOUCHES_MAX);
    let taille = ENTETE_OCTETS + max * TOUCHE_OCTETS;
    super::abi::usercopy::validate_write(args[1], taille)?;

    if !ACTIF.swap(true, Ordering::AcqRel) {
        // La souris PS/2 n'etait armee que par la boucle du bureau NOYAU, qui
        // ne tourne pas ici : meme decision que lui (`legacy_ps2_souris` :
        // pas de double comptage quand une souris USB repond).
        let ps2 = crate::platform::pc::stage2::legacy_ps2_souris();
        if ps2 {
            crate::drivers::mouse::init();
        }
        // APRES l'armement : ses accuses au 8042 sont purges avec le reste.
        let purges = crate::drivers::keyboard::confie_au_compositeur();
        crate::serial_println!(
            "BOUCHAUD_ENTREES_RING3_ACTIVES proprietaire=compositeur clavier=confie souris_ps2={} purges={}",
            ps2 as u8, purges
        );
    }

    let mut fil = [0u8; ENTETE_OCTETS + TOUCHES_MAX * TOUCHE_OCTETS];
    // Le compteur AVANT l'etat : un paquet arrive entre les deux sera rendu
    // a la lecture suivante (le compteur aura bouge), jamais perdu.
    let changements = crate::drivers::mouse::changements();
    VU_SOURIS.store(changements, Ordering::Release);
    let (x, y) = crate::drivers::mouse::pos();
    let boutons = crate::drivers::mouse::buttons() as u32;
    let molette = crate::drivers::mouse::take_wheel();
    let mut n = 0usize;
    while n < max {
        let Some(evenement) = crate::drivers::keyboard::try_key_event_compositeur() else { break };
        let (code, unicode, modificateurs) = crate::gui::window_manager::traduit_touche(&evenement);
        let o = ENTETE_OCTETS + n * TOUCHE_OCTETS;
        fil[o..o + 4].copy_from_slice(&code.to_le_bytes());
        fil[o + 4..o + 8].copy_from_slice(&unicode.to_le_bytes());
        fil[o + 8..o + 12].copy_from_slice(&modificateurs.to_le_bytes());
        fil[o + 12..o + 16].copy_from_slice(&(evenement.appui as u32).to_le_bytes());
        n += 1;
    }
    let entete: [u32; 6] = [x as u32, y as u32, boutons, molette as u32, n as u32, changements as u32];
    for (i, m) in entete.iter().enumerate() {
        fil[4 * i..4 * i + 4].copy_from_slice(&m.to_le_bytes());
    }
    super::abi::usercopy::write(args[1], &fil[..ENTETE_OCTETS + n * TOUCHE_OCTETS])?;
    LECTURES.fetch_add(1, Ordering::Relaxed);
    TOUCHES.fetch_add(n as u64, Ordering::Relaxed);
    Ok(n as u64)
}

/// Les comptes, pour le journal du banc.
pub fn bilan() -> (u64, u64) {
    (LECTURES.load(Ordering::Relaxed), TOUCHES.load(Ordering::Relaxed))
}
