//! Le MODELE VISIBLE de l'application Services, pur.
//!
//! # Pourquoi ce module existe
//!
//! La fenetre Services affichait six lignes Ladybird codees en dur. Le registre
//! central existait a cote, et personne ne le regardait : sur la machine, la
//! photo du 17 septembre montre encore
//!
//! ```text
//! BouchaudBrowserHost   En cours PID [...]
//! WebContent            En cours PID [...]
//! ...
//! Reseau : pret
//! ```
//!
//! -- pas une ligne de Systeme, de Reseau ni de Navigateur.
//!
//! Le defaut n'etait pas dans le registre : il etait dans le fait que la vue
//! ne s'en servait pas. Ce module est la piece qui manquait, et il est PUR :
//! aplatir un arbre, tenir les noeuds replies, calculer une profondeur. Un
//! test hote peut donc exiger que « Systeme, Reseau, Navigateur » soient
//! visibles, et une mutation qui revient aux six lignes Ladybird le fait
//! echouer -- sans demarrer la machine, sans capture d'ecran.
//!
//! Le rendu, lui, ne fait que dessiner ce que ce module a decide.

use super::registre::{Entree, Etat, Genre, Id, Kpi, SERVICES_MAX};

/// Les trois racines, dans l'ordre d'affichage.
///
/// Elles sont une CONSTANTE et non une deduction : une racine qui
/// n'apparaitrait que parce qu'un service a publie quelque chose ferait
/// disparaitre de la vue tout un pan du systeme le jour ou il est muet -- et
/// c'est justement ce jour-la qu'on le cherche.
pub const RACINES: [&str; 3] = ["sys", "net", "browser"];

/// Une ligne affichable.
#[derive(Clone, Copy)]
pub struct Ligne {
    pub entree: Entree,
    /// Profondeur d'indentation, zero pour une racine.
    pub profondeur: u8,
    /// Ce noeud a-t-il des enfants ?
    pub a_des_enfants: bool,
    /// Est-il deploye ?
    pub deploye: bool,
    /// L'etat MONTRE par cette ligne.
    ///
    /// Pour un service, le sien. Pour un groupe, le plus grave de ceux qu'il
    /// porte : sans cela, un arbre replie n'affiche que des entetes muettes,
    /// et il faut ouvrir les sept sous-arbres du reseau pour apprendre lequel
    /// va mal. Ce n'est pas une mesure inventee -- c'est une mesure REPORTEE,
    /// et elle designe toujours un service reel plus bas.
    pub etat_effectif: Etat,
    /// Les indicateurs MONTRES : les siens pour une feuille, la somme de ses
    /// descendants pour un groupe.
    pub kpi_effectif: Kpi,
    /// Les erreurs montrees, meme regle.
    pub erreurs_effectives: u32,
    /// La raison MONTREE, et le service qui la fournit.
    ///
    /// Pour une feuille, la sienne et elle-meme. Pour un groupe, celle de son
    /// descendant le plus grave -- avec son nom, pour savoir ou regarder.
    pub raison_effective: Id,
    pub raison_source: Id,
    /// Ce noeud est-il un regroupement ? Le peintre s'en sert pour distinguer
    /// une branche d'une feuille sans reinterpreter le genre.
    pub est_un_noeud: bool,
}

impl Ligne {
    /// Une ligne vide, constructible en contexte constant.
    ///
    /// Sert a initialiser le tampon statique des vues : sans `const`, il
    /// faudrait le construire a l'execution, sur la pile.
    pub const fn vide() -> Self {
        Self {
            entree: Entree::vide(),
            profondeur: 0,
            a_des_enfants: false,
            deploye: false,
            etat_effectif: Etat::Inconnu,
            kpi_effectif: Kpi {
                cpu_pour_mille: None,
                rss_octets: None,
                vss_octets: None,
                disque_lu: None,
                disque_ecrit: None,
                rx_octets: None,
                tx_octets: None,
                latence_us: None,
                latence_max_us: None,
                operations: None,
                pid: None,
                instances: None,
                fautes_nombre: None,
                fautes_total_us: None,
                fautes_pire_us: None,
            },
            erreurs_effectives: 0,
            raison_effective: Id::vide(),
            raison_source: Id::vide(),
            est_un_noeud: false,
        }
    }

    pub fn id(&self) -> Id {
        self.entree.id
    }

    /// Le libelle affiche : le dernier segment de l'identifiant.
    ///
    /// `net.nic.rtl8168` s'affiche « rtl8168 » sous « nic » sous « net » :
    /// l'arbre porte deja le contexte, le repeter sur chaque ligne volerait la
    /// largeur des colonnes.
    pub fn libelle(&self) -> &str {
        let texte = self.entree.id.texte();
        match texte.rfind('.') {
            Some(point) => &texte[point + 1..],
            None => texte,
        }
    }
}

/// Les noeuds replies, par identifiant. Borne, sans allocation.
///
/// On retient les REPLIES et non les deployes : l'etat par defaut est
/// « deploye », et une vue qui s'ouvre vide ne montre rien de ce qu'on vient
/// d'y chercher.
#[derive(Clone, Copy)]
pub struct Replies {
    ids: [Id; SERVICES_MAX],
    nombre: usize,
}

impl Replies {
    pub const fn neuf() -> Self {
        Self { ids: [Id::vide(); SERVICES_MAX], nombre: 0 }
    }

    /// Replie ou deploie un noeud, et rend son nouvel etat deploye.
    pub fn bascule(&mut self, id: &str) -> bool {
        if let Some(place) = self.ids[..self.nombre].iter().position(|i| i.egale(id)) {
            self.ids.copy_within(place + 1..self.nombre, place);
            self.nombre -= 1;
            return true;
        }
        if self.nombre < SERVICES_MAX {
            self.ids[self.nombre] = Id::depuis(id);
            self.nombre += 1;
        }
        false
    }

    pub fn deploye(&self, id: &str) -> bool {
        !self.ids[..self.nombre].iter().any(|i| i.egale(id))
    }

    /// Deploie un noeud sans le basculer. Sert au premier affichage.
    pub fn deploie(&mut self, id: &str) {
        if let Some(place) = self.ids[..self.nombre].iter().position(|i| i.egale(id)) {
            self.ids.copy_within(place + 1..self.nombre, place);
            self.nombre -= 1;
        }
    }

    /// Replie un noeud sans le basculer. Sert au premier affichage.
    pub fn replie(&mut self, id: &str) {
        if self.deploye(id) && self.nombre < SERVICES_MAX {
            self.ids[self.nombre] = Id::depuis(id);
            self.nombre += 1;
        }
    }
}

impl Default for Replies {
    fn default() -> Self {
        Self::neuf()
    }
}

/// Le repliage du PREMIER affichage : le plus de detail qui TIENNE.
///
/// # Deux captures, deux erreurs opposees
///
/// Tout deployer paraissait genereux : `net` prenait a lui seul vingt-quatre
/// lignes et poussait `browser` sous le bord de la fenetre. La fenetre censee
/// montrer toute la pile en cachait un tiers.
///
/// Tout replier sauf les racines corrigeait cela et creait l'erreur inverse :
/// sept entetes de groupe, aucune mesure, et huit lignes de vide en dessous.
/// Une table des matieres, pas un observatoire.
///
/// # La regle
///
/// On deploie par NIVEAUX, du plus general au plus fin, tant que l'arbre
/// entier tient dans `budget` lignes. Le niveau qui deborderait n'est pas
/// deploye -- ni lui, ni ceux d'en dessous. La premiere vue montre donc
/// toujours le maximum de detail affichable sans defilement, quelle que soit
/// la taille de la topologie ou de la fenetre.
///
/// Les groupes restes replies ne sont pas muets pour autant : chacun reporte
/// l'etat le plus grave qu'il porte (`etat_agrege`).
pub fn repli_par_defaut(entrees: &[Entree], replies: &mut Replies, budget: usize) {
    // Depart : tout est replie sauf les racines.
    for entree in entrees {
        let id = entree.id.texte();
        if !id.is_empty() && !RACINES.iter().any(|r| *r == id) && a_des_enfants(entrees, id) {
            replies.replie(id);
        }
    }

    // Puis on rouvre niveau par niveau, tant que cela tient.
    let mut niveau = 1u8;
    while niveau < 8 {
        let mut essai = *replies;
        let mut candidats = 0usize;
        for entree in entrees {
            let id = entree.id.texte();
            if id.is_empty() || !a_des_enfants(entrees, id) {
                continue;
            }
            if profondeur_de(id) == niveau && !essai.deploye(id) {
                essai.deploie(id);
                candidats += 1;
            }
        }
        if candidats == 0 {
            break;
        }
        if compte(entrees, &essai) > budget {
            // Ce niveau ne tient pas : on s'arrete ici, et les niveaux plus
            // fins restent replies. Les rouvrir un par un donnerait un arbre
            // dont la forme depend de l'ordre de declaration.
            break;
        }
        *replies = essai;
        niveau += 1;
    }
}

/// La profondeur d'un identifiant, comptee en points.
fn profondeur_de(id: &str) -> u8 {
    id.bytes().filter(|o| *o == b'.').count().min(255) as u8
}

/// Combien de lignes cet etat de repliage afficherait.
fn compte(entrees: &[Entree], replies: &Replies) -> usize {
    let mut n = 0usize;
    for racine in RACINES {
        n += compte_sous(entrees, replies, racine);
    }
    n
}

fn compte_sous(entrees: &[Entree], replies: &Replies, id: &str) -> usize {
    if !entrees.iter().any(|e| e.id.egale(id)) {
        return 0;
    }
    let mut n = 1usize;
    if a_des_enfants(entrees, id) && replies.deploye(id) {
        for enfant in entrees.iter().filter(|e| e.parent.egale(id)) {
            n += compte_sous(entrees, replies, enfant.id.texte());
        }
    }
    n
}

/// Construit les lignes visibles, dans l'ordre de l'arbre.
///
/// `sortie` est rempli et sa longueur utile est rendue. Aucune allocation :
/// l'appelant fournit le tableau, comme partout ailleurs dans ce noyau.
pub fn lignes(entrees: &[Entree], replies: &Replies, sortie: &mut [Ligne]) -> usize {
    let mut n = 0usize;
    for racine in RACINES {
        n = pousse(entrees, replies, racine, 0, sortie, n);
    }
    n
}

/// La GRAVITE d'un etat, pour choisir lequel un groupe reporte.
///
/// L'ordre de declaration de `Etat` ne sert pas ici : il met `Arrete` (7)
/// au-dessus de `Degrade` (5), et un groupe contenant un service arrete et un
/// service degrade signalerait l'arret -- le moins urgent des deux.
pub fn gravite(etat: Etat) -> u8 {
    match etat {
        Etat::Inconnu => 0,
        // Un prerequis absent est ATTENDU. Il passe avant « actif » dans
        // l'ordre de remontee, mais il n'alarme pas : une branche entiere
        // indisponible ne doit pas teindre son parent en rouge.
        Etat::Indisponible => 1,
        Etat::Actif => 2,
        Etat::Repos => 3,
        Etat::Attente => 4,
        Etat::Demarrage => 5,
        Etat::Arrete => 6,
        Etat::Reprise => 7,
        Etat::Degrade => 8,
        Etat::Panne => 9,
    }
}

/// L'etat qu'un groupe reporte : le plus grave de ses descendants.
///
/// Un groupe dont personne n'a rien publie reste `Inconnu`, donc « N/A ». Il
/// n'invente pas un « Actif » que rien ne soutient.
pub fn etat_agrege(entrees: &[Entree], id: &str) -> Etat {
    let mut pire = Etat::Inconnu;
    for enfant in entrees.iter().filter(|e| e.parent.egale(id)) {
        let texte = enfant.id.texte();
        let candidat = if a_des_enfants(entrees, texte) {
            etat_agrege(entrees, texte)
        } else {
            enfant.etat
        };
        if gravite(candidat) > gravite(pire) {
            pire = candidat;
        }
    }
    pire
}

/// La RAISON qu'un groupe reporte : celle du descendant le plus grave.
///
/// # Pourquoi un groupe doit parler
///
/// Un arbre replie n'affiche que des entetes. Si l'entete porte l'etat sans
/// la raison, on lit « net : Attente » et il faut ouvrir sept sous-arbres
/// pour apprendre que le serveur DHCP ne repond pas. La raison remonte avec
/// l'etat, sans quoi la remontee ne sert qu'a moitie.
///
/// Elle remonte AVEC son service d'origine : « config : aucun bail » dit d'ou
/// vient la reponse, et ou aller regarder.
pub fn raison_agregee(entrees: &[Entree], id: &str) -> Option<(Id, Id)> {
    let mut pire: Option<(u8, Id, Id)> = None;
    parcourt_feuilles(entrees, id, &mut |feuille: &Entree| {
        if feuille.raison.est_vide() {
            return;
        }
        let g = gravite(feuille.etat);
        if pire.map(|(p, _, _)| g > p).unwrap_or(true) {
            pire = Some((g, feuille.id, feuille.raison));
        }
    });
    pire.map(|(_, qui, quoi)| (qui, quoi))
}

fn parcourt_feuilles(entrees: &[Entree], id: &str, vu: &mut impl FnMut(&Entree)) {
    for enfant in entrees.iter().filter(|e| e.parent.egale(id)) {
        let texte = enfant.id.texte();
        if a_des_enfants(entrees, texte) {
            parcourt_feuilles(entrees, texte, vu);
        } else {
            vu(enfant);
        }
    }
}

/// Les indicateurs qu'un groupe REPORTE : la somme de ce que ses feuilles
/// mesurent.
///
/// # Pourquoi un groupe doit chiffrer
///
/// Une ligne `browser` muette, au-dessus de six processus qui affichent
/// chacun leur CPU, oblige a additionner de tete pour repondre a « qu'est-ce
/// qui mange le processeur ». La somme est exactement ce qu'on vient
/// chercher, et la machine sait la faire.
///
/// Les regles ne sont pas les memes selon la grandeur :
///
/// - CPU, memoire, disque, reseau, operations, erreurs : on ADDITIONNE.
/// - Latence : on garde le MAXIMUM. Additionner des latences produirait un
///   nombre qui ne correspond a aucune attente reelle.
/// - PID : rien. Un groupe n'est pas un processus, et lui en preter un
///   enverrait chercher un fil qui n'existe pas.
///
/// Un groupe dont aucune feuille ne mesure rien rend un `Kpi` vide, donc des
/// tirets -- et non des zeros, qui se liraient comme une mesure.
pub fn kpi_agrege(entrees: &[Entree], id: &str) -> Kpi {
    let mut total = Kpi::default();
    cumule(entrees, id, &mut total);
    total
}

fn cumule(entrees: &[Entree], id: &str, total: &mut Kpi) {
    for enfant in entrees.iter().filter(|e| e.parent.egale(id)) {
        let texte = enfant.id.texte();
        if a_des_enfants(entrees, texte) {
            cumule(entrees, texte, total);
        } else {
            ajoute(total, &enfant.kpi);
        }
    }
}

fn somme(total: &mut Option<u64>, part: Option<u64>) {
    if let Some(v) = part {
        *total = Some(total.unwrap_or(0).saturating_add(v));
    }
}

fn ajoute(total: &mut Kpi, part: &Kpi) {
    if let Some(v) = part.cpu_pour_mille {
        total.cpu_pour_mille = Some(total.cpu_pour_mille.unwrap_or(0).saturating_add(v));
    }
    somme(&mut total.rss_octets, part.rss_octets);
    somme(&mut total.vss_octets, part.vss_octets);
    somme(&mut total.disque_lu, part.disque_lu);
    somme(&mut total.disque_ecrit, part.disque_ecrit);
    somme(&mut total.rx_octets, part.rx_octets);
    somme(&mut total.tx_octets, part.tx_octets);
    somme(&mut total.operations, part.operations);
    somme(&mut total.fautes_nombre, part.fautes_nombre);
    somme(&mut total.fautes_total_us, part.fautes_total_us);
    if let Some(v) = part.instances {
        total.instances = Some(total.instances.unwrap_or(0).saturating_add(v));
    }
    // Ni la latence ni la pire faute ne s'additionnent : la pire attente reste
    // la pire attente, et additionner deux saccades de 4 ms n'en fait pas une
    // de 8 ms -- cela ferait croire a un defaut deux fois plus grave qu'il
    // n'est, dans le noeud meme qu'on regarde pour decider ou chercher.
    if let Some(v) = part.fautes_pire_us {
        total.fautes_pire_us = Some(total.fautes_pire_us.unwrap_or(0).max(v));
    }
    // La latence ne s'additionne pas : la pire attente reste la pire attente.
    if let Some(v) = part.latence_us {
        total.latence_us = Some(total.latence_us.unwrap_or(0).max(v));
    }
    if let Some(v) = part.latence_max_us {
        total.latence_max_us = Some(total.latence_max_us.unwrap_or(0).max(v));
    }
}

/// Les erreurs portees par un sous-arbre.
pub fn erreurs_agregees(entrees: &[Entree], id: &str) -> u32 {
    let mut total = 0u32;
    for enfant in entrees.iter().filter(|e| e.parent.egale(id)) {
        let texte = enfant.id.texte();
        total = total.saturating_add(if a_des_enfants(entrees, texte) {
            erreurs_agregees(entrees, texte)
        } else {
            enfant.erreurs
        });
    }
    total
}

pub fn a_des_enfants(entrees: &[Entree], id: &str) -> bool {
    entrees.iter().any(|e| e.parent.egale(id))
}

fn pousse(
    entrees: &[Entree],
    replies: &Replies,
    id: &str,
    profondeur: u8,
    sortie: &mut [Ligne],
    mut n: usize,
) -> usize {
    let Some(entree) = entrees.iter().find(|e| e.id.egale(id)) else {
        return n;
    };
    if n >= sortie.len() {
        return n;
    }
    let enfants = a_des_enfants(entrees, id);
    let deploye = replies.deploye(id);
    let noeud = enfants && matches!(entree.genre, Genre::Groupe);
    let (etat_effectif, kpi_effectif, erreurs_effectives) = if noeud {
        (
            etat_agrege(entrees, id),
            kpi_agrege(entrees, id),
            erreurs_agregees(entrees, id),
        )
    } else {
        (entree.etat, entree.kpi, entree.erreurs)
    };
    let (raison_effective, raison_source) = if noeud {
        // Une raison n'est remontee que si elle explique l'etat remonte :
        // afficher « aucun bail » a cote d'un « Actif » ferait chercher un
        // probleme la ou il n'y en a plus.
        match raison_agregee(entrees, id) {
            Some((qui, quoi)) if gravite(etat_effectif) > gravite(Etat::Actif) => (quoi, qui),
            _ => (Id::vide(), Id::vide()),
        }
    } else {
        (entree.raison, entree.id)
    };
    sortie[n] = Ligne {
        entree: *entree,
        profondeur,
        a_des_enfants: enfants,
        deploye,
        etat_effectif,
        kpi_effectif,
        erreurs_effectives,
        raison_effective,
        raison_source,
        est_un_noeud: noeud,
    };
    n += 1;
    if !enfants || !deploye {
        return n;
    }
    // L'ORDRE DE DECLARATION EST L'ORDRE D'AFFICHAGE.
    //
    // Trier par nom mettrait `net.arp` avant `net.ethernet` et ferait
    // apparaitre ARP au-dessus de la couche qui le porte. La topologie est
    // declaree dans l'ordre ou on veut la lire.
    let mut place = 0usize;
    while place < entrees.len() {
        if entrees[place].parent.egale(id) {
            let enfant = entrees[place].id;
            n = pousse(entrees, replies, enfant.texte(), profondeur + 1, sortie, n);
        }
        place += 1;
    }
    n
}

/// Le libelle d'etat affiche, en francais court.
pub fn etat_affiche(etat: Etat) -> &'static str {
    match etat {
        // PAS « N/A ». Une colonne entiere de « N/A » ne se lit plus : l'oeil
        // la saute, et le jour ou une vraie valeur y apparait il la saute
        // aussi. Un tiret dit « rien a dire ici » sans occuper le regard.
        Etat::Inconnu => TIRET,
        Etat::Demarrage => "Demarrage",
        Etat::Actif => "Actif",
        Etat::Repos => "Repos",
        Etat::Attente => "Attente",
        Etat::Degrade => "Degrade",
        Etat::Reprise => "Reprise",
        Etat::Arrete => "Arrete",
        // « Panne » decrit la machine ; « Erreur » decrit ce qu'on vient
        // chercher. Le registre garde son nom interne.
        Etat::Panne => "Erreur",
        Etat::Indisponible => "Indisponible",
    }
}

/// Ce qu'on ecrit dans une cellule que personne ne mesure.
pub const TIRET: &str = "\u{2014}";

/// La couleur d'un etat. Sobre : ce tableau doit rester lisible, pas decoratif.
pub fn couleur_etat(etat: Etat) -> u32 {
    match etat {
        // Un service qui tourne ne doit pas attirer l'oeil : c'est le cas
        // normal, et tout mettre en couleur revient a ne rien signaler.
        Etat::Actif => 0x86c98b,
        Etat::Demarrage => 0x8fb6d9,
        Etat::Repos => 0x8a94a6,
        Etat::Attente => 0x7f9bb5,
        Etat::Degrade => 0xd9a05b,
        Etat::Reprise => 0xd9c25b,
        Etat::Arrete => 0x6b7280,
        Etat::Panne => 0xd96b6b,
        // Gris sourd : present dans l'arbre, sans rien exiger de personne.
        Etat::Indisponible => 0x5d6472,
        Etat::Inconnu => 0x454c5a,
    }
}

/// Un genre porte-t-il un PID ?
///
/// « Pour les protocoles : PAS de faux PID. » DNS n'est pas un processus, et
/// lui en inventer un ferait chercher un fil qui n'existe pas.
pub fn porte_un_pid(genre: Genre) -> bool {
    matches!(genre, Genre::Processus | Genre::Service)
}
