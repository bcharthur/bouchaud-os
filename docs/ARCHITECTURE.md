# Architecture de Bouchaud OS

Ce document decrit le decoupage en modules introduit en V0.6. L'objectif est de
garder un code lisible, isoler le materiel, et preparer l'evolution vers un vrai
noyau (interruptions, memoire, processus, reseau).

## Vue d'ensemble

```
src/
├── main.rs            Point d'entree (kernel_main), banniere, sequence de boot
├── macros.rs          Macros print! / println! (VGA)
├── arch/
│   ├── mod.rs         Selection d'architecture
│   └── x86_64/
│       ├── mod.rs     init() des briques bas niveau
│       ├── ports.rs   inb / outb (E/S sur ports)
│       ├── cpu.rs     CPUID, rdtsc, halt_loop, cpuinfo
│       ├── gdt.rs     STUB : Global Descriptor Table
│       ├── idt.rs     STUB : Interrupt Descriptor Table
│       └── interrupts.rs  STUB : etat des IRQ
├── drivers/
│   ├── mod.rs
│   ├── vga.rs         Sortie texte VGA 0xb8000 + couleurs + _print
│   ├── serial.rs      UART 16550 COM1 + serial_print! / serial_println!
│   └── keyboard.rs    Clavier PS/2 polling, mapping AZERTY-FR, read_line
├── fs/
│   ├── mod.rs
│   └── ramfs.rs       FS en memoire a inodes fixes (Node, FileSystem)
├── kernel/
│   ├── mod.rs
│   ├── dmesg.rs       Journal noyau (tampon circulaire) + mirroring serie
│   ├── timer.rs       Ticks + mesure TSC
│   └── panic.rs       Panic handler (#[panic_handler])
├── users/
│   └── mod.rs         User, Session (root / arthur / guest)
├── shell/
│   ├── mod.rs         Boucle shell, parsing, dispatcher
│   └── commands.rs    Implementation de chaque commande
└── net/
    └── mod.rs         ROADMAP reseau OSI + placeholders
```

## Principes de conception

- **`no_std`, pas d'`alloc`** : aucune allocation dynamique tant qu'un heap
  allocator propre n'existe pas (cible V0.8). Tout repose sur des tableaux et
  structures de taille fixe (`static`).
- **Isolation du materiel** : tout ce qui touche au CPU ou aux ports d'E/S vit
  dans `arch/x86_64`. Le reste du noyau l'utilise via des fonctions stables.
- **Etat global controle** : les ressources uniques (VGA, serie, dmesg, session,
  RAMFS) sont des `static mut` encapsules derriere des fonctions d'acces
  (`vga::set_color`, `dmesg::log`, `ramfs::fs()`, `users::session()`...).
- **Sorties** : les macros `print!`/`println!` ecrivent sur l'ecran VGA ; les
  evenements noyau passent par `dmesg::log`, qui copie aussi sur la serie COM1.

## Sequence de boot (`kernel_main`)

1. `drivers::serial::init()` puis `drivers::vga::clear()` — sorties pretes.
2. `kernel::timer::init()` (capture du TSC) et `kernel::dmesg::init()`.
3. `arch::x86_64::init()` — stubs GDT / IDT / interruptions (journalises).
4. Montage du RAMFS, session par defaut (`arthur`), logs des sous-systemes.
5. Banniere d'accueil.
6. `shell::run()` — boucle interactive infinie.

## Points d'extension prevus

| Sous-systeme | Fichier cible           | Etat V0.6 | Prochaine etape |
|--------------|-------------------------|-----------|-----------------|
| GDT          | `arch/x86_64/gdt.rs`    | stub      | charger GDT + TSS |
| IDT          | `arch/x86_64/idt.rs`    | stub      | handlers d'exceptions |
| Interruptions| `arch/x86_64/interrupts.rs` | stub  | PIC 8259, IRQ timer/clavier |
| Timer        | `kernel/timer.rs`       | TSC only  | tick sur IRQ0 (PIT) |
| Memoire      | (a creer) `kernel/mem`  | absent    | frames + paging + heap |
| PCI          | (a creer) `drivers/pci` | absent    | scan du bus |
| Reseau       | `net/mod.rs`            | roadmap   | driver + Ethernet + IPv4 |
| Disque       | (a creer) `drivers/blk` | roadmap   | virtio-blk + BFS |
| Securite     | `users/mod.rs`          | sessions  | syscalls + split user/kernel |
