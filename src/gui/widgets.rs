//! Widgets de rendu du bureau : fenetres, barre des taches, menu, curseur.

use crate::gui::apps;
use crate::gui::framebuffer as fb;
use crate::gui::window::{
    self, icon_rect, menu_rect, start_btn, taskbar_btn, Win,
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
    let (label, _kind) = ICONS[index];
    let r = icon_rect(index);
    let vw = 40i32;
    let vx = r.x + (r.w - vw) / 2;
    let vy = r.y;

    let lw = fb::text_width(label, 10.0, false) as i32;
    let lx = (r.x + (r.w - lw) / 2).max(0);
    let ly = vy + vw + 3;
    // Hauteur majoree d'une ligne de 10 pixels : jambages et ombre comprises.
    const HAUTEUR_LIBELLE: i32 = 16;

    let gauche = r.x.min(lx);
    let haut = r.y.min(vy);
    let droite = (r.x + r.w)
        .max(vx + 3 + vw)      // ombre portee du carre
        .max(lx + lw + 2);     // libelle plus son ombre d'un pixel
    let bas = (r.y + r.h)
        .max(vy + 3 + vw)
        .max(ly + HAUTEUR_LIBELLE);

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

/// Remplit un rectangle arrondi par SEGMENTS, comme le chrome des fenetres.
fn pave_arrondi(rect: crate::gui::windowing::Rect, rayon: u32, couleur: u32) {
    let (cx0, cy0, cx1, cy1) = fb::clip_rect();
    let decoupe = crate::gui::windowing::Rect::new(cx0 as i32, cy0 as i32,
        cx1.saturating_sub(cx0) as u32, cy1.saturating_sub(cy0) as u32);
    crate::gui::graphics::spans_rounded_rect(rect, rayon, decoupe,
        |x, y, largeur| {
            fb::fill_rect_rgb(x.max(0) as usize, y.max(0) as usize,
                largeur as usize, 1, couleur)
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

fn draw_icons() {
    for i in 0..ICONS.len() {
        draw_icon_at(i);
    }
}

/// Une seule icone. Extrait de `draw_icons` pour que `gui::scene` puisse en
/// ecarter une qui ne touche pas le rectangle en cours.
fn draw_icon_at(i: usize) {
    {
        let (label, _kind) = ICONS[i];
        let r = icon_rect(i);
        let vw = 40i32;
        let vx = r.x + (r.w - vw) / 2;
        let vy = r.y;

        // Ombre portée
        fb::fill_rect_rgb((vx + 3) as usize, (vy + 3) as usize, vw as usize, vw as usize, 0x06090f);

        // Fond de l'icône (carré arrondi simulé)
        draw_app_icon(i, vx as usize, vy as usize, vw as usize);

        // Halo de sélection (cadre bleu subtil)
        // fb::rect(vx as usize, vy as usize, vw as usize, vw as usize, fb::C_DKBLUE);

        // Label TTF antialiasé avec ombre
        let lw = fb::text_width(label, 10.0, false) as i32;
        let lx = (r.x + (r.w - lw) / 2).max(0) as usize;
        let ly = (vy + vw + 3) as usize;
        // Ombre du texte
        fb::draw_text_prop(lx + 1, ly + 1, label, 0x000000, 10.0, false);
        fb::draw_text_prop(lx, ly, label, 0xe8f4fd, 10.0, false);
    }
}

/// Dessine l'icone pixel-art de l'application `kind` dans un carre `vw x vw` en (vx, vy).
fn draw_app_icon(icon_idx: usize, vx: usize, vy: usize, vw: usize) {
    match icon_idx {
        0 => draw_icon_ladybird(vx, vy, vw),
        1 => draw_icon_calculator(vx, vy, vw),
        2 => draw_icon_terminal(vx, vy, vw),
        3 => draw_icon_files(vx, vy, vw),
        4 => draw_icon_rustpad(vx, vy, vw),
        _ => { fb::fill_rect_rgb(vx, vy, vw, vw, 0x555555); }
    }
}

/// Remplit un disque plein (cx, cy = coordonnees ecran) clippé dans la zone icone.
fn fill_circle(scx: i32, scy: i32, r: i32, col: u32, clip_x: usize, clip_y: usize, clip_w: usize) {
    if r <= 0 { return; }
    for dy in -r..=r {
        let hw = isqrt(r * r - dy * dy);
        let y = scy + dy;
        if y < clip_y as i32 || y >= (clip_y + clip_w) as i32 { continue; }
        let x0 = (scx - hw).max(clip_x as i32);
        let x1 = (scx + hw).min((clip_x + clip_w - 1) as i32);
        if x0 <= x1 {
            fb::fill_rect_rgb(x0 as usize, y as usize, (x1 - x0 + 1) as usize, 1, col);
        }
    }
}

/// Logo Ladybird : la coccinelle du moteur qu'on execute reellement.
///
/// Le bureau affichait jusqu'ici un nautile, dessine du temps ou le navigateur
/// etait un moteur maison. Le moteur est maintenant Ladybird, et l'icone doit
/// dire lequel : c'est ce que l'utilisateur reconnait, et c'est aussi honnete
/// vis-a-vis d'un projet dont on execute le code.
///
/// Le dessin est fait de disques et de rectangles clippes — les seules
/// primitives dont ce compositeur dispose — et reste lisible a la taille reelle
/// d'une icone de bureau (48 px) comme a celle d'une entree de menu.
fn draw_icon_ladybird(vx: usize, vy: usize, vw: usize) {
    // Fond : le meme bleu profond que les autres icones, pour que la rangee du
    // bureau garde une famille visuelle.
    for dy in 0..vw {
        let c = lerp_color(0x10243c, 0x081525, dy, vw);
        fb::fill_rect_rgb(vx, vy + dy, vw, 1, c);
    }

    let vwi = vw as i32;
    let cx = vx as i32 + vwi / 2;
    let cy = vy as i32 + vwi / 2 + vwi / 12; // legerement bas : la tete prend le haut
    let r = vwi * 2 / 5;

    // Ombre portee, un disque decale d'un pixel vers le bas.
    fill_circle(cx, cy + 1, r, 0x04080f, vx, vy, vw);

    // Tete noire, posee au-dessus du corps et donc dessinee avant lui.
    fill_circle(cx, cy - r * 3 / 4, r * 1 / 2, 0x1a1a1e, vx, vy, vw);
    // Deux yeux blancs.
    fill_circle(cx - r / 4, cy - r * 5 / 6, r / 8, 0xf2f2f2, vx, vy, vw);
    fill_circle(cx + r / 4, cy - r * 5 / 6, r / 8, 0xf2f2f2, vx, vy, vw);

    // Corps rouge.
    fill_circle(cx, cy, r, 0xd8202a, vx, vy, vw);
    // Reflet : un disque plus clair en haut a gauche, ecrete par le corps.
    fill_circle(cx - r / 3, cy - r / 3, r / 3, 0xf0505a, vx, vy, vw);

    // Ligne mediane entre les elytres.
    let ligne_h = (vwi / 24).max(1);
    let y0 = (cy - r).max(vy as i32);
    let y1 = (cy + r).min((vy + vw) as i32 - 1);
    if y1 > y0 {
        fb::fill_rect_rgb(
            (cx - ligne_h / 2).max(vx as i32) as usize,
            y0 as usize,
            ligne_h as usize,
            (y1 - y0) as usize,
            0x1a1a1e,
        );
    }

    // Six points, trois par elytre. Les rayons decroissent vers le bas pour
    // suivre le retrecissement apparent de la coque.
    let points: [(i32, i32, i32); 6] = [
        (-r / 2, -r / 3, r / 5),
        (r / 2, -r / 3, r / 5),
        (-r / 2, r / 4, r / 6),
        (r / 2, r / 4, r / 6),
        (-r / 4, r * 2 / 3, r / 8),
        (r / 4, r * 2 / 3, r / 8),
    ];
    for (dx, dy, pr) in points {
        fill_circle(cx + dx, cy + dy, pr.max(1), 0x1a1a1e, vx, vy, vw);
    }
}

fn isqrt(n: i32) -> i32 {
    if n <= 0 { return 0; }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x { x = y; y = (x + n / x) / 2; }
    x
}


fn draw_icon_calculator(vx: usize, vy: usize, vw: usize) {
    // Fond gris clair avec dégradé
    for dy in 0..vw {
        let c = lerp_color(0xf0f0f0, 0xd0d0d0, dy, vw);
        fb::fill_rect_rgb(vx, vy + dy, vw, 1, c);
    }
    // Cadre foncé
    fb::fill_rect_rgb(vx, vy, vw, 1, 0x888888);
    fb::fill_rect_rgb(vx, vy + vw - 1, vw, 1, 0x888888);
    fb::fill_rect_rgb(vx, vy, 1, vw, 0x888888);
    fb::fill_rect_rgb(vx + vw - 1, vy, 1, vw, 0x888888);
    // Écran
    fb::fill_rect_rgb(vx + 3, vy + 3, vw - 6, 10, 0x0a1628);
    fb::fill_rect_rgb(vx + 3, vy + 3, vw - 6, 1, 0x1e3a5f);
    fb::draw_text_rgb(vx + 6, vy + 5, "0", 0x00ff88, 1);
    // Grille boutons (3x4)
    let colors = [
        [0xdddddd, 0xdddddd, 0xff5555u32],
        [0xdddddd, 0xdddddd, 0xdddddd],
        [0xdddddd, 0xdddddd, 0xdddddd],
        [0xdddddd, 0xdddddd, 0x3377ff],
    ];
    for row in 0..4usize {
        for col in 0..3usize {
            let bx = vx + 3 + col * 11;
            let by = vy + 15 + row * 6;
            fb::fill_rect_rgb(bx, by, 10, 5, colors[row][col]);
            fb::fill_rect_rgb(bx, by, 10, 1, lerp_color(colors[row][col], 0xffffff, 60, 100));
        }
    }
}

fn draw_icon_terminal(vx: usize, vy: usize, vw: usize) {
    // Fond noir profond
    fb::fill_rect_rgb(vx, vy, vw, vw, 0x0a0e1a);
    // Barre de titre macOS-style
    fb::fill_rect_rgb(vx, vy, vw, 8, 0x1c1c1c);
    // Boutons traffic lights
    draw_circle(vx + 5, vy + 4, 2, 0xff5f57);
    draw_circle(vx + 11, vy + 4, 2, 0xffbd2e);
    draw_circle(vx + 17, vy + 4, 2, 0x28c840);
    // Prompt
    fb::draw_text_rgb(vx + 2, vy + 10, ">_", 0x00ff88, 1);
    // Lignes de texte simulées
    let line_cols = [0x3d6b8a, 0x2a4f6e, 0x3d6b8a, 0x1e3a52, 0x2a4f6e];
    for (i, &c) in line_cols.iter().enumerate() {
        let ly = vy + 20 + i * 4;
        let lw = if i % 2 == 0 { vw * 3 / 4 } else { vw / 2 };
        fb::fill_rect_rgb(vx + 2, ly, lw, 2, c);
    }
    // Curseur clignotant (bleu)
    fb::fill_rect_rgb(vx + 10, vy + 10, 2, 7, 0x4488ff);
}

fn draw_icon_files(vx: usize, vy: usize, vw: usize) {
    // Corps du dossier avec dégradé
    let body_y = vy + 8;
    let body_h = vw - 10;
    for dy in 0..body_h {
        let c = lerp_color(0xffc107, 0xf57f17, dy, body_h);
        fb::fill_rect_rgb(vx + 1, body_y + dy, vw - 2, 1, c);
    }
    // Onglet
    for dy in 0..5usize {
        let c = lerp_color(0xffca28, 0xffa000, dy, 5);
        fb::fill_rect_rgb(vx + 1, vy + 4 + dy, 14, 1, c);
    }
    // Reflet en haut du dossier
    fb::fill_rect_rgb(vx + 1, body_y, vw - 2, 2, 0xffe082);
    // Ombre en bas
    fb::fill_rect_rgb(vx + 1, body_y + body_h - 3, vw - 2, 3, 0xe65100);
    // Documents à l'intérieur
    fb::fill_rect_rgb(vx + 6, body_y + 5, vw - 14, 2, 0xfff8e1);
    fb::fill_rect_rgb(vx + 6, body_y + 9, vw - 14, 2, 0xfff8e1);
    fb::fill_rect_rgb(vx + 6, body_y + 13, (vw - 14) * 2 / 3, 2, 0xfff8e1);
}

fn draw_icon_rustpad(vx: usize, vy: usize, vw: usize) {
    // Fond GitHub dark
    fb::fill_rect_rgb(vx, vy, vw, vw, 0x0d1117);
    // Cadre de l'éditeur
    let pad = vw / 8;
    fb::fill_rect_rgb(vx + pad, vy + pad, vw - pad*2, vw - pad*2, 0x161b22);
    fb::fill_rect_rgb(vx + pad, vy + pad, vw - pad*2, 1, 0x30363d);
    // Lignes de code colorées (syntaxe highlight)
    let lh = (vw / 9).max(2);
    let lx = vx + pad + 3;
    let pairs: &[(u32, usize)] = &[
        (0xff7b72, vw / 2),       // fn (rouge)
        (0xa5d6ff, vw / 3),       // let (bleu)
        (0x3fb950, vw * 2 / 5),   // string (vert)
        (0xa5d6ff, vw / 4),       // valeur
        (0x8b949e, vw / 3),       // commentaire
    ];
    for (i, &(color, w)) in pairs.iter().enumerate() {
        let indent = if i == 0 { 0 } else { vw / 8 };
        fb::fill_rect_rgb(lx + indent, vy + pad + 3 + i * (lh + 2), w, lh, color);
    }
    // Bouton Run ▶ (triangle vert)
    let bx = vx + vw - pad - vw / 5;
    let by = vy + pad + 2;
    let bs = (vw / 5).max(4);
    for row in 0..bs {
        let w = (row * 2 + 1).min(bs);
        fb::fill_rect_rgb(bx, by + row, w, 1, 0x238636);
    }
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
    crate::gui::graphics::paint_window_shape_spans(geometry,
        crate::gui::theme::RADIUS_WINDOW,
        crate::gui::windowing::manager::SHADOW_EXTENT, damage,
        crate::gui::theme::COLOR_SURFACE, border,
        |px, py, largeur, color| {
            fb::fill_rect_rgb(px.max(0) as usize, py.max(0) as usize,
                largeur as usize, 1, color)
        });

    let title_h = TITLE_H as usize;
    let title_color = if focused { crate::gui::theme::COLOR_SURFACE_ELEVATED }
        else { crate::gui::theme::COLOR_SURFACE };
    let title = crate::gui::windowing::titlebar_rect(outer,
        crate::gui::windowing::WINDOW_CHROME);
    crate::gui::graphics::spans_rounded_rect(title, crate::gui::theme::RADIUS_WINDOW,
        damage, |px, py, largeur| {
            fb::fill_rect_rgb(px.max(0) as usize, py.max(0) as usize,
                largeur as usize, 1, title_color)
        });
    fb::fill_rect_rgb(x + 1, y + title_h - 1, ww.saturating_sub(2), 1,
        crate::gui::theme::COLOR_BORDER);

    // Titre fenêtre en TTF, tronque a la place REELLE : de son origine jusqu'au
    // premier bouton de la barre de titre. Le compte de caracteres precedent
    // -- `ww / 8 - 6` -- ignorait la largeur des lettres, et un titre etroit
    // comme « Fichiers » laissait un vide pendant qu'un titre large passait
    // sous les boutons.
    let origine_titre = x + 12;
    let premier_bouton = crate::gui::windowing::minimize_button_rect(outer,
        crate::gui::windowing::WINDOW_CHROME).x.max(0) as usize;
    let place = premier_bouton.saturating_sub(origine_titre + 6);
    let title_clipped = tronque_a_largeur(&w.title, place, 11.0);
    fb::draw_text_prop(origine_titre, y + 8, title_clipped,
        crate::gui::theme::COLOR_TEXT_PRIMARY, 11.0, false);

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
    if client.etat == Etat::Demarrage {
        dessine_demarrage(&zone, &client.titre);
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
}

/// Ecran d'attente dessine **dans la fenetre** du client.
///
/// C'est le meme visuel que la carte de lancement d'avant, a un detail pres qui
/// change tout : il ne recouvre plus le bureau. La fenetre existe des le double
/// clic, la barre des taches et l'horloge continuent, et le contenu Web viendra
/// remplacer ce dessin sans qu'aucune transition ne soit visible ailleurs.
fn dessine_demarrage(zone: &crate::gui::protocole::Rect, titre: &str) {
    let zx = zone.x.max(0) as usize;
    let zy = zone.y.max(0) as usize;
    let zl = zone.largeur as usize;
    let zh = zone.hauteur as usize;

    fb::fill_rect_rgb(zx, zy, zl, zh, 0x000B_1220);

    let largeur = 540usize.min(zl);
    let hauteur = 170usize.min(zh);
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
    fb::draw_text_prop(
        x + 34,
        y + 121,
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

        let couleur = if w.min { crate::gui::theme::COLOR_TEXT_SECONDARY }
            else { crate::gui::theme::COLOR_TEXT_PRIMARY };
        // Label, tronque a la LARGEUR du bouton et non a un nombre de
        // caracteres : « Navigateur » et « Fichiers » n'ont pas la meme largeur
        // a sept caracteres, et le premier debordait sur le bouton suivant.
        let lbl = tronque_a_largeur(&w.title, bw.saturating_sub(20), corps);
        fb::draw_text_prop(bx + 10, ligne_texte_barre(sommet), lbl, couleur,
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

    // Pastilles des entrees : la seule couleur vive du menu, une par action.
    let pastilles: &[u32] = &[
        0x3fb950, // Terminal
        0xd29922, // Fichiers
        0x39c5cf, // Moniteur
        0x8b949e, // Calculatrice
        0xff7b72, // Rustpad
        crate::gui::theme::COLOR_DANGER, // Quitter
    ];

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

        // Pastille de l'entree.
        let teinte = pastilles.get(i).copied().unwrap_or(0x555577);
        let pastille_x = mxi + bande + 8;
        let pastille_y = iy + (MENU_ITEM_H as usize - 10) / 2;
        pave_arrondi(
            crate::gui::windowing::Rect::new(pastille_x as i32, pastille_y as i32, 10, 10),
            3, teinte);

        // Texte de l'entree.
        let couleur = if i == sep_idx {
            crate::gui::theme::COLOR_DANGER
        } else if hover_row == Some(i) {
            crate::gui::theme::COLOR_TEXT_PRIMARY
        } else {
            crate::gui::theme::COLOR_TEXT_SECONDARY
        };
        fb::draw_text_prop(
            mxi + bande + 26,
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
