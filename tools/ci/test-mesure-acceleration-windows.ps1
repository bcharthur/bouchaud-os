# BOUCHAUD_WHPX_CPU_V1 -- Mesure-Acceleration.ps1 EXECUTE sous Windows
# PowerShell 5.1, avec un faux QEMU qui tient la liaison serie comme le vrai
# (CreateFile ... FILE_SHARE_READ, chardev/char-file.c).
#
# Defauts constates sur Windows reel (journaux du 13/11) :
#   1. ReadAllText du journal serie pendant que QEMU l'ecrit : IOException
#      "en cours d'utilisation par un autre processus", le script s'arrete ;
#   2. QEMU reste alors orphelin, garde le bootimage ouvert, et le
#      `cargo bootimage` suivant echoue (llvm-objcopy : Permission denied).
#
# Ce test exige : le temoin (1) reproduit sous Windows, la lecture partagee
# qui le corrige, une mesure complete sur le faux QEMU, et aucun faux QEMU
# vivant a la fin. Le banc tourne dans une COPIE de tools\windows : il n'ecrit
# rien dans le target\ du depot (whpx-qualification.json).
$ErrorActionPreference = 'Stop'
$racine = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$echecs = 0
function Verifie([string]$Nom, [bool]$Condition, [string]$Detail = '') {
    if ($Condition) { Write-Host "ok    $Nom" } else { Write-Host "ECHEC $Nom $Detail"; $script:echecs++ }
}

$t = Join-Path ([System.IO.Path]::GetTempPath()) ("bo-mesure-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path (Join-Path $t 'tools\windows') -Force | Out-Null
Copy-Item (Join-Path $racine 'tools\windows\Mesure-Acceleration.ps1') (Join-Path $t 'tools\windows\')
Copy-Item (Join-Path $racine 'tools\windows\BouchaudQemu.psm1') (Join-Path $t 'tools\windows\')
Import-Module (Join-Path $t 'tools\windows\BouchaudQemu.psm1') -Force

# Temoin et correctif, dans ce processus.
$f = Join-Path $t 'tenu.log'
$ecrivain = New-Object System.IO.FileStream($f, [System.IO.FileMode]::Create, [System.IO.FileAccess]::Write, [System.IO.FileShare]::Read)
$o = [System.Text.Encoding]::ASCII.GetBytes("BOUCHAUD_CPU_ISA hyperviseur=MicrosoftHv`n")
$ecrivain.Write($o, 0, $o.Length); $ecrivain.Flush()
$temoin = $false
try { [void][System.IO.File]::ReadAllText($f) } catch [System.IO.IOException] { $temoin = $true }
Verifie 'temoin : ReadAllText refuse un fichier tenu par un ecrivain (defaut reproduit)' $temoin
$lu = Read-BouchaudJournal $f
Verifie 'lecture partagee pendant l''ecriture' ($lu -match 'MicrosoftHv') "lu=$lu"
$ecrivain.Dispose()

# Faux QEMU : ecrit la serie comme le noyau sous WHPX SMP1, puis reste vivant.
$src = @'
using System; using System.IO; using System.Threading;
public static class FauxQemu {
    public static void Main(string[] a) {
        string f = null;
        for (int i = 0; i + 1 < a.Length; i++) { if (a[i] == "-serial" && a[i + 1].StartsWith("file:")) { f = a[i + 1].Substring(5); } }
        using (var s = new FileStream(f, FileMode.Create, FileAccess.Write, FileShare.Read))
        using (var w = new StreamWriter(s)) {
            w.AutoFlush = true;
            w.WriteLine("BOUCHAUD_CPU_ISA hyperviseur=MicrosoftHv fournisseur=AuthenticAMD sse3=1 ssse3=1 sse41=1 sse42=1 popcnt=1 xsave=1 osxsave=0 avx=1 avx2=1 avx512f=1 mpx=0 apxf=0 la57=0");
            w.WriteLine("BOUCHAUD_SMP_DECOUVERTE count=1 source=madt madt_actifs=1");
            w.WriteLine("SMP4_AP_STARTED count=0 reason=single-vcpu");
            w.WriteLine("[kernel] SMP_NG2_SCHEDULER online=1 mode=UP");
            Thread.Sleep(600000);
        }
    }
}
'@
$faux = Join-Path $t 'faux-qemu.exe'
Add-Type -TypeDefinition $src -OutputAssembly $faux -OutputType ConsoleApplication
$boot = Join-Path $t 'boot.bin'
[System.IO.File]::WriteAllBytes($boot, (New-Object byte[] 512))

$sortie = try { & (Join-Path $t 'tools\windows\Mesure-Acceleration.ps1') -Qemu $faux -Bootimage $boot -Configurations @('whpx-smp1') -Repetitions 1 -DelaiS 60 *>&1 | Out-String } catch { "EXCEPTION $($_.Exception.GetType().FullName) $($_.Exception.Message)" }
Write-Host $sortie
Verifie 'mesure : repetition complete et prouvee WHPX' ($sortie -match 'MESURE whpx-smp1 #1 ok=True accel_reel=materiel signature=MicrosoftHv') ''
Verifie 'mesure : aucune exception' ($sortie -notmatch 'EXCEPTION|IOException|en cours d.utilisation|being used by another process') ''
Start-Sleep -Seconds 1
$vivants = @(Get-Process -Name 'faux-qemu' -ErrorAction SilentlyContinue)
Verifie 'mesure : aucun QEMU orphelin' ($vivants.Count -eq 0) "vivants=$($vivants.Count)"
$vivants | Stop-Process -Force -ErrorAction SilentlyContinue

# Interruption en cours de mesure : le finally arrete QEMU.
$job = Start-Job -ScriptBlock {
    param($script, $faux, $boot)
    & $script -Qemu $faux -Bootimage $boot -Configurations @('whpx-smp2') -Repetitions 1 -DelaiS 300 *> $null
} -ArgumentList (Join-Path $t 'tools\windows\Mesure-Acceleration.ps1'), $faux, $boot
$attente = [System.Diagnostics.Stopwatch]::StartNew()
while ($attente.Elapsed.TotalSeconds -lt 60 -and @(Get-Process -Name 'faux-qemu' -ErrorAction SilentlyContinue).Count -eq 0) { Start-Sleep -Milliseconds 250 }
Verifie 'interruption : le faux QEMU a demarre' (@(Get-Process -Name 'faux-qemu' -ErrorAction SilentlyContinue).Count -ge 1)
Stop-Job $job; Remove-Job $job -Force
Start-Sleep -Seconds 2
$vivants = @(Get-Process -Name 'faux-qemu' -ErrorAction SilentlyContinue)
Verifie 'interruption : aucun QEMU orphelin apres arret du script' ($vivants.Count -eq 0) "vivants=$($vivants.Count)"
$vivants | Stop-Process -Force -ErrorAction SilentlyContinue

Remove-Item -LiteralPath $t -Recurse -Force -ErrorAction SilentlyContinue
if ($echecs) { Write-Host "MESURE_ACCELERATION_WINDOWS_ECHEC echecs=$echecs"; exit 1 }
Write-Host 'MESURE_ACCELERATION_WINDOWS_OK'
