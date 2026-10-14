//! Achevement des commandes virtio-gpu par interruption
//! (BOUCHAUD_VIRTIO_GPU_IRQ_V1, P10+.1).
//!
//! L'attente est rejouee contre un monde simule : une horloge, un
//! peripherique qui repond a un instant donne, et des interruptions qu'on
//! peut retarder, perdre ou envoyer a tort. Chaque cas verifie qu'une
//! reponse n'est jamais manquee, jamais inventee, et que l'attente finit a
//! son echeance.

#[path = "../../src/drivers/display/virtio_gpu_irq.rs"]
#[allow(dead_code)]
mod irq;

use irq::{attend, emplacement_table, message, progres, strategie, Bilan, Dormeur, Reveil, Strategie};

/// Un monde simule. Le temps n'avance que par `maintenant` (pas de rotation
/// reelle) et par les sommeils.
struct Monde {
    t: u64,
    /// Instant ou la reponse est posee dans l'anneau (None : jamais).
    reponse_a: Option<u64>,
    /// Instant ou l'interruption de la reponse arrive (None : perdue).
    irq_a: Option<u64>,
    /// Interruptions parasites (sans reponse), aux instants donnes.
    parasites: Vec<u64>,
    /// Pas de l'horloge par lecture (rotation).
    pas: u64,
    sommeils: Vec<(u64, u64, Reveil)>,
}

impl Monde {
    fn neuf(reponse_a: Option<u64>, irq_a: Option<u64>) -> Self {
        Monde { t: 0, reponse_a, irq_a, parasites: Vec::new(), pas: 1_000, sommeils: Vec::new() }
    }
    fn pret(&self) -> bool {
        self.reponse_a.is_some_and(|r| self.t >= r)
    }
}

impl Dormeur for Monde {
    fn maintenant(&mut self) -> u64 {
        self.t += self.pas;
        self.t
    }
    fn dors(&mut self, jusqu_a: u64, pret: &mut dyn FnMut() -> bool) -> Reveil {
        // Le ticket est pris ici ; la relecture qui suit voit tout ce qui est
        // arrive avant.
        if pret() {
            return Reveil::DejaPret;
        }
        let debut = self.t;
        // La prochaine interruption apres maintenant, s'il y en a une avant
        // la borne.
        let mut prochaine: Option<u64> = None;
        for &p in self.parasites.iter().chain(self.irq_a.iter()) {
            if p > debut && p <= jusqu_a {
                prochaine = Some(prochaine.map_or(p, |q: u64| q.min(p)));
            }
        }
        let r = match prochaine {
            Some(p) => {
                self.t = p;
                Reveil::Interruption
            }
            None => {
                self.t = jusqu_a.max(debut);
                Reveil::Borne
            }
        };
        self.sommeils.push((debut, self.t, r));
        r
    }
}

const MS: u64 = 1_000_000;
const US: u64 = 1_000;

fn joue(m: &mut Monde, echeance: u64, rotation: u64, relecture: u64) -> Bilan {
    // `pret` lit le monde a travers un pointeur : le monde est aussi le
    // dormeur. Le test n'a pas d'autre fil.
    let p: *const Monde = m;
    let mut pret = move || unsafe { (*p).pret() };
    attend(&mut pret, m, echeance, rotation, relecture)
}

#[test]
fn reponse_rapide_vue_pendant_la_rotation_sans_dormir() {
    let mut m = Monde::neuf(Some(20 * US), Some(20 * US));
    let b = joue(&mut m, 1_000 * MS, 50 * US, 2 * MS);
    assert!(b.reussi && b.vu_en_rotation);
    assert_eq!(b.sommeils, 0);
    assert!(m.sommeils.is_empty());
}

#[test]
fn reponse_lente_reveille_par_l_interruption_un_seul_sommeil() {
    let mut m = Monde::neuf(Some(700 * US), Some(700 * US));
    let b = joue(&mut m, 1_000 * MS, 50 * US, 2 * MS);
    assert!(b.reussi && !b.vu_en_rotation && !b.vu_par_relecture);
    assert_eq!(b.sommeils, 1);
    assert_eq!(b.reveils_sans_progres, 0);
    assert_eq!(m.sommeils[0].2, Reveil::Interruption);
    assert_eq!(m.t, 700 * US, "l'attente finit a l'interruption, pas a la relecture");
}

#[test]
fn interruption_perdue_la_relecture_trouve_la_reponse_et_le_dit() {
    // La reponse est la a 0,7 ms, l'interruption n'arrive jamais.
    let mut m = Monde::neuf(Some(700 * US), None);
    let b = joue(&mut m, 1_000 * MS, 50 * US, 2 * MS);
    assert!(b.reussi);
    assert!(b.vu_par_relecture, "un achevement vu sans interruption doit etre compte");
    assert!(m.t <= 50 * US + 2 * MS + 10 * US, "la relecture borne le retard : {}", m.t);
}

#[test]
fn interruption_parasite_n_acheve_rien() {
    let mut m = Monde::neuf(Some(5 * MS), Some(5 * MS));
    m.parasites = vec![300 * US, 900 * US];
    let b = joue(&mut m, 1_000 * MS, 50 * US, 2 * MS);
    assert!(b.reussi);
    assert_eq!(b.reveils_sans_progres, 2);
    assert!(m.t >= 5 * MS, "aucune reponse inventee avant qu'elle n'existe");
    assert!(!b.vu_par_relecture);
}

#[test]
fn sans_reponse_l_attente_finit_a_son_echeance() {
    let mut m = Monde::neuf(None, None);
    m.parasites = vec![10 * MS];
    let b = joue(&mut m, 30 * MS, 50 * US, 2 * MS);
    assert!(!b.reussi);
    assert!(m.t >= 30 * MS && m.t <= 30 * MS + 2 * US, "fin a l'echeance : {}", m.t);
    assert_eq!(b.reveils_sans_progres, 1);
    // Chaque sommeil est borne par la relecture.
    for &(debut, fin, _) in &m.sommeils {
        assert!(fin - debut <= 2 * MS, "sommeil de {} ns", fin - debut);
    }
}

#[test]
fn reponse_arrivee_entre_la_rotation_et_le_ticket_n_est_pas_une_perte() {
    // La reponse arrive juste apres la rotation, avant le premier sommeil :
    // `dors` la voit apres le ticket et ne dort pas.
    let mut m = Monde::neuf(Some(52 * US), Some(52 * US));
    m.pas = 1_000;
    let b = joue(&mut m, 1_000 * MS, 51 * US, 2 * MS);
    assert!(b.reussi);
    assert!(!b.vu_par_relecture, "la reponse vue apres le ticket n'est pas une interruption perdue");
    assert_eq!(b.sommeils, 0);
}

#[test]
fn echeance_avant_la_fin_de_la_rotation() {
    let mut m = Monde::neuf(None, None);
    let b = joue(&mut m, 10 * US, 50 * US, 2 * MS);
    assert!(!b.reussi);
    assert_eq!(b.sommeils, 0);
}

#[test]
fn reponse_exactement_a_l_echeance_est_comptee() {
    let mut m = Monde::neuf(Some(4 * MS), None);
    let b = joue(&mut m, 4 * MS, 50 * US, 10 * MS);
    assert!(b.reussi, "une reponse presente a l'echeance n'est pas une panne");
    assert!(b.vu_par_relecture);
}

#[test]
fn strategie_ne_dort_que_si_tout_le_permet() {
    assert_eq!(strategie(true, true, true), Strategie::Dort);
    assert_eq!(strategie(false, true, true), Strategie::Scrute, "hors tache, masque ou verrou tournant");
    assert_eq!(strategie(true, false, true), Strategie::Scrute, "vecteur refuse par le peripherique");
    assert_eq!(strategie(true, true, false), Strategie::Scrute, "politique : scrutation demandee");
}

#[test]
fn message_msi_vers_un_lapic() {
    assert_eq!(message(0, 0x50), Some((0xFEE0_0000, 0, 0x50)));
    assert_eq!(message(3, 0x50), Some((0xFEE0_3000, 0, 0x50)));
    assert_eq!(message(255, 0x50), Some((0xFEEF_F000, 0, 0x50)));
    assert_eq!(message(256, 0x50), None, "x2APIC sans remappage : pas d'interruption");
    assert_eq!(message(0, 0x0E), None, "un vecteur d'exception n'est pas une interruption");
}

#[test]
fn table_msix_bir_et_decalage() {
    assert_eq!(emplacement_table(0x0000_0001), (1, 0));
    assert_eq!(emplacement_table(0x0000_2004), (4, 0x2000));
    assert_eq!(emplacement_table(0x0000_0800 | 2), (2, 0x800));
}

#[test]
fn progres_de_l_anneau() {
    assert!(!progres(7, 7));
    assert!(progres(7, 8));
    assert!(progres(0xFFFF, 0), "l'indice tourne modulo 2^16");
}
