extern "x86-interrupt" fn breakpoint_handler(stack: InterruptStackFrame) {
    let _kernel = crate::kernel::smp_lock::enter();
    println!("exception: breakpoint (int3) capturee, on continue");
    serial_println!("[cpu] breakpoint at {:?}", stack.instruction_pointer);
}

static PANIC_GLOBAL: AtomicBool = AtomicBool::new(false);
static CPU_FAUTIF: AtomicUsize = AtomicUsize::new(usize::MAX);

pub fn panique_globale_en_cours() -> bool {
    PANIC_GLOBAL.load(Ordering::Acquire)
}

pub fn prends_la_panique(cpu: usize) -> bool {
    if PANIC_GLOBAL
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return false;
    }
    CPU_FAUTIF.store(cpu, Ordering::Release);
    true
}

pub fn arret_definitif() -> ! {
    loop {
        unsafe { core::arch::asm!("cli; hlt", options(nomem, nostack)) };
    }
}

fn releve_faute_fatale(nom: &str, stack: &InterruptStackFrame, code: u64) {
    let cpu = smp::cpu_index();
    let rsp_frame = stack.stack_pointer.as_u64();

    serial_println!("");
    serial_println!("======== [{}] ========", nom);
    serial_println!(
        "[FAULT] cpu={} apic={} code={:#x}",
        cpu,
        crate::arch::x86_64::cpu_local::CpuId::from_index(cpu)
            .and_then(crate::arch::x86_64::cpu_local::descriptor)
            .map(|d| d.apic_id)
            .unwrap_or(u32::MAX),
        code,
    );
    serial_println!(
        "[FAULT] rip={:#x} cs={:#x} rflags={:#x} rsp={:#x} ss={:#x}",
        stack.instruction_pointer.as_u64(),
        stack.code_segment,
        stack.cpu_flags,
        rsp_frame,
        stack.stack_segment,
    );

    let (cr2, cr3) = unsafe {
        let cr2: u64;
        let cr3: u64;
        core::arch::asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack));
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack));
        (cr2, cr3)
    };
    serial_println!("[FAULT] cr2={:#x} cr3={:#x}", cr2, cr3);

    releve_contexte_courant(cpu, Some(rsp_frame));
    serial_println!("======== fin du releve ========");
}

pub fn releve_contexte_courant(cpu: usize, rsp_connu: Option<u64>) {
    match crate::kernel::task::identite_pour_faute() {
        Some((index, pid, tid, kstack_top, kstack_base, in_kernel)) => {
            serial_println!(
                "[FAULT] task={} pid={} tid={} nom={} in_kernel={}",
                index, pid, tid, crate::kernel::task::nom_pour_faute(), in_kernel,
            );
            serial_println!(
                "[FAULT] kstack base={:#x} top={:#x} taille={}",
                kstack_base, kstack_top, kstack_top.saturating_sub(kstack_base),
            );
            match rsp_connu {
                Some(rsp) => serial_println!(
                    "[FAULT] rsp_dans_kstack={} ecart_sous_base={}",
                    rsp >= kstack_base && rsp <= kstack_top,
                    kstack_base.saturating_sub(rsp),
                ),
                None => serial_println!("[FAULT] rsp_dans_kstack=<pas de trame>"),
            }
        }
        None => serial_println!("[FAULT] task=<aucune> (idle ou avant premier switch)"),
    }

    serial_println!(
        "[FAULT] tss_rsp0={:#x} gs_cpu_index={}",
        gdt::rsp0_courant(cpu),
        usermode::cpu_index(),
    );

    let provenance = crate::kernel::smp_lock::stall_probe_provenance();
    serial_println!(
        "[FAULT] bkl owner_token={} depth={} coherent={} acquire_kind={} acquire_seq={} release_seq={}",
        provenance.owner_token,
        crate::kernel::smp_lock::stall_probe_depth(cpu),
        provenance.coherent,
        provenance.acquire_kind,
        provenance.acquire_seq,
        provenance.release_seq,
    );
    serial_println!(
        "[FAULT] syscall_nr={} phase={} site={} aux={:#x} task_bkl={}",
        provenance.syscall_nr,
        provenance.syscall_phase,
        provenance.site,
        provenance.aux,
        provenance.task,
    );
    serial_println!(
        "[FAULT] need_resched={} irq_profondeur=<non suivi>",
        crate::kernel::task::besoin_de_replanifier(),
    );
}

extern "x86-interrupt" fn panic_stop_handler(_stack: InterruptStackFrame) {
    arret_definitif();
}

extern "x86-interrupt" fn double_fault_handler(stack: InterruptStackFrame, code: u64) -> ! {
    let cpu = smp::cpu_index();
    if !prends_la_panique(cpu) {
        arret_definitif();
    }

    releve_faute_fatale("DOUBLE FAULT", &stack, code);
    smp::arrete_les_autres_cpu();
    serial_println!("*** KERNEL PANIC *** double faute, cpu={}", cpu);
    arret_definitif();
}

fn from_user(stack: &InterruptStackFrame) -> bool {
    stack.code_segment & 3 == 3
}

struct GsGuard { swapped: bool }

impl GsGuard {
    fn enter(stack: &InterruptStackFrame) -> Self {
        let swapped = from_user(stack);
        if swapped {
            unsafe { crate::arch::x86_64::usermode::swapgs() };
        }
        GsGuard { swapped }
    }
}

impl Drop for GsGuard {
    fn drop(&mut self) {
        if self.swapped {
            unsafe { crate::arch::x86_64::usermode::swapgs() };
        }
    }
}

fn kill_faulting_task(reason: &str, stack: &InterruptStackFrame) -> ! {
    let _kernel = crate::kernel::smp_lock::enter();
    let cr2 = x86_64::registers::control::Cr2::read().as_u64();
    crate::println!(
        "{} dans le programme utilisateur (rip={:#x}) : processus termine",
        reason,
        stack.instruction_pointer.as_u64()
    );
    serial_println!(
        "[cpu] {} en ring 3 : rip={:#x} rsp={:#x} cr2={:#x} flags={:#x}",
        reason,
        stack.instruction_pointer.as_u64(),
        stack.stack_pointer.as_u64(),
        cr2,
        stack.cpu_flags
    );
    crate::kernel::task::exit_group(139)
}

extern "x86-interrupt" fn general_protection_handler(stack: InterruptStackFrame, code: u64) {
    let _gs = GsGuard::enter(&stack);
    let _kernel = crate::kernel::smp_lock::enter();
    if from_user(&stack) && crate::kernel::task::in_user_task() {
        kill_faulting_task("faute de protection generale", &stack);
    }
    releve_faute_fatale("GENERAL PROTECTION FAULT", &stack, code);
    panic!("EXCEPTION: general protection fault (code {})\n{:#?}", code, stack);
}

extern "x86-interrupt" fn invalid_opcode_handler(stack: InterruptStackFrame) {
    let _gs = GsGuard::enter(&stack);
    let _kernel = crate::kernel::smp_lock::enter();
    if from_user(&stack) && crate::kernel::task::in_user_task() {
        kill_faulting_task("instruction illegale", &stack);
    }
    panic!("EXCEPTION: instruction illegale\n{:#?}", stack);
}

extern "x86-interrupt" fn divide_error_handler(stack: InterruptStackFrame) {
    let _gs = GsGuard::enter(&stack);
    let _kernel = crate::kernel::smp_lock::enter();
    if from_user(&stack) && crate::kernel::task::in_user_task() {
        kill_faulting_task("division par zero", &stack);
    }
    panic!("EXCEPTION: division par zero\n{:#?}", stack);
}

extern "x86-interrupt" fn stack_segment_handler(stack: InterruptStackFrame, code: u64) {
    let _gs = GsGuard::enter(&stack);
    let _kernel = crate::kernel::smp_lock::enter();
    if from_user(&stack) && crate::kernel::task::in_user_task() {
        kill_faulting_task("faute de pile", &stack);
    }
    releve_faute_fatale("STACK SEGMENT FAULT", &stack, code);
    panic!("EXCEPTION: faute de segment de pile (code {})", code);
}

extern "x86-interrupt" fn page_fault_handler(
    stack: InterruptStackFrame,
    code: PageFaultErrorCode,
) {
    let _gs = GsGuard::enter(&stack);
    let addr = x86_64::registers::control::Cr2::read();
    let _site = crate::kernel::task::SiteIrq::enter(20, addr.as_u64());
    crate::kernel::task::stall_pf_begin(addr.as_u64());

    if from_user(&stack) {
        x86_64::instructions::interrupts::enable();
    }

    crate::kernel::task::stall_site_set(21, addr.as_u64());
    if from_user(&stack) && crate::kernel::task::in_user_task() {
        let mut retries = 0u32;
        let outcome = loop {
            let outcome = crate::kernel::task::peuple_a_la_demande(
                addr.as_u64(),
                code.contains(PageFaultErrorCode::PROTECTION_VIOLATION),
            );
            if outcome != crate::kernel::task::FaultOutcome::Retry {
                break outcome;
            }
            retries = retries.wrapping_add(1);
            if retries % 8 == 0 {
                crate::kernel::task::fault_retry_yield();
            } else {
                core::hint::spin_loop();
            }
        };
        crate::kernel::task::fault_retry_chain_complete(retries as u64);

        if outcome == crate::kernel::task::FaultOutcome::Resolved {
            crate::kernel::task::stall_pf_done(addr.as_u64());
            let _kernel = crate::kernel::smp_lock::enter();
            crate::kernel::task::retire_current_if_zombie();
            return;
        }

        if outcome == crate::kernel::task::FaultOutcome::Retired {
            let _kernel = crate::kernel::smp_lock::enter();
            crate::kernel::task::retire_current_if_zombie();
        }

        crate::kernel::task::stall_pf_fail(addr.as_u64());
        crate::kernel::task::log_fault_mapping(addr.as_u64());
        crate::println!(
            "faute de page utilisateur @ {:#x} ({:?})",
            addr.as_u64(),
            code
        );
        kill_faulting_task("faute de page", &stack);
    }

    serial_println!("[cpu] page fault @ {:?} code {:?}", addr, code);
    panic!(
        "EXCEPTION: page fault @ {:?}\ncode: {:?}\n{:#?}",
        addr, code, stack
    );
}

extern "x86-interrupt" fn tlb_shootdown_interrupt_handler(stack: InterruptStackFrame) {
    let _gs = GsGuard::enter(&stack);
    smp::handle_tlb_shootdown();
}
