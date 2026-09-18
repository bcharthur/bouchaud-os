//! L'ECHELLE DE LA REPONSE DNS, A L'HOTE.
//!
//! # Ce que l'archive du 24 septembre ne permettait pas de dire
//!
//! `bb(7)` prouve la carte et DHCP : 533 trames, huit tours d'anneau, un bail
//! reel. La requete part -- `M17_UDP_TX dst=192.168.1.254:53 parti=true` --
//! et rien ne revient. Aucun des trois marqueurs de livraison n'apparait,
//! donc la livraison n'a jamais ete TENTEE. Mais rien ne disait lequel des
//! cinq etages en amont avait laisse tomber le datagramme.
//!
//! Ce banc verifie la seule chose qui se teste sans machine : que l'echelle
//! nomme le bon etage pour chaque panne possible. Le verdict est ce que la
//! prochaine archive rendra en une phrase.

#[path = "../../src/net/sonde_dns.rs"]
mod sonde_dns;

use sonde_dns::{compte, concerne, note, verdict_de, Barreau, BARREAUX};

/// Un releve ou les barreaux jusqu'a `dernier` inclus ont ete franchis.
///
/// Les compteurs du module sont globaux ; un banc qui s'appuierait dessus
/// changerait de resultat selon l'ordre d'execution du harnais. On teste donc
/// la REGLE, sur des releves construits ici.
fn jusqu_a(dernier: Barreau) -> [u64; BARREAUX] {
    let mut c = [0u64; BARREAUX];
    for rang in 0..=(dernier as usize) {
        let Some(barreau) = Barreau::depuis_rang(rang) else { continue };
        // Ce sont des PERTES, pas des etages franchis.
        if barreau == Barreau::SocketOccupe || barreau == Barreau::RecvVide {
            continue;
        }
        c[rang] = 1;
    }
    c
}

#[test]
fn l_echelle_nomme_chaque_etage_dans_l_ordre() {
    // Chaque panne possible, et la phrase qu'elle doit produire. C'est ce que
    // l'archive du 24 septembre ne pouvait pas dire : tous ces cas y avaient
    // la meme apparence -- le silence.
    let rien = [0u64; BARREAUX];
    assert_eq!(verdict_de(&rien), "aucune trame port 53 n'est arrivee sur la carte");
    assert_eq!(
        verdict_de(&jusqu_a(Barreau::RxEthernet)),
        "la trame arrive mais route_ipv4 la rejette"
    );
    assert_eq!(
        verdict_de(&jusqu_a(Barreau::RxIpv4)),
        "l'en-tete IPv4 passe mais udp::parse echoue"
    );
    assert_eq!(
        verdict_de(&jusqu_a(Barreau::RxUdp)),
        "le datagramme n'est jamais mis en file"
    );
    assert_eq!(
        verdict_de(&jusqu_a(Barreau::MisEnFile)),
        "il est en file mais jamais reclame"
    );
    assert_eq!(
        verdict_de(&jusqu_a(Barreau::SortiDeFile)),
        "il sort de la file mais aucune socket ne correspond"
    );
    assert_eq!(
        verdict_de(&jusqu_a(Barreau::SocketTrouve)),
        "la socket est trouvee mais rien n'est livre"
    );
    assert_eq!(
        verdict_de(&jusqu_a(Barreau::SocketLivre)),
        "livre, mais poll ne voit jamais la socket prete"
    );
    assert_eq!(
        verdict_de(&jusqu_a(Barreau::PollPret)),
        "livre et poll pret, mais recvfrom ne rend rien"
    );
    assert_eq!(verdict_de(&jusqu_a(Barreau::RecvSucces)), "la chaine est complete");
}

#[test]
fn la_socket_occupee_se_distingue_d_une_socket_muette() {
    // LE CAS QUE `livre_datagramme` PEUT ENCORE PRODUIRE : socket trouvee,
    // `try_lock` echoue, datagramme jete. Tant qu'il n'est pas MESURE sur la
    // machine, on ne remplace pas cette perte par une file -- on la rend
    // visible, et on la distingue d'une socket qui n'a simplement rien recu.
    let mut occupee = jusqu_a(Barreau::SocketTrouve);
    occupee[Barreau::SocketOccupe as usize] = 1;
    assert_eq!(
        verdict_de(&occupee),
        "la socket est trouvee mais occupee : datagramme perdu"
    );
    assert_eq!(
        verdict_de(&jusqu_a(Barreau::SocketTrouve)),
        "la socket est trouvee mais rien n'est livre"
    );
}

#[test]
fn la_sonde_ne_regarde_que_le_port_53() {
    // Un compteur sur tout l'UDP melangerait le mDNS et le SSDP d'un reseau
    // domestique -- plusieurs centaines de trames dans le releve -- aux quatre
    // datagrammes qui nous interessent.
    assert!(concerne(53, 49985));
    assert!(concerne(49985, 53));
    assert!(!concerne(5353, 5353)); // mDNS
    assert!(!concerne(1900, 1900)); // SSDP
    assert!(!concerne(68, 67)); // DHCP
}

#[test]
fn chaque_barreau_a_le_nom_attendu_dans_l_archive() {
    // Les noms sont un contrat avec l'extracteur et avec l'oeil qui lit
    // l'archive : les changer en silence rendrait les anciennes archives
    // illisibles.
    let attendus = [
        "dns53_rx_ethernet", "dns53_rx_ipv4", "dns53_rx_udp",
        "dns53_queued_ip", "dns53_dequeued_ip", "dns53_socket_match",
        "dns53_socket_busy", "dns53_socket_delivered", "dns53_poll_ready",
        "dns53_recv_success", "dns53_recv_eagain",
    ];
    for (rang, attendu) in attendus.iter().enumerate() {
        assert_eq!(Barreau::depuis_rang(rang).unwrap().nom(), *attendu);
    }
    assert!(Barreau::depuis_rang(BARREAUX).is_none());
}

#[test]
fn seule_une_reponse_est_decrite_et_une_requete_ne_l_est_pas() {
    // Une requete SORTANTE va vers le port 53 ; elle n'apprend rien sur le
    // chemin de retour, et la decrire volerait la place de la reponse.
    assert!(!sonde_dns::decris_une_fois([192, 168, 1, 97], [192, 168, 1, 254], 49985, 53, 32, 0));
    assert!(sonde_dns::detail().is_none());

    // La premiere reponse, elle, est retenue.
    assert!(sonde_dns::decris_une_fois(
        [192, 168, 1, 254], [192, 168, 1, 97], 53, 49985, 48,
        sonde_dns::SOMME_IPV4_JUSTE | sonde_dns::SOMME_UDP_JUSTE,
    ));
    let (src, dst, sp, dp, longueur, sommes) = sonde_dns::detail().expect("retenue");
    assert_eq!(src, [192, 168, 1, 254]);
    assert_eq!(dst, [192, 168, 1, 97]);
    assert_eq!((sp, dp, longueur), (53, 49985, 48));
    assert_ne!(sommes & sonde_dns::SOMME_IPV4_JUSTE, 0);

    // Et UNE SEULE : le but est de pouvoir lire une ligne, pas de remplir
    // l'archive.
    assert!(!sonde_dns::decris_une_fois([1, 1, 1, 1], [2, 2, 2, 2], 53, 1, 1, 0));
    assert_eq!(sonde_dns::detail().unwrap().0, [192, 168, 1, 254]);
}

#[test]
fn compte_rend_bien_ce_qui_a_ete_note() {
    let avant = compte(Barreau::RecvVide);
    note(Barreau::RecvVide);
    assert_eq!(compte(Barreau::RecvVide), avant + 1);
}

// ===========================================================================
// LE BARREAU QUI NE POUVAIT PAS S'ALLUMER
//
// La premiere version de cette echelle posait ses trois premiers barreaux
// DANS la branche `if let Some(u) = udp::parse(...)`, elle-meme placee apres
// un `parse_header` reussi. Trois verdicts devenaient inatteignables :
//
//     « la trame arrive mais route_ipv4 la rejette »
//     « l'en-tete IPv4 passe mais udp::parse echoue »
//
// et surtout, un paquet refuse par l'un des deux analyseurs produisait
// « aucune trame port 53 n'est arrivee sur la carte » -- c'est-a-dire
// exactement le contraire de la verite, et la reponse qui ferait remonter
// l'enquete vers la carte et le reseau externe.
//
// `ports_bruts` est la correction : il ne demande que ce qu'il faut pour
// trouver les ports, avec des bornes et aucune exigence de coherence.
// ===========================================================================

use sonde_dns::ports_bruts;

/// Un paquet IPv4/UDP minimal : en-tete de vingt octets, puis UDP.
fn paquet(src_port: u16, dst_port: u16, total_len: u16, longueur_udp: u16) -> Vec<u8> {
    let mut p = vec![0u8; 20 + 8 + 4];
    p[0] = 0x45; // version 4, IHL 5 mots
    p[2] = (total_len >> 8) as u8;
    p[3] = total_len as u8;
    p[9] = 17; // UDP
    p[20] = (src_port >> 8) as u8;
    p[21] = src_port as u8;
    p[22] = (dst_port >> 8) as u8;
    p[23] = dst_port as u8;
    p[24] = (longueur_udp >> 8) as u8;
    p[25] = longueur_udp as u8;
    p
}

#[test]
fn les_ports_se_lisent_sur_un_paquet_bien_forme() {
    let p = paquet(53, 49985, 32, 12);
    assert_eq!(ports_bruts(&p), Some((53, 49985)));
}

#[test]
fn les_ports_se_lisent_meme_quand_la_longueur_ipv4_est_incoherente() {
    // `parse_header` refuse ce paquet -- `total_len` annonce plus que le
    // tampon. Le barreau doit s'allumer QUAND MEME : la trame est bien
    // arrivee, et c'est tout ce qu'il affirme.
    let p = paquet(53, 49985, 9000, 12);
    assert_eq!(ports_bruts(&p), Some((53, 49985)));
}

#[test]
fn les_ports_se_lisent_meme_quand_la_longueur_udp_est_incoherente() {
    // `udp::parse` refuse une longueur superieure au tampon, ou inferieure a
    // huit. Le barreau reste allume : c'est precisement la difference entre
    // « jamais arrivee » et « arrivee et refusee par udp::parse ».
    assert_eq!(ports_bruts(&paquet(53, 49985, 32, 9000)), Some((53, 49985)));
    assert_eq!(ports_bruts(&paquet(53, 49985, 32, 3)), Some((53, 49985)));
}

#[test]
fn un_paquet_qui_n_est_pas_udp_ne_concerne_pas_la_sonde() {
    let mut p = paquet(53, 49985, 32, 12);
    p[9] = 6; // TCP
    assert_eq!(ports_bruts(&p), None);
}

#[test]
fn un_paquet_tronque_ne_rend_pas_de_ports_inventes() {
    // Mieux vaut ne rien dire que lire quatre octets hors du tampon.
    let p = paquet(53, 49985, 32, 12);
    assert_eq!(ports_bruts(&p[..19]), None);
    assert_eq!(ports_bruts(&p[..22]), None);
    assert_eq!(ports_bruts(&[]), None);
}

#[test]
fn une_version_ou_une_taille_d_en_tete_absurde_est_refusee() {
    let mut p = paquet(53, 49985, 32, 12);
    p[0] = 0x65; // version 6
    assert_eq!(ports_bruts(&p), None);
    let mut p = paquet(53, 49985, 32, 12);
    p[0] = 0x43; // IHL 3 mots : en dessous du minimum
    assert_eq!(ports_bruts(&p), None);
}

#[test]
fn les_options_ipv4_decalent_la_lecture_des_ports() {
    // IHL de six mots : les ports sont quatre octets plus loin. Les lire a
    // l'offset fixe donnerait deux nombres pris dans les options.
    let mut p = vec![0u8; 24 + 8];
    p[0] = 0x46;
    p[9] = 17;
    p[24] = 0x00;
    p[25] = 53;
    p[26] = 0xC3;
    p[27] = 0x41;
    assert_eq!(ports_bruts(&p), Some((53, 0xC341)));
}

#[test]
fn seul_le_port_53_allume_la_sonde() {
    // Le mDNS et le SSDP d'un reseau domestique -- des centaines de trames
    // dans le releve -- ne doivent pas franchir un seul barreau.
    let (s, d) = ports_bruts(&paquet(5353, 5353, 32, 12)).unwrap();
    assert!(!concerne(s, d));
    let (s, d) = ports_bruts(&paquet(53, 49985, 32, 12)).unwrap();
    assert!(concerne(s, d));
}
