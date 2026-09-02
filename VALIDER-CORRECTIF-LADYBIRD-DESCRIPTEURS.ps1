param(
    [switch]$Bootimage,
    [switch]$Complet
)

$ErrorActionPreference = "Stop"
$Root = $PSScriptRoot
$AncienEmplacement = Get-Location

function Execute([string]$Nom, [scriptblock]$Action) {
    Write-Host "`n=== $Nom ===" -ForegroundColor Cyan
    $global:LASTEXITCODE = 0
    & $Action
    if ($LASTEXITCODE -ne 0) {
        throw "$Nom a echoue (code $LASTEXITCODE)"
    }
}

try {
    Set-Location $Root
    if (-not (Test-Path ".\Cargo.toml")) {
        throw "Ce script doit rester a la racine de bouchaud-os"
    }

    Execute "garde-fou descripteurs partages" {
        & python .\tools\verifie-descripteurs-partages.py
    }

    Execute "git diff --check" {
        & git diff --check
    }

    if ($Complet) {
        Execute "validation rapide complete" {
            & .\tools\dev\validate-fast.ps1 -Bootimage:$Bootimage
        }
    }
    else {
        Execute "cargo check" {
            & cargo check
        }
        if ($Bootimage) {
            Execute "cargo bootimage" {
                & cargo bootimage
            }
        }
    }

    Write-Host "`nCORRECTIF_LADYBIRD_DESCRIPTEURS_OK" -ForegroundColor Green
}
finally {
    Set-Location $AncienEmplacement
}
