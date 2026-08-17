//! Session entre le gestionnaire de fenetres et un client graphique ring 3.
//!
//! ## Ce que ce module tient
//!
//! Trois choses, et rien d'autre : le processus du client, la surface qu'il
//! peint, et le canal par lequel les deux se parlent. Le format des messages
//! vit dans [`crate::gui::protocole`] ; le rendu vit dans `widgets`. Ici on ne
//! fait que lancer, transporter et recolter.
//!
//! ## Le contrat
//!
//! Le gestionnaire de fenetres est **seul proprietaire du framebuffer
//! physique**. Le client ne le voit jamais : son `/dev/fb0` est redirige vers la
//! surface (voir `Process::ecran`), et ses `/dev/input/event*` lui sont refuses.
//! Ce n'est pas une convention entre gens de bonne compagnie, c'est le noyau qui
//! le tient — un client qui « oublierait » de respecter le protocole n'aurait
//! toujours aucun moyen d'ecrire un pixel a l'ecran ni de voler une touche.
//!
//! ## Transport
//!
//! Une paire de canaux, exactement ce qu'un `socketpair` donne a deux processus
//! ring 3. Le gestionnaire de fenetres vivant dans le noyau, il tient ses deux
//! extremites directement plutot que par un descripteur. Le jour ou un serveur
//! graphique passera en ring 3, seul ce module changera.
//!
//! ## Contre-pression
//!
//! Le canal a une capacite (64 KiB). Un client qui ne lit pas ses evenements ne
//! doit pas pouvoir bloquer le compositeur : les envois qui ne tiennent pas sont
//! **abandonnes**, et les mouvements de souris sont fusionnes avant meme d'etre
//! proposes au canal. C'est la lecon du canal du renderer, appliquee dans
//! l'autre sens : un producteur qui insiste finit par prendre toute la memoire,
//! ou par corrompre son cadrage.

use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cell::RefCell;

use crate::gui::protocole::{self as proto, Genre, Lecture, Rect};
use crate::gui::surface::Surface;
use crate::kernel::fd::{Canal, FdKind, FileDesc, CAPACITE_CANAL};
use crate::kernel::task::{self, EcranVirtuel, Priorite};

/// Chemin du navigateur dans le RAMFS.
pub const CHEMIN_NAVIGATEUR: &str = "/bo-navigateur";

/// Etat d'un client vu par le gestionnaire de fenetres.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Etat {
    /// Processus lance, aucune trame recue : le bureau affiche le splash.
    Demarrage,
    /// Au moins une trame composee.
    Actif,
    /// Le processus est mort (sortie normale, faute, ou fermeture demandee).
    Termine,
}

pub struct Client {
    pub pid: u32,
    pub etat: Etat,
    pub titre: String,
    pub surface: Surface,
    /// Ce que le gestionnaire de fenetres ecrit, le client lit.
    vers_client: Rc<RefCell<Canal>>,
    /// Ce que le client ecrit, le gestionnaire de fenetres lit.
    vers_wm: Rc<RefCell<Canal>>,
    /// Messages recus incomplets, en attente de la suite.
    tampon: Vec<u8>,
    /// Degat accumule depuis la derniere composition, dans le repere surface.
    degat: Rect,
    serie: u32,
    /// Le client a-t-il dit bonjour ? Tant qu'il ne l'a pas fait, il n'utilise
    /// pas le protocole et le compositeur retombe sur un rythme fixe.
    pub protocole_actif: bool,
    /// Le client a demande sa fermeture (`Close`).
    pub fermeture_demandee: bool,
    /// Compteurs, pour que le journal puisse dire ce que fait ce client.
    ///
    /// Sans eux, un client qui ne repond plus et un client qui repond
    /// parfaitement mais peint du blanc se ressemblent beaucoup — dans les deux
    /// cas l'ecran ne bouge pas. Le nombre de trames par seconde tranche la
    /// question en une ligne de journal.
    pub trames: u64,
    pub octets_recus: u64,
    pub evenements_envoyes: u64,
    pub evenements_perdus: u64,
    /// Tick de la derniere trame recue.
    pub derniere_trame: u64,
    /// Tick du lancement.
    debut: u64,
    /// Le client peint sans annoncer ses trames : on compose au rythme fixe.
    pub sans_protocole: bool,
}

impl Client {
    /// Lance un programme en tant que client du gestionnaire de fenetres.
    ///
    /// Rend la main immediatement : le processus est enregistre dans
    /// l'ordonnanceur, pas execute jusqu'a sa fin. C'est toute la difference
    /// avec l'ancien `exec` synchrone, et c'est ce qui permet au bureau de
    /// continuer a vivre pendant que le navigateur demarre.
    pub fn lance(
        chemin: &str,
        cwd: usize,
        largeur: usize,
        hauteur: usize,
    ) -> Result<Client, String> {
        if crate::fs::ramfs::fs().resolve_checked(chemin, cwd).is_err() {
            return Err(alloc::format!(
                "{} absent — voir tools/userland/build-navigateur.sh",
                chemin
            ));
        }

        let surface = Surface::nouvelle(largeur, hauteur)
            .ok_or_else(|| "surface partagee impossible (memoire)".to_string())?;
        let vers_client = Canal::neuf();
        let vers_wm = Canal::neuf();

        let ecran = EcranVirtuel {
            node: surface.node,
            largeur: surface.largeur as u32,
            hauteur: surface.hauteur as u32,
            pas: surface.pas as u32,
        };

        let argv = alloc::vec![chemin.to_string()];
        let base = crate::kernel::exec::shell_environment();

        let surface_node = surface.node;
        let (largeur_px, hauteur_px, pas) = (surface.largeur, surface.hauteur, surface.pas);
        let canal_client = vers_client.clone();
        let canal_wm = vers_wm.clone();

        let mut prepare = move |processus: &mut task::Process| -> Vec<String> {
            processus.ecran = Some(ecran);
            let fd_surface = processus.files.insert(FileDesc::new(FdKind::File(surface_node)));
            let fd_gui = processus.files.insert(FileDesc::new(FdKind::SocketPair(
                canal_client.clone(),
                canal_wm.clone(),
            )));
            alloc::vec![
                alloc::format!("BO_SURFACE_FD={}", fd_surface),
                alloc::format!("BO_SURFACE_WIDTH={}", largeur_px),
                alloc::format!("BO_SURFACE_HEIGHT={}", hauteur_px),
                alloc::format!("BO_SURFACE_STRIDE={}", pas),
                alloc::format!("BO_GUI_FD={}", fd_gui),
                // Qt dimensionne son ecran sur la fenetre, pas sur la dalle : le
                // client ne doit meme pas savoir que l'ecran est plus grand.
                alloc::format!(
                    "QT_QPA_PLATFORM_PLUGIN_ARGS=fb=/dev/fb0:size={}x{}",
                    largeur_px, hauteur_px
                ),
                // Ceinture : le noyau refuse deja `/dev/input/*` a un client qui
                // a un ecran virtuel. Le dire aussi a Qt lui evite d'essayer et
                // de journaliser un echec a chaque demarrage.
                "QT_QPA_FB_DISABLE_INPUT=1".to_string(),
            ]
        };

        let pid = crate::kernel::exec::lance_detache(chemin, &argv, &base, cwd, &mut prepare)?;
        // L'interface d'un client graphique passe avant le calcul de fond :
        // c'est exactement ce que la classe interactive designe. Le renderer,
        // ne d'un `fork`, gardera la priorite normale de sa naissance.
        task::pose_priorite_de(pid, Priorite::Interactive);

        crate::kernel::dmesg::log_fmt(format_args!(
            "gui: client {} pid={} surface {}x{} (ecran virtuel, /dev/fb0 redirige)",
            chemin, pid, largeur_px, hauteur_px
        ));

        let mut client = Client {
            pid,
            etat: Etat::Demarrage,
            titre: "Bouchaud Browser".to_string(),
            surface,
            vers_client,
            vers_wm,
            tampon: Vec::new(),
            degat: Rect::default(),
            serie: 0,
            protocole_actif: false,
            fermeture_demandee: false,
            trames: 0,
            octets_recus: 0,
            evenements_envoyes: 0,
            evenements_perdus: 0,
            derniere_trame: 0,
            debut: crate::kernel::timer::ticks(),
            sans_protocole: false,
        };
        client.annonce_surface();
        Ok(client)
    }

    // --- Emission ------------------------------------------------------------

    /// Ecrit un message vers le client, ou l'abandonne s'il n'y a plus de place.
    ///
    /// Rend `false` quand le message est tombe. Ce n'est pas une erreur : le
    /// protocole est concu pour que perdre un evenement de position ou une
    /// configuration soit rattrape par le suivant. Ce qui serait une erreur,
    /// c'est de faire attendre le compositeur.
    fn envoie(&mut self, genre: Genre, charge: &[u8]) -> bool {
        self.serie = self.serie.wrapping_add(1);
        let message = proto::message(genre, self.serie, charge);
        let mut canal = self.vers_client.borrow_mut();
        if canal.lecteurs == 0 || canal.place() < message.len() {
            drop(canal);
            self.evenements_perdus += 1;
            return false;
        }
        canal.octets.extend_from_slice(&message);
        drop(canal);
        self.evenements_envoyes += 1;
        true
    }

    fn annonce_surface(&mut self) {
        let charge = proto::Surface {
            fenetre: proto::FENETRE_PRINCIPALE,
            tampon: 0,
            largeur: self.surface.largeur as u32,
            hauteur: self.surface.hauteur as u32,
            pas: self.surface.pas as u32,
            format: 0,
        }
        .encode();
        self.envoie(Genre::Surface, &charge);
    }

    pub fn envoie_configuration(&mut self, focus: bool) {
        let charge = proto::Configure {
            fenetre: proto::FENETRE_PRINCIPALE,
            largeur: self.surface.largeur as u32,
            hauteur: self.surface.hauteur as u32,
            focus: focus as u32,
        }
        .encode();
        self.envoie(Genre::Configure, &charge);
    }

    /// Position du curseur, deja convertie dans le repere de la zone utile.
    ///
    /// Les mouvements sont fusionnes : si le client n'a pas encore lu le
    /// precedent, on remplace sa charge utile au lieu d'en empiler une seconde.
    /// Sans cela, un deplacement rapide de la souris remplit 64 KiB de positions
    /// perimees que le client traitera une par une, longtemps apres.
    pub fn envoie_pointeur(&mut self, x: i32, y: i32, boutons: u32) {
        let charge = proto::Pointeur { fenetre: proto::FENETRE_PRINCIPALE, x, y, boutons }.encode();
        if self.remplace_dernier(Genre::Pointer, &charge) {
            return;
        }
        self.envoie(Genre::Pointer, &charge);
    }

    /// Remplace en place le dernier message du canal s'il est du meme genre et
    /// n'a pas encore ete lu. Rend `true` si le remplacement a eu lieu.
    fn remplace_dernier(&mut self, genre: Genre, charge: &[u8]) -> bool {
        let mut canal = self.vers_client.borrow_mut();
        let taille = proto::TAILLE_ENTETE + charge.len();
        if canal.octets.len() < taille {
            return false;
        }
        let debut = canal.octets.len() - taille;
        let entete = match proto::Entete::decode(&canal.octets[debut..]) {
            Some(entete) => entete,
            None => return false,
        };
        // La validite compte autant que le genre : sans elle, la fin de la
        // charge utile d'un autre message pourrait passer pour un en-tete et
        // faire ecraser des octets au milieu du flux.
        if !entete.valide()
            || entete.genre != genre as u16
            || entete.taille_charge as usize != charge.len()
        {
            return false;
        }
        let position = debut + proto::TAILLE_ENTETE;
        canal.octets[position..position + charge.len()].copy_from_slice(charge);
        true
    }

    pub fn envoie_molette(&mut self, delta: i32) {
        let charge = proto::Molette { fenetre: proto::FENETRE_PRINCIPALE, delta }.encode();
        self.envoie(Genre::Wheel, &charge);
    }

    pub fn envoie_touche(&mut self, code: u32, unicode: u32, modificateurs: u32) {
        let charge = proto::Touche {
            fenetre: proto::FENETRE_PRINCIPALE,
            code,
            modificateurs,
            unicode,
            appui: 1,
        }
        .encode();
        self.envoie(Genre::Key, &charge);
    }

    /// Demande poliment au client de se fermer.
    pub fn demande_fermeture(&mut self) {
        let charge = proto::FENETRE_PRINCIPALE.to_le_bytes();
        self.envoie(Genre::CloseRequest, &charge);
        self.fermeture_demandee = true;
    }

    // --- Reception -----------------------------------------------------------

    /// Consomme ce que le client a ecrit. Rend `true` si une trame est prete.
    ///
    /// Le tampon partiel est conserve entre deux appels : un message a cheval
    /// sur deux lectures ne se perd pas, et surtout n'est jamais reinterprete
    /// depuis le mauvais octet.
    pub fn pompe(&mut self) -> bool {
        {
            let mut canal = self.vers_wm.borrow_mut();
            if !canal.octets.is_empty() {
                self.octets_recus += canal.octets.len() as u64;
                self.tampon.append(&mut canal.octets);
            }
            // Un tampon qui grossit sans jamais former de message est un flux
            // qui ne parle pas notre protocole. On le coupe plutot que de le
            // laisser manger la memoire du noyau.
            if self.tampon.len() > CAPACITE_CANAL {
                self.tampon.clear();
                self.protocole_actif = false;
            }
        }

        let mut trame_prete = false;
        loop {
            match proto::examine(&self.tampon) {
                Lecture::Incomplet => break,
                Lecture::Invalide => {
                    crate::kernel::dmesg::log("gui: flux client invalide, canal ignore");
                    self.tampon.clear();
                    self.protocole_actif = false;
                    break;
                }
                Lecture::Message { genre, debut, taille, total } => {
                    let charge: Vec<u8> = self.tampon[debut..debut + taille].to_vec();
                    self.tampon.drain(..total);
                    if self.traite(genre, &charge) {
                        trame_prete = true;
                    }
                }
            }
        }
        trame_prete
    }

    /// Traite un message client. Rend `true` si c'est une trame a composer.
    fn traite(&mut self, genre: Genre, charge: &[u8]) -> bool {
        match genre {
            Genre::Hello => {
                self.protocole_actif = true;
                crate::kernel::dmesg::log_fmt(format_args!(
                    "gui: client pid={} parle le protocole v{}",
                    self.pid,
                    proto::PROTOCOL_VERSION
                ));
                self.annonce_surface();
                false
            }
            Genre::CreateWindow => {
                self.protocole_actif = true;
                false
            }
            Genre::SetTitle => {
                if let Ok(titre) = core::str::from_utf8(charge) {
                    // Un titre est affiche dans une barre de 1100 pixels : le
                    // borner ici evite qu'un client fasse allouer une chaine
                    // que personne ne pourra jamais lire.
                    self.titre = titre.chars().take(96).collect();
                }
                false
            }
            Genre::Damage | Genre::FrameReady => {
                let degat = if genre == Genre::FrameReady {
                    proto::Trame::decode(charge).map(|trame| trame.degat)
                } else {
                    // `Damage` porte l'identifiant de fenetre puis le rectangle.
                    if charge.len() >= 20 { Rect::decode(&charge[4..20]) } else { None }
                };
                let degat = match degat {
                    Some(degat) => proto::rogne_degat(
                        degat,
                        self.surface.largeur as u32,
                        self.surface.hauteur as u32,
                    ),
                    None => return false,
                };
                if degat.vide() {
                    return false;
                }
                self.degat = self.degat.union(&degat);
                self.protocole_actif = true;
                if self.derniere_trame == 0 {
                    crate::kernel::dmesg::log_fmt(format_args!(
                        "gui: premiere trame du client pid={} ({}x{})",
                        self.pid, degat.largeur, degat.hauteur
                    ));
                }
                self.etat = Etat::Actif;
                self.trames += 1;
                self.derniere_trame = crate::kernel::timer::ticks();
                true
            }
            Genre::Close => {
                self.fermeture_demandee = true;
                false
            }
            // Les messages descendants n'ont rien a faire dans ce sens.
            _ => false,
        }
    }

    /// Delai au-dela duquel un client muet est repute ne pas parler le protocole.
    ///
    /// Qt, CPython et le moteur mettent plusieurs secondes a demarrer sous
    /// emulation ; en dessous, on conclurait trop tot.
    const PATIENCE_MS: u64 = 6000;

    /// Un client qui n'a jamais dit bonjour peint-il quand meme ?
    ///
    /// Le cas existe pour de bon : l'image userland est construite par la CI et
    /// peut preceder la version du noyau qu'on demarre. Un binaire d'avant le
    /// protocole ouvre `/dev/fb0` — c'est-a-dire la surface — et peint dedans
    /// sans rien annoncer. Sa fenetre resterait indefiniment sur l'ecran de
    /// demarrage alors que l'image est la, a portee de `memcpy`.
    ///
    /// Passe le delai, on compose donc au rythme fixe du compositeur. C'est
    /// moins efficace — on recopie parfois pour rien — mais c'est la difference
    /// entre un navigateur qui s'affiche et une fenetre vide.
    pub fn verifie_silence(&mut self) -> bool {
        if self.protocole_actif || self.sans_protocole || self.etat == Etat::Termine {
            return false;
        }
        let age = crate::kernel::timer::ticks().saturating_sub(self.debut);
        if age < crate::kernel::timer::ms_to_ticks(Self::PATIENCE_MS) {
            return false;
        }
        crate::kernel::dmesg::log_fmt(format_args!(
            "gui: client pid={} muet apres {} s — composition au rythme fixe",
            self.pid,
            Self::PATIENCE_MS / 1000
        ));
        self.sans_protocole = true;
        self.etat = Etat::Actif;
        self.abime_tout();
        true
    }

    /// Une ligne d'etat pour le journal.
    ///
    /// Format volontairement dense : elle est destinee a etre lue au fil de
    /// l'eau, une fois toutes les quelques secondes, a cote des autres lignes.
    pub fn etat_journal(&self, periode_ms: u64) -> String {
        let par_seconde = if periode_ms > 0 { self.trames * 1000 / periode_ms.max(1) } else { 0 };
        let silence = crate::kernel::timer::ticks().saturating_sub(self.derniere_trame);
        alloc::format!(
            "pid={} {} {} trames ({}/s, silence {} ms) recu {} o, envoye {} ev ({} perdus)",
            self.pid,
            match self.etat {
                Etat::Demarrage => "demarrage",
                Etat::Actif => "actif",
                Etat::Termine => "termine",
            },
            self.trames,
            par_seconde,
            if self.derniere_trame == 0 { 0 } else { silence },
            self.octets_recus,
            self.evenements_envoyes,
            self.evenements_perdus,
        )
    }

    /// Remet a zero les compteurs de periode.
    pub fn remet_compteurs(&mut self) {
        self.trames = 0;
        self.octets_recus = 0;
        self.evenements_envoyes = 0;
        self.evenements_perdus = 0;
    }

    /// Le degat accumule, et le remet a zero.
    pub fn prend_degat(&mut self) -> Rect {
        core::mem::replace(&mut self.degat, Rect::default())
    }

    /// Marque toute la surface comme abimee.
    ///
    /// Employe quand le bureau se redessine entierement — deplacement d'une
    /// fenetre, retour depuis la barre des taches : le contenu du client n'a pas
    /// change, mais les pixels de l'ecran, si.
    pub fn abime_tout(&mut self) {
        self.degat = Rect::neuf(0, 0, self.surface.largeur as u32, self.surface.hauteur as u32);
    }

    // --- Cycle de vie ---------------------------------------------------------

    /// Met a jour l'etat en consultant l'ordonnanceur. Rend `true` s'il vit.
    pub fn vivant(&mut self) -> bool {
        if self.etat == Etat::Termine {
            return false;
        }
        if let Some(code) = task::code_de_sortie(self.pid) {
            crate::kernel::dmesg::log_fmt(format_args!(
                "gui: client pid={} termine (code {})",
                self.pid, code
            ));
            self.etat = Etat::Termine;
            return false;
        }
        true
    }

    /// Termine le client et rend ses ressources.
    ///
    /// La surface part avec le `Client` : c'est `Drop` qui rendra sa reference
    /// au cache de pages, une fois que le processus aura lache ses mappages.
    pub fn termine(&mut self) {
        // L'arbre, pas la racine : le navigateur forke un renderer par onglet,
        // et un renderer orphelin continuerait de calculer une page que plus
        // personne n'affiche.
        let arbre = task::arbre_de(self.pid);
        for pid in &arbre {
            task::tue_processus(*pid, 0);
        }
        self.etat = Etat::Termine;
        // Les taches d'abord, les processus ensuite : un processus se libere
        // quand plus aucune tache ne le reference, et c'est sa disparition qui
        // rend les pages partagees au cache.
        task::nettoie_zombies();
        for pid in arbre {
            task::collect_child(pid);
        }
    }
}

impl Drop for Client {
    /// Filet de securite : une fenetre detruite emporte son processus.
    ///
    /// Le chemin ordinaire passe par `termine`, appele par le gestionnaire de
    /// fenetres. Celui-ci couvre les autres — une erreur en cours de lancement,
    /// un `Vec` de fenetres vide d'un coup. Sans lui, un client survivrait a sa
    /// fenetre : il continuerait a peindre dans une surface que plus personne ne
    /// compose, et cette surface ne serait jamais liberee.
    fn drop(&mut self) {
        self.termine();
    }
}
