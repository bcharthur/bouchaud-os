param(
  [switch]$Fullscreen
)

& "$PSScriptRoot\tools\update-nautile.ps1" -RepoRoot $PSScriptRoot
if ($LASTEXITCODE -ne 0) {
  Write-Host "mise a jour Nautile echouee - bootimage non lance." -ForegroundColor Red
  exit $LASTEXITCODE
}

cargo bootimage
# Si la generation de l'image echoue (erreur de compilation, llvm-objcopy
# bloque, .bin verrouille par un QEMU encore ouvert...), on ARRETE ici au lieu
# de booter silencieusement une ancienne image obsolete.
if ($LASTEXITCODE -ne 0) {
  Write-Host "bootimage a echoue (code $LASTEXITCODE) - QEMU non lance." -ForegroundColor Red
  exit 1
}

# -serial stdio redirige la sortie serie COM1 du noyau vers ce terminal.
# -Fullscreen agrandit QEMU en plein ecran Windows.
# La carte e1000 est reliee au reseau utilisateur (SLIRP) : IMPORTANT, le
# "netdev=net0" sur le -device relie la carte a son backend (sinon "no peer").
# Le service de rendu (tools/render-proxy) tourne sur l'hote a 127.0.0.1:8080 et
# est joignable depuis l'OS invite via l'acces hote SLIRP a 10.0.2.2:8080.
# NB Windows : autoriser node.exe / le port 8080 dans le pare-feu (voir README).
$qemuArgs = @(
  "-drive", "format=raw,file=target\x86_64-bouchaud_os\debug\bootimage-bouchaud-os.bin"
)

# Second disque : l'archive userland. Le noyau la deplie dans le RAMFS au
# demarrage, ce qui permet d'installer un programme sans recompiler l'OS.
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

if ($Fullscreen) {
  $qemuArgs += "-full-screen"
}

& "C:\Program Files\qemu\qemu-system-x86_64.exe" @qemuArgs
