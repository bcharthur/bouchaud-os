<#
.SYNOPSIS
    BOUCHAUD_PERF_V1 -- TCG contre WHPX, SMP1/2/4, sur CETTE machine, mesure.

.DESCRIPTION
    Demarre le MEME bootimage (celui que run.ps1 construit) sous QEMU, sans
    fenetre, pour chaque configuration et `-Repetitions` fois :

        tcg-smp1  tcg-smp4  whpx-smp1  whpx-smp2  whpx-smp4

    et rend DEUX qualifications independantes (BOUCHAUD_WHPX_CPU_V1) :

    A -- WHPX/SMP, lue sur la liaison serie (rien n'est deduit) :
      - l'accelerateur REEL (signature CPUID `BOUCHAUD_CPU_ISA`) ;
      - les processeurs DECOUVERTS (`BOUCHAUD_SMP_DECOUVERTE count=N`) ;
      - les processeurs EN LIGNE (`BOUCHAUD_SMP_PHYSICAL_GREEN online=N`,
        ou `SMP4_SCHEDULER online=1 mode=UP` en monocoeur) ;
      - que CHAQUE coeur bat (`BOUCHAUD_SMP_BATTEMENT cpu=K quantums=Q`,
        Q > 0) -- aussi en SMP1, le noyau la mesure desormais sur un coeur ;
      - l'ordonnanceur (`SMP_NG2_SCHEDULER`), ni panique ni sortie de VP.
      Une application non lancee n'entre JAMAIS dans A.

    B -- Ladybird (-AvecLadybird) : sur une COPIE de banc de
      ladybird-browser.img dont l'autorun lance le navigateur
      (`BO_AUTOSTART_BROWSER=1`, tools\windows\disque-banc-ladybird.py ; le
      disque d'origine et scenario-m9 ne sont pas touches) : lancement
      (`BROWSER_HOST_START`), premiere trame (`BOUCHAUD_UI_FIRST_FRAME
      onglet=1`) et, SEPAREMENT, document HTTPS charge (`[LB:NAV] onglet=1
      document_charge url=https://`). Statuts : OK, ECHEC (cause), NOT_RUN
      (non demande), NON_MESURABLE (cause : disque absent, Python absent,
      autorun inconnu).

    Metriques separees : demarrage (hote -> noyau, noyau -> bureau selon
    l'horloge de l'invite, hote -> bureau), navigation (lancement -> 1re
    trame, lancement -> HTTPS), composition (`[LB:PERF]`), charge CPU
    (`[PROC-STAT]`) ; latence d'entree : NON_MESURABLE (aucune entree
    injectee). Medianes et P95 par configuration, jamais melangees.

    target\whpx-qualification.json (lie a l'empreinte du bootimage) porte
    `smp_max_qualifie` (A) et `ladybird_smp_max_qualifie` (A et B) :
    `run.ps1 -Accel auto` ne choisit WHPX pour Ladybird en SMP que sur B.

    BOUCHAUD_WHPX_CPU_V1 : le modele CPU vient de BouchaudQemu.psm1 (TCG :
    max ; WHPX : qemu64 + SSSE3/SSE4.1/SSE4.2/POPCNT, jamais `max`, qui fait
    sortir le VP en code 4 sous QEMU 11). Une repetition n'est OK que si le
    CPUID publie par le noyau (`BOUCHAUD_CPU_ISA`) prouve l'accelerateur :
    signature TCGTCGTCGTCG pour TCG, toute autre signature pour WHPX ; et si
    l'erreur standard de QEMU (journalisee) ne contient aucun
    `WHPX: Unexpected VP exit`.

    Execute sous Windows PowerShell 5.1 en CI avec un faux QEMU
    (tools/ci/test-mesure-acceleration-windows.ps1) ; WHPX reel : sur la
    machine de l'utilisateur seulement.

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
if ($Repetitions -lt 1) { throw "Repetitions >= 1" }
$Empreinte = (Get-FileHash -LiteralPath $Boot -Algorithm SHA256).Hash.ToLower()
$Travail = Join-Path ([System.IO.Path]::GetTempPath()) ("bouchaud-accel-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $Travail | Out-Null

# B -- Ladybird : une COPIE de banc du disque, dont l'autorun lance le
# navigateur (BO_AUTOSTART_BROWSER=1). ladybird-browser.img et scenario-m9 ne
# sont jamais modifies. Sans -AvecLadybird : NOT_RUN ; copie impossible :
# NON_MESURABLE, avec sa cause.
$Ladybird = @{ statut = 'NOT_RUN'; raison = '-AvecLadybird absent'; chemin = $null }
if ($AvecLadybird) {
    if ($DryRun) { $Ladybird = @{ statut = 'PRET'; raison = 'simulation'; chemin = (Join-Path $Travail 'ladybird-banc.img') } }
    else { $Ladybird = New-BouchaudDisqueBanc -Source $Disque -Cible (Join-Path $Travail 'ladybird-banc.img') }
    Write-Host ("Ladybird   : {0} ({1})" -f $Ladybird.statut, $Ladybird.raison)
}

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
    if ($Ladybird.statut -eq 'PRET') { $a += @('-drive', "format=raw,file=$($Ladybird.chemin),snapshot=on") }
    return @{ accel = $accel; n = $n; args = $a }
}

$resultats = @()
$JalonsBanc = Get-BouchaudJalonsBanc
try {
foreach ($conf in $Configurations) {
    for ($r = 1; $r -le $Repetitions; $r++) {
        $journal = Join-Path $Travail "$conf-$r.log"
        $c = Arguments $conf $journal
        if ($DryRun) { Write-Host ("PLAN {0} #{1} : {2} {3}" -f $conf, $r, $Qemu, ($c.args -join ' ')); continue }
        $chrono = [System.Diagnostics.Stopwatch]::StartNew()
        $erreurs = "$journal.qemu"
        $jalons = @{}
        # Fin d'une repetition : A prouvee (bilan des battements) et, si
        # Ladybird est demande, document HTTPS charge -- ou le delai.
        $fin = @('battement')
        if ($Ladybird.statut -eq 'PRET') { $fin += 'https' }
        $p = $null
        # QEMU n'est JAMAIS laisse orphelin (erreur, Ctrl+C) : un QEMU restant
        # garde le bootimage ouvert et `cargo bootimage` echoue ensuite
        # (llvm-objcopy : Permission denied, constate sous Windows).
        try {
            $p = Start-Process -FilePath $Qemu -ArgumentList (ConvertTo-BouchaudLigneArguments $c.args) -PassThru -NoNewWindow -RedirectStandardError $erreurs
            while ($chrono.Elapsed.TotalSeconds -lt $DelaiS -and -not $p.HasExited) {
                Start-Sleep -Milliseconds 250
                $texte = Read-BouchaudJournal $journal
                foreach ($k in $JalonsBanc.Keys) {
                    if (-not $jalons.ContainsKey($k) -and $texte.Contains($JalonsBanc[$k])) { $jalons[$k] = [math]::Round($chrono.Elapsed.TotalMilliseconds) }
                }
                if ($texte -match 'KERNEL PANIC') { break }
                # Sortie de VP WHPX : QEMU met la VM en pause et reste vivant.
                if ((Read-BouchaudJournal $erreurs) -match 'WHPX: Unexpected VP exit') { break }
                if (@($fin | Where-Object { -not $jalons.ContainsKey($_) }).Count -eq 0) { Start-Sleep -Seconds 2; break }
            }
        }
        finally {
            if ($p -and -not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue; $p.WaitForExit(10000) | Out-Null }
        }
        $a = Get-BouchaudAnalyseRepetition -Serie (Read-BouchaudJournal $journal) -Erreurs (Read-BouchaudJournal $erreurs) `
            -Accel $c.accel -N $c.n -Jalons $jalons -LadybirdStatut $Ladybird.statut -LadybirdRaison $Ladybird.raison
        $ligne = [pscustomobject][ordered]@{ configuration = $conf; repetition = $r; journal = $journal; analyse = $a }
        $resultats += $ligne
        Write-Host ("MESURE {0} #{1} smp={2} accel_reel={3} signature={4} vp_exit={5} decouverte={6}/{7} en_ligne={8} battent={9} muets={10} ladybird={11} causes_smp=[{12}] causes_ladybird=[{13}]" -f `
            $conf, $r, $a.smp.statut, $a.smp.accel_reel, $a.smp.signature, $a.smp.vp_exit, $a.smp.decouverts, $a.smp.source, $a.smp.en_ligne,
            $a.smp.coeurs_qui_battent, $a.smp.coeurs_muets, $a.ladybird.statut, ($a.smp.causes -join ','), ($a.ladybird.causes -join ','))
        Write-Host ("  demarrage hote->noyau={0} hote->ordonnanceur={1} hote->bureau={2} noyau->bureau(invite)={3} | navigation lancement->1re_trame={4} lancement->https={5} | presentation_us moy={6} pire={7} | cpu={8}% | serie={9} o" -f `
            $a.demarrage.hote_jusqu_au_noyau_ms, $a.demarrage.hote_jusqu_a_l_ordonnanceur_ms, $a.demarrage.hote_jusqu_au_bureau_ms, $a.demarrage.noyau_jusqu_au_bureau_ms_invite,
            $a.navigation.lancement_premiere_trame_ms, $a.navigation.lancement_document_https_ms, $a.composition.present_us_moy, $a.composition.present_us_pire,
            $a.charge_cpu.occupation_pct, $a.octets_serie)
    }
}
}
finally {
    # La copie de banc du disque est grosse et jetable ; les journaux restent.
    if ($Ladybird.chemin -and (Test-Path -LiteralPath $Ladybird.chemin)) { Remove-Item -LiteralPath $Ladybird.chemin -Force -ErrorAction SilentlyContinue }
}
if ($DryRun) { Remove-Item -Recurse -Force $Travail; return }

function Resume-Mesure([object[]]$l, [scriptblock]$valeur) {
    $v = @($l | ForEach-Object { & $valeur $_ } | Where-Object { $null -ne $_ } | ForEach-Object { [double]$_ })
    if ($v.Count) { return "{0} / {1} ({2})" -f (Quantile $v 0.5), (Quantile $v 0.95), $v.Count }
    return 'n/m'
}

Write-Host ""
Write-Host "A -- Qualification WHPX/SMP (accelerateur reel, CPU decouverts, en ligne, battants, ordonnanceur, sans panique)"
Write-Host "| Configuration | A OK | Ordonnanceur, hote p50 / p95 (n) ms | Causes |"
Write-Host "|---|---|---|---|"
$qualifieSmp = 0
foreach ($conf in $Configurations) {
    $l = @($resultats | Where-Object configuration -eq $conf)
    $okN = @($l | Where-Object { $_.analyse.smp.statut -eq 'OK' }).Count
    $causes = (@($l | ForEach-Object { $_.analyse.smp.causes }) | Sort-Object -Unique) -join ', '
    Write-Host ("| {0} | {1}/{2} | {3} | {4} |" -f $conf, $okN, $l.Count, (Resume-Mesure $l { $args[0].analyse.demarrage.hote_jusqu_a_l_ordonnanceur_ms }), $causes)
    if ($conf -match '^whpx-smp(\d+)$' -and $okN -eq $l.Count -and $l.Count -ge 1) { $qualifieSmp = [math]::Max($qualifieSmp, [int]$Matches[1]) }
}

Write-Host ""
Write-Host "Demarrage (ms ; hote = temps mural depuis le lancement de QEMU, BIOS et chargeur compris ; invite = horloge du noyau)"
Write-Host "| Configuration | hote -> noyau | noyau -> bureau (invite) | hote -> bureau |"
Write-Host "|---|---|---|---|"
foreach ($conf in $Configurations) {
    $l = @($resultats | Where-Object configuration -eq $conf)
    Write-Host ("| {0} | {1} | {2} | {3} |" -f $conf, (Resume-Mesure $l { $args[0].analyse.demarrage.hote_jusqu_au_noyau_ms }),
        (Resume-Mesure $l { $args[0].analyse.demarrage.noyau_jusqu_au_bureau_ms_invite }), (Resume-Mesure $l { $args[0].analyse.demarrage.hote_jusqu_au_bureau_ms }))
}

$qualifieLb = 0
Write-Host ""
Write-Host ("B -- Ladybird ({0}) : lancement automatique, premiere trame, document HTTPS, separement" -f $Ladybird.statut)
if ($Ladybird.statut -eq 'PRET') {
    Write-Host "| Configuration | B OK | lancement -> 1re trame (ms) | lancement -> document HTTPS (ms) | presentation moy (us) | CPU occupe (%) | Causes |"
    Write-Host "|---|---|---|---|---|---|---|"
    foreach ($conf in $Configurations) {
        $l = @($resultats | Where-Object configuration -eq $conf)
        $okN = @($l | Where-Object { $_.analyse.ladybird.statut -eq 'OK' }).Count
        $causes = (@($l | ForEach-Object { $_.analyse.ladybird.causes }) | Sort-Object -Unique) -join ', '
        Write-Host ("| {0} | {1}/{2} | {3} | {4} | {5} | {6} | {7} |" -f $conf, $okN, $l.Count,
            (Resume-Mesure $l { $args[0].analyse.navigation.lancement_premiere_trame_ms }), (Resume-Mesure $l { $args[0].analyse.navigation.lancement_document_https_ms }),
            (Resume-Mesure $l { $args[0].analyse.composition.present_us_moy }), (Resume-Mesure $l { $args[0].analyse.charge_cpu.occupation_pct }), $causes)
        $smpOk = @($l | Where-Object { $_.analyse.smp.statut -eq 'OK' }).Count
        if ($conf -match '^whpx-smp(\d+)$' -and $okN -eq $l.Count -and $smpOk -eq $l.Count -and $l.Count -ge 1) { $qualifieLb = [math]::Max($qualifieLb, [int]$Matches[1]) }
    }
}
else {
    Write-Host ("LADYBIRD_{0} raison={1}" -f $Ladybird.statut, $Ladybird.raison) -ForegroundColor Yellow
}
Write-Host "Latence entree -> presentation : NON_MESURABLE (aucune entree injectee, -display none)."
Write-Host "Comparabilite : meme bootimage, meme disque, meme memoire, meme -smp ; CPU differents (TCG : max ; WHPX : qemu64+SSE4) ;"
Write-Host "  le temps hote inclut QEMU, le BIOS et le chargeur, que le noyau ne voit pas : comparer TCG et WHPX colonne par colonne."

New-Item -ItemType Directory -Force -Path (Join-Path $Racine 'target') | Out-Null
$resultats | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $Racine 'target\mesure-acceleration.json') -Encoding UTF8
$Preuve = Join-Path $Racine 'target\whpx-qualification.json'
if ($qualifieSmp -ge 1) {
    [pscustomobject][ordered]@{ schema = 'bouchaud-whpx-qualification/2'; bootimage_sha256 = $Empreinte; smp_max_qualifie = $qualifieSmp
        ladybird_smp_max_qualifie = $qualifieLb; ladybird = $Ladybird.statut
        irqchip = $WhpxIrqchip; repetitions = $Repetitions; date = (Get-Date).ToString('s'); environnement = 'WHPX (Windows Hypervisor Platform), cette machine' } |
        ConvertTo-Json | Set-Content -LiteralPath $Preuve -Encoding UTF8
    Write-Host ("WHPX_SMP_QUALIFIE smp_max={0} irqchip={1}" -f $qualifieSmp, $WhpxIrqchip) -ForegroundColor Green
}
else {
    Remove-Item -LiteralPath $Preuve -ErrorAction SilentlyContinue
    Write-Host "WHPX_SMP_NON_QUALIFIE" -ForegroundColor Yellow
}
if ($Ladybird.statut -ne 'PRET') { Write-Host ("LADYBIRD_WHPX_{0} raison={1}" -f $Ladybird.statut, $Ladybird.raison) -ForegroundColor Yellow }
elseif ($qualifieLb -ge 1) { Write-Host ("LADYBIRD_WHPX_QUALIFIE smp_max={0} -> run.ps1 -Accel auto l'utilisera pour Ladybird SMP{0} et moins" -f $qualifieLb) -ForegroundColor Green }
else { Write-Host "LADYBIRD_WHPX_NON_QUALIFIE -> run.ps1 -Accel auto reste en TCG pour Ladybird en SMP" -ForegroundColor Yellow }
Write-Host ("bilan    : {0}" -f (Join-Path $Racine 'target\mesure-acceleration.json'))
Write-Host "journaux : $Travail"
