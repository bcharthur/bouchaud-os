//! Preuve hote de la regle W^X.
//!
//! Une page a la fois inscriptible et executable est exactement ce dont une
//! injection de code a besoin, et rien de plus : obtenir une page ou l'on peut
//! ECRIRE des octets puis les EXECUTER, sans jamais avoir a contourner quoi
//! que ce soit. Un debordement qui atteint un `mmap` suffit ; il n'y a pas de
//! seconde etape a franchir.
//!
//! La regle est de l'arithmetique de drapeaux -- c'est ce qui permet de
//! l'eprouver ici, sur toutes les combinaisons, plutot que d'esperer qu'aucune
//! ne se presente.

#![allow(dead_code)]

#[path = "../../src/kernel/security/wx.rs"]
mod wx;

use wx::*;

// Les constantes de la compatibilite Linux et du format ELF, recopiees ici :
// deux transcriptions independantes valent mieux qu'un import qui ferait
// passer une erreur des deux cotes a la fois.
const PROT_READ: u32 = 0x1;
const PROT_WRITE: u32 = 0x2;
const PROT_EXEC: u32 = 0x4;
const PF_X: u32 = 0x1;
const PF_W: u32 = 0x2;
const PF_R: u32 = 0x4;

const PRESENT: u64 = 1 << 0;
const USER: u64 = 1 << 2;

#[test]
fn l_execution_est_le_defaut_sur_x86_64() {
    // La regle porte sur l'ABSENCE du bit « non executable », et non sur la
    // presence d'un bit d'execution. Oublier de poser le bit 63 suffit a
    // rendre la page executable, et c'est cet oubli qu'on cherche.
    assert!(
        inscriptible_et_executable(PRESENT | USER | ECRITURE),
        "une page inscriptible SANS bit de non-execution est executable"
    );
    assert!(!inscriptible_et_executable(
        PRESENT | USER | ECRITURE | NON_EXECUTABLE
    ));
}

#[test]
fn une_page_lecture_seule_executable_est_permise() {
    // C'est le segment de code de tout programme : executable, non
    // inscriptible. L'interdire rendrait le systeme inutilisable.
    assert!(!inscriptible_et_executable(PRESENT | USER));
}

#[test]
fn une_page_inscriptible_non_executable_est_permise() {
    // Le tas, la pile, les donnees.
    assert!(!inscriptible_et_executable(
        PRESENT | USER | ECRITURE | NON_EXECUTABLE
    ));
}

#[test]
fn toutes_les_combinaisons_de_drapeaux_sont_couvertes() {
    // Les quatre cas, exhaustivement, plutot qu'un echantillon.
    for ecriture in [false, true] {
        for non_exec in [false, true] {
            let mut d = PRESENT | USER;
            if ecriture {
                d |= ECRITURE;
            }
            if non_exec {
                d |= NON_EXECUTABLE;
            }
            assert_eq!(
                inscriptible_et_executable(d),
                ecriture && !non_exec,
                "ecriture={ecriture} non_exec={non_exec}"
            );
        }
    }
}

#[test]
fn les_bits_qui_ne_nous_concernent_pas_ne_changent_rien() {
    // Cache, accede, sale, huge : un jour l'un d'eux sera pose sur une page
    // utilisateur, et la regle ne doit pas changer d'avis pour autant.
    for parasite in [1u64 << 3, 1 << 4, 1 << 5, 1 << 6, 1 << 7, 1 << 9] {
        assert!(inscriptible_et_executable(PRESENT | ECRITURE | parasite));
        assert!(!inscriptible_et_executable(
            PRESENT | ECRITURE | NON_EXECUTABLE | parasite
        ));
    }
}

// ---------------------------------------------------------------------------
// Les demandes `PROT_*`
// ---------------------------------------------------------------------------

#[test]
fn prot_write_plus_prot_exec_est_refuse() {
    assert!(prot_viole_wx(PROT_WRITE | PROT_EXEC, PROT_WRITE, PROT_EXEC));
    assert!(prot_viole_wx(
        PROT_READ | PROT_WRITE | PROT_EXEC,
        PROT_WRITE,
        PROT_EXEC
    ));
}

#[test]
fn les_demandes_ordinaires_passent() {
    for prot in [
        0u32,
        PROT_READ,
        PROT_WRITE,
        PROT_EXEC,
        PROT_READ | PROT_WRITE,
        PROT_READ | PROT_EXEC,
    ] {
        assert!(
            !prot_viole_wx(prot, PROT_WRITE, PROT_EXEC),
            "prot={prot:#x} refuse a tort ; le systeme deviendrait inutilisable"
        );
    }
}

#[test]
fn la_sequence_ecrire_puis_executer_reste_possible() {
    // C'est ainsi que travaille un chargeur dynamique, et ainsi que
    // travaillerait un compilateur a la volee : les deux droits ne coexistent
    // a aucun instant, et la regle ne les separe pas dans le temps.
    assert!(!prot_viole_wx(PROT_READ | PROT_WRITE, PROT_WRITE, PROT_EXEC));
    assert!(!prot_viole_wx(PROT_READ | PROT_EXEC, PROT_WRITE, PROT_EXEC));
}

#[test]
fn toutes_les_demandes_prot_sont_couvertes() {
    for prot in 0u32..8 {
        let attendu = prot & PROT_WRITE != 0 && prot & PROT_EXEC != 0;
        assert_eq!(prot_viole_wx(prot, PROT_WRITE, PROT_EXEC), attendu, "prot={prot:#x}");
    }
}

// ---------------------------------------------------------------------------
// Les segments ELF
// ---------------------------------------------------------------------------

#[test]
fn un_segment_inscriptible_et_executable_est_refuse() {
    // Aucune chaine de compilation moderne n'en emet : un binaire qui en porte
    // un a ete fabrique pour cela.
    assert!(segment_viole_wx(PF_W | PF_X, PF_W, PF_X));
    assert!(segment_viole_wx(PF_R | PF_W | PF_X, PF_W, PF_X));
}

#[test]
fn les_deux_segments_d_un_binaire_ordinaire_passent() {
    // Le texte : lecture + execution. Les donnees : lecture + ecriture.
    assert!(!segment_viole_wx(PF_R | PF_X, PF_W, PF_X));
    assert!(!segment_viole_wx(PF_R | PF_W, PF_W, PF_X));
    assert!(!segment_viole_wx(PF_R, PF_W, PF_X));
    assert!(!segment_viole_wx(0, PF_W, PF_X));
}

#[test]
fn toutes_les_combinaisons_de_segment_sont_couvertes() {
    for flags in 0u32..8 {
        let attendu = flags & PF_W != 0 && flags & PF_X != 0;
        assert_eq!(segment_viole_wx(flags, PF_W, PF_X), attendu, "flags={flags:#x}");
    }
}

// ---------------------------------------------------------------------------
// La coherence entre les deux formes
// ---------------------------------------------------------------------------

#[test]
fn une_demande_refusee_aurait_bien_produit_une_page_w_et_x() {
    // La propriete qui lie les deux : ce que la regle `PROT_*` refuse est
    // exactement ce qui aurait donne une page inscriptible et executable.
    // Deux regles qui divergeraient laisseraient un chemin ouvert.
    for prot in 0u32..8 {
        let mut drapeaux = PRESENT | USER;
        if prot & PROT_WRITE != 0 {
            drapeaux |= ECRITURE;
        }
        if prot & PROT_EXEC == 0 {
            drapeaux |= NON_EXECUTABLE;
        }
        assert_eq!(
            prot_viole_wx(prot, PROT_WRITE, PROT_EXEC),
            inscriptible_et_executable(drapeaux),
            "prot={prot:#x} : la regle des demandes et celle des pages divergent"
        );
    }
}

#[test]
fn un_segment_refuse_aurait_bien_produit_une_page_w_et_x() {
    for flags in 0u32..8 {
        let mut drapeaux = PRESENT | USER;
        if flags & PF_W != 0 {
            drapeaux |= ECRITURE;
        }
        if flags & PF_X == 0 {
            drapeaux |= NON_EXECUTABLE;
        }
        assert_eq!(
            segment_viole_wx(flags, PF_W, PF_X),
            inscriptible_et_executable(drapeaux),
            "flags={flags:#x} : la regle des segments et celle des pages divergent"
        );
    }
}
