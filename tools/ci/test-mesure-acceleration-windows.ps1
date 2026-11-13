# BOUCHAUD_WHPX_CPU_V1 -- Mesure-Acceleration.ps1 EXECUTE sous Windows
# PowerShell 5.1, avec un faux QEMU qui tient la liaison serie comme le vrai
# (CreateFile ... FILE_SHARE_READ, chardev/char-file.c).
#
# Defauts constates sur Windows reel (journaux du 13/11) :
#   1. ReadAllText du journal serie pendant que QEMU l'ecrit : IOException
#      "en cours d'utilisation par un autre processus", le script s'arrete ;
#   2. QEMU reste alors orphelin, garde le bootimage ouvert, et le
#      `cargo bootimage` suivant echoue (llvm-objcopy : Permission denied).
#   3. (15 essais WHPX/TCG du 13/11, tous ok=False) le banc exigeait la
#      premiere trame d'un Ladybird que rien ne lancait : l'autorun normal
#      n'exporte pas BO_AUTOSTART_BROWSER, et -display none n'a pas de souris.
#      Qualifications A (SMP) et B (Ladybird, copie de banc du disque avec
#      lancement automatique) desormais separees.
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
Copy-Item (Join-Path $racine 'tools\windows\disque-banc-ladybird.py') (Join-Path $t 'tools\windows\')
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

# Faux QEMU : ecrit la serie comme le noyau sous WHPX pour `-smp N` (A), et
# les jalons de Ladybird SEULEMENT si le second disque contient vraiment
# `BO_AUTOSTART_BROWSER=1` (B) ; puis reste vivant, comme QEMU.
# FAUX_QEMU_MODE : `sans_trame` (navigateur lance, aucune trame), `muet`.
$src = @'
using System; using System.IO; using System.Text; using System.Threading;
public static class FauxQemu {
    static string Fichier(string drive) {
        int i = drive.IndexOf("file="); if (i < 0) { return null; }
        string f = drive.Substring(i + 5); int c = f.IndexOf(','); return c < 0 ? f : f.Substring(0, c);
    }
    public static void Main(string[] a) {
        string serie = null; int n = 1; int drives = 0; bool autostart = false;
        string mode = Environment.GetEnvironmentVariable("FAUX_QEMU_MODE") ?? "";
        for (int i = 0; i + 1 < a.Length; i++) {
            if (a[i] == "-serial" && a[i + 1].StartsWith("file:")) { serie = a[i + 1].Substring(5); }
            if (a[i] == "-smp") { n = int.Parse(a[i + 1]); }
            if (a[i] == "-drive") {
                drives++;
                string f = Fichier(a[i + 1]);
                if (drives == 2 && f != null && File.Exists(f)) {
                    using (var d = new FileStream(f, FileMode.Open, FileAccess.Read, FileShare.ReadWrite)) {
                        var b = new byte[d.Length]; d.Read(b, 0, b.Length);
                        autostart = Encoding.ASCII.GetString(b).Contains("BO_AUTOSTART_BROWSER=1");
                    }
                }
            }
        }
        using (var s = new FileStream(serie, FileMode.Create, FileAccess.Write, FileShare.Read))
        using (var w = new StreamWriter(s)) {
            w.AutoFlush = true;
            w.WriteLine("BOUCHAUD_BOOT_POINT kernel-main t_ms=0 delta_ms=0");
            w.WriteLine("BOUCHAUD_CPU_ISA hyperviseur=MicrosoftHv fournisseur=AuthenticAMD sse3=1 ssse3=1 sse41=1 sse42=1 popcnt=1 xsave=1 osxsave=0 avx=1 avx2=1 avx512f=1 mpx=0 apxf=0 la57=0");
            if (mode != "muet") {
                w.WriteLine("BOUCHAUD_SMP_DECOUVERTE count=" + n + " source=madt madt_actifs=" + n);
                if (n > 1) { w.WriteLine("BOUCHAUD_SMP_PHYSICAL_GREEN online=" + n + " reported=" + n); }
                else { w.WriteLine("[kernel] SMP4_AP_STARTED count=0 reason=single-vcpu"); w.WriteLine("[kernel] SMP4_SCHEDULER online=1 mode=UP"); }
                w.WriteLine("[kernel] SMP_NG2_SCHEDULER online=" + n + " mode=thread-load-balance+work-steal quantum=4ms");
                for (int c = 0; c < n; c++) { w.WriteLine("BOUCHAUD_SMP_BATTEMENT cpu=" + c + " quantums=50 fenetre_ms=200 bat=1"); }
                w.WriteLine("BOUCHAUD_SMP_BATTEMENT_BILAN battants=" + n + " en_ligne=" + n + " mode=lapic-periodique");
                if (drives >= 2) { w.WriteLine("BOUCHAUD_BOOT_POINT bureau-premiere-trame t_ms=4522 delta_ms=4462"); }
                if (autostart) {
                    Thread.Sleep(300);
                    w.WriteLine("[ladybird-bouchaud] BROWSER_HOST_START architecture=UI/Bouchaud");
                    if (mode == "https_avant_trame") {
                        // Ordre inverse, possible : on_load_finish avant present().
                        Thread.Sleep(300);
                        w.WriteLine("[LB:NAV] onglet=1 document_charge url=https://example.com/");
                        Thread.Sleep(3500);
                        w.WriteLine("BOUCHAUD_UI_FIRST_FRAME onglet=1 page=1 zone=1100x538 source=compositor");
                    }
                    else if (mode != "sans_trame") {
                        Thread.Sleep(300);
                        w.WriteLine("BOUCHAUD_UI_FIRST_FRAME onglet=1 page=1 zone=1100x538 source=compositor");
                        w.WriteLine("[LB:NAV] onglet=1 document_charge url=https://example.com/");
                    }
                }
            }
            Thread.Sleep(600000);
        }
    }
}
'@
$faux = Join-Path $t 'faux-qemu.exe'
Add-Type -TypeDefinition $src -OutputAssembly $faux -OutputType ConsoleApplication
$boot = Join-Path $t 'boot.bin'
[System.IO.File]::WriteAllBytes($boot, (New-Object byte[] 512))
$mesure = Join-Path $t 'tools\windows\Mesure-Acceleration.ps1'
function Lance-Mesure([hashtable]$P) {
    try { return (& $mesure @P *>&1 | Out-String) } catch { return "EXCEPTION $($_.Exception.GetType().FullName) $($_.Exception.Message)" }
}
function Aucun-Orphelin([string]$Nom) {
    Start-Sleep -Seconds 1
    $vivants = @(Get-Process -Name 'faux-qemu' -ErrorAction SilentlyContinue)
    Verifie "$Nom : aucun QEMU orphelin" ($vivants.Count -eq 0) "vivants=$($vivants.Count)"
    $vivants | Stop-Process -Force -ErrorAction SilentlyContinue
}

# A seule : WHPX SMP1 et SMP4, Ladybird non demande -> NOT_RUN, jamais ECHEC.
$env:FAUX_QEMU_MODE = ''
$sortie = Lance-Mesure @{ Qemu = $faux; Bootimage = $boot; Configurations = @('whpx-smp1', 'whpx-smp4'); Repetitions = 1; DelaiS = 60 }
Write-Host $sortie
Verifie 'A : WHPX SMP1 qualifie (battement monocoeur), Ladybird NOT_RUN' ($sortie -match 'MESURE whpx-smp1 #1 smp=OK accel_reel=materiel signature=MicrosoftHv vp_exit= decouverte=1/madt en_ligne=1 battent=1 muets=0 ladybird=NOT_RUN')
Verifie 'A : WHPX SMP4 qualifie, Ladybird NOT_RUN' ($sortie -match 'MESURE whpx-smp4 #1 smp=OK .* en_ligne=4 battent=4 muets=0 ladybird=NOT_RUN')
Verifie 'A : bilan WHPX_SMP_QUALIFIE smp_max=4 et LADYBIRD_WHPX_NOT_RUN' ($sortie -match 'WHPX_SMP_QUALIFIE smp_max=4' -and $sortie -match 'LADYBIRD_WHPX_NOT_RUN')
Verifie 'A : aucune exception de lecture' ($sortie -notmatch 'EXCEPTION|IOException|en cours d.utilisation|being used by another process')
Aucun-Orphelin 'A'

# B : disque Ladybird d'origine (autorun SANS autostart) -> copie de banc.
$scen = Join-Path $t 'scen'; New-Item -ItemType Directory -Force -Path $scen | Out-Null
[System.IO.File]::WriteAllText((Join-Path $scen 'autorun'), "uname`nexport BOUCHAUD_M9_URL='https://example.com'`ngui-surface simple`ndesktop`n")
$disque = Join-Path $t 'ladybird-browser.img'
$py = Get-BouchaudPython
& $py[0] @($py | Select-Object -Skip 1) -c "import tarfile,sys; t=tarfile.open(sys.argv[1],'w',format=tarfile.USTAR_FORMAT); t.add(sys.argv[2]+'/autorun',arcname='./autorun'); t.close()" $disque $scen
$avant = (Get-FileHash -LiteralPath $disque -Algorithm SHA256).Hash
$sortie = Lance-Mesure @{ Qemu = $faux; Bootimage = $boot; Configurations = @('whpx-smp4'); Repetitions = 1; DelaiS = 60; AvecLadybird = $true }
Write-Host $sortie
Verifie 'B : navigateur lance par la copie de banc, 1re trame et HTTPS -> ladybird=OK' ($sortie -match 'MESURE whpx-smp4 #1 smp=OK .* ladybird=OK')
Verifie 'B : bilan LADYBIRD_WHPX_QUALIFIE smp_max=4' ($sortie -match 'LADYBIRD_WHPX_QUALIFIE smp_max=4')
$q = Get-Content -LiteralPath (Join-Path $t 'target\whpx-qualification.json') -Raw | ConvertFrom-Json
Verifie 'B : preuve run.ps1 avec ladybird_smp_max_qualifie=4' ([int]$q.ladybird_smp_max_qualifie -eq 4 -and [int]$q.smp_max_qualifie -eq 4)
Verifie 'B : disque d''origine intact' ($avant -eq (Get-FileHash -LiteralPath $disque -Algorithm SHA256).Hash)
Verifie 'B : copie de banc supprimee apres le banc' (@(Get-ChildItem -Path ([System.IO.Path]::GetTempPath()) -Directory -Filter 'bouchaud-accel-*' | ForEach-Object { Get-ChildItem -LiteralPath $_.FullName -Filter 'ladybird-banc.img' }).Count -eq 0)
Aucun-Orphelin 'B'

# B, ordre inverse : document HTTPS 3,5 s AVANT la premiere trame -> OK, pas
# de faux `premiere-trame-absente` (revue Codex de #317).
$env:FAUX_QEMU_MODE = 'https_avant_trame'
$sortie = Lance-Mesure @{ Qemu = $faux; Bootimage = $boot; Configurations = @('whpx-smp4'); Repetitions = 1; DelaiS = 60; AvecLadybird = $true }
Write-Host $sortie
Verifie 'B ordre inverse : HTTPS avant la 1re trame -> ladybird=OK' ($sortie -match 'MESURE whpx-smp4 #1 smp=OK .* ladybird=OK')
Aucun-Orphelin 'B ordre inverse'

# B en echec : navigateur lance, aucune trame -> ladybird=ECHEC nomme, A reste OK.
$env:FAUX_QEMU_MODE = 'sans_trame'
$sortie = Lance-Mesure @{ Qemu = $faux; Bootimage = $boot; Configurations = @('whpx-smp4'); Repetitions = 1; DelaiS = 10; AvecLadybird = $true }
Write-Host $sortie
Verifie 'B echec : smp=OK, ladybird=ECHEC premiere-trame-absente' ($sortie -match 'MESURE whpx-smp4 #1 smp=OK .* ladybird=ECHEC .*premiere-trame-absente')
Verifie 'B echec : LADYBIRD_WHPX_NON_QUALIFIE mais WHPX_SMP_QUALIFIE' ($sortie -match 'LADYBIRD_WHPX_NON_QUALIFIE' -and $sortie -match 'WHPX_SMP_QUALIFIE smp_max=4')
Aucun-Orphelin 'B echec'

# Interruption en cours de mesure : le finally arrete QEMU.
$env:FAUX_QEMU_MODE = 'muet'
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
# Code explicite : sans commande native, $LASTEXITCODE resterait $null et
# l'etape (`if ($LASTEXITCODE -ne 0)`) echouerait sur un succes.
exit 0
