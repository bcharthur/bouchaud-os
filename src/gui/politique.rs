//! Quand le compositeur doit-il travailler, et quand peut-il dormir.
//!
//! # Pourquoi cette decision vit ici et pas dans la boucle
//!
//! Elle etait repartie sur une dizaine de lignes de `boucle()`, entre deux
//! traitements d'entree, sous la forme d'un `sleep_ticks(4 ou 16)` choisi par un
//! `if` a trois branches. Impossible a tester : il aurait fallu un framebuffer,
//! un clavier, une souris et un ordonnanceur pour poser une question de pure
//! arithmetique.
//!
//! Ce module ne contient donc AUCUN etat et n'appelle rien : il repond a
//! « prochaine echeance ? » a partir de dates. Le harnais hote l'exerce
//! directement.
//!
//! # Le principe
//!
//! Un compositeur event-driven ne se reveille que sur deux choses :
//!
//!   * un SIGNAL -- entree, trame client, changement de fenetre. Il arrive par
//!     `kernel::sync::reveil` et n'a besoin d'aucune echeance ;
//!   * une ECHEANCE -- ce qui change tout seul, sans que personne l'annonce.
//!
//! Tout ce qui n'est ni l'un ni l'autre doit produire un sommeil sans fin.
//! [`prochaine_echeance`] rend `None` dans ce cas, et c'est la propriete que le
//! test verifie : sans horloge, sans client muet et sans degat, le bureau n'a
//! aucune raison de se reveiller.

/// Periode minimale entre deux trames composees, en millisecondes.
pub const PERIODE_TRAME_MS: u64 = 16;

/// Periode de rafraichissement des indicateurs systeme (heure, CPU, memoire).
///
/// # LA SEULE ANIMATION PERMANENTE DU BUREAU
///
/// L'horloge de la barre des taches affiche des secondes : elle change toute
/// seule, sans evenement, et rien ne peut l'annoncer. C'est donc le seul
/// element qui empeche un bureau totalement immobile de dormir indefiniment --
/// il se reveille une fois par seconde, compose la zone de l'horloge, et se
/// rendort.
///
/// Ce n'est pas un defaut a corriger en douce : c'est de l'interface reelle. Ce
/// qui SERAIT un defaut, c'est de le laisser couter une trame plein ecran, et
/// c'est ce que borne `Origine::BarreTaches` en n'invalidant que la barre.
///
/// Une mesure d'inactivite doit donc compter les trames HORS horloge : c'est ce
/// que la ligne `[GUI-COMPOSITOR]` separe explicitement.
pub const PERIODE_HORLOGE_MS: u64 = 1000;

/// Periode du releve de charge par processus, en millisecondes.
pub const PERIODE_RELEVE_MS: u64 = 5000;

/// Duree pendant laquelle un client muet est recompose a pleine cadence apres
/// une interaction, en millisecondes.
pub const REACTIVITE_MUETTE_MS: u64 = 600;

/// Periode de recomposition d'un client muet au repos, en millisecondes.
pub const REPOS_MUET_MS: u64 = 200;

/// Ce que le compositeur sait de lui-meme au moment de decider.
///
/// Uniquement des dates et des booleens : rien qui demande un peripherique.
#[derive(Clone, Copy, Debug, Default)]
pub struct Etat {
    pub maintenant_ms: u64,
    /// Un degat attend d'etre compose.
    pub sale: bool,
    /// Au moins un client visible n'annonce pas ses trames. Lui seul justifie
    /// la recomposition periodique « a l'aveugle ».
    pub client_muet_visible: bool,
    /// L'horloge de la barre des taches est affichee.
    pub horloge_visible: bool,
    pub derniere_trame_ms: u64,
    pub derniere_horloge_ms: u64,
    pub dernier_releve_ms: u64,
    pub dernier_aveugle_ms: u64,
    pub derniere_entree_ms: u64,
}

impl Etat {
    /// Cadence de recomposition d'un client muet : pleine juste apres une
    /// entree, de veille sinon.
    pub fn periode_aveugle(&self) -> u64 {
        if self.maintenant_ms.wrapping_sub(self.derniere_entree_ms) < REACTIVITE_MUETTE_MS {
            PERIODE_TRAME_MS
        } else {
            REPOS_MUET_MS
        }
    }
}

/// Prochaine date a laquelle le compositeur DOIT se reveiller sans qu'aucun
/// evenement ne le lui demande.
///
/// `None` signifie « rien ne changera tout seul » : le compositeur peut dormir
/// jusqu'au prochain signal, sans limite de temps. C'est l'etat que Gate 1B
/// cherche a rendre atteignable.
pub fn prochaine_echeance(etat: &Etat) -> Option<u64> {
    let mut echeance: Option<u64> = None;
    let mut retiens = |date: u64| {
        echeance = Some(match echeance {
            Some(actuelle) => actuelle.min(date),
            None => date,
        });
    };

    // Un degat en attente : il sera compose au prochain creneau de trame.
    if etat.sale {
        retiens(etat.derniere_trame_ms.wrapping_add(PERIODE_TRAME_MS));
    }
    // L'horloge, seule animation permanente. Voir PERIODE_HORLOGE_MS.
    if etat.horloge_visible {
        retiens(etat.derniere_horloge_ms.wrapping_add(PERIODE_HORLOGE_MS));
    }
    // Un client muet ne dit pas quand il peint : c'est le seul polling qui
    // reste, et il disparait des que plus aucun client muet n'est visible.
    if etat.client_muet_visible {
        retiens(etat.dernier_aveugle_ms.wrapping_add(etat.periode_aveugle()));
    }
    // Le releve de charge. Il n'affiche rien : il ecrit dans le journal.
    retiens(etat.dernier_releve_ms.wrapping_add(PERIODE_RELEVE_MS));

    echeance
}

/// Faut-il composer maintenant ?
///
/// Separe de l'echeance : une trame sale n'est composee qu'une fois le creneau
/// atteint, mais l'echeance doit exister des que le degat existe.
pub fn doit_composer(etat: &Etat) -> bool {
    etat.sale
        && etat.maintenant_ms.wrapping_sub(etat.derniere_trame_ms) >= PERIODE_TRAME_MS
}

/// Faut-il recomposer un client muet maintenant ?
pub fn doit_recomposer_aveugle(etat: &Etat) -> bool {
    etat.client_muet_visible
        && etat.maintenant_ms.wrapping_sub(etat.dernier_aveugle_ms) >= etat.periode_aveugle()
}

/// Faut-il rafraichir les indicateurs systeme maintenant ?
pub fn doit_rafraichir_horloge(etat: &Etat) -> bool {
    etat.horloge_visible
        && etat.maintenant_ms.wrapping_sub(etat.derniere_horloge_ms) >= PERIODE_HORLOGE_MS
}

/// Duree de sommeil, en millisecondes, ou `None` pour un sommeil sans fin.
///
/// Zero signifie « ne dors pas » : une echeance deja atteinte doit relancer un
/// tour de boucle, pas un sommeil de duree nulle qui couterait deux changements
/// de contexte pour rien.
pub fn duree_sommeil_ms(etat: &Etat) -> Option<u64> {
    prochaine_echeance(etat).map(|date| date.saturating_sub(etat.maintenant_ms))
}
