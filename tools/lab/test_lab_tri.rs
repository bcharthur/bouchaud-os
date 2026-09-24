//! Les deux piles peuvent-elles partager une carte sans s'envoyer de RST ?
//!
//! # Le piege que ces epreuves defendent
//!
//! La machine porte deux piles sur une seule carte. Le routage donnait a
//! smoltcp une copie de CHAQUE trame brute, avant tout tri. C'est juste tant
//! qu'il n'y a qu'une pile ; des qu'une seconde ecoute, cela devient un RST
//! fratricide : smoltcp recoit un segment destine au port 2222, ne trouve
//! aucune chaussette, et repond `RST`. Le PC voit sa connexion au debugger
//! refusee -- par la machine elle-meme, au moment precis ou l'on cherche a
//! savoir ce qu'elle a.
//!
//! Une pile TCP ne jette pas silencieusement : elle REPOND. Le tri ne peut
//! donc pas se faire par soustraction.
//!
//! Lance par `tools/ci/run_host_tests.sh`.

#[path = "../../src/net/diag_distant/adresse.rs"]
mod adresse;
#[path = "../../src/net/diag_distant/tri.rs"]
mod tri;

use tri::{destination, Config, Motif, Pile, ETHERTYPE_ARP, ETHERTYPE_IPV4,
          PROTO_ICMP, PROTO_TCP, PROTO_UDP};

const MAC: [u8; 6] = [0xb0, 0x41, 0x6f, 0x09, 0x70, 0xa1];
const MAC_PC: [u8; 6] = [0x02, 0x11, 0x22, 0x33, 0x44, 0x55];
const IP_LAB: [u8; 4] = [169, 254, 112, 33];
const IP_BAIL: [u8; 4] = [192, 168, 1, 97];
const IP_PC: [u8; 4] = [192, 168, 1, 50];

fn cfg() -> Config {
    Config {
        ip_diagnostic: IP_LAB,
        mac: MAC,
        port_brdp: 2222,
        port_telemetrie: 2223,
        lab_actif: true,
    }
}

fn eth(dst: [u8; 6], src: [u8; 6], ethertype: u16, charge: &[u8]) -> Vec<u8> {
    let mut t = Vec::new();
    t.extend_from_slice(&dst);
    t.extend_from_slice(&src);
    t.extend_from_slice(&ethertype.to_be_bytes());
    t.extend_from_slice(charge);
    t
}

fn arp(cible: [u8; 4], diffusion: bool) -> Vec<u8> {
    let mut a = Vec::new();
    a.extend_from_slice(&[0, 1]); // ethernet
    a.extend_from_slice(&[0x08, 0x00]); // ipv4
    a.push(6);
    a.push(4);
    a.extend_from_slice(&[0, 1]); // request
    a.extend_from_slice(&MAC_PC);
    a.extend_from_slice(&IP_PC);
    a.extend_from_slice(&[0; 6]);
    a.extend_from_slice(&cible);
    eth(if diffusion { [0xFF; 6] } else { MAC }, MAC_PC, ETHERTYPE_ARP, &a)
}

fn ip(dst: [u8; 4], proto: u8, transport: &[u8]) -> Vec<u8> {
    let mut p = vec![0x45, 0, 0, 0, 0, 0, 0, 0, 64, proto, 0, 0];
    p.extend_from_slice(&IP_PC);
    p.extend_from_slice(&dst);
    p.extend_from_slice(transport);
    let total = p.len() as u16;
    p[2..4].copy_from_slice(&total.to_be_bytes());
    eth(MAC, MAC_PC, ETHERTYPE_IPV4, &p)
}

fn tcp(src_port: u16, dst_port: u16) -> Vec<u8> {
    let mut t = Vec::new();
    t.extend_from_slice(&src_port.to_be_bytes());
    t.extend_from_slice(&dst_port.to_be_bytes());
    t.extend_from_slice(&[0; 16]);
    t
}

fn udp(src_port: u16, dst_port: u16) -> Vec<u8> {
    let mut u = Vec::new();
    u.extend_from_slice(&src_port.to_be_bytes());
    u.extend_from_slice(&dst_port.to_be_bytes());
    u.extend_from_slice(&[0, 8, 0, 0]);
    u
}

// ===========================================================================
// AUCUN RST FRATRICIDE
// ===========================================================================

#[test]
fn la_pile_normale_ne_voit_jamais_une_connexion_au_debugger() {
    // L'EPREUVE CENTRALE. Si smoltcp voit ce segment, il repond RST, et le
    // debugger devient inaccessible au moment precis ou il sert.
    let t = ip(IP_LAB, PROTO_TCP, &tcp(51000, 2222));
    let v = destination(&cfg(), &t);
    assert_eq!(v.pile, Pile::Diagnostic);
    assert_eq!(v.motif, Motif::TcpBrdp);
    assert_eq!(v.port, 2222);
    assert!(!v.pour_normale(), "smoltcp repondrait RST au port 2222");
    assert!(v.pour_diagnostic());
}

#[test]
fn la_pile_de_diagnostic_ne_voit_jamais_le_trafic_du_navigateur() {
    // La reciproque, et elle compte autant : une pile de diagnostic qui
    // verrait une connexion HTTPS y repondrait RST, et le navigateur
    // tomberait a cause du debugger.
    for port in [80u16, 443, 53, 8080] {
        let t = ip(IP_BAIL, PROTO_TCP, &tcp(port, 51000));
        let v = destination(&cfg(), &t);
        assert_eq!(v.pile, Pile::Normale, "port {port}");
        assert!(!v.pour_diagnostic(), "le diagnostic repondrait RST au port {port}");
    }
}

#[test]
fn aucune_trame_ne_va_aux_deux_piles_en_tcp() {
    // Donner un segment TCP aux deux piles, c'est garantir qu'au moins l'une
    // des deux repondra RST. Il n'existe aucun cas ou c'est souhaitable.
    let cas = [
        ip(IP_LAB, PROTO_TCP, &tcp(51000, 2222)),
        ip(IP_LAB, PROTO_TCP, &tcp(51000, 9999)),
        ip(IP_BAIL, PROTO_TCP, &tcp(443, 51000)),
        ip(IP_PC, PROTO_TCP, &tcp(1, 2)),
    ];
    for t in cas {
        let v = destination(&cfg(), &t);
        assert_ne!(v.pile, Pile::LesDeux, "un segment TCP ne se duplique jamais : {v:?}");
    }
}

#[test]
fn un_port_inconnu_sur_l_adresse_lab_reste_au_diagnostic() {
    // Le RST est alors LEGITIME -- rien n'ecoute ce port -- mais il doit
    // venir de la pile de diagnostic, qui est celle qui possede l'adresse.
    // Laisser smoltcp le rendre reviendrait a lui faire prendre en charge une
    // adresse qu'il ne connait pas.
    let t = ip(IP_LAB, PROTO_TCP, &tcp(51000, 9999));
    let v = destination(&cfg(), &t);
    assert_eq!(v.pile, Pile::Diagnostic);
    assert_eq!(v.port, 9999);
}

// ===========================================================================
// ARP
// ===========================================================================

#[test]
fn un_arp_pour_l_adresse_lab_va_au_diagnostic_seul() {
    // La pile normale ne sait pas que cette adresse existe. Elle ne
    // repondrait pas, et le PC conclurait que la machine est morte.
    let v = destination(&cfg(), &arp(IP_LAB, true));
    assert_eq!(v.pile, Pile::Diagnostic);
    assert_eq!(v.motif, Motif::ArpDiagnostic);
}

#[test]
fn un_arp_de_diffusion_pour_une_autre_adresse_va_aux_deux() {
    // ARP n'ouvre pas de connexion : dupliquer ne peut produire aucun RST, et
    // une reponse ARP est unique et jamais retransmise. La perdre coute la
    // resolution entiere, et l'echec est mis en cache.
    let v = destination(&cfg(), &arp(IP_BAIL, true));
    assert_eq!(v.pile, Pile::LesDeux);
    assert!(v.pour_normale() && v.pour_diagnostic());
}

#[test]
fn un_arp_tronque_n_est_jamais_jete() {
    // Trancher sur une supposition ferait perdre du service de lien. On donne
    // aux deux : c'est sans risque, et c'est ce qui coute le moins cher si
    // l'on s'est trompe.
    let court = eth([0xFF; 6], MAC_PC, ETHERTYPE_ARP, &[0; 10]);
    let v = destination(&cfg(), &court);
    assert_eq!(v.pile, Pile::LesDeux);
}

// ===========================================================================
// LES AUTRES CHEMINS
// ===========================================================================

#[test]
fn l_icmp_vers_l_adresse_lab_va_au_diagnostic() {
    // `ping 169.254.x.y` depuis le PC doit repondre meme sans DHCP : c'est le
    // premier geste de quiconque cherche a savoir si la machine est vivante.
    let v = destination(&cfg(), &ip(IP_LAB, PROTO_ICMP, &[8, 0, 0, 0]));
    assert_eq!(v.pile, Pile::Diagnostic);
    assert_eq!(v.motif, Motif::IcmpDiagnostic);
}

#[test]
fn la_telemetrie_udp_va_au_diagnostic() {
    let v = destination(&cfg(), &ip(IP_LAB, PROTO_UDP, &udp(40000, 2223)));
    assert_eq!(v.pile, Pile::Diagnostic);
    assert_eq!(v.motif, Motif::UdpTelemetrie);
}

#[test]
fn le_dhcp_en_diffusion_continue_d_aller_a_la_pile_normale() {
    // UN OFFER DHCP ARRIVE EN DIFFUSION. Le detourner vers le diagnostic
    // couperait la configuration reseau de la machine -- le LAB casserait
    // precisement ce qu'il est cense aider a reparer.
    let v = destination(&cfg(), &ip([255, 255, 255, 255], PROTO_UDP, &udp(67, 68)));
    assert!(v.pour_normale(), "l'OFFER DHCP doit atteindre la pile normale");
    assert_eq!(v.motif, Motif::IpDiffusion);
}

#[test]
fn le_multicast_va_aux_deux() {
    let v = destination(&cfg(), &ip([224, 0, 0, 251], PROTO_UDP, &udp(5353, 5353)));
    assert!(v.pour_normale());
}

#[test]
fn l_ipv6_et_le_vlan_ne_vont_a_personne() {
    for ethertype in [0x86DDu16, 0x8100, 0x8808] {
        let v = destination(&cfg(), &eth(MAC, MAC_PC, ethertype, &[0; 40]));
        assert_eq!(v.pile, Pile::Aucune, "ethertype {ethertype:#06x}");
    }
}

#[test]
fn une_trame_trop_courte_ne_va_a_personne() {
    assert_eq!(destination(&cfg(), &[0u8; 13]).pile, Pile::Aucune);
    assert_eq!(destination(&cfg(), &[]).pile, Pile::Aucune);
}

// ===========================================================================
// LE LAB ETEINT NE CHANGE RIEN
// ===========================================================================

#[test]
fn lab_eteint_tout_va_a_la_pile_normale() {
    // LA SEULE FACON HONNETE DE DIRE « le LAB ne change rien quand il n'est
    // pas la ». Sans cette regle, une machine avec le LAB desactive se
    // comporterait quand meme differemment, et la comparaison avec les
    // campagnes precedentes ne vaudrait plus rien.
    let mut c = cfg();
    c.lab_actif = false;
    for t in [
        ip(IP_LAB, PROTO_TCP, &tcp(51000, 2222)),
        ip(IP_LAB, PROTO_UDP, &udp(40000, 2223)),
        ip(IP_LAB, PROTO_ICMP, &[8, 0, 0, 0]),
        ip(IP_BAIL, PROTO_TCP, &tcp(443, 51000)),
    ] {
        let v = destination(&c, &t);
        assert!(v.pour_normale(), "{v:?}");
        assert!(!v.pour_diagnostic(), "{v:?}");
    }
    // L'ARP visant l'adresse LAB redevient un ARP ordinaire.
    let v = destination(&c, &arp(IP_LAB, true));
    assert!(v.pour_normale());
}

// ===========================================================================
// L'ADRESSE LINK-LOCAL
// ===========================================================================

#[test]
fn l_adresse_derivee_est_toujours_utilisable_au_sens_de_la_rfc() {
    // `169.254.0.0/24` et `169.254.255.0/24` sont reserves. Une adresse qui y
    // tomberait serait ignoree par les piles conformes -- donc silencieusement
    // inutilisable, le pire des echecs pour un canal d'enquete.
    for a in 0..=255u8 {
        for b in [0u8, 1, 127, 128, 254, 255] {
            let mac = [a, b, a ^ b, b.wrapping_add(a), a, b];
            let ip = adresse::depuis_mac(mac);
            assert!(
                adresse::est_utilisable(ip),
                "MAC {mac:02x?} -> {ip:?}, hors de la plage utilisable",
            );
            assert_eq!(ip[0], 169);
            assert_eq!(ip[1], 254);
            assert!((1..=254).contains(&ip[2]), "{ip:?}");
        }
    }
}

#[test]
fn la_meme_mac_donne_toujours_la_meme_adresse() {
    // Le PC de developpement doit connaitre l'adresse AVANT que la machine
    // demarre. Une adresse qui changerait d'une session a l'autre obligerait a
    // la decouvrir -- par le reseau, c'est-a-dire par la chose en panne.
    let mac = [0xb0, 0x41, 0x6f, 0x09, 0x70, 0xa1];
    let premiere = adresse::depuis_mac(mac);
    for _ in 0..100 {
        assert_eq!(adresse::depuis_mac(mac), premiere);
    }
}

#[test]
fn deux_cartes_voisines_ne_recoivent_pas_la_meme_adresse() {
    // Deux cartes du meme lot ne different souvent que par le dernier octet.
    // Prendre les deux derniers octets de la MAC tels quels les ferait entrer
    // en collision de facon systematique.
    let base = [0xb0, 0x41, 0x6f, 0x09, 0x70, 0x00];
    let mut vues = std::collections::HashSet::new();
    for n in 0..=255u8 {
        let mut mac = base;
        mac[5] = n;
        vues.insert(adresse::depuis_mac(mac));
    }
    assert!(
        vues.len() > 250,
        "256 cartes voisines ne produisent que {} adresses distinctes",
        vues.len(),
    );
}

#[test]
fn le_masque_est_un_seize() {
    // La RFC 3927 l'exige. Un /24 ferait sortir vers une passerelle
    // inexistante la moitie du segment link-local.
    assert_eq!(adresse::masque(), [255, 255, 0, 0]);
    assert_eq!(adresse::PREFIXE_BITS, 16);
}

#[test]
fn une_adresse_de_bail_n_est_pas_link_local() {
    assert!(!adresse::est_link_local([192, 168, 1, 97]));
    assert!(!adresse::est_utilisable([192, 168, 1, 97]));
    assert!(adresse::est_link_local([169, 254, 0, 1]));
    assert!(!adresse::est_utilisable([169, 254, 0, 1]), "le /24 zero est reserve");
    assert!(!adresse::est_utilisable([169, 254, 255, 1]), "le /24 255 est reserve");
}

#[test]
fn deux_adresses_link_local_sont_sur_le_meme_segment() {
    assert!(adresse::meme_segment([169, 254, 1, 1], [169, 254, 200, 9]));
    assert!(!adresse::meme_segment([169, 254, 1, 1], [192, 168, 1, 1]));
}
