//! P10: encodeur de commandes Gallium VIRGL de composition de surfaces GPU.
//!
//! Correspond a virglrenderer/virgl_protocol.h (VIRGL_CCMD_BLIT = 16).
//! Ce module ne fait *aucune* allocation de ressources et n'expose pas de
//! handles pilotes directement au ring3. Les surfaces doivent deja etre
//! creees/attachees au contexte 3D et les droits valides par le gestionnaire.
//!
//! Le lot est prepare hors verrou de peripherique, puis soumis dans UNE
//! commande VIRTIO_GPU_CMD_SUBMIT_3D. Son resultat signifie que le transport
//! l'a accepte, PAS que les pixels sont visibles ou que Skia est sur GPU.

pub const VIRGL_CCMD_BLIT: u32 = 16;
pub const VIRGL_CCMD_RESOURCE_COPY_REGION: u32 = 17;
pub const PIPE_FORMAT_B8G8R8X8_UNORM: u32 = 2;
pub const PIPE_FORMAT_B8G8R8A8_UNORM: u32 = 1;
pub const PIPE_MASK_RGBA: u32 = 0x0f;
pub const VIRGL_BLIT_WORDS: usize = 22;
pub const VIRGL_BLIT_BYTES: usize = VIRGL_BLIT_WORDS * 4;
pub const MAX_LAYERS: usize = 32;
pub const MAX_COMMAND_BYTES: usize = MAX_LAYERS * VIRGL_BLIT_BYTES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageRegion {
    pub ressource: u32,
    pub largeur: u32,
    pub hauteur: u32,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Calque {
    pub source: ImageRegion,
    pub cible: ImageRegion,
    pub alpha_premultiplie: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Erreur {
    RessourceNulle,
    MemeRessource,
    Dimensions,
    Debordement,
    TropDeCalques,
    CapaciteInsuffisante,
}

fn valide(r: ImageRegion) -> Result<(), Erreur> {
    if r.ressource == 0 { return Err(Erreur::RessourceNulle); }
    if r.largeur == 0 || r.hauteur == 0 || r.largeur > 8192 || r.hauteur > 8192
        || r.x < 0 || r.y < 0 || r.w <= 0 || r.h <= 0 {
        return Err(Erreur::Dimensions);
    }
    let droite = (r.x as u64).checked_add(r.w as u64).ok_or(Erreur::Debordement)?;
    let bas = (r.y as u64).checked_add(r.h as u64).ok_or(Erreur::Debordement)?;
    if droite > r.largeur as u64 || bas > r.hauteur as u64 {
        return Err(Erreur::Debordement);
    }
    Ok(())
}

pub fn valide_calque(c: Calque) -> Result<(), Erreur> {
    valide(c.source)?;
    valide(c.cible)?;
    if c.source.ressource == c.cible.ressource { return Err(Erreur::MemeRessource); }
    Ok(())
}

fn mot(out: &mut [u8], i: usize, value: u32) {
    out[i * 4..i * 4 + 4].copy_from_slice(&value.to_le_bytes());
}

/// Ecrit le paquet VIRGL BLIT encode par le protocole, sans allocation.
/// `alpha_premultiplie` configure le bit ALPHA_BLEND du BLIT. Le blending
/// reste a verifier par lecture de pixels, car le device peut refuser le
/// format ou la combinaison de ressources.
pub fn encode_blit(calque: Calque, out: &mut [u8]) -> Result<usize, Erreur> {
    valide_calque(calque)?;
    if out.len() < VIRGL_BLIT_BYTES { return Err(Erreur::CapaciteInsuffisante); }
    let s = calque.source;
    let d = calque.cible;
    let out = &mut out[..VIRGL_BLIT_BYTES];
    out.fill(0);
    mot(out, 0, VIRGL_CCMD_BLIT | (21 << 16));
    mot(out, 1, PIPE_MASK_RGBA | (u32::from(calque.alpha_premultiplie) << 12));
    mot(out, 4, d.ressource);
    // BOUCHAUD_P10_GPU_APP_LAYERS_20261109: format reel du scanout.
    mot(out, 6, PIPE_FORMAT_B8G8R8X8_UNORM);
    mot(out, 7, d.x as u32);
    mot(out, 8, d.y as u32);
    mot(out, 10, d.w as u32);
    mot(out, 11, d.h as u32);
    mot(out, 12, 1);
    mot(out, 13, s.ressource);
    mot(out, 15, if calque.alpha_premultiplie {
        PIPE_FORMAT_B8G8R8A8_UNORM
    } else { PIPE_FORMAT_B8G8R8X8_UNORM });
    mot(out, 16, s.x as u32);
    mot(out, 17, s.y as u32);
    mot(out, 19, s.w as u32);
    mot(out, 20, s.h as u32);
    mot(out, 21, 1);
    Ok(VIRGL_BLIT_BYTES)
}

/// Encode jusqu'a 32 calques. L'ordre de `calques` definit l'ordre de
/// composition (fond -> premier plan). Toutes les ressources sont supposees
/// deja appartenir au contexte GPU du compositeur.
pub fn encode_scene(calques: &[Calque], out: &mut [u8]) -> Result<usize, Erreur> {
    if calques.is_empty() || calques.len() > MAX_LAYERS {
        return Err(Erreur::TropDeCalques);
    }
    let len = calques.len().checked_mul(VIRGL_BLIT_BYTES).ok_or(Erreur::CapaciteInsuffisante)?;
    if out.len() < len { return Err(Erreur::CapaciteInsuffisante); }
    // Validation globale avant la moindre ecriture: aucun demi-paquet.
    for c in calques { valide_calque(*c)?; }
    for (i, c) in calques.iter().enumerate() {
        encode_blit(*c, &mut out[i * VIRGL_BLIT_BYTES..(i + 1) * VIRGL_BLIT_BYTES])?;
    }
    Ok(len)
}

// API de compatibilite avec P10 bridge V1 (ancien format [u8;88]).
pub fn blit(src: ImageRegion, dst: ImageRegion, alpha: bool) -> Result<[u8; 88], Erreur> {
    let mut out = [0; 88];
    encode_blit(Calque {source: src, cible: dst, alpha_premultiplie: alpha}, &mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn r(id: u32) -> ImageRegion {
        ImageRegion {ressource:id, largeur:1280, hauteur:720, x:0,y:0,w:200,h:100}
    }
    fn get(bytes:&[u8],i:usize) -> u32 {
        u32::from_le_bytes(bytes[i*4..i*4+4].try_into().unwrap())
    }
    #[test] fn opcode_reel_et_longueur() {
        let b = blit(r(3), r(4), true).unwrap();
        assert_eq!(get(&b,0),16|(21<<16));
        assert_eq!(get(&b,1)&(1<<12),1<<12);
        assert_eq!(get(&b,4),4);
        assert_eq!(get(&b,13),3);
    }
    #[test] fn trois_fenetres_ordonnees() {
        let calques=[Calque{source:r(10),cible:r(40),alpha_premultiplie:false},
                     Calque{source:r(11),cible:r(40),alpha_premultiplie:true},
                     Calque{source:r(12),cible:r(40),alpha_premultiplie:true}];
        let mut bytes=[0;MAX_COMMAND_BYTES];
        let n=encode_scene(&calques,&mut bytes).unwrap();
        assert_eq!(n,264);
        assert_eq!(get(&bytes[88..176],13),11);
        assert_eq!(get(&bytes[176..264],13),12);
    }
    #[test] fn quatre_fenetres_composables_en_un_paquet() {
        let calques = [
            Calque { source: r(0xB120), cible: r(40), alpha_premultiplie: false },
            Calque { source: r(0xB121), cible: r(40), alpha_premultiplie: false },
            Calque { source: r(0xB122), cible: r(40), alpha_premultiplie: false },
            Calque { source: r(0xB123), cible: r(40), alpha_premultiplie: false },
        ];
        let mut bytes = [0xCCu8; MAX_COMMAND_BYTES];
        let n = encode_scene(&calques, &mut bytes).unwrap();
        assert_eq!(n, 4 * VIRGL_BLIT_BYTES);
        for (i, c) in calques.iter().enumerate() {
            let paquet = &bytes[i * VIRGL_BLIT_BYTES..(i + 1) * VIRGL_BLIT_BYTES];
            assert_eq!(get(paquet, 0), VIRGL_CCMD_BLIT | (21 << 16));
            assert_eq!(get(paquet, 13), c.source.ressource);
            assert_eq!(get(paquet, 4), c.cible.ressource);
        }
        assert_eq!(bytes[n], 0xCC);
    }
    #[test] fn aucune_ecriture_sur_scene_invalide() {
        let mut buf=[0xAB;MAX_COMMAND_BYTES];
        let bad=[Calque{source:r(1),cible:r(2),alpha_premultiplie:false},
                 Calque{source:r(0),cible:r(2),alpha_premultiplie:false}];
        assert_eq!(encode_scene(&bad,&mut buf),Err(Erreur::RessourceNulle));
        assert!(buf.iter().all(|&b|b==0xAB));
    }
    #[test] fn rectangles_hostiles_refuses() {
        let mut a=r(1);a.x=1200;a.w=100;
        assert_eq!(blit(a,r(2),false),Err(Erreur::Debordement));
        assert_eq!(blit(r(1),r(1),false),Err(Erreur::MemeRessource));
    }
}
