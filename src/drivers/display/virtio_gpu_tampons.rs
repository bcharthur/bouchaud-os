//! Qui peut ecrire la memoire du scanout virtio-gpu, et ce que coute une
//! trame en copies (BOUCHAUD_VIRTIO_GPU_TAMPONS_V1, P10+.2).
//!
//! # Ce qui motive
//!
//! Sous KVM, presenter l'ecran entier coute ~400 us (`P10_GPU_AUDIT` 4
//! sexies), dont la COPIE CPU de 3,7 Mo du tampon arriere du bureau (`BACK`)
//! vers la memoire DMA du scanout. En 2D, l'hote ne lit cette memoire que
//! pendant `TRANSFER_TO_HOST_2D` : composer DIRECTEMENT dans la memoire DMA
//! supprime la copie, A UNE CONDITION -- personne n'y ecrit pendant qu'un
//! transfert est en vol, sinon l'hote copie un rectangle a moitie redessine.
//!
//! # Le modele
//!
//! Un etat par tampon de scanout :
//!
//!   * `Composition` : le bureau peut ecrire ; aucun transfert en vol ;
//!   * `Transfert` : un `TRANSFER_TO_HOST_2D` est parti et sa reponse
//!     (barriere comprise) n'est pas venue : l'hote PEUT lire. Aucune
//!     ecriture, aucune seconde soumission ;
//!   * `Inconnu` : un transfert est reste sans reponse. On ne sait pas si
//!     l'hote lira encore : etat absorbant, le tampon n'est plus jamais
//!     ecrit (meme regle que `virtio_gpu_dma::Memoire::Inconnue`).
//!
//! Le mode de presentation : `Copie` (le tampon arriere du bureau, puis une
//! copie du rectangle vers la memoire DMA -- le comportement livre) ou
//! `Direct` (le bureau compose dans la memoire DMA). `Direct` n'est jamais
//! choisi pour un tampon `Inconnu`.
//!
//! A deux tampons (bascule par `SET_SCANOUT`), celui qu'on reprend porte la
//! trame d'il y a `age` presentations : les dommages des `age - 1` trames
//! suivantes doivent y etre REJOUES (`a_rejouer`) -- une copie qui ne
//! disparait pas, et qui est comptee (`octets_copies`).
//!
//! Module pur : `tools/display/test_virtio_gpu_tampons.rs`.

/// Un rectangle de pixels (coordonnees du scanout).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub largeur: u32,
    pub hauteur: u32,
}

impl Rect {
    pub const fn vide() -> Self {
        Self { x: 0, y: 0, largeur: 0, hauteur: 0 }
    }

    pub const fn est_vide(&self) -> bool {
        self.largeur == 0 || self.hauteur == 0
    }

    pub const fn pixels(&self) -> u64 {
        self.largeur as u64 * self.hauteur as u64
    }

    /// Le plus petit rectangle qui contient les deux.
    pub fn union(self, autre: Rect) -> Rect {
        if self.est_vide() {
            return autre;
        }
        if autre.est_vide() {
            return self;
        }
        let x0 = self.x.min(autre.x);
        let y0 = self.y.min(autre.y);
        let x1 = (self.x + self.largeur).max(autre.x + autre.largeur);
        let y1 = (self.y + self.hauteur).max(autre.y + autre.hauteur);
        Rect { x: x0, y: y0, largeur: x1 - x0, hauteur: y1 - y0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Etat {
    Composition,
    Transfert,
    Inconnu,
}

/// Comment s'est fini un transfert.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fin {
    /// Le peripherique a REPONDU (accepte ou refuse) -- barriere comprise --,
    /// ou la requete n'est jamais partie : l'hote ne lit plus.
    Achevee,
    /// La requete est partie, la reponse n'est pas venue.
    SansReponse,
}

/// Le bureau peut-il ecrire ce tampon ?
pub fn peut_ecrire(e: Etat) -> bool {
    e == Etat::Composition
}

/// Un transfert part. `None` : refuse (deja en vol, ou tampon inconnu).
pub fn soumet(e: Etat) -> Option<Etat> {
    match e {
        Etat::Composition => Some(Etat::Transfert),
        Etat::Transfert | Etat::Inconnu => None,
    }
}

/// La fin d'un transfert. Une fin sans transfert en vol ne change rien.
pub fn acheve(e: Etat, fin: Fin) -> Etat {
    match (e, fin) {
        (Etat::Inconnu, _) => Etat::Inconnu,
        (Etat::Transfert, Fin::Achevee) => Etat::Composition,
        (Etat::Transfert, Fin::SansReponse) => Etat::Inconnu,
        (Etat::Composition, _) => Etat::Composition,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Le tampon arriere du bureau, puis une copie vers la memoire DMA.
    Copie,
    /// Le bureau compose dans la memoire DMA.
    Direct,
}

/// Le mode d'une trame : `Direct` seulement s'il est voulu, que la memoire
/// DMA est la (assez grande pour l'ecran) et que le tampon n'est pas inconnu.
pub fn mode(direct_voulu: bool, memoire_suffisante: bool, e: Etat) -> Mode {
    if direct_voulu && memoire_suffisante && e != Etat::Inconnu {
        Mode::Direct
    } else {
        Mode::Copie
    }
}

/// Ce qu'il faut rejouer dans un tampon repris apres `age` presentations
/// (`historique[0]` : le dommage de la trame la plus recente). `age == 0` :
/// contenu inconnu (jamais presente), tout l'ecran ; `age == 1` : il porte
/// la derniere trame, rien a rejouer.
pub fn a_rejouer(age: u32, historique: &[Rect], ecran: Rect) -> Rect {
    if age == 0 {
        return ecran;
    }
    let manquantes = (age - 1) as usize;
    if manquantes > historique.len() {
        // L'historique ne remonte pas assez loin : on ne sait pas ce que le
        // tampon a manque.
        return ecran;
    }
    historique[..manquantes].iter().fold(Rect::vide(), |u, r| u.union(*r))
}

/// Octets copies par le processeur pour une trame : le dommage en `Copie`
/// (tampon arriere -> DMA), le rejeu en `Direct` (nul a un seul tampon).
pub fn octets_copies(m: Mode, dommage: Rect, rejoue: Rect) -> u64 {
    match m {
        Mode::Copie => dommage.pixels() * 4,
        Mode::Direct => rejoue.pixels() * 4,
    }
}
