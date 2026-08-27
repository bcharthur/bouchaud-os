//! La chaine `entree -> degat -> trame -> present -> pixels dans le LFB`.
//!
//! # Le probleme que ce module resout
//!
//! Un bureau fige se diagnostique mal parce que tous les symptomes se
//! ressemblent. « L'ecran ne bouge plus » peut vouloir dire :
//!
//!   * le noyau est mort — plus aucun compteur ne bouge ;
//!   * le fil du bureau est bloque — son battement s'arrete, le reste tourne ;
//!   * les IRQ d'entree n'arrivent plus — la boucle tourne, sans rien a faire ;
//!   * la boucle tourne et ne compose pas — les degats ne sont pas crees ;
//!   * elle compose et ne presente pas ;
//!   * elle presente et AUCUN PIXEL n'atteint le framebuffer lineaire.
//!
//! Le dernier cas est le plus trompeur : `frames_composed`, `presents` et
//! `presented_pixels` montent tous, la trace respire, et l'ecran ne change pas.
//! `present_rect` a cinq sorties anticipees silencieuses — affichage cede a un
//! programme ring 3, backbuffer absent, LFB non mappe, rectangle vide.
//!
//! Ce module ne mesure rien lui-meme : il prend un instantane des compteurs et
//! repond a UNE question — quel est le premier maillon qui n'a pas avance.
//! C'est de la logique pure, testee sur l'hote.
//!
//! # Ne pas inonder la trace
//!
//! Un bureau recoit des dizaines d'evenements souris par seconde. Un
//! diagnostic par evenement rend la trace illisible, donc inutile. Le veilleur
//! n'arme qu'une surveillance a la fois, ne parle qu'apres un delai, et une
//! seule fois par episode et par maillon — puis une fois encore quand la
//! chaine se retablit.

/// Instantane des compteurs de la chaine, du premier maillon au dernier.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Chaine {
    /// Evenements d'entree recus par le bureau (souris, clavier).
    pub entrees: u64,
    /// Degats crees, toutes origines confondues.
    pub degats: u64,
    /// Trames composees.
    pub trames: u64,
    /// Appels a `present_rect`, aboutis ou non.
    pub presents: u64,
    /// Presentations ayant reellement ECRIT dans le framebuffer lineaire.
    pub copies: u64,
}

/// Un maillon de la chaine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Maillon {
    Entree,
    Degat,
    Trame,
    Present,
    Copie,
}

impl Maillon {
    /// Nom du maillon, tel qu'il apparait dans la trace.
    pub fn nom(self) -> &'static str {
        match self {
            Maillon::Entree => "input_received",
            Maillon::Degat => "damage_created",
            Maillon::Trame => "frame_composed",
            Maillon::Present => "present_called",
            Maillon::Copie => "lfb_pixels_copied",
        }
    }

    /// Ce qu'il faut regarder quand ce maillon casse.
    pub fn piste(self) -> &'static str {
        match self {
            Maillon::Entree => "IRQ souris/clavier, ou le bureau ne les pompe plus",
            Maillon::Degat => "la boucle tourne mais rien ne se declare sale",
            Maillon::Trame => "degats crees, jamais composes (cadence ou sortie anticipee)",
            Maillon::Present => "trame composee, present_rect jamais appelee",
            Maillon::Copie => "present_rect appelee et refusee : \
                               ecran cede au userland, backbuffer absent, \
                               LFB non mappe, ou rectangle vide",
        }
    }
}

/// Premier maillon qui n'a pas avance entre `reference` et `courant`.
///
/// `None` : la chaine est complete, des pixels ont atteint l'ecran.
///
/// L'ordre compte. Un maillon casse rend tous les suivants immobiles ; ne
/// signaler que le PREMIER evite un diagnostic qui accuse cinq coupables pour
/// une seule cause.
pub fn maillon_rompu(reference: &Chaine, courant: &Chaine) -> Option<Maillon> {
    if courant.entrees <= reference.entrees {
        return Some(Maillon::Entree);
    }
    if courant.degats <= reference.degats {
        return Some(Maillon::Degat);
    }
    if courant.trames <= reference.trames {
        return Some(Maillon::Trame);
    }
    if courant.presents <= reference.presents {
        return Some(Maillon::Present);
    }
    if courant.copies <= reference.copies {
        return Some(Maillon::Copie);
    }
    None
}

/// Ce que le veilleur a a dire. `Rien` la plupart du temps, et c'est le but.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    Rien,
    /// La chaine est cassee a ce maillon. Signale UNE fois par episode.
    Rupture(Maillon),
    /// Elle s'est retablie apres une rupture signalee. Signale une fois.
    Retabli(Maillon),
}

/// Surveille la chaine apres une entree, sans inonder la trace.
///
/// Une seule surveillance a la fois : une entree qui arrive alors que la
/// precedente est encore en cours ne repousse pas l'echeance. C'est
/// volontaire — sinon un mouvement de souris continu repousserait la mesure
/// indefiniment, et le seul cas ou l'on veut vraiment savoir serait le seul
/// qu'on ne verrait jamais.
pub struct Veilleur {
    reference: Chaine,
    arme_ms: u64,
    arme: bool,
    signale: Option<Maillon>,
}

impl Veilleur {
    pub const fn neuf() -> Self {
        Self {
            reference: Chaine { entrees: 0, degats: 0, trames: 0, presents: 0, copies: 0 },
            arme_ms: 0,
            arme: false,
            signale: None,
        }
    }

    pub fn arme(&self) -> bool {
        self.arme
    }

    /// Une entree vient d'arriver.
    pub fn note_entree(&mut self, maintenant_ms: u64, chaine: Chaine) {
        if self.arme {
            return;
        }
        self.reference = chaine;
        self.arme_ms = maintenant_ms;
        self.arme = true;
    }

    /// A appeler a chaque tour de boucle. Rend au plus un verdict par episode.
    pub fn examine(&mut self, maintenant_ms: u64, chaine: Chaine, delai_ms: u64) -> Verdict {
        if !self.arme {
            return Verdict::Rien;
        }
        let rompu = maillon_rompu(&self.reference, &chaine);

        if rompu.is_none() {
            // La chaine est allee jusqu'a l'ecran : la surveillance est finie.
            self.arme = false;
            return match self.signale.take() {
                Some(maillon) => Verdict::Retabli(maillon),
                None => Verdict::Rien,
            };
        }

        if maintenant_ms.wrapping_sub(self.arme_ms) < delai_ms {
            return Verdict::Rien; // pas encore le moment de s'inquieter
        }

        let maillon = rompu.expect("rompu est Some ici");
        if self.signale == Some(maillon) {
            return Verdict::Rien; // deja dit, on ne le repete pas
        }
        self.signale = Some(maillon);
        Verdict::Rupture(maillon)
    }
}
