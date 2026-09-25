//! Les reponses BRDP : le meme etat que le shell, dans la meme ponctuation.
//!
//! # Une seule source de verite
//!
//! Chaque reponse LIT ce que la machine publie deja -- `rtl8168::releve()`,
//! `blackbox::statut()`, `lab::compteurs()`, `dhcp::compteurs()`. Rien n'est
//! recalcule ici, et surtout rien n'est mesure ici : un canal d'enquete qui
//! mesure lui-meme finit par mesurer autre chose que le shell, et l'on passe
//! la campagne suivante a reconcilier deux chiffres.
//!
//! Une seule commande a un effet : `blackbox checkpoint`. C'est voulu -- c'est
//! precisement l'effet qu'on veut pouvoir declencher a distance quand la
//! machine ne repond plus a rien d'autre.
//!
//! # Pourquoi tout est borne
//!
//! Une reponse s'ecrit dans un `Tampon` de taille fixe. Ce chemin s'emprunte
//! quand la machine va mal ; ce n'est pas le moment d'allouer.

use core::fmt::Write;

use super::brdp::{self, Commande};
use super::tampon::Tampon;

/// Taille d'une reponse. Le plus gros est `rtl8168 ring`, qui porte
/// soixante-quatre etats de descripteurs en un objet.
pub const REPONSE_MAX: usize = 4096;
pub type Reponse = Tampon<REPONSE_MAX>;

fn ip_json(t: &mut Reponse, cle: &str, ip: [u8; 4]) {
    let _ = write!(t, ",\"{cle}\":\"{}.{}.{}.{}\"", ip[0], ip[1], ip[2], ip[3]);
}

/// Ecrit la reponse a `commande`. Rend `false` si la connexion doit se fermer.
pub fn rend(t: &mut Reponse, commande: Commande) -> bool {
    t.vide();
    let _ = write!(t, "{{\"ok\":true,\"cmd\":{}", commande.code());
    let continuer = match commande {
        // `hello` est traite par le serveur, qui seul connait le nonce.
        Commande::Hello { .. } => true,
        Commande::Status => {
            status(t);
            true
        }
        Commande::AuditStatus => {
            audit(t);
            true
        }
        Commande::AuditRun => {
            let verdicts = crate::kernel::lab::auditd::tour();
            let _ = write!(t, ",\"verdicts\":{verdicts}");
            audit(t);
            true
        }
        Commande::AuditLast => {
            let (event, quand) = crate::kernel::lab::auditd::dernier_verdict();
            let _ = write!(
                t,
                ",\"event_id\":{event},\"event\":\"{}\",\"t_ns\":{quand}",
                crate::kernel::lab::catalogue::nom(event),
            );
            true
        }
        Commande::NetStatus => {
            net(t);
            true
        }
        Commande::Rtl8168Status => {
            rtl8168(t);
            true
        }
        Commande::Rtl8168Ring => {
            anneau(t);
            true
        }
        Commande::Rtl8168Desc { index } => {
            descripteur(t, index);
            true
        }
        Commande::DhcpStatus => {
            dhcp(t);
            true
        }
        Commande::BlackboxStatus => {
            blackbox(t);
            true
        }
        Commande::BlackboxCheckpoint => {
            // LE SEUL EFFET DE BORD DU PROTOCOLE, et il est assume : c'est
            // exactement ce qu'on veut pouvoir declencher quand la machine ne
            // repond plus a rien d'autre.
            crate::kernel::blackbox::checkpoint_force_echeance();
            let bilan = crate::kernel::blackbox::checkpoint("brdp");
            let _ = write!(
                t,
                ",\"support\":{},\"marque\":{},\"sync\":{},\"seq\":{},\"poses\":{},\
\"duree_us\":{}",
                bilan.support, bilan.marque, bilan.synchronise, bilan.seq,
                bilan.poses, bilan.duree_us,
            );
            true
        }
        Commande::EventsTail { .. } | Commande::EventsWatch => {
            // Les evenements sortent en lignes separees : le serveur s'en
            // charge, parce que lui seul sait ce qu'il a deja envoye.
            true
        }
        Commande::ServicesSnapshot => {
            services(t);
            true
        }
        // BOUCHAUD_HOTFIX12_SERVICES_REMOTE_DETAIL_V1
        Commande::ServicesPage { start } => {
            services_page(t, start);
            true
        }
        Commande::ProcessesSnapshot => {
            processus(t);
            true
        }
        Commande::MemorySnapshot => {
            memoire(t);
            true
        }

        // BOUCHAUD_P0_REMOTE_CONTROL_V1
        Commande::SystemReboot => {
            let _ = write!(t, ",\"accepted\":true,\"action\":\"reboot\",\"delay_ms\":250");
            true
        }
        Commande::SystemShutdown => {
            let _ = write!(t, ",\"accepted\":true,\"action\":\"shutdown\",\"delay_ms\":250");
            true
        }
        Commande::BrowserStart => {
            crate::gui::services::demande(crate::gui::services::DEMARRER);
            let _ = write!(t, ",\"accepted\":true,\"action\":\"browser-start\",\"root_pid\":{}", crate::gui::services::racine());
            true
        }
        Commande::BrowserStop => {
            crate::gui::services::demande(crate::gui::services::ARRETER);
            let _ = write!(t, ",\"accepted\":true,\"action\":\"browser-stop\",\"root_pid\":{}", crate::gui::services::racine());
            true
        }
        Commande::BrowserRestart => {
            crate::gui::services::demande(crate::gui::services::REDEMARRER);
            // BOUCHAUD_P0_REMOTE_CONTROL_V1_2_ACK
            let _ = write!(t, ",\"accepted\":true,\"action\":\"browser-restart\",\"mode\":\"two-phase\",\"root_pid\":{}", crate::gui::services::racine());
            true
        }
        Commande::ProcessKill { pid } => {
            crate::kernel::task::tue_processus(pid, 137);
            let _ = write!(t, ",\"accepted\":true,\"action\":\"process-kill\",\"pid\":{pid}");
            true
        }
        Commande::ProcessKillTree { pid } => {
            let cibles = crate::kernel::task::arbre_de(pid);
            let nombre = cibles.len();
            for cible in cibles.iter().rev() { crate::kernel::task::tue_processus(*cible, 137); }
            let _ = write!(t, ",\"accepted\":true,\"action\":\"process-kill-tree\",\"pid\":{pid},\"targets\":{nombre}");
            true
        }

        // BOUCHAUD_HOTFIX10_INTERNET_PROOF_CHAIN_V1
        Commande::InternetProofStart => {
            let started = crate::net::preuve_internet::lance();
            internet_proof(t, Some(started));
            true
        }
        Commande::InternetProofStatus => {
            internet_proof(t, None);
            true
        }
        // BOUCHAUD_HOTFIX11_SERIAL_BRDP
        Commande::SerialStatus => {
            serial_status(t);
            true
        }
        Commande::SerialRead { start, combien } => {
            serial_read(t, start, combien);
            true
        }
        Commande::Quit => false,
    };
    let _ = t.write_char('}');
    t.termine();
    continuer
}

/// La reponse d'erreur. Courte par construction : elle doit passer meme quand
/// une reponse complete ne passerait pas.
pub fn erreur(t: &mut Reponse, e: brdp::Erreur) {
    t.vide();
    let _ = write!(
        t,
        "{{\"ok\":false,\"error\":\"{}\",\"code\":{}}}",
        brdp::nom_erreur(e),
        e as u16,
    );
    t.termine();
}


// BOUCHAUD_HOTFIX10_INTERNET_PROOF_CHAIN_V1
fn internet_proof(t: &mut Reponse, started: Option<bool>) {
    let r = crate::net::preuve_internet::releve();

    if let Some(v) = started {
        let _ = write!(t, ",\"started\":{}", v);
    }

    let _ = write!(
        t,
        ",\"generation\":{},\"running\":{},\"done\":{},\"chain_ok\":{},\
\"stage\":\"{}\",\"first_failure\":\"{}\",\"start_ms\":{},\"end_ms\":{},\
\"link\":{},\"lease\":{},\
\"arp_ok\":{},\"arp_ms\":{},\"arp_routed\":{},\"arp_seen\":{},\
\"arp_resolved\":{},\"arp_failed\":{},\"arp_not_sent\":{},\
\"dns_ok\":{},\"dns_ms\":{},\
\"dns53_rx_ethernet\":{},\"dns53_rx_ipv4\":{},\"dns53_rx_udp\":{},\
\"dns53_queued_ip\":{},\"dns53_dequeued_ip\":{},\"dns53_socket_match\":{},\
\"dns53_socket_busy\":{},\"dns53_socket_delivered\":{},\"dns53_poll_ready\":{},\
\"dns53_recv_success\":{},\"dns53_recv_eagain\":{},\
\"tcp80_ok\":{},\"tcp80_ms\":{},\"tcp80_bytes\":{},\"tcp80_http_status\":{},\
\"tcp443_ok\":{},\"tcp443_ms\":{},\"tcp_handshakes\":{},\"tcp_syn_retx\":{},\
\"tls_ok\":{},\"tls_ms\":{},\"tls_error\":\"{}\",\"tls_trusted\":{},\
\"tls_hostname_ok\":{},\"tls_expired\":{},\"tls_cipher\":\"{}\",\
\"tls_kx\":\"{}\",\"tls_alpn\":\"{}\",\
\"http_sent\":{},\"http_ok\":{},\"http_ms\":{},\"http_raw_bytes\":{},\
\"http_body_bytes\":{},\"http_status\":{},\"http_html\":{},\"http_complete\":{},\
\"rx_start\":{},\"rx_end\":{},\"tx_start\":{},\"tx_end\":{},\
\"resets_start\":{},\"resets_end\":{}",
        r.generation,
        r.en_cours,
        r.terminee,
        r.chaine_ok,
        crate::net::preuve_internet::etape_nom(r.etape),
        crate::net::preuve_internet::echec_nom(r.premier_echec),
        r.debut_ms,
        r.fin_ms,
        r.lien,
        r.bail,
        r.arp_ok,
        r.arp_ms,
        r.arp_routees_delta,
        r.arp_vues_delta,
        r.arp_resolues_delta,
        r.arp_echecs_delta,
        r.arp_non_emises_delta,
        r.dns_ok,
        r.dns_ms,
        r.dns_delta[0], r.dns_delta[1], r.dns_delta[2], r.dns_delta[3],
        r.dns_delta[4], r.dns_delta[5], r.dns_delta[6], r.dns_delta[7],
        r.dns_delta[8], r.dns_delta[9], r.dns_delta[10],
        r.tcp80_ok,
        r.tcp80_ms,
        r.tcp80_octets,
        r.tcp80_status,
        r.tcp443_ok,
        r.tcp443_ms,
        r.tcp_poignees_delta,
        r.tcp_syn_retx_delta,
        r.tls_ok,
        r.tls_ms,
        crate::net::preuve_internet::tls_erreur_nom(r.tls_erreur),
        r.tls_trusted,
        r.tls_hostname_ok,
        r.tls_expired,
        crate::net::preuve_internet::cipher_nom(r.tls_cipher),
        crate::net::preuve_internet::kx_nom(r.tls_kx),
        crate::net::preuve_internet::alpn_nom(r.tls_alpn),
        r.http_sent,
        r.http_ok,
        r.http_ms,
        r.http_raw,
        r.http_body,
        r.http_status,
        r.http_html,
        r.http_complete,
        r.rx_start,
        r.rx_end,
        r.tx_start,
        r.tx_end,
        r.reset_start,
        r.reset_end,
    );

    ip_json(t, "ip", r.ip);
    ip_json(t, "gateway", r.passerelle);
    ip_json(t, "dns_server", r.dns);
    ip_json(t, "resolved_ip", r.dns_ip);
}

fn status(t: &mut Reponse) {
    let (emises, plus_ancienne, prochaine, capacite) = crate::kernel::lab::compteurs();
    let (tours, verdicts, captures, cadence) = crate::kernel::lab::auditd::compteurs();
    let r = crate::drivers::rtl8168::releve();
    let bb = crate::kernel::blackbox::statut();
    let _ = write!(
        t,
        ",\"brdp\":{},\"t_ns\":{},\"lab\":{{\"emises\":{emises},\
\"plus_ancienne\":{plus_ancienne},\"prochaine\":{prochaine},\"capacite\":{capacite},\
\"avant_horloge\":{}}},\"audit\":{{\"tours\":{tours},\"verdicts\":{verdicts},\
\"captures\":{captures},\"cadence_hz\":{cadence},\"sain\":{}}},\
\"rx_packets\":{},\"rx_rendus_tour2\":{},\"checkpoint_count\":{},\
\"records_ram\":{},\"records_persisted\":{}",
        brdp::VERSION,
        crate::kernel::timer::monotonic_ns(),
        crate::kernel::lab::avant_horloge(),
        crate::kernel::lab::auditd::sain(),
        r.rx_paquets,
        r.rx_rendus_tour2,
        bb.checkpoint_count,
        bb.records_ram,
        bb.records_persisted,
    );
    ip_json(t, "ip_lab", super::ip());
    ip_json(t, "ip_bail", crate::net::our_ip());
}

fn audit(t: &mut Reponse) {
    let (tours, verdicts, captures, cadence) = crate::kernel::lab::auditd::compteurs();
    let (event, quand) = crate::kernel::lab::auditd::dernier_verdict();
    let _ = write!(
        t,
        ",\"lance\":{},\"tours\":{tours},\"verdicts\":{verdicts},\"captures\":{captures},\
\"cadence_hz\":{cadence},\"dernier_event_id\":{event},\"dernier\":\"{}\",\
\"dernier_t_ns\":{quand}",
        crate::kernel::lab::auditd::lance(),
        crate::kernel::lab::catalogue::nom(event),
    );
}

fn net(t: &mut Reponse) {
    let (triees, diag, normale, deux) = super::compteurs();
    let _ = write!(
        t,
        ",\"pilote\":\"{}\",\"lien\":{},\"lab_actif\":{},\"port_brdp\":{},\
\"port_telemetrie\":{},\"triees\":{triees},\"pour_diagnostic\":{diag},\
\"pour_normale\":{normale},\"pour_les_deux\":{deux},\"jetees\":{}",
        crate::drivers::e1000::nom_pilote(),
        crate::drivers::e1000::link_up(),
        super::actif(),
        super::PORT_BRDP,
        super::PORT_TELEMETRIE,
        super::jetees(),
    );
    // L'ETAT DU BRANCHEMENT, DANS LE VOCABULAIRE DU CLIENT PC.
    //
    // Dire « armed » ne dit rien du jeton : c'est le fait qu'un fil ecoute,
    // pas son secret. Un client qui n'obtient pas de reponse a besoin de
    // distinguer « le serveur est desactive faute de jeton » de « le serveur
    // est mort » -- et cette reponse-ci, il ne l'obtient que s'il a deja pu
    // s'authentifier, donc la lui donner n'apprend rien a personne d'autre.
    let etat = super::services();
    let (datagrammes, evenements, _, abandons) = super::telemetrie::compteurs();
    let _ = write!(
        t,
        ",\"brdp\":\"{}\",\"telemetry\":\"{}\",\"telemetrie_datagrammes\":{datagrammes},\
\"telemetrie_evenements\":{evenements},\"telemetrie_abandons\":{abandons},\
\"telemetrie_perdus\":{}",
        etat.brdp.nom(),
        etat.telemetrie.nom(),
        super::telemetrie::perdus(),
    );
    ip_json(t, "ip_lab", super::ip());
}

// BOUCHAUD_HOTFIX9_RX_PROOF_GATE_V1
fn rtl8168(t: &mut Reponse) {
    let r = crate::drivers::rtl8168::releve();
    let _ = write!(
        t,
        ",\"xid\":{},\"generation\":\"{}\",\"chip_cmd\":{},\"intr_status\":{},\
\"rx_missed\":{},\"rx_packets\":{},\"rx_octets\":{},\"rx_cur\":{},\
\"desc_nic\":{},\"desc_cpu\":{},\"rx_tours_cpu\":{},\"rx_rendus_tour1\":{},\
\"rx_rendus_tour2\":{},\"rx_rearmes_tour1\":{},\"rx_reutilises_tour2\":{},\
\"isr_rx_ok\":{},\"isr_rx_err\":{},\"isr_rx_overflow\":{},\"isr_rx_fifo_over\":{},\
\"isr_system_error\":{},\"rx_ok_sans_progres\":{},\"own_rendus\":{},\
\"reparations_demandees\":{},\"reparations_executees\":{},\"repair_degre\":{},\
\"repair_raison\":{},\"repair_derniere_ns\":{},\"reprises_differees\":{},\
\"reprises_sans_effet\":{},\"repair_sans_preuve\":{},\"repair_preuves_consommees\":{},\
\"repair_differee_pending\":{},\"repair_verdict_pending\":{},\
\"reinitialisations\":{},\"reinitialisations_ok\":{},\
\"invariant\":\"{}\",\"tx_packets\":{},\"tx_termines\":{}",
        r.xid, r.generation, r.chip_cmd, r.intr_status, r.rx_missed,
        r.rx_paquets, r.rx_octets, r.rx_cur,
        r.rx_desc_materiel, r.rx_desc_processeur,
        r.rx_tours_cpu, r.rx_rendus_tour1, r.rx_rendus_tour2,
        r.rx_rearmes_tour1, r.rx_reutilises_tour2,
        r.isr_rx_ok, r.isr_rx_err, r.isr_rx_overflow, r.isr_rx_fifo_over,
        r.isr_system_error, r.rx_ok_sans_progres, r.rx_own_rendus,
        r.reparations_demandees, r.reparations_executees, r.reparation_degre,
        r.reparation_raison, r.reparation_derniere_ns, r.reprises_differees,
        r.reprises_sans_effet, r.reparations_refusees_sans_preuve,
        r.preuves_rx_sans_progres_consommees,
        r.reparation_differee_en_attente, r.verdict_en_attente,
        r.reinitialisations, r.reinitialisations_ok,
        r.invariant.unwrap_or("intact"), r.tx_paquets, r.tx_termines,
    );
    // LE CRITERE PHYSIQUE, EXPLICITE DANS LA REPONSE.
    //
    // `rx_packets > 64` ET `rx_rendus_tour2 > 0`. Le client n'a pas a le
    // reconstituer : il est ici, sous le nom que la campagne lui donne.
    let _ = write!(
        t,
        ",\"critere_tour2\":{},\"critere_rx_au_dela_de_64\":{}",
        r.rx_rendus_tour2 > 0,
        r.rx_paquets > 64,
    );
}

fn anneau(t: &mut Reponse) {
    let r = crate::drivers::rtl8168::releve();
    let carte = crate::drivers::rtl8168::carte_own_publique();
    let _ = write!(
        t,
        ",\"descripteurs\":64,\"rx_cur\":{},\"desc_nic\":{},\"desc_cpu\":{},\
\"own_map\":{carte},\"invariant\":\"{}\",\"anneau_dma\":{},\"desc_addr_relu\":{}",
        r.rx_cur, r.rx_desc_materiel, r.rx_desc_processeur,
        r.invariant.unwrap_or("intact"),
        crate::drivers::rtl8168::anneau_dma(),
        crate::drivers::rtl8168::desc_addr_relu(),
    );
    let _ = write!(t, ",\"opts1\":[");
    for i in 0..64usize {
        if i > 0 {
            let _ = t.write_char(',');
        }
        let _ = write!(t, "{}", crate::drivers::rtl8168::desc_opts1(i));
    }
    let _ = write!(t, "]");
}

fn descripteur(t: &mut Reponse, index: u16) {
    let i = index as usize;
    let _ = write!(
        t,
        ",\"index\":{index},\"opts1\":{},\"opts2\":{},\"addr\":{},\"own\":{},\"eor\":{}",
        crate::drivers::rtl8168::desc_opts1(i),
        crate::drivers::rtl8168::desc_opts2(i),
        crate::drivers::rtl8168::desc_addr(i),
        crate::drivers::rtl8168::desc_opts1(i) & (1 << 31) != 0,
        crate::drivers::rtl8168::desc_opts1(i) & (1 << 30) != 0,
    );
}

fn dhcp(t: &mut Reponse) {
    let c = crate::net::application::dhcp::compteurs();
    let _ = write!(
        t,
        ",\"etape\":\"{}\",\"discover\":{},\"offres\":{},\"requests\":{},\"acks\":{},\
\"xid\":{},\"tentatives\":{},\"bail\":{}",
        c.derniere_etape.nom(),
        c.discover_envoyes, c.offres_vues, c.requests_envoyes, c.acks_vus,
        c.dernier_xid, c.tentatives,
        crate::net::bail_obtenu(),
    );
    ip_json(t, "ip", crate::net::our_ip());
    ip_json(t, "passerelle", crate::net::gateway());
    // LE CRITERE PHYSIQUE : OFFER, REQUEST, ACK, IPV4_READY.
    let _ = write!(
        t,
        ",\"critere_offer\":{},\"critere_request\":{},\"critere_ack\":{},\
\"critere_ipv4_ready\":{}",
        c.offres_vues > 0,
        c.requests_envoyes > 0,
        c.acks_vus > 0,
        crate::net::bail_obtenu(),
    );
}

fn blackbox(t: &mut Reponse) {
    let s = crate::kernel::blackbox::statut();
    let _ = write!(
        t,
        ",\"storage_ready\":{},\"last_write_ok_ns\":{},\"last_write_error_ns\":{},\
\"write_errors\":{},\"last_flush_begin_ns\":{},\"last_flush_ok_ns\":{},\
\"last_flush_error_ns\":{},\"checkpoint_due_ns\":{},\"last_checkpoint_begin_ns\":{},\
\"last_checkpoint_ok_ns\":{},\"checkpoint_count\":{},\"records_ram\":{},\
\"records_persisted\":{},\"fin_written\":{},\"sync_ok\":{}",
        s.storage_ready, s.last_write_ok_ns, s.last_write_error_ns, s.write_errors,
        s.last_flush_begin_ns, s.last_flush_ok_ns, s.last_flush_error_ns,
        s.checkpoint_due_ns, s.last_checkpoint_begin_ns, s.last_checkpoint_ok_ns,
        s.checkpoint_count, s.records_ram, s.records_persisted,
        s.fin_written, s.sync_ok,
    );
    // L'ECART QUE `AUDIT_BLACKBOX_PERSISTENCE_STALL` cherche, calcule une fois
    // et rendu tel quel : le client n'a pas a soustraire deux compteurs dont
    // il ignore la semantique.
    let _ = write!(
        t,
        ",\"retard_records\":{}",
        s.records_ram.saturating_sub(s.records_persisted),
    );
    // BOUCHAUD_HOTFIX11_EVIDENCE_SURVIVAL
    let cause_sync = match s.last_checkpoint_cause_sync as u8 {
        crate::drivers::xhci_active::SYNC_OK => "ok",
        crate::drivers::xhci_active::SYNC_SANS_SUPPORT => "sans-support",
        crate::drivers::xhci_active::SYNC_VERROU_REFUSE => "verrou-refuse",
        _ => "pilote-ko",
    };
    let _ = write!(
        t,
        ",\"checkpoint_attempts\":{},\"checkpoint_successes\":{},\"checkpoint_failures\":{},\
\"last_checkpoint_seq\":{},\"last_checkpoint_duration_us\":{},\
\"last_checkpoint_confirmed\":{},\"last_checkpoint_support\":{},\
\"last_checkpoint_marker\":{},\"last_checkpoint_sync\":{},\
\"last_checkpoint_cause_sync\":{},\"last_checkpoint_cause\":\"{}\",\
\"serial_produced_bytes\":{},\"serial_persisted_bytes\":{},\"serial_backlog_bytes\":{},\
\"flight_produced\":{},\"flight_persisted\":{},\"flight_backlog\":{}",
        s.checkpoint_attempts,
        s.checkpoint_successes,
        s.checkpoint_failures,
        s.last_checkpoint_seq,
        s.last_checkpoint_duration_us,
        s.last_checkpoint_confirmed,
        s.last_checkpoint_support,
        s.last_checkpoint_marker,
        s.last_checkpoint_sync,
        s.last_checkpoint_cause_sync,
        cause_sync,
        s.serial_produced_bytes,
        s.serial_persisted_bytes,
        s.serial_produced_bytes.saturating_sub(s.serial_persisted_bytes),
        s.flight_produced,
        s.flight_persisted,
        s.flight_produced.saturating_sub(s.flight_persisted),
    );

}


// BOUCHAUD_HOTFIX11_SERIAL_BRDP
fn serial_status(t: &mut Reponse) {
    let (debut, fin) = crate::drivers::serial::trace_bornes();
    let _ = write!(
        t,
        ",\"oldest\":{},\"end\":{},\"capacity\":{},\"retained\":{}",
        debut,
        fin,
        crate::drivers::serial::trace_capacite(),
        fin.saturating_sub(debut),
    );
}

// Lecture SANS allocation et SANS effet de bord. Le contenu est hexadecimal:
// pas d'echappement JSON variable, donc la borne de REPONSE_MAX reste simple
// a prouver.
fn serial_read(t: &mut Reponse, demande: u64, combien: u16) {
    let (plus_ancien, fin) = crate::drivers::serial::trace_bornes();
    let demande_usize = demande.min(usize::MAX as u64) as usize;
    let debut = demande_usize.max(plus_ancien).min(fin);
    let n = (combien as usize)
        .min(super::brdp::SERIAL_READ_MAX as usize)
        .min(fin.saturating_sub(debut));
    let suivant = debut.saturating_add(n);
    let _ = write!(
        t,
        ",\"requested_start\":{},\"oldest\":{},\"start\":{},\"next\":{},\"end\":{},\
\"lost_before\":{},\"eof\":{},\"data_hex\":\"",
        demande,
        plus_ancien,
        debut,
        suivant,
        fin,
        demande_usize < plus_ancien,
        suivant >= fin,
    );
    for seq in debut..suivant {
        let _ = write!(t, "{:02x}", crate::drivers::serial::trace_octet(seq));
    }
    let _ = t.write_char('"');
}

fn services(t: &mut Reponse) {
    let (utilisateur_ms, systeme_ms, charge) = crate::gui::services::usage();
    let _ = write!(
        t,
        ",\"utilisateur_ms\":{utilisateur_ms},\"systeme_ms\":{systeme_ms},\"charge\":{charge}",
    );
}

// BOUCHAUD_HOTFIX12_SERVICES_REMOTE_DETAIL_V1
//
// La fenetre Services lit deja le registre central complet. BRDP lit maintenant
// exactement cette meme source de verite, par petites pages bornees.
const SERVICES_PAR_PAGE: usize = 1;

fn json_chaine(t: &mut Reponse, texte: &str) {
    let _ = t.write_char('"');
    for c in texte.chars() {
        match c {
            '"' => { let _ = t.write_str("\\\""); }
            '\\' => { let _ = t.write_str("\\\\"); }
            '\n' => { let _ = t.write_str("\\n"); }
            '\r' => { let _ = t.write_str("\\r"); }
            '\t' => { let _ = t.write_str("\\t"); }
            c if c.is_control() => { let _ = write!(t, "\\u{:04x}", c as u32); }
            c => { let _ = t.write_char(c); }
        }
    }
    let _ = t.write_char('"');
}

fn json_option_nombre<T: core::fmt::Display>(t: &mut Reponse, valeur: Option<T>) {
    match valeur {
        Some(v) => { let _ = write!(t, "{v}"); }
        None => { let _ = t.write_str("null"); }
    }
}

fn json_option_chaine(t: &mut Reponse, valeur: Option<&str>) {
    match valeur {
        Some(v) => json_chaine(t, v),
        None => { let _ = t.write_str("null"); }
    }
}

fn service_detail(t: &mut Reponse, e: &crate::kernel::services::registre::Entree) {
    use crate::kernel::services::registre;

    let _ = t.write_str("{\"id\":");
    json_chaine(t, e.id.texte());
    let _ = t.write_str(",\"parent\":");
    json_chaine(t, e.parent.texte());
    let _ = t.write_str(",\"genre\":");
    json_chaine(t, e.genre.nom());
    let _ = t.write_str(",\"etat\":");
    json_chaine(t, e.etat.nom());
    let _ = t.write_str(",\"raison\":");
    if e.raison.est_vide() { let _ = t.write_str("null"); } else { json_chaine(t, e.raison.texte()); }
    let _ = t.write_str(",\"prerequis\":");
    json_option_chaine(t, registre::prerequis(e.id.texte()));

    let _ = write!(
        t,
        ",\"debut_ns\":{},\"pret_ns\":{},\"derniere_activite_ns\":{},\
\"dernier_succes_ns\":{},\"derniere_erreur_ns\":{},\"derniere_transition_ns\":{},\
\"redemarrages\":{},\"reprises\":{},\"erreurs\":{}",
        e.debut_ns,
        e.pret_ns,
        e.derniere_activite_ns,
        e.dernier_succes_ns,
        e.derniere_erreur_ns,
        e.derniere_transition_ns,
        e.redemarrages,
        e.reprises,
        e.erreurs,
    );

    let k = e.kpi;
    let _ = t.write_str(",\"kpi\":{\"cpu_pour_mille\":");
    json_option_nombre(t, k.cpu_pour_mille);
    let _ = t.write_str(",\"rss_octets\":"); json_option_nombre(t, k.rss_octets);
    let _ = t.write_str(",\"vss_octets\":"); json_option_nombre(t, k.vss_octets);
    let _ = t.write_str(",\"disque_lu\":"); json_option_nombre(t, k.disque_lu);
    let _ = t.write_str(",\"disque_ecrit\":"); json_option_nombre(t, k.disque_ecrit);
    let _ = t.write_str(",\"rx_octets\":"); json_option_nombre(t, k.rx_octets);
    let _ = t.write_str(",\"tx_octets\":"); json_option_nombre(t, k.tx_octets);
    let _ = t.write_str(",\"latence_us\":"); json_option_nombre(t, k.latence_us);
    let _ = t.write_str(",\"latence_max_us\":"); json_option_nombre(t, k.latence_max_us);
    let _ = t.write_str(",\"operations\":"); json_option_nombre(t, k.operations);
    let _ = t.write_str(",\"pid\":"); json_option_nombre(t, k.pid);
    let _ = t.write_str(",\"instances\":"); json_option_nombre(t, k.instances);
    let _ = t.write_str(",\"fautes_nombre\":"); json_option_nombre(t, k.fautes_nombre);
    let _ = t.write_str(",\"fautes_total_us\":"); json_option_nombre(t, k.fautes_total_us);
    let _ = t.write_str(",\"fautes_pire_us\":"); json_option_nombre(t, k.fautes_pire_us);
    let _ = t.write_str(",\"ppid\":"); json_option_nombre(t, k.ppid);
    let _ = t.write_str(",\"groupe\":"); json_option_nombre(t, k.groupe);
    let _ = t.write_str(",\"threads\":"); json_option_nombre(t, k.threads);
    let _ = t.write_str(",\"threads_executables\":"); json_option_nombre(t, k.threads_executables);
    let _ = t.write_str(",\"cpu_chaud\":"); json_option_nombre(t, k.cpu_chaud);
    let _ = t.write_str(",\"cpu_chaud_pour_mille\":"); json_option_nombre(t, k.cpu_chaud_pour_mille);
    let _ = t.write_str(",\"migrations\":"); json_option_nombre(t, k.migrations);
    let _ = t.write_str(",\"commutations\":"); json_option_nombre(t, k.commutations);
    let _ = t.write_str(",\"fautes_dominante\":"); json_option_chaine(t, k.fautes_dominante);
    let _ = t.write_str("}}");
}

fn services_page(t: &mut Reponse, start: u16) {
    // Rafraichit les producteurs sans inventer une seconde mesure : chacun
    // conserve sa cadence propre (1 s protocoles, 5 s processus).
    crate::kernel::services::publie_les_indicateurs();
    crate::gui::services::releve_si_du();

    crate::kernel::services::avec_atelier(|atelier| {
        let total = atelier.connues;
        let debut = (start as usize).min(total);
        let fin = debut.saturating_add(SERVICES_PAR_PAGE).min(total);
        let _ = write!(
            t,
            ",\"t_ns\":{},\"total\":{},\"start\":{},\"next\":{},\"done\":{},\"entries\":[",
            crate::kernel::timer::monotonic_ns(), total, debut, fin, fin >= total,
        );
        for (rang, e) in atelier.entrees[debut..fin].iter().enumerate() {
            if rang != 0 { let _ = t.write_char(','); }
            service_detail(t, e);
        }
        let _ = t.write_char(']');
    });
}

fn processus(t: &mut Reponse) {
    let _ = write!(
        t,
        ",\"taches\":{},\"coeurs\":{}",
        crate::kernel::process::count(),
        crate::arch::x86_64::smp::discovered_cpus(),
    );
}

fn memoire(t: &mut Reponse) {
    // LE TAS TEL QUE LE NOYAU LE PUBLIE, sans recalcul. Voir `heap::stats`.
    let (utilise, libre, total) = crate::kernel::heap::stats();
    let _ = write!(
        t,
        ",\"tas_utilise\":{utilise},\"tas_libre\":{libre},\"tas_total\":{total}",
    );
}
