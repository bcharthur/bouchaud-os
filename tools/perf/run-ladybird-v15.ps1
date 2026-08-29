param(
    [string]$Url = "https://www.google.com/",
    [ValidateRange(2048, 16384)][int]$RamMiB = 12288,
    [ValidateSet(1,4,8)][int]$CpuCount = 4,
    [ValidateSet("tcg","whpx","auto")][string]$Accel = "tcg",
    [switch]$Audio,
    [switch]$RefreshLadybird
)

$ErrorActionPreference = "Stop"
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$Run = Join-Path $RepoRoot "run.ps1"
if (-not (Test-Path $Run)) { throw "run.ps1 introuvable: $Run" }

$AudioBackend = if ($Audio) { "dsound" } else { "none" }
Write-Host "V15 Ladybird / SMP" -ForegroundColor Cyan
Write-Host "vCPU        : $CpuCount" -ForegroundColor DarkGray
Write-Host "Accel       : $Accel" -ForegroundColor DarkGray
Write-Host "RAM         : $RamMiB MiB" -ForegroundColor DarkGray
Write-Host "Audio       : $AudioBackend" -ForegroundColor DarkGray
if ($CpuCount -eq 8) {
    Write-Host "SMP8 est permis par MAX_CPUS=16 mais reste un profil de validation." -ForegroundColor Yellow
}
if ($Accel -eq "whpx" -and $CpuCount -gt 1) {
    Write-Host "WHPX SMP est experimental: valider d'abord TCG SMP4." -ForegroundColor Yellow
}

# V15.1 RUNNER FIX
# ----------------
# Ne PAS construire une liste de chaines du genre
#   @("-Ladybird", "-LadybirdUrl", $Url, ...)
# puis faire `& $Run @liste`.
#
# Pour une commande PowerShell, le splatting d'un TABLEAU est positionnel :
# les chaines "-LadybirdUrl", "-RamMiB", etc. contenues dans ce tableau ne
# redeviennent pas magiquement des noms de parametres. Le binder de run.ps1 les
# affecte alors aux parametres positionnels successifs; c'est ainsi que
# `-LadybirdUrl` finissait dans Gate0SerialPort et echouait en conversion Int32.
#
# Un hashtable splatte les parametres PAR NOM et evite aussi d'utiliser `$Args`,
# nom qui entre en collision (PowerShell est insensible a la casse) avec la
# variable automatique `$args`.
$RunParams = @{
    Ladybird    = $true
    LadybirdUrl = $Url
    RamMiB      = $RamMiB
    CpuCount    = $CpuCount
    Accel       = $Accel
    Audio       = $AudioBackend
}
if ($RefreshLadybird) {
    $RunParams.RefreshLadybird = $true
}

& $Run @RunParams
exit $LASTEXITCODE
