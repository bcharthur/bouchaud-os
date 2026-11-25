<#
.SYNOPSIS
    BOUCHAUD_BANC_AVANT_APRES_V1 -- banc avant/apres sur CE PC Windows (WHPX
    ou TCG), avec Ladybird, et rapport automatique.

.DESCRIPTION
    Deux comparaisons, toujours a parametres, scenario et disque identiques :

      -Mode noyaux  : deux bootimages (-Avant, -Apres), UN disque (-Disque).
                      Sert a verifier un correctif noyau (#375, #378, #377...).
      -Mode disques : UN bootimage (-Noyau), deux disques (-DisqueAvant,
                      -DisqueApres). Sert a comparer services separes et
                      BouchaudServices (meme artefact, -ServicesUniques).

    Chaque passage est execute par tools\windows\Banc-Candidat.ps1 (copie du
    disque en snapshot=on, autorun de banc, accelerateur reel verifie par le
    CPUID publie par le noyau, passage invalide publie et jamais retire). Les
    bras alternent (A B, B A, A B...).

    Mesures par passage :
      hote   : ouverture du navigateur, premiere page, bureau, RAM de pointe du
               processus QEMU ;
      invite : FPS du bureau, presentation p99, CPU (cpu_invite_pct), RAM
               utilisee au pic (ram_used_bytes), octets lus sur le disque
               ([BACKING-CACHE]), pics HID (HID_LATENCY_SPIKE : nombre et
               pire attente du fil d'entree entre son reveil et son execution).
    La latence entree -> image n'est pas mesuree (aucun marqueur d'entree).

    Rapport : rapport.md et rapport.json dans -Sortie. Verdict par mesure :
    APRES_MEILLEUR / AVANT_MEILLEUR seulement si les intervalles [min, max]
    des passages VALIDES ne se recouvrent pas ; sinon NON_CONCLUANT ; sans
    passage valide d'un bras : NON_MESURABLE. Aucun gain n'est annonce sur un
    recouvrement. Un passage TCG n'est jamais compte comme WHPX.

.EXAMPLE
    .\tools\windows\Banc-AvantApres.ps1 -Mode noyaux -Avant main.bin -Apres cand.bin -Disque disque-separes.img -Paires 3 -Accel whpx -Smp 1

.EXAMPLE
    .\tools\windows\Banc-AvantApres.ps1 -Mode disques -Noyau main.bin -DisqueAvant disque-separes.img -DisqueApres disque-uniques.img -Paires 3

.EXAMPLE
    .\tools\windows\Banc-AvantApres.ps1 -AnalyserSortie C:\banc\ab-375
#>
[CmdletBinding()]
param(
    [ValidateSet('noyaux', 'disques')][string]$Mode = 'noyaux',
    [string]$Avant,
    [string]$Apres,
    [string]$Noyau,
    [string]$Disque,
    [string]$DisqueAvant,
    [string]$DisqueApres,
    [int]$Paires = 3,
    [ValidateSet('whpx', 'tcg')][string]$Accel = 'whpx',
    [int]$Smp = 1,
    [int]$MemoireMio = 4096,
    [int]$DefilementS = 8,
    [string]$Qemu = 'qemu-system-x86_64',
    [string]$Sortie,
    [string]$AnalyserSortie,
    [switch]$DryRun
)
$ErrorActionPreference = 'Stop'
$Racine = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$BancCandidat = Join-Path $PSScriptRoot 'Banc-Candidat.ps1'
$Inv = [System.Globalization.CultureInfo]::InvariantCulture

# Mesure -> sens : -1 plus petit est meilleur, +1 plus grand est meilleur.
$Sens = [ordered]@{
    page_ms             = -1
    ouverture_ms        = -1
    bureau_ms           = -1
    fps_bureau          = 1
    presentation_p99_us = -1
    cpu_invite_pct      = -1
    ram_qemu_pic_mio    = -1
    ram_invite_pic_mio  = -1
    disque_lu_mio       = -1
    hid_pics            = -1
    hid_pire_ms         = -1
}

# Un nombre lu dans un CSV ecrit sous n'importe quelle culture ("1,5" ou "1.5").
function ConvertTo-Nombre($Valeur) {
    if ($null -eq $Valeur) { return $null }
    $s = ([string]$Valeur).Trim()
    if ($s -eq '') { return $null }
    $d = 0.0
    if ([double]::TryParse($s.Replace(',', '.'), [System.Globalization.NumberStyles]::Float, $Inv, [ref]$d)) { return $d }
    return $null
}

# Mesures lues dans le journal serie d'un passage (horloge de l'invite).
function Get-MesuresJournal([string]$Chemin) {
    $t = [System.IO.File]::ReadAllText($Chemin)
    $pics = [regex]::Matches($t, 'HID_LATENCY_SPIKE delta_us=(\d+)')
    $pire = 0
    foreach ($m in $pics) { $v = [long]$m.Groups[1].Value; if ($v -gt $pire) { $pire = $v } }
    $ram = 0
    foreach ($m in [regex]::Matches($t, 'ram_used_bytes=(\d+)')) { $v = [long]$m.Groups[1].Value; if ($v -gt $ram) { $ram = $v } }
    $lu = [regex]::Matches($t, '\[BACKING-CACHE\] reads=\d+ bytes=(\d+)')
    $luMio = $null
    if ($lu.Count -gt 0) { $luMio = [math]::Round([double]$lu[$lu.Count - 1].Groups[1].Value / 1MB, 1) }
    $ramMio = $null
    if ($ram -gt 0) { $ramMio = [math]::Round([double]$ram / 1MB, 1) }
    return [ordered]@{
        hid_pics           = $pics.Count
        hid_pire_ms        = [math]::Round([double]$pire / 1000.0, 1)
        ram_invite_pic_mio = $ramMio
        disque_lu_mio      = $luMio
    }
}

# Tous les passages d'une sortie : chaque banc.csv de Banc-Candidat, sous
# `noyaux` (main = avant, candidat = apres) ou `disques-N-avant|apres`.
function Get-PassagesBanc([string]$Dossier) {
    $lignes = @()
    foreach ($csv in @(Get-ChildItem -LiteralPath $Dossier -Recurse -Filter 'banc.csv' | Sort-Object FullName)) {
        $bancDir = $csv.Directory
        $etiquette = $null
        if ($bancDir.Name -match '^disques-\d+-(avant|apres)$') { $etiquette = $Matches[1] }
        foreach ($r in @(Import-Csv -LiteralPath $csv.FullName)) {
            $bras = $etiquette
            if (-not $bras) { if ($r.bras -eq 'main') { $bras = 'avant' } else { $bras = 'apres' } }
            $passage = '{0}/{1}-{2}' -f $bancDir.Name, $r.bras, $r.paire
            $l = [ordered]@{
                bras                = $bras
                passage             = $passage
                valide              = ([string]$r.valide -eq 'True')
                causes              = [string]$r.causes
                accel_reel          = [string]$r.accel_reel
                page_ms             = (ConvertTo-Nombre $r.hote_premiere_page_ms)
                ouverture_ms        = (ConvertTo-Nombre $r.hote_ouverture_ladybird_ms)
                bureau_ms           = (ConvertTo-Nombre $r.hote_qemu_bureau_ms)
                fps_bureau          = (ConvertTo-Nombre $r.fps_bureau)
                presentation_p99_us = (ConvertTo-Nombre $r.presentation_us_p99)
                cpu_invite_pct      = (ConvertTo-Nombre $r.cpu_invite_pct)
                ram_qemu_pic_mio    = (ConvertTo-Nombre $r.hote_ram_qemu_pic_mio)
                ram_invite_pic_mio  = $null
                disque_lu_mio       = $null
                hid_pics            = $null
                hid_pire_ms         = $null
            }
            $journal = Join-Path $bancDir.FullName ('{0}-{1}' -f $r.bras, $r.paire)
            $journal = Join-Path $journal 'serie.log'
            if (Test-Path -LiteralPath $journal) {
                $mj = Get-MesuresJournal $journal
                foreach ($k in $mj.Keys) { $l[$k] = $mj[$k] }
            }
            $lignes += [pscustomobject]$l
        }
    }
    return $lignes
}

function Get-VerdictsAvantApres($Lignes) {
    $verdicts = @()
    foreach ($k in $Sens.Keys) {
        $a = @($Lignes | Where-Object { $_.valide -and $_.bras -eq 'avant' -and $null -ne $_.$k } | ForEach-Object { [double]$_.$k })
        $b = @($Lignes | Where-Object { $_.valide -and $_.bras -eq 'apres' -and $null -ne $_.$k } | ForEach-Object { [double]$_.$k })
        $v = [ordered]@{ mesure = $k; n_avant = $a.Count; n_apres = $b.Count; avant_min = $null; avant_max = $null; apres_min = $null; apres_max = $null; verdict = 'NON_MESURABLE' }
        if ($a.Count -gt 0 -and $b.Count -gt 0) {
            $ma = $a | Measure-Object -Minimum -Maximum
            $mb = $b | Measure-Object -Minimum -Maximum
            $v.avant_min = $ma.Minimum; $v.avant_max = $ma.Maximum
            $v.apres_min = $mb.Minimum; $v.apres_max = $mb.Maximum
            $v.verdict = 'NON_CONCLUANT'
            if ($Sens[$k] -lt 0) {
                if ($mb.Maximum -lt $ma.Minimum) { $v.verdict = 'APRES_MEILLEUR' }
                elseif ($ma.Maximum -lt $mb.Minimum) { $v.verdict = 'AVANT_MEILLEUR' }
            } else {
                if ($mb.Minimum -gt $ma.Maximum) { $v.verdict = 'APRES_MEILLEUR' }
                elseif ($ma.Minimum -gt $mb.Maximum) { $v.verdict = 'AVANT_MEILLEUR' }
            }
        }
        $verdicts += [pscustomobject]$v
    }
    return $verdicts
}

function Format-Intervalle($Min, $Max) {
    if ($null -eq $Min) { return '-' }
    return ('{0} - {1}' -f ([double]$Min).ToString('0.#', $Inv), ([double]$Max).ToString('0.#', $Inv))
}

function Write-RapportAvantApres([string]$Dossier, $Meta) {
    $lignes = @(Get-PassagesBanc $Dossier)
    $verdicts = @(Get-VerdictsAvantApres $lignes)
    $invalides = @($lignes | Where-Object { -not $_.valide }).Count
    $accels = (@($lignes | Where-Object { $_.valide } | ForEach-Object { $_.accel_reel }) | Sort-Object -Unique) -join ','
    [ordered]@{ metadonnees = $Meta; accel_reel_valides = $accels; passages = $lignes; verdicts = $verdicts } |
        ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $Dossier 'rapport.json') -Encoding UTF8
    $md = @()
    $md += '# Banc avant / apres'
    $md += ''
    $md += ('Mode : {0} ; accelerateur reel des passages valides : {1} ; passages : {2} (invalides : {3}).' -f $Meta.mode, $accels, $lignes.Count, $invalides)
    $md += 'Verdict seulement si les intervalles [min - max] ne se recouvrent pas.'
    $md += ''
    $md += '| Mesure | Avant (min - max) | Apres (min - max) | n | Verdict |'
    $md += '|---|---|---|---|---|'
    foreach ($v in $verdicts) {
        $md += ('| {0} | {1} | {2} | {3}/{4} | {5} |' -f $v.mesure, (Format-Intervalle $v.avant_min $v.avant_max), (Format-Intervalle $v.apres_min $v.apres_max), $v.n_avant, $v.n_apres, $v.verdict)
    }
    $md += ''
    $md += '| Passage | Bras | Valide | Causes | Accel | Page (ms) | FPS | Pics HID | Pire HID (ms) | RAM invite (Mio) | Lu (Mio) |'
    $md += '|---|---|---|---|---|---|---|---|---|---|---|'
    foreach ($l in $lignes) {
        $md += ('| {0} | {1} | {2} | {3} | {4} | {5} | {6} | {7} | {8} | {9} | {10} |' -f $l.passage, $l.bras, $l.valide, $l.causes, $l.accel_reel, $l.page_ms, $l.fps_bureau, $l.hid_pics, $l.hid_pire_ms, $l.ram_invite_pic_mio, $l.disque_lu_mio)
    }
    Set-Content -LiteralPath (Join-Path $Dossier 'rapport.md') -Value $md -Encoding UTF8
    Write-Host ''
    $verdicts | Format-Table mesure, avant_min, avant_max, apres_min, apres_max, verdict -AutoSize | Out-String | Write-Host
    Write-Host ('BANC_AVANT_APRES_FIN passages={0} invalides={1} accel_reel={2} rapport={3}' -f $lignes.Count, $invalides, $accels, (Join-Path $Dossier 'rapport.md'))
}

# --- Analyse seule d'une sortie existante ----------------------------------
if ($AnalyserSortie) {
    if (-not (Test-Path -LiteralPath $AnalyserSortie)) { throw "sortie absente : $AnalyserSortie" }
    Write-RapportAvantApres $AnalyserSortie ([ordered]@{ mode = 'analyse'; sortie = $AnalyserSortie })
    return
}

if ($Paires -lt 1) { throw '-Paires >= 1' }
if (-not $Sortie) { $Sortie = Join-Path $Racine ('banc-avant-apres-' + (Get-Date -Format 'yyyyMMdd-HHmmss')) }
$communs = @{ Accel = $Accel; Smp = $Smp; MemoireMio = $MemoireMio; DefilementS = $DefilementS; Qemu = $Qemu }
if ($DryRun) { $communs.DryRun = $true }
$meta = [ordered]@{ mode = $Mode; accel_demande = $Accel; smp = $Smp; memoire_mio = $MemoireMio; defilement_s = $DefilementS; paires = $Paires; date = (Get-Date).ToUniversalTime().ToString('o') }

if ($Mode -eq 'noyaux') {
    if (-not $Avant -or -not $Apres) { throw '-Mode noyaux : -Avant et -Apres sont requis (deux bootimages)' }
    if (-not $Disque) { $Disque = Join-Path $Racine 'ladybird-browser.img' }
    $meta.avant = $Avant; $meta.apres = $Apres; $meta.disque = $Disque
    Write-Host ('PLAN_AVANT_APRES mode=noyaux avant={0} apres={1} disque={2} paires={3}' -f $Avant, $Apres, $Disque, $Paires)
    & $BancCandidat -Main $Avant -Candidat $Apres -Disque $Disque -Paires $Paires -Sortie (Join-Path $Sortie 'noyaux') @communs
} else {
    if (-not $Noyau -or -not $DisqueAvant -or -not $DisqueApres) { throw '-Mode disques : -Noyau, -DisqueAvant et -DisqueApres sont requis' }
    $meta.noyau = $Noyau; $meta.disque_avant = $DisqueAvant; $meta.disque_apres = $DisqueApres
    $n = 0
    for ($i = 1; $i -le $Paires; $i++) {
        $ordre = @('avant', 'apres')
        if ($i % 2 -eq 0) { $ordre = @('apres', 'avant') }
        foreach ($bras in $ordre) {
            $n++
            $d = $DisqueAvant
            if ($bras -eq 'apres') { $d = $DisqueApres }
            Write-Host ('PLAN_AVANT_APRES mode=disques n={0} bras={1} disque={2}' -f $n, $bras, $d)
            & $BancCandidat -Main $Noyau -Candidat $Noyau -Disque $d -Paires 1 -Sortie (Join-Path $Sortie ('disques-{0}-{1}' -f $n, $bras)) @communs
        }
    }
}
if ($DryRun) { return }
Write-RapportAvantApres $Sortie $meta
exit 0
