//! D'ou vient le retard d'un reveil (BOUCHAUD_ATTRIBUTION_REVEIL_V1).
//!
//! Les cas sont ceux que la sonde `ordonnanceur-probe` a releves sous TCG :
//! un balayage des echeances tardif avec tous les tics (un fil noyau tenait
//! le coeur), et un balayage tardif faute de tics (temps perdu hors de
//! l'invite).

#[path = "../../src/kernel/scheduler/attribution_reveil.rs"]
mod attribution;

use attribution::*;
use std::sync::Mutex;

const MS: u64 = 1_000_000;

/// Les releves de tics et de masquage sont globaux (comme dans le noyau) :
/// les tests qui les touchent passent l'un apres l'autre.
static GLOBAUX: Mutex<()> = Mutex::new(());

fn mesure(echeance: u64, entree: u64, pret: u64, elu: u64, reprise: u64, tics: u64) -> Mesure {
    Mesure {
        echeance_ns: echeance,
        entree_ns: entree,
        pret_ns: pret,
        elu_ns: elu,
        reprise_ns: reprise,
        tics_entree: 1000,
        tics_reprise: 1000 + tics,
    }
}

#[test]
fn les_trois_parts_somment_le_retard() {
    // Sommeil de 16 ms ; pret 9 ms apres l'echeance, elu 0,3 ms plus tard.
    let m = mesure(116 * MS, 100 * MS, 125 * MS, 125 * MS + 300_000, 125 * MS + 330_000, 25);
    let d = decompose(&m);
    assert_eq!(d.total_ns, 9 * MS + 330_000);
    assert_eq!(d.tardif_ns, 9 * MS);
    assert_eq!(d.attente_ns, 300_000);
    assert_eq!(d.reprise_ns, 30_000);
    assert_eq!(d.tardif_ns + d.attente_ns + d.reprise_ns, d.total_ns);
    assert!(!d.sans_balayage);
}

#[test]
fn balayage_tardif_avec_tous_les_tics_n_est_pas_un_tic_perdu() {
    // run local smp1-c : dormi 23 ms, 23 tics -- un fil noyau tenait le coeur.
    let m = mesure(116 * MS, 100 * MS, 125 * MS, 125 * MS, 123 * MS + 100 * MS, 23);
    let m = Mesure { reprise_ns: 123 * MS + 400_000, ..m };
    let d = decompose(&Mesure { pret_ns: 123 * MS, elu_ns: 123 * MS + 200_000, ..m });
    assert_eq!(d.dormi_ms, 23);
    assert_eq!(d.tics_manquants, 0);
    assert_eq!(d.tardif_ns, 7 * MS);
}

#[test]
fn balayage_tardif_faute_de_tics() {
    // run local smp1-a, passe 5 : dormi 81 ms, 3 tics -- 77 ms sans tic.
    let m = mesure(116 * MS, 100 * MS, 181 * MS, 181 * MS + 300_000, 181 * MS + 330_000, 3);
    let d = decompose(&m);
    assert_eq!(d.dormi_ms, 81);
    assert_eq!(d.tics_vus, 3);
    assert_eq!(d.tics_manquants, 77);
    assert_eq!(d.tardif_ns, 65 * MS);
}

#[test]
fn sans_balayage_tout_le_retard_est_tardif() {
    // Reveillee par un signal, ou jamais garee : pas d'instant `pret`.
    let m = mesure(116 * MS, 100 * MS, 0, 0, 118 * MS, 18);
    let d = decompose(&m);
    assert!(d.sans_balayage);
    assert_eq!(d.tardif_ns, 2 * MS);
    assert_eq!(d.attente_ns, 0);
    // Un `pret` anterieur a l'entree (sommeil precedent) est ignore.
    let d = decompose(&mesure(116 * MS, 100 * MS, 90 * MS, 90 * MS, 118 * MS, 18));
    assert!(d.sans_balayage);
}

#[test]
fn une_election_anterieure_au_pret_est_ignoree() {
    let m = mesure(116 * MS, 100 * MS, 117 * MS, 110 * MS, 118 * MS, 18);
    let d = decompose(&m);
    assert_eq!(d.attente_ns, 0);
    assert_eq!(d.reprise_ns, MS);
}

#[test]
fn le_registre_garde_le_pire_et_se_remet_a_zero() {
    let r = Registre::neuf();
    let petit = decompose(&mesure(116 * MS, 100 * MS, 117 * MS, 117 * MS, 117 * MS, 17));
    let gros = decompose(&mesure(116 * MS, 100 * MS, 125 * MS, 127 * MS, 127 * MS, 27));
    let par = empaquete_par(11, 107, true, false);
    r.note(&petit, 1, 0, 0);
    r.note(&gros, 2, 0, par);
    r.note(&petit, 3, 0, 0);
    let releve = r.releve();
    assert_eq!(releve.reveils, 3);
    assert_eq!(releve.pire, gros);
    assert_eq!(releve.a_ns, 2);
    assert_eq!(depaquete_par(releve.par), Some((11, 107, true, false)));
    assert_eq!(releve.tardifs_signales, 1, "9 ms de tardif > 5 ms");
    assert_eq!(releve.attentes_signalees, 1, "2 ms d'attente > 1 ms");
    // Remis a zero : le releve suivant ne voit que les reveils d'apres.
    let vide = r.releve();
    assert_eq!(vide.reveils, 0);
    assert_eq!(vide.pire.total_ns, 0);
    assert_eq!(depaquete_par(vide.par), None);
}

#[test]
fn un_tic_perdu_garde_son_rip_et_la_section_masquee_qui_le_precede() {
    let _g = GLOBAUX.lock().unwrap_or_else(|e| e.into_inner());
    let _ = releve_tics();
    let _ = releve_masquage();
    let cpm = 1_000_000; // 1 GHz : un cycle par nanoseconde
    note_tic_tsc(0, 100 * MS, 100 * MS, 0x1000, false, cpm);
    note_tic_tsc(0, 101 * MS, 101 * MS, 0x1000, false, cpm);
    // Section masquee de 9 ms, terminee 20 us avant le tic qui clot un ecart
    // de 10 ms : le noyau.
    note_section_masquee(0, 101 * MS + 980_000, 110 * MS + 980_000, SITE_VECTEUR | 0x20, cpm);
    note_tic_tsc(0, 111 * MS, 111 * MS, 0x2000, false, cpm);
    // Puis un ecart de 30 ms qui finit en ring 3, sans section avant : hors
    // de l'invite.
    note_tic_tsc(0, 141 * MS, 141 * MS, 0x4000_0040_1d10, true, cpm);
    let t = releve_tics();
    assert_eq!(t.pire_ns, 30 * MS);
    assert!(t.ring3);
    assert_eq!(t.rip, 0x4000_0040_1d10);
    assert_eq!(t.signales, 2);
    let m = releve_masquage();
    assert_eq!(m.avant_ecart_cyc, 0, "aucune section masquee avant le pire ecart");
    assert_eq!(m.pire_cyc, 9 * MS);
    assert_eq!(m.pire_site, SITE_VECTEUR | 0x20);
    assert_eq!(m.signalees, 1);
}

#[test]
fn un_ecart_clos_par_une_section_masquee_la_nomme() {
    let _g = GLOBAUX.lock().unwrap_or_else(|e| e.into_inner());
    let _ = releve_tics();
    let _ = releve_masquage();
    let cpm = 1_000_000;
    note_tic_tsc(1, 200 * MS, 200 * MS, 0, false, cpm);
    let site = 0xdead_0000u64;
    note_section_masquee(1, 200 * MS + 500_000, 212 * MS, site, cpm);
    note_tic_tsc(1, 212 * MS + 10_000, 212 * MS + 10_000, 0x3000, false, cpm);
    let m = releve_masquage();
    assert_eq!(m.avant_ecart_cyc, 11 * MS + 500_000);
    assert_eq!(m.avant_ecart_site, site);
}

#[test]
fn sortie_irq_mesure_depuis_l_entree() {
    let _g = GLOBAUX.lock().unwrap_or_else(|e| e.into_inner());
    let _ = releve_masquage();
    entree_irq(2, 1_000);
    sortie_irq(2, 3_001_000, 0xf1, 1_000_000);
    let m = releve_masquage();
    assert_eq!(m.pire_cyc, 3_000_000);
    assert_eq!(m.pire_site, SITE_VECTEUR | 0xf1);
    assert_eq!(m.signalees, 1);
}

#[test]
fn une_sortie_apres_commutation_volontaire_n_est_pas_mesuree() {
    // Un gestionnaire suspendu (il avait commute), repris par un `schedule()`
    // volontaire : aucune entree d'IRQ recente sur ce coeur. Mesurer depuis
    // l'entree perimee inventerait une longue section masquee.
    let _g = GLOBAUX.lock().unwrap_or_else(|e| e.into_inner());
    let _ = releve_masquage();
    entree_irq(3, 1_000);
    commutation_hors_irq(3);
    sortie_irq(3, 50_000_000, 0xf1, 1_000_000);
    let m = releve_masquage();
    assert_eq!(m.pire_cyc, 0);
    assert_eq!(m.signalees, 0);
}
