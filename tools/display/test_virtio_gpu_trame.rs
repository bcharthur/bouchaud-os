//! Une trame, une soumission : aucun pixel endommage oublie, moins de
//! commandes (BOUCHAUD_VIRTIO_GPU_TRAME_V1).

#[path = "../../src/drivers/display/virtio_gpu_tampons.rs"]
mod virtio_gpu_tampons;
#[path = "../../src/drivers/display/virtio_gpu_trame.rs"]
mod virtio_gpu_trame;

use virtio_gpu_tampons::Rect;
use virtio_gpu_trame::{couvre, planifie, TRANSFERTS_MAX};

const COUT: u64 = 32_768;

fn r(x: u32, y: u32, largeur: u32, hauteur: u32) -> Rect {
    Rect { x, y, largeur, hauteur }
}

#[test]
fn une_trame_vide_n_envoie_rien() {
    let p = planifie(&[], COUT);
    assert_eq!(p.commandes(), 0);
    let p = planifie(&[Rect::vide()], COUT);
    assert_eq!(p.commandes(), 0);
}

#[test]
fn une_region_un_transfert_un_flush() {
    let barre = r(0, 0, 1280, 30);
    let p = planifie(&[barre], COUT);
    assert_eq!(p.transferts(), &[barre]);
    assert_eq!(p.flush, barre);
    assert_eq!(p.commandes(), 2);
}

#[test]
fn deux_regions_eloignees_restent_separees_mais_un_seul_flush() {
    // La barre du haut et la barre des taches : fusionner transfererait tout
    // l'ecran.
    let haut = r(0, 0, 1280, 30);
    let bas = r(0, 690, 1280, 30);
    let p = planifie(&[haut, bas], COUT);
    assert_eq!(p.nb, 2);
    assert_eq!(p.flush, r(0, 0, 1280, 720));
    assert_eq!(p.commandes(), 3, "historique : 4 commandes");
    assert!(couvre(&p, &[haut, bas]));
}

#[test]
fn deux_regions_voisines_fusionnent() {
    // Deux tuiles contigues : l'union n'ajoute rien.
    let a = r(100, 100, 64, 64);
    let b = r(164, 100, 64, 64);
    let p = planifie(&[a, b], COUT);
    assert_eq!(p.transferts(), &[r(100, 100, 128, 64)]);
    assert_eq!(p.commandes(), 2);
}

#[test]
fn une_fusion_ne_coute_jamais_plus_qu_une_commande() {
    // Deux tuiles a 100 pixels l'une de l'autre : surcout 64x100 = 6 400 px.
    let a = r(100, 100, 64, 64);
    let b = r(264, 100, 64, 64);
    assert_eq!(planifie(&[a, b], 6_399).nb, 2);
    assert_eq!(planifie(&[a, b], 6_400).nb, 1);
}

#[test]
fn au_plus_huit_transferts_et_rien_d_oublie() {
    // Vingt tuiles eparses, aucune fusion rentable : bornees a 8 transferts.
    let regions: Vec<Rect> = (0..20).map(|i| r((i % 5) * 250, (i / 5) * 170, 16, 16)).collect();
    let p = planifie(&regions, 0);
    assert!(p.nb <= TRANSFERTS_MAX);
    assert!(couvre(&p, &regions));
    assert_eq!(p.flush, regions.iter().fold(Rect::vide(), |u, r| u.union(*r)));
}

#[test]
fn chaque_pixel_endommage_est_transfere_quelle_que_soit_l_entree() {
    // Generateur deterministe : 500 trames de 1 a 30 regions.
    let mut graine: u64 = 0x2545_F491_4F6C_DD1D;
    let mut suivant = |n: u32| {
        graine ^= graine << 13;
        graine ^= graine >> 7;
        graine ^= graine << 17;
        (graine % n as u64) as u32
    };
    for _ in 0..500 {
        let n = 1 + suivant(30) as usize;
        let regions: Vec<Rect> = (0..n)
            .map(|_| {
                let (x, y) = (suivant(1200), suivant(700));
                r(x, y, 1 + suivant(1280 - x), 1 + suivant(720 - y))
            })
            .collect();
        for cout in [0, 4_096, 32_768, 1_000_000] {
            let p = planifie(&regions, cout);
            assert!(couvre(&p, &regions), "region oubliee (cout {cout})");
            assert!(p.nb <= TRANSFERTS_MAX);
            assert!(p.commandes() <= 2 * n, "jamais plus de commandes qu'avant");
            assert_eq!(p.flush, regions.iter().fold(Rect::vide(), |u, r| u.union(*r)));
        }
    }
}
