//! Le FIFO serie emet-il exactement les memes octets qu'un octet a la fois ?
//!
//! # Ce qui a change, et le risque
//!
//! `write(2)` s'execute sous le gros verrou du noyau. Le pilote serie y
//! attendait THRE avant CHAQUE octet -- un `inb`, donc une sortie du mode
//! traduit sous TCG. Un programme bavard comme Ladybird y serialisait les
//! quatre coeurs derriere COM1 : `[BKL-SYSCALL]` mesurait jusqu'a 152 ms de
//! detention pour un seul `write`.
//!
//! Le 16550 a un FIFO d'emission de seize octets. Le pilote attend maintenant
//! une fois puis en pousse seize.
//!
//! Le risque n'est pas la vitesse, c'est le JOURNAL. Un lot mal decoupe perd
//! des octets, en reordonne, ou coupe un CRLF en deux -- et le journal serie
//! est le seul instrument dont on dispose pour tout le reste. Un test qui ne
//! verifierait que la vitesse laisserait passer exactement ce qu'il ne faut
//! pas.
//!
//! Lance par `tools/serie/test-lots.sh`.

extern crate alloc;

#[path = "../../src/drivers/serial/lots.rs"]
mod lots;

use lots::{attendu, en_lots};

/// Concatene ce que `en_lots` emet, pour une taille de tampon donnee.
fn emis(octets: &[u8], taille: usize) -> alloc::vec::Vec<u8> {
    let mut tampon = alloc::vec![0u8; taille];
    let mut sortie = alloc::vec::Vec::new();
    en_lots(octets, &mut tampon, |lot| {
        assert!(!lot.is_empty(), "un lot vide ne sert a rien et coute une attente");
        assert!(lot.len() <= taille, "un lot deborde du tampon");
        sortie.extend_from_slice(lot);
    });
    sortie
}

fn reference(octets: &[u8]) -> alloc::vec::Vec<u8> {
    let mut sortie = alloc::vec::Vec::new();
    attendu(octets, &mut sortie);
    sortie
}

// ─── L'equivalence ─────────────────────────────────────────────────────────

/// LA propriete : quelle que soit la taille du tampon, la suite d'octets emise
/// est celle qu'un pilote octet par octet aurait produite.
#[test]
fn le_flux_emis_est_identique_quelle_que_soit_la_taille_du_lot() {
    let cas: &[&[u8]] = &[
        b"",
        b"a",
        b"\n",
        b"\n\n\n",
        b"ligne\n",
        b"une ligne sans fin",
        b"a\nb\nc\n",
        b"\n\ndeux vides\n\n",
        b"[BKL-SYSCALL] write hold=152ms\n[GUI-DAMAGE] presents=17\n",
        &[0u8, 255, 128, 10, 13, 10, 65],
    ];
    for entree in cas {
        let attendu = reference(entree);
        for taille in 2..=70usize {
            assert_eq!(
                emis(entree, taille),
                attendu,
                "taille de lot {taille} pour {entree:?}"
            );
        }
    }
}

/// Un flux long et regulier : c'est le cas reel, une trace de plusieurs
/// centaines d'octets poussee d'un coup.
#[test]
fn un_flux_long_traverse_plusieurs_lots_sans_perte() {
    let mut entree = alloc::vec::Vec::new();
    for index in 0..300u32 {
        entree.extend_from_slice(b"octet ");
        entree.push(b'0' + (index % 10) as u8);
        entree.push(b'\n');
    }
    let attendu = reference(&entree);
    for taille in [2usize, 3, 16, 17, 64, 1024] {
        assert_eq!(emis(&entree, taille), attendu, "taille {taille}");
    }
}

/// Un CRLF n'est JAMAIS coupe entre deux lots.
///
/// Rien ne l'exigerait a la lecture -- les octets arrivent dans l'ordre de
/// toute facon -- mais un terminal qui recoit un CR seul en fin de lot puis un
/// LF au suivant affiche parfois une ligne de trop. Le pilote le garantit ; ce
/// test le fige.
#[test]
fn un_saut_de_ligne_reste_entier_dans_son_lot() {
    let entree = b"aa\nbb\ncc\n";
    for taille in 2..=12usize {
        let mut tampon = alloc::vec![0u8; taille];
        en_lots(entree, &mut tampon, |lot| {
            assert!(
                lot.last() != Some(&b'\r'),
                "un lot se termine sur un CR orphelin (taille {taille})"
            );
        });
    }
}

/// Un tampon trop petit pour loger un CRLF n'emet rien plutot que de couper.
#[test]
fn un_tampon_degenere_n_emet_rien() {
    for taille in [0usize, 1] {
        let mut tampon = alloc::vec![0u8; taille];
        en_lots(b"quelque chose\n", &mut tampon, |_| {
            panic!("un tampon de {taille} octet(s) ne doit rien emettre");
        });
    }
}

/// Le decoupage doit REELLEMENT decouper : sans cette borne, un test
/// d'equivalence resterait vert pour un pilote qui emet un octet a la fois.
#[test]
fn les_lots_remplissent_le_tampon() {
    let entree = alloc::vec![b'x'; 200];
    let mut tampon = alloc::vec![0u8; 16];
    let mut lots = 0usize;
    en_lots(&entree, &mut tampon, |lot| {
        lots += 1;
        // Tous pleins sauf le dernier.
        if lots * 16 <= 200 {
            assert_eq!(lot.len(), 16, "lot {lots} incomplet");
        }
    });
    assert_eq!(
        lots,
        200usize.div_ceil(16),
        "200 octets par lots de 16 : treize attentes, pas deux cents"
    );
}
