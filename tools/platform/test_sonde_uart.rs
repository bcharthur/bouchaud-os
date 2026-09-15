//! La sonde COM1 : ce qu'on conclut d'un port qu'on ne voit pas.
//!
//! Le cout d'une erreur ici n'est pas symetrique. Conclure « absent » sur une
//! machine qui a un UART fait disparaitre TOUS les journaux de CI d'un coup --
//! ce qui se voit immediatement. Conclure « present » sur un mini-PC qui n'en
//! a pas fait payer chaque octet de journal en attentes inutiles, sans qu'un
//! seul test ne rougisse. C'est ce second cas que ces tests couvrent.

#[path = "../../src/drivers/serial/sonde.rs"]
mod sonde;

use sonde::{depuis_octet, verdict, Presence, MOTIF_BOUCLAGE};

// ---------------------------------------------------------------------------
// Bus flottant : le cas de la TRIGKEY
// ---------------------------------------------------------------------------

#[test]
fn un_port_que_personne_ne_decode_rend_0xff_partout() {
    // Aucun peripherique sur 0x3F8 : le bus flotte, chaque registre rend
    // 0xFF. Le bouclage rend 0xFF lui aussi -- c'est precisement pourquoi on
    // ne peut pas se contenter de le comparer au motif.
    assert_eq!(verdict(0xFF, 0xFF, 0xFF, MOTIF_BOUCLAGE), Presence::BusFlottant);
}

#[test]
fn le_bus_flottant_est_ecarte_avant_le_bouclage() {
    // Le piege : si le motif valait 0xFF, tester le bouclage en premier
    // conclurait « present » sur un port vide. L'ordre du verdict doit rendre
    // ce choix de motif sans consequence.
    assert_eq!(verdict(0xFF, 0xFF, 0xFF, 0xFF), Presence::BusFlottant);
}

#[test]
fn un_seul_registre_a_0xff_ne_suffit_pas_a_conclure_au_vide() {
    // Un LSR a 0xFF seul est un etat legitime d'un vrai 16550 : registre
    // vide, tampon vide, pas d'erreur. Conclure au vide sur ce seul signal
    // ferait taire les journaux d'une machine qui parle.
    assert_eq!(verdict(0xFF, 0x01, MOTIF_BOUCLAGE, MOTIF_BOUCLAGE), Presence::Present);
    assert_eq!(verdict(0x60, 0xFF, MOTIF_BOUCLAGE, MOTIF_BOUCLAGE), Presence::Present);
}

// ---------------------------------------------------------------------------
// Port decode
// ---------------------------------------------------------------------------

#[test]
fn un_16550_rend_le_motif_qu_on_lui_a_donne() {
    assert_eq!(verdict(0x60, 0x01, MOTIF_BOUCLAGE, MOTIF_BOUCLAGE), Presence::Present);
    assert!(verdict(0x60, 0x01, MOTIF_BOUCLAGE, MOTIF_BOUCLAGE).ecrire());
}

#[test]
fn un_port_decode_qui_ne_boucle_pas_est_muet() {
    // Quelque chose repond sur le port, mais ce n'est pas un 16550 en boucle
    // locale. On n'y ecrit pas : au mieux c'est perdu, au pire c'est un autre
    // peripherique.
    assert_eq!(verdict(0x60, 0x01, 0x00, MOTIF_BOUCLAGE), Presence::Muet);
    assert!(!verdict(0x60, 0x01, 0x00, MOTIF_BOUCLAGE).ecrire());
}

#[test]
fn seul_present_autorise_a_ecrire() {
    assert!(!Presence::BusFlottant.ecrire());
    assert!(!Presence::Muet.ecrire());
    assert!(Presence::Present.ecrire());
}

// ---------------------------------------------------------------------------
// Le verdict traverse un AtomicU8 : le codage doit tenir
// ---------------------------------------------------------------------------

#[test]
fn le_codage_en_octet_fait_un_aller_retour_exact() {
    for etat in [Presence::BusFlottant, Presence::Muet, Presence::Present] {
        assert_eq!(depuis_octet(etat as u8), etat, "{:?}", etat);
    }
}

#[test]
fn un_octet_inconnu_autorise_a_ecrire() {
    // Perdre le journal parce qu'un octet a ete mal range serait le pire des
    // deux echecs : le doute doit pencher du cote qui garde la trace.
    assert_eq!(depuis_octet(200), Presence::Present);
    assert!(depuis_octet(200).ecrire());
}

#[test]
fn les_numeros_de_variantes_sont_ceux_que_le_pilote_relit() {
    // Le pilote ne relit pas l'enum, il relit 0, 1 ou 2. Fixer ces trois
    // nombres ici empeche une refonte de l'ordre des variantes de retourner
    // le verdict sans rien casser de visible.
    assert_eq!(Presence::BusFlottant as u8, 0);
    assert_eq!(Presence::Muet as u8, 1);
    assert_eq!(Presence::Present as u8, 2);
}

#[test]
fn chaque_etat_a_un_nom_distinct_pour_le_journal() {
    let noms = [
        Presence::BusFlottant.nom(),
        Presence::Muet.nom(),
        Presence::Present.nom(),
    ];
    for (i, a) in noms.iter().enumerate() {
        assert!(!a.is_empty());
        for b in &noms[i + 1..] {
            assert_ne!(a, b);
        }
    }
}
