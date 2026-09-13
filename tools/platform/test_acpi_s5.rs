//! Preuve hote de la lecture de l'objet AML `\_S5_`.
//!
//! # Ce qui se joue ici
//!
//! `SLP_TYP` n'est pas une constante : 0 sur beaucoup de chipsets, 5 sur
//! d'autres, 7 sur certains portables. Une valeur fausse n'eteint pas la
//! machine -- elle l'ENDORT dans un etat dont elle ne sait pas revenir, ou ne
//! fait rien du tout. C'est exactement le genre de faute qu'aucune compilation
//! ne voit et qu'on ne decouvre qu'en appuyant sur « Eteindre » devant la
//! machine, puis en tenant le bouton d'alimentation.
//!
//! Les octets ci-dessous sont ecrits a la main depuis la grammaire AML de la
//! specification ACPI (PackageOp = 0x12, PkgLength, ZeroOp/OneOp/BytePrefix),
//! et non recopies du module : deux transcriptions independantes de la meme
//! grammaire, c'est ce qui rend une faute visible.

#![allow(dead_code)]

#[path = "../../src/kernel/acpi_s5/aml.rs"]
mod aml;

use aml::{extrait_s5, lit_paquet_s5};

/// Le DSDT reel n'est pas un paquet isole : `_S5_` est precede de centaines
/// d'octets d'AML. La recherche doit le trouver la ou il se trouve.
#[test]
fn s5_trouve_au_milieu_du_dsdt() {
    let mut table = vec![0x5b, 0x82, 0x10, b'P', b'C', b'I', b'0', 0x08, 0x00];
    table.extend_from_slice(&[b'_', b'S', b'5', b'_', 0x12, 0x06, 0x02, 0x0a, 0x05, 0x0a, 0x05]);
    table.extend_from_slice(&[0x14, 0x20, b'_', b'P', b'T', b'S']);
    assert_eq!(extrait_s5(&table), Some((5, 5)));
}

/// Une table qui nomme `_S4_` mais pas `_S5_` ne doit rien rendre : eteindre
/// avec la valeur de la veille prolongee laisserait la machine allumee.
#[test]
fn s4_n_est_pas_s5() {
    let table = [b'_', b'S', b'4', b'_', 0x12, 0x06, 0x02, 0x0a, 0x06, 0x0a, 0x06];
    assert_eq!(extrait_s5(&table), None);
}

/// Une occurrence de `_S5_` sans paquet exploitable ne doit pas masquer la
/// suivante, qui elle est valide.
#[test]
fn premiere_occurrence_invalide_puis_valide() {
    let mut table = vec![b'_', b'S', b'5', b'_', 0x5b, 0x00, 0x00];
    table.extend_from_slice(&[b'_', b'S', b'5', b'_', 0x12, 0x06, 0x02, 0x0a, 0x07, 0x0a, 0x07]);
    assert_eq!(extrait_s5(&table), Some((7, 7)));
}

/// Le paquet annonce deux elements mais la table s'arrete avant le second :
/// lire au-dela rendrait une valeur tiree de l'octet suivant en memoire.
#[test]
fn paquet_tronque_refuse() {
    let table = [b'_', b'S', b'5', b'_', 0x12, 0x06, 0x02, 0x0a, 0x05];
    assert_eq!(extrait_s5(&table), None);
}

/// `OneOp` est un entier AML comme un autre, et vaut un.
#[test]
fn one_op_vaut_un() {
    let reste = [0x12, 0x04, 0x02, 0x01, 0x01];
    assert_eq!(lit_paquet_s5(&reste), Some((1, 1)));
}

/// Les deux operandes sont independantes : PM1a et PM1b n'ont aucune raison
/// de recevoir la meme valeur, et les confondre casserait les machines a deux
/// blocs de controle.
#[test]
fn operandes_distinctes() {
    let reste = [0x12, 0x06, 0x02, 0x0a, 0x05, 0x0a, 0x03];
    assert_eq!(lit_paquet_s5(&reste), Some((5, 3)));
}

/// Une longueur de paquet sur quatre octets decale les elements d'autant.
#[test]
fn longueur_de_paquet_sur_quatre_octets() {
    let reste = [0x12, 0xc1, 0x00, 0x00, 0x00, 0x02, 0x0a, 0x02, 0x0a, 0x02];
    assert_eq!(lit_paquet_s5(&reste), Some((2, 2)));
}

/// Un paquet vide n'a pas d'operande du tout.
#[test]
fn paquet_vide_refuse() {
    let reste = [0x12, 0x02, 0x00];
    assert_eq!(lit_paquet_s5(&reste), None);
}

/// Sans `PackageOp`, rien n'est lu : c'est un autre objet qui porte ce nom.
#[test]
fn sans_package_op_refuse() {
    let reste = [0x0a, 0x05, 0x0a, 0x05];
    assert_eq!(lit_paquet_s5(&reste), None);
}

/// Un paquet qui n'annonce QU'UN element ne dit rien de PM1b -- meme si des
/// octets exploitables le suivent dans la table. Les lire quand meme
/// reviendrait a prendre l'objet AML voisin pour une valeur `SLP_TYP`.
#[test]
fn un_seul_element_annonce_ne_suffit_pas() {
    let reste = [0x12, 0x06, 0x01, 0x0a, 0x05, 0x0a, 0x03];
    assert_eq!(lit_paquet_s5(&reste), None);
}
