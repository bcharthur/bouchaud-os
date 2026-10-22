//! Une memoire DMA confiee a virtio-gpu n'est jamais rendue tant que le
//! peripherique peut la tenir (BOUCHAUD_VIRTIO_GPU_DMA_ETAT_V1).
//!
//! Le scenario du defaut : `ATTACH_BACKING` part, l'attente bornee echoue.
//! L'ancien code tenait la memoire pour « jamais attachee » et la rendait.

#[path = "../../src/drivers/display/virtio_gpu_dma.rs"]
mod dma;

use dma::{apres_attache, apres_autre, apres_detache, issue, peut_rendre, Issue, Memoire, REPONSE_OK, SANS_REPONSE};

/// Ce que decidait `active_scanout` avant le correctif.
fn ancienne_decision(attachee: bool, liberee: bool) -> bool {
    !attachee || liberee
}

#[test]
fn attache_sans_reponse_la_memoire_n_est_jamais_rendue() {
    // La requete est partie, la reponse n'est pas venue.
    let rendu: Result<u32, &str> = Err(SANS_REPONSE);
    let etat = apres_attache(Memoire::NonAttachee, issue(rendu));
    assert_eq!(etat, Memoire::Inconnue);
    assert!(!peut_rendre(etat));

    // Le nettoyage qui suit ne peut plus rien envoyer (file cassee) : ni le
    // detachement ni la liberation ne changent l'etat.
    let etat = apres_detache(etat, issue(Err("file-sans-reponse")));
    let etat = apres_autre(etat, issue(Err("file-sans-reponse")));
    assert!(!peut_rendre(etat), "memoire rendue alors que le peripherique peut la tenir");

    // L'ancien code, lui, la rendait : `attachee` faux, liberation non envoyee.
    assert!(ancienne_decision(false, false), "le defaut n'est plus reproduit par le modele de l'ancien code");
}

#[test]
fn un_etat_inconnu_est_absorbant() {
    for i in [Issue::Acceptee, Issue::Refusee, Issue::SansReponse, Issue::NonEnvoyee] {
        assert_eq!(apres_attache(Memoire::Inconnue, i), Memoire::Inconnue);
        assert_eq!(apres_detache(Memoire::Inconnue, i), Memoire::Inconnue);
        assert_eq!(apres_autre(Memoire::Inconnue, i), Memoire::Inconnue);
    }
}

#[test]
fn refus_ou_non_envoi_de_l_attache_la_memoire_reste_a_nous() {
    assert!(peut_rendre(apres_attache(Memoire::NonAttachee, issue(Ok(0x1205)))));
    assert!(peut_rendre(apres_attache(Memoire::NonAttachee, issue(Err("file-sans-reponse")))));
    assert!(peut_rendre(apres_attache(Memoire::NonAttachee, issue(Err("commande-trop-longue")))));
}

#[test]
fn attachee_puis_detachee_la_memoire_peut_etre_rendue() {
    let etat = apres_attache(Memoire::NonAttachee, issue(Ok(REPONSE_OK)));
    assert_eq!(etat, Memoire::Attachee);
    assert!(!peut_rendre(etat));
    // Un SET_SCANOUT refuse ne change rien a la reference.
    let etat = apres_autre(etat, issue(Ok(0x1202)));
    assert_eq!(etat, Memoire::Attachee);
    let etat = apres_detache(etat, issue(Ok(REPONSE_OK)));
    assert!(peut_rendre(etat));
}

#[test]
fn attachee_et_detachement_refuse_ou_sans_reponse_jamais_rendue() {
    let attachee = apres_attache(Memoire::NonAttachee, issue(Ok(REPONSE_OK)));
    assert!(!peut_rendre(apres_detache(attachee, issue(Ok(0x1203)))));
    assert!(!peut_rendre(apres_detache(attachee, issue(Err(SANS_REPONSE)))));
    assert!(!peut_rendre(apres_detache(attachee, issue(Err("file-sans-reponse")))));
}

#[test]
fn transfert_sans_reponse_sur_memoire_attachee_devient_inconnue() {
    let attachee = apres_attache(Memoire::NonAttachee, issue(Ok(REPONSE_OK)));
    let etat = apres_autre(attachee, issue(Err(SANS_REPONSE)));
    assert_eq!(etat, Memoire::Inconnue);
    // ... et un detachement accepte ensuite ne la rend pas (absorbant).
    assert!(!peut_rendre(apres_detache(etat, issue(Ok(REPONSE_OK)))));
}

#[test]
fn classification_des_issues() {
    assert_eq!(issue(Ok(REPONSE_OK)), Issue::Acceptee);
    assert_eq!(issue(Ok(0x1203)), Issue::Refusee);
    assert_eq!(issue(Err(SANS_REPONSE)), Issue::SansReponse);
    assert_eq!(issue(Err("file-sans-reponse")), Issue::NonEnvoyee);
}
