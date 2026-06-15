//! Global Descriptor Table (GDT) + Task State Segment (TSS).
//!
//! On charge notre propre GDT (segment de code noyau) et un TSS fournissant une
//! pile dediee (IST) pour le gestionnaire de double faute : meme si la pile
//! noyau est corrompue, la double faute s'execute sur une pile saine, ce qui
//! evite la triple faute (reboot). Base indispensable au futur split
//! user/kernel.

use core::ptr::addr_of;
use x86_64::instructions::segmentation::{Segment, CS};
use x86_64::instructions::tables::load_tss;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;
use x86_64::VirtAddr;
use crate::kernel::dmesg;

/// Index de la pile dediee au gestionnaire de double faute.
pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

const STACK_SIZE: usize = 4096 * 5;
static mut DF_STACK: [u8; STACK_SIZE] = [0; STACK_SIZE];

static mut TSS: TaskStateSegment = TaskStateSegment::new();
static mut GDT: GlobalDescriptorTable = GlobalDescriptorTable::new();

struct Selectors {
    code: SegmentSelector,
    tss: SegmentSelector,
}

static mut SELECTORS: Option<Selectors> = None;
static mut READY: bool = false;

/// Etat courant de la GDT, expose aux commandes systeme.
pub fn state() -> &'static str {
    if unsafe { READY } {
        "initialisee (GDT + TSS, IST double faute)"
    } else {
        "non chargee"
    }
}

/// Construit et charge la GDT et le TSS.
pub fn init() {
    unsafe {
        // Pile IST pour la double faute.
        let stack_start = VirtAddr::from_ptr(addr_of!(DF_STACK));
        let stack_end = stack_start + STACK_SIZE as u64;
        TSS.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = stack_end;

        let tss_ref: &'static TaskStateSegment = &*addr_of!(TSS);
        let code = GDT.add_entry(Descriptor::kernel_code_segment());
        let tss = GDT.add_entry(Descriptor::tss_segment(tss_ref));

        GDT.load();
        CS::set_reg(code);
        load_tss(tss);

        SELECTORS = Some(Selectors { code, tss });
        READY = true;
    }
    dmesg::log("gdt: GDT + TSS charges (IST double faute)");
}
