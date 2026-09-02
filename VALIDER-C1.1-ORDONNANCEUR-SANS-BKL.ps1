param(
    [switch]$SansQemu,
    [string]$DistributionWsl = "Ubuntu"
)

$ErrorActionPreference = "Stop"
$RepoRoot = (Resolve-Path $PSScriptRoot).Path
$Base = "6d67ef8d1d350965e169b319eae2de8c682df5b0"
$OldLocation = Get-Location

function Invoke-Checked([string]$Name, [scriptblock]$Action) {
    Write-Host "`n=== $Name ===" -ForegroundColor Cyan
    & $Action
    if ($LASTEXITCODE -ne $null -and $LASTEXITCODE -ne 0) {
        throw "$Name a echoue (code $LASTEXITCODE)"
    }
}

function Quote-Bash([string]$Value) {
    return "'" + $Value.Replace("'", "'`"'`"'") + "'"
}

try {
    Set-Location $RepoRoot
    if (-not (Test-Path ".\Cargo.toml")) {
        throw "Ce validateur doit rester a la racine de bouchaud-os"
    }

    & git cat-file -e "$Base^{commit}"
    if ($LASTEXITCODE -ne 0) {
        throw "Base C1.1 absente du depot : $Base"
    }

    Invoke-Checked "diff indexable" { & git diff --check }

    $SourceGuards = @(
        "tools/verifie-ordonnanceur-sans-bkl.py",
        "tools/verifie-domaines-bkl.py",
        "tools/verifie-rangs-verrous.py",
        "tools/verifie-registre-taches.py",
        "tools/verifie-etat-bkl-atomique.py",
        "tools/verifie-bkl-comptes.py",
        "tools/verifie-bkl-parking.py",
        "tools/verifie-echeances.py",
        "tools/verifie-poignee-idle.py"
    )
    foreach ($Guard in $SourceGuards) {
        Invoke-Checked "garde-fou $Guard" { & python $Guard }
    }

    Invoke-Checked "budgets BKL source" { & python tools/ci/check_budgets.py }

    Invoke-Checked "suites hote, cargo check et bootimage" {
        $PowerShellExe = (Get-Process -Id $PID).Path
        & $PowerShellExe -NoProfile -ExecutionPolicy Bypass `
            -File ".\tools\dev\validate-fast.ps1" -Bootimage
    }

    if (-not $SansQemu) {
        if (-not (Get-Command wsl.exe -ErrorAction SilentlyContinue)) {
            throw "wsl.exe absent : installer WSL/Ubuntu ou relancer avec -SansQemu"
        }

        $BootWin = Join-Path $RepoRoot `
            "target\x86_64-bouchaud_os\debug\bootimage-bouchaud-os.bin"
        if (-not (Test-Path $BootWin -PathType Leaf)) {
            throw "Bootimage absente : $BootWin"
        }

        $RepoRaw = & wsl.exe -d $DistributionWsl -e wslpath -a -u $RepoRoot
        if ($LASTEXITCODE -ne 0 -or -not $RepoRaw) {
            throw "Conversion WSL de la racine impossible"
        }
        $BootRaw = & wsl.exe -d $DistributionWsl -e wslpath -a -u $BootWin
        if ($LASTEXITCODE -ne 0 -or -not $BootRaw) {
            throw "Conversion WSL de la bootimage impossible"
        }
        $RepoWsl = (($RepoRaw | Select-Object -First 1) -replace "`0", "").Trim()
        $BootWsl = (($BootRaw | Select-Object -First 1) -replace "`0", "").Trim()

        $Runtime = "set -euo pipefail; " +
            "for c in bash python3 musl-gcc qemu-system-x86_64 timeout file readelf sha256sum; do " +
            "command -v `$c >/dev/null || { echo OUTIL_HOTE_MANQUANT:`$c >&2; exit 127; }; done; " +
            "cd $(Quote-Bash $RepoWsl); bash tools/ci/run_mm_ng6.sh $(Quote-Bash $BootWsl)"

        Invoke-Checked "runtime Bouchaud / QEMU TCG / SMP4 / mm-ng6" {
            & wsl.exe -d $DistributionWsl -e bash -lc $Runtime
        }
    }

    Write-Host "`n=== ETAT FINAL ===" -ForegroundColor Cyan
    & git status --short

    if ($SansQemu) {
        Write-Host "`nC1_1_HOTE_OK_RUNTIME_NON_EXECUTE" -ForegroundColor Yellow
    } else {
        Write-Host "`nC1_1_ORDONNANCEUR_SANS_BKL_OK" -ForegroundColor Green
    }
}
finally {
    Set-Location $OldLocation
}
