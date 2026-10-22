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
        commute_ns: 0,
        passe_ns: 0,
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
    note_tic_tsc(0, 100 * MS, 100 * MS, 0x1000, false, cpm, None, Courant::default());
    note_tic_tsc(0, 101 * MS, 101 * MS, 0x1000, false, cpm, None, Courant::default());
    // Section masquee de 9 ms, terminee 20 us avant le tic qui clot un ecart
    // de 10 ms : le noyau.
    note_section_masquee(0, 101 * MS + 980_000, 110 * MS + 980_000, SITE_VECTEUR | 0x20, cpm);
    note_tic_tsc(0, 111 * MS, 111 * MS, 0x2000, false, cpm, None, Courant::default());
    // Puis un ecart de 30 ms qui finit en ring 3, sans section avant : hors
    // de l'invite.
    note_tic_tsc(0, 141 * MS, 141 * MS, 0x4000_0040_1d10, true, cpm, None, Courant::default());
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
    note_tic_tsc(1, 200 * MS, 200 * MS, 0, false, cpm, None, Courant::default());
    let site = 0xdead_0000u64;
    note_section_masquee(1, 200 * MS + 500_000, 212 * MS, site, cpm);
    note_tic_tsc(1, 212 * MS + 10_000, 212 * MS + 10_000, 0x3000, false, cpm, None, Courant::default());
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
// BOUCHAUD_PREUVE_HOTE_V1 : « externe » exige le vol prouve par l'hote.

fn pit(debut: u64, fin: u64, masque: u64) -> Absence {
    Absence { cpu: 0, source: SOURCE_PIT, debut_ns: debut, fin_ns: fin, masque_ns: masque, ..Default::default() }
}

/// Une absence du PIT dont l'hyperviseur prouve `vole` ns de vol.
fn pit_vole(debut: u64, fin: u64, vole: u64) -> Absence {
    Absence { vole_ns: vole, ..pit(debut, fin, 0) }
}

fn quantum(cpu: u32, debut: u64, fin: u64) -> Absence {
    Absence { cpu, source: SOURCE_QUANTUM, debut_ns: debut, fin_ns: fin, ..Default::default() }
}

fn quantum_vole(cpu: u32, debut: u64, fin: u64) -> Absence {
    Absence { vole_ns: fin - debut, ..quantum(cpu, debut, fin) }
}

fn somme(v: &Ventilation) -> u64 {
    v.livraison_externe_ns + v.livraison_masquee_ns + v.livraison_inexpliquee_ns + v.detection_ns + v.election_ns
        + v.election_externe_ns + v.election_inexpliquee_ns + v.reprise_ns + v.reprise_externe_ns
        + v.reprise_inexpliquee_ns
}

const SANS: Courant = Courant { tache: 0, depuis_ns: 0 };

#[test]
fn un_battement_dans_sa_periode_n_ouvre_aucune_absence() {
    assert_eq!(absence_de(0, SOURCE_PIT, 10 * MS, 12 * MS, PERIODE_PIT_NS, 0, None, SANS), None);
    let a = absence_de(0, SOURCE_PIT, 10 * MS, 13 * MS + 1, PERIODE_PIT_NS, 0, None, SANS).unwrap();
    // L'absence commence quand le battement manquant etait du.
    assert_eq!((a.debut_ns, a.fin_ns), (11 * MS, 13 * MS + 1));
    // Quantum de 4 ms : 5 ms sans battement, c'est encore dans la tolerance
    // (une milliseconde, comme pour le PIT).
    assert_eq!(absence_de(3, SOURCE_QUANTUM, 10 * MS, 15 * MS, 4 * MS, 0, None, SANS), None);
    let a = absence_de(3, SOURCE_QUANTUM, 10 * MS, 15 * MS + 1, 4 * MS, 0, None, SANS).unwrap();
    assert_eq!((a.debut_ns, a.fin_ns), (14 * MS, 15 * MS + 1));
    // Premier battement d'un coeur : rien a comparer.
    assert_eq!(absence_de(0, SOURCE_PIT, 0, 50 * MS, PERIODE_PIT_NS, 0, None, SANS), None);
}

#[test]
fn le_vol_retenu_est_borne_par_l_absence_et_ampute_d_une_periode() {
    // 10 -> 15 ms : absence 11 -> 15 ms (4 ms). L'hote a vole 4,5 ms entre
    // les deux battements ; au plus une periode (1 ms) de ce vol a pu
    // tomber AVANT l'absence : la preuve retenue est 3,5 ms.
    let a = absence_de(0, SOURCE_PIT, 10 * MS, 15 * MS, PERIODE_PIT_NS, 0, Some(4_500_000), SANS).unwrap();
    assert_eq!(a.vole_ns, 3_500_000);
    assert_eq!(a.cause(), Cause::Externe);
    // Jamais plus que l'absence.
    let a = absence_de(0, SOURCE_PIT, 10 * MS, 15 * MS, PERIODE_PIT_NS, 0, Some(40 * MS), SANS).unwrap();
    assert_eq!(a.vole_ns, 4 * MS);
    // Sans preuve : zero, et l'absence n'est pas externe.
    let a = absence_de(0, SOURCE_PIT, 10 * MS, 15 * MS, PERIODE_PIT_NS, 0, None, SANS).unwrap();
    assert_eq!((a.vole_ns, a.cause()), (0, Cause::Inexpliquee));
}

#[test]
fn le_vol_entre_deux_battements_exige_deux_lectures() {
    assert_eq!(vol_entre(1_000, Some(4_000)), Some(3_000));
    assert_eq!(vol_entre(u64::MAX, Some(4_000)), None);
    assert_eq!(vol_entre(1_000, None), None);
    // Un compteur qui recule (structure reprogrammee) ne prouve rien.
    assert_eq!(vol_entre(5_000, Some(4_000)), None);
}

#[test]
fn la_cause_d_une_absence_exige_une_preuve() {
    // Masquee sur la moitie : le noyau.
    assert_eq!(pit(10 * MS, 14 * MS, 2 * MS).cause(), Cause::Masquee);
    // Ni masquee ni prouvee : INEXPLIQUEE, jamais externe.
    assert_eq!(pit(10 * MS, 14 * MS, 0).cause(), Cause::Inexpliquee);
    assert_eq!(pit(10 * MS, 14 * MS, 1_999_999).cause(), Cause::Inexpliquee);
    assert!(!pit(10 * MS, 14 * MS, 0).externe());
    // Le vol prouve couvre la moitie : externe.
    assert_eq!(pit_vole(10 * MS, 14 * MS, 2 * MS).cause(), Cause::Externe);
    assert_eq!(pit_vole(10 * MS, 14 * MS, 1_999_999).cause(), Cause::Inexpliquee);
}

#[test]
fn repos_kvm_un_tic_manquant_prouve_contamine_le_reveil() {
    // Run 38569175760, phase « au repos » : echeance a 100 ms, le PIT manque
    // de 99,5 a 103,9 ms ; ici, l'hyperviseur prouve 4 ms de vol.
    let m = mesure(100 * MS, 84 * MS, 104 * MS, 104 * MS + 20_000, 104 * MS + 30_000, 16);
    let d = decompose(&m);
    let v = ventile(&m, &d, 0, &[pit_vole(99 * MS + 500_000, 103 * MS + 900_000, 4 * MS)], false);
    assert_eq!(v.livraison_externe_ns, 3_900_000);
    assert_eq!(v.detection_ns, 100_000);
    assert_eq!(somme(&v), d.total_ns);
    assert!(v.contamine());
}

#[test]
fn le_meme_tic_manquant_sans_preuve_reste_au_compte_du_noyau() {
    // Meme reveil, mais aucune preuve de l'hote (TCG, ou hote qui n'a rien
    // vole) : la part est INEXPLIQUEE et le reveil n'est pas ecarte.
    let m = mesure(100 * MS, 84 * MS, 104 * MS, 104 * MS + 20_000, 104 * MS + 30_000, 16);
    let d = decompose(&m);
    let v = ventile(&m, &d, 0, &[pit(99 * MS + 500_000, 103 * MS + 900_000, 0)], false);
    assert_eq!(v.livraison_externe_ns, 0);
    assert_eq!(v.livraison_inexpliquee_ns, 3_900_000);
    assert_eq!(v.inexpliquee_ns(), 3_900_000);
    assert_eq!(somme(&v), d.total_ns);
    assert!(!v.contamine());
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
fn le_coeur_qui_elit_absent_et_prouve_rend_l_election_externe() {
    let m = mesure(100 * MS, 84 * MS, 101 * MS, 109 * MS, 109 * MS + 20_000, 25);
    let d = decompose(&m);
    // Le quantum du coeur 3 manque de 102 a 108 ms, vol prouve.
    let v = ventile(&m, &d, 3, &[quantum_vole(3, 102 * MS, 108 * MS)], false);
    assert_eq!(v.election_externe_ns, 6 * MS);
    assert_eq!(v.election_ns, 2 * MS);
    assert_eq!(somme(&v), d.total_ns);
    assert!(v.contamine());
    // Sans preuve : la meme attente est inexpliquee, pas externe.
    let v = ventile(&m, &d, 3, &[quantum(3, 102 * MS, 108 * MS)], false);
    assert_eq!((v.election_externe_ns, v.election_inexpliquee_ns, v.election_ns), (0, 6 * MS, 2 * MS));
    assert!(!v.contamine());
}

#[test]
fn l_absence_d_un_autre_coeur_ne_compte_pas_pour_l_election() {
    let m = mesure(100 * MS, 84 * MS, 101 * MS, 109 * MS, 109 * MS + 20_000, 25);
    let d = decompose(&m);
    let v = ventile(&m, &d, 3, &[quantum_vole(5, 102 * MS, 108 * MS)], false);
    assert_eq!(v.election_externe_ns, 0);
    assert_eq!(v.election_ns, 8 * MS);
    assert!(!v.contamine());
}

#[test]
fn deux_sources_du_meme_coeur_ne_comptent_pas_deux_fois() {
    // Coeur zero : le PIT et le quantum manquent ensemble, 102 -> 108 ms.
    let m = mesure(100 * MS, 84 * MS, 101 * MS, 109 * MS, 109 * MS + 20_000, 25);
    let d = decompose(&m);
    let v = ventile(&m, &d, 0, &[pit_vole(102 * MS, 108 * MS, 6 * MS), quantum_vole(0, 103 * MS, 108 * MS)], false);
    assert_eq!(v.election_externe_ns, 6 * MS);
    assert_eq!(somme(&v), d.total_ns);
}

#[test]
fn sans_balayage_ce_sont_les_battements_du_coeur_de_la_tache() {
    // La tache dormait seule sur le coeur 2 (hlt) et s'est reveillee
    // elle-meme ; le quantum du coeur 2 manquait, vol prouve.
    let m = mesure(100 * MS, 84 * MS, 0, 0, 107 * MS, 20);
    let d = decompose(&m);
    assert!(d.sans_balayage);
    let v = ventile(&m, &d, 2, &[quantum_vole(2, 101 * MS, 106 * MS)], false);
    assert_eq!(v.livraison_externe_ns, 5 * MS);
    assert!(v.contamine());
}

#[test]
fn une_ventilation_incomplete_ne_prouve_rien() {
    // Avant BOUCHAUD_PREUVE_HOTE_V1, un journal incomplet ECARTAIT le reveil.
    // Une inconnue n'est pas une preuve : le reveil reste compte.
    let m = mesure(116 * MS, 100 * MS, 125 * MS, 125 * MS, 125 * MS, 25);
    let d = decompose(&m);
    let v = ventile(&m, &d, 0, &[], true);
    assert!(v.incomplete);
    assert!(!v.contamine());
}

#[test]
fn le_registre_compte_contamines_et_inexpliques_et_se_remet_a_zero() {
    let r = RegistreVentilation::neuf();
    let externe = Ventilation { livraison_externe_ns: 3 * MS, detection_ns: 100_000, ..Default::default() };
    let inexpliquee = Ventilation { livraison_inexpliquee_ns: 3 * MS, ..Default::default() };
    let interne = Ventilation { detection_ns: 9 * MS, ..Default::default() };
    for _ in 0..40 {
        r.note(&externe);
    }
    for _ in 0..7 {
        r.note(&inexpliquee);
    }
    r.note(&interne);
    let v = r.releve();
    assert_eq!((v.ventiles, v.contamines, v.inexpliques), (48, 40, 7));
    assert_eq!(v.livraison_externe.n, 40);
    assert_eq!(v.livraison_inexpliquee.n, 7);
    assert_eq!(v.detection.n, 1);
    assert_eq!(v.detection.pire_ns, 9 * MS);
    let w = r.releve();
    assert_eq!((w.ventiles, w.contamines, w.inexpliques), (0, 0, 0));
}

#[test]
fn le_journal_rend_les_absences_de_l_intervalle_et_avoue_ce_qu_il_a_perdu() {
    let _g = GLOBAUX.lock().unwrap_or_else(|e| e.into_inner());
    let base = 10_000 * MS;
    let premiere = Absence { courant: Courant { tache: 9, depuis_ns: base - MS }, ..pit_vole(base, base + 3 * MS, 2 * MS) };
    journalise_absence(&premiere);
    journalise_absence(&quantum(2, base + 20 * MS, base + 27 * MS));
    let mut sortie = [Absence::default(); 16];
    let (n, incomplet) = absences_entre(base + MS, base + 2 * MS, &mut sortie);
    assert_eq!(n, 1);
    // Le vol et la tache installee traversent le journal.
    assert_eq!(sortie[0], premiere);
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
fn le_quantum_local_journalise_son_absence_avec_sa_cause() {
    let _g = GLOBAUX.lock().unwrap_or_else(|e| e.into_inner());
    let (e0, _, i0) = absences_comptees();
    let base = 50_000 * MS;
    // Coeur 7, quantum de 4 ms : battements a l'heure, puis 9 ms de silence,
    // sans preuve de l'hote.
    note_quantum(7, base, 0, 4 * MS, 0, None, SANS);
    note_quantum(7, base + 4 * MS, 0, 4 * MS, 0, None, SANS);
    note_quantum(7, base + 13 * MS, 0, 4 * MS, 0, None, SANS);
    let (e1, _, i1) = absences_comptees();
    assert_eq!((e1 - e0, i1 - i0), (0, 1));
    let mut sortie = [Absence::default(); 16];
    let (n, _) = absences_entre(base + 9 * MS, base + 10 * MS, &mut sortie);
    assert_eq!(n, 1);
    assert_eq!(sortie[0], quantum(7, base + 8 * MS, base + 13 * MS));
    // Meme silence, l'hote prouve 8 ms de vol entre les deux battements :
    // externe.
    note_quantum(6, base, 0, 4 * MS, 0, Some(1_000), SANS);
    note_quantum(6, base + 4 * MS, 0, 4 * MS, 0, Some(1_000), SANS);
    note_quantum(6, base + 13 * MS, 0, 4 * MS, 0, Some(1_000 + 8 * MS), SANS);
    let (e2, _, i2) = absences_comptees();
    assert_eq!((e2 - e1, i2 - i1), (1, 0));
}

// ---------------------------------------------------------------------------
// BOUCHAUD_PREUVE_HOTE_V1 : les comptes cumules d'un fil.

#[test]
fn une_absence_n_est_imputee_qu_au_fil_installe_et_depuis_sa_tranche() {
    let a = Absence { courant: Courant { tache: 4, depuis_ns: 105 * MS }, ..pit(100 * MS, 110 * MS, 0) };
    // Le fil 4 n'etait installe que depuis 105 ms : 5 ms, pas 10.
    assert_eq!(part_du_fil(&a, 4), 5 * MS);
    // Un autre fil : rien.
    assert_eq!(part_du_fil(&a, 5), 0);
    // Installe avant l'absence : toute l'absence.
    let b = Absence { courant: Courant { tache: 4, depuis_ns: 90 * MS }, ..pit(100 * MS, 110 * MS, 0) };
    assert_eq!(part_du_fil(&b, 4), 10 * MS);
    // Coeur au repos (aucune tache, ou aucune tranche) : rien.
    assert_eq!(part_du_fil(&pit(100 * MS, 110 * MS, 0), 0), 0);
    let c = Absence { courant: Courant { tache: 4, depuis_ns: 0 }, ..pit(100 * MS, 110 * MS, 0) };
    assert_eq!(part_du_fil(&c, 4), 0);
}

#[test]
fn les_comptes_du_fil_ajoutent_livraison_et_election_pas_la_reprise() {
    let f = ComptesFil::neuf();
    let v = Ventilation {
        livraison_externe_ns: 2 * MS,
        livraison_inexpliquee_ns: 3 * MS,
        livraison_masquee_ns: 500_000,
        election_externe_ns: MS,
        election_inexpliquee_ns: 700_000,
        reprise_externe_ns: 9 * MS,
        reprise_inexpliquee_ns: 9 * MS,
        ..Default::default()
    };
    f.note_ventilation(&v);
    f.note_pret(1_200_000);
    let r = f.cumuls();
    assert_eq!((r.externe_ns, r.inexplique_ns, r.masque_ns, r.pret_ns), (3 * MS, 3_700_000, 500_000, 1_200_000));
}

#[test]
fn le_journal_rend_les_absences_closes_du_fil_dans_la_fenetre() {
    let _g = GLOBAUX.lock().unwrap_or_else(|e| e.into_inner());
    let base = 900_000 * MS;
    // Close avant la fenetre : exclue.
    journalise_absence(&Absence { courant: Courant { tache: 77, depuis_ns: base - 50 * MS }, ..pit(base - 20 * MS, base - 10 * MS, 0) });
    // Deux absences du fil 77 dans la fenetre, une prouvee, une non.
    journalise_absence(&Absence { courant: Courant { tache: 77, depuis_ns: base }, ..pit_vole(base + 10 * MS, base + 13 * MS, 3 * MS) });
    journalise_absence(&Absence { courant: Courant { tache: 77, depuis_ns: base }, ..pit(base + 20 * MS, base + 22 * MS, 0) });
    // Une absence d'un autre fil.
    journalise_absence(&Absence { courant: Courant { tache: 78, depuis_ns: base }, ..pit_vole(base + 30 * MS, base + 35 * MS, 5 * MS) });
    let mut sortie = [Absence::default(); 8];
    let (n, incomplet) = absences_closes_entre(base, base + 40 * MS, 77, &mut sortie);
    assert!(!incomplet);
    assert_eq!(n, 2);
    let causes: Vec<_> = sortie[..n].iter().map(|a| (a.cause(), part_du_fil(a, 77))).collect();
    assert!(causes.contains(&(Cause::Externe, 3 * MS)));
    assert!(causes.contains(&(Cause::Inexpliquee, 2 * MS)));
}

#[test]
fn l_enregistreur_range_les_sommeils_du_seul_fil_arme() {
    let e = Enregistreur::neuf();
    let s = |k: u64| Sommeil { echeance_ns: k * 16 * MS, entree_ns: k * 16 * MS - 15 * MS, comptes: ReleveFil { pret_ns: k, ..Default::default() } };
    // Non arme : rien.
    e.note(5, &s(1));
    assert_eq!(e.arme_a(5), 0);
    e.arme(5, 1234);
    assert_eq!(e.arme_a(5), 1234);
    for k in 1..=3 {
        e.note(5, &s(k));
        e.note(6, &s(100 + k)); // un autre fil : ignore
    }
    let mut lus = Vec::new();
    assert_eq!(e.lis(5, |x| lus.push(*x)), 0);
    assert_eq!(lus, vec![s(1), s(2), s(3)]);
    // Un autre fil ne lit rien.
    let mut autres = 0;
    e.lis(6, |_| autres += 1);
    assert_eq!(autres, 0);
    // Au-dela de la capacite : compte, pas range.
    e.arme(5, 0);
    for k in 0..(SOMMEILS_ENREGISTRES as u64 + 7) {
        e.note(5, &s(k + 1));
    }
    let mut n = 0;
    assert_eq!(e.lis(5, |_| n += 1), 7);
    assert_eq!(n, SOMMEILS_ENREGISTRES);
}

#[test]
fn l_enregistreur_range_les_absences_du_seul_fil_arme_rognees_a_sa_tranche() {
    let e = Enregistreur::neuf();
    let avec = |tache: u32, depuis: u64, a: Absence| Absence { courant: Courant { tache, depuis_ns: depuis }, ..a };
    // Non arme : rien.
    e.note_absence(&avec(9, 100 * MS, pit_vole(100 * MS, 104 * MS, 4 * MS)));
    e.arme(9, 50 * MS);
    // Prouvee, installe depuis 102 ms : 2 ms rognees.
    e.note_absence(&avec(9, 102 * MS, pit_vole(100 * MS, 104 * MS, 4 * MS)));
    // Inexpliquee, toute l'absence.
    e.note_absence(&avec(9, 90 * MS, pit(200 * MS, 203 * MS, 0)));
    // Un autre fil, et un coeur au repos : ignores.
    e.note_absence(&avec(8, 90 * MS, pit(300 * MS, 303 * MS, 0)));
    e.note_absence(&pit(400 * MS, 403 * MS, 0));
    let mut lues = Vec::new();
    assert!(!e.lis_absences(9, |a| lues.push(*a)));
    assert_eq!(lues, vec![
        AbsenceDuFil { fin_ns: 104 * MS, cause: Cause::Externe, part_ns: 2 * MS },
        AbsenceDuFil { fin_ns: 203 * MS, cause: Cause::Inexpliquee, part_ns: 3 * MS },
    ]);
    // Rearmer efface ; deborder est avoue.
    e.arme(9, 0);
    for k in 0..(ABSENCES_ENREGISTREES as u64 + 3) {
        e.note_absence(&avec(9, 1, pit(k * 10 * MS + MS, k * 10 * MS + 4 * MS, 0)));
    }
    let mut n = 0;
    assert!(e.lis_absences(9, |_| n += 1), "plus d'absences que de place : incomplet");
    assert_eq!(n, ABSENCES_ENREGISTREES);
}

// BOUCHAUD_REPRISE_DECOUPEE_V1 : la reprise (elu -> retour) en trois segments.
#[test]
fn la_reprise_se_decoupe_en_commutation_passation_retour() {
    let m = Mesure {
        commute_ns: 125 * MS + 300_000 + 5_000,
        passe_ns: 125 * MS + 300_000 + 1_205_000,
        ..mesure(116 * MS, 100 * MS, 125 * MS, 125 * MS + 300_000, 125 * MS + 300_000 + 1_226_000, 25)
    };
    let d = decompose(&m);
    assert_eq!(d.reprise_ns, 1_226_000);
    assert_eq!((d.commutation_ns, d.passation_ns, d.retour_ns), (5_000, 1_200_000, 21_000));
    assert_eq!(d.commutation_ns + d.passation_ns + d.retour_ns, d.reprise_ns);
}

#[test]
fn reprise_sans_instants_ou_incoherents_reste_au_retour() {
    // Non notes : rien n'est invente.
    let d = decompose(&mesure(116 * MS, 100 * MS, 125 * MS, 125 * MS + 300_000, 126 * MS, 25));
    assert_eq!((d.commutation_ns, d.passation_ns, d.retour_ns), (0, 0, 700_000));
    // Instants d'une AUTRE commutation (avant l'election) : ignores.
    let m = Mesure {
        commute_ns: 120 * MS,
        passe_ns: 120 * MS + 10,
        ..mesure(116 * MS, 100 * MS, 125 * MS, 125 * MS + 300_000, 126 * MS, 25)
    };
    let d = decompose(&m);
    assert_eq!((d.commutation_ns, d.passation_ns, d.retour_ns), (0, 0, 700_000));
    // Passation apres la reprise : incoherent, ignore.
    let m = Mesure {
        commute_ns: 125 * MS + 400_000,
        passe_ns: 127 * MS,
        ..mesure(116 * MS, 100 * MS, 125 * MS, 125 * MS + 300_000, 126 * MS, 25)
    };
    let d = decompose(&m);
    assert_eq!((d.commutation_ns, d.passation_ns, d.retour_ns), (0, 0, 700_000));
}
