//! Global Descriptor Table (GDT) + Task State Segment (TSS).
//!
//! La GDT decrit les segments utilises par le CPU en long mode. Son contenu est
//! contraint par l'instruction `syscall`/`sysret`, qui ne prend pas de selecteur
//! en operande mais deduit tout du MSR `IA32_STAR` :
//!
//! ```text
//!   syscall : CS = STAR[47:32]        SS = STAR[47:32] + 8
//!   sysret  : CS = STAR[63:48] + 16   SS = STAR[63:48] + 8   (RPL force a 3)
//! ```
//!
//! D'ou l'ordre **impose** des entrees :
//!
//! ```text
//!   0x00 null
//!   0x08 code noyau   (ring 0)   <- STAR[47:32]
//!   0x10 donnees noyau(ring 0)
//!   0x18 donnees user (ring 3)   <- STAR[63:48] + 8
//!   0x20 code user    (ring 3)   <- STAR[63:48] + 16
//!   0x28 TSS (descripteur systeme sur 16 octets)
//! ```
//!
//! Le TSS fournit deux choses indispensables au ring 3 :
//!   - `privilege_stack_table[0]` (RSP0) : la pile sur laquelle le CPU bascule
//!     quand une interruption survient pendant l'execution d'un processus
//!     utilisateur. Elle est reprogrammee a chaque changement de tache
//!     (cf. `kernel::task`), sinon deux taches se marcheraient dessus.
//!   - une pile IST dediee a la double faute, qui evite la triple faute
//!     (reboot) meme si la pile noyau est corrompue.

use core::ptr::addr_of;
use x86_64::instructions::segmentation::{Segment, CS, DS, ES, SS};
use x86_64::instructions::tables::load_tss;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;
use x86_64::PrivilegeLevel;
use x86_64::VirtAddr;
use crate::kernel::dmesg;

/// Index de la pile dediee au gestionnaire de double faute.
pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

const STACK_SIZE: usize = 4096 * 5;
static mut DF_STACK: [u8; STACK_SIZE] = [0; STACK_SIZE];

/// Pile noyau par defaut pour les interruptions venues du ring 3, utilisee tant
/// qu'aucune tache utilisateur n'a installe la sienne.
static mut RING0_STACK: [u8; STACK_SIZE] = [0; STACK_SIZE];

static mut TSS: TaskStateSegment = TaskStateSegment::new();
static mut GDT: GlobalDescriptorTable = GlobalDescriptorTable::new();

/// Selecteurs de segments, figes par [`init`].
#[derive(Clone, Copy)]
pub struct Selectors {
    pub kernel_code: SegmentSelector,
    pub kernel_data: SegmentSelector,
    pub user_data: SegmentSelector,
    pub user_code: SegmentSelector,
    pub tss: SegmentSelector,
}

static mut SELECTORS: Option<Selectors> = None;
static mut READY: bool = false;

/// Selecteurs charges (panique si appele avant `init`).
pub fn selectors() -> Selectors {
    unsafe { SELECTORS.expect("gdt: selecteurs non initialises") }
}

/// Etat courant de la GDT, expose aux commandes systeme.
pub fn state() -> &'static str {
    if unsafe { READY } {
        "initialisee (GDT ring0+ring3, TSS avec RSP0, IST double faute)"
    } else {
        "non chargee"
    }
}

/// Construit et charge la GDT et le TSS.
pub fn init() {
    unsafe {
        // Pile IST pour la double faute.
        let stack_start = VirtAddr::from_ptr(addr_of!(DF_STACK));
        TSS.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = stack_start + STACK_SIZE as u64;

        // RSP0 : pile noyau utilisee lors d'une entree depuis le ring 3.
        let ring0 = VirtAddr::from_ptr(addr_of!(RING0_STACK));
        TSS.privilege_stack_table[0] = ring0 + STACK_SIZE as u64;

        let tss_ref: &'static TaskStateSegment = &*addr_of!(TSS);

        // L'ordre ci-dessous est impose par syscall/sysret (voir l'entete).
        let kernel_code = GDT.add_entry(Descriptor::kernel_code_segment());
        let kernel_data = GDT.add_entry(Descriptor::kernel_data_segment());
        let user_data = GDT.add_entry(Descriptor::user_data_segment());
        let user_code = GDT.add_entry(Descriptor::user_code_segment());
        let tss = GDT.add_entry(Descriptor::tss_segment(tss_ref));

        GDT.load();
        CS::set_reg(kernel_code);
        DS::set_reg(kernel_data);
        ES::set_reg(kernel_data);
        SS::set_reg(kernel_data);
        load_tss(tss);

        SELECTORS = Some(Selectors {
            kernel_code,
            kernel_data,
            // Les descripteurs utilisateur sont accedes avec RPL 3.
            user_data: SegmentSelector::new(user_data.index(), PrivilegeLevel::Ring3),
            user_code: SegmentSelector::new(user_code.index(), PrivilegeLevel::Ring3),
            tss,
        });
        READY = true;
    }
    dmesg::log("gdt: GDT chargee (code/donnees ring0 + ring3) + TSS (RSP0, IST)");
}

/// Reprogramme RSP0 : pile noyau sur laquelle le CPU basculera a la prochaine
/// interruption venue du ring 3. Appele a chaque changement de tache.
pub fn set_kernel_stack(top: u64) {
    unsafe {
        TSS.privilege_stack_table[0] = VirtAddr::new(top);
    }
}

/// Valeur courante de RSP0 (diagnostic).
pub fn kernel_stack() -> u64 {
    unsafe { TSS.privilege_stack_table[0].as_u64() }
}
