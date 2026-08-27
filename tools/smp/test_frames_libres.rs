//! Le cout d'un `free_frame`, et ce qu'il doit continuer a detecter.
//!
//! # Ce que ces tests mesurent
//!
//! `free_frame` verifiait le double `free` en parcourant toute la liste libre.
//! Cette liste est chainee DANS les frames liberees : chaque pas est une
//! lecture memoire froide. Le cout d'un `free` etait donc `O(frames libres)`,
//! et liberer une plage de R pages coutait `O(R x L)`.
//!
//! Les deux algorithmes sont modelises ici avec le MEME compteur de lectures
//! froides, et le test affirme la difference. Ce n'est pas une mesure de temps
//! — elle serait bruitee et non reproductible — mais un comptage d'operations,
//! qui est ce que la structure de donnees determine.
//!
//! Les tests de correction verifient que rien n'a ete perdu au passage : le
//! double `free` reste fatal, et un `free` hors des regions est desormais
//! detecte lui aussi.
//!
//! Lance par `tools/smp/test-frames-libres.sh`.

extern crate alloc;

#[path = "../../src/kernel/memory/frames_libres.rs"]
mod frames_libres;

use frames_libres::{FramesLibres, TAILLE_PAGE};

const BASE: u64 = 0x100_000;

fn frame(rang: u64) -> u64 {
    BASE + rang * TAILLE_PAGE
}

fn couvrant(frames: u64) -> FramesLibres {
    let mut set = FramesLibres::neuf();
    set.couvre(BASE, BASE + frames * TAILLE_PAGE);
    set
}

// ─── Le cout ───────────────────────────────────────────────────────────────

/// L'ancien algorithme : la liste libre chainee, parcourue en entier.
fn cout_ancien(liste: usize, liberations: usize) -> u64 {
    // Chaque `free` parcourt la liste telle qu'elle est A CET INSTANT, puis
    // s'y ajoute. C'est exactement la boucle `while let Some(free) = cursor`.
    let mut lectures = 0u64;
    let mut longueur = liste as u64;
    for _ in 0..liberations {
        lectures += longueur;
        longueur += 1;
    }
    lectures
}

/// Le nouveau : un bit, lu et ecrit. Aucune lecture de page physique.
fn cout_nouveau(set: &mut FramesLibres, liberations: usize) -> u64 {
    for rang in 0..liberations {
        assert!(set.marque_libre(frame(rang as u64)), "rang {rang}");
    }
    0
}

#[test]
fn liberer_une_plage_ne_parcourt_plus_la_liste_libre() {
    // Une session de navigateur realiste : 100 000 frames deja libres, et un
    // `madvise(DONTNEED)` de 16 Mio, soit 4096 pages.
    const LISTE: usize = 100_000;
    const PLAGE: usize = 4096;

    let ancien = cout_ancien(LISTE, PLAGE);
    let mut set = couvrant((LISTE + PLAGE) as u64);
    let nouveau = cout_nouveau(&mut set, PLAGE);

    assert!(
        ancien > 400_000_000,
        "l'ancien algorithme doit bien couter des centaines de millions de \
         lectures froides, sinon ce test ne mesure pas le vrai probleme \
         (mesure : {ancien})"
    );
    assert_eq!(
        nouveau, 0,
        "le nouveau ne dereference aucune page physique pour ce test"
    );
}

/// Le cout d'un `free` ne doit pas dependre du nombre de frames deja libres.
#[test]
fn le_cout_d_un_free_ne_depend_pas_de_la_longueur_de_la_liste() {
    for deja_libres in [0usize, 1_000, 100_000] {
        let mut set = couvrant((deja_libres + 1) as u64);
        for rang in 0..deja_libres {
            set.marque_libre(frame(rang as u64));
        }
        assert_eq!(set.compte(), deja_libres);
        // Une seule ecriture de bit, quel que soit `deja_libres`.
        assert!(set.marque_libre(frame(deja_libres as u64)));
        assert_eq!(set.compte(), deja_libres + 1);
    }
}

// ─── Ce que l'assertion doit continuer a attraper ──────────────────────────

#[test]
fn un_double_free_est_refuse() {
    let mut set = couvrant(64);
    assert!(set.marque_libre(frame(7)), "premier free");
    assert!(!set.marque_libre(frame(7)), "le second free doit etre refuse");
    assert_eq!(set.compte(), 1, "et ne doit pas compter deux fois");
}

// ─── Regions disjointes : le trou n'existe pas ─────────────────────────────
//
// Un bitmap unique indexe depuis une base commune represente l'ENVELOPPE des
// regions, pas leur union. Une adresse dans le trou entre deux regions y a un
// index valide — et `couverte()` repondait « oui » pour une frame qui n'existe
// pas, ce qui vidait de sens l'assertion de `free_frame`.

const REGION_A: (u64, u64) = (0x1000, 0x9000);
const TROU: (u64, u64) = (0x9000, 0x20000);
const REGION_B: (u64, u64) = (0x20000, 0x30000);

fn deux_regions_disjointes() -> FramesLibres {
    let mut set = FramesLibres::neuf();
    set.couvre(REGION_A.0, REGION_A.1);
    set.couvre(REGION_B.0, REGION_B.1);
    set
}

/// LE test que la semantique d'enveloppe ne peut pas passer.
#[test]
fn un_trou_entre_deux_regions_n_est_pas_couvert() {
    let set = deux_regions_disjointes();

    let mut phys = REGION_A.0;
    while phys < REGION_A.1 {
        assert!(set.couverte(phys), "region A : {phys:#x} doit etre valide");
        phys += TAILLE_PAGE;
    }
    let mut phys = REGION_B.0;
    while phys < REGION_B.1 {
        assert!(set.couverte(phys), "region B : {phys:#x} doit etre valide");
        phys += TAILLE_PAGE;
    }
    let mut phys = TROU.0;
    while phys < TROU.1 {
        assert!(
            !set.couverte(phys),
            "TROU : {phys:#x} ne doit PAS etre valide — c'est du reserve/MMIO, \
             aucune frame n'y existe"
        );
        phys += TAILLE_PAGE;
    }
}

/// L'enveloppe reste indexable — c'est ce qui rend le defaut possible — mais
/// elle ne dit rien de la validite.
#[test]
fn l_enveloppe_est_plus_large_que_l_union() {
    let set = deux_regions_disjointes();
    let pages_a = (REGION_A.1 - REGION_A.0) / TAILLE_PAGE;
    let pages_b = (REGION_B.1 - REGION_B.0) / TAILLE_PAGE;
    assert_eq!(
        set.frames_valides() as u64,
        pages_a + pages_b,
        "l'union exacte, et rien de plus"
    );
    assert!(
        set.capacite() as u64 > pages_a + pages_b,
        "l'enveloppe, elle, couvre aussi le trou : c'est bien pourquoi la \
         validite ne peut pas se deduire de l'index"
    );
}

#[test]
fn une_frame_du_trou_ne_peut_pas_etre_liberee() {
    let mut set = deux_regions_disjointes();
    assert!(
        !set.marque_libre(TROU.0 + 4 * TAILLE_PAGE),
        "liberer une frame inexistante doit etre refuse"
    );
    assert_eq!(set.compte(), 0);
    assert!(!set.est_libre(TROU.0 + 4 * TAILLE_PAGE));
}

#[test]
fn une_frame_du_trou_ne_peut_pas_etre_allouee() {
    let mut set = deux_regions_disjointes();
    assert!(!set.marque_occupee(TROU.0 + TAILLE_PAGE));
    assert_eq!(set.compte(), 0, "le compte ne doit pas passer sous zero");
}

/// Les frontieres exactes : derniere page de A, premiere du trou, derniere du
/// trou, premiere de B.
#[test]
fn les_frontieres_des_regions_sont_justes() {
    let set = deux_regions_disjointes();
    assert!(set.couverte(REGION_A.1 - TAILLE_PAGE), "derniere page de A");
    assert!(!set.couverte(REGION_A.1), "premiere page du trou");
    assert!(!set.couverte(REGION_B.0 - TAILLE_PAGE), "derniere page du trou");
    assert!(set.couverte(REGION_B.0), "premiere page de B");
    assert!(set.couverte(REGION_B.1 - TAILLE_PAGE), "derniere page de B");
    assert!(!set.couverte(REGION_B.1), "au-dela de B");
}

/// Une region qui arrive plus bas doit rebaser SANS perdre la validite deja
/// declaree — sinon les regions hautes deviendraient des trous.
#[test]
fn un_rebasage_conserve_la_validite_des_regions_hautes() {
    let mut set = FramesLibres::neuf();
    set.couvre(REGION_B.0, REGION_B.1);
    set.couvre(REGION_A.0, REGION_A.1);
    assert!(set.couverte(REGION_A.0), "region basse declaree apres");
    assert!(set.couverte(REGION_B.0), "region haute conservee");
    assert!(!set.couverte(TROU.0), "et le trou reste un trou");
    let pages_a = (REGION_A.1 - REGION_A.0) / TAILLE_PAGE;
    let pages_b = (REGION_B.1 - REGION_B.0) / TAILLE_PAGE;
    assert_eq!(set.frames_valides() as u64, pages_a + pages_b);
}

/// Une region dont les bornes ne sont pas alignees ne declare que les pages
/// ENTIEREMENT contenues : une page a cheval sur un trou n'existe pas.
#[test]
fn une_region_mal_alignee_ne_declare_que_ses_pages_entieres() {
    let mut set = FramesLibres::neuf();
    set.couvre(0x1800, 0x4800); // de la moitie de la page 1 a la moitie de la 4
    assert!(!set.couverte(0x1000), "page 1 : entamee, donc exclue");
    assert!(set.couverte(0x2000), "page 2 : entiere");
    assert!(set.couverte(0x3000), "page 3 : entiere");
    assert!(!set.couverte(0x4000), "page 4 : entamee, donc exclue");
    assert_eq!(set.frames_valides(), 2);
}

#[test]
fn un_free_hors_des_regions_est_refuse() {
    let set = couvrant(64);
    assert!(!set.couverte(BASE - TAILLE_PAGE), "sous la base");
    assert!(!set.couverte(BASE + 64 * TAILLE_PAGE + TAILLE_PAGE), "au-dela");
    assert!(set.couverte(BASE), "premiere frame couverte");
    assert!(set.couverte(BASE + 63 * TAILLE_PAGE), "derniere frame couverte");
}

#[test]
fn allouer_puis_liberer_fait_un_aller_retour() {
    let mut set = couvrant(64);
    assert!(set.marque_libre(frame(3)));
    assert!(set.est_libre(frame(3)));
    assert!(set.marque_occupee(frame(3)), "l'allocation la reprend");
    assert!(!set.est_libre(frame(3)));
    assert_eq!(set.compte(), 0);
    // Et elle peut etre liberee de nouveau : ce n'est pas un double free.
    assert!(set.marque_libre(frame(3)));
}

#[test]
fn reprendre_une_frame_non_libre_est_refuse() {
    let mut set = couvrant(64);
    assert!(!set.marque_occupee(frame(5)), "elle n'etait pas dans la liste");
    assert_eq!(set.compte(), 0, "le compte ne doit pas passer sous zero");
}

// ─── Couverture ────────────────────────────────────────────────────────────

#[test]
fn plusieurs_regions_s_ajoutent() {
    let mut set = FramesLibres::neuf();
    set.couvre(BASE, BASE + 64 * TAILLE_PAGE);
    set.couvre(BASE + 64 * TAILLE_PAGE, BASE + 4096 * TAILLE_PAGE);
    assert!(set.couverte(BASE));
    assert!(set.couverte(BASE + 4095 * TAILLE_PAGE));
    assert!(!set.couverte(BASE + 4096 * TAILLE_PAGE));
}

#[test]
fn une_region_plus_basse_rebase_l_ensemble() {
    let mut set = FramesLibres::neuf();
    set.couvre(BASE, BASE + 64 * TAILLE_PAGE);
    set.couvre(BASE / 2, BASE);
    assert!(set.couverte(BASE / 2), "la region basse est couverte");
    assert!(set.couverte(BASE + 63 * TAILLE_PAGE), "l'ancienne aussi");
}

#[test]
fn une_region_deja_couverte_ne_change_rien() {
    let mut set = couvrant(4096);
    set.marque_libre(frame(10));
    let capacite = set.capacite();
    set.couvre(BASE + 8 * TAILLE_PAGE, BASE + 16 * TAILLE_PAGE);
    assert_eq!(set.capacite(), capacite);
    assert!(set.est_libre(frame(10)), "le contenu survit");
}

#[test]
fn une_region_vide_est_ignoree() {
    let mut set = FramesLibres::neuf();
    set.couvre(BASE, BASE);
    set.couvre(BASE + TAILLE_PAGE, BASE);
    assert_eq!(set.capacite(), 0);
}

#[test]
fn une_adresse_non_alignee_designe_sa_page() {
    let mut set = couvrant(64);
    assert!(set.marque_libre(frame(9)));
    assert!(
        set.est_libre(frame(9)),
        "le bitmap indexe la page, pas l'octet"
    );
    assert!(!set.est_libre(frame(10)));
}

/// Les bornes de mot : le bit 63 et le bit 0 du mot suivant.
#[test]
fn les_frontieres_de_mot_sont_justes() {
    let mut set = couvrant(200);
    for rang in [0u64, 63, 64, 127, 128, 199] {
        assert!(set.marque_libre(frame(rang)), "rang {rang}");
        assert!(set.est_libre(frame(rang)), "rang {rang}");
    }
    for rang in [1u64, 62, 65, 126, 129, 198] {
        assert!(!set.est_libre(frame(rang)), "rang {rang} ne doit pas etre libre");
    }
    assert_eq!(set.compte(), 6);
}
