//! BOUCHAUD_GPU_COMPOSITION_V2 : decoupe des fenetres, pages d'une surface,
//! commandes 3D du compositeur GPU.

#[path = "../../src/drivers/display/composition_gpu.rs"]
mod c;

use c::*;

fn aire(v: &[R]) -> u64 {
    v.iter().map(|r| r.aire()).sum()
}

fn disjoints(v: &[R]) -> bool {
    for i in 0..v.len() {
        for j in i + 1..v.len() {
            if v[i].touche(&v[j]) {
                return false;
            }
        }
    }
    true
}

/// Verifie au pixel : chaque pixel de `base` est couvert par exactement un
/// morceau s'il n'est sous aucun trou, par aucun sinon.
fn verifie(base: R, trous: &[R], morceaux: &[R]) {
    for y in base.y..base.bas() {
        for x in base.x..base.droite() {
            let p = R::neuf(x, y, 1, 1);
            let sous_trou = trous.iter().any(|t| t.touche(&p));
            let n = morceaux.iter().filter(|m| m.touche(&p)).count();
            assert_eq!(n, if sous_trou { 0 } else { 1 }, "pixel {x},{y}");
        }
    }
}

#[test]
fn sans_trou_la_fenetre_entiere() {
    let mut s = [R::default(); MORCEAUX_MAX];
    let base = R::neuf(10, 20, 300, 200);
    assert_eq!(soustrait(base, &[], &mut s), Some(1));
    assert_eq!(s[0], base);
}

#[test]
fn le_curseur_au_milieu_donne_quatre_morceaux() {
    let mut s = [R::default(); MORCEAUX_MAX];
    let base = R::neuf(0, 0, 64, 48);
    let curseur = R::neuf(20, 10, 14, 22);
    let n = soustrait(base, &[curseur], &mut s).unwrap();
    assert_eq!(n, 4);
    assert!(disjoints(&s[..n]));
    assert_eq!(aire(&s[..n]), base.aire() - curseur.aire());
    verifie(base, &[curseur], &s[..n]);
}

#[test]
fn fenetres_superieures_et_curseur_au_pixel() {
    let mut s = [R::default(); MORCEAUX_MAX];
    let base = R::neuf(5, 5, 60, 40);
    let trous = [R::neuf(0, 0, 20, 20), R::neuf(50, 30, 30, 30), R::neuf(30, 15, 5, 8)];
    let n = soustrait(base, &trous, &mut s).unwrap();
    assert!(disjoints(&s[..n]));
    verifie(base, &trous, &s[..n]);
}

#[test]
fn entierement_couverte_rien_a_poser() {
    let mut s = [R::default(); MORCEAUX_MAX];
    assert_eq!(soustrait(R::neuf(10, 10, 5, 5), &[R::neuf(0, 0, 100, 100)], &mut s), Some(0));
}

#[test]
fn trop_decoupee_repli_processeur() {
    let mut s = [R::default(); 4];
    let base = R::neuf(0, 0, 100, 100);
    let trous: Vec<R> = (0..6).map(|i| R::neuf(5 + i * 15, 5 + i * 15, 4, 4)).collect();
    assert_eq!(soustrait(base, &trous, &mut s), None);
}

#[test]
fn pages_contigues_fusionnees() {
    let frames = [0x1000u64, 0x2000, 0x3000, 0x9000, 0xA000, 0x5000];
    let mut e = [Entree::default(); 8];
    let n = entrees_memoire(&frames, 6 * 4096 - 100, 4096, &mut e).unwrap();
    assert_eq!(n, 3);
    assert_eq!(e[0], Entree { adresse: 0x1000, longueur: 3 * 4096 });
    assert_eq!(e[1], Entree { adresse: 0x9000, longueur: 2 * 4096 });
    assert_eq!(e[2], Entree { adresse: 0x5000, longueur: 4096 - 100 });
}

#[test]
fn pages_manquantes_ou_desalignees_refusees() {
    let mut e = [Entree::default(); 8];
    assert_eq!(entrees_memoire(&[0x1000], 8192, 4096, &mut e), None);
    assert_eq!(entrees_memoire(&[0x1001], 4096, 4096, &mut e), None);
    assert_eq!(entrees_memoire(&[0], 4096, 4096, &mut e), None);
    let mut petit = [Entree::default(); 1];
    assert_eq!(entrees_memoire(&[0x1000, 0x5000], 8192, 4096, &mut petit), None);
}

fn mot(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap())
}

#[test]
fn texture_origine_en_haut() {
    let q = cree_texture(TEXTURE_BASE, 1278, 648).unwrap();
    assert_eq!(mot(&q, 0), CMD_RESOURCE_CREATE_3D);
    assert_eq!(mot(&q, 6), TEXTURE_BASE);
    assert_eq!((mot(&q, 10), mot(&q, 11)), (1278, 648));
    assert_eq!(mot(&q, 16), DRAPEAU_Y_0_TOP);
    assert!(cree_texture(0, 1, 1).is_none());
}

#[test]
fn transfert_d_un_rectangle_au_bon_decalage() {
    // tampon 1 d'une surface a deux tampons : base = 1 * octets.
    let (l, h, pas) = (640u32, 400u32, 2560u32);
    let base = (pas * h) as u64;
    let q = transfert(TEXTURE_BASE + 1, R::neuf(10, 20, 100, 50), base, pas, l, h).unwrap();
    assert_eq!(mot(&q, 0), CMD_TRANSFER_TO_HOST_3D);
    assert_eq!((mot(&q, 6), mot(&q, 7), mot(&q, 9), mot(&q, 10), mot(&q, 11)), (10, 20, 100, 50, 1));
    let decalage = u64::from_le_bytes(q[48..56].try_into().unwrap());
    assert_eq!(decalage, base + 20 * pas as u64 + 40);
    assert_eq!(mot(&q, 14), TEXTURE_BASE + 1);
    assert_eq!(mot(&q, 16), pas);
    assert!(transfert(1, R::neuf(600, 0, 100, 1), 0, pas, l, h).is_none(), "hors texture");
    assert!(transfert(1, R::neuf(0, 0, 10, 10), 0, 100, l, h).is_none(), "pas trop petit");
}

#[test]
fn blit_identique_au_paquet_eprouve() {
    let mut b = [0u8; BLIT_OCTETS];
    encode_blit(TEXTURE_BASE, R::neuf(1, 2, 30, 40), 1, R::neuf(100, 200, 30, 40), &mut b).unwrap();
    assert_eq!(mot(&b, 0), 16 | (21 << 16));
    assert_eq!(mot(&b, 1), 0x0f);
    assert_eq!((mot(&b, 4), mot(&b, 7), mot(&b, 8), mot(&b, 10), mot(&b, 11)), (1, 100, 200, 30, 40));
    assert_eq!((mot(&b, 13), mot(&b, 16), mot(&b, 17), mot(&b, 19), mot(&b, 20)), (TEXTURE_BASE, 1, 2, 30, 40));
    assert!(encode_blit(1, R::neuf(0, 0, 2, 2), 1, R::neuf(0, 0, 2, 2), &mut b).is_none());
    assert!(encode_blit(2, R::neuf(0, 0, 2, 2), 1, R::neuf(0, 0, 3, 2), &mut b).is_none());
    assert_eq!(BLITS_PAR_SOUMISSION, 22);
}

#[test]
fn entrees_encodees_et_soumission_bornee() {
    let mut b = [0u8; 32];
    let n = encode_entrees(&[Entree { adresse: 0x1234_5000, longueur: 8192 }, Entree { adresse: 0x9000, longueur: 4096 }], &mut b).unwrap();
    assert_eq!(n, 32);
    assert_eq!(u64::from_le_bytes(b[0..8].try_into().unwrap()), 0x1234_5000);
    assert_eq!(mot(&b, 2), 8192);
    assert_eq!(mot(&entete_attache(7, 2), 7), 2);
    assert!(entete_soumission(0).is_none());
    assert!(entete_soumission(SOUMISSION_MAX + 4).is_none());
    assert_eq!(mot(&entete_soumission(88).unwrap(), 6), 88);
}

#[test]
fn convention_brute_reproduit_la_capture_du_10_novembre() {
    // Morceau du haut : ecran (71, 289 lignes) d'un scanout de 720 lignes,
    // texture de 572 lignes, source (0, 289). Envoyes BRUTS, l'hote posait
    // l'ecran 360..649 avec les lignes 283..572 : le modele doit le redire.
    let dst = R::neuf(320, 71, 640, 289);
    let src = R::neuf(0, 0, 640, 289);
    // Ce que l'hote fait d'un rectangle envoye tel quel : il l'interprete en
    // GL brut, c.-a-d. comme `vers_hote(Brut, ...)` le traduit.
    let pose = vers_hote(Convention::Brut, dst, 720);
    let lu = vers_hote(Convention::Brut, src, 572);
    assert_eq!((pose.y, pose.bas()), (360, 649));
    assert_eq!((lu.y, lu.bas()), (283, 572));
    // Corrige : envoyer les rectangles convertis remet tout en place.
    assert_eq!(vers_hote(Convention::Brut, pose, 720), dst);
    assert_eq!(vers_hote(Convention::Haut, dst, 720), dst);
}

#[test]
fn autotest_de_convention_juge_au_pixel() {
    let (l, h) = (8u32, 8u32);
    let mut cible = vec![0u32; 64];
    for y in 1..4 {
        for x in 0..l {
            cible[(y * l + x) as usize] = pixel_mire(x, y + 1);
        }
    }
    assert!(juge_autotest(&cible, l, h, 2, 1, 3));
    assert!(!juge_autotest(&cible, l, h, 3, 1, 3), "mauvaises lignes source");
    cible[0] = 1;
    assert!(!juge_autotest(&cible, l, h, 2, 1, 3), "pixel parasite hors du blit");
    let q = relecture(TEXTURE_BASE, R::neuf(0, 0, 8, 8), 32, 8, 8).unwrap();
    assert_eq!(mot(&q, 0), CMD_TRANSFER_FROM_HOST_3D);
}

#[test]
fn centiles_bornes_hautes_et_classes() {
    assert_eq!(classe(0), 0);
    assert_eq!(classe(1_999), 0);
    assert_eq!(classe(2_000), 1);
    assert_eq!(classe(3_000_000), 11); // 3 ms -> [2048, 4096) us
    assert_eq!(classe(4_100_000), 12); // 4,1 ms -> [4096, 8192) us
    let h = Histo::neuf();
    for _ in 0..90 { h.note(100_000); }   // 100 us -> classe 6 [64,128)
    for _ in 0..9 { h.note(3_000_000); }  // 3 ms  -> classe 11
    h.note(20_000_000);                   // 20 ms -> classe 14
    let v = h.instantane();
    assert_eq!(centile_us(&v, 50), Some(128));
    assert_eq!(centile_us(&v, 95), Some(4096));
    assert_eq!(centile_us(&v, 99), Some(4096));
    assert_eq!(centile_us(&v, 100), Some(32768));
    assert_eq!(h.max_ns.load(core::sync::atomic::Ordering::Relaxed), 20_000_000);
    assert_eq!(centile_us(&[0; CLASSES], 50), None);
    let d = delta(&v, &v);
    assert_eq!(d.iter().sum::<u64>(), 0);
}
