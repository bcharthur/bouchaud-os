//! Contrat d'identite d'applications GUI ring 3, sans charger le noyau.
extern crate alloc;
#[path = "../../src/gui/applications_externes.rs"]
mod applications;
use applications::{argv, pour_chemin, pour_kind, titre_pour_chemin, Identite, APPLICATIONS, DOOM, LADYBIRD};

#[test]
fn chaque_identite_est_unique_et_retrouvable() {
    for (i, a) in APPLICATIONS.iter().enumerate() {
        assert_eq!(pour_kind(a.kind), Some(a));
        assert_eq!(pour_chemin(a.executable), Some(a));
        assert!(!a.titre.is_empty() && a.zone.0 > 0 && a.zone.1 > 0);
        for b in APPLICATIONS.iter().skip(i + 1) {
            assert_ne!(a.kind, b.kind);
            assert_ne!(a.executable, b.executable);
            assert_ne!(a.identite, b.identite);
        }
    }
}

#[test]
fn doom_ne_recoit_jamais_le_titre_ou_les_options_ladybird() {
    assert_eq!(DOOM.identite, Identite::Doom);
    assert_eq!(LADYBIRD.identite, Identite::Ladybird);
    assert_eq!(titre_pour_chemin("/doom"), "DOOM - Freedoom");
    assert_eq!(titre_pour_chemin("/bo-navigateur"), "Ladybird");
    assert_eq!(argv("/doom").iter().map(String::as_str).collect::<Vec<_>>(),
        vec!["/doom", "-iwad", "/freedoom1.wad", "-nosound"]);
    assert_eq!(argv("/bo-navigateur").iter().map(String::as_str).collect::<Vec<_>>(),
        vec!["/bo-navigateur"]);
}

#[test]
fn tout_autre_programme_garde_son_identite() {
    assert_eq!(titre_pour_chemin("/apps/editeur"), "editeur");
    assert_eq!(titre_pour_chemin(""), "Application");
    assert_eq!(titre_pour_chemin("/"), "Application");
    assert_eq!(argv("/apps/editeur"), vec!["/apps/editeur"]);
    assert!(pour_kind(usize::MAX).is_none());
    assert!(pour_chemin("/application-absente").is_none());
}

#[test]
fn jauge_os_reste_universelle() {
    assert!(!applications::TEXTE_DEMARRAGE.contains("Ladybird"));
    assert!(!applications::TEXTE_DEMARRAGE.contains("Qt"));
    assert!(!applications::TEXTE_DEMARRAGE.contains("DOOM"));
    assert!(!applications::TEXTE_DEMARRAGE.contains("Python"));
}
