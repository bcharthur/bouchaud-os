//! La fenetre Services : l'arbre de TOUTE la pile, lu dans le registre.
//!
//! # Ce que cette fenetre montrait, et ce que cela coutait
//!
//! La photo physique du 17 septembre montre six lignes codees en dur :
//!
//! ```text
//! BouchaudBrowserHost   En cours PID [...]
//! WebContent            En cours PID [...]
//! ...
//! Reseau : pret
//! ```
//!
//! Devant une page qui ne charge pas, cette fenetre ne disait rien de la carte
//! reseau, d'ARP, de DHCP, de DNS, de TCP, de TLS, ni de l'ordonnanceur. Il
//! fallait deduire a la main, en recollant des compteurs qui vivent a quatre
//! endroits differents et ne se datent pas entre eux.
//!
//! Le registre central existait deja. Le defaut etait ici : la vue ne s'en
//! servait pas.
//!
//! # Ce que cette fenetre est maintenant
//!
//! Elle ne connait plus aucun nom de service. Elle demande un instantane au
//! registre, demande au modele visible quelles lignes afficher, et dessine.
//! Les deux boutons Ladybird restent : ils font agir, pas observer.
//!
//! ```text
//!   registre --> instantane --> modele visible --> peinture
//! ```
//!
//! Le verrou du registre n'est JAMAIS tenu pendant la peinture : le rendu
//! prend des millisecondes et bloquerait toute publication -- la carte
//! reseau, le navigateur, l'ordonnanceur -- au rythme de l'affichage.

use crate::gui::{framebuffer as fb, services};
use crate::kernel::services::registre::{Entree, Genre, Kpi};
use crate::kernel::services::vue::{self, Ligne, Replies};
use alloc::format;
use alloc::string::String;

/// Hauteur de la barre d'action, boutons Ladybird compris.
const BARRE_H: usize = 54;
/// Hauteur d'une ligne de l'arbre.
const LIGNE_H: usize = 18;
/// Hauteur de l'en-tete de colonnes.
const ENTETE_H: usize = 20;

/// Colonnes, en pixels depuis le bord gauche du corps.
///
/// Elles tiennent dans la largeur de la fenetre, PID compris. La premiere
/// version poussait « Latence » et « Err » au-dela du bord droit : une colonne
/// qu'on ne voit pas ne mesure rien.
const COL_SERVICE: usize = 8;
const COL_PID: usize = 232;
const COL_ETAT: usize = 290;
const COL_CPU: usize = 366;
const COL_RAM: usize = 428;
const COL_DISQUE: usize = 500;
const COL_RESEAU: usize = 578;
const COL_LATENCE: usize = 692;
const COL_ERREURS: usize = 772;
/// Place reservee en pied pour le verdict.
const PIED_H: usize = 22;

/// L'etat de la vue, garde entre deux peintures.
///
/// Un verrou masquant les interruptions : la peinture s'execute sur le fil du
/// compositeur, et le clic arrive par le chemin d'entree.
static ETAT: crate::kernel::sync::SpinLockIrq<EtatVue> =
    crate::kernel::sync::SpinLockIrq::new(EtatVue::neuf());

struct EtatVue {
    replies: Replies,
    defilement: usize,
    /// Nombre de lignes visibles a la derniere peinture, pour borner le
    /// defilement sans avoir a reconstruire le modele au moment du clic.
    lignes_dernier_rendu: usize,
    /// Lignes tenant dans le corps a la derniere peinture.
    fenetre_dernier_rendu: usize,
    /// Repliage initial pose une seule fois.
    initialise: bool,
    /// La ligne dont le pied montre le detail, s'il y en a une.
    ///
    /// Un identifiant et non un rang : le rang designe une autre ligne des
    /// qu'un groupe s'ouvre au-dessus, et le detail suivrait le voisin.
    selection: crate::kernel::services::registre::Id,
}

impl EtatVue {
    const fn neuf() -> Self {
        Self {
            replies: Replies::neuf(),
            defilement: 0,
            lignes_dernier_rendu: 0,
            fenetre_dernier_rendu: 0,
            initialise: false,
            selection: crate::kernel::services::registre::Id::vide(),
        }
    }

    /// Le premier affichage : Reseau et Navigateur ouverts, groupes replies.
    ///
    /// La regle vit dans le modele visible, pas ici : elle est testable a
    /// l'hote, et la fenetre n'a pas a connaitre la forme de la topologie.
    ///
    /// Le repliage n'est pose qu'une fois la topologie CONNUE. Appele sur un
    /// registre encore vide -- une premiere peinture peut preceder la
    /// declaration -- il replierait un arbre sans noeuds et se croirait fait.
    fn initialise(&mut self, entrees: &[Entree], budget: usize) {
        if self.initialise || entrees.is_empty() || budget == 0 {
            return;
        }
        self.initialise = true;
        vue::repli_par_defaut(entrees, &mut self.replies, budget);
    }
}

#[inline]
fn texte(x: usize, y: usize, valeur: &str, couleur: u32, gras: bool) {
    fb::draw_text_prop(x, y, valeur, couleur, 13.0, gras);
}

/// Un nombre d'octets, court et lisible.
fn octets(valeur: u64) -> String {
    if valeur >= 1024 * 1024 * 1024 {
        format!("{} Gio", valeur / (1024 * 1024 * 1024))
    } else if valeur >= 1024 * 1024 {
        format!("{} Mio", valeur / (1024 * 1024))
    } else if valeur >= 1024 {
        format!("{} Kio", valeur / 1024)
    } else {
        format!("{} o", valeur)
    }
}

/// Une duree, depuis des microsecondes.
fn duree(us: u64) -> String {
    if us >= 1_000_000 {
        format!("{}.{} s", us / 1_000_000, (us % 1_000_000) / 100_000)
    } else if us >= 1_000 {
        format!("{} ms", us / 1_000)
    } else {
        format!("{} us", us)
    }
}

/// Ce qui s'affiche quand personne ne mesure.
///
/// « Une valeur inconnue est N/A, et pas zero. » Zero est une mesure ;
/// l'absence de mesure n'en est pas une, et les confondre fait chercher une
/// panne dans un composant qui n'a simplement jamais ete interroge.
const INCONNU: &str = "N/A";

fn cpu_affiche(kpi: &Kpi) -> String {
    match kpi.cpu_pour_mille {
        Some(pm) => format!("{}.{}%", pm / 10, pm % 10),
        None => String::from(INCONNU),
    }
}

fn ram_affiche(kpi: &Kpi) -> String {
    match kpi.rss_octets {
        Some(o) => octets(o),
        None => String::from(INCONNU),
    }
}

fn disque_affiche(kpi: &Kpi) -> String {
    match (kpi.disque_lu, kpi.disque_ecrit) {
        (None, None) => String::from(INCONNU),
        (lu, ecrit) => format!(
            "{}/{}",
            lu.map(octets).unwrap_or_else(|| String::from("-")),
            ecrit.map(octets).unwrap_or_else(|| String::from("-")),
        ),
    }
}

fn reseau_affiche(kpi: &Kpi) -> String {
    match (kpi.rx_octets, kpi.tx_octets) {
        (None, None) => String::from(INCONNU),
        (rx, tx) => format!(
            "{}/{}",
            rx.map(octets).unwrap_or_else(|| String::from("-")),
            tx.map(octets).unwrap_or_else(|| String::from("-")),
        ),
    }
}

fn latence_affiche(kpi: &Kpi) -> String {
    match kpi.latence_us {
        Some(us) => duree(us),
        None => String::from(INCONNU),
    }
}

pub(crate) fn click(x: i32, y: i32) {
    if (12..42).contains(&y) {
        if (12..182).contains(&x) {
            services::demande(services::DEMARRER);
        }
        if (198..368).contains(&x) {
            services::demande(services::ARRETER);
        }
        return;
    }
    if y < 0 || x < 0 {
        return;
    }
    let mut etat = ETAT.lock();
    // Les deux fleches de defilement, a droite de l'en-tete.
    if (BARRE_H as i32..(BARRE_H + ENTETE_H) as i32).contains(&y) {
        let visible = etat.fenetre_dernier_rendu.max(1);
        if x >= COL_ERREURS as i32 + 40 {
            let maximum = etat.lignes_dernier_rendu.saturating_sub(visible);
            etat.defilement = (etat.defilement + visible / 2).min(maximum);
        } else if x >= COL_ERREURS as i32 + 16 {
            etat.defilement = etat.defilement.saturating_sub(visible / 2);
        }
        return;
    }
    let corps = (BARRE_H + ENTETE_H) as i32;
    if y < corps {
        return;
    }
    let rang = ((y - corps) as usize) / LIGNE_H + etat.defilement;

    // Reconstruire le modele pour savoir QUELLE ligne a ete cliquee : la vue
    // ne retient pas les identifiants entre deux peintures, et les retenir
    // ferait vivre une seconde copie de l'arbre.
    //
    // Le tampon vient de l'atelier commun, pas de la pile : voir
    // `kernel::services::Atelier`.
    let cible = crate::kernel::services::avec_atelier(|atelier| {
        let n = atelier.connues;
        let atelier = &mut *atelier;
        let budget = etat.fenetre_dernier_rendu;
        etat.initialise(&atelier.entrees[..n], budget);
        let visibles = vue::lignes(&atelier.entrees[..n], &etat.replies, &mut atelier.lignes);
        if rang >= visibles {
            return None;
        }
        let ligne = atelier.lignes[rang];
        Some((ligne.entree.id, ligne.a_des_enfants))
    });
    match cible {
        // Un groupe s'ouvre ou se ferme.
        Some((id, true)) => {
            etat.replies.bascule(id.texte());
        }
        // Une feuille montre son detail -- et le cache si on la reclique.
        Some((id, false)) => {
            if etat.selection.egale(id.texte()) {
                etat.selection = crate::kernel::services::registre::Id::vide();
            } else {
                etat.selection = id;
            }
        }
        None => {}
    }
}

/// Peint les lignes visibles de l'arbre.
///
/// Sortie de `draw` pour une raison de PILE : le tampon des lignes vit dans
/// l'atelier statique, et cette fonction le recoit en tranche au lieu d'en
/// poser une copie.
#[allow(clippy::too_many_arguments)]
fn peins_les_lignes(
    bx: usize,
    corps_y: usize,
    bw: usize,
    corps_h: usize,
    debut: usize,
    visibles: usize,
    lignes: &[Ligne],
) {
    let mut y = corps_y;
    let mut rang = debut;
    while rang < visibles && y + LIGNE_H <= corps_y + corps_h {
        let ligne = lignes[rang];
        let e = &ligne.entree;
        if rang % 2 == 1 {
            fb::fill_rect_rgb(bx, y, bw, LIGNE_H, 0x161e2e);
        }

        // Le chevron, puis le libelle indente.
        let indent = COL_SERVICE + ligne.profondeur as usize * 14;
        if ligne.a_des_enfants {
            texte(
                bx + indent,
                y + 2,
                if ligne.deploye { "v" } else { ">" },
                0x67d5e8,
                true,
            );
        }
        let gras = ligne.profondeur == 0;
        texte(
            bx + indent + 12,
            y + 2,
            crate::gui::window::clip(ligne.libelle(), 30),
            if gras { 0xeff3f8 } else { 0xc7d0dd },
            gras,
        );

        // UN GROUPE REPORTE LE PIRE DE CE QU'IL PORTE.
        //
        // Il ne mesure rien lui-meme -- ses colonnes chiffrees restent vides.
        // Mais taire son etat obligeait a ouvrir les sept sous-arbres du
        // reseau pour apprendre lequel allait mal, ce qui est exactement le
        // travail que cette fenetre doit epargner.
        texte(
            bx + COL_ETAT,
            y + 2,
            vue::etat_affiche(ligne.etat_effectif),
            vue::couleur_etat(ligne.etat_effectif),
            matches!(e.genre, Genre::Groupe),
        );
        if !matches!(e.genre, Genre::Groupe) {
            texte(bx + COL_CPU, y + 2, &cpu_affiche(&e.kpi), 0xc7d0dd, false);
            texte(bx + COL_RAM, y + 2, &ram_affiche(&e.kpi), 0xc7d0dd, false);
            texte(bx + COL_DISQUE, y + 2, &disque_affiche(&e.kpi), 0xc7d0dd, false);
            texte(bx + COL_RESEAU, y + 2, &reseau_affiche(&e.kpi), 0xc7d0dd, false);
            texte(bx + COL_LATENCE, y + 2, &latence_affiche(&e.kpi), 0xc7d0dd, false);
            let erreurs = if e.erreurs == 0 {
                String::from("0")
            } else {
                format!("{}", e.erreurs)
            };
            texte(
                bx + COL_ERREURS,
                y + 2,
                &erreurs,
                if e.erreurs == 0 { 0x6b7280 } else { 0xd96b6b },
                e.erreurs != 0,
            );
            // Le PID, pour un processus, et POUR LUI SEUL : un faux PID sur un
            // protocole ferait chercher un fil qui n'existe pas.
            if vue::porte_un_pid(e.genre) {
                match e.kpi.pid {
                    Some(pid) => texte(bx + COL_PID, y + 2, &format!("{}", pid), 0x8a94a6, false),
                    None => texte(bx + COL_PID, y + 2, INCONNU, 0x6b7280, false),
                }
            }
        }
        y += LIGNE_H;
        rang += 1;
    }
}

pub(crate) fn draw(bx: usize, by: usize, bw: usize, bh: usize) {
    fb::fill_rect_rgb(bx, by, bw, bh, 0x111827);

    // La barre d'action : elle fait AGIR, pas observer.
    for (x, label, color) in [
        (12, "Demarrer Ladybird", 0x2563eb),
        (198, "Arreter Ladybird", 0x374151),
    ] {
        fb::fill_rect_rgb(bx + x, by + 12, 170, 30, color);
        texte(bx + x + 10, by + 19, label, 0xeff3f8, true);
    }
    let racine = services::racine();
    let session = if services::en_echec() {
        "Ladybird : echec du lancement"
    } else if racine == 0 {
        "Ladybird : session arretee"
    } else {
        "Ladybird : session geree par le bureau"
    };
    texte(bx + 380, by + 19, session, 0x8a94a6, false);

    let mut etat = ETAT.lock();
    let corps_y = by + BARRE_H + ENTETE_H;
    // Le pied est RESERVE : sans cela la derniere ligne et le verdict se
    // superposaient, et aucun des deux ne se lisait.
    let corps_h = bh.saturating_sub(BARRE_H + ENTETE_H + PIED_H);
    let fenetre = corps_h / LIGNE_H;

    // L'INSTANTANE ET LA PEINTURE DES LIGNES, dans l'atelier commun.
    //
    // `avec_atelier` rend le verrou du REGISTRE avant d'appeler ce bloc : la
    // peinture ne bloque aucune publication. Le tampon, lui, est statique --
    // le poser sur la pile demandait quatre-vingt-sept kilo-octets.
    let visibles = crate::kernel::services::avec_atelier(|atelier| {
        let n = atelier.connues;
        let atelier = &mut *atelier;
        etat.initialise(&atelier.entrees[..n], fenetre);
        let visibles = vue::lignes(&atelier.entrees[..n], &etat.replies, &mut atelier.lignes);
        let debut = if etat.defilement + fenetre > visibles {
            visibles.saturating_sub(fenetre)
        } else {
            etat.defilement
        };
        peins_les_lignes(bx, corps_y, bw, corps_h, debut, visibles, &atelier.lignes);
        visibles
    });
    etat.lignes_dernier_rendu = visibles;
    etat.fenetre_dernier_rendu = fenetre;
    if etat.defilement + fenetre > visibles {
        etat.defilement = visibles.saturating_sub(fenetre);
    }

    // L'en-tete de colonnes.
    fb::fill_rect_rgb(bx, by + BARRE_H, bw, ENTETE_H, 0x1b2436);
    for (x, titre) in [
        (COL_SERVICE, "Service"),
        (COL_PID, "PID"),
        (COL_ETAT, "Etat"),
        (COL_CPU, "CPU"),
        (COL_RAM, "RAM"),
        (COL_DISQUE, "Disque"),
        (COL_RESEAU, "Reseau RX/TX"),
        (COL_LATENCE, "Latence"),
        (COL_ERREURS, "Err"),
    ] {
        if bx + x + 40 > bx + bw {
            break;
        }
        texte(bx + x, by + BARRE_H + 4, titre, 0x9da8b8, true);
    }
    if visibles > fenetre {
        texte(bx + COL_ERREURS + 16, by + BARRE_H + 4, "^", 0x67d5e8, true);
        texte(bx + COL_ERREURS + 40, by + BARRE_H + 4, "v", 0x67d5e8, true);
    }

    // LE PIED : le detail du service choisi, sinon le verdict.
    //
    // Cliquer une feuille demande « celui-la, raconte ». Sans cela, la
    // fenetre montre des colonnes et jamais l'histoire d'une ligne : combien
    // d'erreurs, combien de reprises, et la derniere raison connue.
    if !etat.selection.est_vide() {
        let choisi = etat.selection;
        let detail = crate::kernel::services::avec_atelier(|atelier| {
            atelier.entrees[..atelier.connues]
                .iter()
                .find(|e| e.id.egale(choisi.texte()))
                .copied()
        });
        if let Some(e) = detail {
            let ligne = format!(
                "{} [{}] {} | err {} | reprises {} | redemarrages {} | {}",
                e.id.texte(),
                e.genre.nom(),
                vue::etat_affiche(e.etat),
                e.erreurs,
                e.reprises,
                e.redemarrages,
                if e.raison.est_vide() { "aucune raison enregistree" } else { e.raison.texte() },
            );
            texte(
                bx + 8,
                corps_y + corps_h + 4,
                crate::gui::window::clip(&ligne, 110),
                vue::couleur_etat(e.etat),
                false,
            );
            return;
        }
    }
    // Le verdict, en pied : ce qui va mal, ou rien.
    if let Some((id, etat_pire, raison, _)) = crate::kernel::services::pire() {
        let ligne = format!(
            "{} : {} ({})",
            id.texte(),
            vue::etat_affiche(etat_pire),
            if raison.est_vide() { "-" } else { raison.texte() },
        );
        texte(bx + 8, corps_y + corps_h + 4, &ligne, vue::couleur_etat(etat_pire), true);
    } else {
        texte(
            bx + 8,
            corps_y + corps_h + 4,
            "Aucun service degrade.",
            0x6b7280,
            false,
        );
    }
}
