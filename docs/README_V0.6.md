# Bouchaud OS V0.6 — Kernel foundation

V0.6 transforme le gros `src/main.rs` en une **architecture modulaire** et pose
les fondations d'un vrai noyau Unix-like : logs noyau, sortie serie de debug,
stubs propres GDT/IDT/interruptions, base de temps, et de nouvelles commandes
systeme. Le shell, le clavier AZERTY-FR, le RAMFS et toutes les commandes
existantes restent fonctionnels.

> Objectif du projet : un OS souverain francais experimental, from scratch, en
> Rust `no_std`, pedagogique et extensible.

---

## 1. Ce qui est reellement implemente en V0.6

- **Refactor modulaire** : le code est decoupe en modules clairs (voir
  `docs/ARCHITECTURE.md`). Plus de `main.rs` geant.
- **Sortie serie COM1** (`drivers/serial.rs`) : UART 16550 initialise au boot,
  macros `serial_print!` / `serial_println!`. QEMU est lance avec `-serial stdio`.
- **dmesg reel** (`kernel/dmesg.rs`) : tampon circulaire fixe (64 entrees).
  `dmesg::log("...")` ecrit a la fois dans le tampon et sur COM1. La commande
  `dmesg` affiche les vrais evenements enregistres depuis le boot.
- **Base de temps** (`kernel/timer.rs`) : compteur de cycles via le TSC
  (`rdtsc`) et compteur de ticks (fige a 0 tant que l'IRQ timer n'existe pas).
- **Nouvelles commandes** : `version`, `ticks`, `interrupts`, `serial-test`,
  `panic-test`, `roadmap` ; `uptime`, `sysinfo`, `devices`, `meminfo` enrichies.
- **Panic handler** dedie (`kernel/panic.rs`) : affichage VGA rouge + COM1.
- **Historique des commandes** (`shell/history.rs`) : commande `history`
  (et `history clear`). Chaque commande est aussi recopiee sur COM1 sous la
  forme `$ <commande>` : avec `-serial stdio`, le terminal QEMU contient un
  transcript complet de la session, facile a copier/coller et a partager.

## 2. Ce qui est seulement prepare (stubs / roadmap)

- **GDT / IDT / interruptions** (`arch/x86_64/{gdt,idt,interrupts}.rs`) : des
  stubs propres, appeles au boot, qui journalisent leur etat. Aucune table n'est
  encore chargee, les IRQ restent masquees (clavier en polling). Etat visible via
  `sysinfo` et `interrupts`.
- **Reseau** (`net/mod.rs`) : feuille de route OSI documentee. Les commandes
  `ifconfig`, `ip`, `route`, `arp`, `ping`, `dhcp`, `dns`, `wget`, `curl`
  affichent la couche manquante. Aucun paquet n'est envoye.
- **Disque** : `mount`, `df`, `sync`, `mkfs.bfs` annoncent la roadmap BFS
  (Bouchaud File System) persistant. Le RAMFS reste le seul FS actif.
- **Heap allocator / pagination / processus / syscalls** : non implementes,
  documentes dans la roadmap.

---

## 3. Build

```powershell
cd C:\Users\Arthur\RustroverProjects\bouchaud-os
cargo +nightly clean
cargo +nightly bootimage
.\run.ps1
```

`run.ps1` lance QEMU avec `-serial stdio` : les logs noyau et `serial-test`
apparaissent directement dans le terminal PowerShell.

Image generee : `target\x86_64-bouchaud_os\debug\bootimage-bouchaud-os.bin`.

---

## 4. Commandes a tester dans QEMU

```text
help
version
uname
sysinfo
cpuinfo
meminfo
devices
dmesg
history
uptime
ticks
interrupts
serial-test          (regarde la fenetre/terminal serie de QEMU)
roadmap

whoami
id
users
login root
whoami
su

pwd
ls -l /
tree /
mkdir /home/arthur/projets
cd /home/arthur/projets
touch test.txt
write test.txt bonjour depuis bouchaud os souverain francais
cat test.txt
stat test.txt
chmod 600 test.txt
ls -l

ping 1.1.1.1         (placeholder reseau)
mount                (placeholder disque)

panic-test           (root uniquement : declenche une panique volontaire)
```

Au boot, l'ecran affiche :

```text
Bouchaud OS
Version: 0.6.0 - kernel foundation
Clavier: AZERTY-FR
Shell: Unix-like CLI
FS: RAMFS
Serial: COM1 debug enabled
Objectif: OS souverain francais experimental

arthur@bouchaud-os:/$
```

Et sur la sortie serie COM1 (QEMU `-serial stdio`) :

```text
[dmesg] tampon circulaire initialise
[kernel] kernel: boot Bouchaud OS V0.6 - kernel foundation
[kernel] vga: text mode initialise
[kernel] serial: COM1 initialise (debug QEMU)
[kernel] gdt: stub initialise (GDT du bootloader conservee)
[kernel] idt: stub initialise (aucun handler enregistre)
[kernel] interrupts: stub, IRQ masquees, clavier en polling PS/2
[kernel] keyboard: PS/2 polling AZERTY-FR actif
[kernel] ramfs: monte sur /
[kernel] session: utilisateur par defaut arthur
[kernel] net: pile reseau non activee
[kernel] disk: pilote disque non active
[kernel] shell: initialise
```

---

## 5. Roadmap vers V0.7 / V0.8 / V1.0

- **V0.7** : GDT/IDT reelles, exceptions CPU (breakpoint, double fault), PIC 8259,
  IRQ timer (PIT) et clavier en interruption (`sti`).
- **V0.8** : lecture de la memory map, allocateur de frames, pagination, heap
  allocator — passage progressif a `alloc`.
- **V0.9** : scan du bus PCI, enumeration des devices.
- **V1.0** : pile reseau (driver e1000/virtio-net -> Ethernet -> ARP -> IPv4 ->
  ICMP/UDP/TCP) puis disque persistant BFS, processus, syscalls et securite.

---

## 6. Commit conseille

```powershell
git add src/ docs/README_V0.6.md docs/ARCHITECTURE.md docs/ROADMAP.md run.ps1
git commit -m "V0.6: refactor modulaire, serie COM1, dmesg reel, stubs GDT/IDT, timer"
git push -u origin claude/great-wright-jvdnh5
```
