//! Decisions pures sur les vecteurs d'interruption imprevus.
//!
//! Ce module ne touche NI le materiel NI l'etat du noyau : il ne fait que
//! decider. C'est ce qui le rend verifiable sur l'hote, sans lever une vraie
//! exception x86 (voir `tools/platform/test_politique_vecteurs.rs`).

/// Table de descripteurs designee par un code d'erreur de selecteur.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Table {
    Gdt,
    Idt,
    Ldt,
}

/// Code d'erreur des exceptions a selecteur (#TS, #NP, #SS, #GP).
///
/// Disposition Intel SDM Vol.3 6.13 / AMD Vol.2 8.4.2 :
///
/// ```text
///   bit 0      EXT  l'evenement est EXTERNE au programme (IRQ, exception
///                   anterieure) plutot qu'un `INT n` logiciel
///   bits 1..2  TBL  00 = GDT, 01 = IDT, 10 = LDT, 11 = IDT
///   bits 3..15 IDX  index dans cette table
///   bits 16..  reserves
/// ```
///
/// Quand `table == Idt`, l'index EST le numero de vecteur : c'est le seul
/// mecanisme du processeur qui NOMME la porte manquante.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CodeSelecteur {
    pub externe: bool,
    pub table: Table,
    pub index: u16,
}

/// Decode un code d'erreur de selecteur.
///
/// `11` vaut `Idt` comme `01` : le bit TI n'a pas de sens quand la table est
/// l'IDT, et le processeur peut le laisser a un. C'est aussi ce que fait la
/// crate `x86_64` (`SelectorErrorCode::descriptor_table`).
pub fn decode_code_selecteur(code: u64) -> CodeSelecteur {
    let table = match (code >> 1) & 0b11 {
        0b00 => Table::Gdt,
        0b10 => Table::Ldt,
        _ => Table::Idt,
    };
    CodeSelecteur {
        externe: code & 1 != 0,
        table,
        index: ((code >> 3) & 0x1FFF) as u16,
    }
}

/// Le vecteur dont la porte IDT est absente, s'il s'agit bien de cela.
///
/// `None` quand le code ne designe pas l'IDT, ou quand l'index depasse les
/// 256 vecteurs : on ne fabrique pas un numero de vecteur qui n'existe pas.
pub fn vecteur_idt_absent(code: u64) -> Option<u8> {
    let decode = decode_code_selecteur(code);
    if decode.table != Table::Idt || decode.index > 0xFF {
        return None;
    }
    Some(decode.index as u8)
}

// ---------------------------------------------------------------------------
// Fins de traitement du 8259
// ---------------------------------------------------------------------------

/// Ce qu'un vecteur 0x27 / 0x2F impose comme fin d'interruption.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FinInterruption {
    /// Aucun EOI. Le controleur n'a rien mis en service : lui en envoyer un
    /// acquitterait l'interruption de quelqu'un d'autre.
    Aucune,
    /// EOI au maitre seulement.
    Maitre,
    /// EOI a l'esclave PUIS au maitre : l'esclave passe par la cascade.
    EsclavePuisMaitre,
}

/// IRQ7 : vraie interruption, ou parasite du maitre ?
///
/// Le 8259 leve IRQ7 quand une requete disparait avant l'acquittement --
/// une impulsion trop courte, un front parasite. Rien n'est alors « en
/// service » : le bit 7 de l'ISR du maitre est a zero, et **un EOI
/// acquitterait a tort l'interruption reellement en cours**.
pub fn fin_irq7(isr_maitre: u8) -> FinInterruption {
    if isr_maitre & (1 << 7) != 0 {
        FinInterruption::Maitre
    } else {
        FinInterruption::Aucune
    }
}

/// IRQ15 : vraie interruption, ou parasite de l'esclave ?
///
/// Meme mecanisme, un etage plus bas -- et c'est la que la cascade compte.
/// Le maitre, lui, a bel et bien mis IRQ2 en service pour relayer l'esclave :
/// il faut donc l'acquitter meme quand l'esclave n'a rien a acquitter. Ne
/// rien envoyer du tout bloquerait toutes les interruptions de l'esclave.
pub fn fin_irq15(isr_esclave: u8) -> FinInterruption {
    if isr_esclave & (1 << 7) != 0 {
        FinInterruption::EsclavePuisMaitre
    } else {
        FinInterruption::Maitre
    }
}

// ---------------------------------------------------------------------------
// Contrat du timer
// ---------------------------------------------------------------------------

/// IRQ0 a-t-elle le droit d'etre autre chose qu'une source de temps ?
///
/// # Le defaut que ce predicat corrige
///
/// Le handler ne testait que `bootstrap_in_progress()`. Or ce drapeau tombe
/// a la sortie de `init_probe()`, ALORS QUE `scheduler_enabled` est encore
/// faux et qu'aucune tache n'est installee sur le BSP. La toute premiere
/// IRQ0 apres la restauration de l'IF entrait donc dans le chemin complet --
/// reveils, watchdog, comptabilite de tache, preemption -- dans un etat ou
/// `CURRENT` n'existe pas.
///
/// Deux modes, et rien entre les deux :
///
/// * **BOOT** : tick, comptabilite atomique, EOI, retour. Rien qui suppose
///   une tache courante.
/// * **RUNTIME** : reveils, watchdog, ordonnancement.
///
/// Le passage de l'un a l'autre est `smp::enable_scheduler()`, et lui seul.
pub fn timer_runtime_pret(bootstrap_en_cours: bool, scheduler_actif: bool) -> bool {
    scheduler_actif && !bootstrap_en_cours
}
