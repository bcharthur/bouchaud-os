# BOUCHAUD_WHPX_CPU_V1 -- choix de l'accelerateur et du modele CPU, lancement
# de QEMU journalise, verdict lu dans les journaux (jamais suppose).
#
# Utilise par run.ps1 et tools\windows\Mesure-Acceleration.ps1 ; teste sous
# pwsh par tools/ci/test-scripts-windows-trigkey.ps1.
#
# Pourquoi deux modeles CPU :
#
#   TCG  : `max`. QEMU EMULE chaque instruction et leve #UD sur celles que le
#          modele n'annonce pas : sans `max` (modele qemu64), pas de SSE4.1.
#
#   WHPX : `qemu64,+ssse3,+sse4.1,+sse4.2,+popcnt`. Avec `max`, QEMU
#          11.0.50 configure le processeur virtuel avec des fonctions que
#          l'hote n'a pas -- MPX et APX ensemble sur un Ryzen qui n'a aucun
#          des deux ("conflicts with APX/MPX") -- puis
#          "WHPX: Unexpected VP exit code 4" (UnrecoverableException), aucun
#          journal noyau. Le modele retenu demarre (constate sur Windows,
#          Ryzen 7 7800X3D : WHPX SMP2, bureau et Ladybird).
#          CPUID REELLEMENT vu par l'invite sur cette machine (ligne
#          BOUCHAUD_CPU_ISA) : signature `MicrosoftHv`, sse41/sse42/popcnt=1,
#          et AVX, AVX2, AVX-512F annonces par Hyper-V lui-meme (feuille 7 de
#          l'hote, absente de qemu64) ; mpx=0 apxf=0. Le CPUID de l'invite
#          n'est donc PAS seulement celui du modele QEMU : seules certaines
#          feuilles passent par QEMU. Le materiel execute SSE4.1 de toute
#          facon : aucune commande de VMX ou de SVM ne fait lever #UD dessus.
#
# Le CPUID REEL vu par l'invite est publie par le noyau :
#   BOUCHAUD_CPU_ISA hyperviseur=... sse41=... mpx=... apxf=...
# et lu ici. `hyperviseur=TCGTCGTCGTCG` prouve un demarrage TCG : il n'est
# jamais presente comme un demarrage WHPX.

$script:CpuTcg = 'max'
$script:CpuWhpx = 'qemu64,+ssse3,+sse4.1,+sse4.2,+popcnt'

function Get-BouchaudCpuQemu([string]$Accel) {
    if ($Accel -eq 'whpx') { return $script:CpuWhpx }
    return $script:CpuTcg
}

# Les arguments d'acceleration, et ce qu'on annonce. Aucun repli silencieux :
# `-accel whpx -accel tcg` n'est plus jamais passe a QEMU. En `auto`, un
# echec d'initialisation de WHPX est detecte par l'appelant
# (`repli_tcg_explicite`), qui relance en TCG en le DISANT.
function Get-BouchaudAccelQemu {
    param(
        [string]$Accel,
        [int]$CpuCount,
        [bool]$Ladybird,
        [string]$WhpxIrqchip = 'off',
        [bool]$WhpxQualifie = $false
    )
    $whpx = @('-accel', "whpx,kernel-irqchip=$WhpxIrqchip", '-cpu', $script:CpuWhpx)
    $tcg = @('-accel', 'tcg')
    if ($Ladybird) { $tcg += @('-cpu', $script:CpuTcg) }
    $r = [ordered]@{ arguments = @(); accel = $Accel; libelle = ''; repli_tcg_explicite = $false }
    switch ($Accel) {
        'none' {
            $r.libelle = 'aucune acceleration explicite (defaut de QEMU)'
        }
        'auto' {
            if ($Ladybird -and $CpuCount -gt 1 -and -not $WhpxQualifie) {
                # BOUCHAUD_PERF_V1 : WHPX en SMP seulement sur preuve locale.
                $r.arguments = $tcg; $r.accel = 'tcg'
                $r.libelle = "TCG force pour SMP ($CpuCount vCPU) : WHPX non qualifie pour ce noyau -> tools\windows\Mesure-Acceleration.ps1"
            }
            else {
                $r.arguments = $whpx; $r.accel = 'whpx'; $r.repli_tcg_explicite = $true
                $r.libelle = "WHPX SMP$CpuCount, cpu $($script:CpuWhpx) (si WHPX ne s'initialise pas : relance annoncee en TCG)"
            }
        }
        'whpx' {
            $r.arguments = $whpx
            $r.libelle = "WHPX SMP$CpuCount, cpu $($script:CpuWhpx), sans repli"
        }
        'tcg' {
            $r.arguments = $tcg
            $r.libelle = "TCG SMP$CpuCount" + $(if ($Ladybird) { ", cpu $($script:CpuTcg)" } else { '' })
        }
        default {
            $r.arguments = @('-accel', $Accel)
            $r.libelle = "$Accel (non qualifie)"
        }
    }
    return [pscustomobject]$r
}

# Une ligne de commande Windows (regles de CommandLineToArgvW / CRT) : chaque
# argument revient intact, espaces, guillemets et barres obliques compris.
function ConvertTo-BouchaudLigneArguments([string[]]$Arguments) {
    $morceaux = foreach ($a in $Arguments) {
        if ($null -eq $a -or $a -eq '') { '""'; continue }
        if ($a -notmatch '[\s"]') { $a; continue }
        $sb = New-Object System.Text.StringBuilder
        [void]$sb.Append('"')
        $barres = 0
        foreach ($c in $a.ToCharArray()) {
            if ($c -eq '\') { $barres++; continue }
            if ($c -eq '"') { [void]$sb.Append('\' * (2 * $barres + 1)); [void]$sb.Append('"') }
            else { [void]$sb.Append('\' * $barres); [void]$sb.Append($c) }
            $barres = 0
        }
        [void]$sb.Append('\' * (2 * $barres))
        [void]$sb.Append('"')
        $sb.ToString()
    }
    return ($morceaux -join ' ')
}

# Lance QEMU : entree et sortie standard HERITEES (la liaison serie `stdio`
# reste interactive), erreur standard recopiee EN DIRECT sur la console et
# dans $JournalErreurs. Rend le code de sortie de QEMU.
function Invoke-BouchaudQemu {
    param([string]$Exe, [string[]]$Arguments, [string]$JournalErreurs)
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $Exe
    $psi.Arguments = ConvertTo-BouchaudLigneArguments $Arguments
    $psi.UseShellExecute = $false
    $psi.RedirectStandardError = $true
    $p = New-Object System.Diagnostics.Process
    $p.StartInfo = $psi
    $w = New-Object System.IO.StreamWriter($JournalErreurs, $false)
    $w.AutoFlush = $true
    $demarre = $false
    try {
        [void]$p.Start()
        $demarre = $true
        $lecture = $p.StandardError.ReadLineAsync()
        while ($true) {
            if (-not $lecture.Wait(250)) { continue }
            $ligne = $lecture.Result
            if ($null -eq $ligne) { break }
            $w.WriteLine($ligne)
            [Console]::Error.WriteLine($ligne)
            $lecture = $p.StandardError.ReadLineAsync()
        }
        $p.WaitForExit()
        return $p.ExitCode
    }
    finally {
        $w.Dispose()
        # Interruption de PowerShell : QEMU ne reste pas orphelin.
        if ($demarre -and -not $p.HasExited) { try { $p.Kill() } catch { } }
    }
}

# Lecture PARTAGEE : QEMU garde la liaison serie ouverte en ecriture
# (CreateFile avec FILE_SHARE_READ, chardev/char-file.c) ; ReadAllText refuse
# alors d'ouvrir le fichier ("en cours d'utilisation par un autre processus",
# constate sous Windows). FileShare ReadWrite accepte un ecrivain present.
function Read-BouchaudJournal([string]$Chemin) {
    if (-not $Chemin -or -not (Test-Path -LiteralPath $Chemin)) { return '' }
    $fs = New-Object System.IO.FileStream($Chemin, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, ([System.IO.FileShare]::ReadWrite -bor [System.IO.FileShare]::Delete))
    try {
        $sr = New-Object System.IO.StreamReader($fs)
        $t = $sr.ReadToEnd()
    }
    finally { $fs.Dispose() }
    return [regex]::Replace($t, "$([char]27)\[[0-9;]*m", '')
}

# Ce que les journaux PROUVENT, sans rien supposer.
function Get-BouchaudVerdictQemu {
    param([string]$JournalSerie, [string]$JournalErreurs)
    $serie = Read-BouchaudJournal $JournalSerie
    $err = Read-BouchaudJournal $JournalErreurs
    $v = [ordered]@{
        noyau = $false; signature = $null; accel_reel = 'inconnu'; isa = $null
        vp_exit = $null; whpx_indisponible = $false; conflit_mpx_apx = $false
    }
    $m = [regex]::Match($serie, 'BOUCHAUD_CPU_ISA hyperviseur=(\S+) fournisseur=(\S+)((?: [a-z0-9]+=\d)+)')
    if ($m.Success) {
        $v.noyau = $true
        $v.signature = $m.Groups[1].Value
        $isa = [ordered]@{}
        foreach ($c in [regex]::Matches($m.Groups[3].Value, '([a-z0-9]+)=(\d)')) { $isa[$c.Groups[1].Value] = [int]$c.Groups[2].Value }
        $v.isa = [pscustomobject]$isa
        $v.accel_reel = switch ($v.signature) {
            'TCGTCGTCGTCG' { 'TCG' }
            'aucun' { 'aucun-hyperviseur' }
            default { 'materiel' }
        }
    }
    $e = [regex]::Match($err, 'WHPX: Unexpected VP exit code (\d+)')
    if ($e.Success) { $v.vp_exit = [int]$e.Groups[1].Value }
    $v.whpx_indisponible = $err -match '(?i)failed to initiali[sz]e whpx|WHPX: No accelerator found|invalid accelerator whpx|accelerator whpx (is )?not'
    $v.conflit_mpx_apx = $err -match 'conflicts with (APX|MPX)'
    return [pscustomobject]$v
}

# Le resume imprime apres QEMU, et le code a rendre (0 si rien d'anormal).
function Write-BouchaudVerdictQemu {
    param($Verdict, [string]$AccelDemande, [string]$JournalSerie, [string]$JournalErreurs)
    Write-Host ''
    Write-Host '=== Verdict QEMU (BOUCHAUD_WHPX_CPU_V1) ===' -ForegroundColor DarkGray
    Write-Host ("accel demandee : {0}" -f $AccelDemande)
    if ($Verdict.noyau) {
        $isa = ($Verdict.isa.PSObject.Properties | ForEach-Object { "$($_.Name)=$($_.Value)" }) -join ' '
        Write-Host ("accel reelle   : {0} (signature CPUID {1})" -f $Verdict.accel_reel, $Verdict.signature)
        Write-Host ("CPUID invite   : {0}" -f $isa)
    }
    else {
        Write-Host 'accel reelle   : inconnue (aucune ligne BOUCHAUD_CPU_ISA : le noyau n''a rien ecrit)' -ForegroundColor Yellow
    }
    if ($JournalSerie) { Write-Host ("journal serie  : {0}" -f $JournalSerie) }
    Write-Host ("journal QEMU   : {0}" -f $JournalErreurs)
    $code = 0
    if ($Verdict.conflit_mpx_apx) {
        Write-Host 'QEMU a active MPX et APX ensemble : modele CPU incompatible avec cet accelerateur.' -ForegroundColor Red
    }
    if ($null -ne $Verdict.vp_exit) {
        Write-Host ("WHPX_ECHEC vp_exit={0} : le processeur virtuel s'est arrete sur une erreur irrecuperable. Joindre les deux journaux ; relancer en TCG : -Accel tcg." -f $Verdict.vp_exit) -ForegroundColor Red
        $code = 3
    }
    if ($AccelDemande -eq 'whpx' -and $Verdict.accel_reel -eq 'TCG') {
        Write-Host 'ATTENTION : ce demarrage a tourne sous TCG, PAS sous WHPX.' -ForegroundColor Red
        $code = 4
    }
    if ($AccelDemande -eq 'whpx' -and $Verdict.whpx_indisponible) {
        Write-Host 'WHPX_INDISPONIBLE : QEMU n''a pas pu initialiser WHPX (voir journal QEMU).' -ForegroundColor Red
        if ($code -eq 0) { $code = 5 }
    }
    if ($AccelDemande -eq 'whpx' -and $Verdict.noyau -and $Verdict.isa.sse41 -ne 1) {
        Write-Host 'CPUID invite sans SSE4.1 : Ladybird choisira ses variantes SSE2.' -ForegroundColor Yellow
    }
    return $code
}

Export-ModuleMember -Function Read-BouchaudJournal, Get-BouchaudCpuQemu, Get-BouchaudAccelQemu, ConvertTo-BouchaudLigneArguments, Invoke-BouchaudQemu, Get-BouchaudVerdictQemu, Write-BouchaudVerdictQemu
