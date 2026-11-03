//! Preuve hote de la mise a jour A/B (BOUCHAUD_UPDATE_AB_V1).
//!
//!   rustc --edition 2021 --test tools/update/test_ab.rs -o /tmp/test_ab && /tmp/test_ab
//!
//! Le module de production est inclus tel quel. Les tests modelisent ce que la
//! machine physique ne permet pas de rejouer a volonte : une coupure
//! d'alimentation a CHAQUE octet de l'ecriture du bloc d'amorcage, et a CHAQUE
//! etape d'une mise a jour ; puis une exploration exhaustive des sequences
//! d'operations jusqu'a une profondeur donnee.

#![allow(dead_code)]

#[path = "../../src/kernel/update/ab.rs"]
mod ab;

use ab::*;
use std::collections::{HashSet, VecDeque};

const V1: [u8; 32] = [0x11; 32];
const V2: [u8; 32] = [0x22; 32];
const V3: [u8; 32] = [0x33; 32];

/// Le disque : deux copies du bloc d'amorcage.
#[derive(Clone)]
struct Disque { c: [[u8; OCTETS]; 2] }

impl Disque {
    fn neuf(e: &Enregistrement) -> Self {
        let o = encode(e);
        Disque { c: [o, o] }
    }
    fn lit(&self) -> Option<Enregistrement> { selectionne(&self.c[0], &self.c[1]) }
    /// Ecrit `e` ; si `coupure` vaut Some(k), seuls les k premiers octets
    /// atteignent le support.
    fn ecrit(&mut self, e: &Enregistrement, coupure: Option<usize>) {
        let i = copie_a_ecrire(&self.c[0], &self.c[1]);
        let o = encode(e);
        let n = coupure.unwrap_or(OCTETS).min(OCTETS);
        self.c[i][..n].copy_from_slice(&o[..n]);
    }
}

fn invariants(e: &Enregistrement) {
    // I1 : l'actif est toujours Bon (decode le refuse sinon).
    assert_eq!(e.slot(e.actif).etat, Etat::Bon, "actif non Bon : {e:?}");
    // I2 : le choix de boot ne rend jamais Vide ni Mauvais.
    let (s, apres) = e.choisit_boot();
    assert!(!matches!(e.slot(s).etat, Etat::Vide | Etat::Mauvais), "boot de {s:?} dans {e:?}");
    assert_eq!(apres.slot(apres.actif).etat, Etat::Bon);
}

#[test]
fn coupure_a_chaque_octet_rend_ancien_ou_nouveau() {
    let r1 = Enregistrement::initial(Slot::A, V1);
    let r2 = r1.commence_installation(Slot::B).unwrap();
    let r3 = r2.marque_candidat(Slot::B, V2, V2, 3).unwrap();
    for (avant, apres) in [(r1, r2), (r2, r3)] {
        let mut base = Disque::neuf(&r1);
        if avant != r1 { base.ecrit(&avant, None); }
        for k in 0..=OCTETS {
            let mut d = base.clone();
            d.ecrit(&apres, Some(k));
            let lu = d.lit().expect("aucune copie valide apres coupure");
            assert!(lu == avant || lu == apres, "melange a k={k}: {lu:?}");
            if k < OCTETS { assert_eq!(lu, avant, "nouveau visible avant la fin a k={k}"); }
            invariants(&lu);
        }
    }
}

#[test]
fn octets_quelconques_dans_la_copie_ecrite_ne_creent_pas_d_etat() {
    let r1 = Enregistrement::initial(Slot::A, V1);
    let mut base = Disque::neuf(&r1);
    let r2 = r1.commence_installation(Slot::B).unwrap();
    base.ecrit(&r2, None);
    let cible = copie_a_ecrire(&base.c[0], &base.c[1]);
    let mut graine = 0x9e3779b97f4a7c15u64;
    for _ in 0..20_000 {
        let mut d = base.clone();
        for o in d.c[cible].iter_mut() {
            graine ^= graine << 13; graine ^= graine >> 7; graine ^= graine << 17;
            *o = graine as u8;
        }
        let lu = d.lit().unwrap();
        assert_eq!(lu, r2, "des octets aleatoires ont produit un etat");
    }
}

#[test]
fn mise_a_jour_complete_puis_commit() {
    let mut d = Disque::neuf(&Enregistrement::initial(Slot::A, V1));
    let e = d.lit().unwrap().commence_installation(Slot::B).unwrap(); d.ecrit(&e, None);
    let e = d.lit().unwrap().marque_candidat(Slot::B, V2, V2, 3).unwrap(); d.ecrit(&e, None);
    let (s, e) = d.lit().unwrap().choisit_boot(); d.ecrit(&e, None);
    assert_eq!(s, Slot::B);
    let e = d.lit().unwrap().sante_ok(Slot::B); d.ecrit(&e, None);
    let lu = d.lit().unwrap();
    assert_eq!(lu.actif, Slot::B);
    assert_eq!(lu.slot(Slot::B).image, V2);
    assert_eq!(lu.slot(Slot::A).etat, Etat::Bon, "la version precedente reste disponible");
    assert_eq!(lu.choisit_boot().0, Slot::B);
    let r = lu.revient().unwrap();
    assert_eq!(r.actif, Slot::A);
}

#[test]
fn candidat_qui_plante_est_essaye_au_plus_n_fois_puis_rollback() {
    for essais in 1..=5u8 {
        let mut e = Enregistrement::initial(Slot::A, V1)
            .commence_installation(Slot::B).unwrap()
            .marque_candidat(Slot::B, V2, V2, essais).unwrap();
        let mut boots_candidat = 0;
        for _ in 0..20 {
            let (s, n) = e.choisit_boot();
            e = n; // ecrit AVANT le boot ; le boot plante, sans sante_ok
            if s == Slot::B { boots_candidat += 1; }
        }
        assert_eq!(boots_candidat, essais as usize, "I3");
        assert_eq!(e.slot(Slot::B).etat, Etat::Mauvais);
        assert_eq!(e.actif, Slot::A);
        assert_eq!(e.choisit_boot().0, Slot::A);
    }
}

#[test]
fn refus_explicites() {
    let e = Enregistrement::initial(Slot::A, V1);
    assert_eq!(e.commence_installation(Slot::A), Err(Refus::CibleActive));
    let e = e.commence_installation(Slot::B).unwrap();
    assert_eq!(e.marque_candidat(Slot::B, V2, V3, 2), Err(Refus::ImageNonVerifiee));
    assert_eq!(e.marque_candidat(Slot::B, V2, V2, 0), Err(Refus::EssaisNuls));
    let e = e.marque_candidat(Slot::B, V2, V2, 2).unwrap();
    assert_eq!(e.commence_installation(Slot::B), Err(Refus::CandidatEnCours));
    assert_eq!(e.revient(), Err(Refus::EtatInattendu));
}

#[test]
fn coupure_a_chaque_etape_et_chaque_octet_laisse_une_machine_amorcable() {
    // Une mise a jour = suite d'ecritures. Couper pendant n'importe laquelle,
    // a n'importe quel octet, puis redemarrer : l'enregistrement lu satisfait
    // les invariants, et le slot boote est Bon ou un candidat verifie.
    let etapes = |e: Enregistrement| -> Vec<Enregistrement> {
        let a = e.commence_installation(Slot::B).unwrap();
        let b = a.marque_candidat(Slot::B, V2, V2, 2).unwrap();
        let (_, c) = b.choisit_boot();
        let d = c.sante_ok(Slot::B);
        vec![a, b, c, d]
    };
    let depart = Enregistrement::initial(Slot::A, V1);
    let suite = etapes(depart);
    for coupee in 0..suite.len() {
        for k in 0..=OCTETS {
            let mut d = Disque::neuf(&depart);
            for (i, e) in suite.iter().enumerate() {
                if i < coupee { d.ecrit(e, None); }
                if i == coupee { d.ecrit(e, Some(k)); break; }
            }
            let lu = d.lit().expect("plus aucune copie valide");
            invariants(&lu);
        }
    }
}

#[test]
fn exploration_exhaustive_des_sequences() {
    // BFS sur toutes les operations depuis l'etat initial ; chaque etat
    // atteignable, encode puis decode, satisfait les invariants.
    let depart = Enregistrement::initial(Slot::A, V1);
    let mut vus: HashSet<[u8; OCTETS]> = HashSet::new();
    let mut file = VecDeque::from([(depart, 0u32)]);
    let images = [V1, V2, V3];
    while let Some((e, prof)) = file.pop_front() {
        let mut cle = encode(&e);
        cle[8..16].fill(0); // la sequence ne distingue pas deux etats
        cle[92..96].fill(0);
        if !vus.insert(cle) || prof > 9 { continue; }
        let rel = decode(&encode(&e)).expect("etat atteignable non decodable");
        assert_eq!(rel, e);
        invariants(&e);
        let mut suivants = Vec::new();
        for s in [Slot::A, Slot::B] {
            if let Ok(n) = e.commence_installation(s) { suivants.push(n); }
            for img in images {
                for essais in 1..=2 {
                    if let Ok(n) = e.marque_candidat(s, img, img, essais) { suivants.push(n); }
                }
            }
            suivants.push(e.sante_ok(s));
            suivants.push(e.sante_ko(s));
        }
        suivants.push(e.choisit_boot().1);
        let (s, n) = e.choisit_boot();
        suivants.push(n.sante_ok(s));
        if let Ok(n) = e.revient() { suivants.push(n); }
        for n in suivants {
            assert!(n.sequence >= e.sequence);
            file.push_back((n, prof + 1));
        }
    }
    assert!(vus.len() > 50, "exploration trop pauvre : {}", vus.len());
    println!("UPDATE_AB_ETATS_EXPLORES {}", vus.len());
}

#[test]
fn enregistrement_forge_avec_actif_non_bon_est_refuse() {
    // CRC valide, magic valide, mais l'actif designe un slot Vide : un
    // chargeur qui l'accepterait booterait un systeme jamais installe.
    let mut e = Enregistrement::initial(Slot::A, V1);
    e.actif = Slot::B;
    assert_eq!(decode(&encode(&e)), None);
    let bon = encode(&Enregistrement::initial(Slot::A, V1));
    assert_eq!(selectionne(&encode(&e), &bon).unwrap().actif, Slot::A);
}
