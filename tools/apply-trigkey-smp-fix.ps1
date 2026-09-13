<#
Bouchaud OS - correctif Trigkey SMP double fault
Appliquer depuis la racine du repo bouchaud-os.

Ce script modifie uniquement :
- src/arch/x86_64/smp.rs
- src/arch/x86_64/idt/timer.rs
- src/arch/x86_64/idt/reschedule.rs
- src/platform/pc/stage2.rs

Il crée une sauvegarde dans .patch-backup/ avant modification.
#>

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Read-Text($Path) {
    return [System.IO.File]::ReadAllText($Path)
}

function Write-Text($Path, $Text) {
    $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($Path, $Text, $utf8NoBom)
}

function Assert-RepoRoot() {
    if (!(Test-Path ".\Cargo.toml") -or !(Test-Path ".\src")) {
        throw "Lance ce script depuis la racine du repo bouchaud-os."
    }
}

function Backup-File($Path, $BackupRoot) {
    $dest = Join-Path $BackupRoot $Path
    $destDir = Split-Path $dest -Parent
    New-Item -ItemType Directory -Force -Path $destDir | Out-Null
    Copy-Item -Force $Path $dest
}

function Replace-Once($Path, $Needle, $Replacement, $Marker) {
    $text = Read-Text $Path
    if ($text.Contains($Marker)) {
        Write-Host "[skip] $Path : marqueur deja present ($Marker)"
        return $false
    }
    if (!$text.Contains($Needle)) {
        throw "Patch impossible pour $Path : bloc cible introuvable. Verifie que tu es bien a jour sur main."
    }
    $text = $text.Replace($Needle, $Replacement)
    Write-Text $Path $text
    Write-Host "[ok] $Path : $Marker"
    return $true
}

function Insert-After($Path, $Needle, $Insertion, $Marker) {
    $text = Read-Text $Path
    if ($text.Contains($Marker)) {
        Write-Host "[skip] $Path : marqueur deja present ($Marker)"
        return $false
    }
    $idx = $text.IndexOf($Needle)
    if ($idx -lt 0) {
        throw "Patch impossible pour $Path : point d'insertion introuvable. Verifie que tu es bien a jour sur main."
    }
    $idx = $idx + $Needle.Length
    $text = $text.Insert($idx, $Insertion)
    Write-Text $Path $text
    Write-Host "[ok] $Path : $Marker"
    return $true
}

Assert-RepoRoot

$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$backupRoot = Join-Path ".patch-backup" "trigkey-smp-stack-safe-$stamp"
New-Item -ItemType Directory -Force -Path $backupRoot | Out-Null

$files = @(
    "src/arch/x86_64/smp.rs",
    "src/arch/x86_64/idt/timer.rs",
    "src/arch/x86_64/idt/reschedule.rs",
    "src/platform/pc/stage2.rs"
)
foreach ($f in $files) {
    if (!(Test-Path $f)) { throw "Fichier introuvable : $f" }
    Backup-File $f $backupRoot
}

# 1) SMP bootstrap guard : pendant INIT/SIPI, le BSP ne doit pas prendre IRQ0,
# réveiller des files, ni replanifier sur une pile de boot non stabilisée.
$smpPath = "src/arch/x86_64/smp.rs"
$smpNeedle = @'
static SCHEDULER_ENABLED: AtomicBool = AtomicBool::new(false);
static LOCAL_SCHED_TIMER: AtomicBool = AtomicBool::new(false);
'@
$smpReplacement = @'
static SCHEDULER_ENABLED: AtomicBool = AtomicBool::new(false);
static LOCAL_SCHED_TIMER: AtomicBool = AtomicBool::new(false);

// BOUCHAUD_SMP_BOOTSTRAP_GUARD_V1
//
// Le crash physique Trigkey apparaissait juste apres `SMP4_STAGE sipi-1` :
// le BSP avait deja IRQ0 actif, le reseau venait de publier `net-lien`, puis
// la fenetre INIT/SIPI laissait le timer entrer dans le chemin reveil/scheduler
// alors que les AP etaient encore entre trampoline, pile bootstrap et etat CPU
// incomplet. Un RSP ring0 hors pile noyau finissait en double faute.
//
// Pendant cette fenetre, le noyau ne doit faire qu'une chose : demarrer les AP.
// Pas de tick scheduler, pas de flush waitqueue, pas de reschedule IPI traite.
static SMP_BOOTSTRAP_GUARD: AtomicBool = AtomicBool::new(false);

pub fn bootstrap_in_progress() -> bool {
    SMP_BOOTSTRAP_GUARD.load(Ordering::Acquire)
}

struct SmpBootstrapGuard {
    interrupts_were_enabled: bool,
}

impl SmpBootstrapGuard {
    fn enter() -> Self {
        let interrupts_were_enabled = x86_64::instructions::interrupts::are_enabled();
        x86_64::instructions::interrupts::disable();
        SMP_BOOTSTRAP_GUARD.store(true, Ordering::Release);
        crate::serial_println!(
            "SMP_BOOT_GUARD_ENTER irq=off previous_irq={}",
            interrupts_were_enabled as u8,
        );
        Self { interrupts_were_enabled }
    }
}

impl Drop for SmpBootstrapGuard {
    fn drop(&mut self) {
        SMP_BOOTSTRAP_GUARD.store(false, Ordering::Release);
        crate::serial_println!(
            "SMP_BOOT_GUARD_EXIT irq=restored previous_irq={}",
            self.interrupts_were_enabled as u8,
        );
        if self.interrupts_were_enabled {
            x86_64::instructions::interrupts::enable();
        }
    }
}
'@
Replace-Once $smpPath $smpNeedle $smpReplacement "BOUCHAUD_SMP_BOOTSTRAP_GUARD_V1" | Out-Null

$smpProbeNeedle = @'
    if exposed <= 1 {
        dmesg::log("SMP4_AP_STARTED count=0 reason=single-vcpu");
        dmesg::log("SMP4_SCHEDULER online=1 mode=UP");
        return;
    }

'@
$smpProbeInsertion = @'
    let _boot_guard = SmpBootstrapGuard::enter();

'@
Insert-After $smpPath $smpProbeNeedle $smpProbeInsertion "SMP_BOOT_GUARD_ENTER" | Out-Null

$smpStackNeedle = @'
        for cpu in 0..MAX_CPUS {
            let base = core::ptr::addr_of!(AP_STACKS[cpu].0) as *const u8 as u64;
            let top = (base + AP_STACK_SIZE as u64) & !0xF;
            write_volatile(mailbox.add(0x20 + cpu * 8) as *mut u64, top);
        }

'@
$smpStackReplacement = @'
        for cpu in 0..MAX_CPUS {
            let base = core::ptr::addr_of!(AP_STACKS[cpu].0) as *const u8 as u64;
            let top = (base + AP_STACK_SIZE as u64) & !0xF;
            write_volatile(mailbox.add(0x20 + cpu * 8) as *mut u64, top);
        }
        crate::serial_println!(
            "SMP_AP_STACK_TABLE_OK count={} first_top={:#x} last_top={:#x}",
            MAX_CPUS,
            read_volatile(mailbox.add(0x20) as *const u64),
            read_volatile(mailbox.add(0x20 + (MAX_CPUS - 1) * 8) as *const u64),
        );

'@
Replace-Once $smpPath $smpStackNeedle $smpStackReplacement "SMP_AP_STACK_TABLE_OK" | Out-Null

# 2) IRQ0 : si une interruption etait deja entree au moment du guard, elle ne
# doit pas toucher au scheduler/reveils pendant le bootstrap SMP.
$timerPath = "src/arch/x86_64/idt/timer.rs"
$timerNeedle = @'
    notify_end_of_interrupt(InterruptIndex::Timer.as_u8());
    crate::kernel::blackbox::timer_stage(blackbox_cpu, 3);

'@
$timerReplacement = @'
    notify_end_of_interrupt(InterruptIndex::Timer.as_u8());
    crate::kernel::blackbox::timer_stage(blackbox_cpu, 3);

    // BOUCHAUD_SMP_BOOTSTRAP_GUARD_V1
    // Aucun reveil, watchdog ou preemption pendant INIT/SIPI : cette fenetre
    // doit rester strictement materielle, sinon CPU0 peut rentrer dans le
    // scheduler sur une pile de boot pendant que les AP ne sont pas stables.
    if smp::bootstrap_in_progress() {
        crate::kernel::blackbox::timer_stage(blackbox_cpu, 99);
        return;
    }

'@
Replace-Once $timerPath $timerNeedle $timerReplacement "BOUCHAUD_SMP_BOOTSTRAP_GUARD_V1" | Out-Null

# 3) IPI reschedule : meme principe, un IPI recu pendant INIT/SIPI est acquitte,
# mais il ne doit jamais entrer dans le scheduler avant la fin du bootstrap.
$reschedPath = "src/arch/x86_64/idt/reschedule.rs"
$reschedNeedle = @'
extern "x86-interrupt" fn reschedule_interrupt_handler(stack: InterruptStackFrame) {
    let _gs = GsGuard::enter(&stack);
    let interrupted_user = from_user(&stack);
'@
$reschedReplacement = @'
extern "x86-interrupt" fn reschedule_interrupt_handler(stack: InterruptStackFrame) {
    let _gs = GsGuard::enter(&stack);
    // BOUCHAUD_SMP_BOOTSTRAP_GUARD_V1
    if smp::bootstrap_in_progress() {
        smp::eoi_local();
        return;
    }
    let interrupted_user = from_user(&stack);
'@
Replace-Once $reschedPath $reschedNeedle $reschedReplacement "BOUCHAUD_SMP_BOOTSTRAP_GUARD_V1" | Out-Null

# 4) Stage2 : ne pas lancer le thread net-lien avant que le SMP soit cable et que
# le scheduler soit explicitement libere. Le reseau peut etre sonde avant, mais
# le veilleur recurrent est decale juste apres SMP_WIRED.
$stage2Path = "src/platform/pc/stage2.rs"
$stage2Needle = @'
    crate::net::demarre_le_veilleur_de_lien();
'@
$stage2Replacement = @'
    crate::serial_println!("BOUCHAUD_NET_VEILLEUR_DIFFERE raison=smp-bootstrap");
'@
Replace-Once $stage2Path $stage2Needle $stage2Replacement "BOUCHAUD_NET_VEILLEUR_DIFFERE" | Out-Null

$stage2AfterNeedle = @'
    crate::serial_println!(
        "BOUCHAUD_STAGE2_SMP_WIRED_V31 online={} detected={}",
        crate::arch::x86_64::smp::schedulable_cpus(),
        crate::arch::x86_64::smp::discovered_cpus(),
    );

'@
$stage2Insertion = @'
    // BOUCHAUD_SMP_BOOTSTRAP_GUARD_V1
    // Le veilleur reseau est une tache noyau recurrente. Il demarre seulement
    // apres le cablage SMP, afin de ne pas publier/reveiller `net-lien` pendant
    // la fenetre INIT/SIPI qui a declenche la double faute Trigkey.
    crate::net::demarre_le_veilleur_de_lien();
    crate::serial_println!("BOUCHAUD_NET_VEILLEUR_APRES_SMP");

'@
Insert-After $stage2Path $stage2AfterNeedle $stage2Insertion "BOUCHAUD_NET_VEILLEUR_APRES_SMP" | Out-Null

Write-Host ""
Write-Host "Correctif applique. Sauvegarde : $backupRoot"
Write-Host ""
Write-Host "Verification conseillee :"
Write-Host "  git diff -- src/arch/x86_64/smp.rs src/arch/x86_64/idt/timer.rs src/arch/x86_64/idt/reschedule.rs src/platform/pc/stage2.rs"
Write-Host "  cargo +nightly clean"
Write-Host "  cargo +nightly bootimage"
Write-Host ""
Write-Host "Logs attendus sur Trigkey :"
Write-Host "  SMP_BOOT_GUARD_ENTER irq=off"
Write-Host "  SMP_AP_STACK_TABLE_OK ..."
Write-Host "  SMP4_STAGE sipi-1"
Write-Host "  SMP4_STAGE sipi-2"
Write-Host "  SMP_BOOT_GUARD_EXIT irq=restored"
Write-Host "  BOUCHAUD_STAGE2_SMP_WIRED_V31 ..."
Write-Host "  BOUCHAUD_NET_VEILLEUR_APRES_SMP"
