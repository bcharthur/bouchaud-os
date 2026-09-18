//! Le pipeline de navigation : a quelle etape une page bloque.
//!
//! # La question a laquelle ce module repond
//!
//! « Ladybird n'affiche pas la page. » Sans ce module, la reponse demande de
//! lire une trace serie, de recouper un compteur DNS, un compteur TCP et un
//! journal TLS, puis de deviner lequel des trois est arrive en premier. La
//! fenetre Services montrait bien les douze etapes de la navigation -- et les
//! douze etaient vides.
//!
//! Une navigation est une CHAINE. Chaque maillon a un etat, une duree et,
//! quand il echoue, une raison. La premiere case qui n'est pas verte est la
//! reponse.
//!
//! # Ce que ce module observe VRAIMENT
//!
//! Le noyau voit ce qui passe par lui :
//!
//! - l'URL demandee, quand une navigation commence ;
//! - la resolution de nom, le raccordement TCP, la poignee TLS, la requete
//!   HTTP et le corps telecharge -- par `net::fetch_document` pour le chemin
//!   noyau, et par les appels systeme `connect`/`recvfrom` pour le
//!   navigateur, qui vit en anneau 3.
//!
//! Il ne voit PAS ce qui se passe dans `WebContent` : le decodage, l'analyse
//! HTML et CSS, la mise en page, la peinture et la presentation sont du code
//! d'anneau 3 qui ne traverse aucun appel systeme observable.
//!
//! CES ETAPES-LA SE DECLARENT `Indisponible`, AVEC LEUR RAISON. Les laisser
//! vides laisserait croire qu'elles n'ont pas eu lieu, ou qu'elles ont
//! echoue ; « non instrumente (anneau 3) » dit ce qui est vrai, et pourquoi
//! il ne faut pas chercher la panne la.
//!
//! # Pas d'allocation, pas d'horloge propre
//!
//! Le temps est fourni par l'appelant. L'URL vit dans un tampon fixe : une
//! navigation qui alloue dans le noyau a chaque clic est une panne memoire en
//! sursis.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::kernel::services::registre::Etat;
use crate::kernel::sync::SpinLockIrq;

/// Longueur retenue d'une URL. Au-dela, elle est tronquee : on affiche
/// l'hote et le debut du chemin, ce qui suffit a savoir de quelle page on
/// parle.
pub const URL_MAX: usize = 96;

/// Les etapes, dans l'ordre ou elles s'enchainent.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Etape {
    Url = 0,
    Dns = 1,
    Tcp = 2,
    Tls = 3,
    Http = 4,
    Telechargement = 5,
    Decodage = 6,
    Html = 7,
    Css = 8,
    Mise_en_page = 9,
    Peinture = 10,
    Presentation = 11,
}

/// Le nombre d'etapes.
pub const ETAPES: usize = 12;

impl Etape {
    /// L'identifiant de service de cette etape.
    pub fn service(self) -> &'static str {
        match self {
            Etape::Url => "browser.navigation.url",
            Etape::Dns => "browser.navigation.dns",
            Etape::Tcp => "browser.navigation.tcp",
            Etape::Tls => "browser.navigation.tls",
            Etape::Http => "browser.navigation.http",
            Etape::Telechargement => "browser.navigation.download",
            Etape::Decodage => "browser.navigation.decode",
            Etape::Html => "browser.navigation.html",
            Etape::Css => "browser.navigation.css",
            Etape::Mise_en_page => "browser.navigation.layout",
            Etape::Peinture => "browser.navigation.paint",
            Etape::Presentation => "browser.navigation.present",
        }
    }

    /// Cette etape est-elle observable depuis le noyau ?
    ///
    /// Les six premieres traversent la pile reseau ; les six suivantes vivent
    /// dans `WebContent`, en anneau 3.
    pub fn observable(self) -> bool {
        (self as u8) <= (Etape::Telechargement as u8)
    }

    pub fn depuis_rang(rang: usize) -> Option<Etape> {
        Some(match rang {
            0 => Etape::Url,
            1 => Etape::Dns,
            2 => Etape::Tcp,
            3 => Etape::Tls,
            4 => Etape::Http,
            5 => Etape::Telechargement,
            6 => Etape::Decodage,
            7 => Etape::Html,
            8 => Etape::Css,
            9 => Etape::Mise_en_page,
            10 => Etape::Peinture,
            11 => Etape::Presentation,
            _ => return None,
        })
    }
}

/// Ce qu'on retient d'une etape.
#[derive(Clone, Copy)]
pub struct Mesure {
    pub etat: Etat,
    /// Duree de l'etape, en microsecondes. `0` tant qu'elle n'est pas finie.
    pub duree_us: u64,
    /// Octets traites par cette etape, quand cela veut dire quelque chose.
    pub octets: u64,
    /// Debut de l'etape, pour calculer la duree a la fin.
    debut_ns: u64,
}

impl Mesure {
    const fn neuve() -> Self {
        Self { etat: Etat::Inconnu, duree_us: 0, octets: 0, debut_ns: 0 }
    }
}

/// L'etat d'une navigation.
pub struct Navigation {
    url: [u8; URL_MAX],
    url_len: usize,
    pub etapes: [Mesure; ETAPES],
    pub debut_ns: u64,
    pub fin_ns: u64,
}

impl Navigation {
    const fn neuve() -> Self {
        Self {
            url: [0; URL_MAX],
            url_len: 0,
            etapes: [Mesure::neuve(); ETAPES],
            debut_ns: 0,
            fin_ns: 0,
        }
    }

    pub fn url(&self) -> &str {
        core::str::from_utf8(&self.url[..self.url_len]).unwrap_or("?")
    }

    pub fn en_cours(&self) -> bool {
        self.debut_ns != 0 && self.fin_ns == 0
    }
}

static COURANTE: SpinLockIrq<Navigation> = SpinLockIrq::new(Navigation::neuve());
/// Une navigation a-t-elle jamais eu lieu ? Evite de prendre le verrou pour
/// repondre « rien » cent fois par seconde.
static VUE: AtomicBool = AtomicBool::new(false);
static NAVIGATIONS: AtomicU64 = AtomicU64::new(0);

/// Une navigation commence. Remet toutes les etapes a zero.
pub fn debute(url: &str, maintenant_ns: u64) {
    let mut n = COURANTE.lock();
    n.url_len = 0;
    for (place, octet) in url.as_bytes().iter().take(URL_MAX).enumerate() {
        n.url[place] = *octet;
        n.url_len = place + 1;
    }
    n.etapes = [Mesure::neuve(); ETAPES];
    n.debut_ns = maintenant_ns;
    n.fin_ns = 0;
    // L'URL est connue des le depart : c'est la premiere etape, et elle
    // reussit toujours.
    n.etapes[Etape::Url as usize].etat = Etat::Actif;
    n.etapes[Etape::Url as usize].debut_ns = maintenant_ns;
    VUE.store(true, Ordering::Release);
    NAVIGATIONS.fetch_add(1, Ordering::Relaxed);
}

/// Une etape demarre.
pub fn entre(etape: Etape, maintenant_ns: u64) {
    let mut n = COURANTE.lock();
    let m = &mut n.etapes[etape as usize];
    m.etat = Etat::Demarrage;
    m.debut_ns = maintenant_ns;
}

/// Une etape reussit.
pub fn reussit(etape: Etape, octets: u64, maintenant_ns: u64) {
    let mut n = COURANTE.lock();
    let m = &mut n.etapes[etape as usize];
    if m.debut_ns != 0 {
        m.duree_us = maintenant_ns.saturating_sub(m.debut_ns) / 1_000;
    }
    m.etat = Etat::Actif;
    m.octets = m.octets.saturating_add(octets);
}

/// Une etape echoue. C'est CELLE-LA qu'on vient chercher.
pub fn echoue(etape: Etape, maintenant_ns: u64) {
    let mut n = COURANTE.lock();
    let m = &mut n.etapes[etape as usize];
    if m.debut_ns != 0 {
        m.duree_us = maintenant_ns.saturating_sub(m.debut_ns) / 1_000;
    }
    m.etat = Etat::Panne;
    n.fin_ns = maintenant_ns;
}

/// Une etape est SAUTEE : elle n'avait pas lieu d'etre.
///
/// A distinguer d'un echec et d'une absence. Une page en clair ne franchit
/// aucune poignee TLS ; marquer TLS « en panne » enverrait chercher un
/// probleme de certificat, et la laisser vide laisserait croire qu'elle
/// attend encore.
pub fn saute(etape: Etape, _pourquoi: &str, maintenant_ns: u64) {
    let mut n = COURANTE.lock();
    let m = &mut n.etapes[etape as usize];
    m.etat = Etat::Indisponible;
    m.debut_ns = maintenant_ns;
    m.duree_us = 0;
}

/// La navigation se termine.
pub fn termine(maintenant_ns: u64) {
    let mut n = COURANTE.lock();
    n.fin_ns = maintenant_ns;
}

/// Une copie de la navigation courante, pour publier sans tenir le verrou.
pub fn instantane() -> Option<([u8; URL_MAX], usize, [Mesure; ETAPES], u64, u64)> {
    if !VUE.load(Ordering::Acquire) {
        return None;
    }
    let n = COURANTE.lock();
    Some((n.url, n.url_len, n.etapes, n.debut_ns, n.fin_ns))
}

/// Combien de navigations depuis le demarrage.
pub fn compteur() -> u64 {
    NAVIGATIONS.load(Ordering::Relaxed)
}
