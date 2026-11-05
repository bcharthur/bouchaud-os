//! Bouchaud Notes M1 — editeur natif lie au RAMFS.
//!
//! Stockage RAMFS : PAS de promesse de persistance apres reboot.
//! F2 sauvegarde; F3 recharge uniquement si le tampon est propre.
//! L'editeur ne fait aucun acces NVMe et respecte les droits RAMFS.
use crate::fs::ramfs::{self, NodeKind, PERM_R, PERM_W, PERM_X};
use crate::fs::persistance;
use crate::gui::event::Key;
use crate::gui::framebuffer as fb;
use crate::gui::window::clip;
use alloc::format;
use alloc::string::{String, ToString};

const FICHIER: &str = "bouchaud-notes.txt";
const LIMITE: usize = 256 * 1024;
const LIGNES_VISIBLES: usize = 17;
const HAUTEUR_LIGNE: usize = 20;

pub(crate) struct NotesState {
    home: usize,
    /// Fichier ouvert depuis Fichiers (BOUCHAUD_NOTES_OUVRIR_V1). `None` : le
    /// carnet historique (`bouchaud-notes.txt`, sous `/persist` ou l'accueil).
    chemin: Option<String>,
    texte: String,
    curseur: usize, // offset UTF-8, toujours sur une frontiere de caractere
    defilement: usize,
    dirty: bool,
    statut: String,
}

impl NotesState {
    pub(crate) fn new(home: usize) -> Self {
        let mut state = Self {
            home, chemin: None, texte: String::new(), curseur: 0, defilement: 0,
            dirty: false, statut: "RAMFS : non persistant apres redemarrage".to_string(),
        };
        state.reload();
        state
    }

    /// Ouvre `chemin` (un fichier existant, ou a creer a l'enregistrement).
    pub(crate) fn ouvre(home: usize, chemin: &str) -> Self {
        let mut state = Self {
            home, chemin: Some(String::from(chemin)), texte: String::new(), curseur: 0,
            defilement: 0, dirty: false, statut: String::new(),
        };
        state.reload();
        state
    }

    /// Nom du fichier edite, pour le titre de la fenetre.
    pub(crate) fn nom(&self) -> String {
        match &self.chemin {
            Some(c) => String::from(c.rsplit('/').next().unwrap_or(c)),
            None => String::from(FICHIER),
        }
    }

    /// `(dossier, nom, durable)` du fichier edite. `durable` : il est sous
    /// `/persist`, et un enregistrement est une transaction disque.
    fn emplacement(&self, fs: &ramfs::FileSystem) -> Result<(usize, String, bool), &'static str> {
        match &self.chemin {
            Some(c) => {
                // Chemin canonique avant resolution : le client ne peut pas
                // ouvrir un fichier via un chemin ambigu ou echappant a '/'.
                let absolu = crate::fs::vfs_chemins::normalise_absolu(c)
                    .map_err(|_| "chemin absolu invalide")?;
                let (parent, nom) = fs.resolve_parent_name(&absolu, 0)
                    .ok_or("chemin invalide")?;
                Ok((parent, String::from(nom), fs.sous_persistant(parent)))
            }
            None => {
                let durable = persistance::zone_disponible();
                let dossier = if durable {
                    fs.resolve(persistance::RACINE, 0).ok_or("/persist absent")?
                } else { self.home };
                Ok((dossier, String::from(FICHIER), durable))
            }
        }
    }

    /// Le fichier edite est-il persistant ?
    pub(crate) fn durable(&self) -> bool {
        let fs = ramfs::fs();
        self.emplacement(&fs).map_or(false, |(_, _, d)| d)
    }

    fn reload(&mut self) {
        let mut durable = false;
        let loaded: Result<Option<String>, &'static str> = (|| {
            let fs = ramfs::fs();
            let (dossier, nom, d) = self.emplacement(&fs)?;
            durable = d;
            if dossier >= ramfs::MAX_NODES || !fs.nodes[dossier].used
                || fs.nodes[dossier].kind != NodeKind::Dir { return Err("dossier invalide"); }
            if !fs.can(dossier, PERM_X) { return Err("traversee interdite"); }
            let Some(idx) = fs.find_child(dossier, &nom) else { return Ok(None); };
            if fs.nodes[idx].kind != NodeKind::File || !fs.can(idx, PERM_R) {
                return Err("lecture interdite ou type invalide");
            }
            if fs.nodes[idx].content.len() > LIMITE { return Err("fichier trop volumineux"); }
            let text = core::str::from_utf8(&fs.nodes[idx].content)
                .map_err(|_| "fichier non UTF-8")?;
            Ok(Some(text.to_string()))
        })();
        match loaded {
            Ok(Some(text)) => {
                self.texte = text;
                self.curseur = self.texte.len();
                self.defilement = 0;
                self.dirty = false;
                self.statut = if durable { format!("{} ouvert (persistant)", self.nom()) }
                    else { format!("{} ouvert (RAMFS, volatile)", self.nom()) };
            }
            Ok(None) => {
                self.texte.clear(); self.curseur = 0; self.defilement = 0;
                self.dirty = false;
                self.statut = if durable { format!("Nouveau document {} (persistant)", self.nom()) }
                    else { format!("Nouveau document {} (RAMFS, volatile)", self.nom()) };
            }
            Err(err) => self.statut = format!("Ouverture refusee : {}", err),
        }
    }

    fn save(&mut self) {
        if self.texte.len() > LIMITE {
            self.statut = "Document trop volumineux".to_string();
            return;
        }
        // Never hold the RAMFS lock while calling synchronise() (it snapshots
        // RAMFS internally). This lock ordering prevents a GUI deadlock.
        let mut durable = false;
        let result: Result<usize, &'static str> = (|| {
            let mut fs = ramfs::fs();
            let (dossier, nom, d) = self.emplacement(&fs)?;
            durable = d;
            if dossier >= ramfs::MAX_NODES || !fs.nodes[dossier].used
                || fs.nodes[dossier].kind != NodeKind::Dir { return Err("dossier invalide"); }
            if !fs.can(dossier, PERM_W | PERM_X) { return Err("ecriture repertoire interdite"); }
            let idx = match fs.find_child(dossier, &nom) {
                Some(idx) => {
                    if fs.nodes[idx].kind != NodeKind::File { return Err("la cible n'est pas un fichier"); }
                    if !fs.can(idx, PERM_W) { return Err("ecriture fichier interdite"); }
                    idx
                }
                None => fs.touch_at(dossier, &nom)?,
            };
            if !fs.write_node_bytes(idx, self.texte.as_bytes()) { return Err("RAMFS refuse l'ecriture"); }
            Ok(idx)
        })();
        match result {
            Ok(idx) => {
                // Acknowledgement reflects a completed disk transaction, not
                // merely the RAMFS write. -1 = failed; -2 = no volume.
                let sync_result = if durable { persistance::synchronise() }
                    else { persistance::SANS_ZONE };
                let persisted = durable && sync_result >= 0;
                self.dirty = !persisted && durable;
                self.statut = if persisted {
                    format!("{} enregistre sur le disque : {} octets", self.nom(), self.texte.len())
                } else if durable {
                    "ECHEC sync disque : modifications non durables".to_string()
                } else {
                    format!("Sauve en RAMFS : {} octets (volatile)", self.texte.len())
                };
                crate::serial_println!(
                    "BOUCHAUD_PRODUCT_NOTES_SAVED node={} bytes={} durable={} sync_result={}",
                    idx, self.texte.len(), persisted as u8, sync_result,
                );
            }
            Err(err) => {
                self.statut = format!("ECHEC sauvegarde : {}", err);
                crate::serial_println!("BOUCHAUD_PRODUCT_NOTES_SAVE_FAILED reason={}", err);
            }
        }
    }

    fn insert(&mut self, s: &str) {
        if self.texte.len().saturating_add(s.len()) > LIMITE {
            self.statut = "Limite document atteinte".to_string();
            return;
        }
        self.texte.insert_str(self.curseur, s);
        self.curseur += s.len();
        self.dirty = true;
        self.sync_scroll();
    }

    fn sync_scroll(&mut self) {
        let row = self.texte[..self.curseur].bytes().filter(|b| *b == b'\n').count();
        if row < self.defilement { self.defilement = row; }
        if row >= self.defilement + LIGNES_VISIBLES {
            self.defilement = row - LIGNES_VISIBLES + 1;
        }
    }

    fn previous(&self) -> usize {
        self.texte[..self.curseur].char_indices().last().map_or(0, |(p, _)| p)
    }
    fn next(&self) -> usize {
        if self.curseur == self.texte.len() { return self.curseur; }
        self.curseur + self.texte[self.curseur..].chars().next().map_or(0, char::len_utf8)
    }
    fn line_start(&self) -> usize {
        self.texte[..self.curseur].rfind('\n').map_or(0, |pos| pos + 1)
    }
    fn line_end(&self) -> usize {
        self.texte[self.curseur..].find('\n').map_or(self.texte.len(), |pos| self.curseur + pos)
    }
    fn vertical(&mut self, down: bool) {
        let start = self.line_start();
        let col = self.texte[start..self.curseur].chars().count();
        let destination = if down {
            let end = self.line_end();
            if end == self.texte.len() { return; }
            let next_start = end + 1;
            let next_end = self.texte[next_start..].find('\n')
                .map_or(self.texte.len(), |p| next_start + p);
            (next_start, next_end)
        } else {
            if start == 0 { return; }
            let prev_end = start - 1;
            let prev_start = self.texte[..prev_end].rfind('\n').map_or(0, |p| p + 1);
            (prev_start, prev_end)
        };
        let (s, e) = destination;
        self.curseur = self.texte[s..e].char_indices().nth(col)
            .map_or(e, |(p, _)| s + p);
        self.sync_scroll();
    }
}

pub(crate) fn on_key(state: &mut NotesState, key: Key) -> bool {
    match key {
        Key::Fonction(2) => state.save(),
        Key::Fonction(3) => {
            if state.dirty {
                state.statut = "Modifications non sauvegardees : F2 avant F3".to_string();
            } else { state.reload(); }
        }
        Key::Enter => state.insert("\n"),
        Key::Tab => state.insert("    "),
        Key::Char(byte) if byte >= 32 && byte != 127 => {
            let mut buf = [0u8; 4];
            let ch = (byte as char).encode_utf8(&mut buf);
            state.insert(ch);
        }
        Key::Backspace => {
            if state.curseur != 0 {
                let start = state.previous();
                state.texte.replace_range(start..state.curseur, "");
                state.curseur = start;
                state.dirty = true;
                state.sync_scroll();
            }
        }
        Key::Delete => {
            let end = state.next();
            if end != state.curseur {
                state.texte.replace_range(state.curseur..end, "");
                state.dirty = true;
            }
        }
        Key::Left => { state.curseur = state.previous(); state.sync_scroll(); }
        Key::Right => { state.curseur = state.next(); state.sync_scroll(); }
        Key::Home => { state.curseur = state.line_start(); state.sync_scroll(); }
        Key::End => { state.curseur = state.line_end(); state.sync_scroll(); }
        Key::Up => state.vertical(false),
        Key::Down => state.vertical(true),
        Key::PageUp => state.defilement = state.defilement.saturating_sub(LIGNES_VISIBLES),
        Key::PageDown => {
            let count = state.texte.bytes().filter(|b| *b == b'\n').count();
            state.defilement = state.defilement.saturating_add(LIGNES_VISIBLES).min(count);
        }
        _ => {}
    }
    false
}

pub(crate) fn click(state: &mut NotesState, x: i32, y: i32) {
    if !(8..=45).contains(&y) { return; }
    if (10..115).contains(&x) { state.save(); }
    else if (120..245).contains(&x) {
        if state.dirty { state.statut = "F2 pour sauvegarder avant de recharger".to_string(); }
        else { state.reload(); }
    }
}

pub(crate) fn draw(state: &NotesState, bx: usize, by: usize, bw: usize, bh: usize) {
    if bw < 40 || bh < 90 { return; }
    fb::fill_rect_rgb(bx, by, bw, bh, 0x101827);
    fb::fill_rect_rgb(bx, by, bw, 46, 0x1c2a3d);
    fb::fill_rect_rgb(bx + 10, by + 8, 105, 30, 0x2463b8);
    fb::fill_rect_rgb(bx + 120, by + 8, 125, 30, 0x33465e);
    fb::draw_text_prop(bx + 18, by + 15, "F2  Sauver", 0xffffff, 13.0, true);
    fb::draw_text_prop(bx + 130, by + 15, "F3  Recharger", 0xffffff, 13.0, true);
    let marque = if state.dirty { "NON SAUVE" } else if state.durable() { "PERSIST" } else { "RAMFS" };
    fb::draw_text_prop(bx + 255, by + 16, marque, 0xf7c569, 12.0, true);
    let zone_haut = by + 53;
    let max = ((bh.saturating_sub(92)) / HAUTEUR_LIGNE).min(LIGNES_VISIBLES);
    let mut y = zone_haut;
    let mut start = 0usize;
    for (row, line) in state.texte.split('\n').enumerate() {
        if row >= state.defilement && row < state.defilement + max {
            fb::draw_text_prop(bx + 12, y, &format!("{:03}", row + 1), 0x71849d, 12.0, false);
            fb::draw_text_prop(bx + 62, y, clip(line, bw.saturating_sub(82) / 7), 0xe2e8f0, 14.0, false);
            if state.curseur >= start && state.curseur <= start + line.len() {
                let prefix = &state.texte[start..state.curseur];
                let cx = bx + 62 + fb::text_width(prefix, 14.0, false);
                if cx < bx + bw.saturating_sub(3) {
                    fb::fill_rect_rgb(cx, y, 2, 17, 0x67e8f9);
                }
            }
            y += HAUTEUR_LIGNE;
        }
        start += line.len() + 1;
        if row >= state.defilement + max { break; }
    }
    let foot = by + bh - 30;
    fb::fill_rect_rgb(bx, foot, bw, 30, 0x1c2a3d);
    fb::draw_text_prop(bx + 10, foot + 7,
        clip(&state.statut, bw.saturating_sub(24) / 7), 0xf7c569, 12.0, false);
}
