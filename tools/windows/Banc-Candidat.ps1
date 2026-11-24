<#
.SYNOPSIS
    BOUCHAUD_BANC_CANDIDAT_V1 -- A/B main contre candidat sur CE PC Windows,
    sous WHPX (ou TCG), avec Ladybird, en une commande.

.DESCRIPTION
    Deux bootimages (main et candidat), MEME disque Ladybird (copie de banc,
    autorun BO_AUTOSTART_BROWSER=1, montee en snapshot=on : jamais modifiee),
    memes options QEMU. Les bras ALTERNENT par paire (M C, C M, M C...) pour
    que la derive de l'hote ne favorise aucun bras.

    Par passage, horloge de l'HOTE (Stopwatch, scrutation toutes les 100 ms) :
      bureau, ouverture du navigateur (lancement -> 1re trame), 1re page
      (lancement -> document charge) ;
    puis, page chargee, le banc injecte la molette par le moniteur QEMU
    (HMP `mouse_move 0 0 dz`) pendant -DefilementS secondes et releve,
    horloge de l'INVITE, le dernier [GPU-TRAMES] (dessin / presentation
    p50 p95 p99), [GPU-COMPOSITION] (trames, trames GPU, pixels poses GPU,
    pixels clients copies CPU, cpu_pct) et [LB:PERF] (copie_px) ;
    cote hote : temps CPU et memoire de pointe du processus QEMU.

    Un passage n'est VALIDE que si l'accelerateur reel lu dans le noyau
    (BOUCHAUD_CPU_ISA) est celui demande et si la page a charge. Un passage
    invalide est publie avec sa cause, jamais retire en silence, et n'entre
    pas dans les medianes.

    Verdict par mesure : CANDIDAT_MEILLEUR / MAIN_MEILLEUR si les intervalles
    [min, max] des deux bras ne se recouvrent pas ; sinon NON_CONCLUANT. Aucun
    gain n'est annonce sur un recouvrement.

    GPU : le banc DIT ce que QEMU peut exposer (virtio-gpu-gl, affichage GL,
    adaptateurs de l'hote) et lit le backend reel publie par l'invite
    (`GPU=<mode>:`). WHPX n'a pas de passage PCI : la RTX de l'hote n'est
    JAMAIS vue nue par l'invite ; au mieux virgl (OpenGL de l'hote,
    paravirtualise), et seulement si le binaire QEMU l'embarque.

    Latence entree -> image : NON_MESURABLE (aucun marqueur d'horodatage
    d'entree dans le journal serie) -- publie comme tel.

.EXAMPLE
    .\tools\windows\Banc-Candidat.ps1 -Main main.bin -Candidat cand.bin -Paires 3 -Accel whpx -Smp 2

.EXAMPLE
    .\tools\windows\Banc-Candidat.ps1 -Main main.bin -Candidat cand.bin -DryRun
#>
[CmdletBinding()]
param(
    [string]$Main,
    [string]$Candidat,
    [int]$Paires = 3,
    [ValidateSet('whpx', 'tcg')][string]$Accel = 'whpx',
    [int]$Smp = 2,
    [ValidateSet('aucun', 'auto', 'virgl')][string]$Gpu = 'aucun',
    [int]$MemoireMio = 4096,
    [int]$DelaiS = 420,
    [int]$DefilementS = 8,
    [string]$Disque,
    [string]$Qemu = 'qemu-system-x86_64',
    [string]$Sortie,
    [string]$AnalyserJournal,
    [switch]$DryRun
)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'BouchaudQemu.psm1') -Force
$Racine = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)

$Jalons = [ordered]@{
    noyau          = 'BOUCHAUD_BOOT_POINT kernel-main'
    bureau         = 'BOUCHAUD_BOOT_POINT bureau-premiere-trame'
    navigateur     = 'BROWSER_HOST_START'
    premiere_trame = 'BOUCHAUD_UI_FIRST_FRAME onglet=1'
    document       = '[LB:NAV] onglet=1 document_charge'
}

function Dernier([string]$Texte, [string]$Motif) {
    $m = [regex]::Matches($Texte, $Motif)
    if ($m.Count -eq 0) { return $null }
    return $m[$m.Count - 1]
}

function Entier($m, [int]$g) { if ($m) { return [long]$m.Groups[$g].Value } return $null }

# Mesures lues dans le journal serie (horloge de l'invite). $Depuis : ne
# considerer que le texte ecrit APRES cet offset (fenetre de defilement).
function Get-MesuresInvite([string]$Serie, [int]$Depuis = 0) {
    $Serie = [regex]::Replace($Serie, "$([char]27)\[[0-9;]*m", '')
    $fen = if ($Depuis -gt 0 -and $Depuis -lt $Serie.Length) { $Serie.Substring($Depuis) } else { $Serie }
    $t = Dernier $fen '\[GPU-TRAMES\] dessin_cpu_us_p50=(\d+) p95=(\d+) p99=(\d+) max=(\d+) presentation_us_p50=(\d+) p95=(\d+) p99=(\d+) max=(\d+)'
    $c = Dernier $fen '\[GPU-COMPOSITION\] disponible=(\S+) periode_ms=(\d+) trames_bureau=(\d+) trames_clients=(\d+) trames_gpu=(\d+) blits=(\d+) px_poses_gpu=(\d+) px_montes_hote=(\d+) px_client_copies_cpu=(\d+) us_dessin_cpu_par_trame=(\d+) us_presentation_par_trame=(\d+) textures=(\d+) fenetres_gpu=(\d+) cpu_pct=(\d+)'
    $p = Dernier $fen '\[LB:PERF\] trames=(\d+) vues=(\d+) degat_px=(\d+) zone_px=(\d+) copie_px=(\d+)'
    $b = Dernier $Serie 'GPU=(\w+):'
    $bureau = Dernier $Serie 'BOUCHAUD_BOOT_POINT bureau-premiere-trame t_ms=(\d+)'
    $fps = $null
    if ($c -and [long]$c.Groups[2].Value -gt 0) { $fps = [math]::Round(1000.0 * [long]$c.Groups[3].Value / [long]$c.Groups[2].Value, 1) }
    return [ordered]@{
        invite_noyau_bureau_ms   = (Entier $bureau 1)
        dessin_us_p50            = (Entier $t 1); dessin_us_p95 = (Entier $t 2); dessin_us_p99 = (Entier $t 3)
        presentation_us_p50      = (Entier $t 5); presentation_us_p95 = (Entier $t 6); presentation_us_p99 = (Entier $t 7)
        fps_bureau               = $fps
        trames_gpu               = (Entier $c 5)
        px_poses_gpu             = (Entier $c 7)
        px_client_copies_cpu     = (Entier $c 9)
        cpu_invite_pct           = (Entier $c 14)
        lb_copie_px              = (Entier $p 5)
        backend_gpu              = $(if ($b) { $b.Groups[1].Value } else { $null })
    }
}

if ($AnalyserJournal) {
    # Mode de controle (CI) : les mesures d'un journal, en JSON.
    Get-MesuresInvite (Read-BouchaudJournal $AnalyserJournal) | ConvertTo-Json -Compress
    return
}

if (-not $Main -or -not $Candidat) { throw '-Main et -Candidat sont requis (deux bootimages)' }
if ($Paires -lt 1) { throw '-Paires >= 1' }
if (-not $Disque) { $Disque = Join-Path $Racine 'ladybird-browser.img' }
if (-not $Sortie) { $Sortie = Join-Path $Racine ("banc-candidat-" + (Get-Date -Format 'yyyyMMdd-HHmmss')) }

# --- Ce que l'hote peut exposer ---------------------------------------------
function Get-DiagnosticHote {
    $d = [ordered]@{ qemu = $null; whpx_dans_qemu = $null; virtio_gpu_gl = $null; affichage_gl = $null; adaptateurs = @(); gpu_hote = '' }
    if (-not $DryRun) {
        $ErrorActionPreference = 'Continue'
        $d.qemu = ((& $Qemu --version 2>&1 | Out-String).Trim() -split "`n")[0]
        $d.whpx_dans_qemu = [bool]((& $Qemu -accel help 2>&1 | Out-String) -match '(?m)^\s*whpx\s*$')
        $dev = & $Qemu -device help 2>&1 | Out-String
        $d.virtio_gpu_gl = [bool]($dev -match '"virtio-(vga|gpu)-gl"')
        $aff = & $Qemu -display help 2>&1 | Out-String
        $d.affichage_gl = [bool]($aff -match '(?m)^\s*(sdl|gtk)\s*$')
        try { $d.adaptateurs = @(Get-CimInstance Win32_VideoController | ForEach-Object { "$($_.Name) ($($_.DriverVersion))" }) } catch { }
    }
    $d.gpu_hote = if ($d.virtio_gpu_gl -and $d.affichage_gl) {
        'VIRGL_POSSIBLE : virtio-gpu-gl + affichage GL presents (OpenGL de l''hote, paravirtualise ; pas le GPU nu)'
    } else {
        'NON_EXPOSABLE : ce QEMU n''embarque pas virtio-gpu-gl + affichage GL ; WHPX n''a pas de passage PCI'
    }
    return [pscustomobject]$d
}

function Get-ArgumentsQemu([string]$Boot, [string]$Journal, [int]$Port, [string]$CheminDisque, $Diag) {
    $a = @('-machine', 'pc', '-m', "$MemoireMio", '-smp', "$Smp", '-no-reboot',
           '-drive', "format=raw,file=$Boot", '-serial', "file:$Journal", '-nic', 'user,model=e1000',
           '-monitor', "tcp:127.0.0.1:$Port,server,nowait")
    $a += (Get-BouchaudAccelQemu -Accel $Accel -CpuCount $Smp -Ladybird $true).arguments
    $virgl = ($Gpu -eq 'virgl') -or ($Gpu -eq 'auto' -and $Diag.gpu_hote -like 'VIRGL_POSSIBLE*')
    if ($virgl) { $a += @('-device', 'virtio-vga-gl', '-display', 'sdl,gl=on') } else { $a += @('-display', 'none') }
    if ($CheminDisque) { $a += @('-drive', "format=raw,file=$CheminDisque,snapshot=on") }
    return $a
}

function Send-Molette([int]$Port, [int]$DureeS) {
    $client = New-Object System.Net.Sockets.TcpClient('127.0.0.1', $Port)
    try {
        $flux = $client.GetStream()
        $ecrit = New-Object System.IO.StreamWriter($flux)
        $ecrit.AutoFlush = $true
        $tampon = New-Object byte[] 4096
        $fin = [DateTime]::UtcNow.AddSeconds($DureeS)
        $n = 0
        while ([DateTime]::UtcNow -lt $fin) {
            # 2 s vers le bas, 2 s vers le haut : la page ne bute jamais.
            $dz = if ([math]::Floor($n / 40) % 2 -eq 0) { 1 } else { -1 }
            $ecrit.Write("mouse_move 0 0 $dz`n")
            while ($flux.DataAvailable) { [void]$flux.Read($tampon, 0, $tampon.Length) }
            $n++
            Start-Sleep -Milliseconds 50
        }
        return $n
    }
    finally { $client.Close() }
}

function Invoke-Passage([string]$Bras, [string]$Boot, [int]$Numero, [string]$CheminDisque, $Diag) {
    $dossier = Join-Path $Sortie "$Bras-$Numero"
    $journal = Join-Path $dossier 'serie.log'
    $erreurs = Join-Path $dossier 'qemu.err'
    $port = 45000 + (Get-Random -Maximum 2000)
    $args_ = Get-ArgumentsQemu $Boot $journal $port $CheminDisque $Diag
    if ($DryRun) { Write-Host ("PLAN paire {0} {1} : {2} {3}" -f $Numero, $Bras, $Qemu, ($args_ -join ' ')); return $null }
    New-Item -ItemType Directory -Path $dossier -Force | Out-Null
    $vus = @{}
    $molette = 0
    $offsetDefilement = 0
    $cpuHote = $null; $ramHoteMio = $null
    $chrono = [System.Diagnostics.Stopwatch]::StartNew()
    $p = $null
    try {
        $p = Start-Process -FilePath $Qemu -ArgumentList (ConvertTo-BouchaudLigneArguments $args_) -PassThru -NoNewWindow -RedirectStandardError $erreurs
        while ($chrono.Elapsed.TotalSeconds -lt $DelaiS -and -not $p.HasExited) {
            Start-Sleep -Milliseconds 100
            $texte = Read-BouchaudJournal $journal
            foreach ($k in $Jalons.Keys) {
                if (-not $vus.ContainsKey($k) -and $texte.Contains($Jalons[$k])) { $vus[$k] = [math]::Round($chrono.Elapsed.TotalMilliseconds) }
            }
            if ($texte -match 'KERNEL PANIC|DOUBLE FAULT') { break }
            if ((Read-BouchaudJournal $erreurs) -match 'WHPX: Unexpected VP exit') { break }
            if ($vus.ContainsKey('document')) {
                Start-Sleep -Seconds 2
                $offsetDefilement = (Read-BouchaudJournal $journal).Length
                try { $molette = Send-Molette $port $DefilementS } catch { $molette = -1 }
                # Laisser passer au moins un releve periodique apres le geste.
                Start-Sleep -Seconds 3
                break
            }
        }
        if (-not $p.HasExited) {
            $p.Refresh()
            $cpuHote = [math]::Round($p.TotalProcessorTime.TotalMilliseconds)
            $ramHoteMio = [math]::Round($p.PeakWorkingSet64 / 1MB)
        }
    }
    finally {
        if ($p -and -not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue; $p.WaitForExit(10000) | Out-Null }
    }
    $serie = Read-BouchaudJournal $journal
    $verdict = Get-BouchaudVerdictQemu -JournalSerie $journal -JournalErreurs $erreurs
    $attendu = if ($Accel -eq 'tcg') { 'TCG' } else { 'materiel' }
    $causes = @()
    if ($verdict.accel_reel -ne $attendu) { $causes += "accel_reel=$($verdict.accel_reel)" }
    if ($null -ne $verdict.vp_exit) { $causes += "whpx_vp_exit=$($verdict.vp_exit)" }
    if (-not $vus.ContainsKey('document')) { $causes += 'page-non-chargee' }
    if ($molette -le 0) { $causes += 'molette-non-injectee' }
    $ecart = { param($a, $b) if ($vus.ContainsKey($a) -and $vus.ContainsKey($b)) { $vus[$b] - $vus[$a] } else { $null } }
    $r = [ordered]@{
        bras = $Bras; paire = $Numero; valide = ($causes.Count -eq 0); causes = ($causes -join ',')
        accel_reel = $verdict.accel_reel; signature_cpuid = $verdict.signature
        hote_qemu_noyau_ms = $vus['noyau']; hote_qemu_bureau_ms = $vus['bureau']
        hote_ouverture_ladybird_ms = (& $ecart 'navigateur' 'premiere_trame')
        hote_premiere_page_ms = (& $ecart 'navigateur' 'document')
        hote_cpu_qemu_ms = $cpuHote; hote_ram_qemu_pic_mio = $ramHoteMio
        molette_evenements = $molette
    }
    $mi = Get-MesuresInvite $serie $offsetDefilement
    foreach ($k in $mi.Keys) { $r[$k] = $mi[$k] }
    $r.octets_serie = [System.Text.Encoding]::UTF8.GetByteCount($serie)
    $r.journal = $journal
    return [pscustomobject]$r
}

# Mesure -> sens : -1 plus petit est meilleur, +1 plus grand est meilleur.
$Sens = [ordered]@{
    hote_qemu_bureau_ms = -1; hote_ouverture_ladybird_ms = -1; hote_premiere_page_ms = -1
    invite_noyau_bureau_ms = -1; fps_bureau = 1
    dessin_us_p50 = -1; dessin_us_p95 = -1; dessin_us_p99 = -1
    presentation_us_p50 = -1; presentation_us_p95 = -1; presentation_us_p99 = -1
    px_client_copies_cpu = -1; lb_copie_px = -1; cpu_invite_pct = -1
    hote_cpu_qemu_ms = -1; hote_ram_qemu_pic_mio = -1
}

function Get-Mediane([double[]]$v) {
    $t = @($v | Sort-Object); $n = $t.Count
    if ($n -eq 0) { return $null }
    if ($n % 2 -eq 1) { return $t[($n - 1) / 2] }
    return ($t[$n / 2 - 1] + $t[$n / 2]) / 2
}

function Get-Verdicts($Passages) {
    foreach ($k in $Sens.Keys) {
        $vm = @($Passages | Where-Object { $_.valide -and $_.bras -eq 'main' -and $null -ne $_.$k } | ForEach-Object { [double]$_.$k })
        $vc = @($Passages | Where-Object { $_.valide -and $_.bras -eq 'candidat' -and $null -ne $_.$k } | ForEach-Object { [double]$_.$k })
        $v = 'NON_MESURE'
        if ($vm.Count -gt 0 -and $vc.Count -gt 0) {
            $mMin = ($vm | Measure-Object -Minimum).Minimum; $mMax = ($vm | Measure-Object -Maximum).Maximum
            $cMin = ($vc | Measure-Object -Minimum).Minimum; $cMax = ($vc | Measure-Object -Maximum).Maximum
            $v = 'NON_CONCLUANT'
            if ($cMax -lt $mMin) { $v = $(if ($Sens[$k] -lt 0) { 'CANDIDAT_MEILLEUR' } else { 'MAIN_MEILLEUR' }) }
            elseif ($cMin -gt $mMax) { $v = $(if ($Sens[$k] -lt 0) { 'MAIN_MEILLEUR' } else { 'CANDIDAT_MEILLEUR' }) }
        }
        [pscustomobject][ordered]@{
            mesure = $k; main_mediane = (Get-Mediane $vm); main_n = $vm.Count
            candidat_mediane = (Get-Mediane $vc); candidat_n = $vc.Count; verdict = $v
        }
    }
}

# --- Campagne ---------------------------------------------------------------
$diag = Get-DiagnosticHote
Write-Host ("QEMU        : {0}" -f $diag.qemu)
Write-Host ("WHPX (QEMU) : {0} | virtio-gpu-gl : {1} | affichage GL : {2}" -f $diag.whpx_dans_qemu, $diag.virtio_gpu_gl, $diag.affichage_gl)
Write-Host ("Adaptateurs : {0}" -f ($diag.adaptateurs -join ' ; '))
Write-Host ("GPU hote    : {0}" -f $diag.gpu_hote)
if ($Gpu -eq 'virgl' -and $diag.gpu_hote -notlike 'VIRGL_POSSIBLE*' -and -not $DryRun) {
    throw "-Gpu virgl demande mais $($diag.gpu_hote)"
}

$meta = [ordered]@{ date = (Get-Date).ToUniversalTime().ToString('o'); accel = $Accel; smp = $Smp; memoire_mio = $MemoireMio
    gpu_demande = $Gpu; defilement_s = $DefilementS; diagnostic = $diag; disque = $Disque }
$cheminDisque = $null
if (-not $DryRun) {
    New-Item -ItemType Directory -Path $Sortie -Force | Out-Null
    foreach ($b in @($Main, $Candidat)) { if (-not (Test-Path -LiteralPath $b)) { throw "bootimage absent : $b" } }
    $meta.main = [ordered]@{ image = (Resolve-Path $Main).Path; sha256 = (Get-FileHash -LiteralPath $Main -Algorithm SHA256).Hash.ToLower() }
    $meta.candidat = [ordered]@{ image = (Resolve-Path $Candidat).Path; sha256 = (Get-FileHash -LiteralPath $Candidat -Algorithm SHA256).Hash.ToLower() }
    $copie = New-BouchaudDisqueBanc -Source $Disque -Cible (Join-Path $Sortie 'ladybird-banc.img')
    if ($copie.statut -ne 'PRET') { throw "disque Ladybird de banc impossible : $($copie.raison)" }
    $cheminDisque = $copie.chemin
}

$passages = @()
for ($i = 1; $i -le $Paires; $i++) {
    $ordre = if ($i % 2 -eq 1) { @(@('main', $Main), @('candidat', $Candidat)) } else { @(@('candidat', $Candidat), @('main', $Main)) }
    foreach ($b in $ordre) {
        $r = Invoke-Passage $b[0] $b[1] $i $cheminDisque $diag
        if ($null -eq $r) { continue }
        $passages += $r
        Write-Host ("PASSAGE paire={0} bras={1} valide={2} causes=[{3}] accel={4} bureau={5} ouverture={6} page={7} fps={8} presentation_p99={9} copies_cpu={10} backend={11}" -f `
            $i, $r.bras, $r.valide, $r.causes, $r.accel_reel, $r.hote_qemu_bureau_ms, $r.hote_ouverture_ladybird_ms, $r.hote_premiere_page_ms,
            $r.fps_bureau, $r.presentation_us_p99, $r.px_client_copies_cpu, $r.backend_gpu)
        [ordered]@{ metadonnees = $meta; passages = $passages } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $Sortie 'banc.json') -Encoding UTF8
    }
}
if ($DryRun) { return }

$verdicts = @(Get-Verdicts $passages)
[ordered]@{ metadonnees = $meta; passages = $passages; verdicts = $verdicts } | ConvertTo-Json -Depth 6 |
    Set-Content -LiteralPath (Join-Path $Sortie 'banc.json') -Encoding UTF8
$passages | Select-Object * -ExcludeProperty journal | Export-Csv -LiteralPath (Join-Path $Sortie 'banc.csv') -NoTypeInformation -Encoding UTF8
$verdicts | Export-Csv -LiteralPath (Join-Path $Sortie 'verdicts.csv') -NoTypeInformation -Encoding UTF8
Write-Host ''
$verdicts | Format-Table -AutoSize | Out-String | Write-Host
$invalides = @($passages | Where-Object { -not $_.valide }).Count
$backends = (@($passages | ForEach-Object { $_.backend_gpu }) | Sort-Object -Unique) -join ','
Write-Host ("BANC_CANDIDAT_FIN passages={0} invalides={1} backend_gpu_reel={2} sortie={3}" -f $passages.Count, $invalides, $backends, $Sortie)
if ($invalides -gt 0) { exit 2 }
