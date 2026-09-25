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
        Commande::ProcessesSnapshot => {
            processus(t);
            true
        }
        Commande::MemorySnapshot => {
            memoire(t);
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
    ip_json(t, "ip_lab", super::ip());
}

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
\"reinitialisations\":{},\"invariant\":\"{}\",\"tx_packets\":{},\"tx_termines\":{}",
        r.xid, r.generation, r.chip_cmd, r.intr_status, r.rx_missed,
        r.rx_paquets, r.rx_octets, r.rx_cur,
        r.rx_desc_materiel, r.rx_desc_processeur,
        r.rx_tours_cpu, r.rx_rendus_tour1, r.rx_rendus_tour2,
        r.rx_rearmes_tour1, r.rx_reutilises_tour2,
        r.isr_rx_ok, r.isr_rx_err, r.isr_rx_overflow, r.isr_rx_fifo_over,
        r.isr_system_error, r.rx_ok_sans_progres, r.rx_own_rendus,
        r.reparations_demandees, r.reparations_executees, r.reparation_degre,
        r.reinitialisations, r.invariant.unwrap_or("intact"),
        r.tx_paquets, r.tx_termines,
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
}

fn services(t: &mut Reponse) {
    let (utilisateur_ms, systeme_ms, charge) = crate::gui::services::usage();
    let _ = write!(
        t,
        ",\"utilisateur_ms\":{utilisateur_ms},\"systeme_ms\":{systeme_ms},\"charge\":{charge}",
    );
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
