//! Preuve hote de l'allocateur compagnon.
//!
//! Un allocateur de memoire a une facon particuliere d'etre faux : il rend une
//! adresse. L'adresse a l'air bonne. Elle recouvre celle d'un autre, ou elle
//! designe un bloc dont la seconde moitie n'est pas de la memoire, et cela se
//! manifeste ailleurs, plus tard, sous la forme d'une corruption qu'on
//! attribue au sous-systeme ou elle apparait.
//!
//! Le module de production ne touche donc pas la memoire physique : il
//! manipule des numeros de blocs et delegue le chainage a un `Terrain`. On lui
//! en donne un fait de `Vec`, et on verifie ce qu'aucune execution ne montre :
//! que deux allocations ne se recouvrent JAMAIS, que tout ce qui peut
//! fusionner fusionne, et qu'une zone dont la taille n'est pas une puissance
//! de deux ne produit pas un bloc a moitie imaginaire.

#![allow(dead_code)]

#[path = "../../src/kernel/memory/compagnon.rs"]
mod compagnon;

use compagnon::*;

/// Un terrain : un mot de chainage par bloc.
struct Terrasse {
    liens: Vec<usize>,
    /// Blocs dont le lien a ete ecrit alors qu'ils etaient ALLOUES.
    ecritures_interdites: usize,
    alloues: Vec<bool>,
}

impl Terrasse {
    fn neuve(blocs: usize) -> Self {
        Self {
            liens: vec![AUCUN; blocs],
            ecritures_interdites: 0,
            alloues: vec![false; blocs],
        }
    }
}

impl Terrain for Terrasse {
    fn lien(&self, bloc: usize) -> usize {
        self.liens[bloc]
    }
    fn pose_lien(&mut self, bloc: usize, suivant: usize) {
        // Ecrire dans un bloc alloue serait une corruption : le contrat dit
        // que l'allocateur ne touche QUE des blocs libres.
        if self.alloues[bloc] {
            self.ecritures_interdites += 1;
        }
        self.liens[bloc] = suivant;
    }
}

/// Fabrique un allocateur couvrant `blocs` blocs, entierement libre.
fn zone(blocs: usize) -> (Vec<u64>, Terrasse, Vec<(usize, usize)>) {
    let bitmap = vec![0u64; mots_bitmap(blocs)];
    let terrain = Terrasse::neuve(blocs);
    // On rend la zone par blocs du plus grand ordre possible, comme le fera
    // l'alimentation au demarrage.
    let mut morceaux = Vec::new();
    let mut position = 0usize;
    while position < blocs {
        let mut ordre = ORDRES - 1;
        loop {
            let taille = 1usize << ordre;
            if aligne(position, ordre) && position + taille <= blocs {
                break;
            }
            if ordre == 0 {
                break;
            }
            ordre -= 1;
        }
        morceaux.push((position, ordre));
        position += 1usize << ordre;
    }
    (bitmap, terrain, morceaux)
}

fn alimente(blocs: usize) -> (Vec<u64>, Terrasse, Vec<(usize, usize)>) {
    zone(blocs)
}

// ---------------------------------------------------------------------------
// L'arithmetique des jumeaux
// ---------------------------------------------------------------------------

#[test]
fn le_jumeau_est_un_ou_exclusif() {
    // Toute l'astuce : trouver le compagnon d'un bloc ne demande ni table, ni
    // recherche, ni parcours.
    assert_eq!(jumeau(0, 0), 1);
    assert_eq!(jumeau(1, 0), 0);
    assert_eq!(jumeau(0, 1), 2);
    assert_eq!(jumeau(2, 1), 0);
    assert_eq!(jumeau(4, 2), 0);
    assert_eq!(jumeau(8, 3), 0);
    // Le jumeau du jumeau est le bloc de depart, a tous les ordres.
    for ordre in 0..ORDRES {
        for bloc in [0usize, 1 << ordre, 3 << ordre, 100 << ordre] {
            assert_eq!(jumeau(jumeau(bloc, ordre), ordre), bloc);
        }
    }
}

#[test]
fn le_pere_est_la_moitie_basse() {
    // Le pere est la moitie BASSE de la paire : le bloc 2 d'ordre un couvre
    // 2..3, son jumeau est 0, et le bloc d'ordre deux qui les contient
    // commence donc a zero.
    assert_eq!(pere(0, 0), 0);
    assert_eq!(pere(1, 0), 0);
    assert_eq!(pere(2, 1), 0);
    assert_eq!(pere(3, 0), 2);
    assert_eq!(pere(6, 1), 4);
    assert_eq!(pere(4, 2), 0, "4..7 et 0..3 sont jumeaux d'ordre deux");
    assert_eq!(pere(7, 0), 6);
    // Un bloc deja aligne a l'ordre superieur est son propre pere.
    assert_eq!(pere(4, 1), 4);
    assert_eq!(pere(8, 2), 8);
}

#[test]
fn un_bloc_est_aligne_sur_son_ordre() {
    assert!(aligne(0, 5));
    assert!(aligne(32, 5));
    assert!(!aligne(16, 5));
    assert!(aligne(1, 0), "tout bloc est aligne a l'ordre zero");
}

#[test]
fn l_ordre_couvre_la_demande() {
    assert_eq!(ordre_pour(0), 0);
    assert_eq!(ordre_pour(1), 0);
    assert_eq!(ordre_pour(2), 1);
    assert_eq!(ordre_pour(3), 2, "trois blocs demandent un bloc de quatre");
    assert_eq!(ordre_pour(4), 2);
    assert_eq!(ordre_pour(5), 3);
    assert_eq!(ordre_pour(1024), 10);
}

// ---------------------------------------------------------------------------
// Allouer, rendre, fusionner
// ---------------------------------------------------------------------------

#[test]
fn une_zone_entierement_rendue_offre_son_plus_grand_ordre() {
    let blocs = 1024;
    let (mut bitmap, mut terrain, morceaux) = alimente(blocs);
    let mut a = Compagnon::neuf(blocs, &mut bitmap).unwrap();
    for (bloc, ordre) in morceaux {
        assert!(a.rend(&mut terrain, bloc, ordre));
    }
    assert_eq!(a.disponibles(), blocs);
    assert_eq!(
        a.plus_grand_ordre(),
        Some(10),
        "mille vingt-quatre blocs doivent former UN bloc d'ordre dix"
    );
    assert_eq!(a.compte_a_l_ordre(&terrain, 10), 1);
}

#[test]
fn rendre_deux_jumeaux_les_fusionne() {
    let blocs = 64;
    let (mut bitmap, mut terrain, _) = alimente(blocs);
    let mut a = Compagnon::neuf(blocs, &mut bitmap).unwrap();
    assert!(a.rend(&mut terrain, 0, 0));
    assert_eq!(a.plus_grand_ordre(), Some(0));
    assert!(a.rend(&mut terrain, 1, 0));
    assert_eq!(
        a.plus_grand_ordre(),
        Some(1),
        "deux blocs d'ordre zero jumeaux forment un bloc d'ordre un"
    );
    assert_eq!(a.compte_a_l_ordre(&terrain, 0), 0);
    assert_eq!(a.compte_a_l_ordre(&terrain, 1), 1);
    assert_eq!(a.statistiques().fusions, 1);
}

#[test]
fn deux_blocs_voisins_qui_ne_sont_pas_jumeaux_ne_fusionnent_pas() {
    // 1 et 2 sont voisins et ne sont PAS jumeaux : le jumeau de 1 est 0. Les
    // fusionner produirait un bloc d'ordre un a une adresse non alignee.
    let blocs = 64;
    let (mut bitmap, mut terrain, _) = alimente(blocs);
    let mut a = Compagnon::neuf(blocs, &mut bitmap).unwrap();
    a.rend(&mut terrain, 1, 0);
    a.rend(&mut terrain, 2, 0);
    assert_eq!(a.plus_grand_ordre(), Some(0));
    assert_eq!(a.compte_a_l_ordre(&terrain, 0), 2);
    assert_eq!(a.statistiques().fusions, 0);
}

#[test]
fn la_fusion_remonte_en_cascade() {
    // Rendre 0,1,2,3 dans cet ordre doit produire UN bloc d'ordre deux, pas
    // deux blocs d'ordre un.
    let blocs = 64;
    let (mut bitmap, mut terrain, _) = alimente(blocs);
    let mut a = Compagnon::neuf(blocs, &mut bitmap).unwrap();
    for bloc in 0..4 {
        a.rend(&mut terrain, bloc, 0);
    }
    assert_eq!(a.plus_grand_ordre(), Some(2));
    assert_eq!(a.compte_a_l_ordre(&terrain, 2), 1);
    assert_eq!(a.compte_a_l_ordre(&terrain, 1), 0);
    assert_eq!(a.compte_a_l_ordre(&terrain, 0), 0);
    assert_eq!(a.statistiques().fusions, 3);
}

#[test]
fn prendre_divise_ce_qu_il_faut_et_pas_plus() {
    let blocs = 1024;
    let (mut bitmap, mut terrain, morceaux) = alimente(blocs);
    let mut a = Compagnon::neuf(blocs, &mut bitmap).unwrap();
    for (bloc, ordre) in morceaux {
        a.rend(&mut terrain, bloc, ordre);
    }
    // Une seule page depuis un bloc de 1024 : dix divisions, pas plus.
    let bloc = a.prend(&mut terrain, 0).unwrap();
    assert_eq!(bloc, 0);
    assert_eq!(a.statistiques().divisions, 10);
    assert_eq!(a.disponibles(), blocs - 1);
    // Et il reste un bloc libre a chaque ordre de 0 a 9.
    for ordre in 0..10 {
        assert_eq!(
            a.compte_a_l_ordre(&terrain, ordre),
            1,
            "ordre {ordre} : la division doit avoir laisse la moitie haute"
        );
    }
}

#[test]
fn rendre_ce_qu_on_a_pris_restaure_l_etat_de_depart() {
    let blocs = 256;
    let (mut bitmap, mut terrain, morceaux) = alimente(blocs);
    let mut a = Compagnon::neuf(blocs, &mut bitmap).unwrap();
    for (bloc, ordre) in morceaux {
        a.rend(&mut terrain, bloc, ordre);
    }
    let avant = a.plus_grand_ordre();
    let mut pris = Vec::new();
    for ordre in [0usize, 3, 1, 5, 0, 2] {
        pris.push((a.prend(&mut terrain, ordre).unwrap(), ordre));
    }
    assert!(a.disponibles() < blocs);
    for (bloc, ordre) in pris {
        assert!(a.rend(&mut terrain, bloc, ordre), "bloc {bloc} ordre {ordre}");
    }
    assert_eq!(a.disponibles(), blocs, "toute la memoire doit etre revenue");
    assert_eq!(
        a.plus_grand_ordre(),
        avant,
        "la fragmentation doit avoir disparu : c'est tout l'interet de la fusion"
    );
}

#[test]
fn deux_allocations_ne_se_recouvrent_jamais() {
    // Le defaut qui compte : deux adresses valides qui designent la meme
    // memoire. Il ne se manifeste pas a l'allocation ; il se manifeste
    // ailleurs, plus tard.
    let blocs = 512;
    let (mut bitmap, mut terrain, morceaux) = alimente(blocs);
    let mut a = Compagnon::neuf(blocs, &mut bitmap).unwrap();
    for (bloc, ordre) in morceaux {
        a.rend(&mut terrain, bloc, ordre);
    }
    let mut occupe = vec![false; blocs];
    let mut pris = Vec::new();
    // Une suite d'ordres qui force divisions et reutilisations.
    let ordres = [0usize, 1, 0, 2, 3, 0, 1, 4, 0, 2, 5, 1, 0, 3, 0, 0];
    for ordre in ordres {
        let Some(bloc) = a.prend(&mut terrain, ordre) else { continue };
        assert!(aligne(bloc, ordre), "bloc {bloc} mal aligne pour l'ordre {ordre}");
        for i in 0..(1usize << ordre) {
            assert!(
                !occupe[bloc + i],
                "le bloc {} est rendu deux fois (allocation ordre {ordre} en {bloc})",
                bloc + i
            );
            occupe[bloc + i] = true;
        }
        terrain.alloues[bloc] = true;
        pris.push((bloc, ordre));
    }
    assert!(pris.len() >= 10);
    assert_eq!(
        terrain.ecritures_interdites, 0,
        "l'allocateur a ecrit dans un bloc alloue : c'est une corruption"
    );
    // Tout rendre, et retrouver la zone entiere.
    for (bloc, ordre) in pris {
        terrain.alloues[bloc] = false;
        assert!(a.rend(&mut terrain, bloc, ordre));
    }
    assert_eq!(a.disponibles(), blocs);
}

// ---------------------------------------------------------------------------
// Les cas de bord
// ---------------------------------------------------------------------------

#[test]
fn une_zone_qui_n_est_pas_une_puissance_de_deux_ne_fabrique_pas_de_bloc_imaginaire() {
    // 100 blocs. Le jumeau du bloc 96 a l'ordre 5 est 64 ; celui du bloc 96 a
    // l'ordre 2 est 100, qui n'existe pas. Fusionner rendrait un bloc dont la
    // seconde moitie n'est pas de la memoire.
    let blocs = 100;
    let (mut bitmap, mut terrain, morceaux) = alimente(blocs);
    let mut a = Compagnon::neuf(blocs, &mut bitmap).unwrap();
    for (bloc, ordre) in morceaux {
        assert!(a.rend(&mut terrain, bloc, ordre), "bloc {bloc} ordre {ordre}");
    }
    assert_eq!(a.disponibles(), blocs);
    // Toute allocation doit rester dans la zone.
    for ordre in [0usize, 1, 2, 3, 4, 5, 6] {
        if let Some(bloc) = a.prend(&mut terrain, ordre) {
            assert!(
                bloc + (1usize << ordre) <= blocs,
                "l'ordre {ordre} a rendu le bloc {bloc}, qui deborde de la zone"
            );
            a.rend(&mut terrain, bloc, ordre);
        }
    }
}

#[test]
fn une_double_liberation_est_refusee_sans_parcourir() {
    let blocs = 64;
    let (mut bitmap, mut terrain, _) = alimente(blocs);
    let mut a = Compagnon::neuf(blocs, &mut bitmap).unwrap();
    assert!(a.rend(&mut terrain, 4, 0));
    assert!(
        !a.rend(&mut terrain, 4, 0),
        "une double liberation corrompt les listes ; le bitmap la voit"
    );
    assert_eq!(a.disponibles(), 1, "la seconde ne doit rien avoir ajoute");
}

#[test]
fn un_bloc_mal_aligne_est_refuse() {
    let blocs = 64;
    let (mut bitmap, mut terrain, _) = alimente(blocs);
    let mut a = Compagnon::neuf(blocs, &mut bitmap).unwrap();
    assert!(
        !a.rend(&mut terrain, 1, 1),
        "le bloc 1 n'est pas un bloc d'ordre un ; le rendre fusionnerait avec \
         un jumeau qui n'existe pas"
    );
    assert!(!a.rend(&mut terrain, 3, 2));
    assert!(a.rend(&mut terrain, 4, 2));
}

#[test]
fn un_bloc_hors_zone_est_refuse() {
    let blocs = 64;
    let (mut bitmap, mut terrain, _) = alimente(blocs);
    let mut a = Compagnon::neuf(blocs, &mut bitmap).unwrap();
    assert!(!a.rend(&mut terrain, 64, 0));
    assert!(!a.rend(&mut terrain, 1000, 0));
    // Un bloc dont la TETE est dans la zone et la queue non.
    assert!(!a.rend(&mut terrain, 32, 6), "32 + 64 deborde de 64 blocs");
}

#[test]
fn un_ordre_hors_bornes_est_refuse() {
    let blocs = 64;
    let (mut bitmap, mut terrain, _) = alimente(blocs);
    let mut a = Compagnon::neuf(blocs, &mut bitmap).unwrap();
    assert!(!a.rend(&mut terrain, 0, ORDRES));
    assert_eq!(a.prend(&mut terrain, ORDRES), None);
}

#[test]
fn un_bitmap_trop_court_est_refuse_plutot_qu_utilise() {
    // Accepter un bitmap court ferait lire l'etat d'un bloc hors zone : une
    // reponse plausible et fausse.
    let mut court = vec![0u64; 1];
    assert!(Compagnon::neuf(10_000, &mut court).is_none());
    let mut vide: Vec<u64> = Vec::new();
    assert!(Compagnon::neuf(0, &mut vide).is_none());
    let mut juste = vec![0u64; mots_bitmap(64)];
    assert!(Compagnon::neuf(64, &mut juste).is_some());
}

#[test]
fn un_allocateur_vide_echoue_proprement() {
    let blocs = 64;
    let (mut bitmap, mut terrain, _) = alimente(blocs);
    let mut a = Compagnon::neuf(blocs, &mut bitmap).unwrap();
    assert_eq!(a.prend(&mut terrain, 0), None);
    assert_eq!(a.statistiques().echecs, 1);
    assert_eq!(a.plus_grand_ordre(), None);
}

#[test]
fn une_demande_trop_grande_pour_ce_qui_reste_echoue() {
    let blocs = 64;
    let (mut bitmap, mut terrain, _) = alimente(blocs);
    let mut a = Compagnon::neuf(blocs, &mut bitmap).unwrap();
    a.rend(&mut terrain, 0, 0);
    a.rend(&mut terrain, 1, 0);
    // Deux pages fusionnees : un bloc d'ordre un. On ne peut pas en servir
    // quatre.
    assert_eq!(a.prend(&mut terrain, 2), None);
    assert!(a.prend(&mut terrain, 1).is_some());
}

#[test]
fn le_bitmap_dit_la_verite_apres_une_longue_serie() {
    // Une serie qui alterne prises et rendus doit laisser un etat exact.
    let blocs = 256;
    let (mut bitmap, mut terrain, morceaux) = alimente(blocs);
    let mut a = Compagnon::neuf(blocs, &mut bitmap).unwrap();
    for (bloc, ordre) in morceaux {
        a.rend(&mut terrain, bloc, ordre);
    }
    let mut tenus: Vec<(usize, usize)> = Vec::new();
    let mut graine = 0x243F_6A88u32;
    for tour in 0..2000 {
        graine = graine.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let ordre = ((graine >> 16) % 5) as usize;
        if tour % 3 != 2 || tenus.is_empty() {
            if let Some(bloc) = a.prend(&mut terrain, ordre) {
                tenus.push((bloc, ordre));
            }
        } else {
            let index = (graine as usize) % tenus.len();
            let (bloc, ordre) = tenus.swap_remove(index);
            assert!(a.rend(&mut terrain, bloc, ordre));
        }
    }
    let tenu: usize = tenus.iter().map(|(_, o)| 1usize << o).sum();
    assert_eq!(
        a.disponibles() + tenu,
        blocs,
        "la somme du libre et du tenu doit rester la zone entiere"
    );
    for (bloc, ordre) in tenus {
        a.rend(&mut terrain, bloc, ordre);
    }
    assert_eq!(a.disponibles(), blocs);
    assert_eq!(
        a.plus_grand_ordre(),
        Some(8),
        "256 blocs, tous rendus, doivent reformer UN bloc d'ordre huit : \
         c'est la propriete que la fusion existe pour tenir"
    );
}

#[test]
fn les_statistiques_comptent_ce_qui_est_arrive() {
    let blocs = 64;
    let (mut bitmap, mut terrain, morceaux) = alimente(blocs);
    let mut a = Compagnon::neuf(blocs, &mut bitmap).unwrap();
    for (bloc, ordre) in morceaux {
        a.rend(&mut terrain, bloc, ordre);
    }
    let depart = a.statistiques();
    a.prend(&mut terrain, 0).unwrap();
    let apres = a.statistiques();
    assert_eq!(apres.allocations, depart.allocations + 1);
    assert!(apres.divisions > depart.divisions);
    assert_eq!(apres.blocs, blocs);
}
