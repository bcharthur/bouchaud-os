/// Profondeur BKL du CPU courant.
///
/// Sert aux POST-CONDITIONS des primitives bloquantes. Une primitive qui rend
/// la main doit rendre au verrou exactement la profondeur qu'elle a trouvee ;
/// sans ce controle, une profondeur perdue ne se manifeste qu'au Drop d'un
/// garde quelconque, beaucoup plus tard, sous la forme anonyme
/// « release sans acquisition ». La victime n'est alors pas le coupable, et
/// c'est ce qui rendait ce panic si difficile a attribuer.
pub fn profondeur_locale() -> usize {
    let _irq = LocalIrqGuard::acquire();
    let cpu = cpu();
    depth_load(cpu, Ordering::Relaxed)
}

pub fn held_by_current_cpu() -> bool {
    let _irq = LocalIrqGuard::acquire();
    let cpu = cpu();
    let etat = etat_charge(Ordering::Acquire);
    etat.owner == token(cpu) && etat.depth > 0
}
