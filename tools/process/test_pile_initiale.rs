//! LA FENETRE DE PILE INITIALE, A L'HOTE.
//!
//! # Le defaut que ce banc rend impossible
//!
//! `AddressSpace::write()` passe par la vue noyau des tables : une page
//! absente n'y provoque aucune faute, elle PERD l'ecriture. Une fenetre trop
//! petite ne planterait donc pas -- elle tronquerait silencieusement
//! l'environnement d'un processus.
//!
//! Le symptome serait un navigateur qui ne trouve plus ses polices, ou une
//! libc dont le canari de pile vaut zero. Rien dans un test d'integration ne
//! le verrait, et il n'apparaitrait qu'avec un environnement plus gros que
//! celui du jour ou le code a ete ecrit.
//!
//! # Les cas
//!
//! Ceux que la revue demande : un gros `argv`, un gros `envp`, une liste auxv
//! qui grandit, le depassement de la pile entiere, et le comportement aux
//! frontieres de page.

#[path = "../../src/kernel/process/pile_initiale.rs"]
mod pile_initiale;

use pile_initiale::{ecriture_dans_la_fenetre, fenetre, octets_bruts, tient_dans, AUXV_MAX, PAGE};

const PILE: u64 = 8 * 1024 * 1024;
const SOMMET: u64 = 0x7fff_0000_0000;

/// Ce que `build_stack` consomme reellement, simule ici avec la MEME regle
/// que `push_bytes` : reculer de la longueur, puis aligner vers le bas sur
/// huit octets.
///
/// C'est le coeur du banc : si cette simulation et `octets_bruts` divergent,
/// la fenetre est fausse. Elle est ecrite a partir du code de `build_stack`
/// et non a partir de `octets_bruts`, sans quoi le banc ne verifierait que sa
/// propre coherence.
fn consomme_reellement(argv: &[usize], envp: &[usize], auxv_entrees: u64) -> u64 {
    let mut curseur: u64 = SOMMET - 16;
    let mut pousse = |longueur: u64, curseur: &mut u64| {
        *curseur -= longueur;
        *curseur &= !0x7;
    };

    for longueur in argv.iter() {
        pousse(*longueur as u64 + 1, &mut curseur);
    }
    for longueur in envp.iter() {
        pousse(*longueur as u64 + 1, &mut curseur);
    }
    pousse(16, &mut curseur); // AT_RANDOM
    pousse(7, &mut curseur); // "x86_64\0"
    if let Some(premier) = argv.first() {
        pousse(*premier as u64 + 1, &mut curseur); // AT_EXECFN
    }

    // Le bloc argc/argv/envp/auxv, aligne sur seize octets.
    let bloc = 8
        + (argv.len() as u64 + 1) * 8
        + (envp.len() as u64 + 1) * 8
        + auxv_entrees * 16;
    curseur -= bloc;
    curseur &= !0xf;

    SOMMET - curseur
}

fn verifie_couvre(argv: &[usize], envp: &[usize], auxv: u64, cas: &str) {
    let calcule = fenetre(argv, envp, auxv);
    let reel = consomme_reellement(argv, envp, auxv);
    assert!(
        calcule >= reel,
        "{} : la fenetre calculee ({} octets) est plus petite que ce que \
         build_stack consomme ({} octets). Une ecriture serait PERDUE.",
        cas,
        calcule,
        reel
    );
    // Et la postcondition doit etre satisfaite pour l'adresse la plus basse.
    assert!(
        ecriture_dans_la_fenetre(SOMMET - reel, SOMMET, calcule),
        "{} : la postcondition refuse une ecriture pourtant legitime",
        cas
    );
}

#[test]
fn un_cas_ordinaire_tient_dans_deux_pages() {
    let argv = [12usize];
    let envp = [30usize, 25, 40];
    verifie_couvre(&argv, &envp, 20, "ordinaire");
    // Une poignee de chaines ne doit pas couter plus que quelques pages : tout
    // l'interet du changement est de ne plus projeter huit mebioctets.
    assert!(fenetre(&argv, &envp, 20) <= 3 * PAGE);
}

#[test]
fn un_gros_argv_est_couvert() {
    // Mille arguments de deux cent cinquante-cinq octets : une ligne de
    // commande de type `find ... -exec`.
    let argv: Vec<usize> = core::iter::repeat(255).take(1000).collect();
    verifie_couvre(&argv, &[], 20, "gros argv");
}

#[test]
fn un_gros_envp_est_couvert() {
    // Le navigateur pose beaucoup de variables, et certaines sont longues :
    // FONTCONFIG_PATH, LD_LIBRARY_PATH, BOUCHAUD_M9_URL.
    let envp: Vec<usize> = core::iter::repeat(4096).take(200).collect();
    verifie_couvre(&[16], &envp, 20, "gros envp");
}

#[test]
fn des_chaines_vides_et_une_liste_vide_sont_couvertes() {
    verifie_couvre(&[], &[], 20, "tout vide");
    verifie_couvre(&[0], &[0, 0, 0], 20, "chaines vides");
}

#[test]
fn une_liste_auxv_qui_grandit_reste_couverte() {
    // LA REGRESSION QUE LA REVUE DEMANDE D'EMPECHER.
    //
    // Le budget etait devine -- « 64 paires » en dur. Il est desormais
    // PARAMETRE, et `build_stack` refuse au-dela de AUXV_MAX au lieu de
    // deborder. Le banc verifie la couverture sur toute la plage autorisee.
    for entrees in [1u64, 20, 32, AUXV_MAX] {
        verifie_couvre(&[16], &[64, 64], entrees, "auxv qui grandit");
    }
}

#[test]
fn la_fenetre_grandit_avec_le_besoin() {
    let petite = fenetre(&[8], &[8], 20);
    let grande = fenetre(&[8], &core::iter::repeat(1024).take(100).collect::<Vec<_>>(), 20);
    assert!(grande > petite, "une fenetre qui ne suit pas le besoin est une fenetre fausse");
}

#[test]
fn la_fenetre_est_toujours_un_multiple_de_page() {
    // Elle est passee a `map_alloc_accounted`, qui projette des pages.
    for envp in [vec![], vec![1usize], vec![4095], vec![4096], vec![4097]] {
        let f = fenetre(&[7], &envp, 20);
        assert_eq!(f % PAGE, 0, "fenetre non alignee : {}", f);
    }
}

#[test]
fn un_environnement_demesure_est_refuse_et_non_tronque() {
    // Seize mebioctets d'environnement sur une pile de huit : la seule bonne
    // reponse est le refus. Projeter une fenetre plus grande que la pile
    // ecrirait sous elle, dans la memoire d'autre chose.
    let envp: Vec<usize> = core::iter::repeat(65536).take(256).collect();
    let f = fenetre(&[16], &envp, 20);
    assert!(f > PILE);
    assert!(!tient_dans(f, PILE), "un environnement plus gros que la pile doit etre refuse");
}

#[test]
fn la_postcondition_attrape_une_ecriture_hors_fenetre() {
    // C'est le filet qui rattrape une sous-estimation future : le jour ou
    // `build_stack` ecrira quelque chose que le calcul ne connait pas.
    let f = 2 * PAGE;
    assert!(ecriture_dans_la_fenetre(SOMMET - f, SOMMET, f), "la borne exacte est dedans");
    assert!(!ecriture_dans_la_fenetre(SOMMET - f - 1, SOMMET, f), "un octet en dessous est dehors");
    assert!(ecriture_dans_la_fenetre(SOMMET - 8, SOMMET, f));
}

#[test]
fn le_calcul_ne_deborde_pas_sur_des_longueurs_absurdes() {
    // Les longueurs viennent de l'espace utilisateur par `execve`. Une
    // multiplication qui deborderait rendrait une fenetre MINUSCULE, et le
    // refus ne se declencherait pas.
    let argv: Vec<usize> = core::iter::repeat(usize::MAX).take(4).collect();
    let bruts = octets_bruts(&argv, &[], 20);
    assert_eq!(bruts, u64::MAX, "la saturation doit plafonner, pas reboucler");
    let f = fenetre(&argv, &[], 20);
    assert!(!tient_dans(f, PILE), "une demande absurde doit etre refusee");
}
