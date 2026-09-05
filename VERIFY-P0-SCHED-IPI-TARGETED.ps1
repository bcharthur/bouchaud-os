param(
    [switch]$Build
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$ThreadPath = "src\kernel\process\thread.rs"
$IdtPath    = "src\arch\x86_64\idt.rs"
$Marker     = "BOUCHAUD_P0_TARGETED_SCHED_IPI_V1"

$Utf8Strict = New-Object System.Text.UTF8Encoding($false, $true)

function Read-Utf8Strict([string]$Path) {
    return [System.IO.File]::ReadAllText((Resolve-Path -LiteralPath $Path).Path, $Utf8Strict)
}

# BOUCHAUD_C20_JETON_D_ENCODAGE_SANS_ENCODAGE
#
# Le jeton UTF-8 de controle et ses deux formes MOJIBAKE, construits par POINT
# DE CODE plutot qu'ecrits en clair.
#
# Ces trois chaines sont le SUJET du controle : elles disent que le source Rust
# porte bien l'identifiant `n<oe>uds` et qu'il n'a pas ete relu avec une page de
# codes ANSI. Les ecrire litteralement mettait donc dans ce fichier exactement
# les octets contre lesquels il met en garde -- et Windows PowerShell 5.1, qui
# decode un script sans BOM avec la page de codes ANSI, pouvait les abimer avant
# meme la comparaison. Un controle d'encodage ne doit pas dependre de son propre
# encodage.
$JetonNoeuds = "let n" + [char]0x0153 + "uds:"
# Ce que devient `<oe>` quand un fichier UTF-8 est relu en ANSI : deux octets,
# dont le premier se lit `A` rond en chef.
$MojibakeOe = "n" + [char]0x00C5
# Et ce que devient un tiret cadratin dans la meme conversion.
$MojibakeTiret = [char]0x00E2 + [char]0x20AC

Write-Host "=== Verification P0 targeted scheduler IPI v1.1 ===" -ForegroundColor Cyan

$thread = Read-Utf8Strict $ThreadPath
$idt = Read-Utf8Strict $IdtPath

if (-not $thread.Contains($Marker)) { throw "Marqueur P0 absent de thread.rs" }
if (-not $idt.Contains($Marker)) { throw "Marqueur P0 absent de idt.rs" }
if (-not $thread.Contains("pub fn running_user_cpu_mask() -> u64")) { throw "running_user_cpu_mask absent" }
if (-not $idt.Contains("smp::reschedule_cpu(cpu);")) { throw "IPI cible absent" }

if ($thread.Contains($MojibakeOe) -or $thread.Contains($MojibakeTiret)) {
    throw "Mojibake detecte dans thread.rs"
}
if (-not $thread.Contains($JetonNoeuds)) {
    throw "Token UTF-8 de controle '$JetonNoeuds' absent"
}
Write-Host "[OK] UTF-8 strict / aucun mojibake connu" -ForegroundColor Green
Write-Host "[OK] P0 targeted IPI present" -ForegroundColor Green

$timerStart = $idt.IndexOf('extern "x86-interrupt" fn timer_interrupt_handler')
$reschedStart = $idt.IndexOf('extern "x86-interrupt" fn reschedule_interrupt_handler', $timerStart)
if ($timerStart -lt 0 -or $reschedStart -lt 0) { throw "Handlers IDT introuvables" }
$timerBody = $idt.Substring($timerStart, $reschedStart - $timerStart)
if ($timerBody.Contains("smp::broadcast_reschedule();")) {
    throw "Broadcast periodique encore present dans le handler timer"
}
Write-Host "[OK] aucun broadcast periodique du PIT" -ForegroundColor Green

git diff --check
if ($LASTEXITCODE -ne 0) { throw "git diff --check a echoue" }

if ($Build) {
    Write-Host ""
    Write-Host "[BUILD] cargo check" -ForegroundColor Cyan
    cargo check
    if ($LASTEXITCODE -ne 0) { throw "cargo check a echoue" }

    Write-Host ""
    Write-Host "[BUILD] cargo bootimage" -ForegroundColor Cyan
    cargo bootimage
    if ($LASTEXITCODE -ne 0) { throw "cargo bootimage a echoue" }
}

Write-Host ""
Write-Host "[OK] validation terminee." -ForegroundColor Green
