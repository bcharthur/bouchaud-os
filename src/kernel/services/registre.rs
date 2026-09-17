//! Le registre des services et des phases de demarrage, PUR.
//!
//! # Pourquoi ce module existe, et pourquoi il est la SEULE source
//!
//! Le releve physique du 17 septembre pose une question a laquelle rien ne
//! repond : le premier echantillon encore lisible, a t~287 s, porte deja
//!
//! ```text
//! hid_wake_to_run_max_us = 12 179 860
//! hid_poll_body_max_us   = 170
//! hid_run_to_lock_max_us = 2
//! hid_responsable        = ordonnanceur
//! ```
//!
//! Douze secondes d'attente entre un reveil et son election, AVANT t=287 s.
//! Ni la scrutation USB, ni le verrou : l'election. Et l'archive ne permet pas
//! de dire quand, ni pendant quelle phase, parce que le debut a ete efface.
//!
//! Un maximum cumule ne dit jamais QUAND. Il faut donc deux choses que ce
//! module porte ensemble : des PHASES datees, et des EVENEMENTS ponctuels.
//!
//! # Une seule source de verite
//!
//! Le meme registre sert l'ecran de demarrage, le futur observatoire des
//! services et la boite noire. Un compteur dans `netetat`, un autre dans une
//! interface, un troisieme dans l'archive et un quatrieme dans l'ecran de boot
//! divergent le jour ou ils comptent -- et c'est toujours ce jour-la qu'on les
//! lit.
//!
//! # Ce module est PUR
//!
//! Aucun `use crate::`, aucun `unsafe`, aucune allocation, aucune horloge : le
//! temps est fourni par l'appelant. Un test hote met la discipline a l'epreuve
//! sans demarrer la machine.

/// Nombre de services que le registre peut porter.
///
/// Borne fixe : un registre qui alloue dans un noyau, c'est une panne memoire
/// en sursis. Trente-deux couvre la pile actuelle avec de la marge, et le
/// depassement se COMPTE au lieu de se perdre.
pub const SERVICES_MAX: usize = 32;

/// Longueur maximale d'un identifiant de service (`net.rtl8168`).
pub const ID_MAX: usize = 24;

/// L'etat normalise d'un service. L'ordre va du plus sain au plus grave.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[repr(u8)]
pub enum Etat {
    /// Jamais demarre.
    Inconnu = 0,
    /// En cours de demarrage.
    Demarrage = 1,
    /// Pret et actif.
    Actif = 2,
    /// Pret, sans travail en cours.
    Repos = 3,
    /// Pret, mais en attente de quelque chose d'autre.
    Attente = 4,
    /// Fonctionne mal : des erreurs, mais il rend encore un service.
    Degrade = 5,
    /// Une reprise est en cours.
    Reprise = 6,
    /// Arrete proprement.
    Arrete = 7,
    /// En panne.
    Panne = 8,
}

impl Etat {
    pub fn nom(self) -> &'static str {
        match self {
            Etat::Inconnu => "inconnu",
            Etat::Demarrage => "demarrage",
            Etat::Actif => "actif",
            Etat::Repos => "repos",
            Etat::Attente => "attente",
            Etat::Degrade => "degrade",
            Etat::Reprise => "reprise",
            Etat::Arrete => "arrete",
            Etat::Panne => "panne",
        }
    }

    /// Cet etat est-il un etat SAIN et courant ?
    ///
    /// Actif, au repos, en attente : un service qui va bien passe son temps a
    /// osciller entre les trois -- un resolveur DNS attend, repond, attend --
    /// et ces basculements-la ne valent aucune ligne d'archive.
    pub fn sain(self) -> bool {
        matches!(self, Etat::Actif | Etat::Repos | Etat::Attente)
    }

    /// Cet etat demande-t-il une attention ?
    ///
    /// C'est ce qu'une barre d'etat doit montrer, et ce qu'un verdict doit
    /// retenir. Un service arrete proprement n'est pas une alarme.
    pub fn problematique(self) -> bool {
        matches!(self, Etat::Degrade | Etat::Reprise | Etat::Panne)
    }
}

/// Ce qu'un element du registre EST.
///
/// L'arbre represente la RESPONSABILITE, pas forcement un fil d'execution :
/// `DNS` est un protocole, `RequestServer` est un processus, et les deux ont
/// leur place. Fabriquer un travailleur par ligne d'arborescence serait une
/// fiction couteuse.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Genre {
    /// Une brique du noyau.
    Noyau = 0,
    /// Un pilote de peripherique.
    Pilote = 1,
    /// Un protocole : ARP, DNS, TCP, TLS. Observable, sans fil propre.
    Protocole = 2,
    /// Un service reel : un fil, un travailleur.
    Service = 3,
    /// Un processus utilisateur.
    Processus = 4,
    /// Une etape de chaine de traitement : layout, paint, present.
    Etape = 5,
    /// Un regroupement, sans etat propre.
    Groupe = 6,
}

impl Genre {
    pub fn nom(self) -> &'static str {
        match self {
            Genre::Noyau => "noyau",
            Genre::Pilote => "pilote",
            Genre::Protocole => "protocole",
            Genre::Service => "service",
            Genre::Processus => "processus",
            Genre::Etape => "etape",
            Genre::Groupe => "groupe",
        }
    }
}

/// Un identifiant court, sans allocation.
#[derive(Clone, Copy)]
pub struct Id {
    octets: [u8; ID_MAX],
    longueur: usize,
}

impl Id {
    pub const fn vide() -> Self {
        Self { octets: [0; ID_MAX], longueur: 0 }
    }

    pub fn depuis(texte: &str) -> Self {
        let mut id = Self::vide();
        for (place, octet) in texte.as_bytes().iter().take(ID_MAX).enumerate() {
            id.octets[place] = *octet;
            id.longueur = place + 1;
        }
        id
    }

    pub fn texte(&self) -> &str {
        // Les identifiants sont des litteraux ASCII du noyau ; la troncature
        // ne peut pas couper un caractere multi-octet. La verification reste,
        // parce qu'un `from_utf8_unchecked` ici ne gagnerait rien de mesurable.
        core::str::from_utf8(&self.octets[..self.longueur]).unwrap_or("?")
    }

    pub fn est_vide(&self) -> bool {
        self.longueur == 0
    }

    pub fn egale(&self, texte: &str) -> bool {
        self.texte() == texte
    }
}

/// Une entree du registre.
#[derive(Clone, Copy)]
pub struct Entree {
    pub id: Id,
    pub parent: Id,
    pub genre: Genre,
    pub etat: Etat,
    /// Instant du premier passage a `Demarrage`.
    pub debut_ns: u64,
    /// Instant du premier passage a un etat pret.
    pub pret_ns: u64,
    pub derniere_activite_ns: u64,
    pub dernier_succes_ns: u64,
    pub derniere_erreur_ns: u64,
    pub redemarrages: u32,
    pub reprises: u32,
    pub erreurs: u32,
    /// Derniere raison connue d'un etat degrade ou en panne.
    pub raison: Id,
}

impl Entree {
    const fn vide() -> Self {
        Self {
            id: Id::vide(),
            parent: Id::vide(),
            genre: Genre::Groupe,
            etat: Etat::Inconnu,
            debut_ns: 0,
            pret_ns: 0,
            derniere_activite_ns: 0,
            dernier_succes_ns: 0,
            derniere_erreur_ns: 0,
            redemarrages: 0,
            reprises: 0,
            erreurs: 0,
            raison: Id::vide(),
        }
    }

    /// Temps ecoule entre le debut et l'etat pret, en millisecondes.
    ///
    /// C'est le chiffre de l'ecran de demarrage : « USB pret 210 ms ». Zero
    /// tant que le service n'est pas pret -- et non une duree fabriquee.
    pub fn duree_demarrage_ms(&self) -> Option<u64> {
        if self.pret_ns == 0 || self.debut_ns == 0 || self.pret_ns < self.debut_ns {
            return None;
        }
        Some((self.pret_ns - self.debut_ns) / 1_000_000)
    }
}

/// Ce que le registre a refuse ou fait.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Compteurs {
    pub enregistres: usize,
    /// Enregistrements refuses faute de place. Une borne qui se tait est une
    /// borne qu'on decouvre trop tard.
    pub refuses: u64,
    pub transitions: u64,
    /// Transitions dignes d'un evenement immediat.
    pub evenements: u64,
}

/// Le registre : une table bornee, sans allocation.
pub struct Registre {
    entrees: [Entree; SERVICES_MAX],
    occupees: usize,
    compteurs: Compteurs,
}

impl Registre {
    pub const fn neuf() -> Self {
        Self {
            entrees: [Entree::vide(); SERVICES_MAX],
            occupees: 0,
            compteurs: Compteurs {
                enregistres: 0,
                refuses: 0,
                transitions: 0,
                evenements: 0,
            },
        }
    }

    fn place(&self, id: &str) -> Option<usize> {
        self.entrees[..self.occupees]
            .iter()
            .position(|e| e.id.egale(id))
    }

    /// Declare un service. Re-declarer le meme identifiant ne le duplique pas.
    ///
    /// Rend `false` quand la table est pleine -- et le COMPTE : un registre
    /// silencieusement tronque ferait disparaitre de l'arbre le service qu'on
    /// cherche, sans rien dire.
    pub fn declare(&mut self, id: &str, parent: &str, genre: Genre) -> bool {
        if let Some(place) = self.place(id) {
            self.entrees[place].parent = Id::depuis(parent);
            self.entrees[place].genre = genre;
            return true;
        }
        if self.occupees >= SERVICES_MAX {
            self.compteurs.refuses = self.compteurs.refuses.saturating_add(1);
            return false;
        }
        let place = self.occupees;
        self.entrees[place] = Entree {
            id: Id::depuis(id),
            parent: Id::depuis(parent),
            genre,
            ..Entree::vide()
        };
        self.occupees += 1;
        self.compteurs.enregistres = self.occupees;
        true
    }

    /// Change l'etat d'un service. Rend `true` si un EVENEMENT doit etre emis.
    ///
    /// # Ce qui merite d'etre ecrit, et ce qui ne le merite pas
    ///
    /// Un evenement est emis quand l'etat CHANGE, et seulement alors. Un
    /// service qui reste actif sans erreur pendant cinq minutes n'ecrit rien :
    /// c'est ce qui evite que la boite noire efface le demarrage pour decrire
    /// un systeme qui va bien.
    pub fn etat(&mut self, id: &str, etat: Etat, maintenant_ns: u64) -> bool {
        let Some(place) = self.place(id) else { return false };
        let entree = &mut self.entrees[place];
        let ancien = entree.etat;
        entree.derniere_activite_ns = maintenant_ns;
        if ancien == etat {
            return false;
        }
        self.compteurs.transitions = self.compteurs.transitions.saturating_add(1);

        if etat == Etat::Demarrage {
            if entree.debut_ns == 0 {
                entree.debut_ns = maintenant_ns;
            } else {
                entree.redemarrages = entree.redemarrages.saturating_add(1);
                // Un redemarrage repart de zero : garder l'ancienne date
                // ferait afficher une duree de demarrage qui contient la vie
                // entiere du service precedent.
                entree.debut_ns = maintenant_ns;
                entree.pret_ns = 0;
            }
        }
        if matches!(etat, Etat::Actif | Etat::Repos | Etat::Attente) && entree.pret_ns == 0 {
            entree.pret_ns = maintenant_ns;
        }
        if etat == Etat::Reprise {
            entree.reprises = entree.reprises.saturating_add(1);
        }
        entree.etat = etat;

        // CE QUI MERITE UNE LIGNE, ET CE QUI N'EN MERITE PAS.
        //
        // Tout changement d'etat est un evenement -- demarrage, mise en
        // service, degradation, reprise, arret, panne : exactement la liste
        // qu'un journal doit permettre de rejouer -- SAUF les basculements
        // entre etats sains. Un resolveur qui attend, repond, attend encore
        // change d'etat des dizaines de fois par seconde et ne raconte rien.
        //
        // C'est cette exception, et elle seule, qui empeche le registre de
        // refaire ce que le diagnostic d'ordonnancement a fait a l'archive du
        // 17 septembre : effacer le demarrage pour decrire un systeme qui va
        // bien.
        let evenement = !(ancien.sain() && etat.sain());
        if evenement {
            self.compteurs.evenements = self.compteurs.evenements.saturating_add(1);
        }
        evenement
    }

    /// Note une reussite : elle date la derniere activite utile.
    pub fn succes(&mut self, id: &str, maintenant_ns: u64) {
        if let Some(place) = self.place(id) {
            self.entrees[place].dernier_succes_ns = maintenant_ns;
            self.entrees[place].derniere_activite_ns = maintenant_ns;
        }
    }

    /// Note une erreur et sa raison.
    pub fn erreur(&mut self, id: &str, raison: &str, maintenant_ns: u64) {
        if let Some(place) = self.place(id) {
            let entree = &mut self.entrees[place];
            entree.erreurs = entree.erreurs.saturating_add(1);
            entree.derniere_erreur_ns = maintenant_ns;
            entree.derniere_activite_ns = maintenant_ns;
            entree.raison = Id::depuis(raison);
        }
    }

    pub fn lis(&self, id: &str) -> Option<&Entree> {
        self.place(id).map(|place| &self.entrees[place])
    }

    pub fn entrees(&self) -> &[Entree] {
        &self.entrees[..self.occupees]
    }

    /// Les enfants directs d'un parent, pour parcourir l'arbre.
    pub fn enfants<'a>(&'a self, parent: &'a str) -> impl Iterator<Item = &'a Entree> + 'a {
        self.entrees[..self.occupees]
            .iter()
            .filter(move |e| e.parent.egale(parent))
    }

    pub fn compteurs(&self) -> Compteurs {
        self.compteurs
    }

    /// Le service le plus grave, pour un verdict d'un coup d'oeil.
    ///
    /// C'est ce qu'il faut a une barre d'etat : « quelque chose ne va pas, et
    /// voici quoi », sans derouler trente lignes.
    pub fn pire(&self) -> Option<&Entree> {
        self.entrees[..self.occupees]
            .iter()
            .filter(|e| e.etat.problematique())
            .max_by_key(|e| e.etat)
    }
}

// ---------------------------------------------------------------------------
// Les pics de latence
// ---------------------------------------------------------------------------

/// Au-dela de cette attente reveil -> election, un evenement est emis.
///
/// Vingt millisecondes : le critere produit de l'entree est trente, et un pic
/// qui l'approche merite d'etre decrit AVANT de le franchir. En dessous, on ne
/// dit rien -- un journal par reveil ne mesurerait plus que lui-meme.
pub const SEUIL_PIC_REVEIL_US: u64 = 20_000;

/// Un pic vaut-il un enregistrement ?
///
/// La borne seule ne suffit pas : une machine qui souffre produirait mille
/// pics par seconde et effacerait ce qu'elle doit expliquer. Un second pic ne
/// s'ecrit donc qu'apres un repos, ou s'il est NETTEMENT pire que le
/// precedent -- un pic dix fois plus grand est une information neuve.
pub fn pic_a_enregistrer(
    delta_us: u64,
    dernier_pic_ns: u64,
    pire_deja_vu_us: u64,
    maintenant_ns: u64,
    repos_ns: u64,
) -> bool {
    if delta_us < SEUIL_PIC_REVEIL_US {
        return false;
    }
    if dernier_pic_ns == 0 {
        return true;
    }
    if maintenant_ns.saturating_sub(dernier_pic_ns) >= repos_ns {
        return true;
    }
    delta_us >= pire_deja_vu_us.saturating_mul(2)
}

/// Repos entre deux enregistrements de pic, en nanosecondes.
pub const REPOS_PIC_NS: u64 = 5_000_000_000;
