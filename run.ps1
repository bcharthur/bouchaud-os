cargo +nightly bootimage

& "C:\Program Files\qemu\qemu-system-x86_64.exe" `
  -drive format=raw,file=target\x86_64-bouchaud_os\debug\bootimage-bouchaud-os.bin
