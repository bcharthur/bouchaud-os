// Instance globale et fonctions de compatibilité.

pub static INTERFACE: Reveil = Reveil::new();

#[inline]
pub fn signale_interface(source: Source) {
    INTERFACE.signale(source);
}

#[inline]
pub fn signale_interface_irq(source: Source) {
    INTERFACE.signale_irq(source);
}

#[inline]
pub fn flush_interface_irq() -> usize {
    INTERFACE.flush_irq()
}
