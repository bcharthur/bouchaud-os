<#
.SYNOPSIS
  BOUCHAUD_WINDOWS_V1 -- publie, promeut ou verifie une release Trigkey (GitHub).

.DESCRIPTION
  Ne signe RIEN localement : la signature a lieu dans le workflow
  .github/workflows/release-trigkey.yml, environnement protege
  `trigkey-testing` (votre approbation, votre secret). Ce script :

  -Publier -Version V -Sequence N : verifie que HEAD = origin/main, que le
    dossier est propre, puis declenche le workflow (action=publier). La garde
    du workflow refuse : qualification non verte au SHA exact, cle de
    production absente, sequence non croissante, etiquette existante,
    artefact Ladybird absent. Resultat : prerelease `trigkey-s<N>-<V>`.
  -Promouvoir -Etiquette T : declenche la promotion testing -> stable
    (environnement `trigkey-stable`, sans nouvelle signature). C'est CETTE
    action qui rend la release visible aux Trigkey (canal stable).
  -Verifier -Etiquette T : telecharge les artefacts de la release et les
    verifie contre les cles de production engagees dans le depot
    (tools/release/verifie_artefacts_release.py --production).

  Exige `gh` authentifie et python3.

.EXAMPLE
  .\tools\windows\Release-BouchaudOS.ps1 -Publier -Version 2026.11.20-1 -Sequence 1
  .\tools\windows\Release-BouchaudOS.ps1 -Promouvoir -Etiquette trigkey-s1-2026.11.20-1
  .\tools\windows\Release-BouchaudOS.ps1 -Verifier -Etiquette trigkey-s1-2026.11.20-1
#>
[CmdletBinding(DefaultParameterSetName = 'Verifier')]
param(
    [Parameter(ParameterSetName = 'Publier', Mandatory = $true)][switch]$Publier,
    [Parameter(ParameterSetName = 'Publier', Mandatory = $true)][string]$Version,
    [Parameter(ParameterSetName = 'Publier', Mandatory = $true)][int64]$Sequence,
    [Parameter(ParameterSetName = 'Promouvoir', Mandatory = $true)][switch]$Promouvoir,
    [Parameter(ParameterSetName = 'Verifier')][switch]$Verifier,
    [Parameter(ParameterSetName = 'Promouvoir', Mandatory = $true)]
    [Parameter(ParameterSetName = 'Verifier', Mandatory = $true)][string]$Etiquette
)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'BouchaudWindows.psm1') -Force
$journal = Start-JournalBouchaud 'release'
try {
    Assert-Commande 'gh' 'installer GitHub CLI puis gh auth login'
    $racine = Get-RacineDepot
    if ($PSCmdlet.ParameterSetName -eq 'Publier') {
        if ($Version -notmatch '^[A-Za-z0-9._+-]{1,64}$') { Stop-Bouchaud 'version invalide' }
        if ($Sequence -le 0) { Stop-Bouchaud 'sequence > 0 exigee' }
        $sha = Get-ShaGit
        Push-Location $racine
        try {
            & git fetch -q origin main
            $main = (& git rev-parse origin/main).Trim()
        } finally { Pop-Location }
        if ($sha -ne $main) { Stop-Bouchaud "HEAD ($sha) n'est pas origin/main ($main) : une release part de main" }
        Write-Etape "publication demandee : $Version sequence $Sequence au commit $sha"
        & gh workflow run release-trigkey.yml --ref main -f action=publier -f version=$Version -f sequence=$Sequence
        if ($LASTEXITCODE -ne 0) { Stop-Bouchaud 'gh workflow run a echoue' }
        Write-Host "BOUCHAUD_RELEASE_DEMANDEE etiquette=trigkey-s$Sequence-$Version commit=$sha : approuver l'environnement trigkey-testing dans GitHub Actions"
        return
    }
    if ($PSCmdlet.ParameterSetName -eq 'Promouvoir') {
        if ($Etiquette -notmatch '^trigkey-s[1-9][0-9]*-') { Stop-Bouchaud 'etiquette invalide' }
        & gh workflow run release-trigkey.yml --ref main -f action=promouvoir -f etiquette=$Etiquette
        if ($LASTEXITCODE -ne 0) { Stop-Bouchaud 'gh workflow run a echoue' }
        Write-Host "BOUCHAUD_PROMOTION_DEMANDEE etiquette=$Etiquette : approuver l'environnement trigkey-stable"
        return
    }
    # Verifier
    $d = Join-Path $racine "target/release-verifiee/$Etiquette"
    if (Test-Path -LiteralPath $d) { Remove-Item -LiteralPath $d -Recurse -Force }
    New-Item -ItemType Directory -Force -Path $d | Out-Null
    & gh release download $Etiquette --dir $d
    if ($LASTEXITCODE -ne 0) { Stop-Bouchaud "telechargement de $Etiquette en echec" }
    Assert-Commande 'python3' 'installer Python 3'
    & python3 (Join-Path $racine 'tools/release/verifie_artefacts_release.py') $d --production
    if ($LASTEXITCODE -ne 0) { Stop-Bouchaud "release $Etiquette NON conforme" }
    Write-Host "BOUCHAUD_RELEASE_VERIFIEE etiquette=$Etiquette journal=$journal"
} finally {
    Stop-Transcript | Out-Null
}
