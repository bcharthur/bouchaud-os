# Bouchaud OS V0.13.2 — Bureau plein ecran + lanceur Chromium

## Ce qui est ajoute

- `run.ps1 -Fullscreen` pour lancer QEMU en plein ecran Windows.
- `run-fullscreen.ps1` comme raccourci.
- Bouton `Chromium` dans la barre du bureau graphique.
- Fichier applicatif `/apps/chromium.exe` cree au boot dans le RAMFS.
- Commande shell `chromium` qui explique l'etat du lanceur et les briques manquantes.

## Important : resolution reelle vs plein ecran

Le bureau Bouchaud OS utilise encore le mode VGA 13h interne : `320x200x256`.
Le patch agrandit la fenetre QEMU en plein ecran Windows, mais ne transforme pas encore le noyau en framebuffer haute resolution.

Pour une vraie resolution type Windows, par exemple 1280x720 ou 1920x1080, il faudra migrer vers un framebuffer lineaire via une chaine de boot plus moderne, probablement bootloader `0.11`/UEFI framebuffer ou equivalent. C'est une refonte plus risquee que ce patch.

## Important : Chromium.exe

Un executable Windows Chromium ne peut pas encore etre lance nativement par Bouchaud OS.
Il faudrait au minimum :

- un modele de processus utilisateur ;
- un loader executable PE/Win32 ou un port natif ELF/Bouchaud ;
- un sous-systeme graphique fenetre/surface ;
- un driver reseau e1000/virtio-net ;
- TCP/IP, DNS, HTTP/TLS ;
- beaucoup de memoire dynamique et de syscalls.

Le patch prepare donc un lanceur Chromium visible dans le bureau et un fichier `/apps/chromium.exe`, sans pretendre que Chromium est executable aujourd'hui.

## Installation

Dezipper a la racine du projet :

```powershell
cd C:\Users\Arthur\RustroverProjects\bouchaud-os
Expand-Archive -Force .\bouchaud-os-v0.13.2-desktop-fullscreen-chromium-launcher.zip .
```

Build :

```powershell
cargo +nightly clean
cargo +nightly bootimage
```

Run normal :

```powershell
.\run.ps1
```

Run plein ecran :

```powershell
.\run-fullscreen.ps1
# ou
.\run.ps1 -Fullscreen
```

## Tests

Dans Bouchaud OS :

```text
ls /apps
cat /apps/chromium.exe
chromium
desktop
```

Dans le bureau :

- clique `Chromium` ;
- lis l'etat du lanceur ;
- appuie sur Entree ou Echap pour revenir au bureau ;
- clique `Terminal` pour tester le shell GUI ;
- clique `Quitter` pour revenir au shell texte.

## Suite conseillee

1. V0.14 : vrai framebuffer haute resolution.
2. V0.15 : PCI/e1000 reseau.
3. V0.16 : mini navigateur HTTP natif Bouchaud OS.
4. Plus tard : port d'un moteur navigateur reel ou compatibilite executable.
