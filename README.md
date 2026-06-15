# Bouchaud OS

Bouchaud OS est un noyau expérimental **from scratch** écrit en Rust `no_std`.

Objectif de la phase 0 : obtenir un premier kernel qui boote dans QEMU et affiche un message à l'écran via le buffer VGA texte.

## Objectif long terme

- Kernel maison en Rust
- Architecture microkernel à terme
- Services système isolés
- Chaîne de build reproductible
- Dépôt auto-hébergé
- Couche POSIX/Linux-like plus tard pour porter des applications libres

## Prérequis Windows

Installer Rust :

```powershell
winget install Rustlang.Rustup
```

Installer QEMU :

```powershell
winget install SoftwareQInc.QEMU
```

Redémarrer le terminal après installation.

## Initialisation

```powershell
git clone https://github.com/bcharthur/bouchaud-os.git
cd bouchaud-os
rustup toolchain install nightly
rustup component add rust-src llvm-tools-preview --toolchain nightly
cargo +nightly install bootimage
```

## Compiler l'image bootable

```powershell
cargo +nightly bootimage
```

## Lancer dans QEMU

```powershell
qemu-system-x86_64 -drive format=raw,file=target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin
```

Tu dois voir un écran noir avec :

```text
Bouchaud OS
Kernel experimental from scratch en Rust no_std
Version: 0.1.0

Etat: boot OK, VGA text OK, panic handler OK
Prochaine etape: UART, GDT/IDT, interruptions, memoire

bouchaud-os>
```

## Roadmap courte

- [x] Boot minimal
- [x] Affichage VGA texte
- [x] Panic handler
- [ ] UART série
- [ ] GDT
- [ ] IDT
- [ ] Interruptions CPU
- [ ] Timer PIT/APIC
- [ ] Gestion mémoire physique
- [ ] Pagination
- [ ] Allocateur heap
- [ ] Scheduler minimal
- [ ] Shell interactif
