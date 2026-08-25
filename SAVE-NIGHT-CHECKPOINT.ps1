param(
    [switch]$Recheck
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Fail([string]$Message) {
    throw "[Bouchaud night checkpoint v2] $Message"
}

if (-not (Test-Path ".git")) {
    Fail "Run this script from the bouchaud-os repository root."
}

$origin = (git remote get-url origin).Trim()
if ([string]::IsNullOrWhiteSpace($origin)) {
    Fail "Git remote 'origin' is missing."
}

$currentBranch = (git branch --show-current).Trim()
$currentHead = (git rev-parse HEAD).Trim()

# The first checkpoint attempt already created this branch locally before it
# failed. Reuse it instead of trying to create another branch.
if ($currentBranch -like "checkpoint/p0-working-before-p1-*") {
    $branch = $currentBranch
} else {
    $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $branch = "checkpoint/p0-working-before-p1-$stamp"
    git switch -c $branch
    if ($LASTEXITCODE -ne 0) {
        Fail "Could not create checkpoint branch $branch"
    }
}

Write-Host ""
Write-Host "=== Bouchaud OS night checkpoint v2 ===" -ForegroundColor Cyan
Write-Host "Origin     : $origin"
Write-Host "Branch     : $branch"
Write-Host "Base HEAD  : $currentHead"
Write-Host ""

$Utf8Strict = New-Object System.Text.UTF8Encoding($false, $true)

function Read-Utf8Strict([string]$Path) {
    $bytes = [System.IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $Path).Path)
    return $Utf8Strict.GetString($bytes)
}

# ---------------------------------------------------------------------------
# 1. Prove that the source is still the validated P0 state.
# ---------------------------------------------------------------------------

$threadPath = "src\kernel\process\thread.rs"
$bklPath = "src\kernel\sync\bkl.rs"

foreach ($path in @($threadPath, $bklPath)) {
    if (-not (Test-Path -LiteralPath $path)) {
        Fail "Missing source file: $path"
    }
}

$thread = Read-Utf8Strict $threadPath
$bkl = Read-Utf8Strict $bklPath

foreach ($required in @(
    "BOUCHAUD_P0_TARGETED_SCHED_IPI_V1",
    "BOUCHAUD_P0_IDLE_WAKE_HANDSHAKE_V14"
)) {
    if (-not $thread.Contains($required)) {
        Fail "Validated P0 marker missing from thread.rs: $required"
    }
}

if (-not $bkl.Contains("BOUCHAUD_P0_TARGETED_IPI_LIVENESS_V13")) {
    Fail "Validated P0 v1.3 marker missing from bkl.rs"
}

if ($thread.Contains("BOUCHAUD_P1_KERNEL_CONCURRENCY_V1") -or
    $bkl.Contains("BOUCHAUD_P1_KERNEL_CONCURRENCY_V1")) {
    Fail "P1 source marker found. This script only checkpoints the validated P0 state."
}

Write-Host "[OK] P0 v1.2/v1.3/v1.4 source state confirmed." -ForegroundColor Green
Write-Host "[OK] P1 source patch is not partially applied." -ForegroundColor Green

# The fixed P1 script from the v1.1 package must be present so tomorrow's clone
# has the corrected apply logic.
$p1Apply = "APPLY-P1-KERNEL-CONCURRENCY-V1.ps1"
if (-not (Test-Path -LiteralPath $p1Apply)) {
    Fail "Missing $p1Apply"
}
$p1ApplyText = Read-Utf8Strict $p1Apply
if (-not $p1ApplyText.Contains("1.1-night-checkpoint")) {
    Fail "P1 apply script is not the corrected v1.1 copy."
}
Write-Host "[OK] Corrected P1 v1.1 script is present." -ForegroundColor Green

git diff --check
if ($LASTEXITCODE -ne 0) {
    Fail "git diff --check failed."
}

if ($Recheck) {
    Write-Host ""
    Write-Host "[BUILD] cargo check" -ForegroundColor Cyan
    cargo check
    if ($LASTEXITCODE -ne 0) {
        Fail "cargo check failed."
    }
}

# ---------------------------------------------------------------------------
# 2. Write a small portable manifest into the repository itself.
# ---------------------------------------------------------------------------

$manifestPath = "docs\history\CHECKPOINT_P0_BEFORE_P1.md"
$manifestDir = Split-Path -Parent $manifestPath
New-Item -ItemType Directory -Force -Path $manifestDir | Out-Null

$manifest = @"
# Bouchaud OS - validated P0 checkpoint before P1

Checkpoint branch: `$branch`

## Runtime-validated state

- P0 targeted scheduler IPI enabled.
- P0 BKL resume liveness v1.3 enabled.
- P0 scheduler idle/wake handshake v1.4 enabled.
- Ladybird reached and rendered `https://example.com/`.
- P1 kernel-concurrency source patch has **not** been applied yet.
- Corrected P1 PowerShell apply/verify scripts are included for the next session.

## Next session

```powershell
cargo check
.\APPLY-P1-KERNEL-CONCURRENCY-V1.ps1 -Preview
.\APPLY-P1-KERNEL-CONCURRENCY-V1.ps1
.\VERIFY-P1-KERNEL-CONCURRENCY-V1.ps1 -Build
.\run.ps1
```

## Local artifacts intentionally not stored in Git

The following may exist on the original workstation but are intentionally not
part of this Git checkpoint:

- `target/`
- `.idea/`
- `.bouchaud-history/`
- `ladybird-browser.img`
- `native-browser-m9/`
- `scenario-m9/`
- `BOUCHAUD-SMP4-KNOWN-GOOD/`

They are build/debug/runtime artifacts or historical recovery material. If the
second workstation needs a generated Ladybird image, rebuild it there from the
project tooling rather than committing the 1.4 GB image.
"@

$Utf8NoBom = New-Object System.Text.UTF8Encoding($false)
[System.IO.File]::WriteAllText(
    (Join-Path (Get-Location) $manifestPath),
    $manifest,
    $Utf8NoBom
)

# ---------------------------------------------------------------------------
# 3. Stage the source/project state.
# ---------------------------------------------------------------------------

# Preserve modifications/deletions to files Git already knows.
git add -u
if ($LASTEXITCODE -ne 0) {
    Fail "git add -u failed."
}

$projectPaths = @(
    "src",
    ".cargo",
    ".github",
    "docs",
    "internal",
    "tools",
    "userland",
    "targets",
    "tests",
    "Cargo.toml",
    "Cargo.lock",
    "README.md",
    "rust-toolchain.toml",
    "check.ps1",
    "run.ps1",
    "run-browser-host.ps1",
    "run-classic-local.ps1",
    "run-ladybird-local.ps1",
    "migrate-bouchaud-checkpoint.ps1",
    "SAVE-NIGHT-CHECKPOINT.ps1",
    "TOMORROW-RESTORE-CHECKPOINT.ps1",
    "README-NIGHT-CHECKPOINT.txt",
    "README-NIGHT-CHECKPOINT-V2.txt"
)

foreach ($path in $projectPaths) {
    if (Test-Path -LiteralPath $path) {
        git add -- $path
        if ($LASTEXITCODE -ne 0) {
            Fail "git add failed for $path"
        }
    }
}

# Preserve the milestone scripts and their documentation.
Get-ChildItem -File -Force |
    Where-Object {
        $_.Name -like "APPLY-P0-*" -or
        $_.Name -like "FIX-P0-*" -or
        $_.Name -like "REPAIR-P0-*" -or
        $_.Name -like "VERIFY-P0-*" -or
        $_.Name -like "ROLLBACK-P0-*" -or
        $_.Name -like "README-P0-*" -or
        $_.Name -like "P0-*.patch.txt" -or
        $_.Name -like "APPLY-P1-*" -or
        $_.Name -like "VERIFY-P1-*" -or
        $_.Name -like "ROLLBACK-P1-*" -or
        $_.Name -like "README-P1-*"
    } |
    ForEach-Object {
        git add -- $_.Name
        if ($LASTEXITCODE -ne 0) {
            Fail "git add failed for $($_.Name)"
        }
    }

# ---------------------------------------------------------------------------
# 4. Remove generated/local paths ONLY if they are actually staged.
#
# The v1 script called `git restore --staged ladybird-browser.img` even though
# that file is untracked. Git returned pathspec failure and PowerShell stopped.
# Enumerating the real staged index first makes this operation total and safe.
# ---------------------------------------------------------------------------

$stagedNow = @(git diff --cached --name-only)

foreach ($name in $stagedNow) {
    if ([string]::IsNullOrWhiteSpace($name)) {
        continue
    }

    $normalized = $name.Replace("\", "/")
    $exclude =
        $normalized -eq "ladybird-browser.img" -or
        $normalized -eq ".idea" -or
        $normalized.StartsWith(".idea/") -or
        $normalized -eq "target" -or
        $normalized.StartsWith("target/") -or
        $normalized -eq ".bouchaud-history" -or
        $normalized.StartsWith(".bouchaud-history/") -or
        $normalized -eq "native-browser-m9" -or
        $normalized.StartsWith("native-browser-m9/") -or
        $normalized -eq "scenario-m9" -or
        $normalized.StartsWith("scenario-m9/") -or
        $normalized -eq "BOUCHAUD-SMP4-KNOWN-GOOD" -or
        $normalized.StartsWith("BOUCHAUD-SMP4-KNOWN-GOOD/")

    if ($exclude) {
        Write-Host "[UNSTAGE local artifact] $name" -ForegroundColor DarkYellow
        git reset -q HEAD -- "$name"
        if ($LASTEXITCODE -ne 0) {
            Fail "Could not unstage local artifact: $name"
        }
    }
}

# Refuse giant files rather than discovering GitHub's limit during push.
$oversized = @()
$stagedFiles = @(git diff --cached --name-only --diff-filter=ACMR)

foreach ($name in $stagedFiles) {
    if ([string]::IsNullOrWhiteSpace($name)) {
        continue
    }
    if (Test-Path -LiteralPath $name -PathType Leaf) {
        $size = (Get-Item -LiteralPath $name).Length
        if ($size -gt 50MB) {
            $oversized += "$name ($([math]::Round($size / 1MB, 1)) MB)"
        }
    }
}

if ($oversized.Count -gt 0) {
    Write-Host ""
    Write-Host "Oversized staged files:" -ForegroundColor Red
    $oversized | ForEach-Object { Write-Host "  $_" -ForegroundColor Red }
    Fail "Refusing to push files larger than 50 MB."
}

Write-Host ""
Write-Host "Staged checkpoint:" -ForegroundColor Cyan
git diff --cached --stat

$stagedCount = @(git diff --cached --name-only).Count

if ($stagedCount -eq 0) {
    Write-Host "[INFO] No staged delta; creating an empty checkpoint commit." -ForegroundColor Yellow
    git commit --allow-empty -m "checkpoint: validated P0 SMP state before P1 concurrency"
} else {
    git commit -m "checkpoint: validated P0 SMP state before P1 concurrency"
}

if ($LASTEXITCODE -ne 0) {
    Fail "git commit failed."
}

$checkpointHead = (git rev-parse HEAD).Trim()

# ---------------------------------------------------------------------------
# 5. Push and prove the remote branch exists at the same commit.
# ---------------------------------------------------------------------------

Write-Host ""
Write-Host "[PUSH] origin/$branch" -ForegroundColor Cyan
git push -u origin $branch
if ($LASTEXITCODE -ne 0) {
    Fail "git push failed. Local checkpoint commit is still safe on $branch."
}

$remoteLine = (git ls-remote --heads origin "refs/heads/$branch").Trim()
if ([string]::IsNullOrWhiteSpace($remoteLine)) {
    Fail "Push returned success but remote branch could not be verified."
}

$remoteSha = ($remoteLine -split "\s+")[0]
if ($remoteSha -ne $checkpointHead) {
    Fail "Remote SHA $remoteSha does not match local checkpoint $checkpointHead"
}

Write-Host ""
Write-Host "==================================================" -ForegroundColor Green
Write-Host " REMOTE CHECKPOINT VERIFIED - SAFE TO SHUT DOWN " -ForegroundColor Green
Write-Host "==================================================" -ForegroundColor Green
Write-Host ""
Write-Host "Branch : $branch" -ForegroundColor Cyan
Write-Host "Commit : $checkpointHead" -ForegroundColor Cyan
Write-Host ""
Write-Host "Tomorrow on the other PC:" -ForegroundColor Yellow
Write-Host "  git clone $origin"
Write-Host "  cd bouchaud-os"
Write-Host "  git fetch origin"
Write-Host "  git switch $branch"
Write-Host ""
Write-Host "Then:" -ForegroundColor Yellow
Write-Host "  cargo check"
Write-Host "  .\APPLY-P1-KERNEL-CONCURRENCY-V1.ps1 -Preview"
Write-Host "  .\APPLY-P1-KERNEL-CONCURRENCY-V1.ps1"
Write-Host "  .\VERIFY-P1-KERNEL-CONCURRENCY-V1.ps1 -Build"
