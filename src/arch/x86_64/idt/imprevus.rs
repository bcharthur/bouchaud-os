// BOUCHAUD_IDT_VECTEURS_IMPREVUS_V1
//
// ===========================================================================
// UNE PORTE ABSENTE NE DOIT PAS SE LIRE « DOUBLE FAULT »
// ===========================================================================
//
// L'IDT ne peuplait que quinze vecteurs. Les 241 autres sortaient de
// `InterruptDescriptorTable::new()`, donc avec le bit Present a zero.
//
// Le processeur, face a une porte absente, leve #NP (vecteur 11) avec un code
// d'erreur qui NOMME la porte manquante. Mais `segment_not_present` n'etait pas
// installe non plus : la porte du #NP etait elle-meme absente, et la seconde
// faute pendant la livraison de la premiere donnait une #DF.
//
// D'ou la signature exacte relevee sur la machine de reference le 14 septembre,
// juste apres `SMP_BOOT_GUARD_EXIT` :
//
//     vector = 8 DOUBLE FAULT
//     code   = 0        (une #DF pousse toujours zero)
//     CR2    = 0        (aucune #PF n'est intervenue)
//     task   = <aucune>
//     « exception initiale <non entree dans un gestionnaire> »
//
// Aucune information sur le vecteur reellement demande. Ce fragment rend cette
// information observable, et rend survivables les trois parasites que tout x86
// physique produit tot ou tard.
//
// Contrainte de tout ce fichier : EARLY-BOOT SAFE. Aucune allocation, aucun
// gros verrou, aucun mutex, aucun acces a une tache supposee courante, aucun
// formatage dependant du tas. Des atomiques et le port serie, rien d'autre.

use politique_vecteurs::{fin_irq15, fin_irq7, vecteur_idt_absent, FinInterruption};

/// Vecteur du parasite du PIC maitre (IRQ7).
pub const VECTEUR_PIC_PARASITE_MAITRE: u8 = PIC_1_OFFSET + 7;
/// Vecteur du parasite du PIC esclave (IRQ15).
pub const VECTEUR_PIC_PARASITE_ESCLAVE: u8 = PIC_2_OFFSET + 7;
/// Vecteur parasite du LAPIC, tel que `enable_local_apic` le programme.
pub const VECTEUR_LAPIC_PARASITE: u8 = 0xFF;

/// Registre de service en cours du PIC (OCW3 : lire l'ISR).
const OCW3_LIRE_ISR: u8 = 0x0B;
const PORT_COMMANDE_MAITRE: u16 = 0x20;
const PORT_COMMANDE_ESCLAVE: u16 = 0xA0;
const EOI: u8 = 0x20;

static PIC_IRQ7_PARASITES: AtomicUsize = AtomicUsize::new(0);
static PIC_IRQ7_REELLES: AtomicUsize = AtomicUsize::new(0);
static PIC_IRQ15_PARASITES: AtomicUsize = AtomicUsize::new(0);
static PIC_IRQ15_REELLES: AtomicUsize = AtomicUsize::new(0);
static LAPIC_PARASITES: AtomicUsize = AtomicUsize::new(0);

/// Portes IDT absentes rencontrees, et la premiere d'entre elles.
static IDT_ABSENTES: AtomicUsize = AtomicUsize::new(0);
/// `0` = rien vu. Sinon `1 + vecteur`, pour distinguer le vecteur 0.
static IDT_ABSENTE_PREMIER_VECTEUR: AtomicUsize = AtomicUsize::new(0);
static IDT_ABSENTE_PREMIER_CODE: AtomicU64 = AtomicU64::new(0);
static IDT_ABSENTE_PREMIER_RIP: AtomicU64 = AtomicU64::new(0);
static IDT_ABSENTE_PREMIER_RSP: AtomicU64 = AtomicU64::new(0);

/// Au-dela, on cesse de croire qu'on peut survivre a la source.
///
/// Un parasite isole se traverse ; une tempete de portes absentes veut dire
/// que quelque chose emet en boucle un vecteur que personne ne sert. Continuer
/// a rendre la main ferait tourner la machine dans le vide sans jamais le dire.
const IDT_ABSENTES_MAX: usize = 64;

/// Ce que les parasites et les portes absentes ont produit.
///
/// `(irq7_parasites, irq7_reelles, irq15_parasites, irq15_reelles,
///   lapic_parasites, idt_absentes)`
pub fn compteurs_imprevus() -> (usize, usize, usize, usize, usize, usize) {
    (
        PIC_IRQ7_PARASITES.load(Ordering::Relaxed),
        PIC_IRQ7_REELLES.load(Ordering::Relaxed),
        PIC_IRQ15_PARASITES.load(Ordering::Relaxed),
        PIC_IRQ15_REELLES.load(Ordering::Relaxed),
        LAPIC_PARASITES.load(Ordering::Relaxed),
        IDT_ABSENTES.load(Ordering::Relaxed),
    )
}

/// La premiere porte IDT absente rencontree, si elle existe.
///
/// `(vecteur, code, rip, rsp)`. Lue par l'ecran de faute : une #DF qui suit un
/// #NP sur porte absente n'est pas une faute de pile, et le dire evite de
/// chercher la panne au mauvais endroit.
pub fn premiere_porte_absente() -> Option<(u8, u64, u64, u64)> {
    let marque = IDT_ABSENTE_PREMIER_VECTEUR.load(Ordering::Acquire);
    if marque == 0 {
        return None;
    }
    Some((
        (marque - 1) as u8,
        IDT_ABSENTE_PREMIER_CODE.load(Ordering::Relaxed),
        IDT_ABSENTE_PREMIER_RIP.load(Ordering::Relaxed),
        IDT_ABSENTE_PREMIER_RSP.load(Ordering::Relaxed),
    ))
}

/// Lit le registre de service en cours d'un des deux PIC.
///
/// OCW3 n'est pas latche : la commande est reemise avant chaque lecture.
unsafe fn lit_isr(port_commande: u16) -> u8 {
    ports::outb(port_commande, OCW3_LIRE_ISR);
    ports::inb(port_commande)
}

unsafe fn applique_fin(fin: FinInterruption) {
    match fin {
        FinInterruption::Aucune => {}
        FinInterruption::Maitre => ports::outb(PORT_COMMANDE_MAITRE, EOI),
        FinInterruption::EsclavePuisMaitre => {
            ports::outb(PORT_COMMANDE_ESCLAVE, EOI);
            ports::outb(PORT_COMMANDE_MAITRE, EOI);
        }
    }
}

/// IRQ7 : le parasite le plus banal d'un vrai PC, et le plus absent de QEMU.
extern "x86-interrupt" fn pic_parasite_maitre_handler(stack: InterruptStackFrame) {
    smp::note_premiere_irq(
        VECTEUR_PIC_PARASITE_MAITRE,
        stack.instruction_pointer.as_u64(),
        stack.stack_pointer.as_u64(),
    );
    let fin = unsafe { fin_irq7(lit_isr(PORT_COMMANDE_MAITRE)) };
    if fin == FinInterruption::Aucune {
        PIC_IRQ7_PARASITES.fetch_add(1, Ordering::Relaxed);
    } else {
        PIC_IRQ7_REELLES.fetch_add(1, Ordering::Relaxed);
    }
    unsafe { applique_fin(fin) };
}

/// IRQ15 : meme mecanisme, mais la cascade doit etre acquittee quand meme.
///
/// Ce vecteur portait jusqu'ici le gestionnaire ATA secondaire, qui envoyait
/// un EOI inconditionnel a l'esclave : sur un parasite, cela acquitte une
/// interruption que l'esclave n'avait pas mise en service.
extern "x86-interrupt" fn pic_parasite_esclave_handler(stack: InterruptStackFrame) {
    smp::note_premiere_irq(
        VECTEUR_PIC_PARASITE_ESCLAVE,
        stack.instruction_pointer.as_u64(),
        stack.stack_pointer.as_u64(),
    );
    let fin = unsafe { fin_irq15(lit_isr(PORT_COMMANDE_ESCLAVE)) };
    if fin == FinInterruption::Maitre {
        PIC_IRQ15_PARASITES.fetch_add(1, Ordering::Relaxed);
    } else {
        PIC_IRQ15_REELLES.fetch_add(1, Ordering::Relaxed);
        // IRQ15 REELLE : c'est l'ATA secondaire. Lire son registre de statut
        // est ce qui desarme la ligne ; sans cela le peripherique la maintient
        // et l'interruption revient immediatement.
        let _ = unsafe { ports::inb(0x177) };
    }
    unsafe { applique_fin(fin) };
}

/// Parasite du LAPIC.
///
/// Intel SDM Vol.3 11.9 : une interruption parasite ne met rien en service et
/// **n'attend aucun EOI**. En envoyer un acquitterait l'interruption reellement
/// en cours.
extern "x86-interrupt" fn lapic_parasite_handler(stack: InterruptStackFrame) {
    smp::note_premiere_irq(
        VECTEUR_LAPIC_PARASITE,
        stack.instruction_pointer.as_u64(),
        stack.stack_pointer.as_u64(),
    );
    LAPIC_PARASITES.fetch_add(1, Ordering::Relaxed);
}

/// #NP : la seule voix du processeur qui NOMME une porte absente.
///
/// Le gestionnaire ne repare rien -- il ne peut pas fabriquer une porte. Il
/// PUBLIE, puis rend la main apres avoir retire du controleur l'interruption
/// qui n'a pas pu etre livree, sans quoi elle serait represente indefiniment.
extern "x86-interrupt" fn segment_not_present_handler(stack: InterruptStackFrame, code: u64) {
    let rip = stack.instruction_pointer.as_u64();
    let rsp = stack.stack_pointer.as_u64();
    let cpu = smp::cpu_index();
    smp::note_premiere_irq(11, rip, rsp);

    let Some(vecteur) = vecteur_idt_absent(code) else {
        // Un #NP qui ne designe pas l'IDT est une vraie faute de segment :
        // elle n'a rien a faire ici et ne doit pas etre avalee.
        serial_println!(
            "BOUCHAUD_SEGMENT_ABSENT code={:#x} rip={:#x} rsp={:#x} cpu={}",
            code,
            rip,
            rsp,
            cpu,
        );
        releve_faute_fatale("SEGMENT NOT PRESENT", &stack, code);
        arret_definitif();
    };

    let vus = IDT_ABSENTES.fetch_add(1, Ordering::Relaxed) + 1;
    if IDT_ABSENTE_PREMIER_VECTEUR
        .compare_exchange(0, vecteur as usize + 1, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        IDT_ABSENTE_PREMIER_CODE.store(code, Ordering::Relaxed);
        IDT_ABSENTE_PREMIER_RIP.store(rip, Ordering::Relaxed);
        IDT_ABSENTE_PREMIER_RSP.store(rsp, Ordering::Relaxed);
    }

    // Une ligne par occurrence tant qu'elles sont rares, puis plus rien : ce
    // chemin peut se repeter mille fois par seconde, et noyer le port serie
    // ferait perdre le reste du diagnostic.
    if vus <= 8 {
        let decode = politique_vecteurs::decode_code_selecteur(code);
        serial_println!(
            "BOUCHAUD_IDT_NOT_PRESENT vector={:#04x} error={:#x} ext={} idt=1 \
index={} rip={:#x} rsp={:#x} cpu={} occurrences={}",
            vecteur,
            code,
            decode.externe as u8,
            decode.index,
            rip,
            rsp,
            cpu,
            vus,
        );
    }

    if vus >= IDT_ABSENTES_MAX {
        serial_println!(
            "BOUCHAUD_IDT_NOT_PRESENT_TEMPETE vector={:#04x} occurrences={}",
            vecteur,
            vus,
        );
        releve_faute_fatale("SEGMENT NOT PRESENT", &stack, code);
        arret_definitif();
    }

    // RENDRE LA MAIN SANS BOUCLER.
    //
    // L'interruption qui n'a pas pu etre livree est toujours en service dans
    // son controleur. Revenir sans l'acquitter la ferait represente a
    // l'infini. On acquitte donc la source designee par le vecteur, et
    // seulement elle.
    unsafe {
        if (PIC_1_OFFSET..PIC_2_OFFSET + 8).contains(&vecteur) {
            if vecteur >= PIC_2_OFFSET {
                applique_fin(FinInterruption::EsclavePuisMaitre);
            } else {
                applique_fin(FinInterruption::Maitre);
            }
        } else if vecteur >= 0x30 {
            smp::eoi_local();
        }
    }
}

/// Installe tout ce que ce fragment apporte.
fn installe_vecteurs_imprevus(idt: &mut InterruptDescriptorTable) {
    idt.segment_not_present
        .set_handler_fn(segment_not_present_handler);
    idt[VECTEUR_PIC_PARASITE_MAITRE as usize].set_handler_fn(pic_parasite_maitre_handler);
    idt[VECTEUR_PIC_PARASITE_ESCLAVE as usize].set_handler_fn(pic_parasite_esclave_handler);
    idt[VECTEUR_LAPIC_PARASITE as usize].set_handler_fn(lapic_parasite_handler);
}
