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
- [x] Historique des commandes + transcript serie (`history`)
- [x] Permissions Unix reelles (rwx, uid/gid, traversee) : home prive par user
- [x] Login par mot de passe (login / su), repertoire d'accueil
- [x] Scan PCI reel via 0xCF8/0xCFC (`lspci`) + detection carte reseau

## V0.7 - CPU & interruptions (fait)
- [x] GDT maison + TSS (IST double faute)
- [x] IDT + handlers d'exceptions (breakpoint, double faute, page fault, GPF)
- [x] PIC 8259 remappe 32..47, activation `sti`
- [x] IRQ0 timer (PIT) -> ticks reels, uptime en secondes
- [x] Clavier en interruption IRQ1 (fin du polling)
- [ ] APIC, plus tard, en remplacement du PIC

## V0.8 - Pile reseau (logique + loopback)
- [x] Ethernet (L2) encode/decode
- [x] ARP encode/decode
- [x] IPv4 (L3) en-tete + checksum Internet
- [x] ICMP echo + interface loopback (ping 127.0.0.1 fonctionnel)
- [ ] Driver NIC e1000/virtio-net (BAR PCI, rings RX/TX, DMA) -> Internet
- [ ] UDP, DHCP, DNS, puis TCP, HTTP, TLS

## V0.8 - Memoire
- [ ] Lecture de la memory map du bootloader
- [ ] Allocateur de frames physiques
- [ ] Pagination x86_64
- [ ] Heap allocator -> passage progressif a `alloc`

## V0.9 - Bus & devices
- [x] Scan du bus PCI (fait en V0.6.1)
- [x] Enumeration et description des peripheriques (`lspci`)
- [ ] Acces aux BAR (Base Address Registers) pour piloter un device

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
