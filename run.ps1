param(
  [switch]$Fullscreen
)

cargo +nightly bootimage

# -serial stdio redirige la sortie serie COM1 du noyau vers ce terminal.
# -Fullscreen agrandit QEMU en plein ecran Windows.
# Note: le bureau reste rendu en VGA 13h interne pour l'instant, puis QEMU le scale.
$qemuArgs = @(
  "-drive", "format=raw,file=target\x86_64-bouchaud_os\debug\bootimage-bouchaud-os.bin",
  "-serial", "stdio"
)

if ($Fullscreen) {
  $qemuArgs += "-full-screen"
}

& "C:\Program Files\qemu\qemu-system-x86_64.exe" @qemuArgs
