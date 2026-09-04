//! Session GUI entre le gestionnaire de fenêtres et un client ring 3.
//!
//! Ce module conserve le contrat historique (surface + deux canaux) et ajoute
//! deux propriétés de performance :
//! - coalescence des Wheel consécutifs non lus, sans perdre la distance totale ;
//! - corrélation input -> FrameReady via `kernel::perf`.

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::gui::jauge::Jauge;
use crate::gui::protocole::{self as proto, Genre, Lecture, Rect};
use crate::gui::silence::VerdictProtocole;
use crate::gui::surface::Surface;
use crate::kernel::fd::{Canal, FdKind, FileDesc, CAPACITE_CANAL};
use crate::kernel::sync::SpinLock;
use crate::kernel::task::{self, EcranVirtuel, Priorite};

pub const CHEMIN_NAVIGATEUR: &str = "/bo-navigateur";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Etat {
    Demarrage,
    Actif,
    Termine,
}

pub struct Client {
    pub pid: u32,
    pub etat: Etat,
    pub titre: String,
    pub surface: Surface,
    vers_client: Arc<SpinLock<Canal>>,
    vers_wm: Arc<SpinLock<Canal>>,
    tampon: Vec<u8>,
    degat: Rect,
    serie: u32,
    verdict: VerdictProtocole,
    pub fermeture_demandee: bool,
    pub trames: u64,
    pub octets_recus: u64,
    pub evenements_envoyes: u64,
    pub evenements_perdus: u64,
    pub derniere_trame: u64,
    debut: u64,
    /// Lancement du client, sur l'horloge monotone.
    naissance_ms: u64,
    /// Chronometre de chargement affiche dans la fenetre. Voir `gui::jauge`.
    ///
    /// BOUCHAUD_C13_JAUGE_DE_CHARGEMENT_V1
    pub jauge: Jauge,
}

impl Client {
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
        vers_wm.lock().reveille_compositeur = true;

        let ecran = EcranVirtuel {
            node: surface.node,
            largeur: surface.largeur as u32,
            hauteur: surface.hauteur as u32,
            pas: surface.pas as u32,
        };

        let argv = alloc::vec![chemin.to_string()];
        let base = crate::kernel::exec::shell_environment();

        let surface_node = surface.node;
        let (largeur_px, hauteur_px, pas) =
            (surface.largeur, surface.hauteur, surface.pas);
        let canal_client = vers_client.clone();
        let canal_wm = vers_wm.clone();

        let mut prepare = move |processus: &task::Process| -> Vec<String> {
            processus.metadata.lock().ecran = Some(ecran);
            let mut files = processus.files.lock();
            let fd_surface =
                files.insert(FileDesc::fichier_partage_inscriptible(surface_node));
            let fd_gui = files.insert(FileDesc::new(FdKind::SocketPair(
                canal_client.clone(),
                canal_wm.clone(),
            )));
            alloc::vec![
                alloc::format!("BO_SURFACE_FD={}", fd_surface),
                alloc::format!("BO_SURFACE_WIDTH={}", largeur_px),
                alloc::format!("BO_SURFACE_HEIGHT={}", hauteur_px),
                alloc::format!("BO_SURFACE_STRIDE={}", pas),
                alloc::format!("BO_GUI_FD={}", fd_gui),
                alloc::format!(
                    "QT_QPA_PLATFORM_PLUGIN_ARGS=fb=/dev/fb0:size={}x{}",
                    largeur_px, hauteur_px
                ),
                "QT_QPA_FB_DISABLE_INPUT=1".to_string(),
            ]
        };

        let pid =
            crate::kernel::exec::lance_detache(chemin, &argv, &base, cwd, &mut prepare)?;
        task::pose_priorite_de(pid, Priorite::Interactive);
        crate::kernel::perf::browser_client_start(pid);

        crate::kernel::dmesg::log_fmt(format_args!(
            "gui: client {} pid={} surface {}x{} (ecran virtuel, /dev/fb0 redirige)",
            chemin, pid, largeur_px, hauteur_px
        ));

        // Une seule lecture d'horloge : le journal et la jauge doivent dater le
        // lancement du MEME instant, sinon les deux durees de demarrage
        // publiees ne coincident pas.
        let naissance = crate::kernel::timer::monotonic_ms();
        let mut client = Client {
            pid,
            etat: Etat::Demarrage,
            titre: crate::gui::window::TITRE_NAVIGATEUR.to_string(),
            surface,
            vers_client,
            vers_wm,
            tampon: Vec::new(),
            degat: Rect::default(),
            serie: 0,
            verdict: VerdictProtocole::neuf(),
            fermeture_demandee: false,
            trames: 0,
            octets_recus: 0,
            evenements_envoyes: 0,
            evenements_perdus: 0,
            derniere_trame: 0,
            naissance_ms: naissance,
            jauge: Jauge::neuve(naissance),
            debut: crate::kernel::timer::ticks(),
        };
        client.annonce_surface();
        Ok(client)
    }

    fn envoie(&mut self, genre: Genre, charge: &[u8]) -> bool {
        self.serie = self.serie.wrapping_add(1);
        let message = proto::message(genre, self.serie, charge);
        let mut canal = self.vers_client.lock();
        if canal.lecteurs == 0 || canal.place() < message.len() {
            drop(canal);
            self.evenements_perdus += 1;
            return false;
        }
        canal.octets.extend_from_slice(&message);
        drop(canal);
        crate::kernel::fd::notify_readiness();
        self.evenements_envoyes += 1;
        if matches!(genre, Genre::Key | Genre::Pointer | Genre::Wheel) {
            // Ce qui distingue un chargement d'un defilement. Voir `gui::jauge`.
            self.jauge.note_entree(crate::kernel::timer::monotonic_ms());
        }
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
            // Le compositeur noyau presente encore un pixel logique pour un
            // pixel physique. Annoncer l'echelle EXPLICITEMENT plutot que de
            // laisser le champ a zero est ce qui rend la valeur lisible : un
            // client ne peut pas distinguer « pas d'echelle » de « echelle
            // absente » si le compositeur ne dit rien.
            echelle: crate::gui::echelle_affichage(),
            reserve: 0,
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
            echelle: crate::gui::echelle_affichage(),
            reserve: 0,
        }
        .encode();
        self.envoie(Genre::Configure, &charge);
    }

    pub fn envoie_pointeur(&mut self, x: i32, y: i32, boutons: u32) {
        let charge = proto::Pointeur {
            fenetre: proto::FENETRE_PRINCIPALE,
            x,
            y,
            boutons,
        }
        .encode();
        if self.remplace_dernier(Genre::Pointer, &charge) {
            return;
        }
        self.envoie(Genre::Pointer, &charge);
    }

    fn remplace_dernier(&mut self, genre: Genre, charge: &[u8]) -> bool {
        let mut canal = self.vers_client.lock();
        let taille = proto::TAILLE_ENTETE + charge.len();
        if canal.octets.len() < taille {
            return false;
        }
        let debut = canal.octets.len() - taille;
        let entete = match proto::Entete::decode(&canal.octets[debut..]) {
            Some(entete) => entete,
            None => return false,
        };
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

    /// Fusionne uniquement si LE DERNIER message non lu est déjà un Wheel.
    ///
    /// Les deltas s'additionnent : 3 crans restent 3 crans. On évite seulement
    /// que WebContent doive vider une file d'inputs périmés avant de repeindre.
    fn fusionne_derniere_molette(&mut self, delta: i32, x: i32, y: i32) -> bool {
        const CHARGE: usize = 16;
        let total = proto::TAILLE_ENTETE + CHARGE;
        let mut canal = self.vers_client.lock();

        if canal.octets.len() < total {
            return false;
        }

        let debut = canal.octets.len() - total;
        let entete = match proto::Entete::decode(&canal.octets[debut..]) {
            Some(v) => v,
            None => return false,
        };
        if !entete.valide()
            || entete.genre != Genre::Wheel as u16
            || entete.taille_charge as usize != CHARGE
        {
            return false;
        }

        let position = debut + proto::TAILLE_ENTETE;
        let ancienne = match proto::Molette::decode(&canal.octets[position..position + CHARGE]) {
            Some(v) => v,
            None => return false,
        };
        let nouvelle = proto::Molette {
            fenetre: proto::FENETRE_PRINCIPALE,
            delta: ancienne.delta.saturating_add(delta),
            x,
            y,
        }
        .encode();
        canal.octets[position..position + CHARGE].copy_from_slice(&nouvelle);
        true
    }

    pub fn envoie_molette(&mut self, delta: i32, x: i32, y: i32) -> bool {
        let fusionnee = self.fusionne_derniere_molette(delta, x, y);
        let ok = if fusionnee {
            true
        } else {
            let charge = proto::Molette {
                fenetre: proto::FENETRE_PRINCIPALE,
                delta,
                x,
                y,
            }
            .encode();
            self.envoie(Genre::Wheel, &charge)
        };
        crate::kernel::perf::browser_input(self.pid, ok, fusionnee);
        ok
    }

    pub fn envoie_touche(
        &mut self,
        code: u32,
        unicode: u32,
        modificateurs: u32,
        appui: bool,
    ) {
        let charge = proto::Touche {
            fenetre: proto::FENETRE_PRINCIPALE,
            code,
            modificateurs,
            unicode,
            appui: appui as u32,
        }
        .encode();
        self.envoie(Genre::Key, &charge);
    }

    pub fn demande_fermeture(&mut self) {
        let charge = proto::FENETRE_PRINCIPALE.to_le_bytes();
        self.envoie(Genre::CloseRequest, &charge);
        self.fermeture_demandee = true;
    }

    pub fn pompe(&mut self) -> bool {
        {
            let mut canal = self.vers_wm.lock();
            if !canal.octets.is_empty() {
                self.octets_recus += canal.octets.len() as u64;
                self.tampon.append(&mut canal.octets);
            }
            if self.tampon.len() > CAPACITE_CANAL {
                self.tampon.clear();
                self.verdict.retire_le_protocole();
            }
        }

        let mut trame_prete = false;
        loop {
            match proto::examine(&self.tampon) {
                Lecture::Incomplet => break,
                Lecture::Invalide => {
                    crate::kernel::dmesg::log("gui: flux client invalide, canal ignore");
                    self.tampon.clear();
                    self.verdict.retire_le_protocole();
                    break;
                }
                Lecture::Message {
                    genre,
                    debut,
                    taille,
                    total,
                } => {
                    let charge: Vec<u8> =
                        self.tampon[debut..debut + taille].to_vec();
                    self.tampon.drain(..total);
                    if self.traite(genre, &charge) {
                        trame_prete = true;
                    }
                }
            }
        }
        trame_prete
    }

    pub fn protocole_actif(&self) -> bool {
        self.verdict.protocole_actif()
    }

    pub fn recompose_a_l_aveugle(&self) -> bool {
        self.verdict.recompose_a_l_aveugle()
    }

    fn marque_protocole_actif(&mut self) {
        if self.verdict.marque_protocole_actif() {
            crate::kernel::dmesg::log_fmt(format_args!(
                "gui: client pid={} parle finalement le protocole — fin de la recomposition au rythme fixe",
                self.pid
            ));
        }
    }

    fn traite(&mut self, genre: Genre, charge: &[u8]) -> bool {
        match genre {
            Genre::Hello => {
                self.marque_protocole_actif();
                crate::kernel::dmesg::log_fmt(format_args!(
                    "gui: client pid={} parle le protocole v{}",
                    self.pid,
                    proto::PROTOCOL_VERSION
                ));
                self.annonce_surface();
                false
            }
            Genre::CreateWindow => {
                self.marque_protocole_actif();
                false
            }
            Genre::SetTitle => {
                if let Ok(titre) = core::str::from_utf8(charge) {
                    let nouveau: String = titre.chars().take(96).collect();
                    // Un titre REPETE n'est pas une navigation. Ladybird
                    // reannonce le sien a chaque changement d'onglet interne ;
                    // sans ce test, chacun aurait remis le chronometre a zero.
                    if nouveau != self.titre {
                        self.titre = nouveau;
                        self.jauge.note_titre(crate::kernel::timer::monotonic_ms());
                    }
                }
                false
            }
            Genre::Damage | Genre::FrameReady => {
                let degat = if genre == Genre::FrameReady {
                    proto::Trame::decode(charge).map(|trame| trame.degat)
                } else if charge.len() >= 20 {
                    Rect::decode(&charge[4..20])
                } else {
                    None
                };
                let degat = match degat {
                    Some(v) => proto::rogne_degat(
                        v,
                        self.surface.largeur as u32,
                        self.surface.hauteur as u32,
                    ),
                    None => return false,
                };
                if degat.vide() {
                    return false;
                }

                self.degat = self.degat.union(&degat);
                self.marque_protocole_actif();
                if self.derniere_trame == 0 {
                    // La duree de demarrage, chiffree, dans le journal. « Lent
                    // a demarrer » est une phrase ; ceci est une mesure, et
                    // c'est elle qu'une optimisation future devra deplacer.
                    let demarrage = crate::kernel::timer::monotonic_ms()
                        .saturating_sub(self.naissance_ms);
                    crate::kernel::dmesg::log_fmt(format_args!(
                        "gui: premiere trame du client pid={} ({}x{}) apres {} ms",
                        self.pid, degat.largeur, degat.hauteur, demarrage
                    ));
                }
                if self.trames == 0 {
                    crate::kernel::perf::first_paint();
                }

                crate::kernel::perf::browser_frame(
                    self.pid,
                    (degat.largeur as u64).saturating_mul(degat.hauteur as u64),
                );

                self.etat = Etat::Actif;
                self.trames += 1;
                self.derniere_trame = crate::kernel::timer::ticks();
                // La jauge lit l'horloge MONOTONE, pas le tick PIT. Sous TCG,
                // IRQ0 est retardee precisement quand un vCPU sature le
                // traducteur -- c'est-a-dire quand la page charge le plus mal.
                // Un chronometre sur le tick sous-estimerait donc la lenteur au
                // moment exact ou l'utilisateur la constate.
                self.jauge.note_trame(crate::kernel::timer::monotonic_ms());
                true
            }
            Genre::Close => {
                self.fermeture_demandee = true;
                false
            }
            _ => false,
        }
    }

    /// Fait avancer le chronometre de chargement.
    ///
    /// Rend vrai si la jauge doit etre redessinee. La FIN d'une rafale de
    /// trames est un non-evenement : personne ne l'annonce, il faut donc venir
    /// la constater.
    pub fn tic_jauge(&mut self, maintenant_ms: u64) -> bool {
        let visible_avant = self.jauge.visible();
        self.jauge.tic(maintenant_ms);
        // Un chronometre qui tourne change a chaque trame ; une jauge qui
        // s'allume ou s'eteint change une fois.
        self.jauge.en_cours() || self.jauge.visible() != visible_avant
    }

    const PATIENCE_MS: u64 = 6000;

    pub fn verifie_silence(&mut self) -> bool {
        if self.verdict.protocole_actif()
            || self.verdict.recompose_a_l_aveugle()
            || self.etat == Etat::Termine
        {
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
        self.verdict.declare_muet();
        self.jauge.abandonne_demarrage();
        self.etat = Etat::Actif;
        self.abime_tout();
        true
    }

    pub fn etat_journal(&self, periode_ms: u64) -> String {
        let par_seconde = if periode_ms > 0 {
            self.trames * 1000 / periode_ms.max(1)
        } else {
            0
        };
        let silence = crate::kernel::timer::ticks().saturating_sub(self.derniere_trame);
        let silence_ms = if self.derniere_trame == 0 { 0 } else { silence };

        crate::kernel::perf::browser_report(self.pid, silence_ms);

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
            silence_ms,
            self.octets_recus,
            self.evenements_envoyes,
            self.evenements_perdus,
        )
    }

    pub fn remet_compteurs(&mut self) {
        self.trames = 0;
        self.octets_recus = 0;
        self.evenements_envoyes = 0;
        self.evenements_perdus = 0;
    }

    pub fn prend_degat(&mut self) -> Rect {
        core::mem::replace(&mut self.degat, Rect::default())
    }

    pub fn abime_tout(&mut self) {
        self.degat = Rect::neuf(
            0,
            0,
            self.surface.largeur as u32,
            self.surface.hauteur as u32,
        );
    }

    pub fn vivant(&mut self) -> bool {
        if self.etat == Etat::Termine {
            return false;
        }
        if let Some(code) = task::code_de_sortie(self.pid) {
            crate::kernel::perf::browser_process_exit(self.pid, code);
            crate::kernel::dmesg::log_fmt(format_args!(
                "gui: client pid={} termine (code {})",
                self.pid, code
            ));
            self.etat = Etat::Termine;
            return false;
        }
        true
    }

    pub fn termine(&mut self) {
        let arbre = task::arbre_de(self.pid);
        for pid in &arbre {
            task::tue_processus(*pid, 0);
        }
        self.etat = Etat::Termine;
        task::nettoie_zombies();
        for pid in arbre {
            task::collect_child(pid);
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.termine();
    }
}
