param(
    [string]$Output = "",
    [ValidateRange(0, 8192)]
    [int]$MinWidth = 0,
    [ValidateRange(0, 4320)]
    [int]$MinHeight = 0,
    [string]$Ramdisk = ""
)

if (($MinWidth -eq 0) -xor ($MinHeight -eq 0)) {
    throw "MinWidth et MinHeight doivent etre tous deux nuls ou tous deux non nuls"
}

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $RepoRoot

function Fail([string]$Message) {
    Write-Host ""
    Write-Host "ERREUR: $Message" -ForegroundColor Red
    exit 1
}

$UefiTarget = Join-Path $RepoRoot "targets\x86_64-bouchaud_os_uefi.json"

Write-Host "=== Bouchaud OS - Stage 2 Physical Desktop ===" -ForegroundColor Cyan

& cargo build `
    --target "$UefiTarget" `
    --no-default-features `
    --features "uefi-boot,reference-bringup,reference-desktop"

if ($LASTEXITCODE -ne 0) { Fail "build noyau Stage 2 en echec" }

$KernelCandidates = @(
    (Join-Path $RepoRoot "target\x86_64-bouchaud_os_uefi\debug\bouchaud-os"),
    (Join-Path $RepoRoot "target\x86_64-bouchaud_os_uefi\debug\bouchaud-os.exe")
)
$Kernel = $KernelCandidates | Where-Object {
    Test-Path -LiteralPath $_ -PathType Leaf
} | Select-Object -First 1

if (-not $Kernel) { Fail "ELF Stage 2 introuvable" }

& python ".\tools\reference\verifie-reference-uefi-elf.py" "$Kernel"
if ($LASTEXITCODE -ne 0) { Fail "le noyau Stage 2 n'est pas ET_DYN" }

if (-not $Output) {
    $Output = Join-Path $RepoRoot "target\reference\bouchaud-reference-stage2.img"
}
$Output = [System.IO.Path]::GetFullPath($Output)

if ($Ramdisk) {
    $Ramdisk = [System.IO.Path]::GetFullPath($Ramdisk)
    if (-not (Test-Path -LiteralPath $Ramdisk -PathType Leaf)) {
        Fail "ramdisk Stage 2 absent: $Ramdisk"
    }
}

$BuilderSource = Join-Path $RepoRoot "tools\reference\uefi-image-builder"
$BuilderTemp = Join-Path $env:TEMP "bouchaud-reference-stage2-image-builder"
if (Test-Path -LiteralPath $BuilderTemp) {
    Remove-Item -Recurse -Force $BuilderTemp
}
Copy-Item -Recurse -Force $BuilderSource $BuilderTemp

$BuilderTarget = Join-Path $RepoRoot "target\reference\image-builder-target"

Push-Location $BuilderTemp
try {
    if ($Ramdisk -and $MinWidth -gt 0) {
        Write-Host "STAGE2_GOP_MIN_REQUEST=${MinWidth}x${MinHeight}"
        Write-Host "STAGE2_RAMDISK_REQUEST=$Ramdisk"
        & cargo +nightly-2026-06-01 run `
            --release `
            --target-dir "$BuilderTarget" `
            -- "$Kernel" "$Output" "$MinWidth" "$MinHeight" "$Ramdisk"
    }
    elseif ($Ramdisk) {
        Write-Host "STAGE2_RAMDISK_REQUEST=$Ramdisk"
        & cargo +nightly-2026-06-01 run `
            --release `
            --target-dir "$BuilderTarget" `
            -- "$Kernel" "$Output" "$Ramdisk"
    }
    elseif ($MinWidth -gt 0) {
        Write-Host "STAGE2_GOP_MIN_REQUEST=${MinWidth}x${MinHeight}"
        & cargo +nightly-2026-06-01 run `
            --release `
            --target-dir "$BuilderTarget" `
            -- "$Kernel" "$Output" "$MinWidth" "$MinHeight"
    }
    else {
        & cargo +nightly-2026-06-01 run `
            --release `
            --target-dir "$BuilderTarget" `
            -- "$Kernel" "$Output"
    }

    $Code = $LASTEXITCODE
}
finally {
    Pop-Location
}

if ($Code -ne 0) { Fail "creation image Stage 2 en echec" }
if (-not (Test-Path -LiteralPath $Output -PathType Leaf)) {
    Fail "image Stage 2 absente: $Output"
}

$Size = (Get-Item -LiteralPath $Output).Length
$Hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $Output).Hash

Write-Host ""
Write-Host "STAGE2_KERNEL=$Kernel"
Write-Host "STAGE2_ELF=ET_DYN"
Write-Host "STAGE2_IMAGE=$Output" -ForegroundColor Green
Write-Host "STAGE2_IMAGE_BYTES=$Size"
Write-Host "STAGE2_IMAGE_SHA256=$Hash"

if ($MinWidth -gt 0) {
    Write-Host "STAGE2_GOP_MIN=${MinWidth}x${MinHeight}"
}

if ($Ramdisk) {
    $RamdiskHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $Ramdisk).Hash
    Write-Host "STAGE2_RAMDISK=$Ramdisk"
    Write-Host "STAGE2_RAMDISK_SHA256=$RamdiskHash"
    Write-Host "BOUCHAUD_STAGE2_SINGLE_USB_RAMDISK_READY"
}

Write-Host "BOUCHAUD_STAGE2_IMAGE_READY"
