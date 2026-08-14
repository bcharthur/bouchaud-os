param(
  [switch]$Fullscreen,
  # Sortie audio de l'hote. "dsound" sur Windows, "sdl" ou "pa" sur Linux ;
  # "none" pour une machine muette. Sans carte AC97 branchee sur un vrai
  # backend, l'OS ne trouve aucun peripherique et toute video est silencieuse -
  # ce qui se prend facilement pour un defaut du pilote.
  [string]$Audio = "dsound",
  # Acceleration materielle. Sans elle, QEMU emule chaque instruction et le
  # decodage H.264 ne suit pas le temps reel : l'image saccade, et c'est le
  # processeur emule qui est en cause, pas le decodeur.
  [string]$Accel = "auto",
  # Met a jour la branche courante avant de construire. Voir
  # tools/etat-source.ps1 : ce n'est pas le defaut, et cela ne bloque jamais le
  # demarrage.
  [switch]$Sync,
  # Demarre le noyau seul, sans chercher a recuperer le userland.
  [switch]$NoUserlandDownload,
  # Retelecharge le userland meme si celui qui est la est valide.
  [switch]$RefreshUserland,
  # Accepte le userland d'un ancetre du commit courant, faute d'exact. Voir
  # tools/userland.ps1 : le decalage est alors annonce, jamais silencieux.
  [switch]$AllowOlderUserland
)

& "$PSScriptRoot\tools\etat-source.ps1" -RepoRoot $PSScriptRoot -Sync:$Sync

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
#
# Elle est recuperee ici si elle manque - l'integration continue en publie une
# par commit de `main`, et la construire soi-meme demande une heure et une
# chaine Linux. Voir tools/userland.ps1 : jamais un userland d'un autre commit
# sans le dire, jamais une image dont l'empreinte ne correspond pas.
& "$PSScriptRoot\tools\userland.ps1" -RepoRoot $PSScriptRoot `
    -NoDownload:$NoUserlandDownload -Refresh:$RefreshUserland `
    -AllowOlder:$AllowOlderUserland

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

# Carte son AC'97 : c'est le seul modele que le pilote de l'OS connait
# (src/drivers/ac97.rs). Elle doit etre reliee a un backend, comme la carte
# reseau a son netdev.
if ($Audio -eq "none") {
  $qemuArgs += @("-audiodev", "none,id=snd0", "-device", "AC97,audiodev=snd0")
} else {
  $qemuArgs += @("-audiodev", "$Audio,id=snd0", "-device", "AC97,audiodev=snd0")
}

# Acceleration : WHPX sous Windows (activer "Plateforme d'hyperviseur Windows"
# dans les fonctionnalites facultatives), KVM sous Linux. "auto" laisse QEMU
# choisir et retomber sur l'emulation pure si rien n'est disponible.
if ($Accel -ne "none") {
  if ($Accel -eq "auto") {
    $qemuArgs += @("-accel", "whpx,kernel-irqchip=off", "-accel", "tcg")
  } else {
    $qemuArgs += @("-accel", $Accel)
  }
}

if ($Fullscreen) {
  $qemuArgs += "-full-screen"
}

& "C:\Program Files\qemu\qemu-system-x86_64.exe" @qemuArgs
