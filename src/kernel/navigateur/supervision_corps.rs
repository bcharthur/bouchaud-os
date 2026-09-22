/// Processus de navigateur suivis simultanement.
///
/// Borne fixe : un registre qui alloue se fait epuiser par un courtier qui
/// boucle, et c'est precisement le cas qu'on cherche a diagnostiquer.
pub const SUIVIS_MAX: usize = 64;

/// Relances tolerees dans la fenetre ci-dessous, par contexte.
pub const RELANCES_MAX: u32 = 3;

/// Fenetre de la boucle de plantage, en nanosecondes (trente secondes).
pub const FENETRE_RELANCE_NS: u64 = 30_000_000_000;

/// Le role d'un processus dans le navigateur.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// BrowserHost : il lance et supervise les autres.
    Courtier,
    /// WebContent : un moteur de rendu, un par contexte.
    Rendu,
    /// RequestServer : il POSSEDE le reseau.
    Reseau,
    /// ImageDecoder.
    Decodeur,
    /// WebWorker.
    Travailleur,
    /// Compositor : il POSSEDE la surface et le present.
    ///
    /// BOUCHAUD_C24_LE_COMPOSITOR_ETAIT_INVISIBLE
    ///
    /// Il manquait, et c'etait la pire absence des cinq. `security::profile`
    /// lui accordait deja le profil PRIVILEGIE de courtier -- le droit de
    /// lancer des processus -- alors qu'aucun role ne le supervisait : sa
    /// mort ne declenchait aucune politique, et Services ne montrait pas le
    /// processus qui fait le rendu.
    Composition,
}

impl Role {
    pub const fn nom(self) -> &'static str {
        match self {
            Self::Courtier => "courtier",
            Self::Rendu => "rendu",
            Self::Reseau => "reseau",
            Self::Decodeur => "decodeur",
            Self::Travailleur => "travailleur",
            Self::Composition => "composition",
        }
    }

    /// Le role deduit du chemin de l'image.
    ///
    /// La meme source que `security::profile::classify` : les deux doivent
    /// s'accorder, et un role connu ici mais pas la-bas serait un processus
    /// supervise sans etre sandboxe.
    ///
    /// BOUCHAUD_C8_RECONNAITRE_LE_VRAI_CHEMIN_V1
    ///
    /// La reconnaissance du courtier exigeait `/usr/bin/bo-navigateur`. Or le
    /// bureau lance `client::CHEMIN_NAVIGATEUR`, qui vaut `/bo-navigateur` --
    /// le binaire est deplie a la racine du RAMFS, pas dans `/usr/bin`. Aucun
    /// lancement reel n'etait donc reconnu, et le registre restait vide : la
    /// supervision existait et ne supervisait rien.
    ///
    /// La comparaison porte desormais sur le NOM du fichier, ce qui rend le
    /// repertoire sans importance -- il a deja change une fois.
    pub fn depuis_image(image: &str) -> Option<Role> {
        let nom = match image.rsplit_once('/') {
            Some((_, nom)) => nom,
            None => image,
        };
        match nom {
            "WebContent" => Some(Role::Rendu),
            "RequestServer" => Some(Role::Reseau),
            "ImageDecoder" => Some(Role::Decodeur),
            "WebWorker" => Some(Role::Travailleur),
            "Compositor" => Some(Role::Composition),
            // BOUCHAUD_C24_LE_NOM_REELLEMENT_LIVRE
            //
            // `BouchaudBrowserHost` est le nom du binaire que
            // `tools/ci/run_ladybird_browser_host.sh` copie dans l'image, et
            // il n'etait reconnu ni ici ni par `security::profile` : le test
            // `ends_with("/BrowserHost")` echoue dessus, parce que le
            // caractere qui precede n'est pas une barre oblique mais un `d`.
            //
            // Les trois noms designent le meme programme : le binaire est
            // construit sous le nom `BouchaudBrowserHost`, copie a la racine
            // du RAMFS sous `bo-navigateur`, et `BrowserHost` reste le nom
            // historique. Les trois se lisent encore dans le depot ; en
            // oublier un revient a ne pas superviser le navigateur.
            "BrowserHost" | "BouchaudBrowserHost" | "bo-navigateur" => Some(Role::Courtier),
            _ => None,
        }
    }

    /// La mort de ce role condamne-t-elle ses enfants ?
    pub const fn emporte_ses_enfants(self) -> bool {
        matches!(self, Role::Courtier)
    }
}

/// Ou en est un processus supervise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Etat {
    Vivant,
    /// Sorti proprement (code zero).
    Termine,
    /// Sorti sur un code non nul, ou tue.
    Plante,
    /// Son courtier est mort : il n'a plus personne a qui parler.
    Orphelin,
}

#[derive(Clone, Copy)]
struct Entree {
    pid: u32,
    /// Le courtier qui l'a lance, ou zero.
    courtier: u32,
    role: Role,
    /// Le contexte de rendu -- l'onglet -- qu'il sert. Zero pour un courtier.
    contexte: u32,
    etat: Etat,
    depuis_ns: u64,
    code_sortie: i32,
    occupee: bool,
}

/// Combien de fois un contexte a deja ete relance, et quand.
#[derive(Clone, Copy)]
struct Relances {
    contexte: u32,
    role: Role,
    compte: u32,
    premiere_ns: u64,
    occupee: bool,
}

struct Registre {
    entrees: [Entree; SUIVIS_MAX],
    relances: [Relances; SUIVIS_MAX],
}

const ENTREE_VIDE: Entree = Entree {
    pid: 0,
    courtier: 0,
    role: Role::Rendu,
    contexte: 0,
    etat: Etat::Termine,
    depuis_ns: 0,
    code_sortie: 0,
    occupee: false,
};

const RELANCE_VIDE: Relances = Relances {
    contexte: 0,
    role: Role::Rendu,
    compte: 0,
    premiere_ns: 0,
    occupee: false,
};

static REGISTRE: SpinLock<Registre> = SpinLock::new(Registre {
    entrees: [ENTREE_VIDE; SUIVIS_MAX],
    relances: [RELANCE_VIDE; SUIVIS_MAX],
});

// BOUCHAUD_C24_PROFIL_DE_DEMARRAGE
//
// Le profil du demarrage EN COURS. Il vit ici, contre le registre, parce que
// c'est `note_lancement` qui connait l'instant ou chaque processus demarre :
// le faire remonter ailleurs demanderait un second chemin, qui divergerait.
static PROFIL: SpinLock<crate::kernel::navigateur::demarrage::Profil> =
    SpinLock::new(crate::kernel::navigateur::demarrage::Profil::neuf());

/// Le double-clic : un nouveau demarrage commence, le precedent est efface.
pub fn demarrage_commence(maintenant_ns: u64) {
    use crate::kernel::navigateur::demarrage::Jalon;
    let mut profil = PROFIL.lock();
    profil.reinitialise();
    profil.note(Jalon::Clic, maintenant_ns);
}

/// La premiere trame : le demarrage est fini, et on dit ou le temps est passe.
pub fn demarrage_acheve(maintenant_ns: u64) {
    use crate::kernel::navigateur::demarrage::{Jalon, JALONS};
    let (profil, neuf) = {
        let mut profil = PROFIL.lock();
        let neuf = profil.note(Jalon::PremiereTrame, maintenant_ns);
        (*profil, neuf)
    };
    // UNE SEULE FOIS PAR DEMARRAGE. `first_paint` peut etre appelee a chaque
    // trame ; repeter le profil a soixante hertz noierait le journal serie,
    // dont le budget est borne, sous une ligne qui ne change plus.
    if !neuf || !profil.abouti() {
        return;
    }
    let Some(total) = profil.total_ns() else { return };

    let mut manquants = [Jalon::Clic; JALONS];
    let combien = profil.manquants(&mut manquants);

    crate::serial_println!(
        "[LADYBIRD-DEMARRAGE] total_ms={} courtier_ms={} reseau_ms={} decodeur_ms={} \
composition_ms={} rendu_ms={} trame_ms={} manquants={}",
        total / 1_000_000,
        segment_ms(&profil, Jalon::Courtier),
        segment_ms(&profil, Jalon::Reseau),
        segment_ms(&profil, Jalon::Decodeur),
        segment_ms(&profil, Jalon::Composition),
        segment_ms(&profil, Jalon::Rendu),
        segment_ms(&profil, Jalon::PremiereTrame),
        combien,
    );
    for index in 0..combien {
        crate::serial_println!(
            "[LADYBIRD-DEMARRAGE] jamais_vu={}", manquants[index].nom());
    }
    if let Some((jalon, duree)) = profil.plus_long() {
        // L'ETAPE A OPTIMISER, nommee. C'est la seule ligne de ce bloc qui
        // serve a decider quoi faire.
        crate::serial_println!(
            "[LADYBIRD-DEMARRAGE] plus_long={} duree_ms={} part_pour_cent={}",
            jalon.nom(),
            duree / 1_000_000,
            if total == 0 { 0 } else { duree.saturating_mul(100) / total },
        );
    }
}

/// `-1` quand l'etape n'a pas ete vue.
///
/// Un entier signe plutot qu'un `Option` parce que cette valeur part dans une
/// ligne de journal : `-1` s'y lit « non mesure » sans ambiguite, la ou `0`
/// se lirait « instantane » et designerait une etape gratuite qui n'a jamais
/// eu lieu.
fn segment_ms(
    profil: &crate::kernel::navigateur::demarrage::Profil,
    jalon: crate::kernel::navigateur::demarrage::Jalon,
) -> i64 {
    match profil.segment_ns(jalon) {
        Some(ns) => (ns / 1_000_000) as i64,
        None => -1,
    }
}

/// Le profil du demarrage courant, pour qui veut l'afficher.
pub fn profil_de_demarrage() -> crate::kernel::navigateur::demarrage::Profil {
    *PROFIL.lock()
}

static LANCEMENTS: AtomicU64 = AtomicU64::new(0);
static SORTIES: AtomicU64 = AtomicU64::new(0);
static PLANTAGES: AtomicU64 = AtomicU64::new(0);
static ORPHELINS: AtomicU64 = AtomicU64::new(0);
static RELANCES_REFUSEES: AtomicU64 = AtomicU64::new(0);
static REGISTRE_PLEIN: AtomicU64 = AtomicU64::new(0);

/// Enregistre le lancement d'un processus de navigateur.
///
/// Rend `false` si le registre est plein. Ce n'est pas une erreur fatale --
/// le processus tourne quand meme --, mais il n'est alors PAS supervise, et
/// `[LADYBIRD-SUP] registre_plein=` le dit plutot que de laisser croire a une
/// supervision qui n'a pas lieu.
pub fn note_lancement(pid: u32, role: Role, courtier: u32, contexte: u32, maintenant_ns: u64) -> bool {
    let mut registre = REGISTRE.lock();
    // Un pid recycle efface son ancienne entree : sans cela, deux entrees
    // porteraient le meme pid et la sortie serait attribuee a la mauvaise.
    for entree in registre.entrees.iter_mut() {
        if entree.occupee && entree.pid == pid {
            *entree = ENTREE_VIDE;
        }
    }
    let Some(place) = registre.entrees.iter().position(|e| !e.occupee) else {
        REGISTRE_PLEIN.fetch_add(1, Ordering::Relaxed);
        return false;
    };
    // BOUCHAUD_C24_PROFIL_DE_DEMARRAGE : le role qui demarre pose son jalon.
    //
    // ICI, et non dans l'appelant : `note_lancement` est le seul point par
    // lequel passe le demarrage d'un processus du navigateur. Poser les jalons
    // chez les appelants demanderait de les retrouver tous, et le prochain
    // oublierait le sien.
    if let Some(jalon) = jalon_du_role(role) {
        PROFIL.lock().note(jalon, maintenant_ns);
    }
    registre.entrees[place] = Entree {
        pid,
        courtier,
        role,
        contexte,
        etat: Etat::Vivant,
        depuis_ns: maintenant_ns,
        code_sortie: 0,
        occupee: true,
    };
    LANCEMENTS.fetch_add(1, Ordering::Relaxed);
    true
}

/// Enregistre la sortie d'un processus supervise.
///
/// Rend le role sorti, ou `None` si ce pid n'etait pas supervise -- ce qui est
/// le cas normal pour tout processus qui n'est pas du navigateur.
pub fn note_sortie(pid: u32, code: i32, maintenant_ns: u64) -> Option<Role> {
    let mut registre = REGISTRE.lock();
    let index = registre.entrees.iter().position(|e| e.occupee && e.pid == pid)?;
    let role = registre.entrees[index].role;
    let etat = if code == 0 { Etat::Termine } else { Etat::Plante };
    registre.entrees[index].etat = etat;
    registre.entrees[index].code_sortie = code;
    registre.entrees[index].depuis_ns = maintenant_ns;
    SORTIES.fetch_add(1, Ordering::Relaxed);
    if etat == Etat::Plante {
        PLANTAGES.fetch_add(1, Ordering::Relaxed);
    }

    // LA REGLE D'ISOLATION. La mort d'un rendu ne touche que lui. La mort d'un
    // COURTIER condamne ses enfants : ils n'ont plus personne a qui parler, et
    // les laisser vivants ferait des processus qui tournent pour rien.
    if role.emporte_ses_enfants() {
        for entree in registre.entrees.iter_mut() {
            if entree.occupee && entree.courtier == pid && entree.etat == Etat::Vivant {
                entree.etat = Etat::Orphelin;
                ORPHELINS.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    Some(role)
}

/// Le courtier doit-il relancer ce contexte ?
///
/// Le budget est par contexte et par fenetre de temps. Relancer sans compter
/// transforme un moteur de rendu qui plante sur une page en une machine qui
/// ne fait plus que redemarrer.
pub fn autorise_relance(role: Role, contexte: u32, maintenant_ns: u64) -> bool {
    let mut registre = REGISTRE.lock();
    let cible = registre
        .relances
        .iter()
        .position(|r| r.occupee && r.contexte == contexte && r.role == role);

    if let Some(index) = cible {
        let entree = registre.relances[index];
        if maintenant_ns.saturating_sub(entree.premiere_ns) > FENETRE_RELANCE_NS {
            // La fenetre est passee : un plantage isole tous les quarts d'heure
            // n'est pas une boucle, et ne doit pas finir par bloquer.
            registre.relances[index].compte = 1;
            registre.relances[index].premiere_ns = maintenant_ns;
            return true;
        }
        if entree.compte >= RELANCES_MAX {
            RELANCES_REFUSEES.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        registre.relances[index].compte += 1;
        return true;
    }

    let Some(place) = registre.relances.iter().position(|r| !r.occupee) else {
        // Pas de place pour compter : autoriser serait le choix dangereux --
        // c'est exactement le cas ou beaucoup de contextes plantent.
        RELANCES_REFUSEES.fetch_add(1, Ordering::Relaxed);
        return false;
    };
    registre.relances[place] = Relances {
        contexte,
        role,
        compte: 1,
        premiere_ns: maintenant_ns,
        occupee: true,
    };
    true
}

/// Oublie un pid : son entree est libre pour un autre.
pub fn oublie(pid: u32) {
    let mut registre = REGISTRE.lock();
    for entree in registre.entrees.iter_mut() {
        if entree.occupee && entree.pid == pid {
            *entree = ENTREE_VIDE;
        }
    }
}

/// Processus VIVANTS d'un role donne.
pub fn vivants(role: Role) -> usize {
    let registre = REGISTRE.lock();
    registre
        .entrees
        .iter()
        .filter(|e| e.occupee && e.role == role && e.etat == Etat::Vivant)
        .count()
}

/// L'etat d'un pid supervise.
pub fn etat(pid: u32) -> Option<Etat> {
    let registre = REGISTRE.lock();
    registre
        .entrees
        .iter()
        .find(|e| e.occupee && e.pid == pid)
        .map(|e| e.etat)
}

/// Le contexte servi par un pid supervise.
pub fn contexte(pid: u32) -> Option<u32> {
    let registre = REGISTRE.lock();
    registre
        .entrees
        .iter()
        .find(|e| e.occupee && e.pid == pid)
        .map(|e| e.contexte)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Compteurs {
    pub lancements: u64,
    pub sorties: u64,
    pub plantages: u64,
    pub orphelins: u64,
    pub relances_refusees: u64,
    pub registre_plein: u64,
    pub rendus_vivants: u64,
    pub suivis: u64,
}

pub fn compteurs() -> Compteurs {
    let suivis = {
        let registre = REGISTRE.lock();
        registre.entrees.iter().filter(|e| e.occupee).count() as u64
    };
    Compteurs {
        lancements: LANCEMENTS.load(Ordering::Relaxed),
        sorties: SORTIES.load(Ordering::Relaxed),
        plantages: PLANTAGES.load(Ordering::Relaxed),
        orphelins: ORPHELINS.load(Ordering::Relaxed),
        relances_refusees: RELANCES_REFUSEES.load(Ordering::Relaxed),
        registre_plein: REGISTRE_PLEIN.load(Ordering::Relaxed),
        rendus_vivants: vivants(Role::Rendu) as u64,
        suivis,
    }
}

pub fn log_stats() {
    let c = compteurs();
    // Une trace muette quand aucun navigateur ne tourne : le releve periodique
    // sort dans TOUTES les traces, et une ligne de zeros par releve noierait
    // le port serie.
    if c.lancements == 0 {
        return;
    }
    crate::serial_println!(
        "[LADYBIRD-SUP] suivis={} rendus_vivants={} lancements={} sorties={} plantages={} orphelins={} relances_refusees={} registre_plein={}",
        c.suivis, c.rendus_vivants, c.lancements, c.sorties, c.plantages,
        c.orphelins, c.relances_refusees, c.registre_plein
    );
}

/// Le jalon de demarrage que ce role franchit, s'il en a un.
///
/// `Travailleur` n'en a pas : un WebWorker demarre A LA DEMANDE, longtemps
/// apres la premiere trame. Lui donner un jalon ferait entrer dans le profil
/// de demarrage un temps qui n'en fait pas partie.
fn jalon_du_role(role: Role) -> Option<crate::kernel::navigateur::demarrage::Jalon> {
    use crate::kernel::navigateur::demarrage::Jalon;
    match role {
        Role::Courtier => Some(Jalon::Courtier),
        Role::Reseau => Some(Jalon::Reseau),
        Role::Decodeur => Some(Jalon::Decodeur),
        Role::Composition => Some(Jalon::Composition),
        Role::Rendu => Some(Jalon::Rendu),
        Role::Travailleur => None,
    }
}
