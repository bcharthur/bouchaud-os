# Regenere l'image bootable AVEC verification (contrairement a run.ps1 qui lance
# QEMU meme si bootimage echoue) puis lance QEMU sur l'image fraiche.
Set-Location $PSScriptRoot
& "$PSScriptRoot\tools\update-nautile.ps1" -RepoRoot $PSScriptRoot
if ($LASTEXITCODE -ne 0) {
    Write-Host "mise a jour Nautile echouee - bootimage non lance." -ForegroundColor Red
    Read-Host "Entree pour fermer"
    exit $LASTEXITCODE
}

Write-Host "=== cargo bootimage (toolchain epinglee par rust-toolchain.toml) ===" -ForegroundColor Cyan
cargo bootimage 2>&1 | Tee-Object -FilePath boot.log
$code = $LASTEXITCODE
Write-Host "=== bootimage EXIT = $code ===" -ForegroundColor Yellow
if ($code -eq 0) {
    Write-Host "Lancement de QEMU..." -ForegroundColor Green
    $qemuArgs = @(
        "-drive", "format=raw,file=target\x86_64-bouchaud_os\debug\bootimage-bouchaud-os.bin"
    )
    # Second disque : l'archive userland, depliee dans le RAMFS au demarrage.
    # C'est ce qui permet d'installer un programme sans recompiler le noyau.
    # Fabriquer l'image avec tools/userland/mkdisk.sh.
    $userland = "tools\userland\userland.img"
    if (Test-Path $userland) {
        Write-Host "disque userland : $userland" -ForegroundColor Cyan
        $qemuArgs += @("-drive", "format=raw,file=$userland")
    }
    $qemuArgs += @(
        "-m", "2048",
        "-serial", "stdio",
        "-netdev", "user,id=net0",
        "-device", "e1000,netdev=net0"
    )
    & "C:\Program Files\qemu\qemu-system-x86_64.exe" @qemuArgs
} else {
    Write-Host "bootimage a echoue - QEMU non lance." -ForegroundColor Red
    Read-Host "Entree pour fermer"
}
