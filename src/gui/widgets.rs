//! Widgets de rendu du bureau : fenetres, barre des taches, menu, curseur.

use crate::gui::apps;
use crate::gui::framebuffer as fb;
use crate::gui::window::{
    self, icon_rect, menu_rect, App, Win,
    BAR_H, ICONS, MENU, MENU_HEADER_H, MENU_ITEM_H, TITLE_H,
};
use crate::arch::x86_64::{rtc, smp};
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
                if px >= 0 && py >= 0 && (px as usize) < fb::width() && (py as usize) < fb::height() {
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
        if px >= 0 && py >= 0 && (px as usize) < fb::width() && (py as usize) < fb::height() {
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

/// Corps du filigrane, en pixels (BOUCHAUD_ATELIER_MARQUE_V1).
const FILIGRANE_CORPS: f32 = crate::gui::atelier::jetons::CORPS_MARQUE;

/// Sous-titre de la marque : le nom de l'edition.
const SOUS_MARQUE: &str = "Atelier";

/// Hauteur reservee a la marque : titre, sous-titre, jambages et debord
/// d'antialiassage.
const FILIGRANE_HAUTEUR: usize = 84;

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
    let largeur = fb::text_width(FILIGRANE, FILIGRANE_CORPS, true);
    (
        (fb::width() / 2).saturating_sub(largeur / 2),
        // Au tiers haut du ciel, comme la maquette Atelier : la marque se pose
        // sur le paysage, elle ne flotte pas au-dessus du dock.
        fb::height() * 30 / 100,
    )
}

/// Cote du logo au-dessus de la marque.
const LOGO_MARQUE: usize = 72;

pub(crate) fn draw_filigrane() {
    let (x, y) = filigrane_origine();
    crate::gui::atelier::assets::pose(crate::gui::atelier::assets::LOGO_72,
        (fb::width() / 2).saturating_sub(LOGO_MARQUE / 2), y.saturating_sub(LOGO_MARQUE + 14));
    let blanc = crate::gui::atelier::jetons::ENCRE_SUR_FOND;
    // Une ombre d'un pixel, tres diffuse, detache le blanc du ciel clair.
    fb::draw_text_prop(x + 1, y + 2, FILIGRANE, 0x8a87b8, FILIGRANE_CORPS, true);
    fb::draw_text_prop(x, y, FILIGRANE, blanc, FILIGRANE_CORPS, true);
    let corps = crate::gui::atelier::jetons::CORPS_SOUS_MARQUE;
    let largeur = fb::text_width(SOUS_MARQUE, corps, false);
    let sx = (fb::width() / 2).saturating_sub(largeur / 2);
    let sy = y + FILIGRANE_CORPS as usize + 10;
    fb::draw_text_prop(sx + 1, sy + 1, SOUS_MARQUE, 0x8a87b8, corps, false);
    fb::draw_text_prop(sx, sy, SOUS_MARQUE, blanc, corps, false);
}

/// Bornes du filigrane. Volontairement large : un calque qui deborde ses
/// bornes laisserait des trainees, l'inverse ne coute qu'un peu de travail.
pub(crate) fn filigrane_rect() -> (usize, usize, usize, usize) {
    let (x, y) = filigrane_origine();
    let largeur = fb::text_width(FILIGRANE, FILIGRANE_CORPS, true);
    // Marge : `draw_text_prop` pose les glyphes a partir de `x`, mais un `B`
    // peut deborder d'un pixel a gauche et l'antialiassage d'un autre.
    // Le logo est au-dessus du titre : le calque l'inclut.
    let haut = y.saturating_sub(LOGO_MARQUE + 14);
    (x.saturating_sub(4), haut, largeur + 8, FILIGRANE_HAUTEUR + (y - haut))
}

/// Fond d'ecran seul.
pub(crate) fn draw_fond() {
    draw_wallpaper();
}

/// Barre superieure seule. `centre_ouvert` et le pointeur decident du voile
/// pose sur le bouton du centre de controle.
pub(crate) fn draw_barre_haute(centre_ouvert: bool, mx: i32, my: i32) {
    draw_topbar(centre_ouvert, bouton_centre().hit(mx, my));
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
    draw_topbar(false, false);

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
        fb::fill_rect_rgb(0, sommet + ligne, fb::width(), 1, couleur);
    }
    let filet = if filet_en_bas { sommet + BAR_H - 1 } else { sommet };
    fb::fill_rect_rgb(0, filet, fb::width(), 1, crate::gui::theme::COLOR_BORDER);
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

// BOUCHAUD_ATELIER_BARRE_HAUTE_V2
//
// Une barre de verre, pas un tableau de bord. A gauche la marque, au centre la
// date et l'heure, a droite UN bouton : l'etat du lien et le glyphe du centre
// de controle, qui l'ouvre.
//
// Ce qui n'y est plus, et pourquoi :
//   * les menus « Fichiers, Edition, Affichage, Aller, Aide » de l'increment 1
//     etaient INERTES -- aucune commande derriere. Une commande visuelle qui
//     ne fait rien est retiree, pas decoree ;
//   * le compteur de trames ne s'affiche qu'en mode diagnostic
//     (`atelier::reglages`, bascule dans le centre ou `BO_ATELIER_DIAGNOSTIC=1`).
//     Il reste dans la mise en page unique, et dans chaque ligne du journal ;
//   * Wi-Fi, son, notifications et recherche n'ont ni pilote ni service : la
//     barre ne montre pas d'icone pour eux. Le centre le dit en toutes lettres.

const JOURS: [&str; 7] = ["Dim.", "Lun.", "Mar.", "Mer.", "Jeu.", "Ven.", "Sam."];
const MOIS: [&str; 12] = ["janv.", "f\u{e9}vr.", "mars", "avr.", "mai", "juin", "juil.",
    "ao\u{fb}t", "sept.", "oct.", "nov.", "d\u{e9}c."];

/// Jour de la semaine (0 = dimanche), algorithme de Sakamoto.
fn jour_semaine(annee: u16, mois: u8, jour: u8) -> usize {
    const T: [u16; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let mois = mois.clamp(1, 12) as usize;
    let a = if mois < 3 { annee.saturating_sub(1) } else { annee } as usize;
    (a + a / 4 - a / 100 + a / 400 + T[mois - 1] as usize + jour as usize) % 7
}

fn draw_topbar(centre_ouvert: bool, survol_bouton: bool) {
    use crate::gui::atelier::jetons as j;
    crate::gui::atelier::matiere::bande_verre(0, 0, fb::width(), BAR_H, j::VERRE_BARRE);
    let corps = j::CORPS_BARRE;
    let ligne = ligne_texte_barre(0);

    // La marque, a gauche : un ASSET (logo provisoire,
    // BOUCHAUD_ATELIER_LOGO_PROVISOIRE_V1) et le nom.
    let x = j::ESPACE_4 as usize;
    crate::gui::atelier::assets::pose(crate::gui::atelier::assets::LOGO_18, x,
        (BAR_H - 18) / 2);
    fb::draw_text_prop(x + 18 + j::ESPACE_2 as usize, ligne, "Bouchaud OS", j::ENCRE, corps, true);

    // Date et heure, au centre.
    let dt = rtc::now();
    let horloge = format!("{} {} {}   {:02}:{:02}",
        JOURS[jour_semaine(dt.year, dt.month, dt.day)], dt.day,
        MOIS[(dt.month.clamp(1, 12) - 1) as usize], dt.hour, dt.minute);
    let largeur = fb::text_width(&horloge, corps, true);
    fb::draw_text_prop((fb::width() / 2).saturating_sub(largeur / 2), ligne, &horloge,
        j::ENCRE, corps, true);

    // Le bouton du centre de controle : voile au survol, accent quand il est
    // ouvert. Sa boite est `bouton_centre()`, la meme que le test de clic.
    let bouton = bouton_centre();
    if centre_ouvert || survol_bouton {
        let (teinte, opacite) = if centre_ouvert {
            (j::ACCENT, 0x2e)
        } else {
            (j::SURVOL, j::SURVOL_OPACITE)
        };
        crate::gui::atelier::matiere::pave(
            crate::gui::windowing::Rect::new(bouton.x, bouton.y, bouton.w as u32, bouton.h as u32),
            j::RAYON_PASTILLE, teinte, opacite);
    }

    // Zone d'etat, remplie de la DROITE vers la gauche
    // (BOUCHAUD_GUI_BARRE_HAUTE_UNE_SEULE_MISE_EN_PAGE_V1) : chaque element
    // prend sa largeur reelle et le suivant commence ou il s'arrete.
    let mut droite = (bouton.x + bouton.w) as usize - crate::gui::atelier::centre::BOUTON_REMBOURRAGE as usize;
    droite = droite.saturating_sub(GLYPHE_BARRE);
    crate::gui::atelier::assets::pose_masque(crate::gui::atelier::assets::SYMBOLE_REGLAGES_16,
        droite, (BAR_H - GLYPHE_BARRE) / 2, j::ENCRE, 0xff);
    droite = droite.saturating_sub(ECART_GLYPHE);
    dessine_reseau(droite, ligne);
    droite = droite.saturating_sub(largeur_reseau() + ECART_ELEMENTS);

    // Le compteur de trames : une MESURE, montree a qui la demande (mode
    // diagnostic). Toujours dans la meme mise en page, jamais par-dessus.
    if crate::gui::atelier::reglages::diagnostic() {
        let snapshot = crate::gui::frame_clock::snapshot();
        let fps = if snapshot.active {
            format!("FPS:{:3}", snapshot.fps_arrondi())
        } else {
            format!("FPS: --")
        };
        let corps_fps = corps - 1.0;
        let largeur = fb::text_width(&fps, corps_fps, false);
        droite = droite.saturating_sub(largeur);
        fb::draw_text_prop(droite, ligne + 1, &fps, j::ENCRE_TENUE, corps_fps, false);
    }
}

/// Ecart entre deux elements de la partie droite de la barre, en pixels.
const ECART_ELEMENTS: usize = 14;
/// Cote des glyphes Lucide de la barre.
const GLYPHE_BARRE: usize = 16;
/// Ecart entre le libelle reseau et le glyphe du centre de controle.
const ECART_GLYPHE: usize = 10;
/// Ecart entre un glyphe et son libelle.
const ECART_LIBELLE: usize = 5;

/// Largeur de la grappe d'etat : indicateur reseau, ecart, glyphe du centre.
fn largeur_grappe() -> usize {
    largeur_reseau() + ECART_GLYPHE + GLYPHE_BARRE
}

/// Le bouton de la barre qui ouvre le centre de controle. Peintre ET test de
/// clic : une seule definition (`atelier::centre::bouton_barre`).
pub(crate) fn bouton_centre() -> window::Rect {
    let b = crate::gui::atelier::centre::bouton_barre(fb::width() as u32, BAR_H as u32,
        largeur_grappe() as u32);
    window::Rect { x: b.x, y: b.y, w: b.w as i32, h: b.h as i32 }
}

/// Ce que l'indicateur reseau doit montrer, a cet instant.
///
/// Separe du dessin parce que c'est la seule moitie qui se raisonne : trois
/// etats, et une regle pour chacun. Le dessin, lui, n'a pas d'opinion.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EtatReseau {
    /// Aucune carte, ou une carte que personne ne pilote.
    SansCarte,
    /// Carte prete, mais rien au bout du cable -- ou pas de cable.
    Deconnecte,
    /// LE CABLE PORTE, MAIS IL N'Y A PAS D'ADRESSE.
    ///
    /// Sur la TRIGKEY, la barre affichait « Ethernet deconnecte » pendant que
    /// la fenetre Services montrait « rtl8168 Actif » avec des octets qui
    /// circulaient. Les deux disaient vrai : le lien est monte ET aucune
    /// adresse IPv4 n'a ete obtenue -- sur materiel reel on ne fabrique
    /// jamais les adresses SLIRP de QEMU.
    ///
    /// Un seul mot recouvrait deux situations qui n'appellent pas le meme
    /// geste : verifier un cable, ou chercher un serveur DHCP.
    SansAdresse,
    /// Lien monte et configuration obtenue.
    Connecte,
}

/// L'etat courant, lu une seule fois par trame.
pub fn etat_reseau() -> EtatReseau {
    if !crate::net::carte_presente() {
        EtatReseau::SansCarte
    } else if crate::net::connecte() {
        EtatReseau::Connecte
    } else if crate::net::qualite_lien().vitesse_mbps != 0
        || matches!(
            crate::net::etat_demarrage(),
            crate::net::Demarrage::SansConfiguration | crate::net::Demarrage::SansBail
        )
    {
        // Le lien porte ; c'est la configuration qui manque.
        EtatReseau::SansAdresse
    } else {
        EtatReseau::Deconnecte
    }
}

/// Le libelle qui accompagne l'icone.
///
/// Un reseau filaire n'a pas de SSID : son nom est celui que le serveur DHCP
/// annonce (option 15), a defaut son sous-reseau. Quand il n'y a rien a
/// nommer, l'etat se dit en toutes lettres plutot que par une chaine vide,
/// qui laisserait croire a un defaut d'affichage.
pub fn libelle_reseau(etat: EtatReseau) -> String {
    match etat {
        EtatReseau::SansCarte => String::from("Pas de carte"),
        EtatReseau::Deconnecte => String::from("Ethernet deconnecte"),
        EtatReseau::SansAdresse => String::from("Ethernet sans adresse IP"),
        EtatReseau::Connecte => {
            let nom = crate::net::nom_reseau();
            let qualite = crate::net::qualite_lien();
            let debit = debit_lisible(qualite.vitesse_mbps);
            let base = if nom.is_empty() { String::from("Ethernet") } else { nom };
            if debit.is_empty() {
                base
            } else if qualite.duplex_complet {
                format!("{} - {}", base, debit)
            } else {
                // L'ALTERNAT SE DIT, ET NE SE DEVINE PAS.
                //
                // Sur du cuivre moderne, un lien a l'alternat signale presque
                // toujours une negociation ratee d'un cote. Les collisions y
                // divisent le debit utile, et le chiffre de vitesse seul
                // laisserait croire a une liaison saine.
                format!("{} - {} alternat", base, debit)
            }
        }
    }
}

/// Le debit negocie, ecrit comme on l'ecrit. Vide quand on ne le connait pas.
///
/// Un zero signifierait « zero megabit », ce qui est faux : il signifie « ce
/// pilote ne sait pas le lire ». Une chaine vide ne raconte rien, et c'est
/// exactement ce qu'il faut dire.
fn debit_lisible(mbps: u32) -> String {
    match mbps {
        0 => String::new(),
        1000 => String::from("1 Gb/s"),
        2500 => String::from("2,5 Gb/s"),
        _ => format!("{} Mb/s", mbps),
    }
}

/// Couleur du glyphe et du libelle de l'indicateur, pour un etat donne.
///
/// UN LIEN MONTE N'EST PAS FORCEMENT UN LIEN SAIN. L'alternat, ou dix
/// megabits sur un port gigabit, sont des liens qui fonctionnent mal. Les
/// peindre comme un gigabit duplex integral ferait chercher la lenteur
/// ailleurs.
fn teintes_reseau(etat: EtatReseau) -> (u32, u32) {
    use crate::gui::atelier::jetons as j;
    let degrade = matches!(etat, EtatReseau::Connecte) && {
        let q = crate::net::qualite_lien();
        !q.duplex_complet || (q.vitesse_mbps != 0 && q.vitesse_mbps < 100)
    };
    match etat {
        EtatReseau::Connecte if degrade => (crate::gui::theme::COLOR_WARNING, j::ENCRE),
        // Sain : l'encre de la barre. Rien a signaler ne se colore pas.
        EtatReseau::Connecte => (j::ENCRE, j::ENCRE),
        EtatReseau::Deconnecte => (j::DANGER, j::ENCRE_DOUCE),
        // Le lien porte : ce n'est pas une panne, c'est une configuration
        // qui manque. L'orange appelle un serveur DHCP, le rouge un cable.
        EtatReseau::SansAdresse => (crate::gui::theme::COLOR_WARNING, j::ENCRE),
        EtatReseau::SansCarte => (j::ENCRE_TENUE, j::ENCRE_DOUCE),
    }
}

/// Le glyphe Lucide de l'etat, a la taille demandee (16 barre, 18 centre).
///
/// L'ETAT SE LIT PAR LA FORME, PAS SEULEMENT PAR LA COULEUR : un ecran mal
/// regle, ou un daltonien, doivent voir la difference. L'increment 1 dessinait
/// une prise en rectangles et une barre oblique ; ce sont maintenant trois
/// glyphes distincts, rendus depuis Lucide (ISC) par la fabrique d'assets.
fn glyphe_reseau(etat: EtatReseau, grand: bool) -> &'static [u8] {
    use crate::gui::atelier::assets as a;
    if etat != EtatReseau::Connecte {
        match (etat, grand) {
            (EtatReseau::SansAdresse, false) => a::SYMBOLE_ALERTE_16,
            (EtatReseau::SansAdresse, true) => a::SYMBOLE_ALERTE_18,
            (_, false) => a::SYMBOLE_DEBRANCHE_16,
            (_, true) => a::SYMBOLE_DEBRANCHE_18,
        }
    } else if grand {
        a::SYMBOLE_ETHERNET_18
    } else {
        a::SYMBOLE_ETHERNET_16
    }
}

/// Dessine le glyphe d'etat et son libelle, le libelle se terminant en `droite`.
///
/// # Pourquoi une prise et non des barres de signal
///
/// Des barres disent une FORCE de signal, que le cuivre n'a pas : un lien
/// Ethernet est monte ou ne l'est pas. Montrer trois barres sur cinq pour une
/// liaison gigabit parfaite serait une information fausse.
fn dessine_reseau(droite: usize, ligne: usize) {
    let etat = etat_reseau();
    let libelle = libelle_reseau(etat);
    let (couleur, couleur_texte) = teintes_reseau(etat);
    let largeur_texte = fb::text_width(&libelle, CORPS_BARRE - 1.0, false);
    let x_texte = droite.saturating_sub(largeur_texte);
    let x_glyphe = x_texte.saturating_sub(GLYPHE_BARRE + ECART_LIBELLE);
    crate::gui::atelier::assets::pose_masque(glyphe_reseau(etat, false), x_glyphe,
        (BAR_H - GLYPHE_BARRE) / 2, couleur, 0xff);
    fb::draw_text_prop(x_texte, ligne, &libelle, couleur_texte, CORPS_BARRE - 1.0, false);
}

/// Largeur totale occupee par l'indicateur reseau, libelle compris.
///
/// Le compositeur en a besoin pour invalider la bonne bande : l'indicateur
/// change de largeur quand le nom du reseau change, et n'invalider que la
/// largeur courante laisserait la trainee de l'ancien nom a l'ecran.
pub fn largeur_reseau() -> usize {
    let libelle = libelle_reseau(etat_reseau());
    fb::text_width(&libelle, CORPS_BARRE - 1.0, false) + GLYPHE_BARRE + ECART_LIBELLE
}

// ─── Centre de controle ────────────────────────────────────────────────────
//
// BOUCHAUD_ATELIER_CENTRE_V1. Geometrie : `atelier::centre`. Tout ce qui est
// peint ici est soit LU au runtime (reseau, affichage, reglage), soit une
// action qui existe deja (alimentation). Rien n'est simule.

fn rect_de(b: crate::gui::atelier::dock::Boite) -> crate::gui::windowing::Rect {
    crate::gui::windowing::Rect::new(b.x, b.y, b.w.max(0) as u32, b.h.max(0) as u32)
}

fn ipv4_lisible(a: [u8; 4]) -> String {
    format!("{}.{}.{}.{}", a[0], a[1], a[2], a[3])
}

/// Peint le centre de controle. Le survol vient de `centre::action_sous`, la
/// meme fonction que le clic et l'invalidation.
pub(crate) fn draw_centre(mx: i32, my: i32) {
    use crate::gui::atelier::{assets as a, centre as c, jetons as j, matiere};
    let l = fb::width() as u32;
    let b = BAR_H as u32;
    let survol = c::action_sous(l, b, mx, my);
    matiere::plaque_verre(rect_de(c::cadre(l, b)), c::RAYON, 0xe8);
    let corps = j::CORPS_BARRE;
    let petit = CORPS_BARRE - 1.0;

    // Titre.
    let t = c::bloc(l, b, c::Bloc::Titre);
    fb::draw_text_prop(t.x as usize + 2, t.y as usize + 4, "Centre Atelier", j::ENCRE, corps, true);

    let carte = |bloc: c::Bloc, survole: bool| {
        let boite = c::bloc(l, b, bloc);
        matiere::pave(rect_de(boite), c::RAYON_BLOC, 0xffffff, 0xb8);
        if survole {
            matiere::pave(rect_de(boite), c::RAYON_BLOC, j::SURVOL, j::SURVOL_OPACITE);
        }
        boite
    };

    // Reseau : l'etat courant, lu comme la barre le lit.
    let r = carte(c::Bloc::Reseau, false);
    let etat = etat_reseau();
    let (teinte, _) = teintes_reseau(etat);
    a::pose_masque(glyphe_reseau(etat, true), r.x as usize + 12, r.y as usize + 12, teinte, 0xff);
    fb::draw_text_prop(r.x as usize + 40, r.y as usize + 10, &libelle_reseau(etat), j::ENCRE, corps, true);
    let detail = match etat {
        EtatReseau::Connecte => format!("IPv4 {}  \u{b7}  passerelle {}",
            ipv4_lisible(crate::net::our_ip()), ipv4_lisible(crate::net::gateway())),
        EtatReseau::SansAdresse => String::from("Le lien porte, aucun serveur DHCP n'a r\u{e9}pondu"),
        EtatReseau::Deconnecte => String::from("Aucun c\u{e2}ble ou aucun lien en face"),
        EtatReseau::SansCarte => String::from("Aucune carte r\u{e9}seau pilot\u{e9}e"),
    };
    fb::draw_text_prop(r.x as usize + 40, r.y as usize + 32, &detail, j::ENCRE_DOUCE, petit, false);

    // Affichage : ce que le pilote a negocie. Le rendu est LOGICIEL et le dit.
    let d = carte(c::Bloc::Affichage, false);
    a::pose_masque(a::SYMBOLE_ECRAN_18, d.x as usize + 12, d.y as usize + 13, j::ENCRE_DOUCE, 0xff);
    let ecran = format!("{} \u{d7} {}  \u{b7}  rendu logiciel", fb::width(), fb::height());
    fb::draw_text_prop(d.x as usize + 40, d.y as usize + 14, &ecran, j::ENCRE, petit, false);

    // Mode diagnostic : un vrai reglage, un vrai interrupteur.
    let actif = crate::gui::atelier::reglages::diagnostic();
    let g = carte(c::Bloc::Diagnostic, survol == Some(c::Action::Diagnostic));
    a::pose_masque(a::SYMBOLE_DIAGNOSTIC_18, g.x as usize + 12, g.y as usize + 13, j::ENCRE_DOUCE, 0xff);
    fb::draw_text_prop(g.x as usize + 40, g.y as usize + 14, "Compteur de trames (FPS)",
        j::ENCRE, petit, false);
    let interrupteur = crate::gui::windowing::Rect::new(g.x + g.w as i32 - 48, g.y + 12, 36, 20);
    matiere::pave(interrupteur, 10, if actif { j::ACCENT } else { j::ENCRE_TENUE }, 0xff);
    let bouton_x = if actif { interrupteur.x + 18 } else { interrupteur.x + 2 };
    matiere::pave(crate::gui::windowing::Rect::new(bouton_x, interrupteur.y + 2, 16, 16), 8,
        0xffffff, 0xff);

    // Mouvement des fenetres : trois segments, un reglage reel
    // (`atelier::reglages::animations`, lu par `atelier::effets`).
    let m = carte(c::Bloc::Animations, false);
    fb::draw_text_prop(m.x as usize + 12, m.y as usize + 14, "Animations", j::ENCRE, petit, false);
    let courant = crate::gui::atelier::reglages::animations();
    for (action, libelle, mode) in [
        (c::Action::AnimationsNormales, "Normales", crate::gui::atelier::animations::Mode::Normal),
        (c::Action::AnimationsReduites, "R\u{e9}duites", crate::gui::atelier::animations::Mode::Reduit),
        (c::Action::AnimationsAucune, "Aucune", crate::gui::atelier::animations::Mode::Aucun),
    ] {
        let s = c::boite_action(l, b, action);
        let choisi = courant == mode;
        if choisi {
            matiere::pave(rect_de(s), 8, j::ACCENT, 0xff);
        } else if survol == Some(action) {
            matiere::pave(rect_de(s), 8, j::SURVOL, j::SURVOL_OPACITE);
        }
        let lw = fb::text_width(libelle, petit, false) as i32;
        fb::draw_text_prop((s.x + (s.w - lw) / 2).max(0) as usize, s.y as usize + 6, libelle,
            if choisi { 0xffffff } else { j::ENCRE_DOUCE }, petit, false);
    }

    // Ce qui n'existe pas : dit, pas simule.
    let n = c::bloc(l, b, c::Bloc::Indisponibles);
    fb::draw_text_prop(n.x as usize + 4, n.y as usize + 4,
        "Non pris en charge pour l'instant :", j::ENCRE_TENUE, petit, false);
    fb::draw_text_prop(n.x as usize + 4, n.y as usize + 19,
        "Wi-Fi, son, th\u{e8}me sombre.", j::ENCRE_TENUE, petit, false);

    // Alimentation : les memes chemins que le menu Applications.
    for action in c::ALIMENTATION {
        let boite = c::boite_action(l, b, action);
        let survole = survol == Some(action);
        matiere::pave(rect_de(boite), c::RAYON_BLOC, 0xffffff, 0xb8);
        if survole {
            let teinte = if action == c::Action::Eteindre { j::DANGER } else { j::SURVOL };
            matiere::pave(rect_de(boite), c::RAYON_BLOC, teinte,
                if action == c::Action::Eteindre { 0x2a } else { j::SURVOL_OPACITE });
        }
        let (glyphe, libelle) = match action {
            c::Action::Redemarrer => (a::SYMBOLE_REDEMARRER_18, "Red\u{e9}marrer"),
            c::Action::Eteindre => (a::SYMBOLE_ETEINDRE_18, "\u{c9}teindre"),
            _ => (a::SYMBOLE_QUITTER_18, "Quitter"),
        };
        let teinte = if survole && action == c::Action::Eteindre { j::DANGER } else { j::ENCRE };
        a::pose_masque(glyphe, (boite.x + boite.w as i32 / 2 - 9) as usize, boite.y as usize + 10,
            teinte, 0xff);
        let lw = fb::text_width(libelle, petit, false) as i32;
        fb::draw_text_prop((boite.x + (boite.w as i32 - lw) / 2).max(0) as usize,
            boite.y as usize + 36, libelle, j::ENCRE, petit, false);
    }
}

#[allow(dead_code)]
fn sys_stats_str() -> String {
    let total_cpu = timer::cpu_load_pct();
    let online = smp::schedulable_cpus().max(1).min(smp::MAX_CPUS);
    let reported = crate::platform::pc::hardware_facts::reported_cpus().max(online);
    let (heap_used, _heap_free, _heap_pool) = crate::kernel::heap::stats();
    let usable_ram = crate::platform::pc::hardware_facts::usable_ram_bytes()
        .min(usize::MAX as u64) as usize;
    let fs = ramfs::fs();
    let nodes = fs.used_nodes();
    drop(fs);
    let usb = if crate::platform::pc::hardware_facts::xhci_active() {
        format!(
            "USB:{}p K{} M{}",
            crate::platform::pc::hardware_facts::xhci_connected_ports(),
            crate::drivers::xhci_active::hid_keyboards(),
            crate::drivers::xhci_active::hid_mice(),
        )
    } else if crate::platform::pc::hardware_facts::xhci_present() {
        String::from("xHCI:probe")
    } else {
        String::from("xHCI:none")
    };
    format!(
        "CPU:{total_cpu:3}% cores:{online}/{reported}  Heap:{}/RAM:{}  RAMFS:{}nodes  {}",
        human_bytes(heap_used), human_bytes(usable_ram), nodes, usb
    )
}

#[allow(dead_code)]
fn human_bytes(n: usize) -> String {
    if n >= 1_073_741_824 { format!("{}Go", n / 1_073_741_824) }
    else if n >= 1_048_576 { format!("{}Mo", n / 1_048_576) }
    else if n >= 1_024    { format!("{}Ko", n / 1_024) }
    else                  { format!("{}o", n) }
}

fn draw_wallpaper() {
    // BOUCHAUD_ATELIER_FOND_V1 : le paysage d'aube Atelier, calcule une fois
    // a la resolution courante puis recopie sous la decoupe.
    crate::gui::atelier::fond::dessine();
}

// ─── Icônes ────────────────────────────────────────────────────────────────
//
// BOUCHAUD_ATELIER_ASSETS_V1 : les icones sont des ASSETS -- tuiles Atelier
// (degrade + glyphe Lucide) rendues par `tools/assets/fabrique-atelier.py` a
// chaque taille posee. `gui::icones` choisit le PNG, le decode une fois et le
// recopie ; rien n'est dessine ici.

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
            // `composable`, et non la surface allouee : depuis que celle-ci est
            // dimensionnee pour une fenetre MAXIMISEE, elle est plus grande que
            // la zone dans le cas ordinaire, et promettre de peindre ce que le
            // client n'a pas dessine laisserait une bande de fond sale.
            let (peinte_l, peinte_h) = client.composable();
            let largeur = (zone.largeur.max(0) as usize).min(peinte_l) as u32;
            let hauteur = (zone.hauteur.max(0) as usize).min(peinte_h) as u32;
            (largeur > 0 && hauteur > 0)
                .then(|| crate::gui::windowing::Rect::new(zone.x, zone.y, largeur, hauteur))
        }
        _ => None,
    };
    // BOUCHAUD_ATELIER_FENETRE_V1 : ombre douce melangee (plus de cadre gris
    // opaque), surface claire, filet de bordure. Memes bornes que
    // `paint_window_shape_spans` : le plan de scene n'a rien a apprendre.
    crate::gui::atelier::matiere::fenetre(geometry.outer, crate::gui::theme::RADIUS_WINDOW,
        crate::gui::windowing::manager::SHADOW_EXTENT, focused,
        crate::gui::theme::COLOR_SURFACE, opaque);

    let title_h = TITLE_H as usize;
    let title_color = if focused { crate::gui::atelier::jetons::TITRE_ACTIF }
        else { crate::gui::atelier::jetons::TITRE_INACTIF };
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
        crate::gui::atelier::jetons::SEPARATEUR);

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
    let encre_titre = if focused { crate::gui::atelier::jetons::ENCRE }
        else { crate::gui::atelier::jetons::ENCRE_DOUCE };
    fb::draw_text_prop(origine_titre, y + (title_h - 12) / 2, title_clipped,
        encre_titre, crate::gui::atelier::jetons::CORPS_TITRE, focused);
    // BOUCHAUD_P19_BROWSER_LOADING_SINGLE_OWNER_V1
    // La duree noyau reste dans la barre de titre PENDANT le demarrage
    // uniquement. La navigation appartient exclusivement au chrome Ladybird.
    if matches!(&w.app, App::Navigateur { client }
        if client.etat == crate::gui::client::Etat::Demarrage) {
        dessine_duree_chargement(w, origine_titre, fb::text_width(title_clipped, 12.0, false),
            premier_bouton, y + (title_h - 11) / 2);
    }

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
    use crate::gui::atelier::jetons as j;
    use crate::gui::windowing::{close_button_rect, maximize_button_rect,
        minimize_button_rect, HitRegion, WINDOW_CHROME};
    let buttons = [(minimize_button_rect(outer, WINDOW_CHROME), HitRegion::Minimize),
        (maximize_button_rect(outer, WINDOW_CHROME), HitRegion::Maximize),
        (close_button_rect(outer, WINDOW_CHROME), HitRegion::Close)];
    for (rect, region) in buttons {
        let survole = hover == region;
        // BOUCHAUD_ATELIER_BOUTONS_V1 : une pastille arrondie au survol, en
        // retrait du rectangle d'impact -- le bouton reste aussi grand a viser,
        // il parait juste plus calme.
        if survole {
            let pastille = crate::gui::windowing::Rect::new(rect.x + 8, rect.y + 5,
                rect.width.saturating_sub(16), rect.height.saturating_sub(10));
            if region == HitRegion::Close {
                crate::gui::atelier::matiere::pave(pastille, j::RAYON_PASTILLE, j::DANGER, 255);
            } else {
                crate::gui::atelier::matiere::pave(pastille, j::RAYON_PASTILLE, j::SURVOL, 0x1c);
            }
        }
        let color = if survole && region == HitRegion::Close { 0xffffff }
            else if focused { j::ENCRE_DOUCE } else { j::ENCRE_TENUE };
        let cx = rect.x + rect.width as i32 / 2;
        let cy = rect.y + rect.height as i32 / 2;
        match region {
            HitRegion::Minimize => fb::fill_rect_rgb((cx - 5) as usize, cy as usize, 10, 1, color),
            HitRegion::Maximize => {
                let carre = crate::gui::windowing::Rect::new(cx - 5, cy - 5, 10, 10);
                crate::gui::atelier::matiere::filet(carre, 2, color, 255);
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
    // La meme borne que la promesse d'opacite de `draw_window` : les deux
    // doivent decrire le MEME rectangle, sinon l'un peint un fond que l'autre
    // recouvre, ou pire, laisse a decouvert ce que personne ne peint.
    let (peinte_l, peinte_h) = client.composable();
    let hauteur = (zone.hauteur as usize).min(peinte_h);
    let largeur = (zone.largeur as usize).min(peinte_l);
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
        // La FACE : avec deux tampons (BOUCHAUD_GUI_SURFACE_DEUX_TAMPONS_V1),
        // jamais celui dans lequel le client peut ecrire.
        surface.copie_ligne_de(client.face(), y - zy, colonne, pris, destination);
        client.note_lecture(destination);
    }
    fb::note_pixels_dessines(((x_fin - x_debut) * (y_fin - y_debut)) as u64);

    // BOUCHAUD_P19_BROWSER_LOADING_SINGLE_OWNER_V1
    // Ne JAMAIS superposer une jauge OS aux pixels de WebContent :
    // BouchaudChrome.h possede l'indicateur unique sous la barre URL.
    // Les mesures Jauge sont conservees dans le client pour la telemetrie.
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

// BOUCHAUD_P19_BROWSER_LOADING_SINGLE_OWNER_V1 : l ancienne jauge
// graphique d application est retiree. Seule la jauge de DEMARRAGE demeure.

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

// BOUCHAUD_ATELIER_DOCK_V1
//
// La barre des taches devient un DOCK : une pastille de verre centree, qui
// flotte au-dessus du fond avec sa propre ombre. A gauche, la cellule
// Applications (le menu) ; apres le separateur, une cellule par fenetre, dans
// l'ordre de la pile -- la meme liste que l'ancienne barre, donc le meme test
// d'impact (`window::start_btn`, `window::taskbar_btn`) et les memes degats.
//
// Les etats se lisent sans texte : la fenetre au premier plan porte un point
// d'accent sous son icone, une fenetre rangee un point tenu, et la cellule
// survolee un voile.

/// Pictogramme Applications : le glyphe Lucide `layout-grid`, teinte.
fn pictogramme_applications(cellule: crate::gui::atelier::dock::Boite, actif: bool) {
    use crate::gui::atelier::{assets, jetons as j};
    let cote = 22;
    let x = cellule.x + (cellule.w - cote) / 2;
    let y = cellule.y + (cellule.h - cote) / 2;
    assets::pose_masque(assets::SYMBOLE_GRILLE_22, x.max(0) as usize, y.max(0) as usize,
        if actif { j::ACCENT } else { j::ENCRE_DOUCE }, 255);
}

fn voile_cellule(cellule: crate::gui::atelier::dock::Boite, opacite: u8) {
    let r = crate::gui::windowing::Rect::new(cellule.x, cellule.y, cellule.w as u32,
        cellule.h as u32);
    crate::gui::atelier::matiere::pave(r, crate::gui::atelier::jetons::RAYON_CELLULE,
        crate::gui::atelier::jetons::SURVOL, opacite);
}

/// Survol du dock a l'instant de la trame : la cible sous le pointeur, son
/// avancement (0..=PLEIN), la levee de son icone.
fn survol_dock(n: usize, mx: i32, my: i32) -> Option<(crate::gui::atelier::dock::Cible, u32, i32)> {
    use crate::gui::atelier::{dock, effets, mouvement::PLEIN};
    let cible = dock::cible_sous(fb::width() as u32, fb::height() as u32, n, mx, my)?;
    let p = effets::survol(cible);
    let levee = if effets::deplacement_permis() { dock::LEVEE * p as i32 / PLEIN as i32 } else { 0 };
    Some((cible, p, levee))
}

/// Libelle de l'infobulle d'une cible.
fn libelle_cible(wins: &[Win], cible: crate::gui::atelier::dock::Cible) -> String {
    match cible {
        crate::gui::atelier::dock::Cible::Applications => String::from("Applications"),
        crate::gui::atelier::dock::Cible::Fenetre(i) => match wins.get(i) {
            Some(w) => {
                let titre: String = w.title.chars().take(40).collect();
                if w.min { format!("{} (r\u{e9}duite)", titre) } else { titre }
            }
            None => String::new(),
        },
    }
}

/// L'infobulle du dock sous le pointeur : sa cible et sa boite. Une seule
/// definition pour le plan de scene, l'invalidation et le peintre.
pub(crate) fn infobulle_dock(wins: &[Win], mx: i32, my: i32)
    -> Option<(crate::gui::atelier::dock::Cible, crate::gui::atelier::dock::Boite)>
{
    use crate::gui::atelier::dock;
    let (l, h) = (fb::width() as u32, fb::height() as u32);
    let cible = dock::cible_sous(l, h, wins.len(), mx, my)?;
    let texte = fb::text_width(&libelle_cible(wins, cible), CORPS_INFOBULLE, false) as i32;
    Some((cible, dock::infobulle(l, h, wins.len(), cible, texte)))
}

const CORPS_INFOBULLE: f32 = 12.0;

/// Peint l'infobulle du dock (calque `Element::Infobulle`).
pub(crate) fn draw_infobulle(wins: &[Win], mx: i32, my: i32) {
    use crate::gui::atelier::{jetons as j, matiere, mouvement::PLEIN};
    let Some((cible, b)) = infobulle_dock(wins, mx, my) else { return };
    let p = crate::gui::atelier::effets::survol(cible);
    let fond = (0xe6 * p / PLEIN) as u8;
    if fond == 0 {
        return;
    }
    let r = crate::gui::windowing::Rect::new(b.x, b.y, b.w as u32, b.h as u32);
    matiere::pave(r, 8, j::ENCRE, fond);
    let texte = libelle_cible(wins, cible);
    // Texte opaque : la police n'a pas d'opacite propre. Il n'apparait qu'a
    // mi-fondu, quand le fond est assez dense pour le porter.
    if p * 2 >= PLEIN {
        fb::draw_text_prop((b.x + crate::gui::atelier::dock::INFOBULLE_REMBOURRAGE) as usize,
            (b.y + 5) as usize, &texte, 0xffffff, CORPS_INFOBULLE, false);
    }
}

pub(crate) fn draw_taskbar(wins: &[Win], menu_open: bool, mx: i32, my: i32) {
    use crate::gui::atelier::{dock, jetons as j, matiere};
    let (l, h) = (fb::width() as u32, fb::height() as u32);
    let n = wins.len();
    let survol = survol_dock(n, mx, my);
    let cadre = dock::cadre(l, h, n);
    // BOUCHAUD_GFX_CULLING_AMONT_V1 : un degat dans la bande qui ne touche pas
    // le dock (ses cotes) n'a rien a peindre ici -- le fond l'a deja fait.
    if !fb::decoupe_touche((cadre.x - dock::OMBRE).max(0) as usize,
        (cadre.y - dock::OMBRE).max(0) as usize,
        (cadre.w + 2 * dock::OMBRE) as usize, (cadre.h + 2 * dock::OMBRE) as usize) {
        return;
    }
    matiere::plaque_verre(crate::gui::windowing::Rect::new(cadre.x, cadre.y,
        cadre.w as u32, cadre.h as u32), j::RAYON_DOCK, j::VERRE_DOCK);

    // BOUCHAUD_ATELIER_DOCK_SURVOL_V1 : voile et levee de la cellule survolee.
    // Le gestionnaire invalide la cellule quittee ET la cellule atteinte ; la
    // cible vient de `dock::cible_sous`, la meme que le clic.
    let voile = |cible: dock::Cible| -> i32 {
        match survol {
            Some((c, p, levee)) if c == cible => {
                let o = (0x1e * p / crate::gui::atelier::mouvement::PLEIN) as u8;
                if o > 0 {
                    voile_cellule(dock::boite_cible(l, h, n, cible), o);
                }
                levee
            }
            _ => 0,
        }
    };
    let apps = dock::cellule_applications(l, h, n);
    if menu_open {
        voile_cellule(apps, 0x24);
    }
    let levee = voile(dock::Cible::Applications);
    pictogramme_applications(dock::Boite { y: apps.y - levee, ..apps }, menu_open);

    if dock::visibles(l, n) == 0 {
        return;
    }
    // Separateur : un filet vertical court, centre.
    let sx = dock::x_separateur(l, h, n);
    let marge = dock::CELLULE / 4;
    for y in (apps.y + marge)..(apps.y + apps.h - marge) {
        fb::blend_rect_rgb(sx.max(0) as usize, y.max(0) as usize, 1, j::FILET, 0x30);
    }

    let focus = indice_focus(wins);
    for (i, w) in wins.iter().enumerate() {
        let c = dock::cellule_fenetre(l, h, n, i);
        if c.w == 0 { break; }
        if !fb::decoupe_touche(c.x.max(0) as usize, c.y.max(0) as usize,
            c.w as usize, (c.h + dock::REMBOURRAGE) as usize) { continue }
        let levee = voile(dock::Cible::Fenetre(i));
        let ix = c.x + (c.w - dock::ICONE) / 2;
        let iy = c.y + (c.h - dock::ICONE) / 2 - 2 - levee;
        match crate::gui::icones::pour_app(&w.app) {
            Some(icone) => crate::gui::icones::dessine(icone, ix.max(0) as usize,
                iy.max(0) as usize, crate::gui::icones::TAILLE_DOCK),
            None => {
                // Une application sans image garde une pastille a son initiale :
                // une cellule vide ne se distinguerait pas d'un trou.
                let r = crate::gui::windowing::Rect::new(ix, iy, dock::ICONE as u32,
                    dock::ICONE as u32);
                matiere::pave(r, j::RAYON_CELLULE, j::ACCENT, 255);
                let initiale: String = w.title.chars().take(1).collect();
                let lw = fb::text_width(&initiale, 18.0, true);
                fb::draw_text_prop((ix + (dock::ICONE - lw as i32) / 2).max(0) as usize,
                    (iy + 10).max(0) as usize, &initiale, 0xffffff, 18.0, true);
            }
        }
        // Indicateur sous l'icone.
        let actif = focus == Some(i) && !w.min;
        let (largeur, couleur, opacite) = if actif { (14, j::ACCENT, 255) }
            else if w.min { (4, j::ENCRE_TENUE, 160) } else { (4, j::ENCRE_DOUCE, 220) };
        let point = crate::gui::windowing::Rect::new(c.x + (c.w - largeur) / 2,
            c.y + c.h + 1, largeur as u32, 4);
        matiere::pave(point, 2, couleur, opacite);
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

    // LE SEPARATEUR SE DESIGNE PAR SON LIBELLE, PAS PAR SA POSITION.
    //
    // Il valait `MENU.len() - 1`, avec le commentaire « index de Quitter ».
    // C'etait vrai quand Quitter fermait la liste ; l'ajout de Redemarrer et
    // Eteindre l'a fait designer la derniere entree, et le trait a glisse d'un
    // cran sans que rien ne le dise.
    //
    // La table le dit deja d'elle-meme : « le `kind` est explicite et non
    // deduit de la position ». Le separateur suit la meme regle.
    let sep_idx = MENU
        .iter()
        .position(|(libelle, _)| *libelle == "Quitter")
        .unwrap_or(MENU.len());
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
    let px = mx.min(fb::width().saturating_sub(1));
    let py = my.min(fb::height().saturating_sub(1));
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
                            if px2 < fb::width() && py2 < fb::height() {
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
            if bits & (1 << col) != 0 && mx + col < fb::width() && my + row < fb::height() {
                fb::pixel_rgb(mx + col, my + row, fill);
            }
        }
    }
}
