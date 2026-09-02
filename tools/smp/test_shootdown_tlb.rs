//! Modele deterministe de la reemission et de l'echec fail-closed du shootdown.

#[derive(Debug, PartialEq, Eq)]
struct Echec {
    manquants: u64,
    relances: u32,
}

fn attend_acks(
    cibles: u64,
    essais_max: u32,
    mut envoie: impl FnMut(u64, u32) -> u64,
) -> Result<u32, Echec> {
    let mut acks = 0u64;
    for essai in 0..=essais_max {
        let manquants = cibles & !acks;
        if manquants == 0 { return Ok(essai); }
        if essai == essais_max {
            return Err(Echec { manquants, relances: essai });
        }
        acks |= envoie(manquants, essai);
    }
    unreachable!()
}

#[test]
fn un_premier_ipi_perdu_est_reemis_seulement_au_cpu_manquant() {
    let cibles = 0b1110;
    let mut vus = Vec::new();
    let relances = attend_acks(cibles, 4, |manquants, essai| {
        vus.push(manquants);
        if essai == 0 { 0b1010 } else { manquants }
    }).unwrap();
    assert_eq!(relances, 2);
    assert_eq!(vus, vec![0b1110, 0b0100]);
}

#[test]
fn un_cpu_definitivement_muet_termine_en_echec_fail_closed() {
    let echec = attend_acks(0b0110, 3, |manquants, _| manquants & !0b0100)
        .expect_err("continuer serait accepter un TLB perime");
    assert_eq!(echec, Echec { manquants: 0b0100, relances: 3 });
}
