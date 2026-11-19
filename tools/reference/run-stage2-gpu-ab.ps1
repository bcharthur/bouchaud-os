# Campagne Stage 2 GPU à disques en snapshot. Les interactions sont humaines.
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
$Empreintes = @{}
for ($tour = 1; $tour -le $Repetitions; $tour++) {
    $Ordre = if ($tour % 2 -eq 1) { @('cpu','virgl') } else { @('virgl','cpu') }
    foreach ($Bras in $Ordre) {
        $Nom = '{0:00}-{1}' -f $tour,$Bras
        $Rapport = Join-Path $Sortie "$Nom.json"
        $Options = @('-NoProfile','-File',$Profil,'-SkipBuild','-SkipPrepare',
            '-Smp',"$Smp",'-Accel',$Accel,'-Display',$Display,'-QualificationJson',$Rapport)
        if ($Bras -eq 'virgl') { $Options += '-Virgl' }
        Write-Host "$Nom : ouvrir Ladybird, charger $Url dans un onglet ; attendre 30 s puis faire trois PageDown."
        Write-Host "Fermer la fenetre, rouvrir Ladybird, recharger la meme URL puis quitter QEMU."
        if ($DryRun) { continue }
        & $HotePs @Options
        $Code = $LASTEXITCODE
        if (-not (Test-Path -LiteralPath $Rapport -PathType Leaf)) {
            throw "$Nom : QEMU ou la preverification a echoue sans rapport (code $Code)."
        }
        $Run = Get-Content -LiteralPath $Rapport -Raw | ConvertFrom-Json
        foreach ($Cle in @('ladybird','qemu',"$Bras-noyau")) {
            $Valeur = switch ($Cle) {
                'ladybird' { $Run.ladybird_sha256 }
                'qemu' { "$($Run.qemu) $($Run.version_qemu)" }
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
}
if (-not $DryRun) { Write-Host "Rapports et chemins de logs : $Sortie" }
