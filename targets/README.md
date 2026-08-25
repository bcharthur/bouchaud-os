# Cibles de compilation

- `x86_64-bouchaud_os.json` : cible bare-metal actuelle et validée.
- AArch64 commencera avec `aarch64-unknown-none-softfloat` tant qu'aucune
  configuration de linker spécifique à Bouchaud OS n'est nécessaire.

Une architecture CPU et une plateforme sont deux dimensions différentes :
QEMU-PC est aujourd'hui x86_64 ; QEMU `virt` et Raspberry Pi viseront AArch64.
