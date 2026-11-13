<#
.SYNOPSIS
    BOUCHAUD_PERF_V1 -- TCG contre WHPX, SMP1/2/4, sur CETTE machine, mesure.

.DESCRIPTION
    Demarre le MEME bootimage (celui que run.ps1 construit) sous QEMU, sans
    fenetre, pour chaque configuration et `-Repetitions` fois :

        tcg-smp1  tcg-smp4  whpx-smp1  whpx-smp2  whpx-smp4

    et releve sur la liaison serie (rien n'est deduit) :

      - le nombre de processeurs DECOUVERTS et sa source
        (`BOUCHAUD_SMP_DECOUVERTE ... source=madt|cpuid`) ;
      - les processeurs EN LIGNE (`BOUCHAUD_SMP_PHYSICAL_GREEN online=N`) ;
      - que CHAQUE coeur fait tourner des quantums
        (`BOUCHAUD_SMP_BATTEMENT cpu=K quantums=Q`, Q > 0) -- en ligne ne
        suffit pas, un coeur peut etre en ligne et muet ;
      - le temps mural jusqu'a l'ordonnanceur SMP (`SMP_NG2_SCHEDULER`) ;
      - avec -AvecLadybird (disque `ladybird-browser.img` fabrique par
        run.ps1) : premiere trame du bureau et premiere trame de Ladybird.

    Medianes et P95 par configuration, jamais melangees : TCG, WHPX, KVM et
    materiel reel sont des environnements DIFFERENTS.

    Une configuration WHPX SMPn est QUALIFIEE si toutes ses repetitions ont n
    coeurs en ligne qui battent tous, sans panique. La plus grande est ecrite
    dans target\whpx-qualification.json, liee a l'empreinte du bootimage :
    `run.ps1 -Accel auto` ne choisit WHPX en SMP que sur cette preuve.

    BOUCHAUD_WHPX_CPU_V1 : le modele CPU vient de BouchaudQemu.psm1 (TCG :
    max ; WHPX : qemu64 + SSSE3/SSE4.1/SSE4.2/POPCNT, jamais `max`, qui fait
    sortir le VP en code 4 sous QEMU 11). Une repetition n'est OK que si le
    CPUID publie par le noyau (`BOUCHAUD_CPU_ISA`) prouve l'accelerateur :
    signature TCGTCGTCGTCG pour TCG, toute autre signature pour WHPX ; et si
    l'erreur standard de QEMU (journalisee) ne contient aucun
    `WHPX: Unexpected VP exit`.

    NON QUALIFIE SUR WINDOWS par la session qui l'a ecrit (aucun Windows,
    aucun Hyper-V disponibles) : syntaxe et refus verifies sous pwsh Linux.

.EXAMPLE
    .\run.ps1 -Doom -Audio none           # construit le bootimage
    .\tools\windows\Mesure-Acceleration.ps1 -Repetitions 3
    .\tools\windows\Mesure-Acceleration.ps1 -Repetitions 3 -AvecLadybird -WhpxIrqchip on
#>
[CmdletBinding()]
param(
    [int]$Repetitions = 3,
    [string[]]$Configurations = @('tcg-smp1', 'tcg-smp4', 'whpx-smp1', 'whpx-smp2', 'whpx-smp4'),
    [ValidateSet('off', 'on')][string]$WhpxIrqchip = 'off',
    [int]$MemoireMio = 4096,
    [int]$DelaiS = 240,
    [switch]$AvecLadybird,
    [string]$Qemu = 'qemu-system-x86_64',
    [string]$Bootimage,
    [switch]$DryRun
)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'BouchaudQemu.psm1') -Force
$Racine = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$Boot = if ($Bootimage) { $Bootimage } else { Join-Path $Racine 'target\x86_64-bouchaud_os\debug\bootimage-bouchaud-os.bin' }
$Disque = Join-Path $Racine 'ladybird-browser.img'
if (-not (Test-Path -LiteralPath $Boot)) { throw "bootimage absent : $Boot (lancer run.ps1 une fois)" }
if ($AvecLadybird -and -not (Test-Path -LiteralPath $Disque)) { throw "disque Ladybird absent : $Disque (lancer run.ps1 -Ladybird une fois)" }
if ($Repetitions -lt 1) { throw "Repetitions >= 1" }
$Empreinte = (Get-FileHash -LiteralPath $Boot -Algorithm SHA256).Hash.ToLower()
$Travail = Join-Path ([System.IO.Path]::GetTempPath()) ("bouchaud-accel-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $Travail | Out-Null

function Quantile([double[]]$v, [double]$q) {
    if (-not $v -or $v.Count -eq 0) { return $null }
    $t = $v | Sort-Object
    $i = [math]::Ceiling($q * $t.Count) - 1
    return $t[[math]::Max(0, [math]::Min($t.Count - 1, $i))]
}

function Arguments([string]$conf, [string]$journal) {
    if ($conf -notmatch '^(tcg|whpx)-smp(\d+)$') { throw "configuration inconnue : $conf" }
    $accel = $Matches[1]; $n = [int]$Matches[2]
    $a = @('-machine', 'pc', '-m', "$MemoireMio", '-smp', "$n", '-display', 'none', '-no-reboot',
           '-drive', "format=raw,file=$Boot", '-serial', "file:$journal", '-nic', 'user,model=e1000')
    if ($accel -eq 'whpx') { $a += @('-accel', "whpx,kernel-irqchip=$WhpxIrqchip", '-cpu', (Get-BouchaudCpuQemu 'whpx')) }
    else { $a += @('-accel', 'tcg', '-cpu', (Get-BouchaudCpuQemu 'tcg')) }
    if ($AvecLadybird) { $a += @('-drive', "format=raw,file=$Disque,snapshot=on") }
    return @{ accel = $accel; n = $n; args = $a }
}

$resultats = @()
foreach ($conf in $Configurations) {
    for ($r = 1; $r -le $Repetitions; $r++) {
        $journal = Join-Path $Travail "$conf-$r.log"
        $c = Arguments $conf $journal
        if ($DryRun) { Write-Host ("PLAN {0} #{1} : {2} {3}" -f $conf, $r, $Qemu, ($c.args -join ' ')); continue }
        $chrono = [System.Diagnostics.Stopwatch]::StartNew()
        $erreurs = "$journal.qemu"
        $jalons = @{}
        $attendus = @('SMP_NG2_SCHEDULER')
        if ($AvecLadybird) { $attendus += @('bureau-premiere-trame', 'BOUCHAUD_UI_FIRST_FRAME onglet=1') }
        $p = $null
        # QEMU n'est JAMAIS laisse orphelin (erreur, Ctrl+C) : un QEMU restant
        # garde le bootimage ouvert et `cargo bootimage` echoue ensuite
        # (llvm-objcopy : Permission denied, constate sous Windows).
        try {
            $p = Start-Process -FilePath $Qemu -ArgumentList (ConvertTo-BouchaudLigneArguments $c.args) -PassThru -NoNewWindow -RedirectStandardError $erreurs
            while ($chrono.Elapsed.TotalSeconds -lt $DelaiS -and -not $p.HasExited) {
                Start-Sleep -Milliseconds 250
                $texte = Read-BouchaudJournal $journal
                foreach ($j in $attendus) {
                    if (-not $jalons.ContainsKey($j) -and $texte.Contains($j)) { $jalons[$j] = [math]::Round($chrono.Elapsed.TotalMilliseconds) }
                }
                if ($texte -match 'KERNEL PANIC') { break }
                # Sortie de VP WHPX : QEMU met la VM en pause et reste vivant.
                if ((Read-BouchaudJournal $erreurs) -match 'WHPX: Unexpected VP exit') { break }
                $battements = ([regex]::Matches($texte, 'BOUCHAUD_SMP_BATTEMENT cpu=\d+')).Count
                if ($jalons.Count -eq $attendus.Count -and ($battements -ge $c.n -or $c.n -eq 1)) { Start-Sleep -Seconds 2; break }
            }
        }
        finally {
            if ($p -and -not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue; $p.WaitForExit(10000) | Out-Null }
        }
        $texte = Read-BouchaudJournal $journal
        $enLigne = if ($texte -match 'BOUCHAUD_SMP_PHYSICAL_(GREEN|PARTIAL) online=(\d+)') { [int]$Matches[2] } elseif ($texte -match 'reason=single-vcpu') { 1 } else { 0 }
        $source = if ($texte -match 'BOUCHAUD_SMP_DECOUVERTE count=(\d+) source=(\w+)') { "$($Matches[1])/$($Matches[2])" } else { '?' }
        $muets = 0
        foreach ($m in [regex]::Matches($texte, 'BOUCHAUD_SMP_BATTEMENT cpu=(\d+) quantums=(\d+)')) { if ([int]$m.Groups[2].Value -eq 0) { $muets++ } }
        $battent = ([regex]::Matches($texte, 'BOUCHAUD_SMP_BATTEMENT cpu=\d+ quantums=[1-9]')).Count
        $panique = $texte -match 'KERNEL PANIC|DOUBLE FAULT|panicked at'
        $v = Get-BouchaudVerdictQemu -JournalSerie $journal -JournalErreurs $erreurs
        $accelProuve = if ($c.accel -eq 'tcg') { $v.accel_reel -eq 'TCG' } else { $v.accel_reel -eq 'materiel' }
        $ok = (-not $panique) -and $accelProuve -and ($null -eq $v.vp_exit) -and $enLigne -eq $c.n -and ($c.n -eq 1 -or ($battent -ge $c.n -and $muets -eq 0)) -and $jalons.ContainsKey('SMP_NG2_SCHEDULER')
        if ($AvecLadybird) { $ok = $ok -and $jalons.ContainsKey('BOUCHAUD_UI_FIRST_FRAME onglet=1') }
        $ligne = [pscustomobject]@{
            configuration = $conf; repetition = $r; ok = $ok; decouverte = $source; en_ligne = $enLigne
            accel_reel = $v.accel_reel; signature = $v.signature; vp_exit = $v.vp_exit
            sse41 = $(if ($v.isa) { $v.isa.sse41 } else { $null })
            coeurs_qui_battent = $battent; coeurs_muets = $muets; panique = [bool]$panique
            ordonnanceur_ms = $jalons['SMP_NG2_SCHEDULER']; bureau_ms = $jalons['bureau-premiere-trame']
            ladybird_premiere_trame_ms = $jalons['BOUCHAUD_UI_FIRST_FRAME onglet=1']
        }
        $resultats += $ligne
        Write-Host ("MESURE {0} #{1} ok={2} accel_reel={10} signature={11} vp_exit={12} decouverte={3} en_ligne={4} battent={5} muets={6} ordonnanceur_ms={7} bureau_ms={8} ladybird_ms={9}" -f `
            $conf, $r, $ok, $source, $enLigne, $battent, $muets, $ligne.ordonnanceur_ms, $ligne.bureau_ms, $ligne.ladybird_premiere_trame_ms, $v.accel_reel, $v.signature, $v.vp_exit)
    }
}
if ($DryRun) { Remove-Item -Recurse -Force $Travail; return }

Write-Host ""
Write-Host "| Configuration | OK | Ordonnanceur SMP p50 / p95 (ms) | Bureau p50 / p95 (ms) | Ladybird 1re trame p50 / p95 (ms) |"
Write-Host "|---|---|---|---|---|"
$qualifie = 0
foreach ($conf in $Configurations) {
    $l = @($resultats | Where-Object configuration -eq $conf)
    $okN = @($l | Where-Object ok).Count
    $f = { param($cle) $v = @($l | Where-Object { $_.$cle } | ForEach-Object { [double]$_.$cle }); if ($v.Count) { "{0} / {1}" -f (Quantile $v 0.5), (Quantile $v 0.95) } else { 'n/m' } }
    Write-Host ("| {0} | {1}/{2} | {3} | {4} | {5} |" -f $conf, $okN, $l.Count, (& $f 'ordonnanceur_ms'), (& $f 'bureau_ms'), (& $f 'ladybird_premiere_trame_ms'))
    if ($conf -match '^whpx-smp(\d+)$' -and $okN -eq $l.Count -and $l.Count -ge 1) { $qualifie = [math]::Max($qualifie, [int]$Matches[1]) }
}
New-Item -ItemType Directory -Force -Path (Join-Path $Racine 'target') | Out-Null
$resultats | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $Racine 'target\mesure-acceleration.json') -Encoding UTF8
if ($qualifie -ge 1) {
    [pscustomobject]@{ schema = 'bouchaud-whpx-qualification/1'; bootimage_sha256 = $Empreinte; smp_max_qualifie = $qualifie
        irqchip = $WhpxIrqchip; repetitions = $Repetitions; date = (Get-Date).ToString('s'); environnement = 'WHPX (Windows Hypervisor Platform), cette machine' } |
        ConvertTo-Json | Set-Content -LiteralPath (Join-Path $Racine 'target\whpx-qualification.json') -Encoding UTF8
    Write-Host ("WHPX_QUALIFIE smp_max={0} irqchip={1} -> run.ps1 -Accel auto l'utilisera pour ce noyau" -f $qualifie, $WhpxIrqchip) -ForegroundColor Green
}
else {
    Remove-Item -LiteralPath (Join-Path $Racine 'target\whpx-qualification.json') -ErrorAction SilentlyContinue
    Write-Host "WHPX_NON_QUALIFIE -> run.ps1 -Accel auto reste en TCG pour le SMP" -ForegroundColor Yellow
}
Write-Host "journaux : $Travail"
