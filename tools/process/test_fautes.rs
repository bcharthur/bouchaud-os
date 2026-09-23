//! LE LIVRE DE COMPTES DES FAUTES DE PAGE, A L'HOTE.
//!
//! # Ce que ce banc protege
//!
//! Le chemin de faute de page ne s'execute que dans QEMU, et ses erreurs sont
//! silencieuses par nature : un compte attribue au mauvais processus, une
//! entree chassee qui emporte le processus qu'on observait, une moyenne
//! calculee sur zero echantillon. Aucune de ces trois ne fait echouer un test
//! d'integration -- elles rendent seulement un tableau faux, et un tableau
//! faux oriente l'optimisation vers le mauvais processus.
//!
//! Les proprietes verifiees ici sont celles dont depend la CONFIANCE dans le
//! tableau :
//!
//!   * deux processus ne melangent jamais leurs comptes ;
//!   * un PID reutilise ne herite pas des comptes du precedent occupant ;
//!   * la pire faute n'est pas la moyenne -- c'est elle que l'utilisateur voit ;
//!   * la chasse prend la plus ANCIENNE, jamais la plus active ;
//!   * elle se compte, pour que la borne ne mente pas en silence ;
//!   * la categorie dominante est celle qui coute du TEMPS, pas celle qui
//!     compte le plus de fautes ;
//!   * une horloge qui recule ne fait pas chasser un processus vivant.

#[path = "../../src/kernel/process/fautes.rs"]
mod fautes;

use fautes::{Categorie, Compte, Journal, PROCESSUS_MAX};


#[test]
fn deux_processus_ne_melangent_jamais_leurs_comptes() {
    let mut journal = Journal::neuf();
    journal.note(10, Categorie::Zero, 1_000, 1);
    journal.note(10, Categorie::Zero, 3_000, 2);
    journal.note(20, Categorie::FichierPrive, 500, 3);

    let dix = journal.compte(10, Categorie::Zero);
    assert_eq!(dix.nombre, 2);
    assert_eq!(dix.total_ns, 4_000);
    assert_eq!(dix.pire_ns, 3_000, "la pire faute suit le nombre et le total");

    assert!(journal.compte(20, Categorie::Zero).vide(), "le pid 20 n'a jamais faute en Zero");
    assert!(journal.compte(10, Categorie::FichierPrive).vide(), "une categorie n'herite pas d'une autre");
    assert_eq!(journal.total(10).nombre, 2);
    assert_eq!(journal.total(20).nombre, 1);
}

#[test]
fn la_pire_faute_survit_a_mille_fautes_courtes() {
    // Une saccade de 4 ms se voit a l'ecran. La moyenne la noie : c'est pour
    // cela que `pire_ns` existe a cote de `total_ns`.
    let mut journal = Journal::neuf();
    for _ in 0..999 {
        journal.note(7, Categorie::Zero, 10_000, 1);
    }
    journal.note(7, Categorie::Zero, 4_000_000, 2);

    let compte = journal.compte(7, Categorie::Zero);
    assert_eq!(compte.pire_ns, 4_000_000);
    assert!(compte.moyenne_ns() < 20_000, "la moyenne reste basse : elle ne suffit pas");
}

#[test]
fn une_moyenne_sur_zero_echantillon_n_existe_pas() {
    let vide = Compte::default();
    assert!(vide.vide());
    assert_eq!(vide.moyenne_ns(), 0, "zero plutot qu'une division par zero");

    let journal = Journal::neuf();
    // `None` dit « non mesure ». Zero dirait « mesure a zero », et ce n'est
    // pas la meme chose -- c'est toute la difference entre une colonne vide
    // et une colonne qui ment.
    assert!(journal.part_pour_mille(1, 1_000_000).is_none());
    assert!(journal.categorie_dominante(1).is_none());
}

#[test]
fn la_chasse_emporte_le_plus_ancien_et_epargne_le_plus_actif() {
    let mut journal = Journal::neuf();
    for index in 0..PROCESSUS_MAX {
        let pid = index as u32 + 1;
        journal.note(pid, Categorie::Zero, 100, 1000 + index as u64);
    }
    assert_eq!(journal.suivis(), PROCESSUS_MAX, "le livre se remplit jusqu'a sa borne");

    // Le plus recemment inscrit refaute : il ne doit pas etre la victime.
    journal.note(PROCESSUS_MAX as u32, Categorie::Zero, 100, 5_000);
    journal.note(PROCESSUS_MAX as u32 + 99, Categorie::Zero, 100, 6_000);

    assert!(journal.total(1).vide(), "le pid 1 avait la plus vieille faute");
    assert!(!journal.total(PROCESSUS_MAX as u32).vide(), "chasser le plus actif rendrait le livre inutile");
    assert_eq!(journal.chasses, 1, "une borne qui se tait rend des chiffres incomplets sans le dire");
    assert_eq!(journal.suivis(), PROCESSUS_MAX, "le livre ne depasse jamais sa borne");
}

#[test]
fn un_pid_reutilise_n_herite_de_rien() {
    let mut journal = Journal::neuf();
    journal.note(42, Categorie::Zero, 9_000, 1);
    journal.oublie(42);
    assert!(journal.total(42).vide());

    journal.note(42, Categorie::Zero, 7, 2);
    let compte = journal.compte(42, Categorie::Zero);
    assert_eq!(compte.nombre, 1);
    assert_eq!(compte.total_ns, 7);
    assert_eq!(compte.pire_ns, 7, "la pire faute de l'ancien porteur ne doit pas rester");
}

#[test]
fn la_categorie_dominante_est_la_plus_couteuse_en_temps() {
    let mut journal = Journal::neuf();
    for _ in 0..100 {
        journal.note(3, Categorie::Zero, 1_000, 1);
    }
    journal.note(3, Categorie::FichierPrive, 500_000, 2);

    let (categorie, compte) = journal.categorie_dominante(3).expect("mesure faite");
    assert_eq!(categorie, Categorie::FichierPrive, "cent fautes rapides ne valent pas une lente");
    assert_eq!(compte.total_ns, 500_000);
}

#[test]
fn la_part_se_lit_en_pour_mille() {
    let mut journal = Journal::neuf();
    journal.note(5, Categorie::Zero, 3_000_000, 1);
    // En pourcent entier, trois millisecondes par seconde valent 0 : le
    // chiffre qu'on suit pendant une optimisation disparaitrait dans
    // l'arrondi.
    assert_eq!(journal.part_pour_mille(5, 1_000_000_000), Some(3));
    assert!(journal.part_pour_mille(5, 0).is_none(), "une fenetre nulle ne rend pas de part");
}

#[test]
fn un_recul_d_horloge_ne_fait_chasser_personne() {
    let mut journal = Journal::neuf();
    journal.note(8, Categorie::Zero, 10, 10_000);
    // Un autre coeur, dont l'horloge est en retard, note pour le meme pid.
    journal.note(8, Categorie::Zero, 10, 1);

    for index in 1..PROCESSUS_MAX {
        journal.note(100 + index as u32, Categorie::Zero, 10, 5_000 + index as u64);
    }
    journal.note(999, Categorie::Zero, 10, 20_000);

    assert!(
        !journal.total(8).vide(),
        "sinon un processus vivant se ferait chasser par sa propre mesure"
    );
}

#[test]
fn le_classement_descend_du_plus_couteux() {
    let mut journal = Journal::neuf();
    journal.note(1, Categorie::Zero, 100, 1);
    journal.note(2, Categorie::Zero, 900, 2);
    journal.note(3, Categorie::Zero, 500, 3);

    let mut sortie = [(0u32, Compte::default()); 4];
    assert_eq!(journal.classement(&mut sortie), 3);
    assert_eq!(sortie[0].0, 2, "c'est l'ordre dans lequel on regarde qui optimiser");
    assert_eq!(sortie[1].0, 3);
    assert_eq!(sortie[2].0, 1);

    // Le noyau appelle ceci depuis un chemin de diagnostic borne : un tampon
    // plus court que le nombre de processus ne doit pas deborder.
    let mut petite = [(0u32, Compte::default()); 1];
    assert_eq!(journal.classement(&mut petite), 1);
}

#[test]
fn chaque_categorie_a_un_rang_unique_et_reversible() {
    // Le rang sert d'indice de tableau. Deux categories au meme rang
    // additionneraient silencieusement leurs comptes.
    let toutes = [
        Categorie::Zero,
        Categorie::FichierPrive,
        Categorie::Partage,
        Categorie::Materiel,
        Categorie::Copie,
        Categorie::Attente,
        Categorie::Echec,
    ];
    assert_eq!(toutes.len(), fautes::CATEGORIES);
    for (index, categorie) in toutes.iter().enumerate() {
        assert_eq!(categorie.rang(), index);
        assert_eq!(Categorie::depuis_rang(index), Some(*categorie));
        assert!(!categorie.nom().is_empty());
    }
    assert_eq!(Categorie::depuis_rang(fautes::CATEGORIES), None);
}

// ============================================================================
// BOUCHAUD_C54_PAR_PID_OU_RIEN
// ============================================================================
//
// Le defaut que ces trois tests interdisent : publier une ligne
// `FAULT_FILE_BREAKDOWN pid=18` dont les chiffres viennent de compteurs
// GLOBAUX lus a la sortie du processus 18.
//
// L'etiquette promet alors une attribution que les chiffres n'ont pas. Quand
// WebContent, le Compositor et le worker travaillent en meme temps -- ce qui
// est exactement le cas qu'on veut mesurer -- le « cout du worker » contient
// celui des deux autres.
//
// La difference entre deux sorties successives ne repare rien : elle suppose
// que les processus ne se chevauchent pas.

fn phase(total_ns: u64, acquire_ns: u64, backing_dans_acquire: u64) -> fautes::PhasesFichier {
    fautes::PhasesFichier {
        nombre: 1,
        total_ns,
        acquire_ns,
        acquire_backing_ns: backing_dans_acquire,
        miss_n: 1,
        miss_ns: acquire_ns,
        pire_ns: total_ns,
        ..Default::default()
    }
}

#[test]
fn deux_processus_entrelaces_ne_melangent_pas_leurs_phases() {
    // A : dix fautes a une milliseconde.  B : vingt fautes a trois.
    // Entrelacees, puis B « sort » avant A -- le scenario exact qui faisait
    // mentir la version globale.
    let mut journal = fautes::Journal::neuf();
    let mut instant = 0u64;
    let mut a_restantes = 10;
    let mut b_restantes = 20;
    while a_restantes > 0 || b_restantes > 0 {
        if a_restantes > 0 {
            instant += 1;
            journal.note_phases(7, &phase(1_000_000, 900_000, 800_000), instant);
            a_restantes -= 1;
        }
        for _ in 0..2 {
            if b_restantes > 0 {
                instant += 1;
                journal.note_phases(9, &phase(3_000_000, 2_500_000, 2_000_000), instant);
                b_restantes -= 1;
            }
        }
    }

    let a = journal.phases(7);
    let b = journal.phases(9);

    assert_eq!(a.nombre, 10, "A doit porter SES dix fautes");
    assert_eq!(a.total_ns, 10_000_000, "A doit porter ses dix millisecondes");
    assert_eq!(b.nombre, 20, "B doit porter SES vingt fautes");
    assert_eq!(b.total_ns, 60_000_000, "B doit porter ses soixante millisecondes");

    // Et surtout : NI l'un NI l'autre ne porte le cumul.
    assert_ne!(a.total_ns, 70_000_000, "A porte le cumul : attribution globale");
    assert_ne!(b.total_ns, 70_000_000, "B porte le cumul : attribution globale");
}

#[test]
fn acquire_backing_est_un_dont_pas_un_additif() {
    // `acquire_ns` CONTIENT `acquire_backing_ns` : l'acquisition du cache de
    // pages fait la lecture du support elle-meme. Les additionner compterait
    // la lecture deux fois et pourrait rendre un residu NEGATIF -- c'est-a-dire
    // saturer a zero et cacher le trou qu'on cherche.
    let mut journal = fautes::Journal::neuf();
    journal.note_phases(3, &phase(10_000_000, 9_000_000, 8_000_000), 1);
    let p = journal.phases(3);

    assert_eq!(p.explique_ns(), 9_000_000, "explique = attente+acquire+direct+mm+map");
    assert_eq!(p.residu_ns(), 1_000_000);
    assert_eq!(p.residu_pct(), 10);

    // Le sous-champ reste lisible, et reste INFERIEUR a son contenant.
    assert!(p.acquire_backing_ns <= p.acquire_ns,
            "le DONT ne peut pas depasser ce dont il est tire");
}

#[test]
fn un_processus_sans_faute_fichier_rend_une_decomposition_vide() {
    // Rendre `Default` plutot qu'une valeur d'un autre processus : un pid
    // absent du livre ne doit jamais heriter des chiffres du voisin.
    let mut journal = fautes::Journal::neuf();
    journal.note_phases(5, &phase(4_000_000, 3_000_000, 2_000_000), 1);
    let absent = journal.phases(6);
    assert_eq!(absent.nombre, 0);
    assert_eq!(absent.total_ns, 0);
    assert_eq!(absent.residu_pct(), 0, "pas de division par zero sur un vide");
}
