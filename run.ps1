param(
  [switch]$Fullscreen
)

cargo +nightly bootimage

# -serial stdio redirige la sortie serie COM1 du noyau vers ce terminal.
# -Fullscreen agrandit QEMU en plein ecran Windows.
# La carte e1000 est reliee au reseau utilisateur (SLIRP) : IMPORTANT, le
# "netdev=net0" sur le -device relie la carte a son backend (sinon "no peer").
$qemuArgs = @(
  "-drive", "format=raw,file=target\x86_64-bouchaud_os\debug\bootimage-bouchaud-os.bin",
  "-serial", "stdio",
  "-netdev", "user,id=net0",
  "-device", "e1000,netdev=net0"
)

if ($Fullscreen) {
  $qemuArgs += "-full-screen"
}

& "C:\Program Files\qemu\qemu-system-x86_64.exe" @qemuArgs
