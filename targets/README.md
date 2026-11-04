# Cibles de compilation

- `x86_64-bouchaud_os.json` : cible bare-metal par défaut (image BIOS `bootimage`, utilisée par la plupart des bancs).
- `x86_64-bouchaud_os_uefi.json` : même noyau pour le démarrage UEFI et le chargeur A/B (`--no-default-features --features uefi-boot,…`).

Les deux sont `-mmx,-sse,+soft-float` : le noyau n'utilise pas la FPU, qui appartient à la tâche interrompue.

Une architecture CPU et une plateforme sont deux dimensions différentes. Un portage AArch64 (QEMU `virt`, Raspberry Pi) est une direction, **non commencée** ; il partirait de `aarch64-unknown-none-softfloat`.
