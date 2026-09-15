#[path = "../../src/arch/x86_64/idt/politique_vecteurs.rs"]
mod politique;

use politique::{
    decode_code_selecteur, fin_irq15, fin_irq7, timer_runtime_pret, vecteur_idt_absent,
    FinInterruption, Table,
};

// ---------------------------------------------------------------------------
// Decodage du code d'erreur #NP
// ---------------------------------------------------------------------------

#[test]
fn une_porte_idt_absente_nomme_son_vecteur() {
    // Interruption EXTERNE sur une porte IDT absente : EXT=1, IDT=1,
    // index = vecteur. Pour IRQ7 (vecteur 0x27) le processeur pousse donc
    // 0x27 * 8 + 3 = 0x13B.
    let code = 0x27u64 * 8 + 0b011;
    let decode = decode_code_selecteur(code);
    assert!(decode.externe);
    assert_eq!(decode.table, Table::Idt);
    assert_eq!(decode.index, 0x27);
    assert_eq!(vecteur_idt_absent(code), Some(0x27));
}

#[test]
fn un_int_logiciel_n_est_pas_externe() {
    // `INT 0xFF` sur une porte absente : EXT=0, IDT=1.
    let code = 0xFFu64 * 8 + 0b010;
    let decode = decode_code_selecteur(code);
    assert!(!decode.externe);
    assert_eq!(decode.table, Table::Idt);
    assert_eq!(vecteur_idt_absent(code), Some(0xFF));
}

#[test]
fn le_bit_ti_ne_change_rien_quand_la_table_est_l_idt() {
    // bits 1..2 = 0b11 designent aussi l'IDT : le bit TI n'a pas de sens la.
    let code = 0x20u64 * 8 + 0b111;
    assert_eq!(decode_code_selecteur(code).table, Table::Idt);
    assert_eq!(vecteur_idt_absent(code), Some(0x20));
}

#[test]
fn un_code_gdt_ou_ldt_n_est_pas_une_porte_absente() {
    let gdt = 0x10u64 * 8; // EXT=0, TBL=00
    assert_eq!(decode_code_selecteur(gdt).table, Table::Gdt);
    assert_eq!(vecteur_idt_absent(gdt), None);

    let ldt = 0x10u64 * 8 + 0b100; // TBL=10
    assert_eq!(decode_code_selecteur(ldt).table, Table::Ldt);
    assert_eq!(vecteur_idt_absent(ldt), None);
}

#[test]
fn un_index_au_dela_de_255_n_est_pas_un_vecteur() {
    // L'IDT n'a que 256 portes. Un index plus grand ne designe pas un vecteur,
    // et en fabriquer un par troncature inventerait une information.
    let code = 300u64 * 8 + 0b011;
    assert_eq!(decode_code_selecteur(code).index, 300);
    assert_eq!(vecteur_idt_absent(code), None);
}

#[test]
fn le_vecteur_zero_reste_distinguable() {
    let code = 0b011; // vecteur 0, EXT=1, IDT=1
    assert_eq!(vecteur_idt_absent(code), Some(0));
}

// ---------------------------------------------------------------------------
// Fins d'interruption du 8259
// ---------------------------------------------------------------------------

#[test]
fn irq7_reelle_est_acquittee_au_maitre() {
    // Bit 7 de l'ISR maitre a un : IRQ7 est reellement en service.
    assert_eq!(fin_irq7(0b1000_0000), FinInterruption::Maitre);
    assert_eq!(fin_irq7(0b1000_0001), FinInterruption::Maitre);
}

#[test]
fn irq7_parasite_n_est_pas_acquittee() {
    // Rien en service : un EOI acquitterait l'interruption de quelqu'un
    // d'autre. C'est la regle que le 8259 impose et que le gestionnaire
    // generique violait.
    assert_eq!(fin_irq7(0), FinInterruption::Aucune);
    assert_eq!(fin_irq7(0b0111_1111), FinInterruption::Aucune);
}

#[test]
fn irq15_reelle_acquitte_l_esclave_puis_le_maitre() {
    assert_eq!(fin_irq15(0b1000_0000), FinInterruption::EsclavePuisMaitre);
}

#[test]
fn irq15_parasite_acquitte_quand_meme_la_cascade() {
    // L'esclave n'a rien en service, mais le maitre a bel et bien mis IRQ2 en
    // service pour le relayer. Ne rien envoyer bloquerait tout l'esclave.
    assert_eq!(fin_irq15(0), FinInterruption::Maitre);
    assert_eq!(fin_irq15(0b0111_1111), FinInterruption::Maitre);
}

// ---------------------------------------------------------------------------
// Contrat du timer
// ---------------------------------------------------------------------------

#[test]
fn le_timer_reste_horloge_seule_tant_que_le_scheduler_dort() {
    // LE CAS QUI A TUE LA MACHINE : le garde d'amorcage est tombe, mais le
    // scheduler n'est pas active et aucune tache n'existe.
    assert!(!timer_runtime_pret(false, false));
}

#[test]
fn le_timer_reste_horloge_seule_pendant_l_amorcage() {
    assert!(!timer_runtime_pret(true, false));
    assert!(!timer_runtime_pret(true, true));
}

#[test]
fn le_timer_devient_complet_apres_activation_du_scheduler() {
    assert!(timer_runtime_pret(false, true));
}
