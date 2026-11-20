# Campagne Stage 2 GPU a disques en snapshot. Les interactions sont humaines.
param(
    [Parameter(Mandatory = $true)][uri]$Url,
    [Parameter(Mandatory = $true)][string]$Sortie,
    [ValidateRange(3,12)][int]$Repetitions = 3,
    [ValidateSet(1,2,4)][int]$Smp = 1,
    [ValidateSet('whpx','tcg')][string]$Accel = 'whpx',
    [ValidateSet('gtk','sdl')][string]$Display = 'gtk',
    [switch]$DryRun
)
$ErrorActionPreference = 'Stop'
if (-not $Url.IsAbsoluteUri -or $Url.Scheme -notin @('http','https')) {
    throw 'URL de campagne HTTP(S) absolue requise.'
}

$Profil = Join-Path $PSScriptRoot 'run-reference-stage2-ladybird.ps1'
$HotePs = (Get-Process -Id $PID).Path
$Sortie = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Sortie)
if (-not $DryRun) {
    if (Test-Path -LiteralPath $Sortie) { throw "Sortie deja utilisee : $Sortie" }
    New-Item -Path $Sortie -ItemType Directory | Out-Null
}
$Plan = @(
    for ($tour = 1; $tour -le $Repetitions; $tour++) {
        $Ordre = if ($tour % 2 -eq 1) { @('cpu','virgl') } else { @('virgl','cpu') }
        foreach ($Bras in $Ordre) {
            [ordered]@{ nom = '{0:00}-{1}' -f $tour,$Bras; bras = $Bras }
        }
    }
)
$Campagne = [ordered]@{
    url = $Url.AbsoluteUri
    repetitions = $Repetitions
    smp = $Smp
    acceleration = $Accel
    affichage = $Display
    onglets = 1
    attente_s = 30
    defilements_page_down = 3
    ouvertures = 2
    disques = 'snapshot'
    plan = $Plan
    resultats = @()
}
$Manifeste = Join-Path $Sortie 'campagne.json'
if (-not $DryRun) {
    $Campagne | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $Manifeste -Encoding UTF8
}

$Empreintes = @{}
foreach ($Passe in $Plan) {
    $Bras = $Passe.bras
    $Nom = $Passe.nom
    $Rapport = Join-Path $Sortie "$Nom.json"
    $Options = @('-NoProfile','-File',$Profil,'-SkipBuild','-SkipPrepare',
        '-Smp',"$Smp",'-Accel',$Accel,'-Display',$Display,'-QualificationJson',$Rapport)
    if ($Bras -eq 'virgl') { $Options += '-Virgl' }
    Write-Host "$Nom : ouvrir Ladybird, charger $Url dans un onglet ; attendre 30 s puis faire trois PageDown."
    Write-Host 'Fermer la fenetre, rouvrir Ladybird, recharger la meme URL puis quitter QEMU.'
    if ($DryRun) { continue }
    & $HotePs @Options
    $Code = $LASTEXITCODE
    $Existe = Test-Path -LiteralPath $Rapport -PathType Leaf
    $Run = if ($Existe) { Get-Content -LiteralPath $Rapport -Raw | ConvertFrom-Json } else { $null }
    $Campagne.resultats += [ordered]@{
        nom = $Nom; bras = $Bras; rapport = if ($Existe) { "$Nom.json" } else { $null }
        code_sortie = $Code; verdict = if ($Run) { $Run.verdict } else { 'rapport-absent' }
    }
    $Campagne | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $Manifeste -Encoding UTF8
    if (-not $Existe) {
        throw "$Nom : QEMU ou la preverification a echoue sans rapport (code $Code)."
    }
    foreach ($Cle in @('ladybird','qemu','ovmf-code','ovmf-vars',"$Bras-noyau")) {
        $Valeur = switch ($Cle) {
            'ladybird' { $Run.ladybird_sha256 }
            'qemu' { $Run.qemu_sha256 }
            'ovmf-code' { $Run.ovmf_code_sha256 }
            'ovmf-vars' { $Run.ovmf_vars_sha256 }
            default { $Run.noyau_sha256 }
        }
        if (-not $Valeur) { throw "$Nom : empreinte manquante ($Cle)." }
        if ($Empreintes.ContainsKey($Cle) -and $Empreintes[$Cle] -ne $Valeur) {
            throw "$Nom : reference changee ($Cle). Rapports conserves."
        }
        $Empreintes[$Cle] = $Valeur
    }
    if ($Code -ne 0 -or ($Bras -eq 'virgl' -and $Run.verdict -ne 'virgl-invite-confirme')) {
        throw "$Nom : $($Run.verdict), code $Code. Voir $Rapport."
    }
}
if (-not $DryRun) { Write-Host "Manifeste et chemins de logs : $Manifeste" }
