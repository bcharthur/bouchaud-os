//! Quand le compositeur doit-il travailler, et quand peut-il dormir.
//!
//! Module de politique pure : aucune I/O, aucun état global. Les échéances du
//! bureau sont calculées à partir de dates et de booléens.

/// Période minimale entre deux trames composées, en millisecondes.
pub const PERIODE_TRAME_MS: u64 = 16;

/// Rafraîchissement de l'horloge/indicateurs visibles.
pub const PERIODE_HORLOGE_MS: u64 = 1000;

/// Cadence d'animation de la jauge de chargement d'un client.
///
/// Elle ne redessine QUE la barre de titre et les trois premières lignes du
/// contenu (voir `window::zone_jauge`), jamais la fenêtre entière — et
/// seulement tant qu'une jauge est visible. Huit rafraîchissements par seconde
/// suffisent à faire avancer une barre de façon continue à l'œil ; la période
/// de trame en donnerait soixante pour rien.
///
/// BOUCHAUD_C13_JAUGE_DE_CHARGEMENT_V1
pub const PERIODE_JAUGE_MS: u64 = 125;

// BOUCHAUD_V16_2_TELEMETRY_CADENCE
//
// Le relevé détaillé produit plusieurs kilo-octets de diagnostics série. Sous
// TCG, COM1 est un périphérique I/O émulé : l'ancien relevé toutes les 5 s
// devenait lui-même une charge interactive et apparaissait dans les traces
// comme une longue tenue BKL hors syscall. 30 s conserve une photographie
// complète exploitable sans transformer le benchmark en générateur de logs.
pub const PERIODE_RELEVE_MS: u64 = 30_000;

// BOUCHAUD_GUI_CHAINE_ENTREE_LFB_V1
pub const DELAI_VEILLE_MS: u64 = 500;

/// Pleine réactivité après interaction avec un client muet.
pub const REACTIVITE_MUETTE_MS: u64 = 600;

/// Filet de sécurité d'un client muet au repos.
pub const REPOS_MUET_MS: u64 = 200;

#[derive(Clone, Copy, Debug, Default)]
pub struct Etat {
    pub maintenant_ms: u64,
    pub sale: bool,
    pub client_muet_visible: bool,
    pub horloge_visible: bool,
    pub derniere_trame_ms: u64,
    pub derniere_horloge_ms: u64,
    pub dernier_releve_ms: u64,
    pub dernier_aveugle_ms: u64,
    pub derniere_entree_ms: u64,
    /// Une jauge de chargement est affichée quelque part.
    pub jauge_visible: bool,
    pub derniere_jauge_ms: u64,
}

impl Etat {
    pub fn periode_aveugle(&self) -> u64 {
        if self.maintenant_ms.wrapping_sub(self.derniere_entree_ms) < REACTIVITE_MUETTE_MS {
            PERIODE_TRAME_MS
        } else {
            REPOS_MUET_MS
        }
    }
}

pub fn prochaine_echeance(etat: &Etat) -> Option<u64> {
    let mut echeance: Option<u64> = None;
    let mut retiens = |date: u64| {
        echeance = Some(match echeance {
            Some(actuelle) => actuelle.min(date),
            None => date,
        });
    };

    if etat.sale {
        retiens(etat.derniere_trame_ms.wrapping_add(PERIODE_TRAME_MS));
    }
    if etat.horloge_visible {
        retiens(etat.derniere_horloge_ms.wrapping_add(PERIODE_HORLOGE_MS));
    }
    if etat.client_muet_visible {
        retiens(etat.dernier_aveugle_ms.wrapping_add(etat.periode_aveugle()));
    }
    // Une jauge qui charge doit avancer, et une jauge terminée doit finir par
    // s'effacer : les deux sont des échéances que personne d'autre n'annonce.
    // Sans cette ligne, le compositeur dormait une seconde entière et la barre
    // avançait par à-coups d'un dixième.
    if etat.jauge_visible {
        retiens(etat.derniere_jauge_ms.wrapping_add(PERIODE_JAUGE_MS));
    }
    retiens(etat.dernier_releve_ms.wrapping_add(PERIODE_RELEVE_MS));

    echeance
}

pub fn doit_composer(etat: &Etat) -> bool {
    etat.sale
        && etat.maintenant_ms.wrapping_sub(etat.derniere_trame_ms) >= PERIODE_TRAME_MS
}

pub fn doit_recomposer_aveugle(etat: &Etat) -> bool {
    etat.client_muet_visible
        && etat.maintenant_ms.wrapping_sub(etat.dernier_aveugle_ms) >= etat.periode_aveugle()
}

pub fn doit_animer_jauge(etat: &Etat) -> bool {
    etat.jauge_visible
        && etat.maintenant_ms.wrapping_sub(etat.derniere_jauge_ms) >= PERIODE_JAUGE_MS
}

pub fn doit_rafraichir_horloge(etat: &Etat) -> bool {
    etat.horloge_visible
        && etat.maintenant_ms.wrapping_sub(etat.derniere_horloge_ms) >= PERIODE_HORLOGE_MS
}

pub fn duree_sommeil_ms(etat: &Etat) -> Option<u64> {
    prochaine_echeance(etat).map(|date| date.saturating_sub(etat.maintenant_ms))
}
