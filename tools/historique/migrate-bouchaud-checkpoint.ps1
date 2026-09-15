param(
    [string]$Branch = "feat/memory-fabric-vma",
    [string]$Tag = "m8-memory-fabric-vma-ok-2026-08-17"
)

$ErrorActionPreference = "Stop"

Write-Host "=== Bouchaud OS - migration checkpoint ===" -ForegroundColor Cyan

$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"

if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    throw "git introuvable dans le PATH."
}
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "cargo introuvable dans le PATH."
}
if (-not (Test-Path ".git")) {
    throw "Lancer ce script depuis la racine du depot bouchaud-os."
}

Write-Host "`n=== Verification Rust ===" -ForegroundColor Cyan
cargo check
if ($LASTEXITCODE -ne 0) {
    throw "cargo check a echoue. Aucun commit/push."
}

Write-Host "`n=== Creation du checkpoint ===" -ForegroundColor Cyan
$current = (git branch --show-current).Trim()
if ($LASTEXITCODE -ne 0) { throw "Impossible de lire la branche courante." }

if ($current -ne $Branch) {
    $localExists = git branch --list $Branch
    if ([string]::IsNullOrWhiteSpace(($localExists | Out-String))) {
        git switch -c $Branch
    } else {
        throw "La branche locale '$Branch' existe deja. Bascule dessus manuellement ou change -Branch."
    }
}

$paths = @(
    ".github/workflows/ladybird-native-browser.yml",
    ".github/workflows/system-health.yml",
    "run.ps1",

    "docs/architecture/MEMORY_FABRIC.md",
    "docs/architecture/SYSTEM_HEALTH.md",
    "docs/architecture/VMA_ENGINE.md",

    "src/arch/x86_64/idt.rs",

    "src/drivers/block.rs",
    "src/drivers/disk.rs",
    "src/drivers/mod.rs",

    "src/fs/backing.rs",
    "src/fs/mod.rs",
    "src/fs/ramfs.rs",
    "src/fs/tar.rs",

    "src/kernel/abi/file.rs",
    "src/kernel/abi/mem.rs",
    "src/kernel/abi/mod.rs",
    "src/kernel/abi/proc.rs",
    "src/kernel/elf.rs",
    "src/kernel/exec.rs",
    "src/kernel/mod.rs",
    "src/kernel/partage.rs",
    "src/kernel/task.rs",
    "src/kernel/vma.rs",

    "src/shell/commands.rs",

    "tools/health/autorun.bsh",
    "tools/health/fixture_server.py",
    "tools/health/manifest.json",
    "tools/health/verify_health.py"
)

Write-Host "`n=== Stage des fichiers du checkpoint ===" -ForegroundColor Cyan
foreach ($path in $paths) {
    if (Test-Path $path) {
        git add -- $path
        Write-Host "stage: $path"
    } else {
        Write-Host "absent (ignore): $path" -ForegroundColor DarkYellow
    }
}

Write-Host "`n=== Fichiers volontairement exclus ===" -ForegroundColor Cyan
Write-Host "native-browser-m8/"
Write-Host "scenario-m8/"
Write-Host "ladybird-browser.img"
Write-Host "target/"
Write-Host "payload/"
Write-Host "apply-*.py"
Write-Host "README-VMA-V3.txt / README.txt"
Write-Host "make-ladybird-m8-image.py"
Write-Host "m8-old-build.log"

Write-Host "`n=== Diff stage ===" -ForegroundColor Cyan
git diff --cached --stat
git diff --cached --check
if ($LASTEXITCODE -ne 0) {
    throw "git diff --cached --check a echoue. Aucun commit."
}

$staged = git diff --cached --name-only
if ([string]::IsNullOrWhiteSpace(($staged | Out-String))) {
    throw "Aucun fichier stage. Rien a migrer."
}

Write-Host "`n=== Commit ===" -ForegroundColor Cyan
git commit -m "kernel: add memory fabric VMA engine and system health"
if ($LASTEXITCODE -ne 0) {
    throw "Le commit a echoue."
}

Write-Host "`n=== Tag checkpoint M8 ===" -ForegroundColor Cyan
$existingTag = git tag --list $Tag
if ([string]::IsNullOrWhiteSpace(($existingTag | Out-String))) {
    git tag -a $Tag -m "M8 Ladybird OK with Memory Fabric and VMA engine"
}

Write-Host "`n=== Push GitHub ===" -ForegroundColor Cyan
git push -u origin $Branch
if ($LASTEXITCODE -ne 0) {
    throw "Le push de la branche a echoue."
}
git push origin $Tag
if ($LASTEXITCODE -ne 0) {
    throw "Le push du tag a echoue."
}

Write-Host "`n=== CHECKPOINT MIGRE ===" -ForegroundColor Green
Write-Host "Branche : $Branch"
Write-Host "Tag     : $Tag"
Write-Host ""
Write-Host "Sur l'autre PC :"
Write-Host "  git clone https://github.com/bcharthur/bouchaud-os.git"
Write-Host "  cd bouchaud-os"
Write-Host "  git switch $Branch"
Write-Host "  .\run.ps1 -Ladybird"
