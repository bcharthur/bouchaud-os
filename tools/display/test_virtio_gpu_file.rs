//! Ce que la file de controle virtio-gpu croit d'une reponse
//! (BOUCHAUD_VIRTIO_GPU_BARRIERE_V1).
//!
//! La specification permet au peripherique de rendre une commande AVANT d'en
//! avoir fini le traitement, sauf barriere (`VIRTIO_GPU_FLAG_FENCE`), et ne
//! garantit pas l'ordre de traitement de la file de controle. Chaque cas
//! ci-dessous est une reponse que le pilote doit REFUSER de prendre pour
//! celle de sa commande -- et la consequence pour la memoire DMA.

#[path = "../../src/drivers/display/virtio_gpu_file.rs"]
mod file;
#[path = "../../src/drivers/display/virtio_gpu_dma.rs"]
mod dma;

use dma::{apres_attache, apres_autre, apres_detache, issue, peut_rendre, Issue, Memoire, REPONSE_INVALIDE, REPONSE_OK};
use file::{juge, pose_barriere, Utilise, Violation, DRAPEAU_BARRIERE, ENTETE};

/// Une reponse conforme a la commande en vol : tete 0, descripteur
/// ecrivable de 408 octets, barriere 7.
fn conforme() -> (u16, u16, Utilise, u32, u32, u64, u64) {
    (1, 0, Utilise { id: 0, len: ENTETE }, 408, DRAPEAU_BARRIERE, 7, 7)
}

fn juge_t(t: (u16, u16, Utilise, u32, u32, u64, u64)) -> Result<(), Violation> {
    juge(t.0, t.1, t.2, t.3, t.4, t.5, t.6)
}

#[test]
fn une_reponse_conforme_est_acceptee() {
    assert_eq!(juge_t(conforme()), Ok(()));
    // Une reponse plus longue que l'en-tete (GET_DISPLAY_INFO) aussi.
    let mut t = conforme();
    t.2.len = 408;
    assert_eq!(juge_t(t), Ok(()));
}

#[test]
fn achevement_desordonne_l_entree_designe_une_autre_chaine() {
    // Deux commandes en vol (ce que faisait le groupement) : le peripherique
    // rend la SECONDE d'abord. Avec une seule commande en vol, une entree
    // d'une autre chaine ne peut venir que d'un peripherique non conforme.
    let mut t = conforme();
    t.2.id = 2;
    assert_eq!(juge_t(t), Err(Violation::IdInattendu { attendu: 0, recu: 2 }));
}

#[test]
fn reponse_rendue_avant_la_fin_du_traitement_refusee() {
    // Sans barriere renvoyee, rien ne dit que le traitement est fini.
    let mut t = conforme();
    t.4 = 0;
    assert_eq!(juge_t(t), Err(Violation::BarriereAbsente));
}

#[test]
fn reponse_tardive_d_une_commande_precedente_refusee() {
    // La commande 6 etait restee sans reponse ; la sienne arrive pendant que
    // la 7 est en vol (achevement differe).
    let mut t = conforme();
    t.5 = 6;
    assert_eq!(juge_t(t), Err(Violation::BarriereDifferente { attendue: 7, recue: 6 }));
}

#[test]
fn deux_entrees_d_un_coup_refusees() {
    let mut t = conforme();
    t.0 = 2;
    assert_eq!(juge_t(t), Err(Violation::TropDEntrees { avance: 2 }));
    t.0 = 0;
    assert_eq!(juge_t(t), Err(Violation::TropDEntrees { avance: 0 }));
}

#[test]
fn reponse_plus_courte_qu_un_en_tete_refusee() {
    let mut t = conforme();
    t.2.len = ENTETE - 1;
    assert_eq!(juge_t(t), Err(Violation::ReponseCourte { len: ENTETE - 1 }));
}

#[test]
fn reponse_plus_longue_que_le_descripteur_ecrivable_refusee() {
    let mut t = conforme();
    t.2.len = 409;
    assert_eq!(juge_t(t), Err(Violation::ReponseTropLongue { len: 409, capacite: 408 }));
    // Pile la capacite : accepte.
    t.2.len = 408;
    assert_eq!(juge_t(t), Ok(()));
}

#[test]
fn reponse_a_zero_et_barriere_a_zero_n_est_pas_une_reponse() {
    // Le tampon de reponse est remis a zero avant chaque envoi : un
    // peripherique qui avance `used.idx` sans rien ecrire laisse drapeaux=0.
    let mut t = conforme();
    t.4 = 0;
    t.5 = 0;
    assert_eq!(juge_t(t), Err(Violation::BarriereAbsente));
}

#[test]
fn l_indice_utilise_qui_reboucle_reste_une_avance_d_une_entree() {
    // dernier_util = 65535, used.idx = 0 : avance 1 (modulo 2^16).
    let avance = 0u16.wrapping_sub(65535);
    assert_eq!(avance, 1);
    let mut t = conforme();
    t.0 = avance;
    assert_eq!(juge_t(t), Ok(()));
}

#[test]
fn la_barriere_est_posee_sans_ecraser_les_autres_drapeaux() {
    let mut requete = [0u8; 56];
    requete[4] = 0x02; // un autre drapeau, deja present
    pose_barriere(&mut requete, 0x0102_0304_0506_0708);
    let drapeaux = u32::from_le_bytes([requete[4], requete[5], requete[6], requete[7]]);
    assert_eq!(drapeaux, 0x03);
    let mut b = [0u8; 8];
    b.copy_from_slice(&requete[8..16]);
    assert_eq!(u64::from_le_bytes(b), 0x0102_0304_0506_0708);
    // Le corps de la requete n'est pas touche.
    assert!(requete[16..].iter().all(|&o| o == 0));
}

#[test]
fn une_reponse_invalide_a_le_poids_d_une_commande_sans_reponse() {
    assert_eq!(issue(Err(REPONSE_INVALIDE)), Issue::SansReponse);
    // Detachement juge invalide : la memoire n'est JAMAIS rendue.
    let attachee = apres_attache(Memoire::NonAttachee, issue(Ok(REPONSE_OK)));
    let etat = apres_detache(attachee, issue(Err(REPONSE_INVALIDE)));
    assert_eq!(etat, Memoire::Inconnue);
    assert!(!peut_rendre(etat));
    // Attache jugee invalide : peut-etre retenue par le peripherique.
    let etat = apres_attache(Memoire::NonAttachee, issue(Err(REPONSE_INVALIDE)));
    assert_eq!(etat, Memoire::Inconnue);
    assert!(!peut_rendre(etat));
    // Transfert juge invalide sur une memoire attachee : il peut encore la
    // lire.
    let etat = apres_autre(attachee, issue(Err(REPONSE_INVALIDE)));
    assert_eq!(etat, Memoire::Inconnue);
}

#[test]
fn chaque_violation_porte_un_nom_distinct() {
    let noms = [
        Violation::TropDEntrees { avance: 2 }.nom(),
        Violation::IdInattendu { attendu: 0, recu: 2 }.nom(),
        Violation::ReponseCourte { len: 0 }.nom(),
        Violation::ReponseTropLongue { len: 1, capacite: 0 }.nom(),
        Violation::BarriereAbsente.nom(),
        Violation::BarriereDifferente { attendue: 1, recue: 2 }.nom(),
    ];
    for (i, a) in noms.iter().enumerate() {
        for b in &noms[i + 1..] {
            assert_ne!(a, b);
        }
    }
}
