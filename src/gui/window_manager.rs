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
use crate::gui::event::Key;
use crate::gui::framebuffer as fb;
use crate::gui::mouse;
use crate::gui::widgets;
use crate::gui::window::{
    self as window,
    clamp_win, icon_rect, make_app, menu_rect, start_btn, taskbar_btn, toggle_max,
    zone_utile, App, Drag, Win, BAR_H, ICONS, MENU, MENU_HEADER_H, MENU_ITEM_H, MIN_H, MIN_W,
    NAV_HAUTEUR, NAV_LARGEUR, TITLE_H,
};
use crate::drivers::keyboard;
use crate::fs::ramfs;
use crate::kernel::task;
use crate::users;
use alloc::string::String;
use alloc::vec::Vec;

/// Periode minimale entre deux trames composees, en millisecondes.
///
/// 16 ms, soit environ 60 par seconde. Ce n'est pas une cadence a tenir, c'est
/// un plafond : sans changement a l'ecran, aucune trame n'est produite.
const PERIODE_TRAME_MS: u64 = 16;

/// Periode de rafraichissement des indicateurs systeme (heure, CPU, memoire).
///
/// Ils changent tout seuls, sans evenement pour l'annoncer. Une seconde est la
/// granularite de l'horloge : rafraichir plus vite ne montrerait rien de plus.
const PERIODE_HORLOGE_MS: u64 = 1000;

/// Repos court tant que l'utilisateur interagit ou qu'une trame est encore sale.
/// Les IRQ d'entree ne reveillent pas directement le fil bureau, donc on garde
/// la latence historique de 4 ms pendant la phase interactive.
const REPOS_ACTIF_TICKS: u64 = 4;

/// BOUCHAUD_CPU_OPT_DYNAMIC_WM_SLEEP: au repos, 16 ms suffisent largement (une periode de trame).
/// Cela divise jusqu'a quatre le nombre de reveils du fil noyau du bureau tout
/// en plafonnant la latence de reprise a environ une frame.
const REPOS_CALME_TICKS: u64 = 16;

/// Periode du releve de charge par processus, en millisecondes.
///
/// Cinq secondes : assez rare pour ne pas noyer le journal, assez frequent pour
/// qu'une lenteur de quelques secondes laisse au moins une trace. C'est la ligne
/// qu'on lit quand on se demande « qui prend le processeur ».
const PERIODE_RELEVE_MS: u64 = 5000;

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
const REACTIVITE_MUETTE_MS: u64 = 600;

/// Periode de recomposition d'un client muet au repos, en millisecondes.
///
/// 200 ms, soit cinq trames par seconde. Ce n'est pas une cadence d'affichage :
/// c'est un filet de securite pour le cas ou un client repeindrait de lui-meme
/// sans entree — une animation, un chargement. Cinq fois par seconde suffit a
/// ce qu'une telle page reste visiblement vivante, et divise par douze le cout
/// que payait le repos.
const REPOS_MUET_MS: u64 = 200;

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

fn boucle() {
    fb::enter();
    mouse::init();
    crate::serial_println!("[gui] window manager demarre (fil noyau)");

    let home = ramfs::fs().resolve(users::session().home(), 0).unwrap_or(0);
    let mut wins: Vec<Win> = Vec::new();
    let mut menu_open = false;
    let mut prev_left = false;
    let mut drag: Option<Drag> = None;
    let mut spawn_n = 0i32;
    // (icon_idx, offset_x_from_icon, offset_y_from_icon, start_mx, start_my)
    let mut icon_drag: Option<(usize, i32, i32, i32, i32)> = None;
    let mut last_icon_tap: Option<(usize, u64)> = None;

    wins.push(make_app(0, home, &mut spawn_n)); // un terminal pour commencer

    let mut quit = false;
    // Tout est sale au premier tour : il n'y a encore rien a l'ecran.
    let mut sale = true;
    let mut derniere_trame = 0u64;
    let mut derniere_horloge = 0u64;
    let mut derniere_souris = (usize::MAX, usize::MAX);
    let mut derniers_boutons = 0u32;
    let mut dernier_releve = 0u64;
    // Derniere entree transmise a un client, et derniere recomposition
    // « aveugle » : ensemble, ils donnent sa cadence a un client muet.
    let mut derniere_entree = 0u64;
    let mut dernier_aveugle = 0u64;

    while !quit {
        task::note_wm_heartbeat();
        let maintenant = crate::kernel::timer::monotonic_ms();

        // ---- Clavier (non bloquant) ----
        while let Some(k) = keyboard::try_key() {
            sale = true;
            // Echap ferme le menu, puis la fenetre du dessus, puis le bureau —
            // sauf quand un client a le focus. Un navigateur a besoin d'Echap
            // (arreter un chargement, fermer une boite de dialogue) et le lui
            // confisquer pour fermer sa fenetre serait le pire des deux mondes :
            // la touche ne ferait pas ce que la page attend, et detruirait le
            // travail en cours. La croix de la barre de titre reste la.
            let actif = fenetre_active(&wins);
            let client_actif = actif
                .map_or(false, |index| window::est_client(&wins[index]));
            if k == Key::Other && (menu_open || !client_actif) {
                if menu_open { menu_open = false; }
                else if !ferme_fenetre_du_dessus(&mut wins) { quit = true; }
                continue;
            }
            if let Some(index) = actif {
                if let App::Navigateur { client } = &mut wins[index].app {
                    envoie_touche(client, k);
                    derniere_entree = maintenant;
                    continue;
                }
                if apps::key_to_app(&mut wins[index], k, home) {
                    ferme_fenetre(&mut wins, index);
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
            derniere_souris = (mxu, myu);
            derniers_boutons = boutons;
            sale = true;
            derniere_entree = maintenant;
            transmet_position(&mut wins, mx, my, boutons);
        }
        if click || release || wheel != 0 {
            sale = true;
            derniere_entree = maintenant;
        }

        if left {
            if let Some(d) = drag {
                if let Some(w) = wins.last_mut() {
                    match d {
                        Drag::Move(ox, oy) => { w.x = mx - ox; w.y = my - oy; }
                        Drag::Resize => {
                            w.w = (mx - w.x).max(MIN_W);
                            w.h = (my - w.y).max(MIN_H);
                            if w.x + w.w > fb::WIDTH as i32 { w.w = fb::WIDTH as i32 - w.x; }
                            if w.y + w.h > fb::HEIGHT as i32 - BAR_H as i32 { w.h = fb::HEIGHT as i32 - BAR_H as i32 - w.y; }
                        }
                    }
                    clamp_win(w);
                }
            } else if let Some((idx, ox, oy, _, _)) = icon_drag {
                let new_x = (mx - ox).max(0);
                let new_y = (my - oy).max(BAR_H as i32);
                unsafe { window::ICON_POSITIONS[idx] = (new_x, new_y); }
            }
        } else {
            drag = None;
            if release {
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
                                lance_navigateur(&mut wins, home);
                            } else {
                                wins.push(make_app(kind, home, &mut spawn_n));
                            }
                            last_icon_tap = None;
                        } else {
                            last_icon_tap = Some((idx, tick));
                        }
                    }
                }
            }
        }

        if click {
            handle_click(mx, my, &mut wins, &mut menu_open, &mut drag, &mut quit, home, &mut spawn_n, &mut icon_drag);
        }
        if wheel != 0 {
            handle_wheel(mx, my, wheel, &mut wins);
        }

        // ---- Clients ring 3 ----
        //
        // Un client muet ne dit pas quand il peint : le compositeur recopie sa
        // surface sans savoir si elle a change. On ne le fait donc pas a chaque
        // tour, mais a une cadence qui suit ce qui peut faire bouger l'image —
        // pleine cadence juste apres une entree, cadence de veille sinon.
        let periode_aveugle = if maintenant.wrapping_sub(derniere_entree) < REACTIVITE_MUETTE_MS {
            PERIODE_TRAME_MS
        } else {
            REPOS_MUET_MS
        };
        let recompose_aveugle = maintenant.wrapping_sub(dernier_aveugle) >= periode_aveugle;
        if recompose_aveugle {
            dernier_aveugle = maintenant;
        }
        if pompe_clients(&mut wins, recompose_aveugle) {
            sale = true;
        }

        // ---- Rendu ----
        if maintenant.wrapping_sub(derniere_horloge) >= PERIODE_HORLOGE_MS {
            derniere_horloge = maintenant;
            sale = true; // horloge, charge CPU, memoire : ils bougent seuls
        }
        if sale && maintenant.wrapping_sub(derniere_trame) >= PERIODE_TRAME_MS {
            crate::kernel::timer::frame_start();
            widgets::draw_desktop(&wins);
            if menu_open { widgets::draw_menu(mx, my); }
            widgets::draw_taskbar(&wins, menu_open);
            widgets::draw_cursor(mxu, myu);
            crate::kernel::timer::mark_frame();
            fb::present();
            derniere_trame = maintenant;
            sale = false;
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

        // Rend la main. Pendant une interaction on conserve la reactivite 4 ms ;
        // une fois calme, on n'eveille plus le bureau 250 fois/s sans raison.
        // Le navigateur garde ainsi des tranches CPU nettement plus longues.
        let repos_ticks = if sale
            || left
            || maintenant.wrapping_sub(derniere_entree) < REACTIVITE_MUETTE_MS
        {
            REPOS_ACTIF_TICKS
        } else {
            REPOS_CALME_TICKS
        };
        task::sleep_ticks(repos_ticks);
        task::nettoie_zombies();
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
    let (mesures, total) = task::mesure_processus();
    if total > 0 {
        let mut ligne = String::new();
        for mesure in mesures.iter() {
            if mesure.ticks == 0 && mesure.rss_octets < 1024 * 1024 {
                continue; // rien a dire d'un processus qui n'a rien fait
            }
            if !ligne.is_empty() {
                ligne.push_str(" | ");
            }
            ligne.push_str(&alloc::format!(
                "{} pid={} cpu {}% rss {} Mio vss {} Mio{}",
                mesure.nom,
                mesure.pid,
                (mesure.ticks * 100 / total).min(100),
                mesure.rss_octets / (1024 * 1024),
                mesure.vss_octets / (1024 * 1024),
                if mesure.taches > 1 { alloc::format!(" ({} fils)", mesure.taches) } else { String::new() },
            ));
        }
        if !ligne.is_empty() {
            crate::serial_println!("[ps] {}", ligne);
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
fn pompe_clients(wins: &mut Vec<Win>, recompose_aveugle: bool) -> bool {
    let mut recompose = false;
    let mut morts: Vec<usize> = Vec::new();
    for (index, w) in wins.iter_mut().enumerate() {
        if let App::Navigateur { client } = &mut w.app {
            if client.verifie_silence() {
                recompose = true;
            }
            // Un client qui n'annonce pas ses trames est recompose « a
            // l'aveugle » : on ne sait pas quand il peint, seulement qu'il
            // peint. La cadence de cette recopie est decidee par l'appelant —
            // voir `REACTIVITE_MUETTE_MS`. La declarer ici a chaque tour
            // revenait a recopier 1100x604 pixels soixante fois par seconde
            // devant une image immobile.
            if recompose_aveugle && client.sans_protocole && !w.min {
                client.abime_tout();
                recompose = true;
            }
            if client.pompe() {
                // Le degat est consomme meme si la fenetre est minimisee :
                // l'accumuler pour rien ferait grossir un rectangle que
                // personne ne lit, et la restauration recompose de toute facon.
                let degat = client.prend_degat();
                if !w.min && !degat.vide() {
                    recompose = true;
                }
            }
            if client.fermeture_demandee || !client.vivant() {
                morts.push(index);
            }
        }
    }
    // A l'envers : retirer une fenetre decale celles qui suivent.
    for index in morts.into_iter().rev() {
        ferme_fenetre(wins, index);
        recompose = true;
    }
    recompose
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
fn lance_navigateur(wins: &mut Vec<Win>, cwd: usize) {
    // Une seule instance : deux navigateurs, ce sont deux surfaces de 2,6 Mio et
    // deux Qt qui demarrent en meme temps sur un cœur unique.
    if let Some(index) = wins.iter().position(|w| window::est_client(w)) {
        let w = wins.remove(index);
        wins.push(w);
        if let Some(w) = wins.last_mut() {
            w.min = false;
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

    let mut w = Win {
        title: String::from(window::TITRE_NAVIGATEUR),
        x: (fb::WIDTH as i32 - largeur_fenetre) / 2,
        y: BAR_H as i32 + 8,
        w: largeur_fenetre,
        h: hauteur_fenetre,
        min: false,
        restore: None,
        app: App::Navigateur { client: alloc::boxed::Box::new(client) },
    };
    clamp_win(&mut w);
    wins.push(w);
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

fn envoie_touche(client: &mut Client, k: Key) {
    let (code, unicode) = match k {
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
    client.envoie_touche(code, unicode, 0);
}

fn handle_wheel(mx: i32, my: i32, delta: i32, wins: &mut Vec<Win>) {
    for i in (0..wins.len()).rev() {
        let w = &wins[i];
        if !w.min && mx >= w.x && mx < w.x + w.w && my >= w.y && my < w.y + w.h {
            let zone = zone_utile(w);
            if let App::Navigateur { client } = &mut wins[i].app {
                if crate::gui::protocole::vers_local(&zone, mx, my).is_some() {
                    client.envoie_molette(delta);
                }
                return;
            }
            apps::wheel_to_app(&mut wins[i], mx, my, delta);
            break;
        }
    }
}

fn handle_click(
    mx: i32, my: i32,
    wins: &mut Vec<Win>,
    menu_open: &mut bool,
    drag: &mut Option<Drag>,
    quit: &mut bool,
    home: usize,
    spawn_n: &mut i32,
    icon_drag: &mut Option<(usize, i32, i32, i32, i32)>,
) {
    if *menu_open {
        let mr = menu_rect();
        if mr.hit(mx, my) {
            let row = ((my - mr.y - MENU_HEADER_H) / MENU_ITEM_H).max(0) as usize;
            if let Some(&(_, kind)) = MENU.get(row) {
                if kind == usize::MAX { *quit = true; }
                else if kind == window::KIND_NAVIGATEUR { lance_navigateur(wins, home); }
                else { wins.push(make_app(kind, home, spawn_n)); }
            }
        }
        *menu_open = false;
        return;
    }
    if start_btn().hit(mx, my) { *menu_open = true; return; }

    // Barre des taches : restaure (si minimisee) et donne le focus.
    for i in 0..wins.len() {
        if taskbar_btn(i).hit(mx, my) {
            let mut w = wins.remove(i);
            w.min = false;
            // Le contenu d'un client n'est pas redessine par le bureau : il est
            // recopie depuis sa surface. Apres une restauration, il faut donc
            // redemander cette recopie, sinon la fenetre reapparait vide
            // jusqu'a la prochaine trame du client — qui peut ne jamais venir
            // si la page est statique.
            if let App::Navigateur { client } = &mut w.app {
                client.abime_tout();
            }
            wins.push(w);
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
        let w = wins.remove(i);
        wins.push(w);
        let index = wins.len() - 1;
        let top = wins.last_mut().unwrap();
        let r = top.x + top.w;
        let on_title = my >= top.y + 1 && my < top.y + TITLE_H;
        if on_title && mx >= r - 10 && mx < r - 1 {
            // Fermeture : un client a le droit d'etre prevenu et de refuser
            // (une page qui demande confirmation). Il est termine de force si
            // sa fenetre disparait de toute facon a l'iteration suivante.
            if let App::Navigateur { client } = &mut wins[index].app {
                client.demande_fermeture();
            } else {
                ferme_fenetre(wins, index);
            }
        } else if on_title && mx >= r - 19 && mx < r - 10 {
            toggle_max(top);
        } else if on_title && mx >= r - 28 && mx < r - 19 {
            top.min = true;
            let m = wins.pop().unwrap();
            wins.insert(0, m);
        } else if !window::est_client(top) && my >= top.y + top.h - 8 && mx >= r - 8 {
            *drag = Some(Drag::Resize);
        } else if my < top.y + TITLE_H {
            *drag = Some(Drag::Move(mx - top.x, my - top.y));
        } else {
            let zone = zone_utile(top);
            match &mut wins[index].app {
                App::Navigateur { client } => {
                    if let Some((x, y)) = crate::gui::protocole::vers_local(&zone, mx, my) {
                        client.envoie_pointeur(x, y, 1);
                    }
                }
                _ => apps::app_click(&mut wins[index], mx, my, home),
            }
        }
    }
}
