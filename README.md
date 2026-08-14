# Bouchaud OS

Un système d'exploitation x86_64 écrit **from scratch**, et un navigateur Web
écrit from scratch qui tourne dessus.

Le noyau est du Rust `no_std` : il démarre, gère la mémoire virtuelle, ordonnance
des processus et des fils, expose une ABI compatible Linux, et fait tourner du
code en ring 3. Au-dessus, un userland Qt, un interpréteur Python, un moteur
JavaScript QuickJS, et un moteur Web complet — DOM, cascade CSS, mise en page,
JavaScript, Workers, IndexedDB, WebSocket — dont la page vit dans un **processus
séparé** du processus qui tient la fenêtre.

```text
┌─────────────────────────────────────────────────────────────────┐
│  Processus navigateur (ring 3)          Processus renderer      │
│  ┌───────────────────────────┐          ┌────────────────────┐  │
│  │ chrome Qt                 │  canal   │ HTML / CSS / DOM   │  │
│  │ historique                │◄────────►│ mise en page       │  │
│  │ politique (nav, CORS)     │ contrôle │ JavaScript QuickJS │  │
│  │ réseau, témoins, stockage │          │ Workers            │  │
│  └───────────┬───────────────┘  surface └─────────┬──────────┘  │
│              │                  partagée          │             │
│              └──────────── memfd / MAP_SHARED ────┘             │
├─────────────────────────────────────────────────────────────────┤
│  ABI compatible Linux : syscall/sysret, mmap, futex, clone,     │
│  socket, epoll, signaux, fichiers                               │
├─────────────────────────────────────────────────────────────────┤
│  Noyau Rust no_std : processus, fils, ordonnanceur à classes,   │
│  mémoire virtuelle, VFS + persistance, pile TCP/IP + TLS,       │
│  framebuffer, PCI, ELF                                          │
├─────────────────────────────────────────────────────────────────┤
│  x86_64 : GDT + TSS, IDT, PIC/PIT, pagination, ring 0 / ring 3  │
└─────────────────────────────────────────────────────────────────┘
```

## Ce qui existe réellement

**Noyau** — `src/`

| Domaine | État |
|---|---|
| Boot, VGA, série, panic | en place |
| GDT + TSS, IDT, exceptions, IRQ (PIC 8259, PIT) | en place |
| Mémoire physique, pagination, `mmap`, tas noyau | en place |
| Ring 3, `syscall`/`sysret`, ABI compatible Linux | en place |
| Processus, fils, ordonnanceur à classes (Interactive / Normale) | en place |
| Signaux, descripteurs, `futex`, mémoire partagée | en place |
| VFS, persistance disque | en place |
| Pile réseau : Ethernet, IPv4, TCP, UDP, DNS, TLS | en place |
| Framebuffer, PCI, chargeur ELF | en place |
| GUI : gestionnaire de fenêtres, widgets, souris, polices | en place |

**Userland** — `tools/userland/`

Qt, Python, QuickJS et FFmpeg sont construits pour la cible par des scripts
(`build-qt.sh`, `build-python.sh`, `build-quickjs.sh`, `build-ffmpeg.sh`). Le
navigateur natif est un ELF unique qui embarque Python et Qt.

**Navigateur** — `tools/userland/navigateur/`

| Capacité | État |
|---|---|
| HTML, cascade CSS, sélecteurs, flex, grille, transformations, animations | en place |
| Mise en page, peinture, polices (WOFF2), images, `<canvas>` | en place |
| JavaScript (QuickJS) : DOM, événements, minuteries, promesses, modules | en place |
| Formulaires interactifs, édition de texte, foyer | en place |
| `fetch`, XHR, cache HTTP, témoins, `localStorage`, IndexedDB | en place |
| Origines, same-origin policy, CORS, `<iframe>` et contextes de navigation | en place |
| Web Workers réels, clonage structuré, `MessageChannel`, `postMessage` | en place |
| WebSocket et WSS | en place |
| Navigation SPA (`pushState`, `popstate`) | en place |
| **Renderer dans un processus séparé, utilisé par défaut** | en place |
| Isolation : crash, mémoire (`RLIMIT_AS`), descripteurs, priorité | en place |
| Ressources courtées par le navigateur (réseau, témoins, stockage) | en place |
| Rendu déporté vers un Chromium de l'hôte (`distant:`, F2) | en place |

Les capacités fonctionnelles sont suivies une par une dans
`tools/userland/navigateur/tests/jalons.json`, écrit par les épreuves et non à la
main.

## Ce qui n'existe pas

Pas de process-per-origin ni d'isolation par site : un navigateur, un renderer.
Pas de compositeur — le renderer rastérise toute sa surface à chaque trame. Pas
de ServiceWorker, WebGL, WebGPU, WebRTC, HTTP/3. Pas de runner WPT. Le renderer
ferme les descripteurs dont il hérite mais ne dispose d'aucun bac à sable du
noyau : `docs/RENDERER_PRIVILEGE_AUDIT.md` dit exactement ce qu'il peut encore
faire, capacité par capacité.

## Construire et lancer

Prérequis : Rust (la chaîne est épinglée dans `rust-toolchain.toml`, rustup
l'installe au premier `cargo`), QEMU, et `cargo install bootimage`.

```powershell
cargo bootimage
qemu-system-x86_64 -drive format=raw,file=target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin
```

`run.ps1`, `run-fullscreen.ps1` et `boot.ps1` enveloppent ces commandes ;
`check.ps1` enchaîne les vérifications. `run.ps1 -Sync` met à jour la branche
courante avant de construire — ce n'est pas le défaut, et cela n'empêche jamais
de démarrer : booter ne doit pas dépendre du réseau.

Le navigateur ne vit **pas** dans l'image bootable : il est dans le disque
userland, construit à part par `tools/userland/mkdisk.sh`. Sans ce second
disque, l'OS démarre mais sans Qt, sans Python et sans navigateur.

Le userland se construit à part :

```bash
cd tools/userland
./build-quickjs.sh     # moteur JavaScript
./build-python.sh      # interpréteur embarqué
./build-qt.sh          # Qt pour la cible
./build-navigateur.sh  # le navigateur, ELF unique
./mkdisk.sh            # image de disque
```

Ces scripts téléchargent leurs sources et reconstruisent tout : rien de ce
qu'ils produisent n'est versionné.

## Éprouver

Le moteur Web s'éprouve sans démarrer l'OS — il ne dépend que du module `bo` que
l'hôte Qt fournit d'habitude, et un bouchon le remplace :

```bash
cd tools/userland
./test-moteur.sh                                    # ~1600 vérifications
BO_SCRIPT=ordonnanceur-navigateur.py ./test-moteur.sh   # latence du chrome
```

Ce que cela ne prouve pas, c'est le rendu réel et le comportement sous
l'ordonnanceur de Bouchaud OS : pour cela il faut l'OS, donc QEMU.

Le navigateur peut être lancé sur l'ancien chemin en-processus pour trancher
« est-ce le moteur, ou la séparation ? » :

```
BO_BROWSER_INPROCESS=1 /bo-navigateur https://exemple.test/
```

## Documentation

`docs/` contient la documentation **courante**. Ce qui décrit un état révolu vit
dans `docs/history/` et n'est conservé que pour la trace : ces documents disent
ce qui était vrai à leur date, pas ce qui est vrai aujourd'hui.

| Document | Sujet |
|---|---|
| `docs/ARCHITECTURE.md` | découpage du noyau et du userland |
| `docs/BROWSER_ISOLATION.md` | le modèle multiprocessus du navigateur |
| `docs/BROWSER_RENDERER_PROTOCOL.md` | le protocole navigateur ↔ renderer |
| `docs/RENDERER_PRIVILEGE_AUDIT.md` | ce que le renderer peut encore faire |
| `docs/ROADMAP.md` | ce qui vient ensuite, et pourquoi |
| `docs/WEB_ENGINE_MODULES.md` | carte des modules du moteur Web |
