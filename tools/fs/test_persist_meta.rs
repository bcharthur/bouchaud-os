//! Metadonnees de la table de persistance (BOUCHAUD_PERSIST_META_V1).
//!
//! Le fragment de production `src/fs/persistance/meta.rs` est inclus tel quel.
//! Ce qui est etabli ici : les metadonnees font l'aller-retour, un disque
//! ecrit avant elles se relit comme avant, et une entree inconnue ne fait
//! jamais perdre un contenu.

#![allow(dead_code)]

#[path = "../../src/fs/persistance/meta.rs"]
mod meta;

use meta::*;

const TAILLE_ENTREE: usize = 256;

fn entree_vide() -> [u8; TAILLE_ENTREE] {
    [0u8; TAILLE_ENTREE]
}

#[test]
fn les_metadonnees_font_l_aller_retour() {
    for m in [
        Meta { dossier: true, mode: 0o700, uid: 1000, gid: 1000 },
        Meta { dossier: false, mode: 0o644, uid: 0, gid: 0 },
        Meta { dossier: false, mode: 0o4755, uid: 65535, gid: 1 },
    ] {
        let mut e = entree_vide();
        m.encode(&mut e[DEBUT_META..]);
        assert_eq!(Meta::decode(&e[DEBUT_META..]), Some(m));
    }
}

#[test]
fn un_disque_ecrit_avant_les_metadonnees_se_relit_comme_avant() {
    // Une entree d'avant : chemin + taille, octets 248..256 a zero.
    let e = entree_vide();
    assert_eq!(Meta::decode(&e[DEBUT_META..]), None, "format d'avant : pas de metadonnees");
    assert!(a_restaurer("notes.txt", 42, None), "un fichier non vide d'avant est restaure");
    assert!(!a_restaurer("vide", 0, None), "une entree d'avant de taille nulle ne l'a jamais ete");
    assert!(!a_restaurer("", 42, None));
}

#[test]
fn les_dossiers_et_fichiers_vides_sont_restaures_avec_metadonnees() {
    let d = Meta { dossier: true, mode: 0o755, uid: 0, gid: 0 };
    let f = Meta { dossier: false, mode: 0o600, uid: 0, gid: 0 };
    assert!(a_restaurer("home/arthur/Documents", 0, Some(d)));
    assert!(a_restaurer("home/arthur/vide.txt", 0, Some(f)));
}

#[test]
fn un_genre_ou_une_version_inconnus_ne_font_pas_perdre_le_contenu() {
    let mut e = entree_vide();
    Meta { dossier: false, mode: 0o644, uid: 0, gid: 0 }.encode(&mut e[DEBUT_META..]);
    e[DEBUT_META] = b'X';
    assert_eq!(Meta::decode(&e[DEBUT_META..]), None);
    // Traite comme un fichier d'avant : le contenu non vide est restaure.
    assert!(a_restaurer("x", 10, None));
    let mut e = entree_vide();
    Meta { dossier: true, mode: 0o755, uid: 0, gid: 0 }.encode(&mut e[DEBUT_META..]);
    e[DEBUT_META + 1] = 9;
    assert_eq!(Meta::decode(&e[DEBUT_META..]), None, "version inconnue");
}

#[test]
fn le_mode_est_borne_aux_bits_de_droits() {
    let mut e = entree_vide();
    Meta { dossier: false, mode: 0xffff, uid: 0, gid: 0 }.encode(&mut e[DEBUT_META..]);
    assert_eq!(Meta::decode(&e[DEBUT_META..]).unwrap().mode, 0o7777);
}

#[test]
fn les_metadonnees_n_empietent_ni_sur_le_chemin_ni_sur_la_taille() {
    // Chemin 0..240, taille 240..248 : l'encodage n'ecrit qu'en 248..256.
    let mut e = [0xAAu8; TAILLE_ENTREE];
    Meta { dossier: true, mode: 0o700, uid: 7, gid: 8 }.encode(&mut e[DEBUT_META..]);
    assert!(e[..DEBUT_META].iter().all(|&b| b == 0xAA));
    assert_eq!(DEBUT_META, 248);
}
