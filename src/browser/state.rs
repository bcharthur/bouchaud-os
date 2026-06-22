//! Etat du navigateur Nautile : onglets, historique, session JS.
//!
//! Inspiré de l'architecture multi-process de Chrome/Firefox (processus renderer
//! séparé par onglet) — ici un processus léger = un `Tab` avec sa propre Session.

use crate::gui::web::{Page, Session};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

const MAX_HIST: usize = 50;

// ── Onglet ────────────────────────────────────────────────────────────────────

/// Un onglet : page rendue, session JS persistante, historique de navigation.
pub struct Tab {
    /// URL courante (confirmée après chargement).
    pub url: String,
    /// Texte de la barre d'adresse (peut différer de `url` pendant la saisie).
    pub input: String,
    /// Page rendue (liste d'affichage) issue du moteur web.
    pub page: Page,
    /// Contexte JS/DOM persistant pour les interactions inline.
    pub session: Session,
    /// Position de défilement verticale en pixels.
    pub scroll: i32,
    /// Pile d'historique de navigation (URLs).
    pub history: Vec<String>,
    /// Index courant dans `history`.
    pub hist_pos: usize,
    /// Titre affiché dans l'onglet.
    pub title: String,
    /// Message de la barre de statut (survol de lien, etc.).
    pub status: String,
}

impl Tab {
    pub fn new(url: String, page: Page, session: Session) -> Self {
        let title = derive_title(&url, &page.title);
        Tab {
            input:    url.clone(),
            history:  alloc::vec![url.clone()],
            hist_pos: 0,
            url,
            page,
            session,
            scroll:  0,
            title,
            status: String::new(),
        }
    }

    pub fn can_back(&self)    -> bool { self.hist_pos > 0 }
    pub fn can_forward(&self) -> bool { self.hist_pos + 1 < self.history.len() }

    /// Enregistre une nouvelle navigation dans la pile d'historique.
    pub fn push_nav(&mut self, url: &str) {
        self.history.truncate(self.hist_pos + 1);
        self.history.push(url.to_string());
        if self.history.len() > MAX_HIST {
            self.history.remove(0);
        } else {
            self.hist_pos += 1;
        }
        self.url   = url.to_string();
        self.input = url.to_string();
    }

    /// Recule d'une entrée, renvoie l'URL cible ou None.
    pub fn go_back(&mut self) -> Option<String> {
        if self.hist_pos == 0 { return None; }
        self.hist_pos -= 1;
        let u = self.history[self.hist_pos].clone();
        self.url   = u.clone();
        self.input = u.clone();
        Some(u)
    }

    /// Avance d'une entrée, renvoie l'URL cible ou None.
    pub fn go_forward(&mut self) -> Option<String> {
        if self.hist_pos + 1 >= self.history.len() { return None; }
        self.hist_pos += 1;
        let u = self.history[self.hist_pos].clone();
        self.url   = u.clone();
        self.input = u.clone();
        Some(u)
    }

    /// Applique une page fraîchement chargée à cet onglet.
    pub fn apply(&mut self, url: &str, page: Page, session: Session) {
        self.title   = derive_title(url, &page.title);
        self.page    = page;
        self.session = session;
        self.scroll  = 0;
        self.status  = String::new();
        self.url     = url.to_string();
        self.input   = url.to_string();
    }
}

fn derive_title(url: &str, doc_title: &str) -> String {
    if !doc_title.is_empty() { return doc_title.to_string(); }
    if let Some(rest) = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")) {
        return rest.split('/').next().unwrap_or(rest).to_string();
    }
    if let Some(name) = url.strip_prefix("about:") { return name.to_string(); }
    url.to_string()
}

// ── BrowserState ──────────────────────────────────────────────────────────────

/// État global du navigateur : tous les onglets ouverts.
pub struct BrowserState {
    pub tabs:   Vec<Tab>,
    pub active: usize,
}

impl BrowserState {
    pub fn new(url: String, page: Page, session: Session) -> Self {
        BrowserState { tabs: alloc::vec![Tab::new(url, page, session)], active: 0 }
    }

    /// Onglet actif (lecture).
    #[inline] pub fn tab(&self) -> &Tab { &self.tabs[self.active] }
    /// Onglet actif (mutation).
    #[inline] pub fn tab_mut(&mut self) -> &mut Tab { &mut self.tabs[self.active] }

    /// Ouvre un nouvel onglet et l'active.
    pub fn add_tab(&mut self, url: String, page: Page, session: Session) {
        self.tabs.push(Tab::new(url, page, session));
        self.active = self.tabs.len() - 1;
    }

    /// Ferme l'onglet à l'index `idx`. Retourne false si c'est le dernier onglet.
    pub fn close_tab_at(&mut self, idx: usize) -> bool {
        if self.tabs.len() <= 1 || idx >= self.tabs.len() { return false; }
        self.tabs.remove(idx);
        if self.active >= self.tabs.len() { self.active = self.tabs.len() - 1; }
        true
    }

    /// Active l'onglet d'index `idx`.
    pub fn select(&mut self, idx: usize) {
        if idx < self.tabs.len() { self.active = idx; }
    }
}
