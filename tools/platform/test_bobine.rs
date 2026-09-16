//! Le tambour RAM de l'enregistreur de vol.
//!
//! Ce que ces epreuves defendent : un `append` qui ne touche plus le
//! peripherique qu'il observe, une perte qui se COMPTE au lieu de se
//! deviner, et un enregistrement publie qui n'est jamais lu a moitie.

#[path = "../../src/kernel/debug/bobine.rs"]
mod bobine;

use bobine::{ecrases_attendus, Bobine, DESCRIPTEURS, OCTETS, PAYLOAD_MAX};

/// Ecrit un enregistrement complet dans un tambour de test, octets compris.
fn pose(b: &Bobine, memoire: &mut [u8], genre: u16, ts: u64, charge: &[u8]) -> bool {
    let Some(r) = b.reserve(charge.len()) else {
        return false;
    };
    let (depart, premiere, seconde) = r.tranches();
    memoire[depart..depart + premiere].copy_from_slice(&charge[..premiere]);
    if seconde != 0 {
        memoire[..seconde].copy_from_slice(&charge[premiere..]);
    }
    b.publie(&r, genre, ts, charge.len() as u64);
    true
}

fn relit(b: &Bobine, memoire: &[u8], seq: u64) -> Option<(u16, u64, Vec<u8>)> {
    let d = b.lis(seq)?;
    let (depart, premiere, seconde) = d.tranches();
    let mut out = Vec::with_capacity(d.longueur);
    out.extend_from_slice(&memoire[depart..depart + premiere]);
    out.extend_from_slice(&memoire[..seconde]);
    Some((d.genre, d.ts_ns, out))
}

fn memoire() -> Vec<u8> {
    vec![0u8; OCTETS]
}

#[test]
fn l_invariant_supprime_la_seconde_cause_de_perte() {
    // `DESCRIPTEURS` enregistrements au maximum de la charge utile occupent
    // exactement l'anneau d'octets. L'anneau d'octets ne peut donc jamais
    // laper avant l'anneau de descripteurs, et la perte se reduit a UNE
    // cause -- celle qui se compte exactement.
    assert_eq!(OCTETS, DESCRIPTEURS * PAYLOAD_MAX);
}

#[test]
fn un_enregistrement_pose_se_relit_a_l_identique() {
    let b = Bobine::neuve();
    let mut m = memoire();
    let charge = b"sample ts_ns=1234 cpu=3\n";
    assert!(pose(&b, &mut m, 2, 1234, charge));
    let (genre, ts, octets) = relit(&b, &m, 1).expect("l'enregistrement 1 doit etre lisible");
    assert_eq!(genre, 2);
    assert_eq!(ts, 1234);
    assert_eq!(octets, charge);
}

#[test]
fn les_numeros_partent_de_un_et_se_suivent() {
    let b = Bobine::neuve();
    let mut m = memoire();
    for i in 0..8 {
        assert!(pose(&b, &mut m, 2, i, b"x"));
    }
    assert_eq!(b.dernier(), 8);
    for seq in 1..=8u64 {
        let d = b.lis(seq).expect("tous lisibles");
        assert_eq!(d.seq, seq);
        assert_eq!(d.ts_ns, seq - 1);
    }
    assert!(b.lis(0).is_none(), "zero n'est pas un numero");
    assert!(b.lis(9).is_none(), "rien au-dela du dernier reserve");
}

#[test]
fn une_charge_trop_grande_est_refusee_sans_consommer_de_numero() {
    let b = Bobine::neuve();
    let trop = vec![0u8; PAYLOAD_MAX + 1];
    assert!(b.reserve(trop.len()).is_none());
    assert_eq!(b.dernier(), 0, "un refus ne doit pas trouer la numerotation");
    assert_eq!(b.etat().refuses, 1);
    assert_eq!(b.etat().ecrases, 0, "un refus n'est pas une perte du tambour");
}

#[test]
fn une_charge_vide_est_un_enregistrement_valide() {
    // Une marque sans texte reste une marque : la refuser perdrait le seul
    // enregistrement qui dit qu'une session s'est terminee proprement.
    let b = Bobine::neuve();
    let mut m = memoire();
    assert!(pose(&b, &mut m, 3, 7, b""));
    let d = b.lis(1).expect("lisible");
    assert_eq!(d.longueur, 0);
    assert_eq!(d.genre, 3);
}

#[test]
fn un_enregistrement_reserve_mais_non_publie_n_est_pas_lisible() {
    // C'est la garantie de publication atomique : tant que les octets ne
    // sont pas la, le lecteur ne voit rien -- et surtout pas les octets de
    // l'enregistrement precedent.
    let b = Bobine::neuve();
    let r = b.reserve(16).expect("reservation");
    assert_eq!(r.seq, 1);
    assert!(b.lis(1).is_none(), "publication pas encore faite");
    b.publie(&r, 1, 99, 0);
    assert!(b.lis(1).is_some());
}

#[test]
fn la_perte_se_compte_exactement_et_pas_approximativement() {
    let b = Bobine::neuve();
    let mut m = memoire();
    // Juste de quoi remplir l'anneau : rien ne doit etre perdu.
    for i in 0..DESCRIPTEURS {
        assert!(pose(&b, &mut m, 2, i as u64, b"charge"));
    }
    assert_eq!(b.etat().ecrases, 0);
    assert!(b.lis(1).is_some(), "le premier tient encore");

    // Un de plus : le premier disparait, et UN SEUL.
    assert!(pose(&b, &mut m, 2, 9999, b"charge"));
    assert_eq!(b.etat().ecrases, 1);
    assert!(b.lis(1).is_none(), "le premier a ete ecrase");
    assert!(b.lis(2).is_some(), "le deuxieme tient encore");
}

#[test]
fn le_compteur_de_perte_et_la_formule_fermee_s_accordent() {
    // Deux facons de compter la meme chose, dont une seule traverse le chemin
    // chaud. Si elles divergent, c'est le chemin chaud qui a tort.
    let b = Bobine::neuve();
    let mut m = memoire();
    let total = DESCRIPTEURS + DESCRIPTEURS / 2 + 7;
    for i in 0..total {
        assert!(pose(&b, &mut m, 2, i as u64, b"x"));
    }
    let etat = b.etat();
    assert_eq!(etat.reserves, total as u64);
    assert_eq!(etat.poses, total as u64);
    assert_eq!(etat.ecrases, ecrases_attendus(total as u64));
    // Rien n'est sorti vers le support : tout ecrasement est une perte.
    assert_eq!(etat.perdus, etat.ecrases);
}

#[test]
fn ce_qui_survit_apres_un_tour_complet_est_le_plus_recent() {
    let b = Bobine::neuve();
    let mut m = memoire();
    let total = DESCRIPTEURS + 100;
    for i in 0..total {
        let texte = format!("enregistrement {}", i);
        assert!(pose(&b, &mut m, 2, i as u64, texte.as_bytes()));
    }
    // Les cent premiers sont partis.
    for seq in 1..=100u64 {
        assert!(b.lis(seq).is_none(), "seq {} devait etre ecrase", seq);
    }
    // Le plus ancien survivant porte bien son propre texte, pas celui de son
    // successeur dans le meme descripteur.
    let plus_ancien = b.plus_ancien();
    assert_eq!(plus_ancien, 101);
    let (_, ts, octets) = relit(&b, &m, plus_ancien).expect("survivant lisible");
    assert_eq!(ts, 100);
    assert_eq!(octets, b"enregistrement 100");
    let (_, _, dernier) = relit(&b, &m, total as u64).expect("dernier lisible");
    assert_eq!(dernier, format!("enregistrement {}", total - 1).as_bytes());
}

#[test]
fn un_enregistrement_a_cheval_sur_la_fin_de_l_anneau_se_relit_entier() {
    // Le cas qui ne se produit qu'une fois par tour d'anneau, et qui rend
    // quatre kibioctets de charge utile melangee quand il est faux.
    let b = Bobine::neuve();
    let mut m = memoire();
    let grand = vec![0xa5u8; PAYLOAD_MAX];
    // Amener le curseur d'octets pres de la fin sans consommer tous les
    // descripteurs : des charges pleines, moins une demie.
    let pleines = DESCRIPTEURS - 1;
    for _ in 0..pleines {
        assert!(pose(&b, &mut m, 2, 0, &grand));
    }
    let moitie = vec![0x5au8; PAYLOAD_MAX / 2];
    assert!(pose(&b, &mut m, 2, 0, &moitie));
    // Celui-ci commence avant la fin de l'anneau et se termine apres.
    let temoin: Vec<u8> = (0..PAYLOAD_MAX).map(|i| (i % 251) as u8).collect();
    let seq_avant = b.dernier();
    assert!(pose(&b, &mut m, 7, 4242, &temoin));
    let seq = seq_avant + 1;
    let d = b.lis(seq).expect("lisible");
    let (depart, premiere, seconde) = d.tranches();
    assert!(seconde != 0, "ce cas doit vraiment etre a cheval");
    assert_eq!(depart + premiere, OCTETS);
    let (genre, ts, octets) = relit(&b, &m, seq).expect("relecture");
    assert_eq!(genre, 7);
    assert_eq!(ts, 4242);
    assert_eq!(octets, temoin);
}

#[test]
fn le_recyclage_d_un_descripteur_invalide_l_ancien_numero_aussitot() {
    // Le piege : le lecteur tient un numero, le producteur recycle la case.
    // Sans la verification du numero DANS la case, le lecteur rendrait les
    // champs du nouvel enregistrement sous l'ancien numero.
    let b = Bobine::neuve();
    let mut m = memoire();
    assert!(pose(&b, &mut m, 1, 11, b"ancien"));
    let ancien = b.lis(1).expect("lisible");
    assert_eq!(ancien.ts_ns, 11);
    for i in 0..DESCRIPTEURS {
        assert!(pose(&b, &mut m, 2, 1000 + i as u64, b"neuf"));
    }
    assert!(b.lis(1).is_none());
    // La case 0 porte maintenant `1 + DESCRIPTEURS`, c'est-a-dire le DERNIER
    // des « neuf » -- pas le premier. S'y tromper serait exactement la faute
    // que cette epreuve cherche : lire un numero et obtenir les champs d'un
    // autre.
    let remplacant = b.lis(1 + DESCRIPTEURS as u64).expect("remplacant lisible");
    assert_eq!(remplacant.ts_ns, 1000 + DESCRIPTEURS as u64 - 1);
}

#[test]
fn un_enregistrement_deja_sorti_n_est_pas_perdu_quand_il_est_recycle() {
    // Le vidage final convertit le journal serie et les evenements de vol en
    // enregistrements, qui recyclent par CONSTRUCTION les descripteurs qu'on
    // vient de sortir. Les compter comme perdus faisait crier une archive
    // parfaitement complete : le banc affichait `ecrases=223` sur une session
    // dont pas un octet ne manquait.
    let b = Bobine::neuve();
    let mut m = memoire();
    let moitie = DESCRIPTEURS / 2;
    for i in 0..DESCRIPTEURS {
        assert!(pose(&b, &mut m, 2, i as u64, b"x"));
    }
    // Seule la PREMIERE moitie est sortie vers le support.
    b.note_vidange(moitie as u64, moitie as u64);

    // Autant d'enregistrements que la moitie sortie : ils recyclent
    // exactement des descripteurs deja sur le support. Rien ne manque.
    for i in 0..moitie {
        assert!(pose(&b, &mut m, 1, 5000 + i as u64, b"serie"));
    }
    let etat = b.etat();
    assert_eq!(etat.ecrases, moitie as u64, "les descripteurs ont bien ete recycles");
    assert_eq!(etat.perdus, 0, "mais aucun de ceux-la ne manquait");

    // Cent de plus : ceux-ci recyclent la SECONDE moitie, qui n'est jamais
    // sortie. Cette fois ce sont des pertes reelles.
    for i in 0..100 {
        assert!(pose(&b, &mut m, 2, 6000 + i, b"y"));
    }
    let etat = b.etat();
    assert_eq!(etat.ecrases, moitie as u64 + 100);
    assert_eq!(etat.perdus, 100);
}

#[test]
fn le_curseur_de_vidage_ne_recule_jamais() {
    // Deux vidages concurrents -- celui de l'extinction et celui d'un chemin
    // fatal -- ne doivent pas pouvoir se contredire et transformer des
    // enregistrements sauves en perdus.
    let b = Bobine::neuve();
    let mut m = memoire();
    for i in 0..DESCRIPTEURS {
        assert!(pose(&b, &mut m, 2, i as u64, b"x"));
    }
    b.note_vidange(DESCRIPTEURS as u64, DESCRIPTEURS as u64);
    b.note_vidange(1, 5);
    for i in 0..10 {
        assert!(pose(&b, &mut m, 2, 9000 + i, b"z"));
    }
    assert_eq!(b.etat().perdus, 0, "le curseur a recule et a invente dix pertes");
    assert_eq!(b.etat().ecrases, 10);
}

#[test]
fn les_vidanges_se_comptent_sans_toucher_aux_pertes() {
    let b = Bobine::neuve();
    let mut m = memoire();
    for i in 0..10 {
        assert!(pose(&b, &mut m, 2, i, b"x"));
    }
    b.note_vidange(10, 10);
    let etat = b.etat();
    assert_eq!(etat.vidanges, 10);
    assert_eq!(etat.ecrases, 0);
    assert_eq!(etat.poses, 10);
}

#[test]
fn les_octets_vifs_ne_depassent_jamais_l_anneau() {
    let b = Bobine::neuve();
    let mut m = memoire();
    let grand = vec![1u8; PAYLOAD_MAX];
    for _ in 0..(DESCRIPTEURS * 2) {
        assert!(pose(&b, &mut m, 2, 0, &grand));
    }
    assert!(b.etat().octets_vifs <= OCTETS as u64);
}

#[test]
fn un_tambour_vide_ne_ment_pas() {
    let b = Bobine::neuve();
    let etat = b.etat();
    assert_eq!(etat.reserves, 0);
    assert_eq!(etat.poses, 0);
    assert_eq!(etat.ecrases, 0);
    assert_eq!(b.plus_ancien(), 1);
    assert_eq!(b.dernier(), 0);
    assert!(b.lis(1).is_none());
}
