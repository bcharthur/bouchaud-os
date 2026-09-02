param(
    [switch]$SansQemu,
    [string]$DistributionWsl = "Ubuntu"
)

$ErrorActionPreference = "Stop"
$RepoRoot = (Resolve-Path $PSScriptRoot).Path
$ExpectedHead = "6c8cfb2a4b7a24173778a0361dfc2dda6f2301b7"
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
        throw "Ce script doit rester a la racine de bouchaud-os"
    }

    $Head = (& git rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or $Head -ne $ExpectedHead) {
        throw "Overlay prevu pour HEAD 6c8cfb2 ; HEAD courant : $($Head.Substring(0, [Math]::Min(7, $Head.Length)))"
    }

    Write-Host "Bouchaud OS - validation correctif C1/SMP" -ForegroundColor Green
    Write-Host "repo : $RepoRoot"
    Write-Host "base : $($Head.Substring(0, 7))"
    Write-Host "WSL reste un outil hote : aucun composant Windows/Linux n'est ajoute a l'OS."

    Invoke-Checked "diff indexable" {
        & git diff --check
    }

    $SourceGuards = @(
        "tools/verifie-etat-bkl-atomique.py",
        "tools/verifie-registre-taches.py",
        "tools/verifie-shootdown-tlb.py",
        "tools/verifie-poignee-idle.py",
        "tools/verifie-bkl-comptes.py",
        "tools/verifie-bkl-parking.py",
        "tools/verifie-domaines-bkl.py",
        "tools/verifie-rangs-verrous.py",
        "tools/verifie-verrouillage.py"
    )
    foreach ($Guard in $SourceGuards) {
        Invoke-Checked "garde-fou $Guard" {
            & python $Guard
        }
    }

    # Compile les 34 suites hote, les nouvelles falsifications SMP, le noyau et
    # la bootimage bare-metal. Ce script ne lance aucun binaire produit dans
    # l'OS hote : les .rs de tools/smp sont des modeles de concurrence.
    Invoke-Checked "validation rapide complete + bootimage" {
        # `validate-fast.ps1` termine par `exit` pour la CI. Un processus
        # enfant empeche ce `exit` de court-circuiter la preuve QEMU ci-dessous.
        $PowerShellExe = (Get-Process -Id $PID).Path
        & $PowerShellExe -NoProfile -ExecutionPolicy Bypass `
            -File ".\tools\dev\validate-fast.ps1" -Bootimage
    }

    if (-not $SansQemu) {
        $Wsl = Get-Command wsl.exe -ErrorAction SilentlyContinue
        if (-not $Wsl) {
            throw "wsl.exe absent : relancer avec -SansQemu ou utiliser le job Integration mm-ng6"
        }

        $BootWin = Join-Path $RepoRoot "target\x86_64-bouchaud_os\debug\bootimage-bouchaud-os.bin"
        if (-not (Test-Path $BootWin)) {
            throw "Bootimage absente apres cargo bootimage : $BootWin"
        }

        $RepoWsl = (& wsl.exe -d $DistributionWsl -e wslpath -a -u $RepoRoot).Trim()
        if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($RepoWsl)) {
            throw "WSL ne sait pas traduire la racine du depot"
        }
        $BootWsl = (& wsl.exe -d $DistributionWsl -e wslpath -a -u $BootWin).Trim()
        if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($BootWsl)) {
            throw "WSL ne sait pas traduire le chemin de la bootimage"
        }

        $RepoArg = Quote-Bash $RepoWsl
        $BootArg = Quote-Bash $BootWsl
        $Runtime = "set -euo pipefail; " +
            "for c in bash python3 musl-gcc qemu-system-x86_64 timeout file readelf sha256sum; do " +
            "command -v `$c >/dev/null || { echo OUTIL_HOTE_MANQUANT:`$c >&2; exit 127; }; done; " +
            "cd $RepoArg; bash tools/ci/run_mm_ng6.sh $BootArg"

        Write-Host "`n=== runtime Bouchaud OS / QEMU TCG / SMP4 / mm-ng6 ===" -ForegroundColor Cyan
        Write-Host "WSL construit la sonde et pilote QEMU ; le code teste reste le noyau bare-metal Bouchaud."
        & wsl.exe -d $DistributionWsl -e bash -lc $Runtime
        if ($LASTEXITCODE -ne 0) {
            throw "mm-ng6 SMP4 a echoue (code $LASTEXITCODE)"
        }
    }

    Write-Host "`n=== ETAT GIT APRES VALIDATION ===" -ForegroundColor Cyan
    & git status --short
    & git diff --stat

    if ($SansQemu) {
        Write-Host "`nVALIDATION HOTE OK ; PREUVE RUNTIME NON EXECUTEE (-SansQemu)" -ForegroundColor Yellow
    } else {
        Write-Host "`nC1_SMP_MM_NG6_OK" -ForegroundColor Green
    }
}
finally {
    Set-Location $OldLocation
}
