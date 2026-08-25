//! Contrats indépendants de l'ISA.
//!
//! Les primitives de contexte, MMU, interruptions et SMP seront extraites du
//! backend x86_64 progressivement et validées à chaque étape.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Architecture {
    X86_64,
    AArch64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CpuId(pub usize);
