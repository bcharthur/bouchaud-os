# Bouchaud OS V0.5 — System foundation CLI

Patch prêt à glisser-déposer dans la racine du dépôt `bouchaud-os`.

## Objectif

Cette version garde le shell Unix-like, le RAMFS et le clavier AZERTY-FR, puis ajoute les premières briques d'un OS CLI plus complet :

- sessions utilisateur simples : `root`, `arthur`, `guest` ;
- commandes `whoami`, `id`, `users`, `login`, `logout`, `su` ;
- découverte système : `sysinfo`, `cpuinfo`, `meminfo`, `devices`, `dmesg`, `uptime` ;
- permissions simples sur fichiers : `chmod`, `ls -l`, `stat` ;
- placeholders réseau : `ifconfig`, `ip`, `route`, `arp`, `ping`, `dhcp`, `dns`, `wget`, `curl` ;
- documentation intégrée de la roadmap OSI.

## Installation

Dézipper à la racine du dépôt en remplaçant les fichiers existants :

```powershell
cd C:\Users\Arthur\RustroverProjects\bouchaud-os
cargo +nightly clean
cargo +nightly bootimage
.\run.ps1
```

## Tests rapides dans QEMU

```text
help
uname
sysinfo
cpuinfo
meminfo
devices
dmesg
whoami
id
users
login arthur
whoami
pwd
ls -l /
stat /readme.txt
chmod 600 /readme.txt
ls -l /
mkdir /home/arthur/projets
cd /home/arthur/projets
touch test.txt
write test.txt bonjour depuis bouchaud os souverain francais
cat test.txt
tree /
ping 1.1.1.1
```

## Commit conseillé

```powershell
git status
git add src/main.rs run.ps1 docs/README_V0.5.md
git commit -m "Add system foundation commands and session model"
git push
```

## Suite logique

- V0.6 : découpage en modules Rust propres (`arch`, `drivers`, `fs`, `shell`, `users`, `kernel`).
- V0.7 : GDT/IDT, exceptions CPU, timer, interruptions clavier.
- V0.8 : heap allocator + mémoire.
- V0.9 : scan PCI.
- V1.0 : début réseau avec driver e1000 ou virtio-net.


## V0.5.1

Correction de compatibilite Rust nightly : macros `print!` et `println!` exposees avant utilisation et sans point-virgule en position expression.


## V0.5.2 - correction clavier

- Correction de Backspace dans le writer VGA : le caractere ASCII 0x08 est maintenant interprete comme un retour arriere au lieu d'etre affiche comme `?`.
- Support de la touche Suppr/Delete PS/2 etendue `E0 53`, mappee temporairement comme Backspace tant que le shell n'a pas encore de curseur horizontal.
