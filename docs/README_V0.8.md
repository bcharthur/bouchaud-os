# Bouchaud OS V0.7 / V0.8 — Interruptions + pile reseau

Suite directe de V0.6. Deux grandes briques "vrai OS" sont ajoutees :

- **V0.7 : interruptions materielles reelles** (GDT/IDT, exceptions, PIC, timer,
  clavier en IRQ).
- **V0.8 : pile reseau OSI** (Ethernet/ARP/IPv4/ICMP) avec une interface
  **loopback fonctionnelle** : `ping 127.0.0.1` traverse reellement le code.

---

## V0.7 — Interruptions (ce qui est reel)

- **GDT + TSS** (`arch/x86_64/gdt.rs`) : on charge notre propre GDT et un TSS
  avec une pile dediee (IST) pour la double faute -> plus de triple faute/reboot
  si la pile noyau est corrompue.
- **IDT** (`arch/x86_64/idt.rs`) : handlers d'exceptions (breakpoint, double
  faute, page fault, GPF) + IRQ timer (IRQ0) et clavier (IRQ1).
- **PIC 8259** (`arch/x86_64/interrupts.rs`) : remappe sur les vecteurs 32..47,
  `sti` active.
- **Clavier en interruption** : fini le polling. L'IRQ1 empile les scancodes
  dans une file ; l'editeur de ligne consomme la file et met le CPU en veille
  (`hlt`) quand elle est vide (economie CPU).
- **Timer reel** : `uptime` et `ticks` avancent vraiment (~18.2 Hz, PIT par
  defaut). `uptime` affiche les secondes.
- **Commande `breakpoint`** : declenche une exception `int3` et reprend (preuve
  que l'IDT fonctionne).

> Dependances ajoutees : `x86_64` et `pic8259` (briques OSdev standard, `no_std`,
> sans `alloc`). Choix pragmatique pour fiabiliser un code non testable hors QEMU.

## V0.8 — Pile reseau (ce qui est reel)

- **Couches implementees** comme logique reelle, sans allocation :
  `net/ethernet.rs` (L2), `net/arp.rs`, `net/ipv4.rs` (L3 + checksum Internet),
  `net/icmp.rs`, `net/stack.rs` (moteur).
- **Interface loopback `lo` (127.0.0.1) active** : `ping 127.0.0.1` construit un
  vrai paquet IPv4/ICMP echo request, le fait traiter par le moteur de pile, et
  affiche les echo replies (4 paquets, 0% perte). La logique a ete validee bit a
  bit (checksums IPv4/ICMP corrects).
- **`ifconfig` / `ip` / `route` / `arp`** : affichent l'etat reel (lo UP, eth0
  DOWN car driver non charge, table de routage loopback).

## Ce qui reste pour l'Internet "externe"

`ping 127.0.0.1` marche (loopback). Pour **sortir vers Internet** il manque le
**driver de carte reseau** (e1000 ou virtio-net) : configuration via les BAR PCI,
anneaux de descripteurs RX/TX et DMA. C'est la prochaine etape. Le scan PCI
(`lspci`) detecte deja la carte ; lance QEMU avec une NIC pour la voir :

```powershell
& "C:\Program Files\qemu\qemu-system-x86_64.exe" `
  -drive format=raw,file=target\x86_64-bouchaud_os\debug\bootimage-bouchaud-os.bin `
  -serial stdio -device e1000 -netdev user,id=n0
```

---

## Commandes a tester

```text
uptime           secondes reelles, le compteur avance
ticks
interrupts       gdt/idt/interrupts = initialisees/enabled
breakpoint       exception int3 capturee puis reprise

ping 127.0.0.1   4 echo replies via la vraie pile ICMP
ifconfig         lo UP 127.0.0.1, etat eth0
ip
route
arp
lspci            carte reseau detectee (si -device e1000)
```

## Build

```powershell
cargo +nightly clean
cargo +nightly bootimage
.\run.ps1
```
