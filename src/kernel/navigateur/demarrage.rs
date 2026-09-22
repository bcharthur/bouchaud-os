//! LE TEMPS DE DEMARRAGE DU NAVIGATEUR, DECOMPOSE.
//!
//! # La question a laquelle ce fichier repond
//!
//! « Ladybird met combien de temps a s'ouvrir, et ou passe ce temps ? »
//!
//! Le noyau savait deja repondre a la premiere moitie : `PERF_BROWSER_CLICK`
//! et `PERF_FIRST_PAINT` encadrent le demarrage, et leur difference est le
//! total. Mais un total ne se corrige pas. « Dix secondes » et « dix secondes
//! dont neuf a attendre le RequestServer » demandent le meme travail a lire et
//! un travail completement different a reparer.
//!
//! Le portage lance six processus. Entre le clic et la premiere trame, il y a
//! donc six attentes qui se suivent, et l'une d'elles domine presque toujours.
//! C'est celle-la qu'il faut nommer.
//!
//! # Ce que ce fichier ne fait pas
//!
//! Il ne mesure rien. Il RANGE des instants que d'autres lui donnent -- la
//! supervision quand elle enregistre un lancement, le profil quand il voit la
//! premiere trame -- et il en tire des ecarts. C'est ce qui lui permet de ne
//! dependre de rien et d'etre verifie sur l'hote par
//! `tools/navigateur/test_demarrage.rs`, alors que le demarrage lui-meme ne
//! s'observe qu'au bout d'une construction complete de Ladybird.
//!
//! # Les deux pieges qu'il evite, et pourquoi ils meritent du code
//!
//! **Un jalon manquant ne vaut pas zero.** Si le Compositor n'a jamais
//! demarre, son ecart n'est pas « 0 ms » : il n'existe pas. Une colonne a zero
//! se lit « cette etape est gratuite » et detourne l'optimisation vers les
//! autres, alors que la bonne lecture est « ce processus n'est jamais venu ».
//!
//! **Le premier instant l'emporte.** Le portage lance un WebContent par
//! onglet. Le deuxieme ne doit pas repousser le jalon « rendu » : la question
//! porte sur le DEMARRAGE, et le demarrage s'arrete a la premiere trame.

/// Les etapes du chemin, dans l'ordre ou elles sont franchies.
///
/// Elles ne sont pas choisies : chacune correspond a un evenement que le
/// noyau produit deja -- un clic du bureau, un `note_lancement` de la
/// supervision, une premiere trame du profil. Une etape de plus ici sans
/// evenement correspondant serait une colonne vide pour toujours.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Jalon {
    /// Le bureau a recu le double-clic.
    Clic,
    /// Le courtier (BouchaudBrowserHost) est lance.
    Courtier,
    /// RequestServer est lance.
    Reseau,
    /// ImageDecoder est lance.
    Decodeur,
    /// Compositor est lance.
    Composition,
    /// Le premier WebContent est lance.
    Rendu,
    /// La premiere trame est presentee a l'ecran.
    PremiereTrame,
}

pub const JALONS: usize = 7;

impl Jalon {
    pub fn rang(self) -> usize {
        match self {
            Jalon::Clic => 0,
            Jalon::Courtier => 1,
            Jalon::Reseau => 2,
            Jalon::Decodeur => 3,
            Jalon::Composition => 4,
            Jalon::Rendu => 5,
            Jalon::PremiereTrame => 6,
        }
    }

    pub fn depuis_rang(rang: usize) -> Option<Jalon> {
        match rang {
            0 => Some(Jalon::Clic),
            1 => Some(Jalon::Courtier),
            2 => Some(Jalon::Reseau),
            3 => Some(Jalon::Decodeur),
            4 => Some(Jalon::Composition),
            5 => Some(Jalon::Rendu),
            6 => Some(Jalon::PremiereTrame),
            _ => None,
        }
    }

    pub fn nom(self) -> &'static str {
        match self {
            Jalon::Clic => "clic",
            Jalon::Courtier => "courtier",
            Jalon::Reseau => "reseau",
            Jalon::Decodeur => "decodeur",
            Jalon::Composition => "composition",
            Jalon::Rendu => "rendu",
            Jalon::PremiereTrame => "premiere_trame",
        }
    }
}

#[derive(Clone, Copy)]
pub struct Profil {
    instants: [u64; JALONS],
    vus: [bool; JALONS],
}

impl Default for Profil {
    fn default() -> Self {
        Profil::neuf()
    }
}

impl Profil {
    pub const fn neuf() -> Profil {
        Profil { instants: [0; JALONS], vus: [false; JALONS] }
    }

    /// Enregistre l'instant d'un jalon. LE PREMIER L'EMPORTE.
    ///
    /// Rend `true` si le jalon vient d'etre pose, `false` s'il l'etait deja.
    /// Le portage lance un WebContent par onglet : le deuxieme ne doit pas
    /// repousser le jalon « rendu », puisque la question porte sur le
    /// demarrage et que le demarrage s'arrete a la premiere trame.
    pub fn note(&mut self, jalon: Jalon, t_ns: u64) -> bool {
        let rang = jalon.rang();
        if self.vus[rang] {
            return false;
        }
        self.vus[rang] = true;
        self.instants[rang] = t_ns;
        true
    }

    pub fn vu(&self, jalon: Jalon) -> bool {
        self.vus[jalon.rang()]
    }

    pub fn instant(&self, jalon: Jalon) -> Option<u64> {
        if self.vus[jalon.rang()] {
            Some(self.instants[jalon.rang()])
        } else {
            None
        }
    }

    /// Le rang du dernier jalon connu AVANT celui-ci, s'il y en a un.
    ///
    /// C'est la fonction qui rend les ecarts robustes a un jalon manquant :
    /// si le Compositor n'a jamais demarre, l'ecart du rendu se mesure depuis
    /// le decodeur, et non depuis un instant qui n'existe pas.
    fn precedent(&self, rang: usize) -> Option<usize> {
        (0..rang).rev().find(|&avant| self.vus[avant])
    }

    /// Le temps ecoule entre le jalon precedent connu et celui-ci.
    ///
    /// `None` quand le jalon n'a pas ete vu, ou qu'aucun jalon ne le precede.
    /// JAMAIS zero pour dire « non mesure » : une colonne a zero se lit
    /// « cette etape est gratuite ».
    pub fn segment_ns(&self, jalon: Jalon) -> Option<u64> {
        let rang = jalon.rang();
        if !self.vus[rang] {
            return None;
        }
        let avant = self.precedent(rang)?;
        // `saturating_sub` : entre deux coeurs, l'horloge peut reculer de
        // quelques nanosecondes. Un ecart negatif deviendrait un nombre
        // gigantesque en arithmetique non signee, et ce nombre-la designerait
        // l'etape a optimiser.
        Some(self.instants[rang].saturating_sub(self.instants[avant]))
    }

    /// Du clic a la premiere trame.
    pub fn total_ns(&self) -> Option<u64> {
        let debut = self.instant(Jalon::Clic)?;
        let fin = self.instant(Jalon::PremiereTrame)?;
        Some(fin.saturating_sub(debut))
    }

    /// L'etape la plus longue, et sa duree.
    ///
    /// C'est la seule reponse qui serve a quelque chose : c'est elle qu'on
    /// optimise en premier.
    pub fn plus_long(&self) -> Option<(Jalon, u64)> {
        let mut pire: Option<(Jalon, u64)> = None;
        for rang in 1..JALONS {
            let Some(jalon) = Jalon::depuis_rang(rang) else { continue };
            let Some(duree) = self.segment_ns(jalon) else { continue };
            match pire {
                Some((_, record)) if record >= duree => {}
                _ => pire = Some((jalon, duree)),
            }
        }
        pire
    }

    /// Le demarrage est-il alle jusqu'au bout ?
    pub fn abouti(&self) -> bool {
        self.vu(Jalon::Clic) && self.vu(Jalon::PremiereTrame)
    }

    /// Les jalons qu'on n'a jamais vus.
    ///
    /// Ecrit dans `sortie` et rend combien. Un demarrage qui aboutit sans
    /// avoir vu le Compositor n'est pas le meme demarrage qu'un autre, et le
    /// dire vaut mieux que de rendre un tableau avec des trous muets.
    pub fn manquants(&self, sortie: &mut [Jalon]) -> usize {
        let mut ecrits = 0usize;
        for rang in 0..JALONS {
            if self.vus[rang] || ecrits >= sortie.len() {
                continue;
            }
            if let Some(jalon) = Jalon::depuis_rang(rang) {
                sortie[ecrits] = jalon;
                ecrits += 1;
            }
        }
        ecrits
    }

    /// Un nouveau demarrage efface le precedent.
    pub fn reinitialise(&mut self) {
        *self = Profil::neuf();
    }
}
