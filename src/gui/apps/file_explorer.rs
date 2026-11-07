//! Fichiers Atelier (BOUCHAUD_ATELIER_FICHIERS_V1).
//!
//! Barre de navigation (precedent, suivant, parent, fil d'Ariane, vue), barre
//! laterale (Favoris, Emplacements), contenu en grille ou en liste, barre
//! d'etat, menu contextuel. Theme Atelier clair, icones et glyphes rendus par
//! la fabrique d'assets.
//!
//! # Ce qui n'a PAS change
//!
//! Le systeme de fichiers. On lit le RAMFS comme avant, on verifie les droits
//! comme avant (`can(.., PERM_X)` pour traverser), on n'ecrit RIEN dedans. Les
//! raccourcis Documents, Images et Telechargements designent des dossiers de
//! l'accueil : s'ils n'existent pas, ils s'affichent absents -- ils ne sont
//! pas crees ici.
//!
//! # Une seule definition par geometrie
//!
//! Toutes les boites viennent de `atelier::fichiers::Geometrie`, la meme pour
//! le peintre, le clic, le survol et le defilement (tests hote dans
//! `tools/gui/test_atelier.rs`).

use crate::fs::ramfs;
use crate::gui::atelier::{assets as a, dock::Boite, fichiers as f, jetons as j, matiere};
use crate::gui::framebuffer as fb;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

// Product M1 : historique de navigation par fenetre, borne a 32 entrees. Il
// vit dans le module pur `atelier::fichiers`, ou le harnais hote l'eprouve.
pub(crate) use crate::gui::atelier::fichiers::Historique as History;

/// Etat d'interface d'une fenetre Fichiers (rien qui touche au stockage).
pub(crate) struct Ui {
    pub vue: f::Vue,
    /// Dernier clic sur un element : (noeud, instant). Deux clics sur le meme
    /// noeud en moins de `DOUBLE_CLIC_MS` ouvrent.
    pub dernier_clic: Option<(usize, u64)>,
    /// Cible survolee (contrat du survol : la meme que le clic).
    pub survol: f::Cible,
    /// Menu contextuel ouvert : position (relative a la zone utile), element
    /// vise, entree survolee.
    pub menu: Option<Menu>,
    /// Message de la barre d'etat (resultat d'une action), efface a la
    /// navigation.
    pub message: Option<String>,
}

#[derive(Clone, Copy)]
pub(crate) struct Menu {
    pub x: i32,
    pub y: i32,
    pub element: Option<usize>,
    pub dossier: bool,
    pub texte: bool,
    pub survol: Option<usize>,
}

impl Ui {
    pub(crate) fn new() -> Self {
        Self { vue: f::Vue::Grille, dernier_clic: None, survol: f::Cible::Rien, menu: None, message: None }
    }
}

const DOUBLE_CLIC_MS: u64 = 500;
const CORPS: f32 = 13.0;
const CORPS_PETIT: f32 = 12.0;

// ─── Modele ────────────────────────────────────────────────────────────────

/// Les elements du dossier `cur`, tries. Le verrou du RAMFS n'est tenu que
/// le temps de la copie : rien d'autre n'est pris dessous (voir le correctif
/// LOCKDEP du Moniteur).
pub(crate) fn liste(cur: usize) -> Vec<f::Entree> {
    let mut v = Vec::new();
    {
        let fs = ramfs::fs();
        for i in 0..ramfs::MAX_NODES {
            let n = &fs.nodes[i];
            if n.used && i != cur && n.parent == cur {
                v.push(f::Entree { nom: n.name_str().into(), dossier: n.kind == ramfs::NodeKind::Dir,
                    octets: n.content.len(), noeud: i });
            }
        }
    }
    f::trie(&mut v);
    v
}

/// Dossier traversable ?
fn dossier_ouvrable(fs: &ramfs::FileSystem, idx: usize) -> bool {
    idx < ramfs::MAX_NODES && fs.nodes[idx].used && fs.nodes[idx].kind == ramfs::NodeKind::Dir
        && fs.can(idx, ramfs::PERM_X)
}

/// Les ancetres de `cur`, de la racine a lui : (noeud, libelle).
fn ancetres(cur: usize) -> Vec<(usize, String)> {
    let fs = ramfs::fs();
    let mut v = Vec::new();
    let mut i = cur;
    for _ in 0..64 {
        if i == 0 || i >= ramfs::MAX_NODES { break; }
        v.push((i, String::from(fs.nodes[i].name_str())));
        i = fs.nodes[i].parent;
    }
    v.push((0, String::from("Racine")));
    v.reverse();
    v
}

/// Le dossier que designe le raccourci `i`, s'il existe et se traverse.
fn dossier_raccourci(i: usize) -> Option<usize> {
    let (_, chemin, _) = f::RACCOURCIS[i];
    // BOUCHAUD_DOSSIERS_PERSONNELS_V1 : `~` est le dossier personnel
    // (persistant quand la machine a une zone), plus l'accueil du compte.
    let accueil = crate::fs::dossiers::dossier_personnel();
    let absolu: String = if let Some(reste) = chemin.strip_prefix('~') {
        format!("{}{}", accueil.trim_end_matches('/'), reste)
    } else {
        String::from(chemin)
    };
    // Notes et Fichiers partagent exactement la meme politique de chemins.
    // Un raccourci mal forme ne peut pas contourner les controles du VFS.
    let canon = crate::fs::vfs_chemins::normalise_absolu(&absolu).ok()?;
    let fs = ramfs::fs();
    let idx = fs.resolve(&canon, 0)?;
    dossier_ouvrable(&fs, idx).then_some(idx)
}

fn chemin(idx: usize) -> String {
    ramfs::path_string(&ramfs::fs(), idx)
}

/// Le fil d'Ariane de `cur`, place dans la barre : (segment, boite relative),
/// l'ellipse eventuelle, et les libelles.
fn ariane(g: &f::Geometrie, cur: usize) -> (Vec<(usize, Boite)>, Option<Boite>, Vec<(usize, String)>) {
    let segs = ancetres(cur);
    let dernier = segs.len().saturating_sub(1);
    let largeurs: Vec<i32> = segs.iter().enumerate()
        .map(|(k, (_, nom))| fb::text_width(nom, CORPS, k == dernier) as i32 + 16)
        .collect();
    let (boites, ellipse) = f::segments(g.zone_ariane(), &largeurs);
    (boites, ellipse, segs)
}

// ─── Peinture ──────────────────────────────────────────────────────────────

fn abs(b: Boite, bx: usize, by: usize) -> Boite {
    Boite { x: b.x + bx as i32, y: b.y + by as i32, w: b.w, h: b.h }
}

fn rect(b: Boite) -> crate::gui::windowing::Rect {
    crate::gui::windowing::Rect::new(b.x, b.y, b.w.max(0) as u32, b.h.max(0) as u32)
}

/// `(x, y, l, h)` d'une boite, pour `fb::decoupe_touche`.
fn u(b: Boite) -> (usize, usize, usize, usize) {
    (b.x.max(0) as usize, b.y.max(0) as usize, b.w.max(0) as usize, b.h.max(0) as usize)
}

fn texte(x: i32, y: i32, s: &str, couleur: u32, corps: f32, gras: bool) {
    fb::draw_text_prop(x.max(0) as usize, y.max(0) as usize, s, couleur, corps, gras);
}

/// Coupe `s` pour tenir en `largeur` pixels, avec une ellipse.
fn tient(s: &str, largeur: i32, corps: f32, gras: bool) -> String {
    if fb::text_width(s, corps, gras) as i32 <= largeur {
        return String::from(s);
    }
    let mut out = String::new();
    for c in s.chars() {
        out.push(c);
        if fb::text_width(&out, corps, gras) as i32 + 10 > largeur {
            out.pop();
            break;
        }
    }
    out.push('\u{2026}');
    out
}

fn bouton(b: Boite, glyphe: &'static [u8], actif: bool, survole: bool, choisi: bool) {
    if choisi {
        matiere::pave(rect(b), 8, j::ACCENT, 0x2a);
    } else if survole && actif {
        matiere::pave(rect(b), 8, j::SURVOL, j::SURVOL_OPACITE);
    }
    let couleur = if !actif { j::ENCRE_TENUE } else if choisi { j::ACCENT } else { j::ENCRE_DOUCE };
    a::pose_masque(glyphe, (b.x + (b.w - 16) / 2) as usize, (b.y + (b.h - 16) / 2) as usize,
        couleur, if actif { 0xff } else { 0x90 });
}

/// Dessine Fichiers dans la zone utile `(bx, by, bw, bh)`.
pub(crate) fn draw(cur: usize, scroll: i32, selected: Option<usize>, history: &History, ui: &Ui,
                   bx: usize, by: usize, bw: usize, bh: usize) {
    let t0 = crate::kernel::timer::monotonic_ns();
    let complet = { let (cx0, cy0, cx1, cy1) = fb::clip_rect();
        cx0 <= bx && cy0 <= by && cx1 >= bx + bw && cy1 >= by + bh };
    let g = f::Geometrie::new(bw as i32, bh as i32);

    // BOUCHAUD_GFX_CULLING_AMONT_V1
    //
    // Les bandes sont independantes, et chacune coute autre chose que des
    // pixels : le contenu parcourt le RAMFS et alloue une chaine par entree,
    // la barre d'etat le reparcourt, la barre de navigation remonte les
    // ancetres et mesure leurs noms. Un degat ne touche presque jamais tout.

    // ── Barre de navigation ──
    let barre = abs(g.barre(), bx, by);
    if { let (x, y, l, h) = u(barre); fb::decoupe_touche(x, y, l, h) } {
        fb::fill_rect_rgb(barre.x as usize, barre.y as usize, barre.w as usize, barre.h as usize, j::TITRE_ACTIF);
        fb::fill_rect_rgb(barre.x as usize, (barre.y + barre.h - 1) as usize, barre.w as usize, 1, j::SEPARATEUR);
        let s = ui.survol;
        bouton(abs(g.bouton(f::Cible::Retour), bx, by), a::SYMBOLE_PRECEDENT_16,
            history.peut_reculer(), s == f::Cible::Retour, false);
        bouton(abs(g.bouton(f::Cible::Avance), bx, by), a::SYMBOLE_SUIVANT_16,
            history.peut_avancer(), s == f::Cible::Avance, false);
        bouton(abs(g.bouton(f::Cible::Parent), bx, by), a::SYMBOLE_PARENT_16,
            cur != 0, s == f::Cible::Parent, false);
        bouton(abs(g.bouton(f::Cible::VueGrille), bx, by), a::SYMBOLE_GRILLE_16, true,
            s == f::Cible::VueGrille, ui.vue == f::Vue::Grille);
        bouton(abs(g.bouton(f::Cible::VueListe), bx, by), a::SYMBOLE_LISTE_16, true,
            s == f::Cible::VueListe, ui.vue == f::Vue::Liste);

        // Fil d'Ariane : champ discret, segments cliquables, le dossier
        // courant en gras.
        let zone = abs(g.zone_ariane(), bx, by);
        if zone.w > 0 {
            matiere::pave(rect(zone), 8, 0xffffff, 0xff);
            matiere::filet(rect(zone), 8, j::FILET, j::FILET_OPACITE);
            let (boites, ellipse, segs) = ariane(&g, cur);
            let dernier = segs.len().saturating_sub(1);
            for (k, (i, b)) in boites.iter().enumerate() {
                let b = abs(*b, bx, by);
                if k > 0 {
                    texte(b.x - f::SEPARATEUR_L + 5, b.y + 6, "\u{203a}", j::ENCRE_TENUE, CORPS, false);
                }
                if s == f::Cible::Segment(*i) && *i != dernier {
                    matiere::pave(rect(Boite { y: b.y + 3, h: b.h - 6, ..b }), 6, j::SURVOL, j::SURVOL_OPACITE);
                }
                let gras = *i == dernier;
                let nom = tient(&segs[*i].1, b.w - 12, CORPS, gras);
                texte(b.x + 8, b.y + 6, &nom, if gras { j::ENCRE } else { j::ENCRE_DOUCE }, CORPS, gras);
            }
            if let Some(e) = ellipse {
                let e = abs(e, bx, by);
                texte(e.x + 4, e.y + 6, "\u{2026}", j::ENCRE_TENUE, CORPS, false);
            }
        }
    }

    // ── Barre laterale ──
    let lat = abs(g.laterale(), bx, by);
    if lat.w > 0 && { let (x, y, l, h) = u(lat); fb::decoupe_touche(x, y, l, h) } {
        fb::fill_rect_rgb(lat.x as usize, lat.y as usize, lat.w as usize, lat.h as usize, 0xf2f3f8);
        fb::fill_rect_rgb((lat.x + lat.w - 1) as usize, lat.y as usize, 1, lat.h as usize, j::SEPARATEUR);
        for (s, titre) in f::SECTIONS.iter().enumerate() {
            texte(lat.x + 18, g.titre_section(s) + by as i32 + 8, titre, j::ENCRE_TENUE, CORPS_PETIT, true);
        }
        for (i, (libelle, _, _)) in f::RACCOURCIS.iter().enumerate() {
            let b = abs(g.raccourci(i), bx, by);
            let dest = dossier_raccourci(i);
            let courant = dest == Some(cur);
            if courant {
                matiere::pave(rect(b), 8, j::ACCENT, 0x24);
            } else if ui.survol == f::Cible::Raccourci(i) && dest.is_some() {
                matiere::pave(rect(b), 8, j::SURVOL, j::SURVOL_OPACITE);
            }
            let glyphe = [a::SYMBOLE_ACCUEIL_16, a::SYMBOLE_DOCUMENTS_16, a::SYMBOLE_IMAGES_16,
                a::SYMBOLE_TELECHARGEMENTS_16, a::SYMBOLE_RACINE_16, a::SYMBOLE_GRILLE_16,
                a::SYMBOLE_DOSSIER_16][i];
            let (encre, glyphe_encre) = match (dest.is_some(), courant) {
                (false, _) => (j::ENCRE_TENUE, j::ENCRE_TENUE),
                (true, true) => (j::ENCRE, j::ACCENT),
                (true, false) => (j::ENCRE, j::ENCRE_DOUCE),
            };
            a::pose_masque(glyphe, (b.x + 10) as usize, (b.y + 7) as usize, glyphe_encre, 0xff);
            let mut place = b.w - 34 - 8;
            if dest.is_none() {
                // Absent : dit, pas cree. Le raccourci ne reagit pas.
                let w = fb::text_width("absent", CORPS_PETIT - 1.0, false) as i32;
                texte(b.x + b.w - w - 8, b.y + 8, "absent", j::ENCRE_TENUE, CORPS_PETIT - 1.0, false);
                place -= w + 6;
            }
            texte(b.x + 34, b.y + 7, &tient(libelle, place, CORPS, courant), encre, CORPS, courant);
        }
    }

    // ── Contenu ──
    let contenu = abs(g.contenu(), bx, by);
    let mut entrees: Option<Vec<f::Entree>> = None;
    if { let (x, y, l, h) = u(contenu); fb::decoupe_touche(x, y, l, h) } {
        fb::fill_rect_rgb(contenu.x as usize, contenu.y as usize, contenu.w as usize, contenu.h as usize, 0xffffff);
        let e = liste(cur);
        let defil = (scroll.max(0) as usize).min(g.defilement_max(e.len(), ui.vue));
        match ui.vue {
            f::Vue::Grille => peins_grille(&g, &e, defil, selected, ui, bx, by),
            f::Vue::Liste => peins_liste(&g, &e, defil, selected, ui, bx, by),
        }
        if e.is_empty() {
            let msg = "Ce dossier est vide";
            let w = fb::text_width(msg, CORPS, false) as i32;
            texte(contenu.x + (contenu.w - w) / 2, contenu.y + contenu.h / 2 - 8, msg, j::ENCRE_TENUE, CORPS, false);
        }
        entrees = Some(e);
    }

    // ── Barre d'etat ──
    let etat = abs(g.etat(), bx, by);
    if { let (x, y, l, h) = u(etat); fb::decoupe_touche(x, y, l, h) } {
        fb::fill_rect_rgb(etat.x as usize, etat.y as usize, etat.w as usize, etat.h as usize, j::TITRE_ACTIF);
        fb::fill_rect_rgb(etat.x as usize, etat.y as usize, etat.w as usize, 1, j::SEPARATEUR);
        let e = match entrees.take() { Some(e) => e, None => liste(cur) };
        let nombre = e.len();
        let gauche = if let Some(m) = &ui.message {
            m.clone()
        } else if let Some(s) = selected.and_then(|s| e.get(s)) {
            let genre = f::genre(&s.nom, s.dossier);
            if s.dossier {
                format!("\u{ab} {} \u{bb} \u{2014} {}", s.nom, f::libelle(genre))
            } else {
                format!("\u{ab} {} \u{bb} \u{2014} {}, {}", s.nom, f::libelle(genre), f::taille_lisible(s.octets))
            }
        } else {
            format!("{} \u{e9}l\u{e9}ment{}", nombre, if nombre > 1 { "s" } else { "" })
        };
        texte(etat.x + 14, etat.y + 6, &tient(&gauche, etat.w - 160, CORPS_PETIT, false),
            j::ENCRE_DOUCE, CORPS_PETIT, false);
        // Le stockage dit ce qu'il est, dossier par dossier : sous `/persist`,
        // le disque (ecriture differee, BOUCHAUD_ECRITURE_DIFFEREE_V1) ;
        // ailleurs, le RAMFS, qui ne survit pas au redemarrage.
        let persistant = ramfs::fs().sous_persistant(cur);
        let droite = if persistant { "Sur le disque \u{b7} persistant" } else { "RAMFS \u{b7} non persistant" };
        let w = fb::text_width(droite, CORPS_PETIT, false) as i32;
        texte(etat.x + etat.w - w - 14, etat.y + 6, droite, j::ENCRE_TENUE, CORPS_PETIT, false);
    }

    // ── Menu contextuel, par-dessus le reste ──
    if let Some(m) = &ui.menu {
        let actions = f::actions(m.element.is_some(), m.dossier, m.texte);
        let boite = abs(g.menu(m.x, m.y, actions.len()), bx, by);
        if { let (x, y, l, h) = u(Boite { x: boite.x - 6, y: boite.y - 6, w: boite.w + 12, h: boite.h + 12 }); fb::decoupe_touche(x, y, l, h) } {
            matiere::ombre_douce(rect(boite), 10, 6, j::OMBRE_FLOTTANT);
            matiere::pave(rect(boite), 10, 0xffffff, 0xff);
            matiere::filet(rect(boite), 10, j::FILET, j::FILET_OPACITE);
            for (k, act) in actions.iter().enumerate() {
                let e = f::Geometrie::entree_menu(boite, k);
                if m.survol == Some(k) {
                    matiere::pave(rect(e), 6, j::ACCENT, 0xff);
                }
                let encre = if m.survol == Some(k) { 0xffffff } else { j::ENCRE };
                let glyphe = match act {
                    f::Action::Ouvrir => a::SYMBOLE_DOSSIER_16,
                    f::Action::OuvrirNotes => a::SYMBOLE_DOCUMENTS_16,
                    f::Action::CopierChemin => a::SYMBOLE_COPIER_16,
                    f::Action::Proprietes => a::SYMBOLE_DOCUMENTS_16,
                    f::Action::VueGrille => a::SYMBOLE_GRILLE_16,
                    f::Action::VueListe => a::SYMBOLE_LISTE_16,
                };
                a::pose_masque(glyphe, (e.x + 10) as usize, (e.y + 7) as usize, encre, 0xff);
                texte(e.x + 34, e.y + 7, f::libelle_action(*act, m.element.is_some()), encre, CORPS, false);
            }
        }
    }
    if complet {
        mesure_peinture(t0, bw, bh);
    }
}

fn icone(genre: f::Genre, grand: bool, x: i32, y: i32) {
    a::pose(a::TYPES_FICHIERS[genre as usize][grand as usize], x.max(0) as usize, y.max(0) as usize);
}

fn peins_grille(g: &f::Geometrie, e: &[f::Entree], defil: usize, selected: Option<usize>, ui: &Ui,
                bx: usize, by: usize) {
    for (i, entree) in e.iter().enumerate() {
        let b = g.element(i, f::Vue::Grille, defil);
        if b.w == 0 {
            continue;
        }
        let c = abs(b, bx, by);
        // BOUCHAUD_GFX_CULLING_AMONT_V1 : une cellule hors du degat ne coute
        // que son tour de boucle -- ni icone posee, ni libelle mesure.
        if !{ let (x, y, l, h) = u(c); fb::decoupe_touche(x, y, l, h) } { continue }
        let interieur = Boite { x: c.x + 4, y: c.y + 2, w: c.w - 8, h: c.h - 4 };
        let choisi = selected == Some(i);
        if choisi {
            matiere::pave(rect(interieur), 10, j::ACCENT, 0x26);
            matiere::filet(rect(interieur), 10, j::ACCENT, 0x70);
        } else if ui.survol == f::Cible::Element(i) {
            matiere::pave(rect(interieur), 10, j::SURVOL, j::SURVOL_OPACITE);
        }
        let genre = f::genre(&entree.nom, entree.dossier);
        icone(genre, true, c.x + (c.w - f::ICONE_GRILLE) / 2, c.y + 10);
        let nom = tient(&entree.nom, c.w - 12, CORPS_PETIT, false);
        let w = fb::text_width(&nom, CORPS_PETIT, false) as i32;
        texte(c.x + (c.w - w) / 2, c.y + 10 + f::ICONE_GRILLE + 10, &nom,
            if choisi { j::ACCENT } else { j::ENCRE }, CORPS_PETIT, choisi);
    }
}

fn peins_liste(g: &f::Geometrie, e: &[f::Entree], defil: usize, selected: Option<usize>, ui: &Ui,
               bx: usize, by: usize) {
    let c = abs(g.contenu(), bx, by);
    let col_type = c.x + c.w - 250;
    let col_taille = c.x + c.w - 24;
    // En-tete.
    let entete = Boite { x: c.x, y: c.y, w: c.w, h: f::ENTETE_LISTE_H };
    if { let (x, y, l, h) = u(entete); fb::decoupe_touche(x, y, l, h) } {
        texte(c.x + 50, c.y + 7, "Nom", j::ENCRE_TENUE, CORPS_PETIT, true);
        texte(col_type, c.y + 7, "Type", j::ENCRE_TENUE, CORPS_PETIT, true);
        let w = fb::text_width("Taille", CORPS_PETIT, true) as i32;
        texte(col_taille - w, c.y + 7, "Taille", j::ENCRE_TENUE, CORPS_PETIT, true);
        fb::fill_rect_rgb((c.x + 12) as usize, (c.y + f::ENTETE_LISTE_H - 1) as usize,
            (c.w - 24).max(0) as usize, 1, j::SEPARATEUR);
    }
    for (i, entree) in e.iter().enumerate() {
        let b = g.element(i, f::Vue::Liste, defil);
        if b.w == 0 {
            continue;
        }
        let r = abs(b, bx, by);
        if !{ let (x, y, l, h) = u(r); fb::decoupe_touche(x, y, l, h) } { continue }
        let ligne = Boite { x: r.x, y: r.y + 1, w: r.w, h: r.h - 2 };
        let choisi = selected == Some(i);
        if choisi {
            matiere::pave(rect(ligne), 6, j::ACCENT, 0x26);
        } else if ui.survol == f::Cible::Element(i) {
            matiere::pave(rect(ligne), 6, j::SURVOL, j::SURVOL_OPACITE);
        }
        let genre = f::genre(&entree.nom, entree.dossier);
        icone(genre, false, r.x + 14, r.y + 5);
        let nom = tient(&entree.nom, col_type - (r.x + 44) - 12, CORPS, false);
        texte(r.x + 44, r.y + 7, &nom, if choisi { j::ACCENT } else { j::ENCRE }, CORPS, choisi);
        texte(col_type, r.y + 8, f::libelle(genre), j::ENCRE_DOUCE, CORPS_PETIT, false);
        let taille = if entree.dossier { String::from("\u{2014}") } else { f::taille_lisible(entree.octets) };
        let w = fb::text_width(&taille, CORPS_PETIT, false) as i32;
        texte(col_taille - w, r.y + 8, &taille, j::ENCRE_DOUCE, CORPS_PETIT, false);
    }
}

// ─── Entrees ───────────────────────────────────────────────────────────────

/// Va dans `dest` si c'est un dossier traversable. Rend `true` si on y est
/// alle.
fn va(cur: &mut usize, scroll: &mut i32, selected: &mut Option<usize>, history: &mut History,
      ui: &mut Ui, dest: usize, note: bool) -> bool {
    let ok = dossier_ouvrable(&ramfs::fs(), dest);
    if !ok {
        ui.message = Some(String::from("Acc\u{e8}s refus\u{e9} ou dossier introuvable"));
        return false;
    }
    if note {
        history.visit(*cur, dest);
    }
    *cur = dest;
    *scroll = 0;
    *selected = None;
    ui.menu = None;
    ui.message = None;
    ui.dernier_clic = None;
    true
}

/// Ouvre l'element `i` : un dossier s'ouvre ; un fichier n'a pas encore
/// d'application associee, et la barre d'etat le dit.
fn ouvre(cur: &mut usize, scroll: &mut i32, selected: &mut Option<usize>, history: &mut History,
         ui: &mut Ui, e: &[f::Entree], i: usize) {
    let Some(entree) = e.get(i) else { return };
    if entree.dossier {
        let dest = entree.noeud;
        va(cur, scroll, selected, history, ui, dest, true);
    } else if f::editable_dans_notes(f::genre(&entree.nom, false), entree.octets) {
        // BOUCHAUD_NOTES_OUVRIR_V1 : le bureau ouvre une fenetre Notes sur ce
        // fichier au prochain tour.
        let p = chemin(entree.noeud);
        crate::gui::apps::demande_notes(p.clone());
        ui.message = Some(format!("\u{ab} {} \u{bb} ouvert dans Notes", entree.nom));
    } else {
        ui.message = Some(format!("\u{ab} {} \u{bb} : aucune application ne l'ouvre encore", entree.nom));
    }
}

fn agit(cur: &mut usize, scroll: &mut i32, selected: &mut Option<usize>, history: &mut History,
        ui: &mut Ui, e: &[f::Entree], menu: Menu, action: f::Action) {
    match action {
        f::Action::Ouvrir | f::Action::OuvrirNotes => {
            if let Some(i) = menu.element { ouvre(cur, scroll, selected, history, ui, e, i); }
        }
        f::Action::CopierChemin => {
            let noeud = menu.element.and_then(|i| e.get(i)).map_or(*cur, |x| x.noeud);
            let p = chemin(noeud);
            crate::gui::presse_papiers::ecrit(p.as_bytes());
            ui.message = Some(format!("Chemin copi\u{e9} : {}", p));
        }
        f::Action::Proprietes => {
            if let Some(entree) = menu.element.and_then(|i| e.get(i)) {
                let (mode, uid) = {
                    let fs = ramfs::fs();
                    (fs.nodes[entree.noeud].mode, fs.nodes[entree.noeud].uid)
                };
                let genre = f::genre(&entree.nom, entree.dossier);
                ui.message = Some(format!("{} \u{2014} {}{} \u{2014} droits {:03o}, propri\u{e9}taire {}",
                    entree.nom, f::libelle(genre),
                    if entree.dossier { String::new() } else { format!(", {}", f::taille_lisible(entree.octets)) },
                    mode, crate::users::name_of_uid(uid)));
            }
        }
        f::Action::VueGrille => { ui.vue = f::Vue::Grille; *scroll = 0; }
        f::Action::VueListe => { ui.vue = f::Vue::Liste; *scroll = 0; }
    }
}

/// Clic gauche en `(x, y)`, relatif a la zone utile `bw x bh`.
pub(crate) fn clic(cur: &mut usize, scroll: &mut i32, selected: &mut Option<usize>, history: &mut History,
                   ui: &mut Ui, bw: i32, bh: i32, x: i32, y: i32) {
    let g = f::Geometrie::new(bw, bh);
    let e = liste(*cur);
    // Un menu ouvert prend le clic : une entree s'execute, ailleurs il se ferme.
    if let Some(m) = ui.menu.take() {
        let actions = f::actions(m.element.is_some(), m.dossier, m.texte);
        let boite = g.menu(m.x, m.y, actions.len());
        if let Some(k) = f::Geometrie::entree_sous(boite, actions.len(), x, y) {
            agit(cur, scroll, selected, history, ui, &e, m, actions[k]);
        }
        return;
    }
    let defil = (*scroll).max(0) as usize;
    let (boites, _, segs) = ariane(&g, *cur);
    match g.cible(x, y, e.len(), ui.vue, defil, &boites) {
        f::Cible::Retour => {
            let fs = ramfs::fs();
            if let Some(prev) = history.back(*cur, |idx| dossier_ouvrable(&fs, idx)) {
                drop(fs);
                va(cur, scroll, selected, history, ui, prev, false);
            }
        }
        f::Cible::Avance => {
            let fs = ramfs::fs();
            if let Some(next) = history.forward(*cur, |idx| dossier_ouvrable(&fs, idx)) {
                drop(fs);
                va(cur, scroll, selected, history, ui, next, false);
            }
        }
        f::Cible::Parent => {
            if *cur != 0 {
                let parent = ramfs::fs().nodes[*cur].parent;
                va(cur, scroll, selected, history, ui, parent, true);
            }
        }
        f::Cible::Segment(i) => {
            if let Some((noeud, _)) = segs.get(i) {
                if *noeud != *cur { va(cur, scroll, selected, history, ui, *noeud, true); }
            }
        }
        f::Cible::VueGrille => { ui.vue = f::Vue::Grille; *scroll = 0; }
        f::Cible::VueListe => { ui.vue = f::Vue::Liste; *scroll = 0; }
        f::Cible::Raccourci(i) => match dossier_raccourci(i) {
            Some(dest) => { if dest != *cur { va(cur, scroll, selected, history, ui, dest, true); } }
            None => {
                ui.message = Some(format!("\u{ab} {} \u{bb} n'existe pas dans ce dossier personnel",
                    f::RACCOURCIS[i].0));
            }
        },
        f::Cible::Element(i) => {
            let maintenant = crate::kernel::timer::monotonic_ms();
            let noeud = e[i].noeud;
            let double = matches!(ui.dernier_clic,
                Some((n, t)) if n == noeud && maintenant.wrapping_sub(t) < DOUBLE_CLIC_MS);
            *selected = Some(i);
            ui.message = None;
            if double {
                ui.dernier_clic = None;
                ouvre(cur, scroll, selected, history, ui, &e, i);
            } else {
                ui.dernier_clic = Some((noeud, maintenant));
            }
        }
        f::Cible::Fond => { *selected = None; ui.message = None; ui.dernier_clic = None; }
        f::Cible::Rien => {}
    }
}

/// Clic droit en `(x, y)` : ouvre le menu contextuel de ce qui est dessous.
pub(crate) fn clic_droit(cur: usize, scroll: i32, selected: &mut Option<usize>, ui: &mut Ui,
                         bw: i32, bh: i32, x: i32, y: i32) {
    let g = f::Geometrie::new(bw, bh);
    if !g.contenu().contient(x, y) {
        ui.menu = None;
        return;
    }
    let e = liste(cur);
    let element = g.element_sous(x, y, e.len(), ui.vue, scroll.max(0) as usize);
    if element.is_some() {
        *selected = element;
    }
    let dossier = element.and_then(|i| e.get(i)).map_or(false, |x| x.dossier);
    let texte = element.and_then(|i| e.get(i))
        .map_or(false, |x| !x.dossier && f::editable_dans_notes(f::genre(&x.nom, false), x.octets));
    ui.menu = Some(Menu { x, y, element, dossier, texte, survol: None });
    ui.message = None;
}

/// Molette : defilement borne.
pub(crate) fn molette(cur: usize, scroll: &mut i32, ui: &mut Ui, bw: i32, bh: i32, delta: i32) {
    let g = f::Geometrie::new(bw, bh);
    let max = g.defilement_max(liste(cur).len(), ui.vue) as i32;
    *scroll = (*scroll - delta).clamp(0, max);
    // Le menu est ancre a un point du contenu, qui defile sous lui.
    ui.menu = None;
}

/// Clavier. Rend `true` si la touche a ete prise.
pub(crate) fn touche_clavier(cur: &mut usize, scroll: &mut i32, selected: &mut Option<usize>,
                             history: &mut History, ui: &mut Ui, bw: i32, bh: i32,
                             k: crate::gui::event::Key) -> bool {
    use crate::gui::event::Key;
    let g = f::Geometrie::new(bw, bh);
    let e = liste(*cur);
    let cols = if ui.vue == f::Vue::Grille { g.colonnes() } else { 1 };
    let delta = match k {
        Key::Left if ui.vue == f::Vue::Grille => Some((-1, 1)),
        Key::Right if ui.vue == f::Vue::Grille => Some((1, 1)),
        Key::Up => Some((-1, cols)),
        Key::Down => Some((1, cols)),
        _ => None,
    };
    if let Some((d, pas)) = delta {
        *selected = f::deplace_selection(*selected, e.len(), d, pas);
        if let Some(s) = *selected {
            *scroll = f::defilement_pour(s, (*scroll).max(0) as usize, &g, ui.vue) as i32;
        }
        ui.menu = None;
        ui.message = None;
        return true;
    }
    match k {
        Key::Enter => {
            if let Some(i) = *selected { ouvre(cur, scroll, selected, history, ui, &e, i); }
            true
        }
        Key::Backspace => {
            if *cur != 0 {
                let parent = ramfs::fs().nodes[*cur].parent;
                va(cur, scroll, selected, history, ui, parent, true);
            }
            true
        }
        _ => false,
    }
}

/// Echap : ferme le menu contextuel s'il est ouvert. Rend `true` si elle a
/// servi (la fenetre ne doit alors PAS se fermer).
pub(crate) fn echap(ui: &mut Ui) -> bool {
    ui.menu.take().is_some()
}

/// Survol en `(x, y)` : met a jour la cible survolee et rend les boites
/// (relatives) a recomposer si elle a change -- l'ancienne et la nouvelle.
pub(crate) fn survol(cur: usize, scroll: i32, ui: &mut Ui, bw: i32, bh: i32, x: i32, y: i32)
    -> Option<(Boite, Boite)>
{
    let g = f::Geometrie::new(bw, bh);
    if let Some(m) = ui.menu.as_mut() {
        let n = f::actions(m.element.is_some(), m.dossier, m.texte).len();
        let boite = g.menu(m.x, m.y, n);
        let nouveau = f::Geometrie::entree_sous(boite, n, x, y);
        if nouveau == m.survol {
            return None;
        }
        let zone = |k: Option<usize>| k.map_or(Boite { x: 0, y: 0, w: 0, h: 0 },
            |k| f::Geometrie::entree_menu(boite, k));
        let r = (zone(m.survol), zone(nouveau));
        m.survol = nouveau;
        return Some(r);
    }
    // Le contenu n'est lu que si le pointeur y est : survoler la barre ne
    // parcourt pas le RAMFS.
    let n = if g.contenu().contient(x, y) { liste(cur).len() } else { 0 };
    let (boites, _, _) = if g.barre().contient(x, y) {
        ariane(&g, cur)
    } else {
        (Vec::new(), None, Vec::new())
    };
    let nouveau = g.cible(x, y, n, ui.vue, scroll.max(0) as usize, &boites);
    if nouveau == ui.survol {
        return None;
    }
    let boite = |c: f::Cible| -> Boite {
        match c {
            f::Cible::Retour | f::Cible::Avance | f::Cible::Parent | f::Cible::VueGrille
            | f::Cible::VueListe => g.bouton(c),
            // Le fil d'Ariane entier : ses boites dependent des noms.
            f::Cible::Segment(_) => g.zone_ariane(),
            f::Cible::Raccourci(i) => g.raccourci(i),
            f::Cible::Element(i) => g.element(i, ui.vue, scroll.max(0) as usize),
            _ => Boite { x: 0, y: 0, w: 0, h: 0 },
        }
    };
    let r = (boite(ui.survol), boite(nouveau));
    ui.survol = nouveau;
    Some(r)
}

// ─── Mesure (BOUCHAUD_ATELIER_FICHIERS_MESURE_V1) ──────────────────────────
//
// Cout d'une peinture COMPLETE de la fenetre (toutes les bandes touchees par
// la decoupe), en microsecondes, publie toutes les 8 peintures completes.
static PEINTURES: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static NS_TOTAL: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static NS_MAX: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

fn mesure_peinture(t0: u64, bw: usize, bh: usize) {
    use core::sync::atomic::Ordering::Relaxed;
    let ns = crate::kernel::timer::monotonic_ns().saturating_sub(t0);
    let n = PEINTURES.fetch_add(1, Relaxed) + 1;
    let total = NS_TOTAL.fetch_add(ns, Relaxed) + ns;
    let max = NS_MAX.fetch_max(ns, Relaxed).max(ns);
    if n % 8 == 0 {
        crate::serial_println!("BOUCHAUD_ATELIER_FICHIERS_PEINTURE n={} zone={}x{} moy_us={} max_us={}",
            n, bw, bh, total / n / 1000, max / 1000);
    }
}
