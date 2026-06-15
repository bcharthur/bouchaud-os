# Roadmap Bouchaud OS

## Version 0.1 - Boot

Objectif : prouver que le noyau démarre.

- Boot x86_64 via bootloader expérimental
- Rust `no_std`
- `panic = abort`
- Affichage VGA texte
- Boucle CPU `hlt`

## Version 0.2 - Debug sérieux

- Driver UART 16550
- Logs kernel `kprintln!`
- Sortie série QEMU
- Panic handler détaillé

## Version 0.3 - Base CPU

- GDT
- IDT
- Gestion exceptions CPU
- Breakpoint exception
- Double fault handler

## Version 0.4 - Mémoire

- Lecture memory map du bootloader
- Allocateur de frames physiques
- Pagination x86_64
- Heap allocator

## Version 0.5 - Multitâche expérimental

- Tâches kernel
- Scheduler round-robin
- Timer interrupt
- Context switch

## Version 0.6 - Userland minimal

- Syscalls
- Processus userland
- Shell simple
- Isolation basique

## Version 1.0 - Noyau éducatif propre

- Boot reproductible
- Documentation complète
- Tests unitaires lorsque possible
- CI locale ou auto-hébergée
- Modèle de menace initial
