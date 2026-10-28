//! Jamais une ligne sans le jeton hors panique (BOUCHAUD_SERIE_JAMAIS_SANS_JETON_V1).
//!
//! os-primitives 1413289b, KVM SMP4 (run 38608429652) : 717 lignes intactes
//! sur 720, trois emises sans le jeton -- des contextes MASQUES (le noyau qui
//! journalise une faute) a leur borne courte. Ici : l'anneau des lignes
//! differees, et un modele du jeton ou le detenteur est SUSPENDU au milieu de
//! sa ligne (processeur virtuel retire par l'hote), avec les vraies fonctions
//! de decision (`attente::a_l_echeance`, `attente::repli`).

#[path = "../../src/drivers/serial/attente.rs"]
mod attente;
#[path = "../../src/drivers/serial/differees.rs"]
mod differees;

use attente::*;
use differees::Anneau;

fn vide_en_lignes<const N: usize>(a: &Anneau<N>) -> Vec<Vec<u8>> {
    let mut lignes = Vec::new();
    let mut courante = Vec::new();
    let n = a.vide(|morceau| courante.extend_from_slice(morceau));
    // `vide` rend des lignes entieres ; on les recoupe sur le saut de ligne.
    for l in courante.split_inclusive(|&b| b == b'\n') {
        lignes.push(l.to_vec());
    }
    assert_eq!(n, lignes.len());
    lignes
}

#[test]
fn l_anneau_rend_les_lignes_entieres_dans_l_ordre_meme_quand_il_tourne() {
    let a: Anneau<64> = Anneau::neuf();
    for tour in 0..20 {
        let l1 = format!("ligne {tour} a\n");
        let l2 = format!("ligne {tour} b, plus longue\n");
        assert!(a.range(l1.as_bytes()));
        assert!(a.range(l2.as_bytes()));
        let lues = vide_en_lignes(&a);
        assert_eq!(lues, vec![l1.into_bytes(), l2.into_bytes()], "tour {tour}");
        assert!(a.est_vide());
    }
}

#[test]
fn une_ligne_qui_ne_tient_pas_est_refusee_entiere() {
    let a: Anneau<32> = Anneau::neuf();
    assert!(a.range(b"0123456789012345678901234\n")); // 26 + 2 octets
    assert!(!a.range(b"abcdef\n"), "plus de place : refusee");
    let lues = vide_en_lignes(&a);
    assert_eq!(lues, vec![b"0123456789012345678901234\n".to_vec()], "rien de la ligne refusee n'a ete ecrit");
    assert!(a.range(b"abcdef\n"));
}

#[test]
fn hors_panique_le_repli_ne_sort_jamais_sans_le_jeton() {
    assert_eq!(repli(false), Repli::Differe);
    assert_eq!(repli(true), Repli::EmetSansJeton);
}

// ---------------------------------------------------------------------------
// Le modele : un port, un jeton, trois coeurs, une horloge en millisecondes.

const MS: u64 = 1_000_000;

struct Systeme<const N: usize> {
    port: Vec<u8>,
    detenteur: Option<usize>,
    emissions_faites: u64,
    anneaux: [Anneau<N>; 3],
    differees: u64,
    perdues: u64,
    sans_jeton: u64,
    panique: bool,
}

impl<const N: usize> Systeme<N> {
    fn neuf(panique: bool) -> Self {
        Self {
            port: Vec::new(),
            detenteur: None,
            emissions_faites: 0,
            anneaux: [Anneau::neuf(), Anneau::neuf(), Anneau::neuf()],
            differees: 0,
            perdues: 0,
            sans_jeton: 0,
            panique,
        }
    }

    /// Ce que fait `sous_jeton` quand le jeton est libre : les lignes
    /// differees d'abord, puis la sienne.
    fn emet_avec_jeton(&mut self, ligne: &[u8]) {
        for a in &self.anneaux {
            let mut tampon = Vec::new();
            a.vide(|m| tampon.extend_from_slice(m));
            self.port.extend_from_slice(&tampon);
        }
        self.port.extend_from_slice(ligne);
        self.emissions_faites += 1;
    }

    /// Ce que fait `sous_jeton` sans le jeton : `repli`.
    fn sans_le_jeton(&mut self, coeur: usize, ligne: &[u8]) {
        match repli(self.panique) {
            Repli::Differe => {
                if self.anneaux[coeur].range(ligne) {
                    self.differees += 1;
                } else {
                    self.perdues += 1;
                }
            }
            Repli::EmetSansJeton => {
                self.sans_jeton += 1;
                self.port.extend_from_slice(ligne);
            }
        }
    }

    /// Un appelant interruptible : il attend, renouvelle tant que d'autres
    /// emettent, renonce sans progres ou au plafond. Rend l'instant ou il a
    /// fini.
    fn interruptible(&mut self, coeur: usize, ligne: &[u8], debut: u64, libre_a: u64) -> u64 {
        let mut echeance = debut + BORNE_SANS_PROGRES_NS;
        let mut vues = self.emissions_faites;
        loop {
            if libre_a <= echeance {
                // Le jeton se libere avant l'echeance.
                self.detenteur = None;
                self.emet_avec_jeton(ligne);
                return libre_a;
            }
            match a_l_echeance(echeance, debut, vues, self.emissions_faites) {
                Echeance::Renouvelee { echeance_ns, emissions_vues } => {
                    echeance = echeance_ns;
                    vues = emissions_vues;
                }
                Echeance::Renonce(_) => {
                    self.sans_le_jeton(coeur, ligne);
                    return echeance;
                }
            }
        }
    }
}

/// Le detenteur (coeur 0) ecrit la moitie de sa ligne, puis l'hote lui retire
/// le processeur `suspension` ns. Pendant ce temps : un contexte MASQUE
/// (coeur 1) journalise toutes les 100 ms (borne courte : il n'attend pas),
/// un appelant interruptible (coeur 2) ecrit une ligne apres l'autre.
fn detenteur_suspendu<const N: usize>(suspension: u64, panique: bool) -> Systeme<N> {
    let mut s: Systeme<N> = Systeme::neuf(panique);
    let longue = format!("DETENTEUR {}\n", "x".repeat(300));
    let (debut, fin) = longue.as_bytes().split_at(150);
    s.detenteur = Some(0);
    s.port.extend_from_slice(debut);
    let reprise = suspension;
    // Contexte masque : sa borne courte tombe toujours avant la reprise.
    let mut t = 0;
    let mut k = 0;
    while t < reprise {
        s.sans_le_jeton(1, format!("MASQUE {k}\n").as_bytes());
        t += 100 * MS;
        k += 1;
    }
    // Appelant interruptible : chaque tentative commence quand la precedente
    // finit, jusqu'a la reprise.
    let mut t = 0;
    let mut k = 0;
    while t < reprise {
        t = s.interruptible(2, format!("INTERRUPTIBLE {k}\n").as_bytes(), t, u64::MAX) + MS;
        k += 1;
    }
    // Le detenteur reprend et finit sa ligne, puis rend le jeton.
    s.port.extend_from_slice(fin);
    s.emissions_faites += 1;
    s.detenteur = None;
    // L'appelant suivant obtient le jeton : il vide les anneaux d'abord.
    s.interruptible(2, b"APRES LA REPRISE\n", reprise, reprise);
    s
}

fn lignes(port: &[u8]) -> Vec<String> {
    String::from_utf8(port.to_vec()).unwrap().lines().map(str::to_owned).collect()
}

#[test]
fn un_detenteur_suspendu_30_s_ne_fait_couper_aucune_ligne() {
    let s = detenteur_suspendu::<4096>(30_000 * MS, false);
    let l = lignes(&s.port);
    assert_eq!(s.sans_jeton, 0, "aucune ligne hors du jeton");
    assert_eq!(s.perdues, 0);
    // La ligne du detenteur est entiere, et c'est la premiere.
    assert_eq!(l[0], format!("DETENTEUR {}", "x".repeat(300)));
    // Toutes les autres sont entieres, dans l'ordre de chaque ecrivain.
    let masques: Vec<_> = l.iter().filter(|x| x.starts_with("MASQUE ")).cloned().collect();
    assert_eq!(masques.len(), 300);
    for (k, m) in masques.iter().enumerate() {
        assert_eq!(m, &format!("MASQUE {k}"));
    }
    let interruptibles: Vec<_> = l.iter().filter(|x| x.starts_with("INTERRUPTIBLE ")).cloned().collect();
    // Une seconde sans progres par tentative, plafond absolu de dix
    // secondes jamais atteint ici (aucun progres : sans-progres a 1 s).
    assert_eq!(interruptibles.len(), 30);
    for (k, m) in interruptibles.iter().enumerate() {
        assert_eq!(m, &format!("INTERRUPTIBLE {k}"));
    }
    assert_eq!(l.last().unwrap(), "APRES LA REPRISE");
    assert_eq!(l.len(), 1 + 300 + 30 + 1, "chaque ligne une fois, rien d'entrelace");
    assert_eq!(s.differees, 330);
}

#[test]
fn une_suspension_qui_deborde_l_anneau_perd_des_lignes_comptees_jamais_coupees() {
    // Anneau de 512 octets : une minute de suspension le remplit.
    let s = detenteur_suspendu::<512>(60_000 * MS, false);
    let l = lignes(&s.port);
    assert_eq!(s.sans_jeton, 0);
    assert!(s.perdues > 0, "l'anneau a deborde");
    assert_eq!(l[0], format!("DETENTEUR {}", "x".repeat(300)));
    for x in &l[1..] {
        assert!(
            x.starts_with("MASQUE ") || x.starts_with("INTERRUPTIBLE ") || x == "APRES LA REPRISE",
            "ligne coupee : {x:?}"
        );
    }
    assert_eq!(l.len() as u64, 1 + s.differees + 1);
    assert_eq!(s.differees + s.perdues, 600 + 60);
}

#[test]
fn en_panique_la_disponibilite_prime_et_c_est_compte() {
    // Le meme detenteur, mais la machine panique : les lignes sortent sans
    // le jeton, au milieu de la sienne. C'est voulu, et compte.
    let s = detenteur_suspendu::<4096>(2_000 * MS, true);
    assert_eq!(s.differees, 0);
    assert_eq!(s.sans_jeton, 20 + 2);
    let l = lignes(&s.port);
    assert!(l[0].starts_with("DETENTEUR ") && l[0].contains("MASQUE 0"), "la ligne du detenteur est coupee : {:?}", l[0]);
}

// ---------------------------------------------------------------------------
// BOUCHAUD_CONSOLE_ASYNCHRONE_V1 : l'enregistrement en morceaux et la
// vidange une ligne a la fois.

#[test]
fn un_enregistrement_en_morceaux_est_entier_ou_absent() {
    let a: Anneau<64> = Anneau::neuf();
    // Prefixe puis contenu : un seul enregistrement.
    assert!(a.range_morceaux(12, |pose| { pose(b"[pre] "); pose(b"ligne\n"); }));
    // Compte faux (le producteur pose moins ou plus) : rien de publie.
    assert!(!a.range_morceaux(10, |pose| pose(b"court\n")));
    assert!(!a.range_morceaux(4, |pose| pose(b"trop long\n")));
    let mut vu = Vec::new();
    assert!(a.vide_un(|m| vu.extend_from_slice(m)));
    assert_eq!(vu, b"[pre] ligne\n");
    assert!(!a.vide_un(|_| panic!("rien ne doit rester")));
    assert!(a.est_vide());
}

#[test]
fn vide_un_rend_une_ligne_a_la_fois_dans_l_ordre_meme_quand_l_anneau_tourne() {
    let a: Anneau<48> = Anneau::neuf();
    for tour in 0..50u32 {
        let l1 = format!("t{tour} un\n");
        let l2 = format!("t{tour} deux et plus\n");
        assert!(a.range_morceaux(l1.len(), |pose| pose(l1.as_bytes())));
        assert!(a.range_morceaux(l2.len(), |pose| { let (x, y) = l2.as_bytes().split_at(3); pose(x); pose(y); }));
        let mut premier = Vec::new();
        assert!(a.vide_un(|m| premier.extend_from_slice(m)));
        assert_eq!(premier, l1.as_bytes(), "tour {tour}");
        let mut second = Vec::new();
        assert!(a.vide_un(|m| second.extend_from_slice(m)));
        assert_eq!(second, l2.as_bytes(), "tour {tour}");
        assert_eq!(a.occupation(), 0);
    }
}

#[test]
fn un_enregistrement_qui_ne_tient_pas_est_refuse_sans_rien_abimer() {
    let a: Anneau<32> = Anneau::neuf();
    assert!(a.range_morceaux(20, |pose| pose(b"0123456789abcdefghi\n")));
    assert!(!a.range_morceaux(20, |pose| pose(b"0123456789abcdefghi\n")), "plus de place");
    assert_eq!(a.occupation(), 22);
    let mut vu = Vec::new();
    assert!(a.vide_un(|m| vu.extend_from_slice(m)));
    assert_eq!(vu, b"0123456789abcdefghi\n");
}

// BOUCHAUD_CONSOLE_ASYNCHRONE_V1 -- la console : des ecrivains de plusieurs
// coeurs, serialises par un verrou le temps d'une copie, un enregistrement par
// appel (horodatage de 8 octets, puis la ligne prefixee), un lecteur concurrent
// (le fil `console`) qui ne voit jamais qu'une ligne entiere.

use differees::{longueur_prefixee, pose_prefixee};

fn prefixee(octets: &[u8], prefixe: &[u8], debut: bool) -> (Vec<u8>, bool) {
    let mut sortie = Vec::new();
    let suivant = pose_prefixee(octets, prefixe, debut, &mut |m| sortie.extend_from_slice(m));
    assert_eq!(sortie.len(), longueur_prefixee(octets, prefixe.len(), debut), "compte et pose divergent");
    (sortie, suivant)
}

#[test]
fn un_prefixe_devant_chaque_ligne_non_vide_jamais_au_milieu() {
    let p = b"[P] ";
    assert_eq!(prefixee(b"a\nb\n", p, true), (b"[P] a\n[P] b\n".to_vec(), true));
    // Ligne vide : pas de prefixe (comme `ecris_octets`).
    assert_eq!(prefixee(b"\n\nc\n", p, true), (b"\n\n[P] c\n".to_vec(), true));
    // `print!` sans saut de ligne : la suite ne recoit pas de second prefixe.
    let (s1, d1) = prefixee(b"debut ", p, true);
    let (s2, d2) = prefixee(b"fin\n", p, d1);
    assert_eq!((s1, d1, s2, d2), (b"[P] debut ".to_vec(), false, b"fin\n".to_vec(), true));
    assert_eq!(prefixee(b"", p, true), (Vec::new(), true));
}

#[test]
fn console_plusieurs_ecrivains_un_lecteur_lignes_entieres_et_ordre_par_ecrivain() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    const ECRIVAINS: usize = 4;
    const LIGNES: usize = 3000;
    let anneau: Arc<Anneau<4096>> = Arc::new(Anneau::neuf());
    let verrou = Arc::new(AtomicUsize::new(0));
    let fini = Arc::new(AtomicBool::new(false));
    // Le lecteur n'attaque l'anneau qu'une fois celui-ci trouve plein par un
    // ecrivain : la contre-pression est exercee a coup sur, quel que soit
    // l'ordonnancement des fils de l'hote (un lecteur trop rapide la faisait
    // parfois manquer -- run hote du 28/10, `pleines == 0`).
    let plein_vu = Arc::new(AtomicBool::new(false));
    let prefixe = b"\x1b[90m[12:00:00]\x1b[0m ";
    let mut fils = Vec::new();
    for e in 0..ECRIVAINS {
        let (anneau, verrou, plein_vu) = (anneau.clone(), verrou.clone(), plein_vu.clone());
        fils.push(std::thread::spawn(move || {
            let mut pleines = 0u64;
            for n in 0..LIGNES {
                let ligne = format!("E{e} n={n} {}\n", "x".repeat((n * 7 + e) % 90));
                loop {
                    while verrou.compare_exchange_weak(0, e + 1, Ordering::Acquire, Ordering::Relaxed).is_err() {
                        std::hint::spin_loop();
                    }
                    let total = 8 + longueur_prefixee(ligne.as_bytes(), prefixe.len(), true);
                    let fait = anneau.range_morceaux(total, |pose| {
                        pose(&(n as u64).to_le_bytes());
                        pose_prefixee(ligne.as_bytes(), prefixe, true, pose);
                    });
                    verrou.store(0, Ordering::Release);
                    if fait {
                        break;
                    }
                    pleines += 1;
                    plein_vu.store(true, Ordering::Release);
                    std::thread::yield_now();
                }
            }
            pleines
        }));
    }
    let lecteur = {
        let (anneau, fini, plein_vu) = (anneau.clone(), fini.clone(), plein_vu.clone());
        std::thread::spawn(move || {
            while !plein_vu.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            let mut vues = vec![0usize; ECRIVAINS];
            let mut total = 0usize;
            loop {
                let mut rec = Vec::new();
                if anneau.vide_un(|m| rec.extend_from_slice(m)) {
                    let (horo, ligne) = rec.split_at(8);
                    let n = u64::from_le_bytes(horo.try_into().unwrap()) as usize;
                    let texte = std::str::from_utf8(ligne).expect("ligne entiere");
                    assert!(texte.starts_with("\x1b[90m[12:00:00]\x1b[0m E") && texte.ends_with('\n')
                            && texte.matches('\n').count() == 1, "ligne abimee : {texte:?}");
                    let e: usize = texte[prefixe.len() + 1..prefixe.len() + 2].parse().unwrap();
                    assert!(texte.contains(&format!("E{e} n={n} ")), "horodatage d'une autre ligne : {texte:?}");
                    assert_eq!(n, vues[e], "ordre de l'ecrivain {e} rompu");
                    vues[e] += 1;
                    total += 1;
                } else if fini.load(Ordering::Acquire) && anneau.est_vide() {
                    break;
                }
            }
            (vues, total)
        })
    };
    let pleines: u64 = fils.into_iter().map(|f| f.join().unwrap()).sum();
    fini.store(true, Ordering::Release);
    let (vues, total) = lecteur.join().unwrap();
    assert_eq!(total, ECRIVAINS * LIGNES);
    assert!(vues.iter().all(|&v| v == LIGNES));
    // L'anneau de 4 Kio a deborde : la contre-pression a ete exercee.
    assert!(pleines > 0, "l'anneau n'a jamais ete plein : le test n'exerce pas la contre-pression");
}

#[test]
fn un_enregistrement_se_lit_en_plusieurs_fois_meme_a_cheval_sur_la_fin() {
    // Plusieurs lecteurs successifs reprennent ou le precedent s'est arrete ;
    // la tete n'avance qu'au retrait. L'anneau tourne : les enregistrements
    // finissent par chevaucher la fin du tampon.
    let a: Anneau<64> = Anneau::neuf();
    for tour in 0..40usize {
        let ligne = format!("tour {tour} {}\n", "z".repeat(tour % 23));
        assert!(a.range(ligne.as_bytes()));
        let (t, n) = a.premier().expect("un enregistrement");
        assert_eq!(n, ligne.len());
        let mut lu = Vec::new();
        let mut de = 0;
        while de < n {
            let jusqu = (de + 5).min(n);
            a.lis(t, de, jusqu, |m| lu.extend_from_slice(m));
            de = jusqu;
            // Rien n'est retire tant que l'enregistrement n'est pas fini.
            assert_eq!(a.premier(), Some((t, n)));
        }
        assert_eq!(lu, ligne.as_bytes(), "tour {tour}");
        a.retire(t, n);
        assert!(a.premier().is_none() && a.est_vide());
    }
}

/// BOUCHAUD_CONSOLE_ORDRE_DIFFEREES_V1 : `ecrits` et `lus` ne reculent jamais,
/// et un repere pris a `ecrits()` n'est atteint par `lus()` qu'une fois sortis
/// tous les enregistrements ranges avant lui -- jamais pendant la lecture
/// morceau par morceau du dernier d'entre eux.
#[test]
fn repere_atteint_quand_tout_ce_qui_precede_est_sorti() {
    let a: Anneau<256> = Anneau::neuf();
    assert_eq!((a.ecrits(), a.lus()), (0, 0));
    let mut sortis = Vec::new();
    let mut ranges = 0usize;
    for tour in 0..200usize {
        let ligne = format!("ligne {tour} {}\n", "x".repeat(tour % 17));
        while !a.range(ligne.as_bytes()) {
            let (t, n) = a.premier().expect("anneau plein donc non vide");
            sortis.push(t);
            a.retire(t, n);
        }
        ranges += 1;
        let repere = a.ecrits();
        assert!(repere >= a.lus());
        // Tout ce qui precede le repere sort, un enregistrement a la fois ;
        // le repere n'est atteint qu'apres le dernier, en entier.
        while let Some((t, n)) = a.premier() {
            assert!(a.lus() < repere, "repere atteint avant la fin de ce qui le precede");
            a.lis(t, 0, n / 2, |_| {});
            assert!(a.lus() < repere, "repere atteint au milieu d'un enregistrement");
            a.retire(t, n);
            sortis.push(t);
        }
        assert_eq!(a.lus(), repere);
        assert_eq!(a.ecrits(), repere);
    }
    assert_eq!(sortis.len(), ranges);
    assert!(sortis.windows(2).all(|w| w[0] < w[1]), "positions de lecture qui reculent");
}

/// BOUCHAUD_CONSOLE_ORDRE_DIFFEREES_V2 : os-primitives 63b519a8 (KVM SMP4,
/// jeton tenu 12 s, console repliee) -- « ecrivain 1, ligne 0052 apres 0053 ».
/// Console repliee, la position d'ecriture de l'anneau ne bouge plus : toutes
/// les lignes differees portent le meme repere. L'ecrivain a differe sa ligne
/// 52 sur le coeur 2, puis, apres avoir change de coeur, sa ligne 53 sur le
/// coeur 1. Departager par l'ordre des coeurs sortait 53 d'abord.
#[test]
fn a_repere_egal_l_ordre_d_arrivee_decide_jamais_l_ordre_des_coeurs() {
    use differees::plus_ancienne_mure;
    // (coeur, repere, numero) : 52 arrivee la premiere (numero 7) sur le
    // coeur 2, 53 ensuite (numero 8) sur le coeur 1.
    let tetes = [(1usize, 4096u64, 8u64), (2, 4096, 7)];
    assert_eq!(plus_ancienne_mure(tetes.iter().copied(), 4096), Some(2), "53 doublerait 52");
    // Sans numero (l'ancien format : egalite parfaite), le premier coeur
    // gagnait -- c'est l'inversion observee.
    let sans_numero = [(1usize, 4096u64, 0u64), (2, 4096, 0)];
    assert_eq!(plus_ancienne_mure(sans_numero.iter().copied(), 4096), Some(1));
}

#[test]
fn le_repere_prime_sur_le_numero_et_une_ligne_non_mure_attend() {
    use differees::plus_ancienne_mure;
    // Repere plus petit : sort d'abord, meme arrivee apres.
    assert_eq!(plus_ancienne_mure([(0usize, 100u64, 9u64), (1, 200, 1)].into_iter(), 500), Some(0));
    // Repere pas encore atteint par la lecture : jamais.
    assert_eq!(plus_ancienne_mure([(0usize, 600u64, 1u64)].into_iter(), 500), None);
    assert_eq!(plus_ancienne_mure([(0usize, 600u64, 1u64), (3, 500, 2)].into_iter(), 500), Some(3));
    assert_eq!(plus_ancienne_mure(core::iter::empty(), 0), None);
}

/// Propriete : un ecrivain differe ses lignes une a une, en changeant de
/// coeur au hasard, pendant que la position d'ecriture avance ou reste figee ;
/// le lecteur sort toujours la plus ancienne mure. L'ordre de chaque
/// ecrivain est garde.
#[test]
fn property_un_ecrivain_qui_change_de_coeur_garde_son_ordre() {
    use differees::plus_ancienne_mure;
    use std::collections::VecDeque;
    let mut x: u64 = std::env::var("BOUCHAUD_PROP_SEED")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(|s| s.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
        .unwrap_or(0xD1FF_E2EE);
    let mut alea = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for _ in 0..2_000 {
        let coeurs = 1 + (alea() % 8) as usize;
        let mut files: Vec<VecDeque<(u64, u64, (usize, u32))>> = vec![VecDeque::new(); coeurs];
        let mut ecrits = 0u64;
        let mut lus = 0u64;
        let mut numero = 0u64;
        let mut suivant = [0u32; 3];
        let mut sortis: Vec<(usize, u32)> = Vec::new();
        for _ in 0..60 {
            match alea() % 4 {
                // Une ligne differee par l'un des trois ecrivains, sur un coeur
                // au hasard ; l'anneau avance ou reste fige (console repliee).
                0 | 1 => {
                    let e = (alea() % 3) as usize;
                    let c = (alea() % coeurs as u64) as usize;
                    files[c].push_back((ecrits, numero, (e, suivant[e])));
                    numero += 1;
                    suivant[e] += 1;
                    if alea() % 2 == 0 {
                        ecrits += 1 + alea() % 50;
                    }
                }
                // La lecture de l'anneau avance.
                2 => lus = (lus + alea() % 80).min(ecrits),
                // Le lecteur sort ce qui est mur, la plus ancienne d'abord.
                _ => {
                    while let Some(c) = plus_ancienne_mure(
                        files.iter().enumerate().filter_map(|(c, f)| f.front().map(|&(r, n, _)| (c, r, n))),
                        lus,
                    ) {
                        sortis.push(files[c].pop_front().unwrap().2);
                    }
                }
            }
        }
        lus = ecrits;
        while let Some(c) = plus_ancienne_mure(
            files.iter().enumerate().filter_map(|(c, f)| f.front().map(|&(r, n, _)| (c, r, n))),
            lus,
        ) {
            sortis.push(files[c].pop_front().unwrap().2);
        }
        assert!(files.iter().all(|f| f.is_empty()));
        for e in 0..3 {
            let ordre: Vec<u32> = sortis.iter().filter(|(w, _)| *w == e).map(|(_, n)| *n).collect();
            assert!(ordre.windows(2).all(|w| w[0] < w[1]), "ecrivain {e} : {ordre:?}");
        }
    }
}
