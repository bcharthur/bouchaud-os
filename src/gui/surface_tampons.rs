//! Possession des tampons d'une surface client (BOUCHAUD_GUI_SURFACE_DEUX_TAMPONS_V1, P10+.3c).
//!
//! ## Le defaut que ce module ferme
//!
//! Le contrat V1 n'a qu'un tampon et aucun message de retour : le client ecrit
//! dans la surface partagee, annonce `FrameReady`, et peut recommencer a
//! ecrire aussitot. Le gestionnaire de fenetres, lui, relit la surface quand
//! il compose -- apres le `FrameReady`, mais aussi a chaque recomposition
//! provoquee par autre chose (une fenetre deplacee, un menu). Rien n'empeche
//! donc le compositeur de lire une trame a moitie reecrite : une DECHIRURE,
//! sans aucun compteur pour la voir.
//!
//! ## Le contrat V2
//!
//! Deux tampons dans la meme surface (le second suit le premier). Le client
//! annonce la capacite dans `Hello` ; le gestionnaire ne l'accepte que s'il
//! l'a offerte (memoire allouee pour deux). Ensuite :
//!
//!   * `FrameReady { tampon: t }` fait de `t` la FACE : le compositeur ne lit
//!     plus que `t`, pour toute recomposition ;
//!   * l'ancienne face devient A_LIBERER, et le gestionnaire envoie
//!     `Liberation { tampon }` : a partir de la, il ne la lira plus jamais
//!     (jusqu'a ce qu'elle redevienne face) ;
//!   * le client n'ecrit que dans un tampon LIBRE (jamais annonce, ou libere).
//!
//! Une `Liberation` ne se perd pas : tant qu'elle n'a pas pu partir (canal
//! plein), le tampon reste A_LIBERER et l'envoi est retente au tour suivant.
//! Un client qui attend une liberation perdue serait fige pour toujours.
//!
//! Ce module est PUR (ni noyau, ni memoire, ni canal) et teste sur l'hote :
//! `tools/gui/test_surface_tampons.rs`.

/// Bit de `Hello` (troisieme mot) : le client sait travailler a deux tampons.
pub const CAPACITE_DEUX_TAMPONS: u32 = 1;

/// Nombre maximal de tampons d'une surface.
pub const TAMPONS_MAX: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Etat {
    /// Le client peut y ecrire : le compositeur ne le lit pas.
    Libre,
    /// Le compositeur le lit (la derniere trame annoncee).
    Face,
    /// Le compositeur ne le lit plus, mais le client ne le sait pas encore :
    /// la `Liberation` reste a envoyer.
    ALiberer,
}

/// Ce qu'un `FrameReady` a appris au gestionnaire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Issue {
    /// Trame conforme : la face est (ou reste, en V1) ce tampon.
    Acceptee,
    /// Tampon hors de la surface : le degat est ignore, la face ne change pas.
    HorsBornes,
    /// Le client a reecrit le tampon que le compositeur lisait (V2) : la
    /// trame est prise -- elle est deja dans la memoire --, mais comptee.
    EcritureSurFace,
    /// Le client a ecrit un tampon dont il n'avait pas recu la liberation
    /// (V2) : prise et comptee de meme.
    EcritureSansLiberation,
}

/// L'etat des tampons d'une surface, cote gestionnaire de fenetres.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tampons {
    nombre: usize,
    etats: [Etat; TAMPONS_MAX],
    face: Option<usize>,
}

impl Tampons {
    /// Le contrat V1 : un tampon, aucune liberation.
    pub const fn simple() -> Tampons {
        Tampons { nombre: 1, etats: [Etat::Libre; TAMPONS_MAX], face: None }
    }

    /// Le contrat V2 : deux tampons, libres tous deux.
    pub const fn double() -> Tampons {
        Tampons { nombre: 2, etats: [Etat::Libre; TAMPONS_MAX], face: None }
    }

    /// Le contrat retenu a la poignee de main : V2 seulement si le
    /// gestionnaire l'a offert ET si le client l'annonce.
    pub const fn negocie(offerts: usize, capacites: u32) -> Tampons {
        if offerts >= 2 && (capacites & CAPACITE_DEUX_TAMPONS) != 0 {
            Tampons::double()
        } else {
            Tampons::simple()
        }
    }

    pub const fn nombre(&self) -> usize {
        self.nombre
    }

    /// Le tampon que le compositeur doit lire. Avant toute trame : 0 (la
    /// surface neuve, effacee a l'allocation).
    pub const fn face(&self) -> usize {
        match self.face {
            Some(t) => t,
            None => 0,
        }
    }

    pub fn etat(&self, tampon: usize) -> Option<Etat> {
        if tampon < self.nombre {
            Some(self.etats[tampon])
        } else {
            None
        }
    }

    /// `FrameReady { tampon }` vient d'etre lu.
    pub fn trame_prete(&mut self, tampon: u32) -> Issue {
        let t = tampon as usize;
        if t >= self.nombre {
            return Issue::HorsBornes;
        }
        if self.nombre == 1 {
            // V1 : le seul tampon est la face ; il n'y a rien a liberer.
            self.etats[0] = Etat::Face;
            self.face = Some(0);
            return Issue::Acceptee;
        }
        let issue = match self.etats[t] {
            Etat::Libre => Issue::Acceptee,
            Etat::Face => Issue::EcritureSurFace,
            Etat::ALiberer => Issue::EcritureSansLiberation,
        };
        if let Some(ancienne) = self.face {
            if ancienne != t {
                self.etats[ancienne] = Etat::ALiberer;
            }
        }
        self.etats[t] = Etat::Face;
        self.face = Some(t);
        issue
    }

    /// Le prochain tampon dont la `Liberation` reste a envoyer.
    pub fn a_liberer(&self) -> Option<u32> {
        (0..self.nombre).find(|&t| self.etats[t] == Etat::ALiberer).map(|t| t as u32)
    }

    /// La `Liberation` de `tampon` est partie : il redevient libre. Rend
    /// `false` (et ne change rien) si ce tampon n'etait pas a liberer.
    pub fn liberation_envoyee(&mut self, tampon: u32) -> bool {
        let t = tampon as usize;
        if t < self.nombre && self.etats[t] == Etat::ALiberer {
            self.etats[t] = Etat::Libre;
            true
        } else {
            false
        }
    }
}

/// Le troisieme mot de `Hello` (capacites), zero pour une charge V1.
pub fn capacites_hello(charge: &[u8]) -> u32 {
    if charge.len() >= 12 {
        u32::from_le_bytes([charge[8], charge[9], charge[10], charge[11]])
    } else {
        0
    }
}
