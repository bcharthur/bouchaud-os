//! Preuve hote de l'analyse des options DHCP.
//!
//! # Ce qui se joue ici
//!
//! Ces octets viennent du reseau : ils sont, par definition, ecrits par
//! quelqu'un d'autre. Trois fautes y seraient invisibles a la relecture et
//! silencieuses a la compilation :
//!
//!   * une longueur d'option qui depasse le tampon fait lire la memoire
//!     d'apres ;
//!   * un masque de sous-reseau accepte sans verification donne un nom de
//!     reseau faux, et ce nom s'affiche dans la barre des taches ;
//!   * un octet de controle laisse passer dans le nom de domaine arrive
//!     jusqu'au rendu de texte.
//!
//! Les paquets ci-dessous sont fabriques a la main depuis la RFC 2132 (format
//! code/longueur/valeur, option 255 pour la fin, option 0 pour le
//! remplissage), et non recopies du module.

#![allow(dead_code)]

#[path = "../../src/net/application/dhcp/options.rs"]
mod options;

use options::*;

fn entete(msg_type: u8) -> Vec<u8> {
    let mut buf = vec![0u8; 240];
    buf[16..20].copy_from_slice(&[192, 168, 1, 42]);
    buf[236..240].copy_from_slice(&[99, 130, 83, 99]);
    buf.push(53);
    buf.push(1);
    buf.push(msg_type);
    buf
}

/// Le cas nominal d'une box grand public.
#[test]
fn un_ack_complet_rend_tout() {
    let mut buf = entete(5);
    buf.extend_from_slice(&[54, 4, 192, 168, 1, 1]);
    buf.extend_from_slice(&[1, 4, 255, 255, 255, 0]);
    buf.extend_from_slice(&[3, 4, 192, 168, 1, 1]);
    buf.extend_from_slice(&[6, 4, 192, 168, 1, 1]);
    buf.push(15);
    buf.push(9);
    buf.extend_from_slice(b"fritz.box");
    buf.push(255);

    let bail = parse_reply(&buf).expect("un ACK bien forme doit etre lu");
    assert_eq!(bail.msg_type, 5);
    assert_eq!(bail.your_ip, [192, 168, 1, 42]);
    assert_eq!(bail.server_id, [192, 168, 1, 1]);
    assert_eq!(bail.masque, [255, 255, 255, 0]);
    assert_eq!(&bail.domaine[..bail.domaine_len], b"fritz.box");
    assert_eq!(longueur_prefixe(bail.masque), Some(24));
    assert_eq!(adresse_reseau(bail.your_ip, bail.masque), [192, 168, 1, 0]);
}

/// Une option dont la longueur annoncee depasse le paquet.
///
/// C'est le paquet hostile le plus simple qui soit. L'analyse doit s'arreter,
/// pas lire ce qui suit en memoire.
#[test]
fn une_longueur_mensongere_arrete_l_analyse() {
    let mut buf = entete(5);
    buf.extend_from_slice(&[1, 4, 255, 255, 255, 0]);
    buf.extend_from_slice(&[15, 250]);
    buf.extend_from_slice(b"ab");
    let bail = parse_reply(&buf).expect("ce qui precede reste valide");
    assert_eq!(bail.masque, [255, 255, 255, 0]);
    assert_eq!(bail.domaine_len, 0);
}

/// Un nom de domaine plus long que ce qu'on garde.
#[test]
fn un_nom_trop_long_est_tronque() {
    let mut buf = entete(5);
    let long = vec![b'z'; 120];
    buf.push(15);
    buf.push(120);
    buf.extend_from_slice(&long);
    buf.push(255);
    let bail = parse_reply(&buf).unwrap();
    assert_eq!(bail.domaine_len, LONGUEUR_DOMAINE);
    assert!(bail.domaine[..bail.domaine_len].iter().all(|o| *o == b'z'));
}

/// Un nom qui porte une sequence d'echappement ANSI.
#[test]
fn les_octets_de_controle_ne_passent_pas() {
    let mut buf = entete(5);
    let charge = b"\x1b[31mrouge\x00";
    buf.push(15);
    buf.push(charge.len() as u8);
    buf.extend_from_slice(charge);
    buf.push(255);
    let bail = parse_reply(&buf).unwrap();
    let nom = &bail.domaine[..bail.domaine_len];
    assert!(!nom.contains(&0x1b), "pas d'echappement dans un nom de reseau");
    assert!(!nom.contains(&0x00), "pas d'octet nul dans un nom de reseau");
    assert_eq!(nom, b"[31mrouge");
}

/// Les masques que personne n'ecrit mais que n'importe qui peut envoyer.
#[test]
fn un_masque_a_trous_ne_nomme_pas_un_reseau() {
    assert_eq!(longueur_prefixe([255, 0, 255, 0]), None);
    assert_eq!(longueur_prefixe([0, 255, 255, 255]), None);
    assert_eq!(longueur_prefixe([255, 255, 255, 1]), None);
    assert_eq!(longueur_prefixe([0, 0, 0, 0]), None);
}

/// Et ceux qui en nomment un.
#[test]
fn les_masques_valides_donnent_leur_prefixe() {
    assert_eq!(longueur_prefixe([128, 0, 0, 0]), Some(1));
    assert_eq!(longueur_prefixe([255, 0, 0, 0]), Some(8));
    assert_eq!(longueur_prefixe([255, 255, 248, 0]), Some(21));
    assert_eq!(longueur_prefixe([255, 255, 255, 252]), Some(30));
    assert_eq!(longueur_prefixe([255, 255, 255, 255]), Some(32));
}

/// Le message construit doit porter les options qu'on pretend demander.
///
/// Le nom de reseau vient de l'option 15 : si la requete cesse de la demander,
/// la reponse cesse de la porter, et l'icone perd son libelle sans qu'aucune
/// ligne de code du rendu n'ait change.
#[test]
fn la_requete_demande_bien_le_nom_de_domaine() {
    let mut buf = [0u8; 400];
    let taille = build_msg(&mut buf, 0x1234_5678, [1, 2, 3, 4, 5, 6], 1, None, None);
    assert!(taille >= 300);
    assert_eq!(&buf[236..240], &[99, 130, 83, 99], "cookie magique");
    // Liste des parametres souhaites : code 55.
    let position = buf[..taille]
        .windows(2)
        .position(|f| f[0] == 55)
        .expect("la liste des parametres souhaites doit etre presente");
    let longueur = buf[position + 1] as usize;
    let demandes = &buf[position + 2..position + 2 + longueur];
    assert!(demandes.contains(&1), "masque de sous-reseau");
    assert!(demandes.contains(&3), "routeur");
    assert!(demandes.contains(&6), "resolveur");
    assert!(demandes.contains(&15), "nom de domaine");
}

/// Un paquet sans le cookie magique n'est pas du DHCP.
#[test]
fn sans_cookie_rien_n_est_lu() {
    let mut buf = entete(5);
    buf[238] = 0;
    assert!(parse_reply(&buf).is_none());
}
