// Le cycle de vie d'une commande NVM Express, sans un seul acces materiel.
//
// Ce module ne lit ni n'ecrit aucun registre et n'alloue rien. C'est
// delibere : la partie du pilote qui se trompe n'est pas celle qui touche le
// BAR0 -- une sonnette mal calculee se voit du premier coup --, c'est celle
// qui decide A QUI appartient un achevement, et QUAND un tampon redevient
// libre. Ces deux questions n'ont pas de reponse observable au banc : elles
// ne se manifestent qu'en corruption silencieuse, des mois plus tard, sur une
// machine qu'on n'a pas sous la main.
//
// Donc on les sort du pilote, on les rend deterministes, et on leur injecte
// ici les scenarios que le materiel ne produira qu'une fois : achevement
// tardif apres echeance, achevement portant un identifiant deja recolte,
// achevement dans le desordre, bouclage de file avec inversion de phase.
//
// LA REGLE CENTRALE, dont tout le reste decoule :
//
// > Une echeance depassee ne LIBERE PAS une commande. Elle la met en
// > QUARANTAINE.
//
// Un pilote qui rend l'identifiant au pot commun des qu'il cesse d'attendre
// ment sur la propriete du tampon. Le controleur, lui, n'a rien annule : il
// peut ecrire dans la zone DMA de cette commande a n'importe quel moment
// ulterieur. Rendre l'identifiant, c'est autoriser une commande future a
// prendre le meme numero, a recevoir l'achevement de l'ancienne, et a rendre
// a son appelant un tampon que personne n'a rempli. La quarantaine ne se leve
// que sur l'un des deux evenements qui prouvent que le controleur en a fini :
// l'achevement tardif arrive enfin, ou le controleur est reinitialise.

/// Identifiants d'entree-sortie suivis. Zero reste hors du pot.
pub const CID_MAX: usize = 256;

/// Ou en est un identifiant de commande.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EtatCid {
    /// Disponible pour une nouvelle commande.
    Libre,
    /// Soumis au controleur, achevement attendu.
    EnVol,
    /// Achevement recu, pas encore recolte par l'emetteur.
    Acheve,
    /// Echeance depassee. Le controleur peut ENCORE ecrire dans le tampon de
    /// cette commande : l'identifiant ne retourne pas au pot.
    Quarantaine,
}

/// Ce qu'on fait d'un achevement qui arrive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Il appartient a une commande en vol : range pour son emetteur.
    Attendu,
    /// Il appartient a une commande abandonnee sur echeance. On l'avale, et la
    /// quarantaine se leve : le controleur vient de prouver qu'il en a fini
    /// avec le tampon.
    Tardif,
    /// Identifiant jamais emis, ou deja recolte. Rejete.
    Inconnu,
    /// Identifiant emis avant une reinitialisation du controleur. Rejete.
    Perime,
    /// Deuxieme achevement pour un identifiant qui en a deja un. Rejete.
    Double,
    /// Identifiant hors du domaine suivi. Rejete.
    HorsDomaine,
}

/// Le suivi des commandes d'entree-sortie en vol.
pub struct Suivi {
    etat: [EtatCid; CID_MAX],
    /// Epoque a laquelle l'identifiant a ete emis pour la derniere fois.
    epoque_cid: [u16; CID_MAX],
    /// Epoque courante. Elle avance a chaque reinitialisation du controleur.
    epoque: u16,
    /// Statut range, en attente de recolte : bits 7:0 le code, 10:8 le type.
    statut: [u16; CID_MAX],
    /// Ou reprendre la recherche d'un identifiant libre.
    curseur: u16,
    en_vol: u16,
    quarantaine: u16,
    pub rejets_inconnu: u32,
    pub rejets_perime: u32,
    pub rejets_double: u32,
    pub rejets_hors_domaine: u32,
    pub tardifs: u32,
    pub echeances: u32,
}

impl Suivi {
    pub const fn neuf() -> Self {
        Self {
            etat: [EtatCid::Libre; CID_MAX],
            epoque_cid: [0; CID_MAX],
            epoque: 1,
            statut: [0; CID_MAX],
            curseur: 1,
            en_vol: 0,
            quarantaine: 0,
            rejets_inconnu: 0,
            rejets_perime: 0,
            rejets_double: 0,
            rejets_hors_domaine: 0,
            tardifs: 0,
            echeances: 0,
        }
    }

    pub fn en_vol(&self) -> u16 {
        self.en_vol
    }

    pub fn quarantaine(&self) -> u16 {
        self.quarantaine
    }

    /// Le tampon de rebond peut-il etre confie a une nouvelle commande ?
    ///
    /// Non tant qu'une commande abandonnee sur echeance existe : du point de
    /// vue du controleur elle est toujours la, et il peut ecrire dans ce
    /// tampon a n'importe quel moment. C'est la question que le pilote doit
    /// poser AVANT de servir une entree-sortie, et la reponse ne depend
    /// d'aucun etat qui vive hors d'ici.
    pub fn tampon_disponible(&self) -> bool {
        self.quarantaine == 0
    }

    pub fn epoque(&self) -> u16 {
        self.epoque
    }

    pub fn etat_de(&self, cid: u16) -> Option<EtatCid> {
        self.etat.get(cid as usize).copied()
    }

    /// Prend un identifiant libre, ou rend `None`.
    ///
    /// La recherche part du curseur et fait au plus un tour. Elle SAUTE les
    /// identifiants en quarantaine : c'est tout l'interet de l'etat. Rendre
    /// `None` quand il n'en reste plus est le comportement correct -- mieux
    /// vaut refuser une entree-sortie que d'en corrompre une autre.
    pub fn alloue(&mut self) -> Option<u16> {
        let domaine = CID_MAX as u16;
        for pas in 0..(domaine - 1) {
            let cid = ((self.curseur - 1 + pas) % (domaine - 1)) + 1;
            if self.etat[cid as usize] == EtatCid::Libre {
                self.curseur = (cid % (domaine - 1)) + 1;
                self.etat[cid as usize] = EtatCid::EnVol;
                self.epoque_cid[cid as usize] = self.epoque;
                self.statut[cid as usize] = 0;
                self.en_vol += 1;
                return Some(cid);
            }
        }
        None
    }

    /// Range un achevement qui arrive du controleur.
    pub fn range(&mut self, cid: u16, type_statut: u8, code_statut: u8) -> Verdict {
        if cid == 0 || (cid as usize) >= CID_MAX {
            // Aucun repli modulo ici. Plier un identifiant hors domaine sur le
            // domaine suivi, c'est fabriquer de toutes pieces un achevement
            // pour une commande qui n'a rien demande.
            self.rejets_hors_domaine += 1;
            return Verdict::HorsDomaine;
        }
        let index = cid as usize;
        match self.etat[index] {
            EtatCid::EnVol => {
                self.statut[index] = (code_statut as u16) | ((type_statut as u16 & 0x7) << 8);
                self.etat[index] = EtatCid::Acheve;
                self.en_vol -= 1;
                Verdict::Attendu
            }
            EtatCid::Quarantaine => {
                // Le controleur vient de prouver qu'il en a fini avec le
                // tampon de cette commande. La quarantaine se leve ICI, et
                // nulle part ailleurs.
                self.etat[index] = EtatCid::Libre;
                self.quarantaine -= 1;
                self.tardifs += 1;
                Verdict::Tardif
            }
            EtatCid::Acheve => {
                self.rejets_double += 1;
                Verdict::Double
            }
            EtatCid::Libre => {
                if self.epoque_cid[index] != 0 && self.epoque_cid[index] != self.epoque {
                    self.rejets_perime += 1;
                    Verdict::Perime
                } else {
                    self.rejets_inconnu += 1;
                    Verdict::Inconnu
                }
            }
        }
    }

    /// L'emetteur recolte son achevement et rend l'identifiant au pot.
    pub fn recolte(&mut self, cid: u16) -> Option<(u8, u8)> {
        if cid == 0 || (cid as usize) >= CID_MAX {
            return None;
        }
        let index = cid as usize;
        if self.etat[index] != EtatCid::Acheve {
            return None;
        }
        let mot = self.statut[index];
        self.etat[index] = EtatCid::Libre;
        self.statut[index] = 0;
        Some((((mot >> 8) & 0x7) as u8, (mot & 0xFF) as u8))
    }

    /// L'emetteur abandonne l'attente : l'identifiant passe en quarantaine.
    ///
    /// Rend `false` si la commande s'etait achevee entre-temps -- la course
    /// existe, et la traiter comme une echeance perdrait un achevement valide.
    pub fn expire(&mut self, cid: u16) -> bool {
        if cid == 0 || (cid as usize) >= CID_MAX {
            return false;
        }
        let index = cid as usize;
        if self.etat[index] != EtatCid::EnVol {
            return false;
        }
        self.etat[index] = EtatCid::Quarantaine;
        self.en_vol -= 1;
        self.quarantaine += 1;
        self.echeances += 1;
        true
    }

    /// Le controleur a ete reinitialise : plus rien n'est en vol.
    ///
    /// C'est le SEUL autre evenement qui leve une quarantaine. Un controleur
    /// remis a zero a perdu ses files ; il ne peut plus ecrire dans les
    /// tampons des commandes d'avant.
    pub fn reinitialise(&mut self) {
        for index in 0..CID_MAX {
            self.etat[index] = EtatCid::Libre;
            self.statut[index] = 0;
        }
        self.en_vol = 0;
        self.quarantaine = 0;
        self.epoque = self.epoque.wrapping_add(1).max(1);
        self.curseur = 1;
    }
}

// ---------------------------------------------------------------------------
// L'arithmetique des anneaux
// ---------------------------------------------------------------------------

/// La file d'achevement, reduite a son index et sa phase.
///
/// La phase est le seul discriminant valide entre une entree neuve et une
/// entree du tour precedent : le controleur ne remet pas la zone a zero. Une
/// implementation qui comparerait un contenu « vide » marcherait au premier
/// tour et se tromperait a tous les suivants.
#[derive(Clone, Copy, Debug)]
pub struct AnneauAchevement {
    pub entrees: u32,
    pub tete: u32,
    pub phase: bool,
}

impl AnneauAchevement {
    pub const fn neuf(entrees: u32) -> Self {
        // Le controleur commence par ecrire des entrees de phase 1 dans une
        // zone mise a zero : la phase attendue au premier tour est donc VRAIE.
        Self { entrees, tete: 0, phase: true }
    }

    /// L'entree lue a la tete est-elle a nous ?
    pub fn est_neuve(&self, phase_lue: bool) -> bool {
        phase_lue == self.phase
    }

    /// Avance d'une entree, en basculant la phase au bouclage.
    pub fn avance(&mut self) {
        self.tete += 1;
        if self.tete >= self.entrees {
            self.tete = 0;
            self.phase = !self.phase;
        }
    }
}

/// La file de soumission, reduite a ses deux index.
///
/// `queue` est ce qu'on publie a la sonnette ; `tete` est ce que le controleur
/// nous rend dans chaque achevement. Sans la seconde, on ne sait pas combien
/// de places restent, et une profondeur superieure a un finit par ecraser une
/// commande que le controleur n'a pas encore lue.
#[derive(Clone, Copy, Debug)]
pub struct AnneauSoumission {
    pub entrees: u32,
    pub queue: u32,
    pub tete: u32,
}

impl AnneauSoumission {
    pub const fn neuf(entrees: u32) -> Self {
        Self { entrees, queue: 0, tete: 0 }
    }

    /// Places libres. Une entree reste toujours vide : sinon queue == tete
    /// voudrait dire « pleine » et « vide » a la fois.
    pub fn places(&self) -> u32 {
        if self.entrees == 0 {
            return 0;
        }
        (self.tete + self.entrees - self.queue - 1) % self.entrees
    }

    /// Reserve une entree et rend son index, ou `None` si la file est pleine.
    pub fn pose(&mut self) -> Option<u32> {
        if self.places() == 0 {
            return None;
        }
        let index = self.queue;
        self.queue = (self.queue + 1) % self.entrees;
        Some(index)
    }

    /// Le controleur publie sa tete dans chaque achevement.
    pub fn tete_vue(&mut self, tete: u16) {
        let tete = tete as u32;
        if tete < self.entrees {
            self.tete = tete;
        }
    }
}
