//! BOUCHAUD_BUREAU_RING3_2A -- une fenetre d'application confiee au
//! compositeur ring 3.
//!
//! L'application ne change pas : c'est un client du protocole BOGU (surface
//! memfd + canal), lance par `gui::client::Client`, exactement comme sous le
//! bureau noyau. Le noyau garde ce cote-la -- il pompe et VALIDE les messages
//! du client, tient le contrat de tampons, recolte un client mort -- et ne
//! donne au compositeur qu'une CAPACITE : l'objet natif `Fenetre`.
//!
//! Ce que le compositeur peut en faire, et rien d'autre :
//!   - `bo_fenetre_pompe` (READ) : faire valider ce que le client a envoye et
//!     apprendre son etat (vivant, taille peinte, dommage, trames) ;
//!   - `bo_fenetre_lit` (READ ; region WRITE) : faire recopier un rectangle
//!     de la face livree dans UNE DE SES regions partagees. Les bornes sont
//!     jugees par `contrat::valide`, pure et testee sur l'hote, AVANT toute
//!     copie : jamais au-dela de ce que le client a peint, jamais hors de la
//!     region ;
//!   - l'attendre dans un waitset : READABLE quand le client a ecrit,
//!     PEER_CLOSED quand il est mort.
//!
//! Le compositeur ne voit ni adresse, ni descripteur, ni le processus client.
//! La capacite ne se duplique pas et ne se transfere pas.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::gui::client::Client;
use crate::kernel::fd::Canal;
use crate::kernel::sync::SpinLock;

use super::abi::types::{Error, HandleId, ObjectKind, Result, Rights, Signals};
use super::handle;
use super::object::Object;

#[path = "fenetre_contrat.rs"]
pub mod contrat;

/// Droits de la capacite donnee au compositeur : pomper/lire, attendre,
/// inspecter. Ni DUP ni TRANSFER.
pub const DROITS_COMPOSITEUR: Rights = Rights(Rights::READ.0 | Rights::WAIT.0 | Rights::INSPECT.0);

/// Taille du rapport ecrit par `bo_fenetre_pompe` : dix `u32`.
pub const ETAT_OCTETS: usize = 40;

static LECTURES: AtomicU64 = AtomicU64::new(0);
static PIXELS_LUS: AtomicU64 = AtomicU64::new(0);

pub struct Fenetre {
    client: SpinLock<Client>,
    /// Lu sans prendre le client : `signals` est appele sous le verrou d'un
    /// waitset.
    entrant: Arc<SpinLock<Canal>>,
    pid: u32,
}

impl Fenetre {
    pub fn neuve(client: Client) -> Self {
        let entrant = client.canal_entrant_natif();
        let pid = client.pid;
        Self { client: SpinLock::new(client), entrant, pid }
    }

    pub fn pid(&self) -> u32 { self.pid }

    pub fn signals(&self) -> Signals {
        let mut s = Signals::NONE;
        if !self.entrant.lock().octets.is_empty() {
            s = s | Signals::READABLE;
        }
        if crate::kernel::task::code_de_sortie(self.pid).is_some() {
            s = s | Signals::PEER_CLOSED;
        }
        s
    }
}

fn fenetre_de(raw: u64, droit: Rights) -> Result<Arc<Object>> {
    let entree = handle::current_table().lookup_kind(HandleId::from_raw(raw), droit, ObjectKind::Fenetre)?;
    Ok(entree.object.clone())
}

/// `bo_fenetre_pompe(fenetre, etat*)` : valide les messages en attente du
/// client et ecrit son etat. Rend 1 si le client vit, 0 s'il est mort.
///
/// Etat (dix u32) : vivant, pid, largeur et hauteur PEINTES (`composable`),
/// dommage x/y/l/h (repris : un dommage n'est rendu qu'une fois), trames
/// livrees (32 bits bas), face lue.
pub fn pompe(args: [u64; 6]) -> Result<u64> {
    super::abi::usercopy::validate_write(args[1], ETAT_OCTETS)?;
    let objet = fenetre_de(args[0], Rights::READ)?;
    let Object::Fenetre(fenetre) = objet.as_ref() else { return Err(Error::WrongType) };
    let mut c = fenetre.client.lock();
    let vivant = c.vivant();
    if vivant {
        c.pompe();
    }
    let (l, h) = c.composable();
    let degat = if vivant { c.prend_degat() } else { crate::gui::protocole::Rect::default() };
    let mots: [u32; 10] = [
        vivant as u32, c.pid, l as u32, h as u32,
        degat.x.max(0) as u32, degat.y.max(0) as u32, degat.largeur, degat.hauteur,
        c.trames_livrees() as u32, c.face() as u32,
    ];
    drop(c);
    let mut fil = [0u8; ETAT_OCTETS];
    for (i, m) in mots.iter().enumerate() {
        fil[4 * i..4 * i + 4].copy_from_slice(&m.to_le_bytes());
    }
    super::abi::usercopy::write(args[1], &fil)?;
    Ok(vivant as u64)
}

/// `bo_fenetre_lit(fenetre, region, lecture*)` : recopie un rectangle de la
/// face livree dans la region du compositeur. Rend le nombre de pixels copies.
pub fn lit(args: [u64; 6]) -> Result<u64> {
    let objet = fenetre_de(args[0], Rights::READ)?;
    let Object::Fenetre(fenetre) = objet.as_ref() else { return Err(Error::WrongType) };
    let region = handle::current_table()
        .lookup_kind(HandleId::from_raw(args[1]), Rights::WRITE, ObjectKind::SharedRegion)?;
    let Object::SharedRegion(region) = region.object.as_ref() else { return Err(Error::WrongType) };
    let octets = super::abi::usercopy::read(args[2], contrat::LECTURE_OCTETS)?;
    let mut fil = [0u8; contrat::LECTURE_OCTETS];
    fil.copy_from_slice(&octets);
    let d = contrat::Lecture::depuis_octets(&fil);

    let c = fenetre.client.lock();
    let (l, h) = c.composable();
    let face = c.face();
    region.avec_octets_mut(|octets| -> Result<()> {
        contrat::valide(&d, (l as u32, h as u32), octets.len()).map_err(|_| Error::InvalidArgument)?;
        for r in 0..d.h {
            let debut = d.destination_ligne(r);
            let ligne = &mut octets[debut..debut + d.l as usize * 4];
            c.surface.copie_ligne_de_octets(face, (d.y + r) as usize, d.x as usize, d.l as usize, ligne);
        }
        Ok(())
    })?;
    drop(c);
    let pixels = d.l as u64 * d.h as u64;
    LECTURES.fetch_add(1, Ordering::Relaxed);
    PIXELS_LUS.fetch_add(pixels, Ordering::Relaxed);
    Ok(pixels)
}

/// Les comptes du pont, pour le journal du banc.
pub fn bilan() -> (u64, u64) {
    (LECTURES.load(Ordering::Relaxed), PIXELS_LUS.load(Ordering::Relaxed))
}
