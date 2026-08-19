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
git clone https://github.com/bcharthur/bouchaud-os.git
cd bouchaud-os
.\run.ps1
```

C'est tout, et **sans WSL**. `run.ps1` construit le noyau, récupère le userland
du commit courant s'il manque, vérifie qu'il porte bien ce commit et la bonne
empreinte SHA-256, puis lance QEMU avec les deux disques.

Le navigateur ne vit **pas** dans l'image bootable : il est dans le disque
userland, un ELF unique embarquant Qt, CPython, QuickJS et FFmpeg. Le construire
demande une heure et une chaîne de compilation Linux ; l'intégration continue le
fait une fois par commit de `main` et le publie en release. Rien de Linux
n'intervient à l'exécution : ce sont des ELF statiques produits par compilation
croisée, comme un firmware.

### Naviguer

```powershell
.\run.ps1
```

Bouchaud OS démarre sur son bureau. **Double-clic sur l'icône « Navigateur »**
— ou menu Démarrer — et Ladybird s'ouvre dans une fenêtre du bureau. Fermer la
fenêtre rend la main au bureau ; double-cliquer à nouveau relance le navigateur.
Une seule instance à la fois : deux moteurs Web sur un cœur unique ne rendraient
service à personne.

Le navigateur ne s'ouvre **pas** tout seul au démarrage. Un système qui ouvre un
navigateur sans qu'on le lui demande est une démonstration, pas un système.

| Dans la fenêtre | Effet |
|---|---|
| clic sur la barre d'adresse | prend le foyer clavier |
| texte + Entrée | navigue (une entrée sans point part en recherche) |
| `<` `>` `@` | reculer, avancer, recharger |
| clic dans la page | suit les liens, remplit les formulaires |
| molette, flèches | défilement |
| Échap | rend le foyer à la page, ou arrête le chargement |

| Option de `run.ps1` | Effet |
|---|---|
| `-Legacy` | revient au userland historique (Qt + CPython + QuickJS) |
| `-RamMiB <n>` | mémoire de la machine (12288 par défaut, voir plus bas) |
| `-CpuCount <n>` | vCPU exposés — le noyau n'en ordonnance qu'un, voir plus bas |
| `-LadybirdUrl <url>` | page de départ |
| `-LadybirdSansChrome` | retire la barre d'outils, capture unique de M9 |
| `-Fullscreen` | QEMU en plein écran |
| `-LadybirdM8`, `-LadybirdM9Test` | régressions déterministes de la CI |
| `-Sync` | met à jour la branche courante avant de construire |
| `-NoUserlandDownload` | démarre le noyau seul, sans chercher de userland |
| `-RefreshUserland` | retélécharge même si l'image locale est valide |
| `-AllowOlderUserland` | accepte le userland d'un ancêtre, en annonçant l'écart |

**Mémoire.** 12288 Mio par défaut : c'est la plus grande valeur réellement
éprouvée — le noyau démarre, la sonde réseau passe, et le démarrage ne coûte que
trois secondes de plus qu'à 2048 Mio. 16384 est accepté mais n'a pas pu être
vérifié sur la machine de développement, qui n'a que 15 Gio.

**Processeurs.** Un seul, et ce n'est pas une timidité : le noyau ne lit ni ACPI
ni MADT, n'a pas de LAPIC, ne peut donc pas réveiller un processeur applicatif,
et route ses interruptions par le PIC 8259 qui ne parle qu'au BSP. Demander huit
vCPU en allumerait sept qui resteraient éteints — `run.ps1` prévient au lieu de
laisser croire à une accélération. Détail dans `docs/ladybird/M13_DNS.md`.

**Résolution de noms.** Elle marche depuis M13. Elle ne marchait pas avant, et
la raison n'était pas dans Ladybird : la couche UDP du noyau jetait les
datagrammes destinés à un autre socket que celui qu'elle regardait, et attendait
en brûlant un cœur. `tools/net/verifie-dns.sh` le vérifie en quelques minutes,
sans construire Ladybird.

**Jamais un userland d'un autre commit sans le dire.** Une image d'un autre jour
ne se signale pas : elle démarre, et elle se comporte comme le système d'alors.
La panne qui suit accuse le code source, qui n'y est pour rien.

Il n'y a **qu'un** lanceur. `run-fullscreen.ps1` n'était qu'un alias de
`.\run.ps1 -Fullscreen`, et `boot.ps1` n'existait que parce que `run.ps1`
lançait QEMU même quand `bootimage` échouait — ce qui n'est plus vrai depuis
qu'il s'arrête sur l'erreur. Les deux ont été supprimés. `check.ps1` reste : il
compile sans lancer QEMU, ce qui est un autre métier.

### Construire le userland soi-même

Inutile pour s'en servir — `run.ps1` le récupère. Nécessaire pour le modifier.
**Sous WSL ou Linux** : cette chaîne compile du C, du C++ et du Python pour une
cible Linux-ABI, rien n'en tourne sous Windows.

```bash
cd tools/userland
./build-tout.sh        # QuickJS, FFmpeg, CPython, Qt, navigateur, disque
```

Compter une heure environ la première fois sur quatre cœurs, Qt en représentant
les deux tiers. Chaque étape est sautée si son résultat est déjà là. Les étapes
restent lançables une par une, mais dans cet ordre et avec ces variables — Qt
est du C++, donc CPython doit être en glibc, et `build-navigateur.sh` lie
`libavcodec.a` :

```bash
./build-quickjs.sh                                   # moteur JavaScript
./build-ffmpeg.sh                                    # décodeurs audio/vidéo
LIBC=glibc OUT=out-python-embed ./build-python.sh    # interpréteur embarqué
./build-qt.sh                                        # Qt 5.15 statique + linuxfb
./build-navigateur.sh                                # le navigateur, ELF unique
./mkdisk.sh out-navigateur                           # -> userland.img
```

Ces scripts téléchargent leurs sources et reconstruisent tout : rien de ce
qu'ils produisent n'est versionné.

Sans `userland.img`, l'OS démarre normalement et le bureau annonce
`/bo-navigateur absent` : le noyau est complet, c'est le second disque qui
manque.

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
| `docs/ETAT_DES_LIEUX.md` | **ce qui est acquis, et la preuve pour chaque ligne** |
| `docs/VISION.md` | **où va le système : mémoire, processeur, graphisme, IA** |
| `docs/ladybird/MASTER_PLAN.md` | le portage Ladybird et son échelle de jalons |
| `docs/ladybird/M13_DNS.md` | **résolution de noms : la cause, la preuve, la mesure mémoire/CPU** |
| `docs/ladybird/M11_NAVIGATEUR.md` | **le chrome du navigateur natif : barre d'adresse, entrées, trames** |
| `docs/ladybird/M12_HTTPS.md` | HTTPS, autorités racine, résolution de noms |
| `docs/ARCHITECTURE.md` | découpage du noyau et du userland |
| `docs/BROWSER_ISOLATION.md` | le modèle multiprocessus du navigateur |
| `docs/BROWSER_RENDERER_PROTOCOL.md` | le protocole navigateur ↔ renderer |
| `docs/RENDERER_PRIVILEGE_AUDIT.md` | ce que le renderer peut encore faire |
| `docs/ROADMAP.md` | journal des versions passées (pas un plan : voir `VISION.md`) |
| `docs/WEB_ENGINE_MODULES.md` | carte des modules du moteur Web |
