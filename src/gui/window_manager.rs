//! Boucle d'evenements du gestionnaire de fenetres : entree souris/clavier,
//! focus / z-order, deplacement / redimensionnement, composition et rendu.
//!
//! ## Le bureau est une tache
//!
//! Il l'est depuis que le navigateur doit tourner **pendant** que le bureau
//! vit. Auparavant, ouvrir le navigateur appelait `exec`, qui ne rendait la main
//! qu'a la mort du programme : le bureau etait gele, sa barre des taches et son
//! horloge disparaissaient de l'ecran avec lui. Le bureau passe donc par
//! [`task::run_noyau`] et devient un fil noyau ordonnance comme les autres, ce
//! qui permet a l'ordonnanceur d'alterner entre lui et les clients.
//!
//! Un fil noyau n'est jamais preempte — l'IRQ0 ne commute que depuis le ring 3.
//! Une composition n'est donc jamais coupee en son milieu, et c'est ce qui rend
//! une surface simple (sans double tampon) suffisante pour ce jalon : le client
//! ne peut pas ecrire pendant qu'on le lit, faute de pouvoir s'executer.
//!
//! ## Le bureau ne redessine que s'il se passe quelque chose
//!
//! La boucle dessinait autrefois a chaque tour, c'est-a-dire aussi vite que le
//! processeur le permettait — le PIT bat a 1 kHz. Tant que rien d'autre ne
//! tournait, cela ne se voyait pas. Face a un navigateur qui a besoin du meme
//! processeur, c'est une famine. On ne redessine donc que sur evenement, et au
//! plus [`PERIODE_TRAME_MS`] fois par seconde.

use crate::gui::apps;
use crate::gui::client::{self, Client};
use crate::gui::event::{Key, KeyEvent};
use crate::gui::framebuffer as fb;
use crate::gui::mouse;
use crate::gui::degats::{Degats, Origine};
use crate::gui::disposition;
use crate::gui::transition;
use crate::gui::chaine::{Veilleur, Verdict};
use crate::gui::protocole::Rect;
use crate::gui::widgets;
use crate::gui::window::{
    self as window,
    clamp_win, icon_rect, make_app, menu_rect, start_btn, taskbar_btn,
    zone_utile, App, Drag, Win, BAR_H, ICONS, MENU, MIN_H, MIN_W,
    NAV_HAUTEUR, NAV_LARGEUR, TITLE_H,
};
use crate::drivers::keyboard;
use crate::fs::ramfs;
use crate::gui::politique;
use crate::gui::scene::{self, Calque, Element};
use crate::gui::reveil;
use crate::kernel::sync::reveil::INTERFACE;
use crate::kernel::task;
use crate::users;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

// Diagnostic matrix only; MODE 0 is the shipped behavior.
// 1 = full damage + normal culling, 2 = sparse damage + no occlusion,
// 3 = full damage + no occlusion. Never use these as a permanent fix.
const GUI_RENDER_DIAGNOSTIC_MODE: u8 = 0;

fn diagnostic_mode_name() -> &'static str {
    match GUI_RENDER_DIAGNOSTIC_MODE { 1 => "full-damage", 2 => "no-occlusion",
        3 => "full-no-occlusion", _ => "normal" }
}

/// Periode minimale entre deux trames composees, en millisecondes.
///
/// 16 ms, soit environ 60 par seconde. Ce n'est pas une cadence a tenir, c'est
/// un plafond : sans changement a l'ecran, aucune trame n'est produite.
use politique::PERIODE_TRAME_MS;

/// Periode de rafraichissement des indicateurs systeme (heure, CPU, memoire).
///
/// Ils changent tout seuls, sans evenement pour l'annoncer. Une seconde est la
/// granularite de l'horloge : rafraichir plus vite ne montrerait rien de plus.
use politique::PERIODE_HORLOGE_MS;



/// Periode du releve de charge par processus, en millisecondes.
///
/// Cinq secondes : assez rare pour ne pas noyer le journal, assez frequent pour
/// qu'une lenteur de quelques secondes laisse au moins une trace. C'est la ligne
/// qu'on lit quand on se demande « qui prend le processeur ».
use politique::PERIODE_RELEVE_MS;

/// Duree pendant laquelle un client muet est recompose a pleine cadence apres
/// une interaction, en millisecondes.
///
/// Un client qui n'annonce pas ses trames ne dit pas quand il peint : le
/// compositeur ne peut que recopier sa surface « au cas ou ». Le faire a chaque
/// tour de boucle coutait le prix fort — 1100x604 pixels recopies puis presentes
/// jusqu'a soixante fois par seconde, y compris devant une surface qui n'avait
/// pas change d'un pixel depuis cinq minutes. C'est ce qui mettait le bureau a
/// 76 % de processeur pendant que le navigateur attendait une reponse DNS.
///
/// La cadence est donc liee a ce qui peut faire changer l'image : une entree.
/// Apres une touche, un clic ou un mouvement transmis au client, on recompose a
/// pleine cadence pendant cette duree — le temps qu'une page reagisse, defile,
/// affiche un curseur. Passe ce delai sans aucune entree, plus rien ne peut
/// bouger a l'ecran sans que le client nous le dise, et on retombe a la cadence
/// de veille.

/// Periode de recomposition d'un client muet au repos, en millisecondes.
///
/// 200 ms, soit cinq trames par seconde. Ce n'est pas une cadence d'affichage :
/// c'est un filet de securite pour le cas ou un client repeindrait de lui-meme
/// sans entree — une animation, un chargement. Cinq fois par seconde suffit a
/// ce qu'une telle page reste visiblement vivante, et divise par douze le cout
/// que payait le repos.

fn plein_ecran() -> Rect {
    Rect::neuf(0, 0, fb::WIDTH as u32, fb::HEIGHT as u32)
}

// BOUCHAUD_UX_KEY_DAMAGE_V1
//
// De quoi prouver, et non supposer, qu'une frappe ne repeint plus le bureau.
// Trois compteurs, publies une fois par releve : ce n'est pas une trace par
// touche, qui changerait justement ce qu'on cherche a mesurer.
static TOUCHES_VERS_CLIENT: AtomicU64 = AtomicU64::new(0);
static TOUCHES_VERS_BUREAU: AtomicU64 = AtomicU64::new(0);

/// Compteurs d'entree du bureau : (touches remises a un client, touches
/// traitees par le bureau, degats plein ecran imposes).
pub fn stats_entree() -> (u64, u64, u64) {
    (
        TOUCHES_VERS_CLIENT.load(Ordering::Relaxed),
        TOUCHES_VERS_BUREAU.load(Ordering::Relaxed),
        crate::gui::degats::degats_plein_ecran(),
    )
}

/// Rectangle de widget (`window::Rect`) vers rectangle de degat.
///
/// Les deux existent pour des raisons differentes -- l'un decrit une zone
/// cliquable en largeur/hauteur signees, l'autre une region de degat que le
/// protocole GUI transporte. La conversion est ecrite une fois ici plutot que
/// dupliquee a chaque appel.
fn depuis_widget(r: window::Rect) -> Rect {
    Rect::neuf(r.x, r.y, r.w.max(0) as u32, r.h.max(0) as u32)
}

/// Lossless bridge from policy transitions into the established sparse damage
/// pipeline; no second compositor queue is retained.
#[allow(dead_code)]
fn ajoute_transition(degats: &mut Degats, transition: &crate::gui::windowing::Transition) {
    for crate::gui::windowing::Damage(rect) in &transition.damage {
        degats.ajoute(Origine::Fenetre,
            Rect::neuf(rect.x, rect.y, rect.width, rect.height));
    }
}

// BOUCHAUD_GUI_SCENE_CULLING_V1
//
// Les calques du bureau, du fond vers le haut. L'ordre EST celui du dessin :
// c'est le meme que celui de l'ancien `draw_desktop` suivi de `draw_menu`,
// `draw_taskbar` et `draw_cursor`, et il ne doit pas changer -- une inversion
// se verrait a l'ecran, pas dans un test.
//
// Chaque calque annonce des bornes qui MAJORENT ce qu'il dessine, et dit s'il
// est opaque. Dans le doute, non : un calque declare opaque a tort fait
// disparaitre ce qu'il y a dessous ; l'inverse ne coute qu'un peu de travail.
fn plan_de_scene(
    wins: &[Win],
    menu_open: bool,
    souris: (usize, usize),
    calques: &mut Vec<Calque>,
) {
    calques.clear();
    calques.push(Calque::plein(Element::Fond, plein_ecran()));

    let (fx, fy, fw, fh) = widgets::filigrane_rect();
    calques.push(Calque::transparent(
        Element::Filigrane,
        Rect::neuf(fx as i32, fy as i32, fw as u32, fh as u32),
    ));

    for index in 0..window::ICONS.len() {
        // Bornes elargies de 6 pixels : l'icone porte une ombre portee et un
        // libelle. Un calque qui deborde ses bornes laisse des trainees ;
        // l'inverse ne coute qu'un peu de travail.
        calques.push(Calque::transparent(
            Element::Icone(index),
            widgets::empreinte_icone(index),
        ));
    }

    calques.push(Calque::plein(Element::BarreHaute, barre_haute_rect()));

    for (index, w) in wins.iter().enumerate() {
        if w.min {
            continue;
        }
        // One canonical contract: painted bounds include the full eight-pixel
        // shadow; opacity is only the rounded shape's guaranteed central strip.
        let geometry = render_geometry(w);
        calques.push(Calque::avec_ombre(
            Element::Fenetre(index),
            proto_from_window_rect(geometry.painted_bounds),
            proto_from_window_rect(geometry.opaque),
        ));
    }

    if menu_open {
        // Meme forme que les fenetres, meme raison.
        calques.push(Calque::avec_ombre(
            Element::Menu,
            empreinte_menu(),
            depuis_widget(menu_rect()),
        ));
    }

    calques.push(Calque::plein(Element::BarreTaches, barre_taches_rect()));
    calques.push(Calque::transparent(
        Element::Curseur,
        degat_curseur(souris.0, souris.1),
    ));
}

/// Dessine un calque. Le seul endroit qui traduit un `Element` en pixels.
fn dessine_calque(
    calque: &Calque,
    wins: &[Win],
    menu_open: bool,
    souris: (usize, usize),
    mx: i32,
    my: i32,
) {
    match calque.element {
        Element::Fond => widgets::draw_fond(),
        Element::Filigrane => widgets::draw_filigrane(),
        Element::Icone(index) => widgets::draw_icone(index),
        Element::BarreHaute => widgets::draw_barre_haute(),
        Element::Fenetre(index) => {
            if let Some(w) = wins.get(index) {
                widgets::draw_fenetre(w, widgets::indice_focus(wins) == Some(index));
            }
        }
        Element::Menu => widgets::draw_menu(mx, my),
        Element::BarreTaches => widgets::draw_taskbar(wins, menu_open),
        Element::Curseur => widgets::draw_cursor(souris.0, souris.1),
    }
}

// BOUCHAUD_GUI_CIBLE_DEGAT_V1
//
// La seule traduction entre ce qu'une transition VISE et ce que la mesure
// COMPTE. Une transition qui touche deux elements de nature differente -- le
// menu et le bouton Demarrer, deux fenetres et les boutons de la barre -- rend
// desormais chaque rectangle avec sa cible, au lieu de laisser l'appelant les
// etiqueter tous pareil.
//
// Sans cela, `[GUI-DAMAGE] taskbar=0` sur une session entiere, pendant que la
// barre des taches etait repeinte des dizaines de fois.
// BOUCHAUD_GUI_CHAINE_ENTREE_LFB_V1
//
// LE FAUX POSITIF QUE CE HELPER SUPPRIME
// --------------------------------------
// Les deux gestes -- compter l'entree, armer la surveillance -- doivent se
// faire dans CET ordre : le veilleur memorise la chaine TELLE QU'ELLE ETAIT
// AVANT l'entree qui l'arme.
//
// Arme apres, sa reference contient deja l'evenement. Le premier maillon ne
// peut alors plus avancer, et le veilleur annonce « aucune entree recue »
// chaque fois que la souris s'arrete cinq cents millisecondes. C'est ce qui
// s'est vu au runtime : des paires BROKEN/RECOVERED sur `input_received` a
// longueur de journal, alors que les entrees arrivaient parfaitement.
//
// Un diagnostic qui crie au loup est pire qu'aucun diagnostic : il apprend a
// ne pas le lire. Les deux gestes sont donc reunis ici, ou l'ordre se voit.

/// Compte une entree du bureau ET arme la surveillance de la chaine.
fn note_entree_bureau(veilleur: &mut Veilleur, maintenant_ms: u64) {
    // La reference D'ABORD : l'etat d'avant cette entree.
    veilleur.note_entree(maintenant_ms, reveil::chaine());
    reveil::note_entree();
}

fn origine_de(cible: transition::Cible) -> Origine {
    match cible {
        transition::Cible::Fenetre => Origine::Fenetre,
        transition::Cible::Menu => Origine::Menu,
        transition::Cible::BarreTaches => Origine::BarreTaches,
        transition::Cible::BarreHaute => Origine::BarreHaute,
        transition::Cible::Curseur => Origine::Curseur,
        transition::Cible::Icone => Origine::Icone,
    }
}

/// Rectangle de la barre des taches — celle du BAS.
///
/// Bouton Demarrer et boutons de fenetres. Rien n'y change avec le temps.
fn barre_taches_rect() -> Rect {
    disposition::barre_taches(fb::WIDTH as u32, fb::HEIGHT as u32)
}

// BOUCHAUD_GUI_TOPBAR_DAMAGE_V1
//
// LE BUG QUE CETTE FONCTION CORRIGE
// ---------------------------------
// Le bureau a deux barres. En haut : titre, charge CPU par coeur, memoire,
// disque, et l'horloge. En bas : Demarrer et les fenetres. Ce sont deux
// rectangles opposes de l'ecran.
//
// Les seuls pixels du bureau qui changent SANS que personne ne l'annonce sont
// tous dans celle du HAUT. C'est pour eux que `PERIODE_HORLOGE_MS` existe.
//
// Le tic invalidait pourtant `barre_taches_rect()` — la barre du BAS. Le
// compositeur faisait alors exactement ce qu'on lui demandait : il recomposait
// et presentait une bande de 11 pixels tout en bas, ou strictement rien n'avait
// change, et laissait intacte celle du haut ou l'heure venait d'avancer.
//
// Le symptome est trompeur : `frames_clock_only` monte, `presents` monte,
// `presented_pixels` monte — toutes les metriques disent « je travaille » —
// et `HH:MM:SS` reste fige a l'ecran. Rien dans les compteurs ne pouvait le
// reveler, parce que le compositeur n'avait commis aucune faute : il presentait
// fidelement la zone qu'on lui avait designee.
//
// Ce n'est donc pas un renommage d'`Origine` : c'est le RECTANGLE qui etait
// faux. Il vient maintenant de `gui::disposition`, la meme definition que celle
// dont `plan_de_scene` derive les bornes d'`Element::BarreHaute`. Les deux ne
// peuvent plus designer deux bandes differentes.

/// Rectangle de la barre du HAUT : horloge, CPU, RAM, disque.
fn barre_haute_rect() -> Rect {
    disposition::barre_haute(fb::WIDTH as u32)
}

// BOUCHAUD_GUI_EMPREINTE_OMBRE_V1
//
// CE QUE LA FENETRE PEINT, par opposition a son cadre.
//
// `draw_window` peint une ombre decalee de `DEBORD_OMBRE` pixels : son
// empreinte reelle deborde donc du cadre en bas et a droite. Les
// invalidations, elles, utilisaient `cadre_fenetre` -- le cadre seul.
//
// Consequence a l'ecran : quand une fenetre bouge, se restaure ou se ferme, la
// bande d'ombre de son ANCIENNE position n'est jamais invalidee. Personne ne la
// repeint, et elle reste : un rectangle sombre abandonne sur le bureau. Le
// menu Demarrer avait exactement le meme defaut, d'ou les artefacts autour de
// lui.
//
// Lot 2 exposed both failure modes: transitions used the outer rectangle while
// the painter touched an eight-pixel outset, and scene culling advertised the
// entire rectangular outer frame as opaque although rounded corners are not
// painted. Both now derive from `WindowRenderGeometry`.
//
// Ces deux fonctions sont donc la seule facon autorisee de designer « ce que ce
// calque occupe a l'ecran ». `plan_de_scene` et toutes les invalidations
// passent par elles, de sorte qu'elles ne peuvent plus diverger.
fn empreinte_fenetre(w: &Win) -> Rect {
    proto_from_window_rect(render_geometry(w).painted_bounds)
}

fn render_geometry(w: &Win) -> crate::gui::windowing::WindowRenderGeometry {
    crate::gui::windowing::window_render_geometry(w.rect(), TITLE_H as u32,
        crate::gui::windowing::WINDOW_RADIUS,
        crate::gui::windowing::manager::SHADOW_EXTENT)
}

fn proto_from_window_rect(rect: crate::gui::windowing::Rect) -> Rect {
    Rect::neuf(rect.x, rect.y, rect.width, rect.height)
}

/// Idem pour le menu deroulant.
fn empreinte_menu() -> Rect {
    disposition::empreinte_avec_ombre(depuis_widget(menu_rect()))
}

/// Rectangle ecran d'une fenetre, cadre et barre de titre compris.
///
/// C'est la zone PLEINE -- ce que la fenetre remplit reellement. Pour ce
/// qu'elle OCCUPE, ombre comprise, voir [`empreinte_fenetre`].
///
/// C'est aussi ce qu'attendent `transition::fenetre_bougee` et
/// `transition::focus_transfere` : elles ajoutent l'ombre elles-memes, pour
/// qu'un appelant ne puisse pas l'oublier. Leur passer une empreinte deja
/// dilatee la dilaterait deux fois -- des degats trop larges, donc du travail
/// de composition inutile a chaque deplacement.
fn cadre_fenetre(w: &Win) -> Rect {
    Rect::neuf(w.x, w.y, w.w.max(0) as u32, w.h.max(0) as u32)
}

/// Empreinte volontairement un peu large du curseur logiciel (fleche 12x19).
fn degat_curseur(x: usize, y: usize) -> Rect {
    disposition::curseur(x as i32, y as i32)
}

/// Lance le bureau (bloquant jusqu'a Quitter).
pub fn run() {
    task::run_noyau(fil_bureau, "desktop");
}

/// Corps du fil noyau du bureau.
fn fil_bureau() -> ! {
    boucle();
    task::exit_current(0)
}

/// Codes de touche du protocole GUI.
///
/// Ce ne sont pas des codes evdev : le pilote clavier du bureau ne produit pas
/// de code brut mais une touche deja interpretee selon la disposition. Envoyer
/// un faux code Linux serait pire qu'un code a nous — le client le croirait.
mod touche {
    /// La touche porte un caractere ; le point de code est dans `unicode`.
    pub const CARACTERE: u32 = 0;
    pub const ENTREE: u32 = 1;
    pub const RETOUR: u32 = 2;
    pub const TABULATION: u32 = 3;
    pub const HAUT: u32 = 4;
    pub const BAS: u32 = 5;
    pub const GAUCHE: u32 = 6;
    pub const DROITE: u32 = 7;
    pub const ECHAP: u32 = 8;
}

/// Bits du champ `modificateurs` d'un message `Key`.
///
/// Meme raison d'etre que `touche` : c'est le bureau qui les produit, donc
/// c'est ici qu'ils sont definis, et `tools/verifie-protocole-gui.py` verifie
/// que les trois implementations du protocole s'accordent dessus. Les valeurs
/// etaient deja ecrites en clair chez l'hote Qt ; les nommer est ce qui permet
/// a la barriere de les voir.
mod modificateur {
    pub const SHIFT: u32 = 1;
    pub const CTRL: u32 = 2;
    pub const ALT: u32 = 4;
    pub const ALTGR: u32 = 8;
}

fn boucle() {
    fb::enter();
    mouse::init();
    crate::serial_println!("[gui] window manager demarre (fil noyau)");
    crate::serial_println!(
        "[GUI-RENDER-CONTRACT] mode={} titlebar={} shadow={} rounded={} window_bounds=outer+shadow",
        diagnostic_mode_name(), TITLE_H,
        crate::gui::windowing::manager::SHADOW_EXTENT,
        crate::gui::windowing::WINDOW_RADIUS,
    );

    let home = ramfs::fs().resolve(users::session().home(), 0).unwrap_or(0);
    let mut wins: Vec<Win> = Vec::new();
    let mut menu_open = false;
    let mut prev_left = false;
    let mut drag: Option<Drag> = None;
    let mut spawn_n = 0i32;
    // (icon_idx, offset_x_from_icon, offset_y_from_icon, start_mx, start_my)
    let mut icon_drag: Option<(usize, i32, i32, i32, i32)> = None;
    let mut last_icon_tap: Option<(usize, u64)> = None;
    let mut title_clicks = crate::gui::windowing::DoubleClickDetector::default();
    let mut hover_button: Option<Rect> = None;


    let mut quit = false;
    // Tout est sale au premier tour : il n'y a encore rien a l'ecran.
    let mut sale = true;
    let mut degats = Degats::neuf(plein_ecran());
    // Reutilise d'une trame a l'autre : construire le plan ne doit pas allouer.
    let mut calques: Vec<Calque> = Vec::new();
    degats.tout(); // premier tour : rien n'est encore a l'ecran

    // Un terminal pour commencer. Le premier tour est deja plein ecran, mais on
    // passe par le meme chemin que tout le monde : c'est ce qui rend la regle
    // verifiable au lieu d'etre une habitude.
    {
        let fenetre = make_app(0, home, &mut spawn_n);
        ouvre_fenetre(&mut wins, fenetre, &mut degats);
    }
    let mut derniere_trame = 0u64;
    let mut derniere_horloge = 0u64;
    let mut derniere_souris = (usize::MAX, usize::MAX);
    let mut derniers_boutons = 0u32;
    // BOUCHAUD_GUI_HOVER_CONTRAT_V1 : la ligne du menu actuellement en
    // surbrillance. C'est de l'etat du BUREAU, pas du peintre : sans elle,
    // personne ne sait quelle ligne doit cesser de l'etre. Voir plus bas.
    let mut survol_menu: Option<usize> = None;
    let mut dernier_releve = 0u64;
    // Derniere entree transmise a un client, et derniere recomposition
    // « aveugle » : ensemble, ils donnent sa cadence a un client muet.
    let mut derniere_entree = 0u64;
    let mut dernier_aveugle = 0u64;
    // BOUCHAUD_GUI_CHAINE_ENTREE_LFB_V1 : voir `gui::chaine`.
    let mut veilleur = Veilleur::neuf();

    while !quit {
        // BOUCHAUD_GUI_EVENT_DRIVEN_V1
        //
        // Le billet est pris AVANT toute lecture d'etat, et c'est tout
        // l'interet : un evenement qui arrive pendant le traitement rend ce
        // billet perime, et le sommeil de fin de tour sera refuse. Le prendre
        // apres avoir constate « rien a faire » rouvrirait exactement la
        // fenetre de reveil perdu que ce protocole existe pour fermer.
        let billet = INTERFACE.billet();
        reveil::note_tour();
        task::note_wm_heartbeat();
        let maintenant = crate::kernel::timer::monotonic_ms();

        // ---- Clavier (non bloquant) ----
        //
        // BOUCHAUD_UX_KEY_DAMAGE_V1
        //
        // Une touche ne salit l'ecran que si le BUREAU change. Ce qui part chez
        // un client n'en fait pas partie : le client annoncera son propre degat
        // quand il aura repeint, et `pompe_clients` le reprend. Le bureau, lui,
        // n'a rien a redessiner.
        //
        // Chaque frappe imposait ici `sale = true` et un degat PLEIN ECRAN,
        // avant meme de savoir ou allait la touche. Le compositeur repeignait
        // donc fond, barre des taches, cadres et curseur, puis reprojetait tout
        // l'ecran -- a chaque lettre. C'est exactement ce qui se voyait comme
        // « la page se recharge » : ni navigation, ni requete HTTP, ni capture
        // LibWeb supplementaire, seulement le bureau redessine par-dessus.
        while let Some(evenement) = keyboard::try_key_event() {
            let k = evenement.logique;
            // Echap ferme le menu, puis la fenetre du dessus, puis le bureau —
            // sauf quand un client a le focus. Un navigateur a besoin d'Echap
            // (arreter un chargement, fermer une boite de dialogue) et le lui
            // confisquer pour fermer sa fenetre serait le pire des deux mondes :
            // la touche ne ferait pas ce que la page attend, et detruirait le
            // travail en cours. La croix de la barre de titre reste la.
            let actif = fenetre_active(&wins);
            let client_actif = actif
                .map_or(false, |index| window::est_client(&wins[index]));
            // Un client recoit les deux transitions ; le bureau, lui, n'agit
            // que sur l'appui. Fermer une fenetre sur le relachement d'Echap la
            // fermerait une seconde fois, et une application du noyau
            // insererait chaque caractere en double.
            if !evenement.appui && client_actif && !menu_open {
                if let Some(index) = actif {
                    if let App::Navigateur { client } = &mut wins[index].app {
                        envoie_touche(client, evenement);
                        TOUCHES_VERS_CLIENT.fetch_add(1, Ordering::Relaxed);
                        derniere_entree = maintenant;
                    }
                }
                continue;
            }
            if !evenement.appui {
                continue;
            }
            if k == Key::Other && (menu_open || !client_actif) {
                // Un menu ou une fenetre disparait : ce qu'elle couvrait
                // redevient fond, et personne d'autre ne le sait.
                sale = true;
                degats.tout();
                TOUCHES_VERS_BUREAU.fetch_add(1, Ordering::Relaxed);
                if menu_open { menu_open = false; }
                else if !ferme_fenetre_du_dessus(&mut wins) { quit = true; }
                continue;
            }
            if let Some(index) = actif {
                if let App::Navigateur { client } = &mut wins[index].app {
                    envoie_touche(client, evenement);
                    TOUCHES_VERS_CLIENT.fetch_add(1, Ordering::Relaxed);
                    derniere_entree = maintenant;
                    continue;
                }
                // Application du noyau : c'est le bureau qui la dessine, donc
                // c'est bien lui qui se salit — mais sa fenetre seulement.
                sale = true;
                degats.ajoute(Origine::Fenetre, empreinte_fenetre(&wins[index]));
                TOUCHES_VERS_BUREAU.fetch_add(1, Ordering::Relaxed);
                if apps::key_to_app(&mut wins[index], k, home) {
                    ferme_fenetre(&mut wins, index);
                    // Une fenetre qui disparait decouvre ce qu'elle couvrait,
                    // et le bureau est seul a le savoir.
                    degats.tout();
                }
            }
        }

        // ---- Souris ----
        let (mxu, myu) = mouse::pos();
        let mx = mxu as i32;
        let my = myu as i32;
        let wheel = mouse::take_wheel();
        let left = mouse::left_down();
        let click = left && !prev_left;
        let release = !left && prev_left;
        prev_left = left;
        // La position part au client quand le curseur bouge **ou** quand un
        // bouton change d'etat. Ne suivre que le mouvement laissait le client
        // croire que le bouton etait reste enfonce : sans deplacement entre
        // l'appui et le relachement, aucun message ne portait le relachement, et
        // le clic suivant n'avait plus de front montant a montrer. Un seul clic
        // fonctionnait, puis plus aucun.
        let boutons = crate::drivers::mouse::buttons() as u32;
        if (mxu, myu) != derniere_souris || boutons != derniers_boutons {
            let avant = if derniere_souris.0 == usize::MAX {
                None // premier tour : rien n'a encore ete dessine
            } else {
                Some((derniere_souris.0 as i32, derniere_souris.1 as i32))
            };
            for (rect, cible) in transition::curseur_deplace(avant, (mx, my)).iter() {
                degats.ajoute(origine_de(cible), rect);
            }
            derniere_souris = (mxu, myu);
            derniers_boutons = boutons;
            sale = true;
            derniere_entree = maintenant;
            note_entree_bureau(&mut veilleur, maintenant);
            transmet_position(&mut wins, mx, my, boutons);
            let new_hover = hovered_button_rect(&wins, mx, my);
            if new_hover != hover_button {
                if let Some(rect) = hover_button { degats.ajoute(Origine::Fenetre, rect); }
                if let Some(rect) = new_hover { degats.ajoute(Origine::Fenetre, rect); }
                hover_button = new_hover;
            }
        }
        // BOUCHAUD_GUI_DAMAGE_ORIGIN_V1
        //
        // Ce qui etait ici : `click || release || wheel != 0` -> plein ecran.
        // Le meme defaut que le clavier, par une autre porte. Un clic dans une
        // page ne change rien au BUREAU : le client recoit l'evenement et
        // annoncera son degat s'il repeint. Un cran de molette encore moins.
        //
        // L'entree reste notee -- elle pilote la cadence de recomposition d'un
        // client muet -- mais elle ne salit plus rien par elle-meme. Ce sont
        // `handle_click` et `handle_wheel` qui disent ce qu'ils ont change.
        if click || release || wheel != 0 {
            derniere_entree = maintenant;
            note_entree_bureau(&mut veilleur, maintenant);
        }

        if left {
            if let Some(d) = drag {
                // Deplacer ou redimensionner ne salit que l'union de la
                // position quittee et de celle atteinte. Le fond redecouvert
                // est dans la premiere, le cadre nouveau dans la seconde.
                let avant = wins.last().map(cadre_fenetre).unwrap_or_default();
                if let Some(w) = wins.last_mut() {
                    match d {
                        Drag::Move(ox, oy) => { w.x = mx - ox; w.y = my - oy; }
                        Drag::Resize(edge) => {
                            use crate::gui::windowing::ResizeEdge::*;
                            if matches!(edge, Left | NorthWest | SouthWest) {
                                let right = w.x + w.w; w.x = mx.min(right - MIN_W); w.w = right - w.x;
                            }
                            if matches!(edge, Right | NorthEast | SouthEast) { w.w = (mx - w.x).max(MIN_W); }
                            if matches!(edge, Top | NorthWest | NorthEast) {
                                let bottom = w.y + w.h; w.y = my.min(bottom - MIN_H); w.h = bottom - w.y;
                            }
                            if matches!(edge, Bottom | SouthWest | SouthEast) { w.h = (my - w.y).max(MIN_H); }
                            if w.x + w.w > fb::WIDTH as i32 { w.w = fb::WIDTH as i32 - w.x; }
                            if w.y + w.h > fb::HEIGHT as i32 - BAR_H as i32 { w.h = fb::HEIGHT as i32 - BAR_H as i32 - w.y; }
                        }
                    }
                    clamp_win(w);
                }
                let apres = wins.last().map(cadre_fenetre).unwrap_or_default();
                for (rect, cible) in transition::fenetre_bougee(avant, apres).iter() {
                    degats.ajoute(origine_de(cible), rect);
                }
                sale = true;
            } else if let Some((idx, ox, oy, _, _)) = icon_drag {
                // BOUCHAUD_GUI_EMPREINTE_ICONE_V1 : le libelle deborde de
                // l'icone. C'est son empreinte, pas son rectangle, qu'il faut
                // invalider — des deux cotes du deplacement.
                let avant = widgets::empreinte_icone(idx);
                let new_x = (mx - ox).max(0);
                let new_y = (my - oy).max(BAR_H as i32);
                unsafe { window::ICON_POSITIONS[idx] = (new_x, new_y); }
                degats.ajoute(Origine::Icone, avant);
                degats.ajoute(Origine::Icone, widgets::empreinte_icone(idx));
                sale = true;
            }
        } else {
            let ended_drag = drag.take();
            if release {
                if matches!(ended_drag, Some(Drag::Move(..))) {
                    if let Some(window) = wins.last_mut() {
                        let before = cadre_fenetre(window);
                        if mx <= crate::gui::windowing::SNAP_THRESHOLD && window.flags.snappable {
                            let id = window.id;
                            route_window_command(window, crate::gui::windowing::WindowCommand::Snap(
                                id, crate::gui::windowing::SnapZone::Left));
                        } else if mx >= fb::WIDTH as i32 - crate::gui::windowing::SNAP_THRESHOLD && window.flags.snappable {
                            let id = window.id;
                            route_window_command(window, crate::gui::windowing::WindowCommand::Snap(
                                id, crate::gui::windowing::SnapZone::Right));
                        } else if my <= BAR_H as i32 + crate::gui::windowing::SNAP_THRESHOLD {
                            let id = window.id;
                            route_window_command(window, crate::gui::windowing::WindowCommand::Maximize(id));
                        }
                        for (rect, cible) in transition::fenetre_bougee(before, cadre_fenetre(window)).iter() {
                            degats.ajoute(origine_de(cible), rect);
                        }
                    }
                }
                if let Some((idx, _, _, smx, smy)) = icon_drag.take() {
                    let moved = (mx - smx).abs().max((my - smy).abs());
                    if moved < 6 {
                        // Clic — verifier double-clic
                        let tick = crate::kernel::timer::ticks();
                        // Fenetre de double-clic : une demi-seconde.
                        let fenetre = crate::kernel::timer::TICKS_PER_SECOND / 2;
                        let double = last_icon_tap
                            .map_or(false, |(li, lt)| li == idx && tick.wrapping_sub(lt) < fenetre);
                        if double {
                            let kind = ICONS[idx].1;
                            if kind == window::KIND_NAVIGATEUR {
                                lance_navigateur(&mut wins, home, &mut degats);
                            } else {
                                let fenetre = make_app(kind, home, &mut spawn_n);
                                ouvre_fenetre(&mut wins, fenetre, &mut degats);
                            }
                            // Une fenetre vient d'apparaitre : sans cela, la
                            // trame ne serait composee qu'au prochain degat.
                            sale = true;
                            last_icon_tap = None;
                        } else {
                            last_icon_tap = Some((idx, tick));
                        }
                    }
                }
            }
        }

        if click {
            handle_click(mx, my, maintenant, &mut title_clicks, &mut wins, &mut menu_open,
                &mut drag, &mut quit, home, &mut spawn_n, &mut icon_drag, &mut degats);
            sale = true;
        }
        if wheel != 0 {
            if handle_wheel(mx, my, wheel, &mut wins, &mut degats) {
                sale = true;
            }
        }

        // BOUCHAUD_GUI_HOVER_CONTRAT_V1
        //
        // LE BUG QUE CE BLOC CORRIGE
        // --------------------------
        // `draw_menu` met en valeur la ligne sous le pointeur : fond plus clair
        // sur toute la largeur, bordure de selection a gauche, texte blanc et
        // en gras. Cela repeint une bande de 178 x 22 pixels.
        //
        // Un deplacement de souris n'invalidait pourtant que deux empreintes de
        // curseur de 14 x 22. Passer d'une entree du menu a la suivante
        // presentait donc la nouvelle ligne — le curseur est dessus, son
        // empreinte la recoupe — mais JAMAIS l'ancienne. Deux lignes
        // apparaissaient en surbrillance, puis trois, puis toute la colonne
        // parcourue : le menu gardait la trace du chemin du pointeur.
        //
        // Entrer dans le menu et en sortir sont le meme defaut par les bords :
        // en sortant, la derniere ligne survolee n'etait invalidee par personne.
        //
        // CE QUI N'EST PAS FAIT : `degats.tout()`. Le survol change une ligne,
        // parfois deux. Repeindre l'ecran pour cela rendrait le compositeur
        // event-driven inutile a chaque pixel de deplacement dans le menu.
        //
        // Le bureau garde donc l'ancien survol, et invalide les DEUX lignes.
        // `window::ligne_menu_survolee` est la meme fonction que celle dont
        // `draw_menu` deduit ce qu'il peint : les deux ne peuvent pas diverger.
        let nouveau_survol = if menu_open {
            window::ligne_menu_survolee(mx, my)
        } else {
            None
        };
        if nouveau_survol != survol_menu {
            let lignes = transition::survol_menu_change(
                window::menu_proto(), survol_menu, nouveau_survol,
            );
            for (rect, cible) in lignes.iter() {
                degats.ajoute(origine_de(cible), rect);
            }
            survol_menu = nouveau_survol;
            sale = true;
        }

        // ---- Clients ring 3 ----
        //
        // Un client muet ne dit pas quand il peint : le compositeur recopie sa
        // surface sans savoir si elle a change. On ne le fait donc pas a chaque
        // tour, mais a une cadence qui suit ce qui peut faire bouger l'image —
        // pleine cadence juste apres une entree, cadence de veille sinon.
        let client_muet_visible = wins.iter().any(|w| {
            !w.min && matches!(&w.app, App::Navigateur { client } if client.recompose_a_l_aveugle())
        });
        let etat_aveugle = politique::Etat {
            maintenant_ms: maintenant,
            client_muet_visible,
            dernier_aveugle_ms: dernier_aveugle,
            derniere_entree_ms: derniere_entree,
            ..Default::default()
        };
        let recompose_aveugle = politique::doit_recomposer_aveugle(&etat_aveugle);
        if recompose_aveugle {
            dernier_aveugle = maintenant;
            reveil::note_recomposition_aveugle();
        }
        let (degat_clients, perte_fenetre) = pompe_clients(&mut wins, recompose_aveugle);
        if !degat_clients.vide() {
            sale = true;
            degats.ajoute(Origine::Client, degat_clients);
        }
        if perte_fenetre {
            // Une fenetre a disparu : ce qu'elle couvrait redevient fond.
            degats.tout();
            sale = true;
        }

        // ---- Rendu ----
        // L'horloge est la SEULE animation permanente du bureau : elle change
        // sans que rien puisse l'annoncer. Voir `politique::PERIODE_HORLOGE_MS`.
        // On note si le degat de ce tour ne vient QUE d'elle, pour que la mesure
        // d'inactivite puisse distinguer « le bureau dort » de « le bureau se
        // reveille pour rien ».
        let mut horloge_seule = false;
        if maintenant.wrapping_sub(derniere_horloge) >= PERIODE_HORLOGE_MS {
            derniere_horloge = maintenant;
            horloge_seule = !sale;
            sale = true; // horloge, charge CPU, memoire : ils bougent seuls
            // BOUCHAUD_GUI_TOPBAR_DAMAGE_V1 : la barre du HAUT. Voir
            // `barre_haute_rect`. La barre du bas n'a rien qui bouge tout seul.
            for (rect, cible) in transition::tic_horloge(fb::WIDTH as u32).iter() {
                degats.ajoute(origine_de(cible), rect);
            }
        }
        if sale && maintenant.wrapping_sub(derniere_trame) < PERIODE_TRAME_MS {
            reveil::note_trame_differee();
        }
        if sale && maintenant.wrapping_sub(derniere_trame) >= PERIODE_TRAME_MS {
            // BOUCHAUD_GUI_DAMAGE_REGION_V2
            //
            // Une trame peut maintenant porter plusieurs rectangles eloignes.
            // Pour chaque rectangle, le MEME clip borne le dessin et la copie :
            // aucun pixel de backbuffer perime ne peut etre presente.
            // BOUCHAUD_GUI_CURSEUR_ADAPTATIF_V1
            //
            // Derniere regle avant de composer, et elle doit l'etre : le
            // curseur choisit sa couleur d'apres le pixel sous son point chaud.
            // Tout degat qui repeint ce pixel change donc la fleche ENTIERE, y
            // compris les parties qu'il ne couvre pas. On l'ajoute une fois que
            // les degats de ce tour sont connus, jamais avant.
            let recoloration = transition::recoloration_curseur(degats.regions(), (mx, my));
            for (rect, cible) in recoloration.iter() {
                degats.ajoute(origine_de(cible), rect);
            }

            if matches!(GUI_RENDER_DIAGNOSTIC_MODE, 1 | 3) { degats.tout(); }
            if !degats.vide() {
                crate::kernel::timer::frame_start();
                crate::gui::degats::note_trame(&degats);

                // BOUCHAUD_GUI_SCENE_CULLING_V1
                //
                // Le plan est construit UNE fois par trame, pas une fois par
                // rectangle : ses bornes ne dependent pas du rectangle.
                plan_de_scene(&wins, menu_open, (mxu, myu), &mut calques);

                for region in degats.regions().iter().copied() {
                    let present = proto_rect_ecran(region);
                    if present.vide() {
                        continue;
                    }

                    fb::set_clip(
                        present.x as usize, present.y as usize,
                        present.largeur as usize, present.hauteur as usize,
                    );

                    // Deux regles, dans cet ordre : on part du premier calque
                    // opaque qui recouvre entierement la zone -- tout ce qui est
                    // dessous est invisible --, puis on ecarte ceux qui ne la
                    // touchent pas.
                    let debut = if matches!(GUI_RENDER_DIAGNOSTIC_MODE, 2 | 3) {
                        0
                    } else { scene::premier_calque(&calques, &present) };
                    let mut dessines = 0usize;
                    for calque in &calques[debut..] {
                        if !scene::doit_dessiner(calque, &present) {
                            continue;
                        }
                        dessine_calque(calque, &wins, menu_open, (mxu, myu), mx, my);
                        dessines += 1;
                    }
                    reveil::note_culling(calques.len(), debut, dessines);
                    fb::reset_clip();

                    fb::present_rect(
                        present.x as usize, present.y as usize,
                        present.largeur as usize, present.hauteur as usize,
                    );
                    crate::gui::degats::note_presentation(present);
                }

                crate::kernel::timer::mark_frame();
                reveil::note_trame(horloge_seule);
            }

            derniere_trame = maintenant;
            sale = false;
            degats.efface();
        }

        if maintenant.wrapping_sub(dernier_releve) >= PERIODE_RELEVE_MS {
            let periode = if dernier_releve == 0 {
                PERIODE_RELEVE_MS
            } else {
                maintenant.wrapping_sub(dernier_releve)
            };
            dernier_releve = maintenant;
            releve_charge(&mut wins, periode);
        }

        // BOUCHAUD_GUI_CHAINE_ENTREE_LFB_V1
        //
        // Une seule ligne par episode, et seulement quand la chaine est
        // reellement rompue. Voir `gui::chaine` pour pourquoi ce n'est ni une
        // trace par mouvement ni un simple « le bureau ne repond plus ».
        match veilleur.examine(maintenant, reveil::chaine(), politique::DELAI_VEILLE_MS) {
            Verdict::Rupture(maillon) => {
                let (demandes, copies, pixels, userland, tampon, lfb, vide, ns) =
                    fb::trace_present();
                let (px, py, pw, ph) = fb::dernier_present_rect();
                crate::serial_println!(
                    "[GUI-CHAIN] BROKEN at={} hint=\"{}\" \
                     kernel_alive=1 wm_heartbeat={} loops={} \
                     input_events={} damages={} frames_composed={} \
                     present_calls={} lfb_copies={} lfb_pixels={} \
                     backbuffer_generation={} \
                     refused_userland={} refused_backbuffer={} refused_lfb={} \
                     refused_empty_rect={} last_present_rect={},{},{},{} \
                     last_present_ns={} now_ns={}",
                    maillon.nom(), maillon.piste(),
                    task::wm_heartbeat(), reveil::tours(),
                    reveil::entrees(), crate::gui::degats::total_degats(),
                    reveil::trames_composees(),
                    demandes, copies, pixels,
                    fb::pixels_dessines(),
                    userland, tampon, lfb, vide,
                    px, py, pw, ph, ns,
                    crate::kernel::timer::monotonic_ns(),
                );
            }
            Verdict::Retabli(maillon) => {
                crate::serial_println!(
                    "[GUI-CHAIN] RECOVERED at={} lfb_copies={}",
                    maillon.nom(),
                    fb::lfb_present_generation(),
                );
            }
            Verdict::Rien => {}
        }

        task::nettoie_zombies();

        // BOUCHAUD_GUI_EVENT_DRIVEN_V1
        //
        // CE QUI A CHANGE, ET POURQUOI CE N'EST PAS UN SLEEP PLUS LONG
        // -----------------------------------------------------------
        // Avant : `sleep_ticks(4 ou 16)`. Le bureau se reveillait entre quinze
        // et soixante fois par seconde pour CONSTATER qu'il n'y avait rien a
        // faire. Allonger ce delai n'aurait rien change au fond : c'est encore
        // du polling, seulement plus lent -- et cela aurait ajoute de la
        // latence a la premiere frappe.
        //
        // Maintenant : le bureau dort jusqu'a ce qu'un producteur signale, ou
        // jusqu'a la prochaine echeance REELLE. Sans horloge affichee, sans
        // client muet et sans degat en attente, `prochaine_echeance` rend
        // `None` et le sommeil n'a pas de fin.
        //
        // L'echeance du releve de charge existe toujours : elle n'affiche rien,
        // elle ecrit dans le journal, et c'est elle qui garantit qu'on ne perd
        // jamais totalement la trace d'un bureau endormi.
        let etat = politique::Etat {
            maintenant_ms: maintenant,
            sale,
            client_muet_visible,
            horloge_visible: true,
            derniere_trame_ms: derniere_trame,
            derniere_horloge_ms: derniere_horloge,
            dernier_releve_ms: dernier_releve,
            dernier_aveugle_ms: dernier_aveugle,
            derniere_entree_ms: derniere_entree,
        };

        // Pas de traitement particulier du glisser. Il serait tentant de
        // reboucler sans dormir tant que le bouton est enfonce -- « la fenetre
        // doit suivre le curseur ». Ce serait une attente active : sans paquet
        // PS/2, le curseur n'a pas bouge, et il n'y a donc rien a suivre. Le
        // moindre mouvement produit un paquet, donc un signal, donc un reveil.
        match politique::prochaine_echeance(&etat) {
            // Echeance deja atteinte : reboucler tout de suite plutot que de
            // payer deux changements de contexte pour un sommeil nul.
            Some(date) if date <= maintenant => {}
            Some(date) => {
                let attente_ns = date
                    .saturating_sub(maintenant)
                    .saturating_mul(1_000_000);
                let echeance_ns = crate::kernel::timer::monotonic_ns()
                    .saturating_add(attente_ns);
                let _ = INTERFACE.attends(billet, echeance_ns);
            }
            None => {
                // Rien ne changera tout seul : le sommeil n'a pas de fin, seul
                // un signal en sortira.
                //
                // CE CHEMIN N'EST PAS ATTEINT AUJOURD'HUI, et il faut le dire :
                // `horloge_visible` vaut toujours `true` ci-dessus, parce que la
                // barre des taches est toujours affichee. L'architecture le
                // permet, la configuration actuelle non.
                //
                // Deux choses devront changer avant : une barre des taches
                // masquable, et un chien de garde (`task::watchdog_from_timer`)
                // qui sache distinguer un bureau VOLONTAIREMENT endormi d'un
                // bureau bloque -- il crie aujourd'hui apres deux secondes sans
                // battement. Le compteur est la pour que le jour ou ce chemin
                // s'ouvre, on le voie.
                reveil::note_sommeil_sans_fin();
                let _ = INTERFACE.attends(billet, u64::MAX);
            }
        }
    }

    ferme_tous_les_clients(&mut wins);
    fb::leave();
    crate::serial_println!("[gui] window manager ferme");
}

/// Ecrit dans le journal qui consomme le processeur et la memoire.
///
/// Un gestionnaire des taches tient dans une ligne : le nom, le pid, la part de
/// processeur sur la periode et la taille de l'espace d'adressage. Le
/// Le denominateur est le temps mur du timer. L'idle n'est donc plus
/// redistribue artificiellement aux processus : la somme peut etre inferieure
/// a 100 %, ce qui represente du vrai idle ou du travail noyau.
fn releve_charge(wins: &mut Vec<Win>, periode_ms: u64) {
    // BOUCHAUD_SMP_NG2_LOAD_LOG_V1
    task::log_smp_load();

    // BOUCHAUD_UX_KEY_DAMAGE_V1 : la preuve que la frappe ne repeint plus le
    // bureau. Taper du texte dans une page doit faire monter `vers_client`
    // seul ; `plein_ecran` ne bouge que sur une fermeture de fenetre ou de
    // menu. Une ligne par releve, pas une par touche.
    let (touches_client, touches_bureau, degats_pleins) = stats_entree();
    crate::serial_println!(
        "[GUI-INPUT] touches_client={} touches_bureau={} degats_plein_ecran={}",
        touches_client, touches_bureau, degats_pleins,
    );

    // BOUCHAUD_GUI_DAMAGE_ORIGIN_V1 : d'ou viennent les degats, et ce que la
    // composition finit par copier. Une ligne par releve, jamais par evenement.
    let (par_origine, trames, pixels) = crate::gui::degats::stats_degats();
    let (rects, demandes, boite_gate0, fusions, debordements) =
        crate::gui::degats::stats_regions();
    let evites = boite_gate0.saturating_sub(pixels);
    crate::serial_println!(
        "[GUI-DAMAGE] full={} window={} cursor={} client={} taskbar={} menu={} icon={} topbar={} presents={} rects={} presented_pixels={} requested_pixels={} gate0_bbox_pixels={} saved_pixels={} merges={} overflows={} drawn_pixels={}",
        par_origine[0], par_origine[1], par_origine[2], par_origine[3],
        par_origine[4], par_origine[5], par_origine[6], par_origine[7],
        trames, rects, pixels,
        demandes, boite_gate0, evites, fusions, debordements, fb::pixels_dessines(),
    );
    // BOUCHAUD_GFX_PRESENT_TRACE_V1
    //
    // Le dernier maillon, sans lequel tout le reste ment. `present_calls` monte
    // meme quand `present_rect` refuse ; seul `lfb_copies` prouve que des pixels
    // ont atteint l'ecran. Si `present_calls` avance et `lfb_copies` non, le
    // motif de refus est dans les quatre compteurs suivants.
    let (demandes, copies, pixels_lfb, userland, tampon, lfb, vide, dernier_ns) =
        fb::trace_present();
    let (px, py, pw, ph) = fb::dernier_present_rect();
    let maintenant_ns = crate::kernel::timer::monotonic_ns();
    crate::serial_println!(
        "[GUI-PRESENT] present_calls={} lfb_copies={} lfb_pixels={} \
         backbuffer_generation={} refused_userland={} refused_backbuffer={} \
         refused_lfb={} refused_empty_rect={} last_present_rect={},{},{},{} \
         last_present_ns={} since_last_present_ms={}",
        demandes, copies, pixels_lfb,
        fb::pixels_dessines(),
        userland, tampon, lfb, vide,
        px, py, pw, ph,
        dernier_ns,
        if dernier_ns == 0 { 0 } else { maintenant_ns.saturating_sub(dernier_ns) / 1_000_000 },
    );

    // BOUCHAUD_GUI_EVENT_DRIVEN_V1 : ce que le compositeur a reellement fait,
    // et surtout ce qu'il n'a PAS fait. Une ligne par releve.
    crate::gui::reveil::publie();

    let (mesures, total) = task::mesure_processus();
    if total > 0 {
        let mut ligne = String::new();
        let sample_ns = crate::kernel::timer::monotonic_ns();
        for mesure in mesures.iter() {
            if mesure.ticks == 0 && mesure.rss_octets < 1024 * 1024 {
                continue; // rien a dire d'un processus qui n'a rien fait
            }
            if !ligne.is_empty() {
                ligne.push_str(" | ");
            }
            let online = crate::arch::x86_64::smp::schedulable_cpus();
            let mut cpu_map = String::from("[");
            for cpu in 0..online {
                if cpu != 0 { cpu_map.push(','); }
                cpu_map.push_str(&alloc::format!(
                    "{}",
                    mesure.cpu_map_ns[cpu].saturating_mul(100) / total,
                ));
            }
            cpu_map.push(']');
            crate::serial_println!(
                "[PROC-SAMPLE] v=1 t_ns={} pid={} name={} cpu_pct={} cpu_map={} ctx_delta={} mig_delta={} runnable_threads={} threads={} rss={} vss={}",
                sample_ns,
                mesure.pid,
                mesure.nom,
                mesure.ticks.saturating_mul(100) / total,
                cpu_map,
                mesure.context_switches,
                mesure.migrations,
                mesure.runnable_threads,
                mesure.taches,
                mesure.rss_octets,
                mesure.vss_octets,
            );
            ligne.push_str(&alloc::format!(
                "{} pid={} cpu {}% cpu_map={} rss {} Mio vss {} Mio thr={} ctx={} mig={}",
                mesure.nom,
                mesure.pid,
                mesure.ticks * 100 / total,
                cpu_map,
                mesure.rss_octets / (1024 * 1024),
                mesure.vss_octets / (1024 * 1024),
                mesure.taches,
                mesure.context_switches,
                mesure.migrations,
            ));
        }
        if !ligne.is_empty() {
            crate::serial_println!("[ps] {}", ligne);
        }

        // ResourceGroup est une vue d'agrégation, pas une contrainte scheduler.
        // Un groupe multithread/multiprocessus peut utiliser tous les CPU.
        let mut groups: Vec<(u32, String, u64, u64, usize, u64, u64)> = Vec::new();
        for mesure in mesures.iter() {
            if let Some(group) = groups.iter_mut().find(|g| g.0 == mesure.resource_group_id) {
                group.2 = group.2.saturating_add(mesure.ticks);
                group.3 = group.3.saturating_add(mesure.rss_octets);
                group.4 = group.4.saturating_add(1);
                group.5 = group.5.saturating_add(mesure.context_switches);
                group.6 = group.6.saturating_add(mesure.migrations);
            } else {
                groups.push((mesure.resource_group_id, mesure.resource_group_name.clone(),
                    mesure.ticks, mesure.rss_octets, 1, mesure.context_switches, mesure.migrations));
            }
        }
        for (id, name, cpu_ns, rss, processes, ctx, migrations) in groups {
            crate::serial_println!(
                "[APP-SAMPLE] v=1 t_ns={} id={} name={} cpu_pct={} rss={} processes={} ctx_delta={} mig_delta={}",
                sample_ns, id, name, cpu_ns.saturating_mul(100) / total,
                rss, processes, ctx, migrations,
            );
        }
    }


    let sched = task::diagnostic_ordonnanceur();
    crate::serial_println!(
        "[sched] switches={} irq-preempt={} deferred={} wm-age={} ms ready={}/{}",
        sched.switches,
        sched.irq_preemptions,
        sched.deferred_preemptions,
        sched.wm_age_ms,
        sched.ready,
        sched.live
    );
    let clavier = keyboard::stats();
    crate::serial_println!(
        "[kbd] irq={} attente={} perdus={} last={:#04x} status={:#04x} cfg={:#04x} PIC1={:#04x} ACK(F6/F4)={:#04x}/{:#04x}",
        clavier.irq,
        clavier.pending,
        clavier.dropped,
        clavier.last_scancode,
        clavier.controller_status,
        clavier.controller_config,
        clavier.pic_master_mask,
        clavier.ack_defaults,
        clavier.ack_enable
    );

    for w in wins.iter_mut() {
        if let App::Navigateur { client } = &mut w.app {
            crate::serial_println!("[gui] client {}", client.etat_journal(periode_ms));
            client.remet_compteurs();
        }
    }
}

// --- Clients -----------------------------------------------------------------

/// Indice de la fenetre active (celle du dessus, non minimisee).
fn fenetre_active(wins: &[Win]) -> Option<usize> {
    wins.iter().rposition(|w| !w.min)
}

/// Consulte tous les clients : trames recues, processus morts.
///
/// Rend `true` si l'ecran doit etre recompose.
/// Rend le degat accumule et **si une fenetre a disparu**.
///
/// Les deux ne se deduisent pas l'un de l'autre : un degat peut etre vide
/// alors qu'une fenetre vient de se fermer, et c'est precisement ce cas qui
/// exige que le bureau redessine ce qu'elle couvrait. Rendre un plein ecran
/// ici, comme avant, obligeait l'appelant a le subir sans savoir pourquoi.
fn pompe_clients(wins: &mut Vec<Win>, recompose_aveugle: bool) -> (Rect, bool) {
    let mut degat_ecran = Rect::default();
    let mut morts: Vec<usize> = Vec::new();
    for (index, w) in wins.iter_mut().enumerate() {
        let zone_fenetre = zone_utile(w);
        let visible = !w.window.min;
        if let App::Navigateur { client } = &mut w.app {
            if client.verifie_silence() {
                degat_ecran = degat_ecran.union(&zone_fenetre);
            }
            // Un client qui n'annonce pas ses trames est recompose « a
            // l'aveugle » : on ne sait pas quand il peint, seulement qu'il
            // peint. La cadence de cette recopie est decidee par l'appelant —
            // voir `REACTIVITE_MUETTE_MS`. La declarer ici a chaque tour
            // revenait a recopier 1100x604 pixels soixante fois par seconde
            // devant une image immobile.
            if recompose_aveugle && client.recompose_a_l_aveugle() && visible {
                client.abime_tout();
                degat_ecran = degat_ecran.union(&zone_fenetre);
            }
            if client.pompe() {
                // Le degat est consomme meme si la fenetre est minimisee :
                // l'accumuler pour rien ferait grossir un rectangle que
                // personne ne lit, et la restauration recompose de toute facon.
                let degat = client.prend_degat();
                if visible && !degat.vide() {
                    let ecran = Rect::neuf(
                        zone_fenetre.x.saturating_add(degat.x),
                        zone_fenetre.y.saturating_add(degat.y),
                        degat.largeur,
                        degat.hauteur,
                    );
                    degat_ecran = degat_ecran.union(&ecran);
                }
            }
            if client.fermeture_demandee || !client.vivant() {
                morts.push(index);
            }
        }
    }
    // A l'envers : retirer une fenetre decale celles qui suivent.
    let perte = !morts.is_empty();
    for index in morts.into_iter().rev() {
        ferme_fenetre(wins, index);
    }
    (degat_ecran, perte)
}

fn proto_rect_ecran(rect: Rect) -> Rect {
    crate::gui::protocole::rogne_degat(rect, fb::WIDTH as u32, fb::HEIGHT as u32)
}

/// Ferme une fenetre, en terminant son client s'il y en a un.
fn ferme_fenetre(wins: &mut Vec<Win>, index: usize) {
    if index >= wins.len() {
        return;
    }
    let mut w = wins.remove(index);
    if let App::Navigateur { client } = &mut w.app {
        client.termine();
    }
}

/// Ferme la fenetre du dessus. Rend `false` s'il n'y en avait aucune.
fn ferme_fenetre_du_dessus(wins: &mut Vec<Win>) -> bool {
    match wins.len() {
        0 => false,
        n => {
            ferme_fenetre(wins, n - 1);
            true
        }
    }
}

fn ferme_tous_les_clients(wins: &mut Vec<Win>) {
    // Le bureau s'en va : plus personne ne composera ces surfaces. Laisser les
    // clients vivre les laisserait peindre dans le vide — et surtout empecherait
    // le fil noyau du bureau de se terminer, puisqu'il attend que plus aucune
    // tache ne soit executable.
    for w in wins.iter_mut() {
        if let App::Navigateur { client } = &mut w.app {
            client.termine();
        }
    }
    wins.clear();
}

/// Ouvre le navigateur dans une vraie fenetre du bureau.
///
/// Le processus est lance sans attendre : la fenetre existe immediatement, avec
/// son ecran de demarrage, et se remplira a la premiere trame du client.
// BOUCHAUD_GUI_OUVERTURE_DAMAGE_V1
//
// LE DEFAUT QUE CE HELPER SUPPRIME
// --------------------------------
// Ouvrir une application par DOUBLE-CLIC sur une icone du bureau poussait la
// fenetre dans `wins` et n'annoncait RIEN : ni degat, ni `sale`. Le chemin par
// le menu Demarrer, lui, appelait `degats.tout()`.
//
// Consequence exacte, et c'est celle qu'on a vue a l'ecran : la fenetre et son
// bouton de barre des taches n'existaient que dans l'etat. Ils n'apparaissaient
// qu'au moment ou un AUTRE degat passait par la — le curseur qu'on promene par
// exemple, d'ou « la barre des taches n'affiche Fichiers que si je passe la
// souris dessus ».
//
// Une fenetre qui apparait est le seul cas ou le plein ecran est justifie : ce
// qu'elle recouvre n'a jamais ete dessine, et le bureau est seul a le savoir.
//
// Pousser directement dans `wins` est desormais interdit ailleurs qu'ici :
// `tools/verifie-ouverture-fenetre.py` echoue si un `wins.push` reapparait hors
// de cette fonction. Le contrat ne peut donc plus etre oublie a un appelant.

/// Ajoute une fenetre au bureau ET annonce ce que son apparition change.
fn ouvre_fenetre(wins: &mut Vec<Win>, fenetre: Win, degats: &mut Degats) {
    wins.push(fenetre);
    degats.tout();
}

/// Remonte au premier plan une fenetre DEJA presente, et rend son nouvel index.
///
/// Ce n'est pas une apparition : la fenetre etait deja dessinee, elle etait
/// seulement partiellement recouverte. Le degat correspondant est celui du
/// focus (`transition::focus_transfere`), pas le plein ecran — d'ou une
/// fonction distincte de `ouvre_fenetre`, pour que les deux intentions ne se
/// confondent pas a la relecture.
fn remonte_fenetre(wins: &mut Vec<Win>, index: usize) -> usize {
    let fenetre = wins.remove(index);
    wins.push(fenetre);
    wins.len() - 1
}

fn lance_navigateur(wins: &mut Vec<Win>, cwd: usize, degats: &mut Degats) {
    // Une seule instance : deux navigateurs, ce sont deux surfaces de 2,6 Mio et
    // deux Qt qui demarrent en meme temps sur un cœur unique.
    if let Some(index) = wins.iter().position(|w| window::est_client(w)) {
        let w = wins.remove(index);
        // Une instance existante remonte et se demasque : meme raison qu'une
        // apparition, ce qu'elle recouvre n'a pas ete dessine sous elle.
        ouvre_fenetre(wins, w, degats);
        if let Some(w) = wins.last_mut() {
            let id = w.id;
            route_window_command(w, crate::gui::windowing::WindowCommand::Restore(id));
        }
        return;
    }

    let (largeur_fenetre, hauteur_fenetre) = window::fenetre_pour_zone(NAV_LARGEUR, NAV_HAUTEUR);
    let client = match Client::lance(
        client::CHEMIN_NAVIGATEUR,
        cwd,
        NAV_LARGEUR as usize,
        NAV_HAUTEUR as usize,
    ) {
        Ok(client) => client,
        Err(message) => {
            crate::kernel::dmesg::log_fmt(format_args!("gui: navigateur : {}", message));
            return;
        }
    };

    let mut w = Win::new(String::from(window::TITRE_NAVIGATEUR),
        (fb::WIDTH as i32 - largeur_fenetre) / 2, BAR_H as i32 + 8,
        largeur_fenetre, hauteur_fenetre,
        crate::gui::windowing::WindowFlags::FIXED_SURFACE,
        App::Navigateur { client: alloc::boxed::Box::new(client) });
    clamp_win(&mut w);
    ouvre_fenetre(wins, w, degats);
    if let Some(App::Navigateur { client }) = wins.last_mut().map(|w| &mut w.app) {
        client.envoie_configuration(true);
    }
}

/// Transmet la position du curseur au client survole, s'il a le focus.
///
/// Seule la fenetre active recoit les mouvements : un client qui suivrait la
/// souris par-dessous une autre fenetre afficherait des survols fantomes.
fn transmet_position(wins: &mut Vec<Win>, mx: i32, my: i32, boutons: u32) {
    let index = match fenetre_active(wins) {
        Some(index) => index,
        None => return,
    };
    let zone = zone_utile(&wins[index]);
    if let App::Navigateur { client } = &mut wins[index].app {
        if let Some((x, y)) = crate::gui::protocole::vers_local(&zone, mx, my) {
            client.envoie_pointeur(x, y, boutons);
        }
    }
}

/// Transmet une transition de touche au client, appui comme relachement.
///
/// Le message porte `appui` depuis le premier jour du protocole ; ce qui
/// manquait, c'etait un pilote capable de le remplir.
fn envoie_touche(client: &mut Client, evenement: KeyEvent) {
    let (code, unicode) = match evenement.logique {
        Key::Char(octet) => (touche::CARACTERE, octet as u32),
        Key::Enter => (touche::ENTREE, 0),
        Key::Backspace => (touche::RETOUR, 0),
        Key::Tab => (touche::TABULATION, 0),
        Key::Up => (touche::HAUT, 0),
        Key::Down => (touche::BAS, 0),
        Key::Left => (touche::GAUCHE, 0),
        Key::Right => (touche::DROITE, 0),
        Key::Other => (touche::ECHAP, 0),
    };
    let m = evenement.modificateurs;
    let masque = (if m.shift { modificateur::SHIFT } else { 0 })
        | (if m.ctrl { modificateur::CTRL } else { 0 })
        | (if m.alt { modificateur::ALT } else { 0 })
        | (if m.altgr { modificateur::ALTGR } else { 0 });
    client.envoie_touche(code, unicode, masque, evenement.appui);
}

/// Route un cran de molette vers la fenetre sous le pointeur.
///
/// Chaque sortie se dit. Un cran perdu en silence ne se distingue pas d'un
/// cran jamais produit, et le defilement traverse cinq couches avant d'arriver
/// a la page : il faut pouvoir nommer celle qui l'a arrete. `delta` est deja
/// dans la convention du protocole (positif vers le haut), la conversion ayant
/// eu lieu dans `gui::mouse`.
fn handle_wheel(
    mx: i32,
    my: i32,
    delta: i32,
    wins: &mut Vec<Win>,
    degats: &mut Degats,
) -> bool {
    for i in (0..wins.len()).rev() {
        let w = &wins[i];
        if !w.min && mx >= w.x && mx < w.x + w.w && my >= w.y && my < w.y + w.h {
            let zone = zone_utile(w);
            if let App::Navigateur { client } = &mut wins[i].app {
                if let Some((client_x, client_y)) = crate::gui::protocole::vers_local(&zone, mx, my) {
                    // Le journal part APRES l'envoi, et porte son resultat : le
                    // canal est borne, et un client qui ne lit pas voit ses
                    // evenements abandonnes. Annoncer la transmission avant de
                    // la tenter faisait dire au bureau une chose qu'il ne
                    // savait pas encore.
                    let transmis = client.envoie_molette(delta, client_x, client_y);
                    crate::serial_println!(
                        "[GUI-WHEEL-TX] pid={} dx=0 dy={} screen_x={} screen_y={} client_x={} client_y={} transmis={} perdus={}",
                        client.pid, delta, mx, my, client_x, client_y,
                        transmis as u8, client.evenements_perdus,
                    );
                } else {
                    crate::serial_println!(
                        "[GUI-WHEEL-DROP] pid={} reason=outside-client screen_x={} screen_y={} dy={}",
                        client.pid, mx, my, delta,
                    );
                }
                // Un cran remis a un client ne salit RIEN cote bureau. Le
                // client repeindra s'il defile, et annoncera son degat ; le
                // fond, la barre et les cadres n'ont pas bouge.
                return false;
            }
            // Une application du noyau, pas le navigateur : le cran est bien
            // consomme, mais pas par la page. C'est une reponse a « ou est
            // passe mon defilement », pas une panne.
            crate::serial_println!(
                "[GUI-WHEEL-APP] fenetre={} screen_x={} screen_y={} dy={}",
                i, mx, my, delta,
            );
            apps::wheel_to_app(&mut wins[i], mx, my, delta);
            // Celle-la, c'est le bureau qui la dessine : sa fenetre se salit.
            degats.ajoute(Origine::Fenetre, empreinte_fenetre(&wins[i]));
            return true;
        }
    }
    crate::serial_println!(
        "[GUI-WHEEL-DROP] reason=no-window screen_x={} screen_y={} dy={}",
        mx, my, delta,
    );
    false
}

fn handle_click(
    mx: i32, my: i32, now_ms: u64,
    title_clicks: &mut crate::gui::windowing::DoubleClickDetector,
    wins: &mut Vec<Win>,
    menu_open: &mut bool,
    drag: &mut Option<Drag>,
    quit: &mut bool,
    home: usize,
    spawn_n: &mut i32,
    icon_drag: &mut Option<(usize, i32, i32, i32, i32)>,
    degats: &mut Degats,
) {
    // BOUCHAUD_GUI_DAMAGE_ORIGIN_V1
    //
    // Chaque sortie declare CE QU'ELLE A CHANGE. Le plein ecran n'est plus le
    // choix par defaut : il reste reserve a ce qui change vraiment la majorite
    // de l'image -- une fenetre qui apparait ou disparait, parce que le fond
    // qu'elle decouvre n'est connu de personne d'autre.
    if *menu_open {
        let mut fenetre_ouverte = false;
        // BOUCHAUD_GUI_HOVER_CONTRAT_V1
        //
        // Le clic lisait la ligne avec sa PROPRE formule :
        // `((my - mr.y - MENU_HEADER_H) / MENU_ITEM_H).max(0)`. Deux ecarts avec
        // ce que `draw_menu` met en valeur, et tous deux se voyaient :
        //
        //   * `.max(0)` ramene a la ligne 0 tout clic dans les 8 pixels
        //     d'entete, qui ne surlignent rien : on cliquait sur du vide et
        //     Ladybird se lancait ;
        //   * la bande d'accent de gauche n'etait pas exclue, alors qu'elle ne
        //     surligne rien non plus.
        //
        // Ce qui est surligne doit etre ce qui s'ouvre. Une seule definition.
        if let Some(row) = window::ligne_menu_survolee(mx, my) {
            if let Some(&(_, kind)) = MENU.get(row) {
                if kind == usize::MAX { *quit = true; }
                else if kind == window::KIND_NAVIGATEUR {
                    lance_navigateur(wins, home, degats);
                    fenetre_ouverte = true;
                } else {
                    let fenetre = make_app(kind, home, spawn_n);
                    ouvre_fenetre(wins, fenetre, degats);
                    fenetre_ouverte = true;
                }
            }
        }
        *menu_open = false;
        // Le menu se referme : la zone qu'il OCCUPAIT redevient bureau -- son
        // ombre portee comprise, sans quoi la bande sombre resterait a l'ecran --
        // ET le bouton Demarrer, qui change de couleur avec l'ouverture du menu.
        for (rect, cible) in transition::menu_bascule(window::menu_proto(), barre_taches_rect()).iter() {
            degats.ajoute(origine_de(cible), rect);
        }
        if fenetre_ouverte {
            // `ouvre_fenetre` a deja annonce le plein ecran ; ce drapeau ne
            // sert plus qu'a documenter qu'une fenetre est apparue ici.
            debug_assert!(!degats.vide());
        }
        return;
    }
    if start_btn().hit(mx, my) {
        *menu_open = true;
        for (rect, cible) in transition::menu_bascule(window::menu_proto(), barre_taches_rect()).iter() {
            degats.ajoute(origine_de(cible), rect);
        }
        return;
    }

    // Barre des taches : restaure (si minimisee) et donne le focus.
    for i in 0..wins.len() {
        if taskbar_btn(i).hit(mx, my) {
            // BOUCHAUD_GUI_FOCUS_DAMAGE_V1 : meme raison qu'a la remontee par
            // clic. Le bouton de la barre des taches donne aussi le focus, donc
            // il le retire aussi a quelqu'un.
            let cadre_focus_perdu = widgets::indice_focus(wins)
                .filter(|&precedent| precedent != i)
                .map(|precedent| cadre_fenetre(&wins[precedent]));
            let etait_minimisee = wins[i].min;
            let id = wins[i].id;
            route_window_command(&mut wins[i], crate::gui::windowing::WindowCommand::Restore(id));
            // Le contenu d'un client n'est pas redessine par le bureau : il est
            // recopie depuis sa surface. Apres une restauration, il faut donc
            // redemander cette recopie, sinon la fenetre reapparait vide
            // jusqu'a la prochaine trame du client — qui peut ne jamais venir
            // si la page est statique.
            if let App::Navigateur { client } = &mut wins[i].app {
                client.abime_tout();
            }
            // Remonter n'est pas apparaitre : on reordonne, on ne cree rien.
            let index = remonte_fenetre(wins, i);
            let bascule = transition::focus_transfere(
                cadre_focus_perdu, cadre_fenetre(&wins[index]), barre_taches_rect(),
            );
            for (rect, cible) in bascule.iter() {
                degats.ajoute(origine_de(cible), rect);
            }
            if etait_minimisee {
                // Une fenetre reapparait : ce qu'elle recouvre n'a jamais ete
                // dessine sous elle. Seul cas ou le plein ecran est justifie.
                degats.tout();
            }
            return;
        }
    }

    // Fenetres visibles, du dessus vers le dessous.
    let mut hit: Option<usize> = None;
    for i in (0..wins.len()).rev() {
        let w = &wins[i];
        if !w.min && mx >= w.x && mx < w.x + w.w && my >= w.y && my < w.y + w.h {
            hit = Some(i);
            break;
        }
    }
    if hit.is_none() {
        // Clic sur le bureau : commence le drag d'icone (ouverture sur double-clic).
        for j in 0..ICONS.len() {
            if icon_rect(j).hit(mx, my) {
                let (ix, iy) = unsafe { window::ICON_POSITIONS[j] };
                *icon_drag = Some((j, mx - ix, my - iy, mx, my));
                return;
            }
        }
    }
    if let Some(i) = hit {
        // Remonter une fenetre ne change des pixels QUE dans son propre
        // rectangle : elle etait deja dessinee, elle etait seulement
        // partiellement recouverte. Rien ne bouge en dehors.
        let deja_au_dessus = i + 1 == wins.len();
        // BOUCHAUD_GUI_FOCUS_DAMAGE_V1
        //
        // LE MEME DEFAUT QUE L'HORLOGE ET LE SURVOL, PAR UNE TROISIEME PORTE
        // ------------------------------------------------------------------
        // `draw_window(w, focused)` ne peint pas seulement un cadre : la barre
        // de titre passe du bleu au gris, la ligne qui la separe du contenu et
        // les quatre bordures changent de couleur avec le focus. Remonter une
        // fenetre change donc des pixels dans DEUX fenetres : celle qui monte,
        // et celle qui vient de perdre le focus.
        //
        // Seule la premiere etait invalidee. L'ancienne gardait sa barre de
        // titre bleue a l'ecran jusqu'a ce qu'un autre degat passe par la —
        // deux fenetres actives en meme temps, ce qu'aucune ne peut etre.
        //
        // On note son empreinte AVANT la reorganisation : la fenetre ne bouge
        // pas, son rectangle est donc le meme apres, et on evite l'arithmetique
        // d'indices que `remove` puis `push` imposeraient.
        let cadre_focus_perdu = widgets::indice_focus(wins)
            .filter(|&precedent| precedent != i)
            .map(|precedent| cadre_fenetre(&wins[precedent]));
        let index = remonte_fenetre(wins, i);
        if !deja_au_dessus {
            let bascule = transition::focus_transfere(
                cadre_focus_perdu, cadre_fenetre(&wins[index]), barre_taches_rect(),
            );
            for (rect, cible) in bascule.iter() {
                degats.ajoute(origine_de(cible), rect);
            }
        }
        let top = wins.last_mut().unwrap();
        let cadre_avant = cadre_fenetre(top);
        let region = crate::gui::windowing::hit_test(top.rect(),
            crate::gui::windowing::Point { x: mx, y: my },
            crate::gui::windowing::WINDOW_CHROME, top.flags.resizable);
        if region == crate::gui::windowing::HitRegion::Close && top.flags.closable {
            // Fermeture : un client a le droit d'etre prevenu et de refuser
            // (une page qui demande confirmation). Il est termine de force si
            // sa fenetre disparait de toute facon a l'iteration suivante.
            if let App::Navigateur { client } = &mut wins[index].app {
                client.demande_fermeture();
            } else {
                ferme_fenetre(wins, index);
                // Une fenetre disparait : le fond qu'elle couvrait n'est connu
                // de personne d'autre que du bureau.
                degats.tout();
            }
        } else if region == crate::gui::windowing::HitRegion::Maximize {
            let command = if top.placement == crate::gui::windowing::WindowPlacement::Maximized {
                crate::gui::windowing::WindowCommand::Restore(top.id)
            } else { crate::gui::windowing::WindowCommand::Maximize(top.id) };
            if route_window_command(top, command) {
                // Maximiser ou restaurer : l'ancienne empreinte peinte et la nouvelle.
                let mouvement = transition::fenetre_bougee(
                    cadre_avant, cadre_fenetre(&wins[index]),
                );
                for (rect, cible) in mouvement.iter() {
                    degats.ajoute(origine_de(cible), rect);
                }
            }
        } else if region == crate::gui::windowing::HitRegion::Minimize && top.flags.minimizable {
            route_window_command(top, crate::gui::windowing::WindowCommand::Minimize(top.id));
            let m = wins.pop().unwrap();
            wins.insert(0, m);
            // Elle s'efface : ce qui etait dessous doit reapparaitre.
            degats.tout();
        } else if let Some(edge) = hit_resize_edge(region) {
            if top.flags.resizable { *drag = Some(Drag::Resize(edge)); }
        } else if region == crate::gui::windowing::HitRegion::Titlebar {
            if title_clicks.click(top.id, 1, crate::gui::windowing::Point { x: mx, y: my }, now_ms) {
                let command = if top.placement == crate::gui::windowing::WindowPlacement::Maximized {
                    crate::gui::windowing::WindowCommand::Restore(top.id)
                } else { crate::gui::windowing::WindowCommand::Maximize(top.id) };
                if route_window_command(top, command) {
                    for (rect, cible) in transition::fenetre_bougee(cadre_avant, cadre_fenetre(top)).iter() {
                        degats.ajoute(origine_de(cible), rect);
                    }
                }
                return;
            }
            if top.placement != crate::gui::windowing::WindowPlacement::Normal {
                let id = top.id;
                if route_window_command(top, crate::gui::windowing::WindowCommand::Restore(id)) {
                    for (rect, cible) in transition::fenetre_bougee(cadre_avant, cadre_fenetre(top)).iter() {
                        degats.ajoute(origine_de(cible), rect);
                    }
                }
            }
            *drag = Some(Drag::Move(mx - top.x, my - top.y));
        } else {
            let zone = zone_utile(top);
            match &mut wins[index].app {
                App::Navigateur { client } => {
                    if let Some((x, y)) = crate::gui::protocole::vers_local(&zone, mx, my) {
                        client.envoie_pointeur(x, y, 1);
                    }
                    // Un clic remis a un client ne salit rien : il repeindra
                    // s'il en a besoin, et annoncera lui-meme son degat.
                }
                _ => {
                    apps::app_click(&mut wins[index], mx, my, home);
                    degats.ajoute(Origine::Fenetre, empreinte_fenetre(&wins[index]));
                }
            }
        }
    }
}

fn hit_resize_edge(region: crate::gui::windowing::HitRegion)
    -> Option<crate::gui::windowing::ResizeEdge> {
    use crate::gui::windowing::{HitRegion as H, ResizeEdge as E};
    match region {
        H::Left => Some(E::Left), H::Right => Some(E::Right), H::Top => Some(E::Top),
        H::Bottom => Some(E::Bottom), H::NorthWest => Some(E::NorthWest),
        H::NorthEast => Some(E::NorthEast), H::SouthWest => Some(E::SouthWest),
        H::SouthEast => Some(E::SouthEast), _ => None,
    }
}

fn hovered_button_rect(wins: &[Win], mx: i32, my: i32) -> Option<Rect> {
    use crate::gui::windowing::{close_button_rect, hit_test, maximize_button_rect,
        minimize_button_rect, HitRegion, Point, WINDOW_CHROME};
    let window = wins.iter().rev().find(|window| !window.min && window.rect().contains(Point { x: mx, y: my }))?;
    let rect = match hit_test(window.rect(), Point { x: mx, y: my }, WINDOW_CHROME,
        window.flags.resizable) {
        HitRegion::Close => close_button_rect(window.rect(), WINDOW_CHROME),
        HitRegion::Maximize => maximize_button_rect(window.rect(), WINDOW_CHROME),
        HitRegion::Minimize => minimize_button_rect(window.rect(), WINDOW_CHROME),
        _ => return None,
    };
    Some(Rect::neuf(rect.x, rect.y, rect.width, rect.height))
}

/// Runtime adapter: the legacy event loop now emits the same explicit command
/// model as the policy tests while `Win::window` remains the sole state owner.
fn route_window_command(window: &mut Win, command: crate::gui::windowing::WindowCommand) -> bool {
    use crate::gui::windowing::{SnapZone, WindowCommand, WindowPlacement};
    match command {
        WindowCommand::Close(id) if id == window.id => window.flags.closable,
        WindowCommand::Minimize(id) if id == window.id && window.flags.minimizable => {
            window.min = true; true
        }
        WindowCommand::Maximize(id) if id == window.id && window.flags.maximizable => {
            if window.placement == WindowPlacement::Normal { window.restore_rect = Some(window.rect()); }
            window.set_rect(crate::gui::windowing::Rect::new(0, BAR_H as i32,
                fb::WIDTH as u32, (fb::HEIGHT - 2 * BAR_H) as u32));
            window.placement = WindowPlacement::Maximized; true
        }
        WindowCommand::Restore(id) if id == window.id => {
            if window.min { window.min = false; return true }
            if let Some(rect) = window.restore_rect.take() { window.set_rect(rect); }
            window.placement = WindowPlacement::Normal; true
        }
        WindowCommand::Snap(id, zone) if id == window.id && window.flags.snappable => {
            if window.placement == WindowPlacement::Normal { window.restore_rect = Some(window.rect()); }
            let work = crate::gui::windowing::WorkArea(crate::gui::windowing::Rect::new(
                0, BAR_H as i32, fb::WIDTH as u32, (fb::HEIGHT - 2 * BAR_H) as u32));
            match zone { SnapZone::Left => { window.set_rect(work.snap_left()); window.placement=WindowPlacement::SnappedLeft; }
                SnapZone::Right => { window.set_rect(work.snap_right()); window.placement=WindowPlacement::SnappedRight; } }
            true
        }
        _ => false,
    }
}
