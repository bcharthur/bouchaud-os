//! Textures applicatives à mémoire DMA segmentée, possédées par ouverture.
use alloc::{sync::{Arc, Weak}, vec::Vec};
use super::*;

pub const OCTETS_TEXTURE: usize = 256;
const LIMITE_COTE: u32 = 1024;
const SEGMENT: usize = 64 * 1024;
const MAX_SEGMENTS: usize = 64;
const QUOTA_SESSION: usize = 8 * 1024 * 1024;
const QUOTA_TOTAL: usize = 16 * 1024 * 1024;
const CAPACITE: usize = 8;
const PAR_OUVERTURE: usize = 2;

#[derive(Debug, PartialEq, Eq)]
pub enum ErreurRendu {
    Indisponible,
    Plein,
    #[cfg(feature = "banc-gpu-venus")]
    Occupe,
    #[cfg(feature = "banc-gpu-venus")]
    Existe,
    Inconnue,
    Invalide,
    Commande,
}

#[derive(Clone, Copy)]
pub struct RegionRendu {
    pub x: u32,
    pub y: u32,
    pub largeur: u32,
    pub hauteur: u32,
}

impl RegionRendu {
    fn rectangle(self, largeur: u32, hauteur: u32) -> Option<cg::R> {
        if self.largeur == 0 || self.hauteur == 0
            || self.x.checked_add(self.largeur)? > largeur
            || self.y.checked_add(self.hauteur)? > hauteur
        {
            return None;
        }
        Some(cg::R::neuf(self.x as i32, self.y as i32, self.largeur as i32, self.hauteur as i32))
    }

    pub fn longueur(self, pas: usize) -> Option<usize> {
        self.rectangle(LIMITE_COTE, LIMITE_COTE)?;
        let ligne = self.largeur as usize * 4;
        if pas < ligne || pas > LIMITE_COTE as usize * 4 || pas % 4 != 0 {
            return None;
        }
        Some((self.hauteur as usize - 1) * pas + ligne)
    }
}

struct Bloc {
    physique: u64,
    virtuelle: usize,
    octets: usize,
}

struct Texture {
    proprietaire: Weak<()>,
    id: u32,
    largeur: u32,
    hauteur: u32,
    blocs: Vec<Bloc>,
    memoire: Memoire,
    valide: bool,
}

impl Texture {
    fn taille(&self) -> usize {
        (self.largeur as usize * self.hauteur as usize * 4).next_multiple_of(PAGE)
    }

    fn pleine(&self) -> cg::R {
        cg::R::neuf(0, 0, self.largeur as i32, self.hauteur as i32)
    }

    fn region(&self, zone: Option<RegionRendu>) -> Result<cg::R, ErreurRendu> {
        match zone {
            Some(zone) => zone.rectangle(self.largeur, self.hauteur).ok_or(ErreurRendu::Invalide),
            None if self.largeur == 8 && self.hauteur == 8 => Ok(self.pleine()),
            None => Err(ErreurRendu::Invalide),
        }
    }

    fn pas(&self) -> u32 {
        self.largeur * 4
    }

    unsafe fn copie_dma(&self, mut position: usize, mut octets: &mut [u8], vers_gpu: bool) {
        while !octets.is_empty() {
            let bloc = &self.blocs[position / SEGMENT];
            let decalage = position % SEGMENT;
            let longueur = octets.len().min(bloc.octets - decalage);
            let adresse = (bloc.virtuelle as *mut u8).add(decalage);
            if vers_gpu {
                core::ptr::copy_nonoverlapping(octets.as_ptr(), adresse, longueur);
            } else {
                core::ptr::copy_nonoverlapping(adresse, octets.as_mut_ptr(), longueur);
            }
            octets = &mut octets[longueur..];
            position += longueur;
        }
    }
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

    fn indice(&self, cle: &Weak<()>, id: u32) -> Result<usize, ErreurRendu> {
        self.textures.iter().position(|t| t.as_ref().is_some_and(|t| {
            t.id == id && t.proprietaire.ptr_eq(cle)
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

unsafe fn rend_blocs(f: &mut FileControle, t: &mut Texture) {
    for bloc in t.blocs.drain(..) {
        rend_si_sur(f, 0, bloc.physique, bloc.octets, &mut t.memoire, "texture-ring3");
    }
}

unsafe fn libere_texture(f: &mut FileControle, t: &mut Texture) -> bool {
    if !f.cassee {
        let _ = commande(f, &cg::contexte(false, t.id));
    }
    let liberee = detache_et_libere(f, t.id, &mut t.memoire);
    rend_blocs(f, t);
    liberee && dma::peut_rendre(t.memoire)
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

pub fn cree_texture_rendu(session: &Arc<()>, dimensions: Option<(u32, u32)>) -> Result<u32, ErreurRendu> {
    let (largeur, hauteur) = dimensions.unwrap_or((8, 8));
    if !(1..=LIMITE_COTE).contains(&largeur) || !(1..=LIMITE_COTE).contains(&hauteur) {
        return Err(ErreurRendu::Invalide);
    }
    let mut garde = verrou().ok_or(ErreurRendu::Indisponible)?;
    let p = garde.as_mut().filter(|p| disponible(p)).ok_or(ErreurRendu::Indisponible)?;
    let magasin = &mut p.textures_rendu;
    if !magasin.peut_creer { return Err(ErreurRendu::Commande); }
    // La fermeture du descripteur ne prend jamais le verrou GPU.
    for i in 0..CAPACITE {
        if magasin.textures[i].as_ref().is_some_and(|t| t.proprietaire.strong_count() == 0) {
            unsafe { magasin.retire(&mut p.file, i)? };
        }
    }

    let id = magasin.prochain;
    let mut t = Texture {
        proprietaire: Arc::downgrade(session), id, largeur, hauteur,
        blocs: Vec::new(), memoire: Memoire::NonAttachee, valide: false,
    };
    let taille = t.taille();
    let mut compte = 0;
    let mut session_octets = 0;
    let mut total = 0;
    for autre in magasin.textures.iter().flatten() {
        let octets = autre.taille();
        total += octets;
        if autre.proprietaire.ptr_eq(&t.proprietaire) {
            compte += 1;
            session_octets += octets;
        }
    }
    if compte >= PAR_OUVERTURE || session_octets + taille > QUOTA_SESSION || total + taille > QUOTA_TOTAL {
        return Err(ErreurRendu::Plein);
    }
    let indice = magasin.textures.iter().position(Option::is_none).ok_or(ErreurRendu::Plein)?;
    magasin.prochain = id.checked_add(1).ok_or(ErreurRendu::Plein)?;
    let creation = cg::cree_texture_liee(id, largeur, hauteur, cg::LIAISON_CIBLE | (1 << 3))
        .ok_or(ErreurRendu::Invalide)?;
    let f = &mut p.file;
    unsafe {
        prepare_blocs(f, &mut t, taille)?;
        if let Err(e) = commande(f, &creation) {
            rend_blocs(f, &mut t);
            return Err(e);
        }
        let result = attache_texture(f, &mut t).and_then(|()| {
            if dimensions.is_none() { return Ok(()); }
            // Le nouveau contrat garantit l'initialisation entière, y compris
            // quand la première opération applicative ne touche qu'un rectangle.
            let req = cg::transfert(t.id, t.pleine(), 0, t.pas(), t.largeur, t.hauteur)
                .ok_or(ErreurRendu::Invalide)?;
            t.valide = suivie(&mut t.memoire, f.commande(&req, ENTETE)) == Ok(RESP_OK_NODATA);
            if t.valide { Ok(()) } else { Err(ErreurRendu::Commande) }
        });
        if let Err(e) = result {
            magasin.peut_creer = libere_texture(f, &mut t);
            return Err(e);
        }
    }
    magasin.textures[indice] = Some(t);
    Ok(id)
}

unsafe fn prepare_blocs(f: &mut FileControle, t: &mut Texture, taille: usize) -> Result<(), ErreurRendu> {
    t.blocs.try_reserve_exact(taille.div_ceil(SEGMENT)).map_err(|_| ErreurRendu::Plein)?;
    for position in (0..taille).step_by(SEGMENT) {
        let octets = (taille - position).min(SEGMENT);
        let Some((physique, virtuelle)) = memory::alloc_dma(octets) else {
            rend_blocs(f, t);
            return Err(ErreurRendu::Plein);
        };
        t.blocs.push(Bloc { physique, virtuelle: virtuelle as usize, octets });
    }
    Ok(())
}

unsafe fn attache_texture(f: &mut FileControle, t: &mut Texture) -> Result<(), ErreurRendu> {
    let mut req = [0; 32 + 16 * MAX_SEGMENTS];
    req[..32].copy_from_slice(&cg::entete_attache(t.id, t.blocs.len() as u32));
    for (bloc, slot) in t.blocs.iter().zip(req[32..].chunks_exact_mut(16)) {
        slot[..8].copy_from_slice(&bloc.physique.to_le_bytes());
        slot[8..12].copy_from_slice(&(bloc.octets as u32).to_le_bytes());
    }
    let attache = f.commande(&req[..32 + 16 * t.blocs.len()], ENTETE);
    t.memoire = dma::apres_attache(t.memoire, dma::issue(attache));
    if attache != Ok(RESP_OK_NODATA) { return Err(ErreurRendu::Commande); }
    commande(f, &cg::contexte(true, t.id))
}

pub fn detruit_texture_rendu(session: &Arc<()>, id: u32) -> Result<(), ErreurRendu> {
    let mut garde = verrou().ok_or(ErreurRendu::Indisponible)?;
    let p = garde.as_mut().ok_or(ErreurRendu::Indisponible)?;
    let i = p.textures_rendu.indice(&Arc::downgrade(session), id)?;
    unsafe { p.textures_rendu.retire(&mut p.file, i) }
}

pub enum TransfertRendu {
    VersGpu,
    DepuisGpu,
}

pub fn transfert_texture_rendu(session: &Arc<()>, id: u32, zone: Option<RegionRendu>,
    pas: usize, octets: &mut [u8], sens: TransfertRendu) -> Result<(), ErreurRendu>
{
    let mut garde = verrou().ok_or(ErreurRendu::Indisponible)?;
    let p = garde.as_mut().filter(|p| disponible(p)).ok_or(ErreurRendu::Indisponible)?;
    let i = p.textures_rendu.indice(&Arc::downgrade(session), id)?;
    let t = p.textures_rendu.textures[i].as_mut().ok_or(ErreurRendu::Inconnue)?;
    let r = t.region(zone)?;
    let longueur = r.l as usize * 4;
    let attendu = (r.h as usize - 1).checked_mul(pas).and_then(|n| n.checked_add(longueur));
    if pas < longueur || attendu != Some(octets.len()) { return Err(ErreurRendu::Invalide); }
    let vers_gpu = matches!(sens, TransfertRendu::VersGpu);
    let req = if vers_gpu {
        cg::transfert(id, r, 0, t.pas(), t.largeur, t.hauteur)
    } else {
        cg::relecture(id, r, t.pas(), t.largeur, t.hauteur)
    }.ok_or(ErreurRendu::Invalide)?;
    if !(t.valide || vers_gpu && r == t.pleine()) { return Err(ErreurRendu::Invalide); }
    let copier = |octets: &mut [u8]| {
        for ligne in 0..r.h as usize {
            let dma = (r.y as usize + ligne) * t.pas() as usize + r.x as usize * 4;
            unsafe { t.copie_dma(dma, &mut octets[ligne * pas..ligne * pas + longueur], vers_gpu) };
        }
    };
    if vers_gpu {
        copier(octets);
        fence(Ordering::SeqCst);
    }
    let reponse = unsafe { p.file.commande(&req, ENTETE) };
    if !vers_gpu && reponse == Ok(RESP_OK_NODATA) {
        fence(Ordering::SeqCst);
        copier(octets);
    }
    let ok = suivie(&mut t.memoire, reponse) == Ok(RESP_OK_NODATA);
    if vers_gpu { t.valide = ok; }
    if ok { Ok(()) } else { Err(ErreurRendu::Commande) }
}

pub fn blit_texture_rendu(session: &Arc<()>, source: u32, cible: u32,
    zones: Option<(RegionRendu, RegionRendu)>) -> Result<(), ErreurRendu>
{
    if source == cible { return Err(ErreurRendu::Invalide); }
    let mut garde = verrou().ok_or(ErreurRendu::Indisponible)?;
    let p = garde.as_mut().filter(|p| disponible(p)).ok_or(ErreurRendu::Indisponible)?;
    let cle = Arc::downgrade(session);
    let src = p.textures_rendu.indice(&cle, source)?;
    let dst = p.textures_rendu.indice(&cle, cible)?;
    let textures = &mut p.textures_rendu.textures;
    let s = textures[src].as_ref().filter(|t| t.valide).ok_or(ErreurRendu::Invalide)?;
    let d = textures[dst].as_ref().ok_or(ErreurRendu::Inconnue)?;
    let (rs, rd) = (s.region(zones.map(|(r, _)| r))?, d.region(zones.map(|(_, r)| r))?);
    if !d.valide && rd != d.pleine() { return Err(ErreurRendu::Invalide); }
    let conv = convention().ok_or(ErreurRendu::Indisponible)?;
    let rs = cg::vers_hote(conv, rs, s.hauteur as i32);
    let rd = cg::vers_hote(conv, rd, d.hauteur as i32);
    let mut q = [0; 32 + cg::BLIT_OCTETS];
    q[..32].copy_from_slice(&cg::entete_soumission(cg::BLIT_OCTETS).ok_or(ErreurRendu::Invalide)?);
    cg::encode_blit(source, rs, cible, rd, &mut q[32..]).ok_or(ErreurRendu::Invalide)?;
    let rendu = unsafe { p.file.commande(&q, ENTETE) };
    let ok = rendu == Ok(RESP_OK_NODATA);
    for i in [src, dst] {
        if let Some(t) = &mut textures[i] {
            t.memoire = dma::apres_autre(t.memoire, dma::issue(rendu));
            if i == dst { t.valide = ok; }
        }
    }
    if ok { Ok(()) } else { Err(ErreurRendu::Commande) }
}
