from pathlib import Path

ROOT = Path.cwd()

def load(path):
    p = ROOT / path
    if not p.exists():
        raise SystemExit(f"[ERREUR] fichier absent: {path}")
    return p, p.read_text(encoding="utf-8")

def save(p, text):
    p.write_text(text, encoding="utf-8", newline="\n")

def replace_once(text, old, new, label):
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"[ERREUR] {label}: attendu 1 occurrence, trouve {count}")
    return text.replace(old, new, 1)

# 1) PIC IRQ11 -> Network
p, s = load("src/arch/x86_64/interrupts.rs")
s = replace_once(
    s,
    "    Timer = PIC_1_OFFSET,\n    Keyboard,\n    Mouse = PIC_1_OFFSET + 12,\n",
    "    Timer = PIC_1_OFFSET,\n    Keyboard,\n"
    "    /// IRQ11 PCI : e1000 sous la plateforme QEMU de reference.\n"
    "    Network = PIC_1_OFFSET + 11,\n"
    "    Mouse = PIC_1_OFFSET + 12,\n",
    "InterruptIndex::Network",
)
save(p, s)

# 2) e1000 : IRQ RX + moderation
p, s = load("src/drivers/network/e1000.rs")
s = replace_once(
    s,
    "use core::ptr::{read_volatile, write_volatile};\nuse crate::arch::x86_64::pci;\n",
    "use core::ptr::{read_volatile, write_volatile};\n"
    "use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};\n"
    "use crate::arch::x86_64::{interrupts, pci};\n",
    "imports e1000",
)
s = replace_once(
    s,
    "const REG_ICR: u32 = 0x00C0;\nconst REG_IMC: u32 = 0x00D8;\n",
    "const REG_ICR: u32 = 0x00C0;\n"
    "const REG_ITR: u32 = 0x00C4;\n"
    "const REG_IMS: u32 = 0x00D0;\n"
    "const REG_IMC: u32 = 0x00D8;\n",
    "registres IRQ e1000",
)
s = replace_once(
    s,
    "const DESC_SZ: usize = 16;\n\nstatic mut MMIO: u64 = 0;\n",
    "const DESC_SZ: usize = 16;\n\n"
    "/// Causes RX transformees en reveil de readiness.\n"
    "/// RXT0 = paquet recu apres moderation, RXO = overflow RX.\n"
    "pub const RX_INTERRUPT_MASK: u32 = 0x80 | 0x40;\n\n"
    "static RX_INTERRUPTS_ACTIVE: AtomicBool = AtomicBool::new(false);\n"
    "static RX_IRQ_COUNT: AtomicU64 = AtomicU64::new(0);\n\n"
    "static mut MMIO: u64 = 0;\n",
    "etat IRQ e1000",
)
s = replace_once(
    s,
    "pub fn tx_anneau_plein() -> u64 {\n    unsafe { TX_ANNEAU_PLEIN }\n}\n\n"
    "unsafe fn reg_read(off: u32) -> u32 {\n",
    "pub fn tx_anneau_plein() -> u64 {\n    unsafe { TX_ANNEAU_PLEIN }\n}\n\n"
    "pub fn rx_interrupts_active() -> bool {\n"
    "    RX_INTERRUPTS_ACTIVE.load(Ordering::Acquire)\n"
    "}\n\n"
    "pub fn rx_irq_count() -> u64 {\n"
    "    RX_IRQ_COUNT.load(Ordering::Relaxed)\n"
    "}\n\n"
    "/// Acquitte ICR. Le handler ne consomme aucun descripteur RX.\n"
    "pub fn ack_interrupt() -> u32 {\n"
    "    if !rx_interrupts_active() { return 0; }\n"
    "    let cause = unsafe { reg_read(REG_ICR) };\n"
    "    if cause & RX_INTERRUPT_MASK != 0 {\n"
    "        RX_IRQ_COUNT.fetch_add(1, Ordering::Relaxed);\n"
    "    }\n"
    "    cause\n"
    "}\n\n"
    "unsafe fn reg_read(off: u32) -> u32 {\n",
    "API IRQ e1000",
)
s = replace_once(
    s,
    '    if dev.vendor != 0x8086 {\n'
    '        dmesg::log("e1000: carte non Intel, driver non charge");\n'
    '        return false;\n'
    '    }\n\n'
    '    pci::enable_bus_master(&dev);\n',
    '    if dev.vendor != 0x8086 {\n'
    '        dmesg::log("e1000: carte non Intel, driver non charge");\n'
    '        return false;\n'
    '    }\n\n'
    '    let irq_line = pci::interrupt_line(&dev);\n\n'
    '    pci::enable_bus_master(&dev);\n',
    "lecture ligne IRQ PCI",
)
s = replace_once(
    s,
    "        // Masque toutes les interruptions (on fonctionne en polling).\n"
    "        reg_write(REG_IMC, 0xFFFF_FFFF);\n"
    "        let _ = reg_read(REG_ICR);\n",
    "        // Tout reste masque jusqu'a ce que les anneaux RX/TX soient prets.\n"
    "        reg_write(REG_IMC, 0xFFFF_FFFF);\n"
    "        let _ = reg_read(REG_ICR);\n",
    "masque IRQ init",
)
s = replace_once(
    s,
    '        READY = true;\n'
    '    }\n'
    '    dmesg::log("e1000: initialise (RX/TX prets)");\n'
    '    true\n'
    '}\n',
    '        READY = true;\n\n'
    '        if irq_line == 11 {\n'
    '            // ITR : quanta de 256 ns. 4000 ~= 1,024 ms.\n'
    '            reg_write(REG_ITR, 4_000);\n'
    '            let _ = reg_read(REG_ICR);\n'
    '            reg_write(REG_IMS, RX_INTERRUPT_MASK);\n'
    '            RX_INTERRUPTS_ACTIVE.store(true, Ordering::Release);\n'
    '            interrupts::unmask_irq(irq_line);\n'
    '        }\n'
    '    }\n\n'
    '    if rx_interrupts_active() {\n'
    '        crate::serial_println!("[kernel] e1000: RX interrupt-driven irq={} itr=1024us", irq_line);\n'
    '    } else {\n'
    '        crate::serial_println!("[kernel] e1000: IRQ RX indisponible ligne={} ; fallback poll 2ms conserve", irq_line);\n'
    '    }\n'
    '    dmesg::log("e1000: initialise (RX/TX prets)");\n'
    '    true\n'
    '}\n',
    "activation IRQ e1000",
)
s = replace_once(
    s,
    '    crate::println!("e1000: MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}  lien={}",\n'
    '        m[0], m[1], m[2], m[3], m[4], m[5], if link_up() { "UP" } else { "DOWN" });\n',
    '    crate::println!(\n'
    '        "e1000: MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}  lien={}  rx_irq={} count={}",\n'
    '        m[0], m[1], m[2], m[3], m[4], m[5],\n'
    '        if link_up() { "UP" } else { "DOWN" },\n'
    '        if rx_interrupts_active() { "on" } else { "off" },\n'
    '        rx_irq_count()\n'
    '    );\n',
    "ethinfo IRQ",
)
save(p, s)

# 3) IDT : handler IRQ11 minimal
p, s = load("src/arch/x86_64/idt.rs")
s = replace_once(
    s,
    "        IDT[InterruptIndex::Keyboard.as_usize()].set_handler_fn(keyboard_interrupt_handler);\n"
    "        IDT[InterruptIndex::Mouse.as_usize()].set_handler_fn(mouse_interrupt_handler);\n",
    "        IDT[InterruptIndex::Keyboard.as_usize()].set_handler_fn(keyboard_interrupt_handler);\n"
    "        IDT[InterruptIndex::Network.as_usize()].set_handler_fn(network_interrupt_handler);\n"
    "        IDT[InterruptIndex::Mouse.as_usize()].set_handler_fn(mouse_interrupt_handler);\n",
    "IDT Network",
)
marker = (
    'extern "x86-interrupt" fn breakpoint_handler(stack: InterruptStackFrame) {\n'
    '    let _kernel = crate::kernel::smp_lock::enter();\n'
    '    println!("exception: breakpoint (int3) capturee, on continue");\n'
    '    serial_println!("[cpu] breakpoint at {:?}", stack.instruction_pointer);\n'
    '}\n'
)
handler = marker + (
    '\n/// IRQ11 e1000 : acquittement + reveil readiness, aucun drain reseau ici.\n'
    'extern "x86-interrupt" fn network_interrupt_handler(_stack: InterruptStackFrame) {\n'
    '    let cause = crate::drivers::network::e1000::ack_interrupt();\n'
    '    if cause & crate::drivers::network::e1000::RX_INTERRUPT_MASK != 0 {\n'
    '        crate::kernel::fd::notify_readiness();\n'
    '    }\n'
    '    notify_end_of_interrupt(InterruptIndex::Network.as_u8());\n'
    '}\n'
)
s = replace_once(s, marker, handler, "handler IRQ e1000")
save(p, s)

# 4) poll : 2 ms permanent -> IRQ + watchdog 50 ms
p, s = load("src/compat/linux/file.rs")
start_token = "        // IMPORTANT: les sockets n'ont pas encore de signal IRQ RX."
start = s.find(start_token)
if start < 0:
    raise SystemExit("[ERREUR] commentaire socket poll 2ms introuvable")
end = s.find("\n\n", start)
if end < 0:
    raise SystemExit("[ERREUR] fin bloc socket poll introuvable")
old = s[start:end+2]
if "POLL_SOCKETS_NS: u64 = 2_000_000" not in old or "sockets_presentes" not in old:
    raise SystemExit("[ERREUR] bloc socket poll inattendu, refus de patcher")
new = '''        // BOUCHAUD_NET_RX_IRQ_V1
        //
        // Le vieux chemin reveillait chaque poller de socket toutes les 2 ms.
        // Avec l'IRQ RX active, le materiel reveille READINESS immediatement.
        // Le watchdog 50 ms ne sert que de filet de securite si une IRQ se perd.
        let sockets_presentes = fd_snapshot
            .iter()
            .any(|(_, desc)| matches!(desc, Descripteur::Socket(_)));
        if sockets_presentes {
            const POLL_SOCKETS_FALLBACK_NS: u64 = 2_000_000;
            const POLL_SOCKETS_WATCHDOG_NS: u64 = 50_000_000;
            let reveil_ns = if crate::drivers::network::e1000::rx_interrupts_active() {
                POLL_SOCKETS_WATCHDOG_NS
            } else {
                POLL_SOCKETS_FALLBACK_NS
            };
            timeout_effectif_ns = Some(match timeout_effectif_ns {
                Some(ns) => ns.min(reveil_ns),
                None => reveil_ns,
            });
        }

'''
s = s[:start] + new + s[end+2:]
save(p, s)

print("[OK] Patch WebContent liveness applique.")
