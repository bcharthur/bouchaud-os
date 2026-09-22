//! La fenetre Services : le tableau de bord de TOUTE la pile.
//!
//! # Ce que cette fenetre montrait, et ce que cela coutait
//!
//! La photo du 17 septembre montrait six lignes codees en dur -- les
//! processus de Ladybird -- et rien d'autre. Devant une page qui ne charge
//! pas, elle ne disait rien de la carte reseau, d'ARP, de DHCP, de DNS, de
//! TCP, de TLS ni de l'ordonnanceur.
//!
//! La passe suivante a corrige cela : l'arbre du registre s'affiche. Mais la
//! photo du lendemain a montre le defaut inverse -- un arbre juste, et une
//! grille de « N/A » a perte de vue, deux boutons Ladybird en tete d'une
//! fenetre censee surveiller le systeme entier, et un pied qui tenait sur une
//! ligne.
//!
//! # Ce que cette fenetre est maintenant
//!
//! Un moniteur. Elle ne connait aucun nom de service : elle demande un
//! instantane au registre, demande au modele visible quelles lignes afficher,
//! et dessine.
//!
//! ```text
//!   registre --> instantane --> modele visible --> peinture
//! ```
//!
//! Quatre regles la gouvernent :
//!
//! 1. UN NOEUD CHIFFRE. Une branche porte la somme de ses feuilles : c'est la
//!    reponse a « qu'est-ce qui consomme ».
//! 2. UNE CELLULE VIDE SE TAIT. Un tiret, pas « N/A » : une colonne de « N/A »
//!    ne se lit plus, et le jour ou une vraie valeur y apparait, l'oeil la
//!    saute aussi.
//! 3. UN ETAT DIT POURQUOI. La raison s'affiche a cote de l'etat.
//! 4. LES ACTIONS SONT CONTEXTUELLES. Les boutons Ladybird ne sont pas
//!    l'en-tete d'un moniteur systeme ; ils appartiennent a la ligne
//!    selectionnee quand elle les concerne.
//!
//! Le verrou du registre n'est JAMAIS tenu pendant la peinture : le rendu
//! prend des millisecondes et bloquerait toute publication -- la carte
//! reseau, le navigateur, l'ordonnanceur -- au rythme de l'affichage.

use crate::gui::{framebuffer as fb, services};
use crate::kernel::services::registre::{self, Entree, Etat, Genre, Kpi};
use crate::kernel::services::vue::{self, Ligne, Replies, TIRET};
use alloc::format;
use alloc::string::String;

/// Hauteur de la barre de resume et d'action.
const BARRE_H: usize = 34;
/// Hauteur d'une ligne de l'arbre.
const LIGNE_H: usize = 18;
/// Hauteur de l'en-tete de colonnes.
const ENTETE_H: usize = 20;
/// Largeur a partir de laquelle le detail tient a DROITE de l'arbre.
///
/// En dessous, il passe en bas : un panneau de trois cents pixels ampute un
/// arbre qui en fait six cents, et on perd ce qu'on est venu lire.
const LARGEUR_DETAIL_A_DROITE: usize = 1100;
/// Largeur du panneau de detail quand il est a droite.
const DETAIL_W: usize = 340;
/// Hauteur du panneau de detail quand il est en bas.
const DETAIL_H: usize = 170;

const FOND: u32 = 0x111827;
const FOND_ENTETE: u32 = 0x1b2436;
const FOND_ALTERNE: u32 = 0x161e2e;
const FOND_SELECTION: u32 = 0x24405e;
const FOND_NOEUD: u32 = 0x18212f;
const TRAIT: u32 = 0x243044;
const ENCRE: u32 = 0xeff3f8;
const ENCRE_DOUCE: u32 = 0xc7d0dd;
const ENCRE_FAIBLE: u32 = 0x8a94a6;
const ENCRE_ETEINTE: u32 = 0x4b5563;
const CHEVRON: u32 = 0x67d5e8;

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
    /// La ligne dont le panneau montre le detail, s'il y en a une.
    ///
    /// Un identifiant et non un rang : le rang designe une autre ligne des
    /// qu'un groupe s'ouvre au-dessus, et le detail suivrait le voisin.
    selection: registre::Id,
    /// Geometrie de la derniere peinture, pour que le clic vise juste.
    corps_h_dernier_rendu: usize,
    action_derniere: Option<(usize, usize, bool)>,
}

impl EtatVue {
    const fn neuf() -> Self {
        Self {
            replies: Replies::neuf(),
            defilement: 0,
            lignes_dernier_rendu: 0,
            fenetre_dernier_rendu: 0,
            initialise: false,
            selection: registre::Id::vide(),
            corps_h_dernier_rendu: 0,
            action_derniere: None,
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

// ---------------------------------------------------------------------------
// Les colonnes, calculees sur la largeur reelle
// ---------------------------------------------------------------------------

/// Les abscisses des colonnes, en pixels depuis le bord gauche de l'arbre.
///
/// # Pourquoi elles ne sont pas des constantes
///
/// Elles l'etaient. Sur la TRIGKEY, ou la fenetre s'ouvre en plein ecran, la
/// derniere colonne tombait vers le neuvieme de la largeur et laissait mille
/// pixels de vide a droite : une grille tassee dans un coin, illisible, sur
/// un ecran qui avait toute la place. Une colonne se pose donc en proportion
/// de ce qu'on lui donne, avec un minimum pour les fenetres etroites.
struct Colonnes {
    service: usize,
    pid: usize,
    etat: usize,
    raison: usize,
    cpu: usize,
    ram: usize,
    disque: usize,
    reseau: usize,
    latence: usize,
    erreurs: usize,
}

fn colonnes(largeur: usize) -> Colonnes {
    // Les largeurs minimales de chaque colonne, dans l'ordre.
    const MINIMA: [usize; 9] = [170, 44, 92, 130, 58, 70, 84, 120, 70];
    // UNE COLONNE QU'ON NE VOIT PAS NE MESURE RIEN.
    //
    // La premiere version posait neuf colonnes quelle que soit la largeur :
    // a neuf cents pixels, « Err » tombait au-dela du bord droit et les
    // valeurs se dessinaient dans le vide. La raison est la plus longue et la
    // moins indispensable des colonnes -- elle reste lisible dans le panneau
    // de detail --, c'est donc elle qui cede en premier.
    let sans_raison: usize = MINIMA.iter().sum::<usize>() - MINIMA[3] + 24;
    // La marge de quatre-vingt-dix pixels n'est pas decorative : sans elle,
    // a neuf cents pixels, « Err » se dessinait dans les seize derniers et son
    // titre disparaissait de l'en-tete.
    let montre_raison = largeur >= MINIMA.iter().sum::<usize>() + 90;
    let total_minimal = if montre_raison {
        MINIMA.iter().sum::<usize>() + 24
    } else {
        sans_raison
    };
    // Le surplus va au nom du service et a la raison : ce sont les deux
    // seules colonnes dont le contenu s'allonge vraiment.
    let surplus = largeur.saturating_sub(total_minimal);
    let pour_service = if montre_raison { surplus / 3 } else { surplus };
    let pour_raison = surplus - pour_service;

    let mut x = 8usize;
    let service = x;
    x += MINIMA[0] + pour_service;
    let pid = x;
    x += MINIMA[1];
    let etat = x;
    x += MINIMA[2];
    let raison = if montre_raison { x } else { 0 };
    if montre_raison {
        x += MINIMA[3] + pour_raison;
    }
    let cpu = x;
    x += MINIMA[4];
    let ram = x;
    x += MINIMA[5];
    let disque = x;
    x += MINIMA[6];
    let reseau = x;
    x += MINIMA[7];
    let latence = x;
    x += MINIMA[8];
    let erreurs = x;
    Colonnes { service, pid, etat, raison, cpu, ram, disque, reseau, latence, erreurs }
}

// ---------------------------------------------------------------------------
// Mise en forme
// ---------------------------------------------------------------------------

/// Le dernier segment d'un identifiant : « net.config » -> « config ».
fn court(id: &str) -> &str {
    match id.rfind('.') {
        Some(point) => &id[point + 1..],
        None => id,
    }
}

#[inline]
fn texte(x: usize, y: usize, valeur: &str, couleur: u32, gras: bool) {
    fb::draw_text_prop(x, y, valeur, couleur, 13.0, gras);
}

/// Un nombre d'octets, court et lisible.
fn octets(valeur: u64) -> String {
    if valeur >= 1024 * 1024 * 1024 {
        format!("{}.{} Gio", valeur / (1024 * 1024 * 1024),
                (valeur % (1024 * 1024 * 1024)) / (107 * 1024 * 1024))
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

/// Une cellule chiffree, ou un tiret.
///
/// LE TIRET N'EST PAS UN ZERO. Zero est une mesure ; l'absence de mesure n'en
/// est pas une, et les confondre fait chercher une panne dans un composant
/// que personne n'a jamais interroge. Mais le tiret n'est pas non plus un
/// « N/A » repete cent fois : il s'ecrit en gris eteint et se laisse oublier.
fn cellule(valeur: Option<String>) -> (String, u32) {
    match valeur {
        Some(v) => (v, ENCRE_DOUCE),
        None => (String::from(TIRET), ENCRE_ETEINTE),
    }
}

fn cpu_affiche(kpi: &Kpi) -> Option<String> {
    kpi.cpu_pour_mille.map(|pm| format!("{}.{}%", pm / 10, pm % 10))
}

fn ram_affiche(kpi: &Kpi) -> Option<String> {
    kpi.rss_octets.map(octets)
}

fn paire(a: Option<u64>, b: Option<u64>) -> Option<String> {
    match (a, b) {
        (None, None) => None,
        (x, y) => Some(format!(
            "{}/{}",
            x.map(octets).unwrap_or_else(|| String::from(TIRET)),
            y.map(octets).unwrap_or_else(|| String::from(TIRET)),
        )),
    }
}

fn latence_affiche(kpi: &Kpi) -> Option<String> {
    kpi.latence_us.or(kpi.latence_max_us).map(duree)
}

// ---------------------------------------------------------------------------
// L'action contextuelle
// ---------------------------------------------------------------------------

/// L'action que la ligne selectionnee propose, s'il y en a une.
///
/// # Pourquoi ce n'est plus un en-tete fixe
///
/// « Demarrer Ladybird / Arreter Ladybird » occupait la premiere ligne d'une
/// fenetre qui surveille l'ordonnanceur, la memoire, l'USB et la pile
/// reseau. Un moniteur systeme dont l'en-tete pilote une application, c'est
/// un panneau d'application deguise.
///
/// L'action existe toujours -- elle est utile -- mais elle appartient a ce
/// qu'on a selectionne. Rien de choisi, ou une ligne qui ne concerne pas le
/// navigateur : pas de bouton.
fn action_de(id: &str) -> Option<(&'static str, bool)> {
    if !id.starts_with("browser") {
        return None;
    }
    if services::racine() == 0 {
        Some(("Demarrer Ladybird", true))
    } else {
        Some(("Arreter Ladybird", false))
    }
}

// ---------------------------------------------------------------------------
// Le clic
// ---------------------------------------------------------------------------

pub(crate) fn click(x: i32, y: i32) {
    if y < 0 || x < 0 {
        return;
    }
    let mut etat = ETAT.lock();

    // La barre d'action : un seul bouton, contextuel, et seulement s'il a ete
    // dessine a la derniere peinture.
    if (y as usize) < BARRE_H {
        if let Some((gauche, droite, demarrer)) = etat.action_derniere {
            if (gauche..droite).contains(&(x as usize)) {
                services::demande(if demarrer { services::DEMARRER } else { services::ARRETER });
            }
        }
        return;
    }

    // Les deux fleches de defilement, a droite de l'en-tete.
    if ((BARRE_H)..(BARRE_H + ENTETE_H)).contains(&(y as usize)) {
        let visible = etat.fenetre_dernier_rendu.max(1);
        let maximum = etat.lignes_dernier_rendu.saturating_sub(visible);
        // Elles vivent a gauche de l'en-tete, contre le nom de la colonne :
        // a droite, sur un ecran large, elles finissaient hors de portee du
        // regard comme de la souris.
        if (110..134).contains(&(x as usize)) {
            etat.defilement = etat.defilement.saturating_sub(visible / 2);
        } else if (134..158).contains(&(x as usize)) {
            etat.defilement = (etat.defilement + visible / 2).min(maximum);
        }
        return;
    }

    let corps = (BARRE_H + ENTETE_H) as i32;
    let corps_h = etat.corps_h_dernier_rendu;
    if y < corps || (y - corps) as usize >= corps_h {
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
        Some((ligne.entree.id, ligne.a_des_enfants, ligne.profondeur))
    });

    let Some((id, a_des_enfants, profondeur)) = cible else { return };
    // TOUTE ligne se selectionne -- un noeud aussi : c'est sur les noeuds qu'on
    // lit une somme, et le detail doit pouvoir l'expliquer.
    etat.selection = id;
    // Sur un noeud, le clic dans la zone du chevron plie ; ailleurs il
    // selectionne seulement. Un arbre qui se referme des qu'on veut lire une
    // ligne est intenable.
    if a_des_enfants {
        let indent = 8 + profondeur as usize * 14;
        if (x as usize) < indent + 14 {
            etat.replies.bascule(id.texte());
        }
    }
}

// ---------------------------------------------------------------------------
// La peinture
// ---------------------------------------------------------------------------

pub(crate) fn draw(bx: usize, by: usize, bw: usize, bh: usize) {
    fb::fill_rect_rgb(bx, by, bw, bh, FOND);

    let mut etat = ETAT.lock();

    // Le detail va a droite quand il y a la place, en bas sinon.
    let detail_a_droite = bw >= LARGEUR_DETAIL_A_DROITE;
    let arbre_w = if detail_a_droite { bw - DETAIL_W } else { bw };
    let arbre_h = if detail_a_droite { bh } else { bh.saturating_sub(DETAIL_H) };
    let cols = colonnes(arbre_w);

    let corps_y = by + BARRE_H + ENTETE_H;
    let corps_h = arbre_h.saturating_sub(BARRE_H + ENTETE_H);
    let fenetre = corps_h / LIGNE_H;
    etat.corps_h_dernier_rendu = corps_h;

    // LE PANNEAU NE S'OUVRE PAS VIDE.
    //
    // « Choisir un service pour en voir le detail » occupait un tiers de la
    // fenetre et n'apprenait rien. A l'ouverture, on selectionne ce qui va le
    // plus mal -- c'est la ligne qu'on vient lire -- et a defaut la premiere
    // racine.
    if etat.selection.est_vide() {
        etat.selection = match crate::kernel::services::pire() {
            Some((id, _, _, _)) => id,
            None => registre::Id::depuis(vue::RACINES[0]),
        };
    }
    let selection = etat.selection;
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
        peins_les_lignes(
            bx, corps_y, arbre_w, corps_h, debut, visibles, &atelier.lignes, &cols, &selection,
        );
        visibles
    });
    etat.lignes_dernier_rendu = visibles;
    etat.fenetre_dernier_rendu = fenetre;
    if etat.defilement + fenetre > visibles {
        etat.defilement = visibles.saturating_sub(fenetre);
    }

    peins_la_barre(bx, by, arbre_w, &mut etat);
    peins_l_entete(bx, by, arbre_w, &cols, visibles > fenetre);

    if detail_a_droite {
        peins_le_detail(bx + arbre_w, by, DETAIL_W, bh, &selection, true);
    } else {
        peins_le_detail(bx, by + arbre_h, bw, DETAIL_H, &selection, false);
    }
}

/// La barre du haut : ce que la pile vaut en une ligne, et l'action du
/// moment.
fn peins_la_barre(bx: usize, by: usize, bw: usize, etat: &mut EtatVue) {
    fb::fill_rect_rgb(bx, by, bw, BARRE_H, FOND_ENTETE);
    fb::fill_rect_rgb(bx, by + BARRE_H - 1, bw, 1, TRAIT);

    let c = crate::kernel::services::compteurs();
    let resume = format!("{} services", c.enregistres);
    texte(bx + 8, by + 9, &resume, ENCRE, true);

    // LE VERDICT, EN TETE. C'est la question qu'on pose en ouvrant la
    // fenetre ; elle ne doit pas obliger a parcourir soixante lignes.
    let mut x = bx + 8 + 96;
    if let Some((id, pire, raison, _)) = crate::kernel::services::pire() {
        let verdict = format!(
            "{} : {}{}",
            id.texte(),
            vue::etat_affiche(pire),
            if raison.est_vide() {
                String::new()
            } else {
                format!(" ({})", raison.texte())
            },
        );
        texte(x, by + 9, &verdict, vue::couleur_etat(pire), true);
    } else {
        texte(x, by + 9, "rien de degrade", ENCRE_FAIBLE, false);
    }

    // L'action contextuelle, a droite, et seulement si la selection en a une.
    etat.action_derniere = None;
    if !etat.selection.est_vide() {
        if let Some((libelle, demarrer)) = action_de(etat.selection.texte()) {
            let largeur = 150usize;
            x = bx + bw.saturating_sub(largeur + 10);
            let fond = if demarrer { 0x2563eb } else { 0x334155 };
            fb::fill_rect_rgb(x, by + 5, largeur, BARRE_H - 11, fond);
            texte(x + 12, by + 9, libelle, ENCRE, true);
            etat.action_derniere = Some((x - bx, x - bx + largeur, demarrer));
        }
    }
}

fn peins_l_entete(bx: usize, by: usize, bw: usize, cols: &Colonnes, defilable: bool) {
    let y = by + BARRE_H;
    fb::fill_rect_rgb(bx, y, bw, ENTETE_H, FOND_ENTETE);
    fb::fill_rect_rgb(bx, y + ENTETE_H - 1, bw, 1, TRAIT);
    for (x, titre) in [
        (cols.service, "Service"),
        (cols.pid, "PID"),
        (cols.etat, "Etat"),
        (cols.raison, if cols.raison == 0 { "" } else { "Raison" }),
        (cols.cpu, "CPU"),
        (cols.ram, "RAM"),
        (cols.disque, "Disque"),
        (cols.reseau, "Reseau RX/TX"),
        (cols.latence, "Latence"),
        (cols.erreurs, "Err"),
    ] {
        if titre.is_empty() {
            continue;
        }
        if x + 40 > bw {
            break;
        }
        texte(bx + x, y + 4, titre, ENCRE_FAIBLE, true);
    }
    if defilable {
        texte(bx + 110, y + 4, "^", CHEVRON, true);
        texte(bx + 134, y + 4, "v", CHEVRON, true);
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
    cols: &Colonnes,
    selection: &registre::Id,
) {
    let mut y = corps_y;
    let mut rang = debut;
    while rang < visibles && y + LIGNE_H <= corps_y + corps_h {
        let ligne = lignes[rang];
        let e = &ligne.entree;
        let choisie = selection.egale(e.id.texte());

        // LE FOND DIT LE ROLE. Un noeud porte une bande un peu plus claire, la
        // selection une bande franche : sur soixante lignes, retrouver celle
        // qu'on a cliquee ne doit pas demander un effort.
        if choisie {
            fb::fill_rect_rgb(bx, y, bw, LIGNE_H, FOND_SELECTION);
        } else if ligne.est_un_noeud {
            fb::fill_rect_rgb(bx, y, bw, LIGNE_H, FOND_NOEUD);
        } else if rang % 2 == 1 {
            fb::fill_rect_rgb(bx, y, bw, LIGNE_H, FOND_ALTERNE);
        }

        // Le chevron, puis le libelle indente.
        let indent = cols.service + ligne.profondeur as usize * 14;
        if ligne.a_des_enfants {
            texte(
                bx + indent,
                y + 2,
                if ligne.deploye { "v" } else { ">" },
                CHEVRON,
                true,
            );
        }
        let gras = ligne.est_un_noeud;
        texte(
            bx + indent + 12,
            y + 2,
            crate::gui::window::clip(ligne.libelle(), 34),
            if gras { ENCRE } else { ENCRE_DOUCE },
            gras,
        );

        // L'etat, et la raison a cote. Un « Attente » sans raison oblige a
        // deviner ce qu'il attend.
        texte(
            bx + cols.etat,
            y + 2,
            vue::etat_affiche(ligne.etat_effectif),
            vue::couleur_etat(ligne.etat_effectif),
            true,
        );
        if cols.raison != 0 && !ligne.raison_effective.est_vide() {
            // Sur un groupe, la raison vient d'un descendant : on le nomme,
            // sinon elle envoie chercher au mauvais endroit.
            let mot = if ligne.est_un_noeud && !ligne.raison_source.est_vide() {
                format!(
                    "{} : {}",
                    court(ligne.raison_source.texte()),
                    ligne.raison_effective.texte()
                )
            } else {
                String::from(ligne.raison_effective.texte())
            };
            texte(
                bx + cols.raison,
                y + 2,
                crate::gui::window::clip(&mot, 34),
                ENCRE_FAIBLE,
                false,
            );
        }

        // Les chiffres. Sur un noeud ce sont des SOMMES -- c'est la reponse a
        // « quel sous-systeme consomme quoi ».
        let k = &ligne.kpi_effectif;
        for (x, valeur) in [
            (cols.cpu, cpu_affiche(k)),
            (cols.ram, ram_affiche(k)),
            (cols.disque, paire(k.disque_lu, k.disque_ecrit)),
            (cols.reseau, paire(k.rx_octets, k.tx_octets)),
            (cols.latence, latence_affiche(k)),
        ] {
            let (mot, couleur) = cellule(valeur);
            texte(bx + x, y + 2, &mot, if gras { ENCRE_DOUCE } else { couleur }, false);
        }

        let erreurs = ligne.erreurs_effectives;
        texte(
            bx + cols.erreurs,
            y + 2,
            &if erreurs == 0 { String::from(TIRET) } else { format!("{}", erreurs) },
            if erreurs == 0 { ENCRE_ETEINTE } else { 0xd96b6b },
            erreurs != 0,
        );

        // Le PID, pour un processus, et POUR LUI SEUL : un faux PID sur un
        // protocole ferait chercher un fil qui n'existe pas.
        if vue::porte_un_pid(e.genre) {
            let (mot, couleur) = cellule(k.pid.map(|p| format!("{}", p)));
            texte(bx + cols.pid, y + 2, &mot, couleur, false);
        }

        y += LIGNE_H;
        rang += 1;
    }
}

// ---------------------------------------------------------------------------
// Le panneau de detail
// ---------------------------------------------------------------------------

/// Le detail de la ligne selectionnee.
///
/// # Ce qu'il repond
///
/// Une grille de colonnes dit CE QUI se passe ; elle ne dit pas POURQUOI.
/// Ce panneau ajoute ce que la grille ne peut pas tenir : depuis quand l'etat
/// dure, ce qui l'explique, ce dont ce service depend, et les compteurs qui
/// n'ont pas de colonne.
///
/// La chaine de prerequis est remontee JUSQU'A LA PREMIERE CASE QUI NE VA PAS.
/// C'est la reponse a « a quelle etape ca bloque » : inutile de lire trente
/// lignes pour s'apercevoir que tout attend une adresse IP.
fn peins_le_detail(
    bx: usize,
    by: usize,
    bw: usize,
    bh: usize,
    selection: &registre::Id,
    a_droite: bool,
) {
    fb::fill_rect_rgb(bx, by, bw, bh, FOND_ENTETE);
    if a_droite {
        fb::fill_rect_rgb(bx, by, 1, bh, TRAIT);
    } else {
        fb::fill_rect_rgb(bx, by, bw, 1, TRAIT);
    }

    if selection.est_vide() {
        texte(
            bx + 12,
            by + 10,
            "Choisir un service pour en voir le detail.",
            ENCRE_ETEINTE,
            false,
        );
        return;
    }

    let choisi = *selection;
    // Tout ce que le panneau montre est lu dans le MEME instantane : deux
    // lectures separees afficheraient un etat et une raison qui ne se
    // rapportent pas au meme instant.
    let trouve = crate::kernel::services::avec_atelier(|atelier| {
        let n = atelier.connues;
        let entrees = &atelier.entrees[..n];
        let e = *entrees.iter().find(|e| e.id.egale(choisi.texte()))?;
        let noeud = vue::a_des_enfants(entrees, choisi.texte())
            && matches!(e.genre, Genre::Groupe);
        let etat_montre = if noeud { vue::etat_agrege(entrees, choisi.texte()) } else { e.etat };
        let kpi = if noeud { vue::kpi_agrege(entrees, choisi.texte()) } else { e.kpi };
        let erreurs = if noeud {
            vue::erreurs_agregees(entrees, choisi.texte())
        } else {
            e.erreurs
        };
        // La chaine de prerequis, jusqu'au premier maillon qui ne va pas.
        let mut chaine: Option<(registre::Id, Etat)> = None;
        let mut courant = choisi.texte();
        for _ in 0..8 {
            let Some(avant) = registre::prerequis(courant) else { break };
            let Some(entree) = entrees.iter().find(|e| e.id.egale(avant)) else { break };
            let etat_avant = if vue::a_des_enfants(entrees, avant) {
                vue::etat_agrege(entrees, avant)
            } else {
                entree.etat
            };
            chaine = Some((entree.id, etat_avant));
            if !matches!(etat_avant, Etat::Actif | Etat::Repos) {
                // Celui-la bloque : inutile de remonter plus haut.
                break;
            }
            courant = avant;
        }
        Some((e, etat_montre, kpi, erreurs, noeud, chaine))
    });

    let Some((e, etat_montre, kpi, erreurs, noeud, chaine)) = trouve else {
        texte(bx + 12, by + 10, "Service disparu du registre.", ENCRE_ETEINTE, false);
        return;
    };

    let mut y = by + 8;
    let saut = 17usize;
    // L'identite.
    texte(bx + 12, y, crate::gui::window::clip(e.id.texte(), 40), ENCRE, true);
    y += saut;
    texte(
        bx + 12,
        y,
        &format!("{}{}", e.genre.nom(), if noeud { " (agrege)" } else { "" }),
        ENCRE_ETEINTE,
        false,
    );
    y += saut + 2;

    // L'etat, sa raison, et depuis quand.
    texte(bx + 12, y, "Etat", ENCRE_FAIBLE, false);
    texte(bx + 96, y, vue::etat_affiche(etat_montre), vue::couleur_etat(etat_montre), true);
    y += saut;
    texte(bx + 12, y, "Raison", ENCRE_FAIBLE, false);
    texte(
        bx + 96,
        y,
        if e.raison.est_vide() { TIRET } else { crate::gui::window::clip(e.raison.texte(), 30) },
        if e.raison.est_vide() { ENCRE_ETEINTE } else { ENCRE_DOUCE },
        false,
    );
    y += saut;
    texte(bx + 12, y, "Depuis", ENCRE_FAIBLE, false);
    let depuis = if e.derniere_transition_ns == 0 {
        String::from(TIRET)
    } else {
        let maintenant = crate::kernel::timer::monotonic_ns();
        duree(maintenant.saturating_sub(e.derniere_transition_ns) / 1_000)
    };
    texte(bx + 96, y, &depuis, ENCRE_DOUCE, false);
    y += saut;

    // Ce dont il depend, et ou cela bloque.
    texte(bx + 12, y, "Depend de", ENCRE_FAIBLE, false);
    match chaine {
        Some((qui, etat_avant)) => {
            texte(
                bx + 96,
                y,
                &format!("{} ({})", crate::gui::window::clip(qui.texte(), 22),
                         vue::etat_affiche(etat_avant)),
                vue::couleur_etat(etat_avant),
                false,
            );
        }
        None => {
            texte(bx + 96, y, TIRET, ENCRE_ETEINTE, false);
        }
    }
    y += saut + 2;

    // Les compteurs, et les mesures sans colonne.
    // EN BAS, LE PANNEAU EST LARGE ET COURT.
    //
    // Une colonne unique s'arretait a « Erreurs » et laissait les trois
    // quarts de la largeur vides. Deux colonnes montrent tout ce qui rentre.
    let deux_colonnes = !a_droite && bw > 700;
    // En bas, la liste ne se met pas SOUS l'identite -- il ne resterait que
    // deux lignes -- mais A COTE : l'identite tient dans le premier tiers, la
    // liste occupe les deux autres sur toute la hauteur du panneau.
    let pas_de_colonne = if deux_colonnes { bw / 3 } else { 0 };
    let haut = if deux_colonnes { by + 8 } else { y };
    let mut rang = 0usize;
    for (nom, valeur) in [
        ("Erreurs", if erreurs == 0 { String::from("0") } else { format!("{}", erreurs) }),
        ("Reprises", format!("{}", e.reprises)),
        ("Relances", format!("{}", e.redemarrages)),
        (
            // BOUCHAUD_C24_PLUSIEURS_INSTANCES
            //
            // Un seul PID quand il y en a trois designe arbitrairement l'un
            // d'eux, et l'on va chercher la lenteur dans le mauvais processus.
            // Au-dela d'une instance, c'est leur NOMBRE qui se lit.
            "PID",
            match (kpi.pid, kpi.instances) {
                (_, Some(n)) if n > 1 => format!("{} processus", n),
                (Some(p), _) => format!("{}", p),
                _ => String::from(TIRET),
            },
        ),
        ("CPU", cpu_affiche(&kpi).unwrap_or_else(|| String::from(TIRET))),
        ("RSS", ram_affiche(&kpi).unwrap_or_else(|| String::from(TIRET))),
        (
            "VSS",
            kpi.vss_octets.map(octets).unwrap_or_else(|| String::from(TIRET)),
        ),
        (
            "Reseau",
            paire(kpi.rx_octets, kpi.tx_octets).unwrap_or_else(|| String::from(TIRET)),
        ),
        (
            "Disque",
            paire(kpi.disque_lu, kpi.disque_ecrit).unwrap_or_else(|| String::from(TIRET)),
        ),
        (
            // BOUCHAUD_C24_FAUTES_PAR_PROCESSUS
            //
            // Le nombre ET le temps : mille fautes qui coutent une
            // milliseconde au total ne sont pas un probleme, dix qui en
            // coutent quarante en sont un. Le nombre seul ne permet pas de
            // faire la difference, et c'est le journal physique qui l'a
            // montre -- il disait « bottleneck=memory-pagefault » sans jamais
            // dire combien de temps.
            "Fautes",
            match (kpi.fautes_nombre, kpi.fautes_total_us) {
                (Some(n), Some(us)) => format!("{} / {}", n, duree(us)),
                (Some(n), None) => format!("{}", n),
                // `TIRET` et non « 0 » : le livre des fautes est borne et il
                // chasse. Ne rien savoir d'un processus et savoir qu'il ne
                // faute pas sont deux choses differentes.
                _ => String::from(TIRET),
            },
        ),
        (
            "Pire faute",
            kpi.fautes_pire_us
                .map(duree)
                .unwrap_or_else(|| String::from(TIRET)),
        ),
        (
            "Latence",
            latence_affiche(&kpi).unwrap_or_else(|| String::from(TIRET)),
        ),
        (
            "Lat. max",
            kpi.latence_max_us.map(duree).unwrap_or_else(|| String::from(TIRET)),
        ),
        (
            "Operations",
            kpi.operations.map(|o| format!("{}", o)).unwrap_or_else(|| String::from(TIRET)),
        ),
    ] {
        let (x, ligne_y) = if deux_colonnes {
            let par_colonne = (by + bh).saturating_sub(haut) / saut;
            if par_colonne == 0 {
                break;
            }
            let colonne = rang / par_colonne;
            if colonne > 1 {
                break;
            }
            (bx + 12 + pas_de_colonne + colonne * pas_de_colonne, haut + (rang % par_colonne) * saut)
        } else {
            (bx + 12, y)
        };
        if ligne_y + saut > by + bh {
            break;
        }
        texte(x, ligne_y, nom, ENCRE_FAIBLE, false);
        let eteint = valeur == TIRET;
        texte(
            x + 104,
            ligne_y,
            &valeur,
            if eteint { ENCRE_ETEINTE } else { ENCRE_DOUCE },
            false,
        );
        rang += 1;
        if !deux_colonnes {
            y += saut;
        }
    }
}
