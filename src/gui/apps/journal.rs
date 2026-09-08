//! Application Journal : la trace du noyau, lisible sur la machine elle-meme.
//!
//! # Pourquoi une fenetre, et pas un cable
//!
//! Le noyau ecrit tout sur COM1, et un mini-PC n'expose pas COM1. Toute la
//! trace etait donc produite, dupliquee en RAM par
//! `drivers::serial::trace_snapshot` -- et lisible par personne. Les pages de
//! diagnostic la montraient huit secondes pendant l'amorcage, en bloquant le
//! demarrage pour le faire ; c'etait le seul acces, et il coutait le bureau.
//!
//! Cette fenetre rend le meme contenu sans rien bloquer : elle s'ouvre quand
//! on le demande, se filtre, se fait defiler, et se photographie.
//!
//! # Pourquoi la capture est figee
//!
//! `trace_snapshot` alloue soixante-quatre kilooctets et les redecoupe en
//! lignes. Le faire a chaque trame ferait de l'ouverture d'une fenetre une
//! allocation de 64 Kio soixante fois par seconde, sur le chemin du
//! compositeur -- exactement ce qu'on retire partout ailleurs.
//!
//! La capture est donc prise a l'OUVERTURE, et refaite seulement quand on la
//! demande (touche R). Ce que la fenetre montre est un instantane, et son
//! en-tete le dit : un journal qui pretendrait etre en direct sans l'etre
//! serait pire qu'un journal date.

use alloc::string::String;
use alloc::vec::Vec;

use crate::gui::event::Key;
use crate::gui::framebuffer as fb;
use crate::gui::window::clip;

/// Ce que le filtre laisse passer.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Filtre {
    Tout,
    Amorcage,
    Usb,
    Stockage,
    Reseau,
    Fautes,
}

impl Filtre {
    fn nom(self) -> &'static str {
        match self {
            Filtre::Tout => "TOUT",
            Filtre::Amorcage => "AMORCAGE",
            Filtre::Usb => "USB/HID",
            Filtre::Stockage => "STOCKAGE",
            Filtre::Reseau => "RESEAU",
            Filtre::Fautes => "FAUTES",
        }
    }

    fn suivant(self) -> Self {
        match self {
            Filtre::Tout => Filtre::Amorcage,
            Filtre::Amorcage => Filtre::Usb,
            Filtre::Usb => Filtre::Stockage,
            Filtre::Stockage => Filtre::Reseau,
            Filtre::Reseau => Filtre::Fautes,
            Filtre::Fautes => Filtre::Tout,
        }
    }

    /// Le filtre retient-il cette ligne ?
    ///
    /// La comparaison est faite sur la ligne DEJA mise en majuscules a la
    /// capture. Replier a chaque test referait le meme travail a chaque trame,
    /// pour un resultat qui ne change pas.
    fn retient(self, majuscule: &str) -> bool {
        match self {
            Filtre::Tout => true,
            Filtre::Amorcage => {
                majuscule.contains("BOUCHAUD_") || majuscule.contains("[STAGE2]")
            }
            Filtre::Usb => {
                majuscule.contains("XHCI")
                    || majuscule.contains("USB")
                    || majuscule.contains("HID")
                    || majuscule.contains("CLAVIER")
                    || majuscule.contains("SOURIS")
            }
            Filtre::Stockage => {
                majuscule.contains("NVME")
                    || majuscule.contains("GPT")
                    || majuscule.contains("BLOC")
                    || majuscule.contains("PERSIST")
                    || majuscule.contains("INSTALL")
            }
            Filtre::Reseau => {
                majuscule.contains("NET")
                    || majuscule.contains("DHCP")
                    || majuscule.contains("E1000")
                    || majuscule.contains("RTL")
                    || majuscule.contains("ETH0")
                    || majuscule.contains("DNS")
            }
            Filtre::Fautes => {
                majuscule.contains("FAULT")
                    || majuscule.contains("PANIC")
                    || majuscule.contains("ERREUR")
                    || majuscule.contains("ERROR")
                    || majuscule.contains("ECHEC")
                    || majuscule.contains("FAIL")
                    || majuscule.contains("HORS_SERVICE")
            }
        }
    }
}

/// Une ligne capturee, et sa forme repliee pour le filtre.
struct Ligne {
    texte: String,
    majuscule: String,
}

pub(crate) struct JournalState {
    lignes: Vec<Ligne>,
    /// Premiere ligne VISIBLE, comptee parmi celles que le filtre retient.
    scroll: usize,
    filtre: Filtre,
    /// Octets que le noyau avait ecrits au moment de la capture.
    octets: usize,
    /// La capture suit-elle la fin du journal ?
    suit_la_fin: bool,
}

impl JournalState {
    pub(crate) fn neuf() -> Self {
        let mut etat = Self {
            lignes: Vec::new(),
            scroll: 0,
            filtre: Filtre::Tout,
            octets: 0,
            suit_la_fin: true,
        };
        etat.capture();
        etat
    }

    /// Reprend la trace du noyau telle qu'elle est maintenant.
    fn capture(&mut self) {
        let brut = crate::drivers::serial::trace_snapshot();
        self.octets = crate::drivers::serial::trace_total_bytes();
        self.lignes.clear();

        let mut courante = String::new();
        for octet in brut {
            match octet {
                b'\n' => {
                    let majuscule = courante.to_uppercase();
                    self.lignes.push(Ligne { texte: core::mem::take(&mut courante), majuscule });
                }
                b'\r' => {}
                b'\t' => courante.push(' '),
                0x20..=0x7E => courante.push(octet as char),
                _ => courante.push('?'),
            }
        }
        if !courante.is_empty() {
            let majuscule = courante.to_uppercase();
            self.lignes.push(Ligne { texte: courante, majuscule });
        }
        // La PREMIERE ligne de l'anneau est presque toujours tronquee : il n'y
        // a aucune raison qu'un tampon circulaire s'ouvre sur un debut de
        // ligne. La montrer laisserait croire a un message coupe par le noyau.
        if self.lignes.len() > 1 && self.octets > brut_capacite() {
            self.lignes.remove(0);
        }
        // Une recapture demandee depuis la fin du journal doit rendre la
        // NOUVELLE fin : c'est ce qu'on veut voir quand on appuie sur R apres
        // avoir reproduit un probleme. Le calage precis est fait au dessin,
        // qui seul connait la hauteur de la fenetre ; ici on marque l'intention.
        if self.suit_la_fin {
            self.scroll = usize::MAX;
        } else {
            self.scroll = 0;
        }
    }

    fn retenues(&self) -> usize {
        self.lignes.iter().filter(|l| self.filtre.retient(&l.majuscule)).count()
    }
}

/// Capacite de l'anneau de trace, en octets.
fn brut_capacite() -> usize {
    64 * 1024
}

/// Lignes de texte qui tiennent dans le corps de la fenetre.
fn lignes_visibles(bh: usize) -> usize {
    // Deux lignes d'en-tete, une de pied.
    (bh / 10).saturating_sub(3).max(1)
}

pub(crate) fn draw(st: &JournalState, bx: usize, by: usize, bw: usize, bh: usize) {
    let cols = bw / 8;
    let visibles = lignes_visibles(bh);
    let retenues = st.retenues();
    // `usize::MAX` veut dire « la fin », pose par une recapture. Le borner ici
    // est le seul endroit qui connaisse le nombre de lignes affichables.
    let depart = st.scroll.min(retenues.saturating_sub(visibles));

    let mut yy = by;
    fb::draw_text(
        bx,
        yy,
        clip(
            &alloc::format!(
                "JOURNAL NOYAU  filtre:{}  {}/{} lignes  {} o",
                st.filtre.nom(),
                retenues,
                st.lignes.len(),
                st.octets,
            ),
            cols,
        ),
        fb::C_YELLOW,
    );
    yy += 10;
    fb::draw_text(
        bx,
        yy,
        clip("F filtre   R recapture   fleches/PgUp/PgDn/Origine/Fin defilent", cols),
        fb::C_CYAN,
    );
    yy += 10;

    let mut rang = 0usize;
    let mut dessinees = 0usize;
    for ligne in st.lignes.iter() {
        if !st.filtre.retient(&ligne.majuscule) {
            continue;
        }
        if rang < depart {
            rang += 1;
            continue;
        }
        if dessinees >= visibles {
            break;
        }
        // La couleur porte le sens : ce qui a echoue doit sauter aux yeux dans
        // une page de texte monochrome.
        let couleur = if ligne.majuscule.contains("FAULT")
            || ligne.majuscule.contains("PANIC")
            || ligne.majuscule.contains("_FAIL")
            || ligne.majuscule.contains("HORS_SERVICE")
        {
            fb::C_RED
        } else if ligne.majuscule.contains("_OK")
            || ligne.majuscule.contains("_GREEN")
            || ligne.majuscule.contains("_READY")
            || ligne.majuscule.contains("PRET")
        {
            fb::C_GREEN
        } else if ligne.majuscule.contains("BOUCHAUD_") {
            fb::C_WHITE
        } else {
            fb::C_GRAY
        };
        fb::draw_text(bx, yy, clip(&ligne.texte, cols), couleur);
        yy += 10;
        rang += 1;
        dessinees += 1;
    }

    if dessinees == 0 {
        fb::draw_text(bx, yy, clip("(aucune ligne ne passe ce filtre)", cols), fb::C_GRAY);
    }
}

/// Fait defiler, en restant dans les bornes de ce que le filtre retient.
fn deplace(st: &mut JournalState, delta: i32, visibles: usize) {
    let retenues = st.retenues();
    let plafond = retenues.saturating_sub(visibles);
    // Resoudre le sentinel « la fin » AVANT d'ajouter le delta. Sans cela,
    // `usize::MAX as i64` vaut -1, et la premiere fleche vers le bas renverrait
    // au debut du journal au lieu d'avancer d'une ligne.
    let courant = st.scroll.min(plafond);
    let nouveau = courant as i64 + delta as i64;
    st.scroll = nouveau.clamp(0, plafond as i64) as usize;
    st.suit_la_fin = st.scroll >= plafond;
}

pub(crate) fn on_key(st: &mut JournalState, k: Key) -> bool {
    // La fenetre ne connait pas sa hauteur ici ; vingt lignes est la page
    // nominale, et `deplace` borne de toute facon le resultat.
    const PAGE: i32 = 20;
    match k {
        Key::Char(b'f') | Key::Char(b'F') => {
            st.filtre = st.filtre.suivant();
            st.scroll = 0;
        }
        Key::Char(b'r') | Key::Char(b'R') => {
            let filtre = st.filtre;
            st.capture();
            st.filtre = filtre;
        }
        Key::Up => deplace(st, -1, PAGE as usize),
        Key::Down => deplace(st, 1, PAGE as usize),
        Key::PageUp => deplace(st, -PAGE, PAGE as usize),
        Key::PageDown => deplace(st, PAGE, PAGE as usize),
        Key::Home => {
            st.scroll = 0;
            st.suit_la_fin = false;
        }
        Key::End => deplace(st, i32::MAX / 2, PAGE as usize),
        _ => {}
    }
    false
}

pub(crate) fn on_wheel(st: &mut JournalState, delta: i32) {
    deplace(st, -delta * 3, 20);
}
