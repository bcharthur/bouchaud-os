//! Widgets de rendu du bureau : fenetres, barre des taches, menu, curseur.

use crate::gui::apps;
use crate::gui::framebuffer as fb;
use crate::gui::window::{
    self, icon_rect, menu_rect, start_btn, taskbar_btn, App, Win,
    BAR_H, ICONS, MENU, MENU_HEADER_H, MENU_ITEM_H, TITLE_H,
};
use crate::arch::x86_64::{cpu, rtc, smp};
use crate::kernel::timer;
use crate::fs::ramfs;
use alloc::format;
use alloc::string::String;

// ─── Utilitaires couleur ───────────────────────────────────────────────────

fn lerp_color(c1: u32, c2: u32, t: usize, max: usize) -> u32 {
    let m = max.max(1) as i32;
    let t = t as i32;
    let ch = |shift: u32| -> u32 {
        let a = ((c1 >> shift) & 0xff) as i32;
        let b = ((c2 >> shift) & 0xff) as i32;
        ((a + (b - a) * t / m).clamp(0, 255)) as u32
    };
    (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

fn draw_circle(cx: usize, cy: usize, r: i32, color: u32) {
    for dy in -r..=r {
        for dx in -r..=r {
            if dx*dx + dy*dy <= r*r {
                let px = cx as i32 + dx;
                let py = cy as i32 + dy;
                if px >= 0 && py >= 0 && (px as usize) < fb::WIDTH && (py as usize) < fb::HEIGHT {
                    fb::pixel_rgb(px as usize, py as usize, color);
                }
            }
        }
    }
}

fn draw_circle_highlight(cx: usize, cy: usize, r: i32, base: u32) {
    draw_circle(cx, cy, r, base);
    // Top highlight arc
    for dx in -(r-1)..=(r-1) {
        let dy = -r + 1;
        let px = cx as i32 + dx; let py = cy as i32 + dy;
        if px >= 0 && py >= 0 && (px as usize) < fb::WIDTH && (py as usize) < fb::HEIGHT {
            fb::pixel_rgb(px as usize, py as usize, lerp_color(base, 0xffffff, 60, 100));
        }
    }
}

// ─── Bureau ────────────────────────────────────────────────────────────────

/// Dessine le fond du bureau, la barre du haut et toutes les fenetres visibles.
// BOUCHAUD_GUI_SCENE_CULLING_V1
//
// `draw_desktop` dessinait la scene entiere en un bloc. Le compositeur ne
// pouvait donc rien eviter : meme pour un rectangle de curseur de 16x16, il
// payait la lecture de l'horloge RTC, deux formatages de chaine et trois
// rasterisations TrueType de la barre du haut.
//
// Les morceaux sont desormais separes et adressables un par un. C'est
// `gui::scene` qui decide lesquels appeler, a partir de leurs bornes.
//
// La fonction d'origine reste, pour les appelants qui veulent tout : elle
// n'est plus utilisee par la boucle de composition.

/// Filigrane « Bouchaud OS », en bas au centre.
/// Texte du filigrane du bureau.
const FILIGRANE: &str = "Bouchaud OS";

/// Corps du filigrane, en pixels.
const FILIGRANE_CORPS: f32 = 34.0;

/// Interligne reserve au filigrane : jambages et debord d'antialiassage.
const FILIGRANE_HAUTEUR: usize = 44;

// BOUCHAUD_GUI_FILIGRANE_VECTORIEL_V1
//
// Le filigrane etait peint par `draw_text_rgb(.., scale = 2)`, c'est-a-dire la
// police BITMAP 8x8 du noyau, agrandie deux fois : des marches d'escalier a
// chaque diagonale, alors que le bureau dispose d'une DejaVu vectorielle
// antialiasee -- celle qui sert deja aux titres de fenetres et aux libelles.
//
// Le rectangle du calque ne peut plus etre une constante : la largeur d'un mot
// en police proportionnelle ne se devine pas. Il est MESURE, avec une marge
// pour l'antialiassage, et `draw_filigrane` part exactement du meme point.
fn filigrane_origine() -> (usize, usize) {
    let largeur = fb::text_width(FILIGRANE, FILIGRANE_CORPS, false);
    (
        (fb::WIDTH / 2).saturating_sub(largeur / 2),
        fb::HEIGHT.saturating_sub(FILIGRANE_HAUTEUR + 22),
    )
}

pub(crate) fn draw_filigrane() {
    let (x, y) = filigrane_origine();
    fb::draw_text_prop(x, y, FILIGRANE, 0x2f4468, FILIGRANE_CORPS, false);
}

/// Bornes du filigrane. Volontairement large : un calque qui deborde ses
/// bornes laisserait des trainees, l'inverse ne coute qu'un peu de travail.
pub(crate) fn filigrane_rect() -> (usize, usize, usize, usize) {
    let (x, y) = filigrane_origine();
    let largeur = fb::text_width(FILIGRANE, FILIGRANE_CORPS, false);
    // Marge : `draw_text_prop` pose les glyphes a partir de `x`, mais un `B`
    // peut deborder d'un pixel a gauche et l'antialiassage d'un autre.
    (x.saturating_sub(4), y, largeur + 8, FILIGRANE_HAUTEUR)
}

/// Fond d'ecran seul.
pub(crate) fn draw_fond() {
    draw_wallpaper();
}

/// Barre superieure seule.
pub(crate) fn draw_barre_haute() {
    draw_topbar();
}

/// Une icone du bureau.
pub(crate) fn draw_icone(index: usize) {
    draw_icon_at(index);
}

// BOUCHAUD_GUI_EMPREINTE_ICONE_V1
//
// CE QUE L'ICONE PEINT, par opposition a son rectangle.
//
// `icon_rect` fait 56 x 60. Le libelle, lui, est centre sur cette largeur mais
// n'y est pas contraint : `lx = (r.x + (r.w - lw) / 2).max(0)`. Des que le
// texte est plus large que l'icone — et « Calculatrice » l'est a 10 pixels —
// `lx` passe A GAUCHE de `r.x` et le texte deborde des deux cotes.
//
// Le calque annoncait `r.w + 6` et `r.h + 6`, un debord vers la droite et le
// bas seulement. Deux consequences, toutes deux visibles : un degat clippe
// exactement sur ces bornes tronque le libelle, et deplacer une icone laisse
// derriere elle les moities de texte qui sortaient des bornes.
//
// Ce que le calque annonce doit MAJORER ce qu'il peint. Cette fonction est donc
// la seule reponse a « ou une icone met-elle des pixels ? » : le carre, son
// ombre portee de 3 pixels, et le libelle avec la sienne d'un pixel.

/// Empreinte reelle de l'icone `index`, libelle compris.
pub(crate) fn empreinte_icone(index: usize) -> crate::gui::protocole::Rect {
    let r = icon_rect(index);
    let (vx, vy) = image_icone(index);
    let cote = crate::gui::icones::TAILLE_BUREAU as i32;
    let (lx, ly, lw) = libelle_icone(index);

    // L'union de TROIS choses : la cellule, l'image, et le libelle avec son
    // ombre portee d'un pixel. Le libelle est centre sur la cellule mais n'y
    // est pas contraint -- « Calculatrice » est plus large qu'elle --, donc il
    // deborde des deux cotes, et c'est ce debord que le calque doit annoncer.
    let gauche = r.x.min(vx).min(lx);
    let haut = r.y.min(vy);
    let droite = (r.x + r.w).max(vx + cote).max(lx + lw + 2);
    let bas = (r.y + r.h).max(vy + cote).max(ly + HAUTEUR_LIBELLE);

    crate::gui::protocole::Rect::neuf(
        gauche,
        haut,
        (droite - gauche).max(0) as u32,
        (bas - haut).max(0) as u32,
    )
}

/// Une fenetre, focalisee ou non.
pub(crate) fn draw_fenetre(w: &Win, focused: bool) {
    draw_window(w, focused);
}

/// Indice de la fenetre focalisee, s'il y en a une.
pub(crate) fn indice_focus(wins: &[Win]) -> Option<usize> {
    wins.iter().rposition(|w| !w.min)
}

pub(crate) fn draw_desktop(wins: &[Win]) {
    draw_fond();
    draw_filigrane();
    draw_icons();
    draw_topbar();

    let focus = indice_focus(wins);
    for (i, w) in wins.iter().enumerate() {
        if w.min { continue; }
        draw_window(w, Some(i) == focus);
    }
}

// BOUCHAUD_GUI_COQUILLE_V1
//
// Les deux barres partagent la meme matiere que les fenetres : la surface du
// theme, un degrade a peine perceptible, un filet de bordure du cote de
// l'ecran, et du texte en DejaVu -- plus de bitmap 8x8 colle au bord.
//
// Un seul endroit decrit cette matiere, pour que la barre du haut et celle du
// bas ne puissent pas se mettre a diverger : elles se ressemblent trop dans le
// code pour qu'on remarque la difference.

/// Corps du texte des barres, en pixels.
const CORPS_BARRE: f32 = 13.0;

/// Ligne de base du texte dans une barre de `BAR_H` pixels.
fn ligne_texte_barre(sommet: usize) -> usize {
    sommet + (BAR_H - CORPS_BARRE as usize) / 2 - 1
}

/// Fond commun aux deux barres : degrade vertical plus filet de separation.
///
/// `filet_en_bas` distingue la barre du haut -- dont la bordure regarde le
/// bureau, donc vers le bas -- de celle du bas.
fn fond_de_barre(sommet: usize, filet_en_bas: bool) {
    for ligne in 0..BAR_H {
        let couleur = lerp_color(
            crate::gui::theme::COLOR_SURFACE_ELEVATED,
            crate::gui::theme::COLOR_SURFACE,
            if filet_en_bas { ligne } else { BAR_H - 1 - ligne },
            BAR_H,
        );
        fb::fill_rect_rgb(0, sommet + ligne, fb::WIDTH, 1, couleur);
    }
    let filet = if filet_en_bas { sommet + BAR_H - 1 } else { sommet };
    fb::fill_rect_rgb(0, filet, fb::WIDTH, 1, crate::gui::theme::COLOR_BORDER);
}

/// Remplit un rectangle arrondi par SEGMENTS, ANTI-CRENELE.
///
/// BOUCHAUD_C12_CHROME_SANS_MARCHE_V1
///
/// Ce remplisseur dessine les fonds arrondis du chrome -- barre d'URL, boutons,
/// onglets, pastilles. Il utilisait `spans_rounded_rect`, dont les segments
/// sont binaires : chaque coin sortait en marche d'escalier, et c'est ce que
/// l'oeil lit comme « des pixels ».
///
/// `spans_rounded_rect_aa` rend les memes segments avec leur COUVERTURE. Les
/// lignes hors des bandes de coins sortent pleines et retombent sur
/// `fill_rect_rgb` : l'interieur ne paie donc rien de plus qu'avant. Seule la
/// frange des coins est melangee, et elle ne fait que quelques courses par
/// ligne.
fn pave_arrondi(rect: crate::gui::windowing::Rect, rayon: u32, couleur: u32) {
    let (cx0, cy0, cx1, cy1) = fb::clip_rect();
    let decoupe = crate::gui::windowing::Rect::new(cx0 as i32, cy0 as i32,
        cx1.saturating_sub(cx0) as u32, cy1.saturating_sub(cy0) as u32);
    crate::gui::graphics::spans_rounded_rect_aa(rect, rayon, decoupe,
        |x, y, largeur, couverture| {
            fb::blend_rect_rgb(x.max(0) as usize, y.max(0) as usize,
                largeur as usize, couleur, couverture)
        });
}

fn draw_topbar() {
    fond_de_barre(0, true);
    let ligne = ligne_texte_barre(0);

    // Marque du systeme, a gauche.
    fb::draw_text_prop(window::MARGE_BARRE as usize * 2 + 2, ligne, "Bouchaud OS",
        crate::gui::theme::COLOR_TEXT_PRIMARY, CORPS_BARRE, true);

    // Statistiques CPU/RAM/Disque, au centre.
    let stats = sys_stats_str();
    let largeur = fb::text_width(&stats, CORPS_BARRE - 1.0, false);
    let x = (fb::WIDTH / 2).saturating_sub(largeur / 2);
    fb::draw_text_prop(x, ligne, &stats,
        crate::gui::theme::COLOR_TEXT_SECONDARY, CORPS_BARRE - 1.0, false);

    // Horloge, a droite.
    let dt = rtc::now();
    let heure = format!("{:02}:{:02}:{:02}", dt.hour, dt.minute, dt.second);
    let largeur = fb::text_width(&heure, CORPS_BARRE, true);
    fb::draw_text_prop(fb::WIDTH - largeur - window::MARGE_BARRE as usize * 2 - 2,
        ligne, &heure, crate::gui::theme::COLOR_TEXT_PRIMARY, CORPS_BARRE, true);
}

fn sys_stats_str() -> String {
    // BOUCHAUD_SMP_NG2_TOPBAR_PERCPU_V1
    let total_cpu = timer::cpu_load_pct();
    let online = smp::schedulable_cpus().max(1).min(smp::MAX_CPUS);
    let mut cores = String::new();
    for index in 0..online.min(8) {
        if index != 0 { cores.push('/'); }
        cores.push_str(&format!("{}", cpu::load_percent_cpu(index)));
    }
    if online > 8 { cores.push_str("/+"); }

    let (used, _free, total) = crate::kernel::heap::stats();
    let ram_pct = if total > 0 { (used * 100 / total) as u8 } else { 0 };
    let ram_used_str = human_bytes(used);
    let ram_total_str = human_bytes(total);
    let fs = ramfs::fs();
    let disk_used = fs.used_nodes();
    let disk_total = crate::fs::ramfs::MAX_NODES;
    let disk_pct = if disk_total > 0 { (disk_used * 100 / disk_total) as u8 } else { 0 };
    format!(
        "CPU:{total_cpu:3}% [{cores}]  RAM:{ram_used_str}/{ram_total_str} {ram_pct:3}%  Disk:{disk_used}/{disk_total} {disk_pct:3}%"
    )
}

fn human_bytes(n: usize) -> String {
    if n >= 1_073_741_824 { format!("{}Go", n / 1_073_741_824) }
    else if n >= 1_048_576 { format!("{}Mo", n / 1_048_576) }
    else if n >= 1_024    { format!("{}Ko", n / 1_024) }
    else                  { format!("{}o", n) }
}

fn draw_wallpaper() {
    // Gradient bleu nuit du haut (profond) vers le bas (moins foncé)
    //
    // BOUCHAUD_GFX_CULLING_AMONT_V1 : la couleur d'une ligne est une fonction
    // PURE de son ordonnee, donc ne parcourir que les lignes presentees rend
    // exactement la meme image. Sans ce bornage, un degat de curseur de 22
    // lignes faisait quand meme 720 tours de boucle -- et 720 lectures de la
    // decoupe, une par appel a `fill_rect_rgb`, pour 698 lignes jetees.
    let h = fb::HEIGHT.max(1);
    let (_, cy0, _, cy1) = fb::clip_rect();
    for y in cy0..cy1.min(fb::HEIGHT) {
        let c = lerp_color(0x080e1c, 0x1a2f50, y, h);
        fb::fill_rect_rgb(0, y, fb::WIDTH, 1, c);
    }
    // Subtiles étoiles (pixels clairs fixes, déterministes)
    let stars: &[(usize, usize)] = &[
        (120, 80), (340, 45), (600, 130), (780, 60), (1000, 90),
        (200, 200), (500, 180), (850, 220), (1100, 150), (1200, 300),
        (50, 350), (420, 400), (700, 380), (950, 420), (1150, 500),
    ];
    for &(sx, sy) in stars {
        if sx < fb::WIDTH && sy < fb::HEIGHT {
            fb::pixel_rgb(sx, sy, 0x4a6fa5);
        }
    }
}

// ─── Icônes ────────────────────────────────────────────────────────────────
//
// BOUCHAUD_GUI_ICONES_IMAGES_V1
//
// Les cinq icones etaient DESSINEES ici : plus de deux cents lignes de disques,
// de rectangles et de degrades, un peintre par application, rejoues a chaque
// rectangle de degat. Elles ne ressemblaient pas a des icones, et elles ne
// pouvaient pas : ce compositeur n'a ni courbes ni antialiassage.
//
// Ce sont maintenant de vraies images -- quatre PNG fabriques par
// `tools/assets/fabrique-icones.py`, plus le VRAI logo de Ladybird, pris a son
// depot. `gui::icones` les decode et les reduit UNE fois, dans la taille exacte
// ou elles seront posees.

/// Corps du libelle d'une icone de bureau.
const CORPS_LIBELLE: f32 = 12.0;

/// Hauteur reservee au libelle sous l'icone.
const HAUTEUR_LIBELLE: i32 = 18;

fn draw_icons() {
    for i in 0..ICONS.len() {
        draw_icon_at(i);
    }
}

/// Origine de l'image d'une icone, dans son rectangle.
fn image_icone(index: usize) -> (i32, i32) {
    let r = icon_rect(index);
    (r.x + (r.w - crate::gui::icones::TAILLE_BUREAU as i32) / 2, r.y)
}

/// Origine du libelle d'une icone, et sa largeur.
fn libelle_icone(index: usize) -> (i32, i32, i32) {
    let (label, _kind) = ICONS[index];
    let r = icon_rect(index);
    let largeur = fb::text_width(label, CORPS_LIBELLE, false) as i32;
    let x = r.x + (r.w - largeur) / 2;
    let y = r.y + crate::gui::icones::TAILLE_BUREAU as i32 + 6;
    (x, y, largeur)
}

/// Une seule icone. Extrait de `draw_icons` pour que `gui::scene` puisse en
/// ecarter une qui ne touche pas le rectangle en cours.
fn draw_icon_at(i: usize) {
    let (label, _kind) = ICONS[i];
    let (vx, vy) = image_icone(i);
    crate::gui::icones::dessine(i, vx.max(0) as usize, vy.max(0) as usize,
        crate::gui::icones::TAILLE_BUREAU);

    // Libelle : une ombre portee d'un pixel le detache du fond d'ecran, dont la
    // clarte varie du haut au bas de l'ecran.
    let (lx, ly, _) = libelle_icone(i);
    let (lx, ly) = (lx.max(0) as usize, ly.max(0) as usize);
    fb::draw_text_prop(lx + 1, ly + 1, label, 0x050a12, CORPS_LIBELLE, false);
    fb::draw_text_prop(lx, ly, label, crate::gui::theme::COLOR_TEXT_PRIMARY,
        CORPS_LIBELLE, false);
}


// ─── Fenêtres ──────────────────────────────────────────────────────────────

// BOUCHAUD_GUI_EMPREINTE_OMBRE_V1
/// Debordement de l'ombre portee du menu Demarrer, en pixels.
///
/// Simple relais de `disposition::DEBORD_OMBRE`, qui est LA definition : trois
/// endroits doivent s'accorder -- ce que `draw_menu` peint, les bornes que
/// `plan_de_scene` declare, et le rectangle que le compositeur invalide. Les
/// deux derniers avaient diverge du premier, et la bande d'ombre restait a
/// l'ecran : des rectangles sombres abandonnes sur le bureau.
///
/// `draw_window` n'utilise plus ce chemin : sa forme et son debord viennent de
/// `WindowRenderGeometry`, et `window::verifie_constantes` refuse de compiler
/// si `SHADOW_EXTENT` s'ecarte de cette valeur.
pub(crate) const DEBORD_OMBRE: i32 = crate::gui::disposition::DEBORD_OMBRE as i32;

fn draw_window(w: &Win, focused: bool) {
    let x = w.x.max(0) as usize;
    let y = w.y.max(0) as usize;
    let ww = w.w as usize;

    let clip_bounds = fb::clip_rect();
    let damage = crate::gui::windowing::Rect::new(clip_bounds.0 as i32, clip_bounds.1 as i32,
        clip_bounds.2.saturating_sub(clip_bounds.0) as u32,
        clip_bounds.3.saturating_sub(clip_bounds.1) as u32);
    let outer = w.rect();
    let geometry = crate::gui::windowing::window_render_geometry(outer,
        TITLE_H as u32, crate::gui::theme::RADIUS_WINDOW,
        crate::gui::windowing::manager::SHADOW_EXTENT);
    let border = if focused { 0x454c58 } else { crate::gui::theme::COLOR_BORDER };
    // BOUCHAUD_GUI_RASTER_SEGMENTS_V1 : par SEGMENTS, pas par pixels. Une
    // fenetre maximisee coutait huit millions d'iterations et autant de
    // `fetch_add` atomiques par trame ; elle en coute maintenant ~700.
    // BOUCHAUD_C13_PAS_DEUX_FOIS_LE_MEME_PIXEL_V1
    //
    // La promesse : ce rectangle sera repeint OPAQUE avant la fin de la trame.
    // Elle n'est tenable que pour un client ring 3 -- `compose_client` recopie
    // sa surface ligne par ligne, sans transparence, et son ecran d'attente
    // remplit la zone entiere. Une application native peint ce qu'elle veut :
    // elle garde son fond.
    //
    // Le rectangle est borne par la surface REELLE du client, pas par la zone :
    // si un jour la surface est plus petite que la fenetre, la difference doit
    // continuer de recevoir un fond, sinon elle montrerait le bureau.
    let opaque = match &w.app {
        App::Navigateur { client } => {
            let zone = crate::gui::window::zone_utile(w);
            let largeur = (zone.largeur.max(0) as usize).min(client.surface.largeur) as u32;
            let hauteur = (zone.hauteur.max(0) as usize).min(client.surface.hauteur) as u32;
            (largeur > 0 && hauteur > 0)
                .then(|| crate::gui::windowing::Rect::new(zone.x, zone.y, largeur, hauteur))
        }
        _ => None,
    };
    crate::gui::graphics::paint_window_shape_spans(geometry,
        crate::gui::theme::RADIUS_WINDOW,
        crate::gui::windowing::manager::SHADOW_EXTENT, damage,
        crate::gui::theme::COLOR_SURFACE, border, opaque,
        |px, py, largeur, color| {
            fb::fill_rect_rgb(px.max(0) as usize, py.max(0) as usize,
                largeur as usize, 1, color)
        });

    let title_h = TITLE_H as usize;
    let title_color = if focused { crate::gui::theme::COLOR_SURFACE_ELEVATED }
        else { crate::gui::theme::COLOR_SURFACE };
    let title = crate::gui::windowing::titlebar_rect(outer,
        crate::gui::windowing::WINDOW_CHROME);
    // Les coins HAUTS de la barre de titre sont le bord le plus regarde d'une
    // fenetre : c'est la que la marche d'escalier se voyait le mieux.
    crate::gui::graphics::spans_rounded_rect_aa(title, crate::gui::theme::RADIUS_WINDOW,
        damage, |px, py, largeur, couverture| {
            fb::blend_rect_rgb(px.max(0) as usize, py.max(0) as usize,
                largeur as usize, title_color, couverture)
        });
    fb::fill_rect_rgb(x + 1, y + title_h - 1, ww.saturating_sub(2), 1,
        crate::gui::theme::COLOR_BORDER);

    // L'icone de l'application dans sa barre de titre, comme dans la barre des
    // taches : c'est ce qui permet de reconnaitre une fenetre au coin de
    // l'oeil, sans lire.
    let petite = crate::gui::icones::TAILLE_PETITE;
    let mut origine_titre = x + 12;
    if let Some(icone) = crate::gui::icones::pour_app(&w.app) {
        crate::gui::icones::dessine(icone, x + 10, y + (title_h - petite) / 2, petite);
        origine_titre = x + 10 + petite + 8;
    }

    // Titre fenêtre en TTF, tronque a la place REELLE : de son origine jusqu'au
    // premier bouton de la barre de titre. Le compte de caracteres precedent
    // -- `ww / 8 - 6` -- ignorait la largeur des lettres, et un titre etroit
    // comme « Fichiers » laissait un vide pendant qu'un titre large passait
    // sous les boutons.
    let premier_bouton = crate::gui::windowing::minimize_button_rect(outer,
        crate::gui::windowing::WINDOW_CHROME).x.max(0) as usize;
    let place = premier_bouton.saturating_sub(origine_titre + 6);
    let title_clipped = tronque_a_largeur(&w.title, place, 12.0);
    fb::draw_text_prop(origine_titre, y + (title_h - 12) / 2, title_clipped,
        crate::gui::theme::COLOR_TEXT_PRIMARY, 12.0, false);
    dessine_duree_chargement(w, origine_titre, fb::text_width(title_clipped, 12.0, false),
        premier_bouton, y + (title_h - 11) / 2);

    let mouse = crate::gui::mouse::pos();
    let hovered = crate::gui::windowing::hit_test(outer,
        crate::gui::windowing::Point { x: mouse.0 as i32, y: mouse.1 as i32 },
        crate::gui::windowing::WINDOW_CHROME, w.flags.resizable);
    draw_window_controls(outer, hovered, focused);

    // Contenu de l'application
    apps::draw_app(w);

}

fn draw_window_controls(outer: crate::gui::windowing::Rect,
    hover: crate::gui::windowing::HitRegion, focused: bool) {
    use crate::gui::windowing::{close_button_rect, maximize_button_rect,
        minimize_button_rect, HitRegion, WINDOW_CHROME};
    let buttons = [(minimize_button_rect(outer, WINDOW_CHROME), HitRegion::Minimize),
        (maximize_button_rect(outer, WINDOW_CHROME), HitRegion::Maximize),
        (close_button_rect(outer, WINDOW_CHROME), HitRegion::Close)];
    for (rect, region) in buttons {
        if hover == region {
            let color = if region == HitRegion::Close { crate::gui::theme::COLOR_DANGER }
                else { 0x303640 };
            fb::fill_rect_rgb(rect.x.max(0) as usize, rect.y.max(0) as usize,
                rect.width as usize, rect.height as usize, color);
        }
        let color = if focused { crate::gui::theme::COLOR_TEXT_PRIMARY }
            else { crate::gui::theme::COLOR_TEXT_SECONDARY };
        let cx = rect.x + rect.width as i32 / 2;
        let cy = rect.y + rect.height as i32 / 2;
        match region {
            HitRegion::Minimize => fb::fill_rect_rgb((cx - 5) as usize, cy as usize, 10, 1, color),
            HitRegion::Maximize => {
                fb::fill_rect_rgb((cx - 5) as usize, (cy - 4) as usize, 10, 1, color);
                fb::fill_rect_rgb((cx - 5) as usize, (cy + 4) as usize, 10, 1, color);
                fb::fill_rect_rgb((cx - 5) as usize, (cy - 4) as usize, 1, 9, color);
                fb::fill_rect_rgb((cx + 4) as usize, (cy - 4) as usize, 1, 9, color);
            }
            HitRegion::Close => for offset in -4..=4 {
                fb::pixel_rgb((cx + offset) as usize, (cy + offset) as usize, color);
                fb::pixel_rgb((cx + offset) as usize, (cy - offset) as usize, color);
            },
            _ => {}
        }
    }
}

// ─── Clients ring 3 ────────────────────────────────────────────────────────

/// Compose la surface d'un client dans sa fenetre, ou son ecran de demarrage.
///
/// Le gestionnaire de fenetres redessine tout le bureau a chaque trame : la
/// zone utile est recopiee sous la DECOUPE de la trame : le rectangle de degat
/// decide s'il faut recomposer, et la decoupe decide de combien. Sans elle, une
/// fenetre de navigateur coutait 664 400 pixels a chaque trame, y compris quand
/// seul le curseur avait bouge.
pub(crate) fn compose_client(w: &Win, client: &crate::gui::client::Client) {
    use crate::gui::client::Etat;
    let zone = crate::gui::window::zone_utile(w);
    if zone.largeur == 0 || zone.hauteur == 0 {
        return;
    }
    let maintenant = crate::kernel::timer::monotonic_ms();
    if client.etat == Etat::Demarrage {
        dessine_demarrage(&zone, &client.titre, &client.jauge, maintenant);
        return;
    }

    let surface = &client.surface;
    let hauteur = (zone.hauteur as usize).min(surface.hauteur);
    let largeur = (zone.largeur as usize).min(surface.largeur);
    let (zx, zy) = (zone.x.max(0) as usize, zone.y.max(0) as usize);

    // BOUCHAUD_GUI_CLIP_V1
    //
    // Cette recopie ne passe pas par les primitives de dessin : elle ecrit des
    // lignes entieres par `copy_from_slice`, ce qui est justement ce qui la
    // rend rapide. Elle doit donc appliquer la decoupe elle-meme.
    //
    // Ce qu'elle coutait sans : 1100x604, soit 664 400 pixels a CHAQUE trame,
    // pour une fenetre de navigateur -- davantage que le fond d'ecran. Et cela
    // meme quand la seule chose qui avait bouge etait le curseur.
    let (cx0, cy0, cx1, cy1) = fb::clip_rect();
    let x_debut = zx.max(cx0);
    let x_fin = (zx + largeur).min(cx1);
    let y_debut = zy.max(cy0);
    let y_fin = (zy + hauteur).min(cy1);
    if x_fin <= x_debut || y_fin <= y_debut {
        return;
    }
    // Colonne de depart DANS la surface : la decoupe peut couper a gauche.
    let colonne = x_debut - zx;
    let compte = x_fin - x_debut;

    for y in y_debut..y_fin {
        let destination = fb::ligne_mut(x_debut, y, compte);
        if destination.is_empty() {
            continue;
        }
        let pris = destination.len();
        surface.copie_ligne(y - zy, colonne, pris, destination);
    }
    fb::note_pixels_dessines(((x_fin - x_debut) * (y_fin - y_debut)) as u64);

    // La jauge se pose APRES la recopie : c'est ce qui la fait disparaitre
    // toute seule. Quand elle cesse d'etre visible, la surface du client
    // reprend ces trois lignes sans qu'aucun effacement soit necessaire.
    dessine_jauge(&zone, &client.jauge, maintenant);
}

// ─── Jauge de chargement ───────────────────────────────────────────────────
//
// BOUCHAUD_C13_JAUGE_DE_CHARGEMENT_V1
//
// « Au bout de combien de temps la page charge-t-elle ? » n'avait aucune
// reponse a l'ecran : ni barre, ni chiffre, ni meme un signe que quelque chose
// se passait. La seule facon de le savoir etait de chronometrer l'ecran a la
// main.
//
// Deux traits, dessines par ce module :
//
//   * une barre de trois pixels en haut de la zone utile, qui avance ;
//   * la duree, en clair, dans la barre de titre (voir `draw_window`).
//
// La machine a etats qui decide de tout cela est `gui::jauge`, pure et testee
// sur l'hote. Ici il n'y a que du dessin.

/// Hauteur de la barre, en pixels.
pub(crate) const JAUGE_H: usize = 3;

fn dessine_jauge(zone: &crate::gui::protocole::Rect, jauge: &crate::gui::jauge::Jauge,
    maintenant: u64) {
    use crate::gui::jauge::Phase;
    if !jauge.visible() {
        return;
    }
    let largeur = zone.largeur.max(0) as usize;
    if largeur == 0 || zone.hauteur as usize <= JAUGE_H {
        return;
    }
    let x = zone.x.max(0) as usize;
    let y = zone.y.max(0) as usize;

    // Le rail. Une page blanche et une barre bleue sur un fond blanc : sans
    // rail, la partie NON remplie serait invisible et la barre paraitrait
    // toujours pleine.
    fb::fill_rect_rgb(x, y, largeur, JAUGE_H, crate::gui::theme::COLOR_BORDER);

    let progression = jauge.progression(maintenant) as usize;
    // Arrondi vers le HAUT d'un pixel minimum : a 1 % sur une fenetre etroite,
    // la division rendait zero et la barre restait eteinte pendant que la page
    // chargeait -- exactement le contraire de ce qu'elle doit dire.
    let remplie = (largeur * progression / 100).max(1).min(largeur);
    let couleur = if jauge.phase() == Phase::Termine {
        crate::gui::theme::COLOR_SUCCESS
    } else {
        crate::gui::theme::COLOR_ACCENT
    };
    fb::fill_rect_rgb(x, y, remplie, JAUGE_H, couleur);
}

/// La duree de chargement, en clair, a droite du titre de la fenetre.
///
/// Rendue dans la barre de titre et non sur la page : un indicateur qui
/// recouvre le contenu pendant qu'on essaie de le lire est un defaut, pas une
/// fonctionnalite.
fn dessine_duree_chargement(w: &Win, origine: usize, apres_titre: usize,
    premier_bouton: usize, ligne: usize) {
    let App::Navigateur { client } = &w.app else { return };
    if !client.jauge.visible() {
        return;
    }
    let maintenant = crate::kernel::timer::monotonic_ms();
    let duree = crate::gui::jauge::formate_duree(client.jauge.duree_affichee_ms(maintenant));
    let texte = duree.as_str();
    let debut = (origine + apres_titre).max(origine) + 10;
    // On n'ecrit RIEN plutot que d'ecrire sous les boutons : un debord interdit
    // tout culling par rectangle, et la duree passerait sous la croix.
    if debut + fb::text_width(texte, 11.0, false) + 6 > premier_bouton {
        return;
    }
    let couleur = if client.jauge.phase() == crate::gui::jauge::Phase::Termine {
        crate::gui::theme::COLOR_SUCCESS
    } else {
        crate::gui::theme::COLOR_TEXT_SECONDARY
    };
    fb::draw_text_prop(debut, ligne, texte, couleur, 11.0, false);
}

/// Ecran d'attente dessine **dans la fenetre** du client.
///
/// C'est le meme visuel que la carte de lancement d'avant, a un detail pres qui
/// change tout : il ne recouvre plus le bureau. La fenetre existe des le double
/// clic, la barre des taches et l'horloge continuent, et le contenu Web viendra
/// remplacer ce dessin sans qu'aucune transition ne soit visible ailleurs.
fn dessine_demarrage(zone: &crate::gui::protocole::Rect, titre: &str,
    jauge: &crate::gui::jauge::Jauge, maintenant: u64) {
    let zx = zone.x.max(0) as usize;
    let zy = zone.y.max(0) as usize;
    let zl = zone.largeur as usize;
    let zh = zone.hauteur as usize;

    fb::fill_rect_rgb(zx, zy, zl, zh, 0x000B_1220);

    let largeur = 540usize.min(zl);
    let hauteur = 190usize.min(zh);
    let x = zx + (zl - largeur) / 2;
    let y = zy + (zh - hauteur) / 2;

    fb::fill_rect_rgb(x, y, largeur, hauteur, 0x0011_1B2E);
    fb::fill_rect_rgb(x, y, largeur, 4, 0x003D_8BFF);
    fb::draw_text_prop(x + 34, y + 34, titre, 0x00F3_F6FC, 30.0, true);
    fb::draw_text_prop(
        x + 34,
        y + 86,
        "Demarrage de Qt, Python et du renderer...",
        0x00B8_C4D9,
        16.0,
        false,
    );

    // Le chronometre du demarrage.
    //
    // Il ne cache pas la lenteur, il la CHIFFRE. Un ecran d'attente qui ne
    // montre rien laisse croire que la machine est bloquee ; le meme ecran
    // avec « 8,40 s » qui defile dit qu'elle travaille, et donne d'un coup
    // d'oeil le nombre qu'une optimisation devra faire baisser.
    if largeur > 80 && hauteur > 150 {
        let rail_l = largeur - 68;
        let rail_x = x + 34;
        let rail_y = y + 118;
        fb::fill_rect_rgb(rail_x, rail_y, rail_l, 4, 0x001E_2E4A);
        let remplie = (rail_l * jauge.progression(maintenant) as usize / 100).max(2);
        fb::fill_rect_rgb(rail_x, rail_y, remplie.min(rail_l), 4, 0x003D_8BFF);

        let ecoule = crate::gui::jauge::formate_duree(jauge.duree_affichee_ms(maintenant));
        fb::draw_text_prop(rail_x, y + 140, ecoule.as_str(), 0x00B8_C4D9, 14.0, false);
    }

    fb::draw_text_prop(
        x + 34,
        y + 164,
        "Le bureau reste actif pendant le chargement.",
        0x007F_93B8,
        14.0,
        false,
    );
}

// ─── Barre des tâches ──────────────────────────────────────────────────────

/// Dessine la barre des taches.
/// Plus long prefixe de `s` dont le rendu tient dans `largeur` pixels.
///
/// `window::clip` compte des CARACTERES, ce qui n'a pas de sens pour une police
/// proportionnelle : sept caracteres font quarante pixels ou soixante selon le
/// mot. Les libelles debordaient donc de leur bouton -- et un debord interdit
/// tout culling par rectangle, puisque le voisin ecarte emporte les pixels
/// qu'il posait chez l'autre.
fn tronque_a_largeur(s: &str, largeur: usize, px: f32) -> &str {
    if fb::text_width(s, px, false) <= largeur {
        return s;
    }
    let mut fin = 0;
    for (indice, _) in s.char_indices().skip(1) {
        if fb::text_width(&s[..indice], px, false) > largeur {
            break;
        }
        fin = indice;
    }
    &s[..fin]
}

pub(crate) fn draw_taskbar(wins: &[Win], menu_open: bool) {
    let sommet = fb::HEIGHT - BAR_H;
    fond_de_barre(sommet, false);

    let focus = indice_focus(wins);
    let rayon = crate::gui::theme::RADIUS_SM;

    // Bouton Demarrer : accentue quand le menu est ouvert, sourd sinon. Meme
    // forme arrondie que le chrome des fenetres.
    let sb = start_btn();
    let cadre = crate::gui::windowing::Rect::new(sb.x, sb.y, sb.w as u32, sb.h as u32);
    let fond = if menu_open { crate::gui::theme::COLOR_ACCENT }
        else { crate::gui::theme::COLOR_SURFACE_ELEVATED };
    pave_arrondi(cadre, rayon, fond);
    let corps = CORPS_BARRE - 1.0;
    let etiquette = "Demarrer";
    let largeur = fb::text_width(etiquette, corps, true);
    fb::draw_text_prop(
        (sb.x as usize) + (sb.w as usize).saturating_sub(largeur) / 2,
        ligne_texte_barre(sommet),
        etiquette,
        crate::gui::theme::COLOR_TEXT_PRIMARY,
        corps,
        true,
    );

    // Boutons des fenetres.
    for (i, w) in wins.iter().enumerate() {
        let b = taskbar_btn(i);
        if b.x + b.w > fb::WIDTH as i32 { break; }
        let bx = b.x as usize; let by = b.y as usize;
        let bw = b.w as usize; let bh = b.h as usize;
        // BOUCHAUD_GFX_CULLING_AMONT_V1 : un degat sur un bouton n'a aucune
        // raison de faire dessiner les autres. `continue`, et non `break` : la
        // barre est parcourue de gauche a droite, mais rien ne garantit que le
        // degat soit d'un seul tenant.
        //
        // Ce test n'est CORRECT que parce que le bouton ne peint rien hors de
        // son rectangle -- d'ou la troncature du libelle a la largeur reelle,
        // juste en dessous. Un libelle qui debordait chez le voisin aurait ete
        // emporte par le culling.
        if !fb::decoupe_touche(bx, by, bw, bh) { continue }

        let actif = focus == Some(i) && !w.min;
        let cadre = crate::gui::windowing::Rect::new(b.x, b.y, bw as u32, bh as u32);
        let fond = if actif { crate::gui::theme::COLOR_SURFACE_ELEVATED }
            else { crate::gui::theme::COLOR_SURFACE };
        pave_arrondi(cadre, rayon, fond);

        // La fenetre au premier plan porte un liset d'accent en bas, comme un
        // onglet actif. Une fenetre minimisee n'en a pas : c'est le seul signe
        // qui distingue « au-dessus » de « rangee », et il manquait.
        if actif {
            fb::fill_rect_rgb(bx + 3, by + bh - 2, bw.saturating_sub(6), 2,
                crate::gui::theme::COLOR_ACCENT);
        }

        // L'icone de l'application, la meme que sur le bureau. Une barre des
        // taches sans icones oblige a lire pour reconnaitre une fenetre.
        let petite = crate::gui::icones::TAILLE_PETITE;
        let mut texte_x = bx + 10;
        if let Some(icone) = crate::gui::icones::pour_app(&w.app) {
            crate::gui::icones::dessine(icone, bx + 8,
                by + (bh - petite) / 2, petite);
            texte_x = bx + 8 + petite + 8;
        }

        let couleur = if w.min { crate::gui::theme::COLOR_TEXT_SECONDARY }
            else { crate::gui::theme::COLOR_TEXT_PRIMARY };
        // Label, tronque a la LARGEUR du bouton et non a un nombre de
        // caracteres : « Navigateur » et « Fichiers » n'ont pas la meme largeur
        // a sept caracteres, et le premier debordait sur le bouton suivant.
        let reste = (bx + bw).saturating_sub(texte_x + 8);
        let lbl = tronque_a_largeur(&w.title, reste, corps);
        fb::draw_text_prop(texte_x, ligne_texte_barre(sommet), lbl, couleur,
            corps, false);
    }
}

// ─── Menu Démarrer (style Windows moderne) ─────────────────────────────────

/// Dessine le menu Démarrer avec hover selon la position souris (mx, my).
pub(crate) fn draw_menu(mx: i32, my: i32) {
    let mr = menu_rect();
    let mxi = mr.x as usize;
    let myi = mr.y as usize;
    let mw = mr.w as usize;
    let mh = mr.h as usize;

    // Ombre portée, meme debordement que les fenetres.
    let debord = DEBORD_OMBRE as usize;
    fb::fill_rect_rgb(mxi + debord, myi + debord, mw, mh, 0x030608);

    // BOUCHAUD_GUI_COQUILLE_V1 : meme matiere que les fenetres -- surface du
    // theme, coins arrondis, filet de bordure -- plutot que les bleus en dur
    // d'un theme qui n'existe plus ailleurs.
    let cadre = crate::gui::windowing::Rect::new(mr.x, mr.y, mw as u32, mh as u32);
    let rayon = crate::gui::theme::RADIUS_MD;
    pave_arrondi(cadre, rayon, crate::gui::theme::COLOR_SURFACE);

    // Bordure
    fb::fill_rect_rgb(mxi, myi, mw, 1, crate::gui::theme::COLOR_BORDER);
    fb::fill_rect_rgb(mxi, myi, 1, mh, crate::gui::theme::COLOR_BORDER);
    fb::fill_rect_rgb(mxi + mw - 1, myi, 1, mh, crate::gui::theme::COLOR_BORDER);
    fb::fill_rect_rgb(mxi, myi + mh - 1, mw, 1, crate::gui::theme::COLOR_BORDER);

    // BOUCHAUD_GUI_HOVER_CONTRAT_V1
    //
    // Le survol n'est plus calcule ici. `window::ligne_menu_survolee` est la
    // seule definition, et c'est elle que le gestionnaire de fenetres consulte
    // pour invalider l'ancienne et la nouvelle ligne. Recalculer localement,
    // meme a l'identique, rouvrirait la porte a l'ecart qui laissait une ligne
    // en surbrillance derriere le pointeur.
    let hover_row: Option<usize> = window::ligne_menu_survolee(mx, my);

    let sep_idx = MENU.len() - 1; // index de "Quitter"
    let bande = crate::gui::disposition::BANDE_ACCENT as usize;

    for (i, (item, _kind)) in MENU.iter().enumerate() {
        let iy = myi + MENU_HEADER_H as usize + i * MENU_ITEM_H as usize;

        // Séparateur avant Quitter
        if i == sep_idx {
            fb::fill_rect_rgb(mxi + bande + 4, iy, mw - bande - 8, 1,
                crate::gui::theme::COLOR_BORDER);
        }

        // Fond survolé : un pave arrondi, comme un bouton de barre.
        if hover_row == Some(i) {
            let ligne = crate::gui::windowing::Rect::new(
                mr.x + bande as i32, iy as i32,
                (mw - bande * 2) as u32, MENU_ITEM_H as u32);
            pave_arrondi(ligne, crate::gui::theme::RADIUS_SM,
                crate::gui::theme::COLOR_SURFACE_ELEVATED);
        }

        // L'icone de l'application, la meme que sur le bureau et dans la barre
        // des taches. « Quitter » et « Moniteur » n'en ont pas : une pastille
        // prend leur place, rouge pour la premiere.
        let petite = crate::gui::icones::TAILLE_PETITE;
        let image_x = mxi + bande + 8;
        let image_y = iy + (MENU_ITEM_H as usize - petite) / 2;
        match crate::gui::icones::pour_kind(*_kind) {
            Some(icone) => crate::gui::icones::dessine(icone, image_x, image_y, petite),
            None => {
                let teinte = if i == sep_idx { crate::gui::theme::COLOR_DANGER }
                    else { crate::gui::theme::COLOR_TEXT_SECONDARY };
                pave_arrondi(
                    crate::gui::windowing::Rect::new(
                        (image_x + 4) as i32, (image_y + 4) as i32, 10, 10),
                    3, teinte);
            }
        }

        // Texte de l'entree.
        let couleur = if i == sep_idx {
            crate::gui::theme::COLOR_DANGER
        } else if hover_row == Some(i) {
            crate::gui::theme::COLOR_TEXT_PRIMARY
        } else {
            crate::gui::theme::COLOR_TEXT_SECONDARY
        };
        fb::draw_text_prop(
            mxi + bande + 8 + crate::gui::icones::TAILLE_PETITE + 10,
            iy + (MENU_ITEM_H as usize - 12) / 2,
            item,
            couleur,
            12.0,
            hover_row == Some(i),
        );
    }
}

// ─── Curseur souris ────────────────────────────────────────────────────────

/// Dessine le curseur souris avec couleur adaptee au fond.
pub(crate) fn draw_cursor(mx: usize, my: usize) {
    const CUR: [u16; 12] = [
        0b0000000000000001,
        0b0000000000000011,
        0b0000000000000111,
        0b0000000000001111,
        0b0000000000011111,
        0b0000000000111111,
        0b0000000001111111,
        0b0000000000001111,
        0b0000000000011011,
        0b0000000000110001,
        0b0000000001100000,
        0b0000000001000000,
    ];
    let px = mx.min(fb::WIDTH.saturating_sub(1));
    let py = my.min(fb::HEIGHT.saturating_sub(1));
    let bg = fb::get_pixel_rgb(px, py);
    let lum = ((bg >> 16 & 0xff) * 299 + (bg >> 8 & 0xff) * 587 + (bg & 0xff) * 114) / 1000;
    let (fill, outline) = if lum > 140 { (0x000000u32, 0xffffffu32) } else { (0xffffffu32, 0x000000u32) };

    // Outline
    for (row, &bits) in CUR.iter().enumerate() {
        for col in 0..12usize {
            if bits & (1 << col) != 0 {
                for (ddy, ddx) in [(-1i32,0i32),(1,0),(0,-1),(0,1)].iter() {
                    let nx = col as i32 + ddx;
                    let ny = row as i32 + ddy;
                    if nx >= 0 && ny >= 0 && (nx as usize) < 12 {
                        let nr = ny as usize;
                        if nr < CUR.len() && CUR[nr] & (1 << nx) == 0 {
                            let px2 = mx + col;
                            let py2 = my + row;
                            if px2 < fb::WIDTH && py2 < fb::HEIGHT {
                                fb::pixel_rgb(px2, py2, outline);
                            }
                        }
                    }
                }
            }
        }
    }
    // Fill
    for (row, &bits) in CUR.iter().enumerate() {
        for col in 0..12usize {
            if bits & (1 << col) != 0 && mx + col < fb::WIDTH && my + row < fb::HEIGHT {
                fb::pixel_rgb(mx + col, my + row, fill);
            }
        }
    }
}
