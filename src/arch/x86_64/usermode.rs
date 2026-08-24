//! Bascule ring 0 <-> ring 3 : syscall/sysretq/iretq, avec etat GS per-CPU.

use core::arch::{asm, naked_asm};
use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::x86_64::{gdt, smp};

const MSR_EFER: u32 = 0xC000_0080;
const MSR_STAR: u32 = 0xC000_0081;
const MSR_LSTAR: u32 = 0xC000_0082;
const MSR_FMASK: u32 = 0xC000_0084;
pub const MSR_FS_BASE: u32 = 0xC000_0100;
pub const MSR_GS_BASE: u32 = 0xC000_0101;
pub const MSR_KERNEL_GS_BASE: u32 = 0xC000_0102;

pub const USER_CS: u64 = 0x20 | 3;
pub const USER_SS: u64 = 0x18 | 3;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct TrapFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

impl TrapFrame {
    pub fn new_user(entry: u64, stack: u64) -> Self {
        TrapFrame {
            rip: entry,
            cs: USER_CS,
            rflags: 0x202,
            rsp: stack,
            ss: USER_SS,
            ..Default::default()
        }
    }

    pub fn syscall_args(&self) -> (u64, [u64; 6]) {
        (self.rax, [self.rdi, self.rsi, self.rdx, self.r10, self.r8, self.r9])
    }
}

/// Les trois premiers champs gardent strictement les offsets historiques du
/// stub syscall. `cpu_index` est ajoute apres eux.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PerCpu {
    pub kernel_rsp: u64, // gs:0
    pub user_rsp: u64,   // gs:8
    pub current: u64,    // gs:16 (tid)
    pub cpu_index: u64,  // gs:24
}

impl PerCpu {
    const fn new() -> Self {
        Self { kernel_rsp: 0, user_rsp: 0, current: 0, cpu_index: 0 }
    }
}

static mut PER_CPU: [PerCpu; smp::MAX_CPUS] = [PerCpu::new(); smp::MAX_CPUS];
static READY: AtomicBool = AtomicBool::new(false);

pub fn per_cpu_for(cpu: usize) -> &'static mut PerCpu {
    unsafe { &mut *core::ptr::addr_of_mut!(PER_CPU[cpu.min(smp::MAX_CPUS - 1)]) }
}

pub fn per_cpu() -> &'static mut PerCpu {
    let base = read_msr(MSR_GS_BASE);
    if base == 0 {
        return per_cpu_for(0);
    }
    unsafe { &mut *(base as *mut PerCpu) }
}

pub fn cpu_index() -> usize {
    per_cpu().cpu_index as usize
}

pub fn set_kernel_stack(top: u64) {
    let cpu = cpu_index().min(smp::MAX_CPUS - 1);
    per_cpu_for(cpu).kernel_rsp = top;
    gdt::set_kernel_stack_for(cpu, top);
}

pub fn read_msr(msr: u32) -> u64 {
    let (high, low): (u32, u32);
    unsafe {
        asm!("rdmsr", in("ecx") msr, out("eax") low, out("edx") high,
             options(nomem, nostack, preserves_flags));
    }
    ((high as u64) << 32) | low as u64
}

pub unsafe fn write_msr(msr: u32, value: u64) {
    asm!(
        "wrmsr",
        in("ecx") msr,
        in("eax") (value & 0xFFFF_FFFF) as u32,
        in("edx") (value >> 32) as u32,
        options(nomem, nostack, preserves_flags)
    );
}

pub fn state() -> &'static str {
    if READY.load(Ordering::Acquire) {
        "active (syscall/sysretq, ring3, GS/TSS per-CPU, SSE)"
    } else {
        "inactive"
    }
}

pub fn ready() -> bool {
    READY.load(Ordering::Acquire)
}

pub fn init() {
    init_cpu(0, true);
}

pub fn init_ap(cpu: usize) {
    init_cpu(cpu, false);
}

fn init_cpu(cpu: usize, log: bool) {
    let sel = gdt::selectors_for(cpu);
    assert_eq!(sel.user_code.0 as u64, USER_CS, "gdt: ordre des selecteurs user modifie");
    assert_eq!(sel.user_data.0 as u64, USER_SS, "gdt: ordre des selecteurs user modifie");

    unsafe {
        write_msr(MSR_EFER, read_msr(MSR_EFER) | 1);
        let syscall_cs = sel.kernel_code.0 as u64;
        let sysret_base = (sel.kernel_data.0 as u64) | 3;
        write_msr(MSR_STAR, (sysret_base << 48) | (syscall_cs << 32));
        write_msr(MSR_LSTAR, syscall_entry as *const () as usize as u64);
        write_msr(MSR_FMASK, 0x0004_0700);

        let pcpu = per_cpu_for(cpu);
        pcpu.cpu_index = cpu as u64;
        pcpu.current = 0;
        pcpu.kernel_rsp = gdt::kernel_stack_for(cpu);
        write_msr(MSR_GS_BASE, pcpu as *mut PerCpu as u64);
        write_msr(MSR_KERNEL_GS_BASE, 0);

        enable_sse();
    }
    READY.store(true, Ordering::Release);
    if log {
        crate::kernel::dmesg::log(
            "usermode: syscall/sysret armes, GS/TSS per-CPU, SSE actif"
        );
    }
}

unsafe fn enable_sse() {
    use x86_64::registers::control::{Cr0, Cr0Flags, Cr4, Cr4Flags};
    let mut cr0 = Cr0::read();
    cr0.remove(Cr0Flags::EMULATE_COPROCESSOR);
    cr0.insert(Cr0Flags::MONITOR_COPROCESSOR);
    Cr0::write(cr0);
    let mut cr4 = Cr4::read();
    cr4.insert(Cr4Flags::OSFXSR);
    cr4.insert(Cr4Flags::OSXMMEXCPT_ENABLE);
    Cr4::write(cr4);
    asm!("fninit", options(nomem, nostack));
}

pub fn set_fs_base(base: u64) {
    unsafe { write_msr(MSR_FS_BASE, base) };
}

pub fn fs_base() -> u64 {
    read_msr(MSR_FS_BASE)
}

pub unsafe fn fxsave(area: *mut u8) {
    asm!("fxsave64 [{}]", in(reg) area, options(nostack));
}

pub unsafe fn fxrstor(area: *const u8) {
    asm!("fxrstor64 [{}]", in(reg) area, options(nostack));
}

pub unsafe fn swapgs() {
    asm!("swapgs", options(nomem, nostack, preserves_flags));
}

#[unsafe(naked)]
unsafe extern "C" fn syscall_entry() {
    naked_asm!(
        "swapgs",
        "mov gs:[8], rsp",
        "mov rsp, gs:[0]",
        "push {user_ss}",
        "push qword ptr gs:[8]",
        "push r11",
        "push {user_cs}",
        "push rcx",
        "push rax",
        "push rbx",
        "push rcx",
        "push rdx",
        "push rsi",
        "push rdi",
        "push rbp",
        "push r8",
        "push r9",
        "push r10",
        "push r11",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "sti",
        "mov rdi, rsp",
        "call {dispatch}",
        "cli",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "pop rbp",
        "pop rdi",
        "pop rsi",
        "pop rdx",
        "pop rcx",
        "pop rbx",
        "pop rax",
        "pop rcx",
        "add rsp, 8",
        "pop r11",
        "pop rsp",
        "swapgs",
        "sysretq",
        dispatch = sym syscall_dispatch,
        user_cs = const USER_CS,
        user_ss = const USER_SS,
    )
}

unsafe extern "C" fn syscall_dispatch(frame: *mut TrapFrame) {
    // BOUCHAUD_SMP4_STALL_PROBE_V1
    // Phase 1 est visible meme si ce CPU se bloque dans enter(BKL).
    crate::kernel::task::stall_syscall_enter((*frame).rax);
    let kernel = crate::kernel::smp_lock::enter();
    crate::kernel::task::stall_syscall_bkl_acquired();
    crate::kernel::task::account_kernel_enter();
    // These MM calls touch only Arc<Process>::Mm and the IRQ-safe TLB
    // protocol. Release the legacy syscall BKL after accounting so sibling
    // threads can mutate independent address spaces concurrently.
    let audited_mm = matches!((*frame).rax, 10 | 11 | 12);
    if audited_mm {
        drop(kernel);
        crate::kernel::abi::handle(&mut *frame);
        let kernel = crate::kernel::smp_lock::enter();
        crate::kernel::task::account_kernel_exit();
        drop(kernel);
    } else {
        crate::kernel::abi::handle(&mut *frame);
        crate::kernel::task::account_kernel_exit();
        drop(kernel);
    }
    crate::kernel::task::stall_syscall_exit();
}

#[unsafe(naked)]
pub unsafe extern "C" fn resume_usermode(frame: *const TrapFrame) -> ! {
    naked_asm!(
        "cli",
        "swapgs",
        "mov rsp, rdi",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "pop rbp",
        "pop rdi",
        "pop rsi",
        "pop rdx",
        "pop rcx",
        "pop rbx",
        "pop rax",
        "iretq",
    )
}
