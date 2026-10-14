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

// ---------------------------------------------------------------------------
// BOUCHAUD_ATTRIBUTION_REVEIL_V2 : la ventilation de chaque reveil.

fn pit(debut: u64, fin: u64, masque: u64) -> Absence {
    Absence { cpu: 0, source: SOURCE_PIT, debut_ns: debut, fin_ns: fin, masque_ns: masque }
}

fn quantum(cpu: u32, debut: u64, fin: u64) -> Absence {
    Absence { cpu, source: SOURCE_QUANTUM, debut_ns: debut, fin_ns: fin, masque_ns: 0 }
}

fn somme(v: &Ventilation) -> u64 {
    v.livraison_externe_ns + v.livraison_masquee_ns + v.detection_ns + v.election_ns + v.election_externe_ns
        + v.reprise_ns + v.reprise_externe_ns
}

#[test]
fn un_battement_dans_sa_periode_n_ouvre_aucune_absence() {
    assert_eq!(absence_de(0, SOURCE_PIT, 10 * MS, 12 * MS, PERIODE_PIT_NS, 0), None);
    let a = absence_de(0, SOURCE_PIT, 10 * MS, 13 * MS + 1, PERIODE_PIT_NS, 0).unwrap();
    // L'absence commence quand le battement manquant etait du.
    assert_eq!((a.debut_ns, a.fin_ns), (11 * MS, 13 * MS + 1));
    // Quantum de 4 ms : 5 ms sans battement, c'est encore dans la tolerance
    // (une milliseconde, comme pour le PIT).
    assert_eq!(absence_de(3, SOURCE_QUANTUM, 10 * MS, 15 * MS, 4 * MS, 0), None);
    let a = absence_de(3, SOURCE_QUANTUM, 10 * MS, 15 * MS + 1, 4 * MS, 0).unwrap();
    assert_eq!((a.debut_ns, a.fin_ns), (14 * MS, 15 * MS + 1));
    // Premier battement d'un coeur : rien a comparer.
    assert_eq!(absence_de(0, SOURCE_PIT, 0, 50 * MS, PERIODE_PIT_NS, 0), None);
}

#[test]
fn une_absence_close_par_une_longue_section_masquee_est_celle_du_noyau() {
    assert!(pit(10 * MS, 14 * MS, 0).externe());
    assert!(pit(10 * MS, 14 * MS, 1_999_999).externe());
    assert!(!pit(10 * MS, 14 * MS, 2 * MS).externe());
}

#[test]
fn repos_kvm_le_tic_manquant_contamine_le_reveil() {
    // Run 38569175760, phase « au repos » : echeance a 100 ms, le PIT manque
    // de 99,5 a 103,9 ms, hors de toute section masquee ; le balayage rend la
    // tache prete a 104 ms.
    let m = mesure(100 * MS, 84 * MS, 104 * MS, 104 * MS + 20_000, 104 * MS + 30_000, 16);
    let d = decompose(&m);
    let v = ventile(&m, &d, 0, &[pit(99 * MS + 500_000, 103 * MS + 900_000, 0)], false);
    assert_eq!(v.livraison_externe_ns, 3_900_000);
    assert_eq!(v.detection_ns, 100_000);
    assert_eq!(somme(&v), d.total_ns);
    assert!(v.contamine());
}

#[test]
fn un_fil_noyau_qui_tient_le_coeur_n_est_pas_une_contamination() {
    // Cause 1 de l'endurance : tous les tics la, balayage 9 ms en retard.
    let m = mesure(116 * MS, 100 * MS, 125 * MS, 125 * MS + 300_000, 125 * MS + 330_000, 25);
    let d = decompose(&m);
    let v = ventile(&m, &d, 0, &[], false);
    assert_eq!(v.detection_ns, 9 * MS);
    assert_eq!(v.livraison_externe_ns, 0);
    assert!(!v.contamine());
}

#[test]
fn une_absence_masquee_reste_au_noyau() {
    let m = mesure(100 * MS, 84 * MS, 104 * MS, 104 * MS, 104 * MS + 10_000, 16);
    let d = decompose(&m);
    let v = ventile(&m, &d, 0, &[pit(100 * MS, 104 * MS, 3 * MS)], false);
    assert_eq!(v.livraison_masquee_ns, 4 * MS);
    assert_eq!(v.livraison_externe_ns, 0);
    assert!(!v.contamine());
}

#[test]
fn attendre_son_tour_derriere_un_calcul_est_la_politique() {
    // SMP8 : prete a 101 ms, elue 4,8 ms plus tard sur le coeur 3, dont
    // tous les battements sont la.
    let m = mesure(100 * MS, 84 * MS, 101 * MS, 105 * MS + 800_000, 105 * MS + 820_000, 21);
    let d = decompose(&m);
    let v = ventile(&m, &d, 3, &[], false);
    assert_eq!(v.election_ns, 4_800_000);
    assert_eq!(v.election_externe_ns, 0);
    assert!(!v.contamine());
}

#[test]
fn le_coeur_qui_elit_absent_rend_l_election_externe() {
    let m = mesure(100 * MS, 84 * MS, 101 * MS, 109 * MS, 109 * MS + 20_000, 25);
    let d = decompose(&m);
    // Le quantum du coeur 3 manque de 102 a 108 ms.
    let v = ventile(&m, &d, 3, &[quantum(3, 102 * MS, 108 * MS)], false);
    assert_eq!(v.election_externe_ns, 6 * MS);
    assert_eq!(v.election_ns, 2 * MS);
    assert_eq!(somme(&v), d.total_ns);
    assert!(v.contamine());
}

#[test]
fn l_absence_d_un_autre_coeur_ne_compte_pas_pour_l_election() {
    let m = mesure(100 * MS, 84 * MS, 101 * MS, 109 * MS, 109 * MS + 20_000, 25);
    let d = decompose(&m);
    let v = ventile(&m, &d, 3, &[quantum(5, 102 * MS, 108 * MS)], false);
    assert_eq!(v.election_externe_ns, 0);
    assert_eq!(v.election_ns, 8 * MS);
    assert!(!v.contamine());
}

#[test]
fn deux_sources_du_meme_coeur_ne_comptent_pas_deux_fois() {
    // Coeur zero : le PIT et le quantum manquent ensemble, 102 -> 108 ms.
    let m = mesure(100 * MS, 84 * MS, 101 * MS, 109 * MS, 109 * MS + 20_000, 25);
    let d = decompose(&m);
    let v = ventile(&m, &d, 0, &[pit(102 * MS, 108 * MS, 0), quantum(0, 103 * MS, 108 * MS)], false);
    assert_eq!(v.election_externe_ns, 6 * MS);
    assert_eq!(somme(&v), d.total_ns);
}

#[test]
fn sans_balayage_ce_sont_les_battements_du_coeur_de_la_tache() {
    // La tache dormait seule sur le coeur 2 (hlt) et s'est reveillee
    // elle-meme ; le quantum du coeur 2 manquait.
    let m = mesure(100 * MS, 84 * MS, 0, 0, 107 * MS, 20);
    let d = decompose(&m);
    assert!(d.sans_balayage);
    let v = ventile(&m, &d, 2, &[quantum(2, 101 * MS, 106 * MS)], false);
    assert_eq!(v.livraison_externe_ns, 5 * MS);
    assert!(v.contamine());
}

#[test]
fn une_ventilation_incomplete_est_tenue_pour_contaminee() {
    let m = mesure(116 * MS, 100 * MS, 125 * MS, 125 * MS, 125 * MS, 25);
    let d = decompose(&m);
    let v = ventile(&m, &d, 0, &[], true);
    assert!(v.incomplete && v.contamine());
}

#[test]
fn le_registre_liste_les_contamines_et_se_remet_a_zero() {
    let r = RegistreVentilation::neuf();
    let externe = Ventilation { livraison_externe_ns: 3 * MS, detection_ns: 100_000, ..Default::default() };
    let interne = Ventilation { detection_ns: 9 * MS, ..Default::default() };
    for i in 0..40u64 {
        r.note(&externe, (1000 + i * 16) * MS);
    }
    r.note(&interne, 5000 * MS);
    let v = r.releve();
    assert_eq!((v.ventiles, v.contamines, v.listes), (41, 40, LISTE_CONTAMINES));
    assert_eq!(v.liste[0], (1_000_000, 3000));
    assert_eq!(v.liste[1], (1_016_000, 3000));
    assert_eq!(v.livraison_externe.n, 40);
    assert_eq!(v.detection.n, 1);
    assert_eq!(v.detection.pire_ns, 9 * MS);
    let w = r.releve();
    assert_eq!((w.ventiles, w.contamines, w.listes), (0, 0, 0));
}

#[test]
fn le_journal_rend_les_absences_de_l_intervalle_et_avoue_ce_qu_il_a_perdu() {
    let _g = GLOBAUX.lock().unwrap();
    let base = 10_000 * MS;
    journalise_absence(&pit(base, base + 3 * MS, 0));
    journalise_absence(&quantum(2, base + 20 * MS, base + 27 * MS));
    let mut sortie = [Absence::default(); 16];
    let (n, incomplet) = absences_entre(base + MS, base + 2 * MS, &mut sortie);
    assert_eq!(n, 1);
    assert_eq!(sortie[0], pit(base, base + 3 * MS, 0));
    let _ = incomplet;
    // L'anneau tourne : 300 absences plus recentes ecrasent les anciennes.
    for i in 0..300u64 {
        let t = base + 100 * MS + i * 10 * MS;
        journalise_absence(&pit(t, t + 3 * MS, 0));
    }
    let (_, incomplet) = absences_entre(base + MS, base + 2 * MS, &mut sortie);
    assert!(incomplet, "un intervalle anterieur au plus ancien releve conserve doit etre declare incomplet");
    // Un intervalle recent reste connu.
    let recent = base + 100 * MS + 299 * 10 * MS;
    let (n, incomplet) = absences_entre(recent + MS, recent + 2 * MS, &mut sortie);
    assert_eq!(n, 1);
    assert!(!incomplet);
}

#[test]
fn le_quantum_local_journalise_son_absence_et_la_compte() {
    let _g = GLOBAUX.lock().unwrap();
    let (externes_avant, _) = absences_comptees();
    let base = 50_000 * MS;
    // Coeur 7, quantum de 4 ms : battements a l'heure, puis 9 ms de silence.
    note_quantum(7, base, 0, 4 * MS, 0);
    note_quantum(7, base + 4 * MS, 0, 4 * MS, 0);
    note_quantum(7, base + 13 * MS, 0, 4 * MS, 0);
    let (externes, _) = absences_comptees();
    assert_eq!(externes - externes_avant, 1);
    let mut sortie = [Absence::default(); 16];
    let (n, _) = absences_entre(base + 9 * MS, base + 10 * MS, &mut sortie);
    assert_eq!(n, 1);
    assert_eq!(sortie[0], quantum(7, base + 8 * MS, base + 13 * MS));
}
