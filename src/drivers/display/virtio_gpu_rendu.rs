//! Textures 8x8 à commandes bornées. Les ressources sont référencées par
//! description de fichier ouverte, et jamais par une adresse DMA fournie.

use alloc::sync::{Arc, Weak};
use super::*;

pub const OCTETS_TEXTURE: usize = 256;
const COTE: u32 = 8;
const PAS: u32 = COTE * 4;
const PLAGE: cg::R = cg::R::neuf(0, 0, COTE as i32, COTE as i32);
const CAPACITE: usize = 8;
const PAR_OUVERTURE: usize = 2;

#[derive(Debug, PartialEq, Eq)]
pub enum ErreurRendu {
    Indisponible,
    Plein,
    Inconnue,
    Invalide,
    Commande,
}

struct Texture {
    proprietaire: Weak<()>,
    id: u32,
    physique: u64,
    virtuelle: usize,
    memoire: Memoire,
    valide: bool,
}

pub(super) struct TexturesRendu {
    textures: [Option<Texture>; CAPACITE],
    prochain: u32,
    peut_creer: bool,
}

impl TexturesRendu {
    pub(super) const fn neuf() -> Self {
        Self { textures: [const { None }; CAPACITE], prochain: 0x4000_0000, peut_creer: true }
    }

    fn indice(&self, session: &Arc<()>, id: u32) -> Result<usize, ErreurRendu> {
        let cle = Arc::downgrade(session);
        self.textures.iter().position(|t| t.as_ref().is_some_and(|t| {
            t.id == id && t.proprietaire.ptr_eq(&cle)
        })).ok_or(ErreurRendu::Inconnue)
    }

    unsafe fn retire(&mut self, f: &mut FileControle, indice: usize) -> Result<(), ErreurRendu> {
        let Some(mut t) = self.textures[indice].take() else { return Ok(()) };
        if !libere_texture(f, &mut t) {
            self.peut_creer = false;
            return Err(ErreurRendu::Commande);
        }
        Ok(())
    }
}

unsafe fn libere_texture(f: &mut FileControle, t: &mut Texture) -> bool {
    if !f.cassee {
        let _ = commande(f, &cg::contexte(false, t.id));
    }
    rend_si_sur(f, t.id, t.physique, PAGE, &mut t.memoire, "texture-ring3")
}

fn disponible(p: &Peripherique) -> bool {
    p.capset.is_some() && p.p10_virgl.is_some() && !p.file.cassee
}

unsafe fn commande(f: &mut FileControle, q: &[u8]) -> Result<(), ErreurRendu> {
    match f.commande(q, ENTETE) {
        Ok(RESP_OK_NODATA) => Ok(()),
        _ => Err(ErreurRendu::Commande),
    }
}

pub fn cree_texture_rendu(session: &Arc<()>) -> Result<u32, ErreurRendu> {
    let mut garde = verrou().ok_or(ErreurRendu::Indisponible)?;
    let p = garde.as_mut().filter(|p| disponible(p)).ok_or(ErreurRendu::Indisponible)?;
    let magasin = &mut p.textures_rendu;
    // La fermeture du dernier descripteur ne prend jamais le verrou GPU.
    // Les textures orphelines restent dans le quota et sont reprises ici.
    for i in 0..CAPACITE {
        if magasin.textures[i].as_ref().is_some_and(|t| t.proprietaire.strong_count() == 0) {
            unsafe { magasin.retire(&mut p.file, i)? };
        }
    }
    if !magasin.peut_creer { return Err(ErreurRendu::Commande); }

    let proprietaire = Arc::downgrade(session);
    let compte = magasin.textures.iter().flatten().filter(|t| t.proprietaire.ptr_eq(&proprietaire)).count();
    if compte >= PAR_OUVERTURE { return Err(ErreurRendu::Plein); }
    let indice = magasin.textures.iter().position(Option::is_none).ok_or(ErreurRendu::Plein)?;
    let id = magasin.prochain;
    magasin.prochain = id.checked_add(1).ok_or(ErreurRendu::Plein)?;
    let q = cg::cree_texture_liee(id, COTE, COTE, cg::LIAISON_CIBLE | (1 << 3))
        .ok_or(ErreurRendu::Invalide)?;
    let (physique, virtuelle) = memory::alloc_dma(PAGE).ok_or(ErreurRendu::Plein)?;
    let mut t = Texture { proprietaire, id, physique, virtuelle: virtuelle as usize,
        memoire: Memoire::NonAttachee, valide: false };
    let f = &mut p.file;
    unsafe {
        if let Err(e) = commande(f, &q) {
            rend_si_sur(f, 0, physique, PAGE, &mut t.memoire, "texture-ring3-non-creee");
            return Err(e);
        }
        let rendu = attache(f, id, physique, OCTETS_TEXTURE as u32);
        t.memoire = dma::apres_attache(t.memoire, dma::issue(rendu));
        let result = if rendu == Ok(RESP_OK_NODATA) {
            commande(f, &cg::contexte(true, id))
        } else {
            Err(ErreurRendu::Commande)
        };
        if let Err(e) = result {
            magasin.peut_creer = libere_texture(f, &mut t);
            return Err(e);
        }
    }
    magasin.textures[indice] = Some(t);
    Ok(id)
}

pub fn detruit_texture_rendu(session: &Arc<()>, id: u32) -> Result<(), ErreurRendu> {
    let mut garde = verrou().ok_or(ErreurRendu::Indisponible)?;
    let p = garde.as_mut().ok_or(ErreurRendu::Indisponible)?;
    let i = p.textures_rendu.indice(session, id)?;
    unsafe { p.textures_rendu.retire(&mut p.file, i) }
}

pub enum TransfertRendu<'a> {
    VersGpu(&'a [u8; OCTETS_TEXTURE]),
    DepuisGpu(&'a mut [u8; OCTETS_TEXTURE]),
}

pub fn transfert_texture_rendu(session: &Arc<()>, id: u32, sens: TransfertRendu<'_>)
    -> Result<(), ErreurRendu>
{
    let mut garde = verrou().ok_or(ErreurRendu::Indisponible)?;
    let p = garde.as_mut().filter(|p| disponible(p)).ok_or(ErreurRendu::Indisponible)?;
    let i = p.textures_rendu.indice(session, id)?;
    let t = p.textures_rendu.textures[i].as_mut().ok_or(ErreurRendu::Inconnue)?;
    unsafe {
        match sens {
            TransfertRendu::VersGpu(octets) => {
                let q = cg::transfert(id, PLAGE, 0, PAS, COTE, COTE).ok_or(ErreurRendu::Invalide)?;
                core::ptr::copy_nonoverlapping(octets.as_ptr(), t.virtuelle as *mut u8, OCTETS_TEXTURE);
                fence(Ordering::SeqCst);
                let r = suivie(&mut t.memoire, p.file.commande(&q, ENTETE));
                t.valide = r == Ok(RESP_OK_NODATA);
                if !t.valide { return Err(ErreurRendu::Commande); }
            }
            TransfertRendu::DepuisGpu(octets) => {
                if !t.valide { return Err(ErreurRendu::Invalide); }
                let q = cg::relecture(id, PLAGE, PAS, COTE, COTE).ok_or(ErreurRendu::Invalide)?;
                if suivie(&mut t.memoire, p.file.commande(&q, ENTETE)) != Ok(RESP_OK_NODATA) {
                    return Err(ErreurRendu::Commande);
                }
                fence(Ordering::SeqCst);
                for (i, octet) in octets.iter_mut().enumerate() {
                    *octet = read_volatile((t.virtuelle as *const u8).add(i));
                }
            }
        }
    }
    Ok(())
}

pub fn blit_texture_rendu(session: &Arc<()>, source: u32, cible: u32) -> Result<(), ErreurRendu> {
    if source == cible { return Err(ErreurRendu::Invalide); }
    let mut garde = verrou().ok_or(ErreurRendu::Indisponible)?;
    let p = garde.as_mut().filter(|p| disponible(p)).ok_or(ErreurRendu::Indisponible)?;
    let src = p.textures_rendu.indice(session, source)?;
    let dst = p.textures_rendu.indice(session, cible)?;
    let textures = &mut p.textures_rendu.textures;
    if !textures[src].as_ref().is_some_and(|t| t.valide) { return Err(ErreurRendu::Invalide); }
    let mut q = [0; 32 + cg::BLIT_OCTETS];
    q[..32].copy_from_slice(&cg::entete_soumission(cg::BLIT_OCTETS).ok_or(ErreurRendu::Invalide)?);
    cg::encode_blit(source, PLAGE, cible, PLAGE, &mut q[32..]).ok_or(ErreurRendu::Invalide)?;
    let rendu = unsafe { p.file.commande(&q, ENTETE) };
    for i in [src, dst] {
        if let Some(t) = &mut textures[i] {
            t.memoire = dma::apres_autre(t.memoire, dma::issue(rendu));
            if i == dst { t.valide = rendu == Ok(RESP_OK_NODATA); }
        }
    }
    if rendu == Ok(RESP_OK_NODATA) { Ok(()) } else { Err(ErreurRendu::Commande) }
}
