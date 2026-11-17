// Instance globale et fonctions de compatibilité.

pub static INTERFACE: Reveil = Reveil::new();

#[inline]
pub fn signale_interface(source: Source) {
    INTERFACE.signale(source);
    entree_ring3(source);
}

/// BOUCHAUD_ENTREES_RING3_V1 : une entree (souris, clavier) a reveiller aussi
/// cote compositeur ring 3, s'il est proprietaire des entrees. Atomiques
/// seulement : appele depuis les IRQ d'entree.
#[inline]
fn entree_ring3(source: Source) {
    if matches!(source, Source::Clavier | Source::Souris) {
        crate::kernel::native::entrees::irq();
    }
}

#[inline]
pub fn signale_interface_irq(source: Source) {
    INTERFACE.signale_irq(source);
    entree_ring3(source);
}

#[inline]
pub fn flush_interface_irq() -> usize {
    INTERFACE.flush_irq()
}
