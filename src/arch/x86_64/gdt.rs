//! GDT + TSS par CPU.
//!
//! En SMP, RSP0 et la pile IST sont un etat materiel local au coeur. Partager
//! un TSS entre plusieurs CPU ferait pointer une interruption ring3 vers la
//! pile noyau de la derniere tache installee sur n'importe quel coeur. Chaque
//! CPU possede donc sa propre GDT (meme disposition de selecteurs) et son TSS.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::spin_loop;
use core::sync::atomic::{AtomicBool, Ordering};
use x86_64::instructions::segmentation::{Segment, CS, DS, ES, SS};
use x86_64::instructions::tables::load_tss;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;
use x86_64::{PrivilegeLevel, VirtAddr};

use crate::arch::x86_64::smp::MAX_CPUS;
use crate::kernel::dmesg;

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;
const STACK_SIZE: usize = 4096 * 5;

#[derive(Clone, Copy)]
pub struct Selectors {
    pub kernel_code: SegmentSelector,
    pub kernel_data: SegmentSelector,
    pub user_data: SegmentSelector,
    pub user_code: SegmentSelector,
    pub tss: SegmentSelector,
}

struct CpuGdt {
    gdt: GlobalDescriptorTable,
    selectors: Selectors,
    tss: *mut TaskStateSegment,
}

static mut CPU_GDTS: Option<Vec<CpuGdt>> = None;
static mut BASE_SELECTORS: Option<Selectors> = None;
static READY: AtomicBool = AtomicBool::new(false);
// Les AP chargent leur GDT avant que le BKL soit utilisable (GS n'est pas
// encore initialise). Ce mini-verrou ne sert qu'au bootstrap GDT/TSS.
static ACCESS: AtomicBool = AtomicBool::new(false);

struct AccessGuard;
impl Drop for AccessGuard {
    fn drop(&mut self) { ACCESS.store(false, Ordering::Release); }
}

fn access() -> AccessGuard {
    while ACCESS
        .compare_exchange_weak(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        spin_loop();
    }
    AccessGuard
}

fn build_cpu() -> CpuGdt {
    let df_stack = Box::leak(vec![0u8; STACK_SIZE].into_boxed_slice());
    let ring0_stack = Box::leak(vec![0u8; STACK_SIZE].into_boxed_slice());
    let tss_mut = Box::leak(Box::new(TaskStateSegment::new()));
    let tss_ptr = tss_mut as *mut TaskStateSegment;

    let df_start = VirtAddr::from_ptr(df_stack.as_ptr());
    tss_mut.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] =
        df_start + STACK_SIZE as u64;
    let ring0 = VirtAddr::from_ptr(ring0_stack.as_ptr());
    tss_mut.privilege_stack_table[0] = ring0 + STACK_SIZE as u64;

    // Le descripteur TSS exige une reference 'static immutable. Le TSS reste
    // pourtant modifiable via son pointeur brut pour RSP0 : c'est exactement le
    // modele de l'ancien static mut TSS, mais replique par CPU.
    let tss_ref: &'static TaskStateSegment = unsafe { &*tss_ptr };

    let mut gdt = GlobalDescriptorTable::new();
    let kernel_code = gdt.add_entry(Descriptor::kernel_code_segment());
    let kernel_data = gdt.add_entry(Descriptor::kernel_data_segment());
    let user_data_raw = gdt.add_entry(Descriptor::user_data_segment());
    let user_code_raw = gdt.add_entry(Descriptor::user_code_segment());
    let tss_sel = gdt.add_entry(Descriptor::tss_segment(tss_ref));

    let selectors = Selectors {
        kernel_code,
        kernel_data,
        user_data: SegmentSelector::new(user_data_raw.index(), PrivilegeLevel::Ring3),
        user_code: SegmentSelector::new(user_code_raw.index(), PrivilegeLevel::Ring3),
        tss: tss_sel,
    };

    CpuGdt { gdt, selectors, tss: tss_ptr }
}

fn all_unlocked() -> &'static mut Vec<CpuGdt> {
    unsafe {
        if CPU_GDTS.is_none() {
            let mut states = Vec::with_capacity(MAX_CPUS);
            for _ in 0..MAX_CPUS { states.push(build_cpu()); }
            BASE_SELECTORS = Some(states[0].selectors);
            CPU_GDTS = Some(states);
        }
        CPU_GDTS.as_mut().unwrap()
    }
}

fn load_cpu(cpu: usize) {
    let _access = access();
    let state = &mut all_unlocked()[cpu.min(MAX_CPUS - 1)];
    unsafe { state.gdt.load_unsafe(); }
    unsafe {
        CS::set_reg(state.selectors.kernel_code);
        DS::set_reg(state.selectors.kernel_data);
        ES::set_reg(state.selectors.kernel_data);
        SS::set_reg(state.selectors.kernel_data);
        load_tss(state.selectors.tss);
    }
}

pub fn init() {
    {
        let _access = access();
        let _ = all_unlocked(); // construit tout pendant le boot BSP mono-CPU
    }
    load_cpu(0);
    READY.store(true, Ordering::Release);
    dmesg::log("gdt: GDT/TSS per-CPU prets (RSP0 + IST double faute)");
}

pub fn init_ap(cpu: usize) { load_cpu(cpu); }

/// La disposition des selecteurs est identique sur chaque GDT. Un cache Copy
/// evite qu'un AP lisant STAR ait a emprunter le Vec pendant qu'un autre AP
/// charge son propre GDTR/TSS.
pub fn selectors() -> Selectors {
    unsafe { BASE_SELECTORS.expect("gdt: selecteurs non initialises") }
}

pub fn selectors_for(_cpu: usize) -> Selectors { selectors() }

pub fn state() -> &'static str {
    if READY.load(Ordering::Acquire) {
        "initialisee (GDT/TSS per-CPU, ring0+ring3, RSP0, IST)"
    } else {
        "non chargee"
    }
}

pub fn set_kernel_stack_for(cpu: usize, top: u64) {
    let _access = access();
    let state = &mut all_unlocked()[cpu.min(MAX_CPUS - 1)];
    unsafe { (*state.tss).privilege_stack_table[0] = VirtAddr::new(top); }
}

pub fn kernel_stack_for(cpu: usize) -> u64 {
    let _access = access();
    let state = &mut all_unlocked()[cpu.min(MAX_CPUS - 1)];
    unsafe { (*state.tss).privilege_stack_table[0].as_u64() }
}

pub fn set_kernel_stack(top: u64) {
    let cpu = crate::arch::x86_64::usermode::cpu_index();
    set_kernel_stack_for(cpu, top);
}

pub fn kernel_stack() -> u64 {
    let cpu = crate::arch::x86_64::usermode::cpu_index();
    kernel_stack_for(cpu)
}
