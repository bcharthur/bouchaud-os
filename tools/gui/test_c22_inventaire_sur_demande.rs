//! BOUCHAUD_C22_INVENTAIRE_VMA_SUR_DEMANDE_V1 : l'inventaire `[C22:VMA]`
//! (une ligne serie par VMA, toutes les 5 s) n'est emis que sur demande, et
//! le banc memoire C22 le demande toujours -- le diagnostic reste disponible
//! la ou il sert, sans couter au navigateur ailleurs.
use std::fs;

fn lire(p: &str) -> String {
    fs::read_to_string(p).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn regle(services: &str, banc: &str) -> Result<(), String> {
    let i = services.find("\"[C22:VMA-BEGIN]").ok_or("inventaire disparu")?;
    let avant = &services[..i];
    let g = avant.rfind("inventaire_vma_demande()").ok_or("inventaire sans condition")?;
    if avant[g..].contains("\n                    }\n") {
        return Err("condition hors du bloc de l'inventaire".into());
    }
    if !services.contains("e == \"BOUCHAUD_LB_MEMORY_PROOF=1\"") {
        return Err("le banc memoire ne declenche plus l'inventaire".into());
    }
    if !services.contains("\"[C22:RSS] t={}") {
        return Err("la ligne [C22:RSS] du verdict a disparu".into());
    }
    if !banc.contains("export BOUCHAUD_LB_MEMORY_PROOF=1") {
        return Err("run_ladybird_memoire.sh n'exporte plus BOUCHAUD_LB_MEMORY_PROOF=1".into());
    }
    Ok(())
}

#[test]
fn inventaire_sur_demande_et_banc_le_demande() {
    regle(&lire("src/gui/services.rs"), &lire("tools/ci/run_ladybird_memoire.sh")).unwrap();
}

#[test]
fn temoins_negatifs() {
    let s = lire("src/gui/services.rs");
    let b = lire("tools/ci/run_ladybird_memoire.sh");
    assert!(regle(&s, &b.replace("export BOUCHAUD_LB_MEMORY_PROOF=1", "")).is_err());
    assert!(regle(&s.replace("e == \"BOUCHAUD_LB_MEMORY_PROOF=1\"", "false"), &b).is_err());
    assert!(regle(&s.replacen("&& inventaire_vma_demande()", "", 1).replacen("fn inventaire_vma_demande", "fn x", 1), &b).is_err());
}
