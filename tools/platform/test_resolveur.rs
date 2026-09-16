//! Quel resolveur part avec le navigateur.
//!
//! Le releve du 16 septembre : `dns=10.0.2.3` sur une TRIGKEY branchee a un
//! vrai reseau. C'est le resolveur du NAT de QEMU, la valeur compilee. Elle
//! ne mene nulle part, et toutes les pages repondaient « Unable to resolve
//! host » pendant que le reseau, lui, fonctionnait.

#[path = "../../src/net/resolveur.rs"]
mod resolveur;

use resolveur::{choisis, utilisable, Source};

const COMPILE: [u8; 4] = [10, 0, 2, 3];
const RIEN: [u8; 4] = [0, 0, 0, 0];

#[test]
fn le_bail_prime_sur_tout() {
    let (a, s) = choisis([192, 168, 1, 254], [192, 168, 1, 1], COMPILE);
    assert_eq!(a, [192, 168, 1, 254]);
    assert_eq!(s, Source::Bail);
}

#[test]
fn sans_bail_la_passerelle_sert() {
    // Le cas de la machine : lien monte, bail pas encore revenu. La
    // passerelle EXISTE sur ce reseau-ci ; la valeur compilee, non.
    let (a, s) = choisis(RIEN, [192, 168, 1, 1], COMPILE);
    assert_eq!(a, [192, 168, 1, 1]);
    assert_eq!(s, Source::Passerelle);
}

#[test]
fn la_valeur_compilee_est_le_dernier_recours() {
    let (a, s) = choisis(RIEN, RIEN, COMPILE);
    assert_eq!(a, COMPILE);
    assert_eq!(s, Source::Compile, "et la source le DIT");
}

#[test]
fn sans_rien_on_ne_ment_pas() {
    let (a, s) = choisis(RIEN, RIEN, RIEN);
    assert_eq!(a, RIEN);
    assert_eq!(s, Source::Aucun);
}

#[test]
fn sous_qemu_la_valeur_compilee_reste_juste() {
    // SLIRP donne bien 10.0.2.3 par bail : rien ne change sous QEMU.
    let (a, s) = choisis([10, 0, 2, 3], [10, 0, 2, 2], COMPILE);
    assert_eq!(a, [10, 0, 2, 3]);
    assert_eq!(s, Source::Bail);
}

#[test]
fn zero_n_est_pas_une_adresse() {
    assert!(!utilisable([0, 0, 0, 0]));
}

#[test]
fn la_boucle_locale_ne_resout_rien_ici() {
    // Aucun resolveur n'ecoute dans ce noyau : la pointer ferait echouer
    // chaque requete APRES un delai d'attente, au lieu de tout de suite.
    assert!(!utilisable([127, 0, 0, 1]));
    assert!(!utilisable([127, 53, 53, 53]));
}

#[test]
fn diffusion_et_multicast_ne_sont_pas_des_interlocuteurs() {
    assert!(!utilisable([255, 255, 255, 255]));
    assert!(!utilisable([224, 0, 0, 251]));
    assert!(!utilisable([239, 255, 255, 250]));
    assert!(!utilisable([240, 0, 0, 1]));
}

#[test]
fn une_adresse_privee_ordinaire_est_utilisable() {
    for a in [[192, 168, 1, 1], [10, 0, 0, 1], [172, 16, 5, 4], [1, 1, 1, 1]] {
        assert!(utilisable(a), "{:?}", a);
    }
}

#[test]
fn un_bail_inutilisable_ne_bloque_pas_la_passerelle() {
    // Un bail qui rend 0.0.0.0 ou une adresse de diffusion ne doit pas
    // l'emporter simplement parce qu'il vient du DHCP.
    let (a, s) = choisis([255, 255, 255, 255], [192, 168, 1, 1], COMPILE);
    assert_eq!(a, [192, 168, 1, 1]);
    assert_eq!(s, Source::Passerelle);
}

#[test]
fn les_noms_de_source_sont_ceux_du_journal() {
    // Ces chaines sortent dans BOUCHAUD_NAVIGATEUR_RESOLVEUR source=... et
    // c'est sur elles qu'on lira la prochaine archive physique.
    assert_eq!(Source::Bail.nom(), "bail-dhcp");
    assert_eq!(Source::Passerelle.nom(), "passerelle");
    assert_eq!(Source::Compile.nom(), "compile");
    assert_eq!(Source::Aucun.nom(), "aucun");
}
