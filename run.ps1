cargo +nightly bootimage

# -serial stdio redirige la sortie serie COM1 du noyau vers ce terminal.
# Les logs kernel (dmesg) et la commande serial-test y apparaissent.
& "C:\Program Files\qemu\qemu-system-x86_64.exe" `
  -drive format=raw,file=target\x86_64-bouchaud_os\debug\bootimage-bouchaud-os.bin `
  -serial stdio
