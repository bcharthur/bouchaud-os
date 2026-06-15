# Roadmap Bouchaud OS

OS souverain francais experimental, from scratch, en Rust `no_std`.
Etat des versions : `[x]` fait, `[~]` prepare/stub, `[ ]` planifie.

## V0.1 - Boot
- [x] Boot x86_64 via bootloader 0.9
- [x] Rust `no_std`, `panic = abort`
- [x] Affichage VGA texte
- [x] Boucle CPU `hlt`

## V0.5 - Fondations CLI
- [x] Shell interactif Unix-like
- [x] Clavier AZERTY-FR (polling PS/2), Backspace/Suppr
- [x] RAMFS (fichiers, dossiers, permissions simples)
- [x] Sessions root / arthur / guest
- [x] Commandes systeme de base (sysinfo, cpuinfo, devices, dmesg...)

## V0.6 - Kernel foundation (actuel)
- [x] Refactor modulaire (arch / drivers / fs / kernel / users / shell / net)
- [x] Sortie serie COM1 (UART 16550) + `serial_print!` / `serial_println!`
- [x] dmesg reel (tampon circulaire) avec mirroring serie
- [x] Base de temps TSC (`uptime`, `ticks`)
- [x] Panic handler dedie (VGA + serie)
- [x] Commandes : version, interrupts, serial-test, panic-test, roadmap
- [~] Stubs propres GDT / IDT / interruptions appeles au boot
- [~] Roadmap reseau OSI + placeholders detailles
- [~] Roadmap disque BFS (mount, df, sync, mkfs.bfs)

## V0.7 - CPU & interruptions
- [ ] GDT maison (segments noyau/utilisateur + TSS)
- [ ] IDT + handlers d'exceptions (breakpoint, double fault, page fault)
- [ ] PIC 8259 (ou APIC), activation `sti`
- [ ] IRQ0 timer (PIT) -> incrementation reelle des ticks
- [ ] Clavier en interruption (fin du polling)

## V0.8 - Memoire
- [ ] Lecture de la memory map du bootloader
- [ ] Allocateur de frames physiques
- [ ] Pagination x86_64
- [ ] Heap allocator -> passage progressif a `alloc`

## V0.9 - Bus & devices
- [ ] Scan du bus PCI
- [ ] Enumeration et description des peripheriques

## V1.0 - Reseau & disque
- [ ] Driver reseau (e1000 ou virtio-net)
- [ ] Ethernet -> ARP -> IPv4 -> ICMP/UDP -> DHCP/DNS -> TCP -> HTTP
- [ ] Block device (virtio-blk)
- [ ] BFS (Bouchaud File System) persistant : mount, df, sync, mkfs.bfs

## Au-dela
- [ ] Processus et ordonnanceur
- [ ] Syscalls + split user/kernel
- [ ] Permissions completes, audit log
- [ ] Signature du noyau, secure boot
- [ ] Interface graphique
