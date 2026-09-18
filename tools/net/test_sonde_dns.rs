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
