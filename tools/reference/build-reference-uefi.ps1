param(
    [string]$Output = ""
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $RepoRoot

function Fail([string]$Message) {
    Write-Host ""
    Write-Host "ERREUR: $Message" -ForegroundColor Red
    exit 1
}

Write-Host "=== Bouchaud Reference Device - build UEFI Stage 1 ===" -ForegroundColor Cyan

& cargo build --no-default-features --features "uefi-boot,reference-bringup"
if ($LASTEXITCODE -ne 0) {
    Fail "build noyau UEFI en echec"
}

$KernelCandidates = @(
    (Join-Path $RepoRoot "target\x86_64-bouchaud_os\debug\bouchaud-os"),
    (Join-Path $RepoRoot "target\x86_64-bouchaud_os\debug\bouchaud-os.exe")
)
$Kernel = $KernelCandidates | Where-Object {
    Test-Path -LiteralPath $_ -PathType Leaf
} | Select-Object -First 1

if (-not $Kernel) {
    Fail "ELF noyau UEFI introuvable dans target\x86_64-bouchaud_os\debug"
}

if (-not $Output) {
    $Output = Join-Path $RepoRoot "target\reference\bouchaud-reference-uefi-stage1.img"
}
$Output = [System.IO.Path]::GetFullPath($Output)

$BuilderSource = Join-Path $RepoRoot "tools\reference\uefi-image-builder"
$BuilderTemp = Join-Path $env:TEMP "bouchaud-reference-uefi-image-builder"
if (Test-Path -LiteralPath $BuilderTemp) {
    Remove-Item -Recurse -Force $BuilderTemp
}
Copy-Item -Recurse -Force $BuilderSource $BuilderTemp

$BuilderTarget = Join-Path $RepoRoot "target\reference\image-builder-target"

Push-Location $BuilderTemp
try {
    & cargo +nightly-2026-06-01 run `
        --release `
        --target-dir "$BuilderTarget" `
        -- "$Kernel" "$Output"

    $Code = $LASTEXITCODE
}
finally {
    Pop-Location
}

if ($Code -ne 0) {
    Fail "creation image UEFI en echec"
}

if (-not (Test-Path -LiteralPath $Output -PathType Leaf)) {
    Fail "image attendue absente: $Output"
}

$Size = (Get-Item -LiteralPath $Output).Length
Write-Host ""
Write-Host "UEFI_IMAGE_PATH=$Output" -ForegroundColor Green
Write-Host "UEFI_IMAGE_BYTES=$Size"
Write-Host "BOUCHAUD_REFERENCE_UEFI_IMAGE_READY"
