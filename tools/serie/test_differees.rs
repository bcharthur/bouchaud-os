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
