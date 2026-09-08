//! Preuve hote du depot de magasins du tas noyau.
//!
//! Le module de production `src/kernel/memory/magasin.rs` est inclus tel quel :
//! ce qui est mis a l'epreuve ici est le code qui tourne dans le noyau.
//!
//! # Ce que le depot doit garantir
//!
//! Il s'intercale entre les listes libres per-CPU et l'allocateur backing
//! global. La seule chose qu'il ne doit JAMAIS faire est de perdre un bloc :
//! un magasin detache d'une liste et refuse par un depot plein est une fuite
//! silencieuse, proportionnelle au trafic.
//!
//!   1. detacher puis rendre un magasin conserve exactement les memes blocs ;
//!   2. un depot plein REFUSE, il n'ecrase pas -- l'appelant doit rendre les
//!      blocs au backing, et il ne peut le faire que si on les lui rend ;
//!   3. sous contention SMP, aucun bloc n'est duplique ni perdu ;
//!   4. les compteurs disent la verite : `servis` + `vides` = demandes.

#[path = "../../src/kernel/memory/magasin.rs"]
mod magasin;

use magasin::{detache, lien_plausible, longueur_chaine, Depot, Magasin, LOT, MAGASINS_MAX};
use std::collections::HashSet;
use std::sync::Arc;

/// Une arene de blocs, comme la classe de taille d'un tas.
///
/// Les blocs sont de vraies adresses : le module de production ecrit le lien
/// dans leur premier mot, exactement comme dans le noyau.
struct Arene {
    _memoire: Vec<u64>,
    base: usize,
    taille_bloc: usize,
    blocs: usize,
}

impl Arene {
    fn neuve(blocs: usize) -> Self {
        let taille_bloc = 32usize;
        let mots = blocs * taille_bloc / 8;
        let mut memoire = vec![0u64; mots];
        let base = memoire.as_mut_ptr() as usize;
        Self { _memoire: memoire, base, taille_bloc, blocs }
    }

    fn bloc(&self, indice: usize) -> usize {
        assert!(indice < self.blocs);
        self.base + indice * self.taille_bloc
    }

    /// Enchaine `n` blocs a partir de `depart` et rend la tete.
    fn chaine(&self, depart: usize, n: usize) -> usize {
        assert!(depart + n <= self.blocs);
        for i in 0..n {
            let suivant = if i + 1 == n { 0 } else { self.bloc(depart + i + 1) };
            unsafe { magasin::lien_ecrit(self.bloc(depart + i), suivant) };
        }
        self.bloc(depart)
    }

    fn adresses(&self, tete: usize) -> Vec<usize> {
        let mut vues = Vec::new();
        let mut courant = tete;
        while courant != 0 {
            vues.push(courant);
            courant = unsafe { magasin::lien_lit(courant) };
        }
        vues
    }
}

// ---------------------------------------------------------------------------
// 1 : detacher ne perd rien.
// ---------------------------------------------------------------------------

#[test]
fn detacher_conserve_exactement_les_blocs() {
    let arene = Arene::neuve(64);
    let tete = arene.chaine(0, 64);
    let attendus: HashSet<usize> = arene.adresses(tete).into_iter().collect();
    assert_eq!(attendus.len(), 64);

    let (magasin, reste) = unsafe { detache(tete, LOT) };
    assert_eq!(magasin.compte, LOT);

    let mut obtenus: HashSet<usize> = arene.adresses(magasin.tete).into_iter().collect();
    assert_eq!(obtenus.len(), LOT, "le magasin doit etre termine par un lien nul");
    obtenus.extend(arene.adresses(reste));

    assert_eq!(obtenus, attendus, "aucun bloc ne doit disparaitre ni apparaitre");
}

#[test]
fn detacher_une_chaine_plus_courte_que_le_lot_prend_tout() {
    let arene = Arene::neuve(8);
    let tete = arene.chaine(0, 5);
    let (magasin, reste) = unsafe { detache(tete, LOT) };
    assert_eq!(magasin.compte, 5);
    assert_eq!(reste, 0);
    assert_eq!(unsafe { longueur_chaine(magasin.tete, 64) }, 5);
}

#[test]
fn detacher_une_chaine_vide_ne_panique_pas() {
    let (magasin, reste) = unsafe { detache(0, LOT) };
    assert_eq!(magasin.compte, 0);
    assert_eq!(reste, 0);
}

// ---------------------------------------------------------------------------
// 2 : un depot plein refuse au lieu d'ecraser.
// ---------------------------------------------------------------------------

#[test]
fn un_depot_plein_refuse_et_ne_perd_aucun_bloc() {
    let arene = Arene::neuve((MAGASINS_MAX + 2) * LOT);
    let depot = Depot::neuf();

    let mut refuses = 0usize;
    for magasin_index in 0..(MAGASINS_MAX + 2) {
        let tete = arene.chaine(magasin_index * LOT, LOT);
        if !depot.depose(Magasin { tete, compte: LOT }) {
            refuses += 1;
            // Le refus doit laisser la chaine INTACTE : c'est ce qui permet a
            // l'appelant de la rendre au backing.
            assert_eq!(unsafe { longueur_chaine(tete, 64) }, LOT);
        }
    }
    assert_eq!(refuses, 2, "les deux magasins de trop doivent etre refuses");
    assert_eq!(depot.longueur(), MAGASINS_MAX);

    let c = depot.compteurs();
    assert_eq!(c.deposes, MAGASINS_MAX as u64);
    assert_eq!(c.pleins, 2);
    assert_eq!(c.pic, MAGASINS_MAX as u64);
}

#[test]
fn un_depot_vide_le_dit_au_lieu_d_inventer() {
    let depot = Depot::neuf();
    assert_eq!(depot.retire(), None);
    assert_eq!(depot.retire(), None);
    let c = depot.compteurs();
    assert_eq!(c.vides, 2, "les descentes vers le backing doivent etre comptees");
    assert_eq!(c.servis, 0);
}

#[test]
fn un_magasin_vide_est_accepte_sans_occuper_de_place() {
    let depot = Depot::neuf();
    assert!(depot.depose(Magasin { tete: 0, compte: 0 }));
    assert_eq!(depot.longueur(), 0, "un magasin vide n'occupe pas une place");
    assert_eq!(depot.retire(), None);
}

// ---------------------------------------------------------------------------
// 3 : stress SMP -- ni duplication ni fuite.
// ---------------------------------------------------------------------------

/// Plusieurs « CPU » deposent et retirent en meme temps. A la fin, chaque bloc
/// doit se retrouver EXACTEMENT une fois -- soit dans le depot, soit dans la
/// main d'un fil.
#[test]
fn aucun_bloc_n_est_duplique_ni_perdu_sous_contention() {
    const FILS: usize = 4;
    const MAGASINS_PAR_FIL: usize = 24;

    let arene = Arc::new(Arene::neuve(FILS * MAGASINS_PAR_FIL * LOT));
    let depot = Arc::new(Depot::neuf());

    let mut fils = Vec::new();
    for fil in 0..FILS {
        let arene = Arc::clone(&arene);
        let depot = Arc::clone(&depot);
        fils.push(std::thread::spawn(move || {
            let mut en_main: Vec<usize> = Vec::new();
            for magasin_index in 0..MAGASINS_PAR_FIL {
                let depart = (fil * MAGASINS_PAR_FIL + magasin_index) * LOT;
                let tete = arene.chaine(depart, LOT);
                if !depot.depose(Magasin { tete, compte: LOT }) {
                    en_main.push(tete);
                }
                // Et on reprend, comme un CPU dont la liste se vide.
                if let Some(pris) = depot.retire() {
                    en_main.push(pris.tete);
                }
            }
            en_main
        }));
    }

    let mut toutes: Vec<usize> = Vec::new();
    for f in fils {
        toutes.extend(f.join().unwrap());
    }
    while let Some(magasin) = depot.retire() {
        toutes.push(magasin.tete);
    }

    let mut blocs: Vec<usize> = Vec::new();
    for tete in toutes {
        blocs.extend(arene.adresses(tete));
    }
    let uniques: HashSet<usize> = blocs.iter().copied().collect();
    assert_eq!(
        uniques.len(), blocs.len(),
        "un bloc apparait dans deux magasins : il serait alloue deux fois"
    );
    assert_eq!(
        blocs.len(), FILS * MAGASINS_PAR_FIL * LOT,
        "des blocs ont disparu : c'est une fuite proportionnelle au trafic"
    );
}

// ---------------------------------------------------------------------------
// 4 : les compteurs disent la verite.
// ---------------------------------------------------------------------------

/// Ce que le depot RETIRE au verrou global : `servis` magasins, c'est
/// `servis * LOT` allocations backing evitees.
#[test]
fn les_compteurs_permettent_de_chiffrer_le_gain() {
    let arene = Arene::neuve(4 * LOT);
    let depot = Depot::neuf();

    for i in 0..4 {
        let tete = arene.chaine(i * LOT, LOT);
        assert!(depot.depose(Magasin { tete, compte: LOT }));
    }
    let mut servis = 0u64;
    while depot.retire().is_some() {
        servis += 1;
    }
    // La derniere tentative echoue : c'est elle qui descendra au backing.
    let c = depot.compteurs();
    assert_eq!(c.servis, servis);
    assert_eq!(c.deposes, 4);
    assert_eq!(c.vides, 1);
    assert_eq!(
        servis * LOT as u64, 64,
        "quatre magasins servis, c'est soixante-quatre descentes evitees"
    );
}

#[test]
fn un_magasin_ressort_tel_qu_il_est_entre() {
    let arene = Arene::neuve(LOT);
    let depot = Depot::neuf();
    let tete = arene.chaine(0, LOT);
    let avant = arene.adresses(tete);

    assert!(depot.depose(Magasin { tete, compte: LOT }));
    let repris = depot.retire().expect("le magasin depose doit ressortir");
    assert_eq!(repris.tete, tete);
    assert_eq!(repris.compte, LOT);
    assert_eq!(arene.adresses(repris.tete), avant);
}

/// Le depot est une PILE : le magasin le plus recent ressort en premier, donc
/// le plus chaud en cache.
#[test]
fn le_magasin_le_plus_chaud_ressort_en_premier() {
    let arene = Arene::neuve(3 * LOT);
    let depot = Depot::neuf();
    let mut tetes = Vec::new();
    for i in 0..3 {
        let tete = arene.chaine(i * LOT, LOT);
        tetes.push(tete);
        assert!(depot.depose(Magasin { tete, compte: LOT }));
    }
    for attendu in tetes.iter().rev() {
        assert_eq!(depot.retire().unwrap().tete, *attendu);
    }
}

// ---------------------------------------------------------------------------
// Validation des liens : ce qui separe une corruption reperee d'une ecriture
// sauvage.
// ---------------------------------------------------------------------------

/// Zero est une FIN DE CHAINE, jamais un bloc. Le confondre avec une adresse
/// faisait ecrire un pointeur du tas a l'adresse nulle.
#[test]
fn le_lien_nul_n_est_jamais_un_bloc() {
    assert!(!lien_plausible(0, 64, 0x1000, 0x9000));
    assert!(!lien_plausible(0, 64, 0, 0));
}

/// Un bloc d'une classe est aligne sur sa taille : l'allocateur le decoupe avec
/// `Layout::from_size_align(taille, taille)`. Une adresse qui ne l'est pas ne
/// vient pas de cette classe, et la suivre lirait a cote.
#[test]
fn un_bloc_est_aligne_sur_sa_classe() {
    assert!(lien_plausible(0x2000, 64, 0x1000, 0x9000));
    assert!(!lien_plausible(0x2001, 64, 0x1000, 0x9000));
    assert!(!lien_plausible(0x2020, 64, 0x1000, 0x9000));
}

/// Un bloc du tas est DANS le tas, et il y tient en entier : un bloc qui
/// commence avant la fin mais deborde n'est pas dans l'arene.
#[test]
fn un_bloc_tient_entierement_dans_l_arene() {
    assert!(lien_plausible(0x1000, 64, 0x1000, 0x9000));
    assert!(lien_plausible(0x8FC0, 64, 0x1000, 0x9000));
    // Commence dans l'arene, se termine apres : refuse.
    assert!(!lien_plausible(0x9000, 64, 0x1000, 0x9000));
    // Avant l'arene.
    assert!(!lien_plausible(0x0FC0, 64, 0x1000, 0x9000));
}

/// Une arene inconnue (`debut == fin`) ne doit pas TOUT refuser : le tas
/// bootstrap fonctionne avant que ses bornes soient enregistrees. Alignement et
/// non-nullite restent verifies.
#[test]
fn une_arene_inconnue_ne_refuse_pas_tout() {
    assert!(lien_plausible(0x2000, 64, 0, 0));
    assert!(!lien_plausible(0x2001, 64, 0, 0));
    assert!(!lien_plausible(0, 64, 0, 0));
}

/// Une taille nulle ne decrit aucune classe : le modulo serait une division
/// par zero, et la question n'a pas de sens.
#[test]
fn une_classe_de_taille_nulle_est_refusee() {
    assert!(!lien_plausible(0x2000, 0, 0x1000, 0x9000));
}

/// Une adresse dont la fin deborde l'espace d'adressage ne peut pas etre un
/// bloc : le calcul de son bout n'a pas de resultat.
#[test]
fn un_bloc_qui_deborde_l_espace_est_refuse() {
    assert!(!lien_plausible(usize::MAX - 8, 64, 0, 0));
}

// ---------------------------------------------------------------------------
// La vidange : ce que le changement d'arene exige du depot.
// ---------------------------------------------------------------------------

/// Le depot se vide COMPLETEMENT par `retire()` repete.
///
/// C'est la propriete sur laquelle `heap::vide_les_caches` repose. Le tas
/// bascule une fois au demarrage de l'arene bootstrap -- un statique de
/// l'image noyau -- vers l'arene physique. Tout bloc encore retenu par le
/// depot a cet instant pointe dans l'ANCIENNE region, et serait rendu apres le
/// changement comme s'il venait de la nouvelle.
///
/// Si `retire()` pouvait laisser un magasin derriere lui, la vidange serait
/// incomplete et le melange des deux arenes resterait possible.
#[test]
fn le_depot_se_vide_entierement() {
    let arene = Arene::neuve(MAGASINS_MAX * LOT);
    let depot = Depot::neuf();
    for i in 0..MAGASINS_MAX {
        let tete = arene.chaine(i * LOT, LOT);
        assert!(depot.depose(Magasin { tete, compte: LOT }));
    }

    let mut blocs = 0usize;
    let mut tours = 0usize;
    while let Some(magasin) = depot.retire() {
        blocs += magasin.compte;
        tours += 1;
        assert!(tours <= MAGASINS_MAX, "la vidange ne se termine pas");
    }
    assert_eq!(tours, MAGASINS_MAX);
    assert_eq!(blocs, MAGASINS_MAX * LOT);
    // Vide, et il le reste.
    assert!(depot.retire().is_none());
    assert!(depot.retire().is_none());
}
