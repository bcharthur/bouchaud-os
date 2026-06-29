# Regenere l'image bootable AVEC verification (contrairement a run.ps1 qui lance
# QEMU meme si bootimage echoue) puis lance QEMU sur l'image fraiche.
Set-Location $PSScriptRoot
& "$PSScriptRoot\tools\update-nautile.ps1" -RepoRoot $PSScriptRoot
if ($LASTEXITCODE -ne 0) {
    Write-Host "mise a jour Nautile echouee - bootimage non lance." -ForegroundColor Red
    Read-Host "Entree pour fermer"
    exit $LASTEXITCODE
}

Write-Host "=== cargo +nightly bootimage ===" -ForegroundColor Cyan
cargo +nightly bootimage 2>&1 | Tee-Object -FilePath boot.log
$code = $LASTEXITCODE
Write-Host "=== bootimage EXIT = $code ===" -ForegroundColor Yellow
if ($code -eq 0) {
    Write-Host "Lancement de QEMU..." -ForegroundColor Green
    & "C:\Program Files\qemu\qemu-system-x86_64.exe" `
        -drive "format=raw,file=target\x86_64-bouchaud_os\debug\bootimage-bouchaud-os.bin" `
        -m 2048 `
        -serial stdio `
        -netdev "user,id=net0" `
        -device "e1000,netdev=net0"
} else {
    Write-Host "bootimage a echoue - QEMU non lance." -ForegroundColor Red
    Read-Host "Entree pour fermer"
}
