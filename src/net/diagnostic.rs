//! `netdiag` : la preuve physique que le reseau tient DANS LA DUREE.
//!
//! # Ce que ce banc mesure, et pourquoi il fallait l'ecrire
//!
//! Le releve TRIGKEY du 17 septembre montre un reseau qui vit deux secondes
//! puis meurt :
//!
//! ```text
//! [NET-LIEN]    lien=1 vitesse_mbps=1000 duplex=complet trames_perdues=0
//! [NET-ROUTAGE] trames=104 arp=11 dhcp=2 arp_resolus=1 arp_echoues=2
//! ```
//!
//! UNE resolution ARP reussie, puis deux echecs, puis plus rien. Toutes les
//! commandes existantes -- `arping`, `ping`, `dns` -- posent UNE question et
//! s'arretent. Aucune ne pouvait voir la difference entre « cela marche » et
//! « cela a marche une fois ».
//!
//! `netdiag` pose la meme question cent fois de suite, pendant plusieurs
//! minutes, en OUBLIANT VOLONTAIREMENT la reponse entre chaque tour.
//!
//! # Ce qu'il ne fait pas
//!
//! Il ne code en dur aucune adresse materielle, aucune adresse IP de
//! passerelle, aucun serveur de noms. Tout vient de la configuration obtenue
//! par DHCP. Une machine posee demain sur un autre reseau Ethernet doit faire
//! tourner ce banc sans qu'une ligne change -- c'est le seul test qui vaille.
//!
//! Il n'allonge pas non plus la duree de vie du cache ARP. C'est l'inverse :
//! il la contourne EN OUBLIANT, ce qui rend chaque tour reel. Un cache
//! eternel ferait passer ce banc au vert sur une carte dont la reception est
//! morte, ce qui est exactement la panne a attraper.

use alloc::vec::Vec;

use crate::net::{self, ipv4, Ipv4Addr};

/// Duree par defaut du banc, en secondes.
///
/// Trois minutes : assez pour traverser trois fois la duree de vie d'une
/// entree ARP positive, et assez pour qu'une reception qui meurt apres deux
/// secondes n'ait aucune chance de passer inapercue.
const SECONDES_DEFAUT: u64 = 180;

/// Pause entre deux tours ARP, en millisecondes.
///
/// Cent vingt tours en cent quatre-vingts secondes : au-dessus du minimum de
/// cent demande, avec de la marge pour les tours plus lents.
const PAUSE_TOUR_MS: u64 = 1_200;

/// Periode des epreuves de couche 3 et 4, en secondes.
const PERIODE_L34_S: u64 = 20;

/// Nom resolu pour l'epreuve DNS, et hote des deux connexions TCP.
///
/// Un NOM, pas une adresse : coder une adresse en dur ferait passer
/// l'epreuve DNS sans que DNS fonctionne, et l'epreuve TCP sur une machine
/// qui n'est peut-etre plus la.
const HOTE_EPREUVE: &str = "example.com";

/// Ce que chaque famille d'epreuve a donne.
#[derive(Clone, Copy, Default)]
struct Bilan {
    tours: u64,
    succes: u64,
    echecs: u64,
    ms_min: u64,
    ms_max: u64,
    ms_total: u64,
}

impl Bilan {
    fn note(&mut self, ok: bool, ms: u64) {
        self.tours += 1;
        if ok {
            self.succes += 1;
            self.ms_total = self.ms_total.saturating_add(ms);
            self.ms_max = self.ms_max.max(ms);
            self.ms_min = if self.ms_min == 0 { ms } else { self.ms_min.min(ms) };
        } else {
            self.echecs += 1;
        }
    }

    fn moyenne_ms(&self) -> u64 {
        if self.succes == 0 {
            0
        } else {
            self.ms_total / self.succes
        }
    }
}

fn maintenant_ms() -> u64 {
    crate::kernel::timer::monotonic_ms()
}

fn dort_ms(ms: u64) {
    crate::kernel::task::sleep_ticks(crate::kernel::timer::ms_to_ticks(ms));
}

/// Un tour ARP COMPLET : oublier, demander, verifier.
///
/// Rend `(reussi, duree_ms)`. L'oubli fait partie du tour : sans lui, les
/// tours deux a cent ne feraient que relire un cache.
fn tour_arp(cible: Ipv4Addr) -> (bool, u64) {
    net::oublie_voisin(cible);
    // L'OUBLI DOIT AVOIR EU LIEU. Sinon le tour ne mesure rien, et un banc qui
    // ne mesure rien passe toujours.
    if net::voisin_en_cache(cible).is_some() {
        return (false, 0);
    }
    let debut = maintenant_ms();
    let resultat = net::resout_voisin(cible);
    let duree = maintenant_ms().saturating_sub(debut);
    match resultat {
        // Une adresse nulle ou de diffusion n'est pas une resolution : c'est
        // un cache qui repond n'importe quoi.
        Some(mac) if mac.iter().any(|&o| o != 0) && mac.iter().any(|&o| o != 0xFF) => {
            (true, duree)
        }
        _ => (false, duree),
    }
}

/// Une epreuve DNS, chronometree.
fn tour_dns() -> (bool, u64, Option<Ipv4Addr>) {
    let debut = maintenant_ms();
    let adresse = net::resolve(HOTE_EPREUVE);
    let duree = maintenant_ms().saturating_sub(debut);
    (adresse.is_some(), duree, adresse)
}

/// Une connexion TCP, chronometree.
///
/// On envoie une requete minimale et on regarde s'il revient quelque chose :
/// c'est la seule facon, avec l'interface `fetch`, de prouver qu'une poignee
/// a bien eu lieu. Sur le port 443 la reponse est un enregistrement TLS
/// d'alerte -- le serveur refuse du clair --, et cela suffit : il a fallu
/// SYN, SYN-ACK et ACK pour l'obtenir.
fn tour_tcp(adresse: Ipv4Addr, port: u16) -> (bool, u64) {
    let requete = "HEAD / HTTP/1.0\r\nHost: example.com\r\nConnection: close\r\n\r\n";
    let mut reponse: Vec<u8> = Vec::new();
    let debut = maintenant_ms();
    let ok = crate::net::transport::smol_tcp::fetch(
        adresse,
        port,
        requete.as_bytes(),
        &mut reponse,
    );
    let duree = maintenant_ms().saturating_sub(debut);
    (ok && !reponse.is_empty(), duree)
}

/// Le banc, en une commande.
pub fn netdiag(argc: usize, argv: &[&str; 12]) {
    let secondes = if argc > 1 {
        argv[1].parse::<u64>().unwrap_or(SECONDES_DEFAUT)
    } else {
        SECONDES_DEFAUT
    };

    let passerelle = net::gateway();
    let serveur_noms = net::dns_server();
    if passerelle == [0, 0, 0, 0] {
        crate::println!(
            "netdiag: aucune passerelle configuree -- le reseau n'est pas monte."
        );
        crate::println!("         lance `dhcp` ou `ifup`, puis recommence.");
        return;
    }

    crate::println!(
        "netdiag: {} s, passerelle {}, dns {}, cache ARP {} ms",
        secondes,
        ipv4::format_addr(&passerelle),
        ipv4::format_addr(&serveur_noms),
        net::arp_ttl_ms(),
    );
    crate::println!("         chaque tour OUBLIE la passerelle avant de la redemander.");
    crate::serial_println!(
        "BOUCHAUD_NETDIAG_DEPART secondes={} passerelle={} dns={} arp_ttl_ms={}",
        secondes,
        ipv4::format_addr(&passerelle),
        ipv4::format_addr(&serveur_noms),
        net::arp_ttl_ms(),
    );

    let avant = crate::drivers::rtl8168::releve();
    let debut_ms = maintenant_ms();
    let fin_ms = debut_ms.saturating_add(secondes.saturating_mul(1_000));
    let mut arp = Bilan::default();
    let mut dns = Bilan::default();
    let mut tcp80 = Bilan::default();
    let mut tcp443 = Bilan::default();
    let mut prochaine_l34_ms = debut_ms;
    let mut derniere_adresse: Option<Ipv4Addr> = None;
    let mut serie_echecs = 0u64;
    let mut serie_echecs_max = 0u64;

    while maintenant_ms() < fin_ms {
        let (ok, ms) = tour_arp(passerelle);
        arp.note(ok, ms);
        if ok {
            serie_echecs = 0;
        } else {
            serie_echecs += 1;
            serie_echecs_max = serie_echecs_max.max(serie_echecs);
        }

        if maintenant_ms() >= prochaine_l34_ms {
            prochaine_l34_ms = maintenant_ms()
                .saturating_add(PERIODE_L34_S.saturating_mul(1_000));
            let (ok_dns, ms_dns, adresse) = tour_dns();
            dns.note(ok_dns, ms_dns);
            if let Some(a) = adresse {
                derniere_adresse = Some(a);
            }
            if let Some(a) = derniere_adresse {
                let (ok80, ms80) = tour_tcp(a, 80);
                tcp80.note(ok80, ms80);
                let (ok443, ms443) = tour_tcp(a, 443);
                tcp443.note(ok443, ms443);
            }
        }

        dort_ms(PAUSE_TOUR_MS);
    }

    let apres = crate::drivers::rtl8168::releve();
    let ecoule_s = maintenant_ms().saturating_sub(debut_ms) / 1_000;

    // LE CRITERE, ET IL EST DUR : cent resolutions, cent succes, zero echec.
    let verdict_arp = arp.succes >= 100 && arp.echecs == 0;
    let rx_a_progresse = apres.rx_paquets > avant.rx_paquets;
    let verdict = verdict_arp && rx_a_progresse;

    crate::println!(
        "netdiag: ARP {}/{} en {} s -- min {} ms, moy {} ms, max {} ms (pire serie d'echecs {})",
        arp.succes, arp.tours, ecoule_s, arp.ms_min, arp.moyenne_ms(), arp.ms_max,
        serie_echecs_max,
    );
    crate::println!(
        "         DNS {}/{} max {} ms | TCP:80 {}/{} max {} ms | TCP:443 {}/{} max {} ms",
        dns.succes, dns.tours, dns.ms_max,
        tcp80.succes, tcp80.tours, tcp80.ms_max,
        tcp443.succes, tcp443.tours, tcp443.ms_max,
    );
    crate::println!(
        "         RX {} -> {} paquets | reprises {} | chip_cmd {:#04x} | intr_status {:#06x}",
        avant.rx_paquets, apres.rx_paquets,
        apres.rx_reprises.saturating_sub(avant.rx_reprises),
        apres.chip_cmd, apres.intr_status,
    );
    crate::println!(
        "         verdict {} (critere : >= 100 resolutions, 0 echec, reception qui progresse)",
        if verdict { "OK" } else { "ECHEC" },
    );

    // UNE SEULE LIGNE POUR LA MACHINE, et elle porte tout.
    crate::serial_println!(
        "BOUCHAUD_NETDIAG_BILAN ecoule_s={} \
arp_tours={} arp_ok={} arp_ko={} arp_ms_min={} arp_ms_moy={} arp_ms_max={} arp_serie_ko_max={} \
dns_tours={} dns_ok={} dns_ms_max={} \
tcp80_tours={} tcp80_ok={} tcp80_ms_max={} tcp443_tours={} tcp443_ok={} tcp443_ms_max={} \
rx_paquets_avant={} rx_paquets_apres={} rx_octets={} tx_paquets={} \
rx_reprises={} rx_reprises_echouees={} rx_abandonnees={} rx_abimees={} rx_missed={} \
chip_cmd={:#04x} intr_status={:#06x} rx_cur={} desc_materiel={} desc_processeur={} \
isr_rx_ok={} isr_rx_err={} isr_rx_overflow={} isr_rx_fifo_over={} isr_sys_err={} \
xid={:#05x} generation={} invariant={} arp_ttl_ms={} verdict={}",
        ecoule_s,
        arp.tours, arp.succes, arp.echecs, arp.ms_min, arp.moyenne_ms(), arp.ms_max,
        serie_echecs_max,
        dns.tours, dns.succes, dns.ms_max,
        tcp80.tours, tcp80.succes, tcp80.ms_max,
        tcp443.tours, tcp443.succes, tcp443.ms_max,
        avant.rx_paquets, apres.rx_paquets, apres.rx_octets, apres.tx_paquets,
        apres.rx_reprises, apres.rx_reprises_echouees, apres.rx_abandonnees,
        apres.rx_abimees, apres.rx_missed,
        apres.chip_cmd, apres.intr_status, apres.rx_cur,
        apres.rx_desc_materiel, apres.rx_desc_processeur,
        apres.isr_rx_ok, apres.isr_rx_err, apres.isr_rx_overflow,
        apres.isr_rx_fifo_over, apres.isr_system_error,
        apres.xid, apres.generation,
        apres.invariant.unwrap_or("intact"),
        net::arp_ttl_ms(),
        if verdict { "OK" } else { "ECHEC" },
    );
}

/// `netetat` : l'etat du pilote reseau, tout de suite, sans banc.
///
/// C'est la reponse a « que fait la carte MAINTENANT », et elle tient sur
/// l'ecran de la machine sans archive a extraire.
pub fn netetat() {
    let nic = crate::drivers::rtl8168::releve();
    let qualite = net::qualite_lien();
    crate::println!(
        "lien : {} {} Mb/s duplex {} | ip {} gw {} dns {}",
        if crate::drivers::e1000::link_up() { "UP" } else { "DOWN" },
        qualite.vitesse_mbps,
        if qualite.duplex_complet { "complet" } else { "alternat" },
        ipv4::format_addr(&net::our_ip()),
        ipv4::format_addr(&net::gateway()),
        ipv4::format_addr(&net::dns_server()),
    );
    if nic.xid == 0 && nic.rx_paquets == 0 && nic.tx_paquets == 0 {
        crate::println!("rtl8168 : absent ou non initialise.");
        return;
    }
    crate::println!(
        "puce : xid {:#05x} ({}) | chip_cmd {:#04x}{}{}{} | intr_status {:#06x}",
        nic.xid,
        nic.generation,
        nic.chip_cmd,
        if nic.chip_cmd & 0x08 != 0 { " RxEnb" } else { " !RxEnb" },
        if nic.chip_cmd & 0x04 != 0 { " TxEnb" } else { " !TxEnb" },
        if nic.chip_cmd & 0x01 != 0 { " RxBufEmpty" } else { "" },
        nic.intr_status,
    );
    crate::println!(
        "anneau : rx_cur {} tx_cur {} | descripteurs {} materiel / {} processeur | courant {:#010x} | invariant {}",
        nic.rx_cur, nic.tx_cur,
        nic.rx_desc_materiel, nic.rx_desc_processeur, nic.rx_desc_courant,
        nic.invariant.unwrap_or("intact"),
    );
    crate::println!(
        "trafic : rx {} paquets / {} o | tx {} paquets / {} o | rx_missed {} | abimees {} | anneau tx plein {}",
        nic.rx_paquets, nic.rx_octets, nic.tx_paquets, nic.tx_octets,
        nic.rx_missed, nic.rx_abimees, nic.tx_anneau_plein,
    );
    crate::println!(
        "statuts : {} lectures | rx_ok {} rx_err {} overflow {} fifo_over {} tx_err {} link_chg {} sys_err {} absente {}",
        nic.isr_lectures, nic.isr_rx_ok, nic.isr_rx_err, nic.isr_rx_overflow,
        nic.isr_rx_fifo_over, nic.isr_tx_err, nic.isr_link_chg,
        nic.isr_system_error, nic.isr_carte_absente,
    );
    crate::println!(
        "reprises : {} rearmements | {} reprises ({} sans effet) | {} trames abandonnees",
        nic.rx_rearmements, nic.rx_reprises, nic.rx_reprises_echouees, nic.rx_abandonnees,
    );
    let (routees, arp_vues, dhcp_vues, arp_ok, arp_ko, arp_non_emis) =
        net::compteurs_routage();
    crate::println!(
        "routage : {} trames ({} arp, {} dhcp) | arp {} resolus / {} echoues / {} non emis | ttl {} ms",
        routees, arp_vues, dhcp_vues, arp_ok, arp_ko, arp_non_emis, net::arp_ttl_ms(),
    );
}
