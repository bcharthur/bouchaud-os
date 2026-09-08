// L'allocateur compagnon : des blocs CONTIGUS, et une fusion qui ne parcourt
// rien.
//
// # Ce que l'allocateur de frames ne savait pas faire
//
// Il alloue une page. Une seule. `alloc_frame()` rend une frame, `free_frame()`
// en rend une. Le bitmap a supprime le parcours O(n) du double `free`, et il
// n'a rien change au fait qu'on ne peut demander qu'une page a la fois.
//
// Trois consequences, et aucune n'est theorique :
//
//   * **Le DMA a du se faire une arene a part.** Un pilote qui veut seize
//     pages CONTIGUES ne peut pas les demander : seize appels rendent seize
//     pages dispersees, et un anneau de commandes NVMe disperse ne marche pas.
//     L'arene DMA existe donc a cote, avec sa propre reserve fixe, ses propres
//     regions et sa propre fragmentation -- deux allocateurs qui ne peuvent
//     pas se preter de memoire.
//
//   * **Pas de grandes pages.** Une entree de 2 Mio demande 512 pages
//     contigues ALIGNEES. Sans elles, chaque acces au tas du navigateur coute
//     une entree de TLB de 4 Kio, et le TLB deborde bien avant la fin d'un
//     document.
//
//   * **Toute allocation noyau de plus d'un kibioctet descend dans
//     `LockedHeap`**, avec son verrou global. Les six classes de taille
//     s'arretent a 1024 octets ; au-dela, un `Vec` qui grandit prend le verrou
//     que tout le reste du chantier 1 s'emploie a eviter.
//
// # Pourquoi « compagnon » plutot qu'une liste par taille
//
// Une liste par taille alloue vite et ne FUSIONNE pas : la memoire rendue
// reste decoupee dans la classe ou elle a ete decoupee, et un systeme qui
// alterne petites et grandes demandes finit avec de la memoire libre qu'il ne
// peut plus donner. Le compagnon fusionne, et il fusionne en temps constant :
// l'adresse du bloc jumeau est un OU EXCLUSIF, pas une recherche.
//
// # Ce fichier ne touche pas la memoire physique
//
// Il manipule des NUMEROS de blocs et delegue les deux seules operations qui
// touchent la memoire -- lire et ecrire le chainage d'un bloc libre -- a un
// `Terrain`. C'est ce qui permet a `tools/platform/test_compagnon.rs` de lui
// donner un terrain fait de `Vec`, et de verifier chaque fusion, chaque
// division et chaque cas de bord sans une machine.
//
// # Le chainage vit DANS les blocs libres
//
// Un bloc libre ne sert a personne : son premier mot porte le numero du bloc
// libre suivant. C'est pour cela qu'un allocateur compagnon n'a pas besoin de
// memoire a lui pour ses listes -- seulement pour son bitmap.

#![allow(dead_code)]

/// Ordre maximal. L'ordre `k` couvre `2^k` blocs elementaires ; avec des pages
/// de 4 Kio, l'ordre 10 est un bloc de 4 Mio.
///
/// Au-dela, la fusion coute plus qu'elle ne rend : les demandes de plus de
/// quatre mebioctets contigus sont rares, et les servir depuis un ordre
/// superieur immobiliserait de gros blocs pour les rares fois ou l'on en a
/// besoin.
pub const ORDRES: usize = 11;

/// Marque de fin de liste. Zero serait un numero de bloc valide.
pub const AUCUN: usize = usize::MAX;

/// Ce que l'allocateur a besoin de faire au terrain.
///
/// Deux operations, et elles ne touchent QUE des blocs libres -- c'est-a-dire
/// de la memoire dont personne ne se sert. Ecrire dans un bloc alloue serait
/// une corruption ; le contrat le dit, et `verifie-compagnon.py` le verifie.
pub trait Terrain {
    /// Le numero du bloc libre suivant, lu dans le bloc `bloc`.
    fn lien(&self, bloc: usize) -> usize;
    /// Ecrit le chainage dans le bloc `bloc`.
    fn pose_lien(&mut self, bloc: usize, suivant: usize);
}

/// Le numero du bloc JUMEAU, au meme ordre.
///
/// C'est un ou exclusif, et c'est toute l'astuce : trouver le compagnon d'un
/// bloc ne demande aucune recherche, aucune table, aucun parcours. Deux blocs
/// d'ordre `k` sont jumeaux si et seulement s'ils ne different que par le bit
/// `k`.
#[inline]
pub const fn jumeau(bloc: usize, ordre: usize) -> usize {
    bloc ^ (1usize << ordre)
}

/// Le bloc pere, a l'ordre `ordre + 1`.
#[inline]
pub const fn pere(bloc: usize, ordre: usize) -> usize {
    bloc & !(1usize << ordre)
}

/// Le bloc est-il aligne pour son ordre ?
///
/// Un bloc d'ordre `k` commence toujours a un multiple de `2^k`. Un numero qui
/// ne l'est pas ne designe pas un bloc de cet ordre : le liberer fusionnerait
/// avec un jumeau qui n'existe pas.
#[inline]
pub const fn aligne(bloc: usize, ordre: usize) -> bool {
    bloc & ((1usize << ordre) - 1) == 0
}

/// L'ordre minimal capable de porter `blocs` blocs elementaires.
pub const fn ordre_pour(blocs: usize) -> usize {
    if blocs <= 1 {
        return 0;
    }
    let mut ordre = 0usize;
    while ordre + 1 < ORDRES {
        if (1usize << ordre) >= blocs {
            return ordre;
        }
        ordre += 1;
    }
    ordre
}

/// Mots de bitmap necessaires pour `blocs` blocs elementaires.
///
/// Un bit par bloc et par ordre. Les ordres superieurs en demandent moins,
/// mais leur donner la meme place a tous evite une arithmetique de decalage
/// dont chaque erreur ferait lire l'etat d'un autre ordre.
pub const fn mots_bitmap(blocs: usize) -> usize {
    ORDRES * ((blocs + 63) / 64)
}

/// L'allocateur.
///
/// `libre` porte, pour chaque ordre, un bit par bloc : « ce bloc est en tete
/// d'un bloc libre de CET ordre ». Il ne sert pas a trouver un bloc -- les
/// listes le font -- mais a decider d'une FUSION : quand on rend un bloc, on
/// demande si son jumeau est libre au meme ordre, et la reponse est un test de
/// bit.
pub struct Compagnon<'a> {
    /// Blocs elementaires couverts.
    blocs: usize,
    /// Mots par ordre dans `libre`.
    mots_par_ordre: usize,
    libre: &'a mut [u64],
    tetes: [usize; ORDRES],
    /// Blocs elementaires actuellement libres.
    disponibles: usize,
    allocations: u64,
    liberations: u64,
    divisions: u64,
    fusions: u64,
    echecs: u64,
}

impl<'a> Compagnon<'a> {
    /// Un allocateur vide couvrant `blocs` blocs elementaires.
    ///
    /// Rend `None` quand le bitmap fourni est trop court : accepter un bitmap
    /// court ferait lire l'etat d'un bloc hors zone, ce qui rendrait une
    /// reponse plausible et fausse.
    pub fn neuf(blocs: usize, libre: &'a mut [u64]) -> Option<Self> {
        if blocs == 0 || libre.len() < mots_bitmap(blocs) {
            return None;
        }
        libre.fill(0);
        Some(Self {
            blocs,
            mots_par_ordre: (blocs + 63) / 64,
            libre,
            tetes: [AUCUN; ORDRES],
            disponibles: 0,
            allocations: 0,
            liberations: 0,
            divisions: 0,
            fusions: 0,
            echecs: 0,
        })
    }

    #[inline]
    fn position(&self, bloc: usize, ordre: usize) -> (usize, u64) {
        let mot = ordre * self.mots_par_ordre + bloc / 64;
        (mot, 1u64 << (bloc % 64))
    }

    #[inline]
    fn est_libre(&self, bloc: usize, ordre: usize) -> bool {
        if bloc >= self.blocs || ordre >= ORDRES {
            return false;
        }
        let (mot, masque) = self.position(bloc, ordre);
        self.libre[mot] & masque != 0
    }

    #[inline]
    fn marque(&mut self, bloc: usize, ordre: usize, libre: bool) {
        let (mot, masque) = self.position(bloc, ordre);
        if libre {
            self.libre[mot] |= masque;
        } else {
            self.libre[mot] &= !masque;
        }
    }

    /// Met un bloc en tete de la liste de son ordre.
    fn empile<T: Terrain>(&mut self, terrain: &mut T, bloc: usize, ordre: usize) {
        terrain.pose_lien(bloc, self.tetes[ordre]);
        self.tetes[ordre] = bloc;
        self.marque(bloc, ordre, true);
    }

    /// Retire un bloc PRECIS de la liste de son ordre.
    ///
    /// C'est la seule operation de l'allocateur qui parcourt. Elle ne sert
    /// qu'a la fusion -- retirer le jumeau qu'on vient de trouver libre -- et
    /// la liste d'un ordre reste courte parce que tout ce qui peut fusionner
    /// fusionne. Un allocateur qui ne fusionnerait pas aurait ici des listes
    /// interminables, ce qui est une raison de plus de fusionner.
    fn retire<T: Terrain>(&mut self, terrain: &mut T, bloc: usize, ordre: usize) -> bool {
        let mut precedent = AUCUN;
        let mut courant = self.tetes[ordre];
        let mut garde = 0usize;
        while courant != AUCUN {
            if courant == bloc {
                let suivant = terrain.lien(courant);
                if precedent == AUCUN {
                    self.tetes[ordre] = suivant;
                } else {
                    terrain.pose_lien(precedent, suivant);
                }
                self.marque(bloc, ordre, false);
                return true;
            }
            precedent = courant;
            courant = terrain.lien(courant);
            garde += 1;
            // Une liste circulaire -- memoire abimee, double liberation qui a
            // echappe au bitmap -- ferait tourner le noyau pour toujours. On
            // borne, et on echoue plutot que de figer.
            if garde > self.blocs {
                return false;
            }
        }
        false
    }

    /// Rend un bloc a l'allocateur, en le fusionnant tant que c'est possible.
    ///
    /// C'est aussi la facon d'ALIMENTER l'allocateur au demarrage : donner une
    /// region revient a rendre les blocs qui la composent.
    pub fn rend<T: Terrain>(&mut self, terrain: &mut T, bloc: usize, ordre: usize) -> bool {
        if ordre >= ORDRES || bloc >= self.blocs || !aligne(bloc, ordre) {
            return false;
        }
        if bloc + (1usize << ordre) > self.blocs {
            return false;
        }
        // Une double liberation corrompt les listes. Le bitmap la voit sans
        // parcourir quoi que ce soit.
        if self.est_libre(bloc, ordre) {
            return false;
        }

        // Ce que cette liberation AJOUTE, c'est la taille du bloc rendu -- pas
        // celle du bloc apres fusion. Les moities avec lesquelles il fusionne
        // ont deja ete comptees quand elles ont ete rendues ; les recompter
        // ferait grandir le total a chaque fusion, et l'allocateur croirait
        // avoir plusieurs fois la memoire qu'il a.
        let ajout = 1usize << ordre;

        let mut bloc = bloc;
        let mut ordre = ordre;
        while ordre + 1 < ORDRES {
            let compagnon = jumeau(bloc, ordre);
            // Le jumeau doit exister ENTIEREMENT dans la zone. Sur une zone
            // dont la taille n'est pas une puissance de deux, le jumeau du
            // dernier bloc deborde -- le fusionner rendrait un bloc dont la
            // seconde moitie n'est pas de la memoire.
            if compagnon >= self.blocs || compagnon + (1usize << ordre) > self.blocs {
                break;
            }
            if !self.est_libre(compagnon, ordre) {
                break;
            }
            if !self.retire(terrain, compagnon, ordre) {
                break;
            }
            bloc = pere(bloc, ordre);
            ordre += 1;
            self.fusions += 1;
        }
        self.empile(terrain, bloc, ordre);
        self.disponibles += ajout;
        self.liberations += 1;
        true
    }

    /// Prend un bloc d'ordre `ordre`, en divisant un plus gros si besoin.
    ///
    /// Rend le numero du bloc, ou `None`. La division est ce qui evite de
    /// servir une demande d'une page avec un bloc de quatre mebioctets.
    pub fn prend<T: Terrain>(&mut self, terrain: &mut T, ordre: usize) -> Option<usize> {
        if ordre >= ORDRES {
            self.echecs += 1;
            return None;
        }
        let mut source = ordre;
        while source < ORDRES && self.tetes[source] == AUCUN {
            source += 1;
        }
        if source >= ORDRES {
            self.echecs += 1;
            return None;
        }

        let bloc = self.tetes[source];
        let suivant = terrain.lien(bloc);
        self.tetes[source] = suivant;
        self.marque(bloc, source, false);

        // On descend en rendant a chaque etage la MOITIE HAUTE. Rendre la
        // moitie basse marcherait aussi, et rendrait des adresses moins
        // previsibles : garder la basse fait que le bloc rendu a l'appelant
        // est celui dont l'adresse est la plus alignee.
        while source > ordre {
            source -= 1;
            let haut = bloc + (1usize << source);
            self.empile(terrain, haut, source);
            self.divisions += 1;
        }

        self.disponibles -= 1usize << ordre;
        self.allocations += 1;
        Some(bloc)
    }

    /// Blocs elementaires actuellement libres.
    pub fn disponibles(&self) -> usize {
        self.disponibles
    }

    /// Blocs elementaires couverts.
    pub fn blocs(&self) -> usize {
        self.blocs
    }

    /// Combien de blocs libres a cet ordre.
    ///
    /// Parcourt : reserve au diagnostic et aux tests, jamais au chemin normal.
    pub fn compte_a_l_ordre<T: Terrain>(&self, terrain: &T, ordre: usize) -> usize {
        if ordre >= ORDRES {
            return 0;
        }
        let mut n = 0usize;
        let mut courant = self.tetes[ordre];
        while courant != AUCUN && n <= self.blocs {
            n += 1;
            courant = terrain.lien(courant);
        }
        n
    }

    /// Le plus grand ordre qui a au moins un bloc.
    ///
    /// C'est la mesure de la FRAGMENTATION : un allocateur qui a beaucoup de
    /// blocs libres et dont le plus grand ordre est zero ne peut plus rien
    /// servir de contigu.
    pub fn plus_grand_ordre(&self) -> Option<usize> {
        (0..ORDRES).rev().find(|ordre| self.tetes[*ordre] != AUCUN)
    }

    pub fn statistiques(&self) -> Statistiques {
        Statistiques {
            blocs: self.blocs,
            disponibles: self.disponibles,
            allocations: self.allocations,
            liberations: self.liberations,
            divisions: self.divisions,
            fusions: self.fusions,
            echecs: self.echecs,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Statistiques {
    pub blocs: usize,
    pub disponibles: usize,
    pub allocations: u64,
    pub liberations: u64,
    pub divisions: u64,
    pub fusions: u64,
    pub echecs: u64,
}
