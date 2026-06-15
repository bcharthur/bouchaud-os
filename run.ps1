cargo +nightly bootimage

& "C:\Program Files\qemu\qemu-system-x86_64.exe" `
  -m 64M `
  -drive format=raw,file=target\x86_64-bouchaud_os\debug\bootimage-bouchaud-os.bin
