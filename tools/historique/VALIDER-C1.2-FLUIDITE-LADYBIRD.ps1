param(
    [string]$RepoRoot = ".",
    [string]$Journal = "",
    [switch]$SansBootimage
)

$ErrorActionPreference = "Stop"
$Root = (Resolve-Path $RepoRoot).Path
$Temp = Join-Path ([System.IO.Path]::GetTempPath()) ("bouchaud-c1-2-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $Temp | Out-Null

function Invoke-Etape([string]$Nom, [scriptblock]$Action) {
    Write-Host "`n=== $Nom ===" -ForegroundColor Cyan
    & $Action
    if ($LASTEXITCODE -ne 0) {
        throw "$Nom a echoue (code $LASTEXITCODE)"
    }
}

try {
    Set-Location $Root
    Write-Host "C1.2 : RSS O(1) + grappes de fautes groupees" -ForegroundColor Green
    Write-Host "repo   : $Root"
    Write-Host "commit : $(& git rev-parse --short HEAD)"

    Invoke-Etape "base d780b8f" {
        & git merge-base --is-ancestor d780b8f HEAD
    }

    Invoke-Etape "garde-fou source" {
        & python .\tools\verifie-rss-incremental.py
    }

    Invoke-Etape "contrat Ladybird / Skia" {
        & python .\tools\dev\verifie-v15.py
    }

    Invoke-Etape "modele hote" {
        $Exe = Join-Path $Temp "test-rss-incremental.exe"
        & rustc --edition 2021 --test -O -o $Exe .\tools\smp\test_rss_incremental.rs
        if ($LASTEXITCODE -eq 0) { & $Exe }
    }

    Invoke-Etape "diff indexable" {
        & git diff --check -- `
            src/kernel/memory/virtual.rs `
            src/kernel/process/elf.rs `
            src/kernel/process/resource.rs `
            src/kernel/process/thread/faute_cluster.rs `
            src/kernel/process/thread/faute_memoire.rs `
            src/kernel/process/thread/metriques.rs `
            tools/dev/validate-fast.ps1 `
            tools/smp/test_rss_incremental.rs `
            tools/verifie-rss-incremental.py `
            tools/perf/analyse-fluidite-c1-2.py
    }

    Invoke-Etape "cargo check" { & cargo check }
    if (-not $SansBootimage) {
        Invoke-Etape "cargo bootimage" { & cargo bootimage }
    }

    if ($Journal) {
        $JournalPath = (Resolve-Path $Journal).Path
        Invoke-Etape "preuve runtime" {
            & python .\tools\perf\analyse-fluidite-c1-2.py $JournalPath
        }
    }

    Write-Host "`nC1_2_FLUIDITE_LADYBIRD_OK" -ForegroundColor Green
    if (-not $Journal) {
        Write-Host "Validation runtime restante :"
        Write-Host '  .\run.ps1 2>&1 | Tee-Object .\ladybird-c1-2.log'
        Write-Host '  .\VALIDER-C1.2-FLUIDITE-LADYBIRD.ps1 -Journal .\ladybird-c1-2.log -SansBootimage'
    }
}
finally {
    Set-Location $PSScriptRoot
    if (Test-Path $Temp) {
        Remove-Item -Recurse -Force $Temp
    }
}
