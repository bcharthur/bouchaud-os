//! BOUCHAUD_GPU_COMPOSITION_V2 (P10) : la partie PURE du compositeur GPU.
//!
//! # Le partage du travail
//!
//! * Le PROCESSEUR peint ce qui change rarement et coute peu : fond, cadres,
//!   barres, curseur. Il le peint directement dans la memoire du scanout
//!   (P10+.2, composition directe) et ne transfere que les rectangles abimes.
//! * Le GPU (virglrenderer cote hote) compose ce qui est gros et change a
//!   chaque trame : le contenu des fenetres ring 3 (Ladybird, DOOM, toute
//!   application). Chaque surface cliente devient une TEXTURE adossee
//!   directement a ses propres pages physiques ; le contenu y monte par
//!   `TRANSFER_TO_HOST_3D` du seul rectangle abime (copie faite par l'hote),
//!   puis un `BLIT` GPU le pose dans le scanout. Le processeur invite ne copie
//!   plus un seul pixel client.
//!
//! # Ce que ce module decide
//!
//! * `soustrait` : la partie VISIBLE d'une fenetre (sa zone moins les fenetres
//!   au-dessus d'elle, le curseur, les menus) -- ce que le GPU pose. Le reste
//!   (sous une ombre, sous le curseur) est peint par le processeur, au pixel.
//! * `entrees_memoire` : la liste `(adresse, longueur)` des pages d'une
//!   surface, pages contigues fusionnees, pour `RESOURCE_ATTACH_BACKING`.
//! * l'encodage des commandes 3D propres au compositeur (texture a origine en
//!   haut, transfert d'un rectangle, BLIT).
//!
//! Aucun `use crate::`, aucun `unsafe` : tout se verifie sur l'hote
//! (`tools/display/test_composition_gpu.rs`).

/// Un rectangle en pixels, origine en haut a gauche.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct R {
    pub x: i32,
    pub y: i32,
    pub l: i32,
    pub h: i32,
}

impl R {
    pub const fn neuf(x: i32, y: i32, l: i32, h: i32) -> R {
        R { x, y, l, h }
    }
    pub fn vide(&self) -> bool {
        self.l <= 0 || self.h <= 0
    }
    pub fn droite(&self) -> i32 {
        self.x.saturating_add(self.l)
    }
    pub fn bas(&self) -> i32 {
        self.y.saturating_add(self.h)
    }
    pub fn inter(&self, o: &R) -> R {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = self.droite().min(o.droite());
        let y1 = self.bas().min(o.bas());
        if x1 <= x0 || y1 <= y0 {
            R::default()
        } else {
            R::neuf(x0, y0, x1 - x0, y1 - y0)
        }
    }
    pub fn touche(&self, o: &R) -> bool {
        !self.inter(o).vide()
    }
    /// Plus petit rectangle qui contient les deux (le vide est neutre).
    pub fn union(&self, o: &R) -> R {
        if self.vide() {
            return *o;
        }
        if o.vide() {
            return *self;
        }
        let x0 = self.x.min(o.x);
        let y0 = self.y.min(o.y);
        let x1 = self.droite().max(o.droite());
        let y1 = self.bas().max(o.bas());
        R::neuf(x0, y0, x1 - x0, y1 - y0)
    }
    pub fn aire(&self) -> u64 {
        if self.vide() { 0 } else { self.l as u64 * self.h as u64 }
    }
}

/// Nombre maximal de morceaux visibles d'une fenetre composee par le GPU.
/// Au-dela, la fenetre est trop decoupee pour que le GPU y gagne : elle
/// retombe sur la composition par le processeur.
pub const MORCEAUX_MAX: usize = 16;

/// `base` moins l'union des `trous`, en rectangles disjoints, dans `sortie`.
///
/// Rend le nombre de morceaux, ou `None` si `sortie` ne suffit pas (l'appelant
/// compose alors la fenetre par le processeur : jamais de morceau perdu).
/// Decoupe classique en bandes : chaque trou coupe chaque morceau restant en
/// au plus quatre (haut, bas, gauche, droite).
pub fn soustrait(base: R, trous: &[R], sortie: &mut [R]) -> Option<usize> {
    if base.vide() {
        return Some(0);
    }
    if sortie.is_empty() {
        return None;
    }
    let mut n = 1usize;
    sortie[0] = base;
    let mut tampon = [R::default(); MORCEAUX_MAX * 4];
    for trou in trous {
        if trou.vide() {
            continue;
        }
        let mut m = 0usize;
        for i in 0..n {
            let p = sortie[i];
            let c = p.inter(trou);
            if c.vide() {
                if m >= tampon.len() { return None; }
                tampon[m] = p;
                m += 1;
                continue;
            }
            let morceaux = [
                R::neuf(p.x, p.y, p.l, c.y - p.y),                       // haut
                R::neuf(p.x, c.bas(), p.l, p.bas() - c.bas()),           // bas
                R::neuf(p.x, c.y, c.x - p.x, c.h),                       // gauche
                R::neuf(c.droite(), c.y, p.droite() - c.droite(), c.h),  // droite
            ];
            for q in morceaux {
                if q.vide() { continue; }
                if m >= tampon.len() { return None; }
                tampon[m] = q;
                m += 1;
            }
        }
        if m > sortie.len() {
            return None;
        }
        sortie[..m].copy_from_slice(&tampon[..m]);
        n = m;
        if n == 0 {
            break;
        }
    }
    Some(n)
}

/// Une entree `virtio_gpu_mem_entry` : adresse physique et longueur.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Entree {
    pub adresse: u64,
    pub longueur: u32,
}

/// Les pages `frames` (une par 4 Kio, dans l'ordre de la surface) en entrees
/// memoire, les pages physiquement contigues fusionnees, sur `octets` octets.
/// Rend le nombre d'entrees, ou `None` si `sortie` ne suffit pas ou si les
/// pages ne couvrent pas `octets`.
pub fn entrees_memoire(frames: &[u64], octets: usize, page: usize, sortie: &mut [Entree]) -> Option<usize> {
    if octets == 0 || page == 0 {
        return None;
    }
    let pages = octets.div_ceil(page);
    if frames.len() < pages {
        return None;
    }
    let mut n = 0usize;
    let mut reste = octets;
    for &f in &frames[..pages] {
        if f == 0 || f % page as u64 != 0 {
            return None;
        }
        let ici = reste.min(page) as u32;
        reste -= ici as usize;
        if n > 0 {
            let d = &mut sortie[n - 1];
            if d.adresse + d.longueur as u64 == f && d.longueur as u64 + ici as u64 <= u32::MAX as u64 {
                d.longueur += ici;
                continue;
            }
        }
        if n >= sortie.len() {
            return None;
        }
        sortie[n] = Entree { adresse: f, longueur: ici };
        n += 1;
    }
    Some(n)
}

// ---- Commandes virtio-gpu 3D du compositeur ---------------------------------

pub const CMD_RESOURCE_UNREF: u32 = 0x0102;
pub const CMD_RESOURCE_ATTACH_BACKING: u32 = 0x0106;
pub const CMD_RESOURCE_DETACH_BACKING: u32 = 0x0107;
pub const CMD_CTX_ATTACH_RESOURCE: u32 = 0x0202;
pub const CMD_CTX_DETACH_RESOURCE: u32 = 0x0203;
pub const CMD_RESOURCE_CREATE_3D: u32 = 0x0204;
pub const CMD_TRANSFER_TO_HOST_3D: u32 = 0x0205;
pub const CMD_SUBMIT_3D: u32 = 0x0207;
/// Le contexte VIRGL du noyau (celui de la sonde, `virtio_gpu_3d`).
pub const CONTEXTE: u32 = 0xB104;
/// Les textures des fenetres : `TEXTURE_BASE + emplacement`.
pub const TEXTURE_BASE: u32 = 0xB200;
pub const TEXTURES_MAX: usize = 8;
/// `PIPE_TEXTURE_2D`, `B8G8R8X8_UNORM`, `PIPE_BIND_SAMPLER_VIEW`.
const CIBLE_TEXTURE_2D: u32 = 2;
pub const FORMAT_B8G8R8X8: u32 = 2;
const LIAISON_ECHANTILLON: u32 = 1 << 3;
/// `VIRTIO_GPU_RESOURCE_FLAG_Y_0_TOP` : la ligne 0 est en HAUT, comme la
/// surface du client et comme la ressource 2D du scanout (QEMU la cree avec ce
/// drapeau). Sans lui, la texture est lue a l'envers -- c'etait l'image
/// retournee du 9 novembre, corrigee alors par une recopie ligne a ligne.
pub const DRAPEAU_Y_0_TOP: u32 = 1;

fn p32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
fn p64(b: &mut [u8], o: usize, v: u64) {
    b[o..o + 8].copy_from_slice(&v.to_le_bytes());
}

/// `PIPE_BIND_RENDER_TARGET` : cible d'un BLIT (autotest).
pub const LIAISON_CIBLE: u32 = 1 << 1;

/// `RESOURCE_CREATE_3D` d'une texture de fenetre `largeur` x `hauteur`.
pub fn cree_texture(id: u32, largeur: u32, hauteur: u32) -> Option<[u8; 72]> {
    cree_texture_liee(id, largeur, hauteur, LIAISON_ECHANTILLON)
}

/// `cree_texture` avec des liaisons choisies.
pub fn cree_texture_liee(id: u32, largeur: u32, hauteur: u32, liaison: u32) -> Option<[u8; 72]> {
    if id == 0 || largeur == 0 || hauteur == 0 || largeur > 8192 || hauteur > 8192 {
        return None;
    }
    let mut q = [0u8; 72];
    p32(&mut q, 0, CMD_RESOURCE_CREATE_3D);
    p32(&mut q, 24, id);
    p32(&mut q, 28, CIBLE_TEXTURE_2D);
    p32(&mut q, 32, FORMAT_B8G8R8X8);
    p32(&mut q, 36, liaison);
    p32(&mut q, 40, largeur);
    p32(&mut q, 44, hauteur);
    p32(&mut q, 48, 1); // profondeur
    p32(&mut q, 52, 1); // tableau
    p32(&mut q, 64, DRAPEAU_Y_0_TOP);
    Some(q)
}

/// En-tete de `RESOURCE_ATTACH_BACKING` pour `n` entrees (les entrees suivent
/// dans un descripteur a part, voir `virtio_gpu::commande_chainee`).
pub fn entete_attache(id: u32, n: u32) -> [u8; 32] {
    let mut q = [0u8; 32];
    p32(&mut q, 0, CMD_RESOURCE_ATTACH_BACKING);
    p32(&mut q, 24, id);
    p32(&mut q, 28, n);
    q
}

/// Les entrees, 16 octets chacune, dans `sortie` ; rend la longueur ecrite.
pub fn encode_entrees(entrees: &[Entree], sortie: &mut [u8]) -> Option<usize> {
    let n = entrees.len().checked_mul(16)?;
    if sortie.len() < n {
        return None;
    }
    for (i, e) in entrees.iter().enumerate() {
        p64(sortie, i * 16, e.adresse);
        p32(sortie, i * 16 + 8, e.longueur);
        p32(sortie, i * 16 + 12, 0);
    }
    Some(n)
}

pub fn detache_memoire(id: u32) -> [u8; 32] {
    let mut q = [0u8; 32];
    p32(&mut q, 0, CMD_RESOURCE_DETACH_BACKING);
    p32(&mut q, 24, id);
    q
}

pub fn libere(id: u32) -> [u8; 32] {
    let mut q = [0u8; 32];
    p32(&mut q, 0, CMD_RESOURCE_UNREF);
    p32(&mut q, 24, id);
    q
}

pub fn contexte(op_attache: bool, id: u32) -> [u8; 32] {
    let mut q = [0u8; 32];
    p32(&mut q, 0, if op_attache { CMD_CTX_ATTACH_RESOURCE } else { CMD_CTX_DETACH_RESOURCE });
    p32(&mut q, 16, CONTEXTE);
    p32(&mut q, 24, id);
    q
}

/// `TRANSFER_TO_HOST_3D` du rectangle `r` (coordonnees de la texture) depuis
/// la memoire attachee, ou la ligne `y` du tampon commence a
/// `base + y * pas`.
pub fn transfert(id: u32, r: R, base: u64, pas: u32, largeur: u32, hauteur: u32) -> Option<[u8; 72]> {
    if r.vide() || r.x < 0 || r.y < 0 || r.droite() as u32 > largeur || r.bas() as u32 > hauteur {
        return None;
    }
    if pas < largeur.checked_mul(4)? {
        return None;
    }
    let decalage = base.checked_add(r.y as u64 * pas as u64)?.checked_add(r.x as u64 * 4)?;
    let mut q = [0u8; 72];
    p32(&mut q, 0, CMD_TRANSFER_TO_HOST_3D);
    // Contexte 0 : le transfert ne depend d'aucun etat de contexte.
    // virtio_gpu_box : x, y, z, w, h, d
    p32(&mut q, 24, r.x as u32);
    p32(&mut q, 28, r.y as u32);
    p32(&mut q, 32, 0);
    p32(&mut q, 36, r.l as u32);
    p32(&mut q, 40, r.h as u32);
    p32(&mut q, 44, 1);
    p64(&mut q, 48, decalage);
    p32(&mut q, 56, id);
    p32(&mut q, 60, 0); // niveau
    p32(&mut q, 64, pas);
    p32(&mut q, 68, 0); // pas de couche (une seule)
    Some(q)
}

/// Un BLIT VIRGL : `source` (dans une texture) vers `cible` (dans le scanout).
pub const BLIT_OCTETS: usize = 88;
pub const SOUMISSION_MAX: usize = 2016;
pub const BLITS_PAR_SOUMISSION: usize = SOUMISSION_MAX / BLIT_OCTETS;

pub fn encode_blit(src_id: u32, src: R, dst_id: u32, dst: R, sortie: &mut [u8]) -> Option<usize> {
    if src.vide() || dst.vide() || src.l != dst.l || src.h != dst.h
        || src.x < 0 || src.y < 0 || dst.x < 0 || dst.y < 0 || src_id == 0 || dst_id == 0
        || src_id == dst_id || sortie.len() < BLIT_OCTETS {
        return None;
    }
    let o = &mut sortie[..BLIT_OCTETS];
    o.fill(0);
    p32(o, 0, 16 | (21 << 16)); // VIRGL_CCMD_BLIT, 21 mots
    p32(o, 4, 0x0f);            // masque RGBA, filtre nearest, sans ciseaux
    p32(o, 16, dst_id);
    p32(o, 24, FORMAT_B8G8R8X8);
    p32(o, 28, dst.x as u32);
    p32(o, 32, dst.y as u32);
    p32(o, 40, dst.l as u32);
    p32(o, 44, dst.h as u32);
    p32(o, 48, 1);
    p32(o, 52, src_id);
    p32(o, 60, FORMAT_B8G8R8X8);
    p32(o, 64, src.x as u32);
    p32(o, 68, src.y as u32);
    p32(o, 76, src.l as u32);
    p32(o, 80, src.h as u32);
    p32(o, 84, 1);
    Some(BLIT_OCTETS)
}

/// En-tete de `SUBMIT_3D` pour `longueur` octets de commandes VIRGL.
pub fn entete_soumission(longueur: usize) -> Option<[u8; 32]> {
    if longueur == 0 || longueur % 4 != 0 || longueur > SOUMISSION_MAX {
        return None;
    }
    let mut q = [0u8; 32];
    p32(&mut q, 0, CMD_SUBMIT_3D);
    p32(&mut q, 16, CONTEXTE);
    p32(&mut q, 24, longueur as u32);
    Some(q)
}

// ---- Convention de coordonnees du BLIT (calibree au demarrage) --------------
//
// Mesure du 10 novembre (QEMU 8.2 + virglrenderer, llvmpipe) : le BLIT copie
// en coordonnees GL BRUTES, sans tenir compte du drapeau Y_0_TOP -- or une
// ressource Y_0_TOP est STOCKEE a l'envers. Une fenetre posee en (y, h)
// atterrissait donc en (H - y - h) du scanout et lisait (Htex - y - h) de sa
// texture : la capture montrait l'ecran 249..648 alimente par les lignes
// 172..571 de la surface, exactement ce que predit ce modele. Rien ne garantit
// qu'un autre hote fasse de meme : la convention est MESUREE par un autotest
// (BLIT puis relecture `TRANSFER_FROM_HOST_3D`), jamais supposee.

/// Comment l'hote interprete les coordonnees d'un BLIT entre ressources Y_0_TOP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Convention {
    /// Coordonnees GL brutes : il faut retourner `y` soi-meme.
    Brut,
    /// L'hote traduit lui-meme (ligne 0 en haut).
    Haut,
}

/// Le rectangle `r` (ligne 0 en haut) d'une ressource Y_0_TOP de hauteur
/// `hauteur`, dans la convention de l'hote.
pub fn vers_hote(c: Convention, r: R, hauteur: i32) -> R {
    match c {
        Convention::Haut => r,
        Convention::Brut => R::neuf(r.x, hauteur - r.y - r.h, r.l, r.h),
    }
}

pub const CMD_TRANSFER_FROM_HOST_3D: u32 = 0x0206;

/// `TRANSFER_FROM_HOST_3D` : relit le rectangle `r` de la texture vers sa
/// memoire attachee (meme disposition que `transfert`).
pub fn relecture(id: u32, r: R, pas: u32, largeur: u32, hauteur: u32) -> Option<[u8; 72]> {
    let mut q = transfert(id, r, 0, pas, largeur, hauteur)?;
    p32(&mut q, 0, CMD_TRANSFER_FROM_HOST_3D);
    Some(q)
}

/// La mire de l'autotest : chaque pixel code sa ligne et sa colonne.
pub fn pixel_mire(x: u32, y: u32) -> u32 {
    0x0080_0000 | (y << 8) | x
}

/// Juge la relecture de l'autotest : la cible (`largeur` x `hauteur`, pas
/// `largeur * 4`) doit montrer, aux lignes `[dy, dy + h)`, les lignes
/// `[sy, sy + h)` de la mire, et rien d'autre ne doit y etre ecrit.
pub fn juge_autotest(cible: &[u32], largeur: u32, hauteur: u32, sy: u32, dy: u32, h: u32) -> bool {
    if cible.len() < (largeur * hauteur) as usize {
        return false;
    }
    for y in 0..hauteur {
        for x in 0..largeur {
            let v = cible[(y * largeur + x) as usize];
            let attendu = if y >= dy && y < dy + h { pixel_mire(x, sy + (y - dy)) } else { 0 };
            if v & 0x00ff_ffff != attendu {
                return false;
            }
        }
    }
    true
}
