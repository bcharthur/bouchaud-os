# AArch64 et Raspberry Pi — frontière de portage

Le premier jalon doit être **QEMU `virt` AArch64**, pas une carte physique. Il
fournit DTB, PSCI, GICv2/v3, timer générique ARM, PL011 et virtio sans imposer au
premier boot les particularités du firmware Raspberry.

## Frontières minimales

Le code générique doit consommer cinq services, sans reproduire chaque registre
matériel sous forme de trait : horloge monotone, attente/notification CPU,
contrôleur d'interruptions, espace d'adressage et découverte de plateforme. Les
implémentations x86 restent TSC/LAPIC/IOAPIC/CR3/ACPI; AArch64 utilisera
CNTVCT_EL0, GIC, TTBR, PSCI et DTB. Le framebuffer, les surfaces GUI, le scene
graph futur et son protocole IPC restent indépendants de l'architecture.

## Ordre proposé

1. cible Rust AArch64 bare-metal et UART PL011 sur QEMU `virt`;
2. DTB, allocateur de frames et tables de traduction ARMv8;
3. timer générique, GIC et exceptions;
4. SMP via PSCI `CPU_ON`;
5. virtio-blk puis virtio-net;
6. simple framebuffer et pile GUI indépendante de l'architecture;
7. seulement ensuite une carte Raspberry.

## Pi 4 contre Pi 5

Le **Pi 4** est la première cible physique recommandée : Cortex-A72 ARMv8,
firmware et documentation communautaire mûrs, PL011, GIC-400 et chemin
framebuffer connu. Le Pi 5 est plus rapide mais ajoute RP1 et une topologie
PCIe/interruptions plus récente; il devient la seconde cible, une fois DTB,
GIC et PCIe abstraits sur QEMU/Pi 4. Le boot physique doit accepter DTB fourni
par le firmware plutôt que coder les adresses MMIO en dur.

Le port complet, les drivers SD/PCIe et le boot firmware ne font pas partie du
checkpoint SMP actuel; ce document fixe l'ordre qui évite de contaminer le
renderer ou le kernel générique avec des hypothèses x86.
