$ErrorActionPreference = "Stop"

$path = "src\arch\x86_64\smp.rs"
if (-not (Test-Path $path)) {
    throw "Fichier introuvable: $path. Lance ce script depuis la racine de bouchaud-os."
}

$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$content = [System.IO.File]::ReadAllText((Resolve-Path $path), $utf8NoBom)

$activation = 'let _bootstrap_guard = SmpBootstrapGuard::enter();'

if ($content.Contains($activation)) {
    Write-Host "[skip] Le guard SMP est deja ACTIVE dans init_probe()." -ForegroundColor Yellow
    exit 0
}

$needle = @'
    dmesg::log("SMP4_STAGE bootstrap-identity-ok");

    unsafe {
'@

$replacement = @'
    dmesg::log("SMP4_STAGE bootstrap-identity-ok");

    // BOUCHAUD_SMP_BOOTSTRAP_GUARD_ACTIVATION_V2
    // IMPORTANT: declarer le type de guard ne suffit pas. Il doit etre
    // instancie ici pour maintenir IRQ0 / reschedule hors de la fenetre
    // LAPIC + INIT/SIPI jusqu'a la fin de init_probe().
    let _bootstrap_guard = SmpBootstrapGuard::enter();

    unsafe {
'@

if (-not $content.Contains($needle)) {
    throw "Contexte attendu introuvable. Le fichier a peut-etre evolue; aucune modification n'a ete faite."
}

$backupDir = ".patch-backup\smp-guard-activation-" + (Get-Date -Format "yyyyMMdd-HHmmss")
New-Item -ItemType Directory -Force -Path $backupDir | Out-Null
Copy-Item $path (Join-Path $backupDir "smp.rs") -Force

$content = $content.Replace($needle, $replacement)
[System.IO.File]::WriteAllText((Resolve-Path $path), $content, $utf8NoBom)

# Verification structurelle : l'activation doit etre dans init_probe.
$after = [System.IO.File]::ReadAllText((Resolve-Path $path), $utf8NoBom)
if (-not $after.Contains($activation)) {
    throw "Echec verification: activation absente apres ecriture."
}

Write-Host "[ok] SmpBootstrapGuard::enter() active dans init_probe()." -ForegroundColor Green
Write-Host "[ok] Backup: $backupDir" -ForegroundColor Green
Write-Host ""
Write-Host "Verification:"
Write-Host '  Select-String -Path .\src\arch\x86_64\smp.rs -Pattern "SmpBootstrapGuard::enter|SMP_BOOT_GUARD_ENTER" -Context 2,2'
Write-Host '  cargo clean'
Write-Host '  cargo bootimage'
