//! Le detecteur Alt+SysRq de l'IRQ clavier (#283).
//!
//! Les sequences ci-dessous sont celles du jeu 1 que produit le 8042 avec la
//! traduction active (configuration posee par `ps2_keyboard::init`, bit 0x40).
//! Elles ne sont PAS recopiees du module : ce sont les octets que QEMU injecte
//! pour `sendkey alt-sysrq` (Alt 0x38, puis 0x84 en jeu 2, traduit 0x54).
//!
//! Lance par `tools/ci/run_host_tests.sh`.

#[path = "../../src/drivers/input/sysrq.rs"]
mod sysrq;

use sysrq::DetecteurSysRq;

fn declenchements(octets: &[u8]) -> usize {
    let mut d = DetecteurSysRq::nouveau();
    octets.iter().filter(|&&sc| d.pousse(sc)).count()
}

#[test]
fn sendkey_alt_sysrq_declenche_une_fois() {
    // Alt appui, SysRq appui, SysRq relache, Alt relache.
    assert_eq!(declenchements(&[0x38, 0x54, 0xD4, 0xB8]), 1);
}

#[test]
fn altgr_sysrq_declenche() {
    assert_eq!(declenchements(&[0xE0, 0x38, 0x54, 0xD4, 0xE0, 0xB8]), 1);
}

#[test]
fn sysrq_sans_alt_ne_declenche_pas() {
    assert_eq!(declenchements(&[0x54, 0xD4]), 0);
}

#[test]
fn impr_ecran_seul_ne_declenche_pas() {
    // Impr. ecran sans Alt : E0 2A E0 37, puis E0 B7 E0 AA.
    assert_eq!(declenchements(&[0xE0, 0x2A, 0xE0, 0x37, 0xE0, 0xB7, 0xE0, 0xAA]), 0);
}

#[test]
fn alt_relache_desarme() {
    assert_eq!(declenchements(&[0x38, 0xB8, 0x54, 0xD4]), 0);
}

#[test]
fn relachement_d_un_alt_garde_l_autre() {
    // Les deux Alt tenus, le gauche relache : le droit suffit.
    assert_eq!(declenchements(&[0x38, 0xE0, 0x38, 0xB8, 0x54]), 1);
    // Le droit relache : le gauche suffit.
    assert_eq!(declenchements(&[0x38, 0xE0, 0x38, 0xE0, 0xB8, 0x54]), 1);
}

#[test]
fn prefixe_e0_ne_survit_qu_un_octet() {
    // `E0 54` n'est pas SysRq ; le 0x54 qui suit, lui, l'est.
    assert_eq!(declenchements(&[0x38, 0xE0, 0x54, 0x54]), 1);
}

#[test]
fn repetition_automatique_de_sysrq_declenche_a_chaque_appui() {
    // Le clavier repete l'appui tenu : chaque repetition est une demande ;
    // le noyau les fusionne (un seul releve en attente).
    assert_eq!(declenchements(&[0x38, 0x54, 0x54, 0x54]), 3);
}

#[test]
fn touches_ordinaires_sous_alt_ne_declenchent_pas() {
    // Alt+Tab, Alt+F4.
    assert_eq!(declenchements(&[0x38, 0x0F, 0x8F, 0x3E, 0xBE, 0xB8]), 0);
}
