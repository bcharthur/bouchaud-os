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

| Option de `run.ps1` | Effet |
|---|---|
| `-NoUserlandDownload` | démarre le noyau seul, sans chercher le userland |
| `-RefreshUserland` | retélécharge même si l'image locale est valide |
| `-AllowOlderUserland` | accepte le userland d'un ancêtre, en annonçant l'écart |
| `-Sync` | met à jour la branche courante avant de construire |
| `-Fullscreen` | QEMU en plein écran |

### Naviguer avec le port Ladybird

Le port Ladybird a maintenant une barre d'adresse, un historique, des liens
cliquables et HTTPS. Il s'utilise :

```powershell
.\run.ps1 -Ladybird                                   # fixture locale, page reelle
.\run.ps1 -Ladybird -LadybirdUrl "https://10.0.2.2:18443/"
```

> **Un site par son nom ne se charge pas encore.** HTTPS est prouvé — chaîne
> validée, nom d'hôte vérifié — mais uniquement contre un hôte désigné par son
> **adresse**. Résoudre un nom fait boucler `RequestServer` : cinq minutes à
> 50 % de processeur sans réponse, là où la même pile charge une adresse IP en
> quinze secondes. C'est mesuré, pas supposé, et c'est le point qui sépare
> « HTTPS fonctionne » de « on peut naviguer sur le Web » :
> `docs/ladybird/M12_HTTPS.md`, section « Ce qui bloque encore ».

| Dans la fenêtre | Effet |
|---|---|
| clic sur la barre d'adresse | prend le foyer clavier |
| texte + Entrée | navigue (une entrée sans point part en recherche) |
| `<` `>` `@` | reculer, avancer, recharger |
| clic dans la page | suit les liens, remplit les formulaires |
| molette, flèches | défilement |
| Échap | rend le foyer à la page, ou arrête le chargement |

QEMU donne au guest un accès sortant par NAT, et le magasin d'autorités requis
par TLS est fabriqué au premier lancement à partir de celui de la machine hôte
(`tools/ladybird/certs/README.md`). La page de départ est la fixture locale,
`run.ps1` la démarre alors sur l'hôte — c'est le seul chemin réseau prouvé de
bout en bout, et une page qui s'affiche vaut mieux qu'une page qui se fige.

| Option Ladybird de `run.ps1` | Effet |
|---|---|
| `-LadybirdUrl <url>` | page de départ, `http://` ou `https://` |
| `-LadybirdSansChrome` | retire la barre d'outils, revient à la capture unique de M9 |
| `-LadybirdRamMiB <n>` | RAM donnée à QEMU (8192 par défaut) |
| `-LadybirdM9Test` | régression HTTP déterministe sur fixture locale |
| `-LadybirdM8` | régression HTML local finie |

Ce qui manque encore : **la résolution de nom** (ci-dessus), les onglets (M13),
le bac à sable du renderer (M14), le redimensionnement de la fenêtre, et l'écran
d'avertissement pour un certificat invalide. Sans `-Ladybird`, `run.ps1` lance le navigateur Qt/QuickJS documenté
plus haut, qui reste le mode par défaut.


**Jamais un userland d'un autre commit sans le dire.** Une image d'un autre jour
ne se signale pas : elle démarre, et elle se comporte comme le système d'alors.
La panne qui suit accuse le code source, qui n'y est pour rien.

`run-fullscreen.ps1` et `boot.ps1` enveloppent les mêmes commandes ;
`check.ps1` enchaîne les vérifications.

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
| `docs/ladybird/M11_NAVIGATEUR.md` | **le chrome du navigateur natif : barre d'adresse, entrées, trames** |
| `docs/ladybird/M12_HTTPS.md` | HTTPS, autorités racine, résolution de noms |
| `docs/ARCHITECTURE.md` | découpage du noyau et du userland |
| `docs/BROWSER_ISOLATION.md` | le modèle multiprocessus du navigateur |
| `docs/BROWSER_RENDERER_PROTOCOL.md` | le protocole navigateur ↔ renderer |
| `docs/RENDERER_PRIVILEGE_AUDIT.md` | ce que le renderer peut encore faire |
| `docs/ROADMAP.md` | journal des versions passées (pas un plan : voir `VISION.md`) |
| `docs/WEB_ENGINE_MODULES.md` | carte des modules du moteur Web |
