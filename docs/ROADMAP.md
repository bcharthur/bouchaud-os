# Roadmap Bouchaud OS

OS souverain francais experimental, from scratch, en Rust `no_std`.
Etat des versions : `[x]` fait, `[~]` prepare/stub, `[ ]` planifie.

## V0.35 - Navigateur AUTONOME par defaut (Nautile), zero proxy
- [x] Le bureau ouvre par defaut **Nautile**, le moteur web from-scratch de
      l'OS -> navigation **100% autonome**, aucun service externe requis.
      "Navigateur" (icone + menu + fenetre d'accueil) = Nautile.
- [x] Verifie en QEMU que Nautile est reellement autonome : handshake TLS 1.3
      MAISON avec un vrai site HTTPS (pypi.org), recuperation du HTML + des CSS
      (90+84+18 Ko) + du JS (107 Ko) et EXECUTION du JS, sans aucun proxy.
- [x] Recherche par defaut de la barre d'adresse = DuckDuckGo **HTML** (rendu
      cote serveur, donc affichable par Nautile) au lieu de Google (100% JS).
      Bangs conserves (`!g` Google, `!w` Wikipedia, `!yt`...).
- [x] WebView (rendu Chromium deporte) devient l'option **secondaire "compat"**
      (menu Demarrer -> "WebView (compat)"), a n'utiliser que pour les sites
      100% JavaScript et seulement si on veut lancer le proxy sur un PC.
- Limite assumee : Nautile rend fidelement le web rendu cote serveur (docs,
      wikis, moteurs HTML) mais pas les SPA complexes (Google/YouTube) ni le CSS
      le plus lourd (pypi.org s'affiche partiellement) -- c'est l'ecart normal
      d'un moteur from-scratch face a Chromium, et c'est justement ce que couvre
      le mode WebView compat.

## V0.34 - WebView + reseau rapide (smoltcp)
- [x] Serveur `tools/render-proxy` v2 (Chromium interactif) + app WebView.
- [x] Ecran d'accueil propre (titre, instructions), barre d'adresse avec invite
      "Rechercher ou saisir une adresse", messages d'erreur centres, detection
      auto du proxy (`10.0.2.2:8080` puis IP LAN) via `/healthz`.
- [x] **Reseau ~200x plus rapide** : le fetch HTTP en clair passe desormais par
      la vraie pile `smoltcp` (RFC 793 : controle de congestion, fast-retransmit,
      fenetre glissante) au lieu de la pile maison. Un rendu de page (PNG 44 Ko)
      tombe de ~250 s a **< 1 s** ; une capture de 264 Ko charge en ~300-900 ms.
      Repli automatique sur la pile maison si smoltcp echoue. Voir
      `net/mod.rs::fetch_document` et `net/transport/smol_tcp.rs`.
      (Diagnostic de la lenteur maison : pertes de segments cote reception +
      retransmissions au RTO ; smoltcp gere ca nativement.)
- [x] Valide en QEMU : bureau -> Navigateur ouvert d'office, saisie d'URL,
      rendu reel de pypi.org en ~1 s, clic-navigation (`/` -> `/search/`),
      bouton precedent, molette (scroll), barre d'adresse synchronisee.

## V0.33 - WebView : navigation web moderne (proxy Chromium)
- [x] App bureau WebView : affiche une page rendue par un vrai Chromium headless
      sur l'hote et lui renvoie chaque interaction -> JS, SPA, formulaires,
      connexion a un compte (architecture "cloud" facon Opera Mini / Puffin).
- [x] Serveur `tools/render-proxy` v2 : sessions Chromium persistantes,
      endpoints `/wv/open|shot|click|scroll|type|key|back|forward|reload|url`
      renvoyant une capture PNG du viewport (coordonnees 1:1 avec la fenetre OS).
- [x] Reassemblage TCP HORS-ORDRE + option MSS 1460 (`net/transport/tcp.rs`)
      pour la pile maison (utile en repli et pour les autres chemins).
- [ ] Pas encore de multi-onglets dans l'app WebView ; liens cliques via le
      hit-test reel de Chromium (`/wv/click`), pas de carte de liens locale.

## V0.32 - Python complet + pip (RustPython via WASM/WASI)
- [x] Vrai interprete Python 3.13 (RustPython 0.5 + stdlib native : `os`,
      `open()`, `json`, `re`, `math`, `hashlib`, `zlib`, `csv`, `random`,
      `struct`, `unicodedata`...) precompile en wasm32-wasip1, execute par le
      runtime `wasmi` existant. Binaire ~16 Mo embarque via `include_bytes!`
      (recette : `tools/python-wasm/`)
- [x] `python` (REPL interactif), `python <f.py> [args]`, `python -c "code"`,
      `input()`, `sys.argv`, vrais tracebacks, codes de sortie
- [x] Pont WASI preview1 complet -> RAMFS (`src/wasm/mod.rs`) : path_open,
      fd_read/write/seek/readdir, mkdir/unlink/rename..., permissions Unix de
      la session, preouvertures `/` et `.` (chemins relatifs), stdio interactif
      (clavier/VGA au fil de l'eau), `time.sleep` reel via poll_oneoff + PIT
- [x] `pip install <paquet>` / `pip list` : installeur cote noyau (PyPI via la
      pile TLS 1.3 maison, unzip via l'inflate maison) pour les wheels pures
      `py3-none-any` -> `/usr/lib/python/site-packages`, deps recursives non
      conditionnelles ; les paquets s'importent normalement (PYTHONPATH)
- [x] RAMFS refondu : contenu des fichiers dynamique (`Vec<u8>`, 4 Mo max),
      1024 inodes, noms 64 car. (prerequis scripts/paquets ; l'ancien format
      768 octets/fichier ne pouvait rien stocker d'utile)
- [x] Toolchain epinglee (`rust-toolchain.toml`, nightly-2026-06-01) : le
      nightly flottant cassait la cible custom (rustc-abi renomme, trait Step
      du crate x86_64 0.14) ; `run.ps1`/`check.ps1`/`boot.ps1` sans `+nightly`
- [x] Valide en QEMU : boot, login, python-selftest OK, script `.py` depuis le
      RAMFS, REPL (`1+1` -> `2`, `import six` apres `pip install six`),
      `pip install six` reel (DHCP + TLS + PyPI) en ~10 s
- [ ] Demarrage de l'interpreteur lent sous emulation TCG (~30 s : init de la
      VM Python interpretee par wasmi) ; pistes : accel QEMU (WHPX), wasmi
      recent (compilation paresseuse), snapshot memoire post-init
- [ ] Paquets a extensions C (numpy...) hors de portee (pas de cc cible) ;
      `socket` indisponible dans le bac a sable WASM (pas de sockets WASI p1)

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

## V0.27.1 - Stabilite rendu (anti-crash, anti-fuite)
- [x] Tas 48 MiB (fin du KERNEL PANIC "allocation failed" sur pages lourdes)
- [x] Pre-traitement bulletproof : <script>/<style> retires avant parsing
      (le code CSS/JS ne peut plus fuiter dans le rendu), CSS extrait a part
- [x] Bornes memoire : corps HTML plafonne (1,5 Mo analyse / 4 Mo lu),
      images > ~1,2 Mpx refusees (placeholder) pour eviter l'OOM
- [ ] JavaScript : execution hors de portee (pas de moteur JS) -> les sites
      entierement rendus cote client (Google/YouTube) restent quasi vides

## V0.27 - CSS + couleurs + images
- [x] Primitives truecolor (fill_rect_rgb / draw_text_rgb / blit_rgb)
- [x] Moteur CSS (subset) : `<style>` + `style=""`, selecteurs balise/.classe/#id,
      cascade par specificite ; color, background, font-size, font-weight (gras),
      text-align, display:none / visibility:hidden
- [x] Rendu truecolor : couleurs reelles, titres dimensionnes, gras, centrage,
      fonds de blocs
- [x] Decodeur PNG from-scratch (via notre zlib) : gris 1/2/4/8b, RGB, RGBA
      (alpha sur blanc), palette ; valide contre Pillow (vraies images web)
- [x] Images dans le navigateur : `<img>` data:URI (base64) + fetch reseau
      (plafonne) ; downscale plus-proche-voisin (rendu pixelise, DA bitmap)
- [ ] JPEG/GIF/WebP, CSS box-model (margin/padding), flex/grid, formulaires actifs

## V0.26 - Moteur de rendu web graphique
- [x] gui/web.rs : HTML -> DOM (parseur tolerant) -> layout flux blocs/inline
      (retour a la ligne, titres dimensionnes, listes, regles, champs de form)
      -> liste d'affichage peinte dans le framebuffer HD avec defilement
- [x] Liens colores cliquables (souris) + navigation par numero/URL au clavier
- [x] Navigateur graphique : about:/file:/http(s) ; fetch via fetch_document
- [x] ALPN force HTTP/1.1 (chemin robuste : chunked + gzip/deflate/brotli) pour
      fiabiliser Google/Cloudflare (h2 contourne)
- [x] Moteur valide hors-ligne : 0 panic sur HTML malforme/imbrique/gros (320 Ko)
- [x] Perf : opt-level=2 en debug (handshake TLS ~20x plus rapide, fin du "lag")
- [x] Timeouts reseau bases sur l'horloge PIT (robustes quel que soit l'opt-level)
- [x] Ecran "Chargement..." avant le fetch (retour visuel immediat)
- [ ] CSS (couleurs/box model), images decodees, formulaires actifs
- [ ] Google : ferme la connexion sur client minimal (h1) -> piste h2/fingerprint

## V0.25 - Bureau HD truecolor (Bochs VBE)
- [x] Framebuffer lineaire HD 1280x720x32 via Bochs VBE/BGA (carte `-vga std`)
- [x] gfx reecrit en truecolor 32 bits (double-buffer RAM -> LFB), API conservee
      (le window manager et les apps passent en HD sans modification)
- [x] LFB localise via le BAR0 PCI de la carte graphique + offset memoire phys
- [x] Tas porte a 16 MiB (double-buffer HD + tampons reseau/TLS)
- [x] `desktop` bascule en HD ; `leave()` restaure le mode texte VGA pour le shell
- [ ] Police mise a l'echelle / polices HD, anti-aliasing
- [ ] Migration bootloader 0.11 (framebuffer fourni au boot, mode texte FB)

## V0.24 - Compatibilite web moderne (en cours)
- [x] HTTPS/TLS 1.3 reel : handshake complet, X.509, magasin de CA
- [x] 3 suites TLS 1.3 : AES-128-GCM, AES-256-GCM (SHA-384), ChaCha20-Poly1305
- [x] Decompression gzip/deflate (RFC 1950/1951/1952) cote reponse HTTP
- [x] HTTP/1.1 : Content-Length, Transfer-Encoding: chunked, redirections 3xx
- [x] Navigateur texte : HTML->texte, titre, entites, liens numerotes
- [x] Rendu enrichi : titres (#), listes (-), citations (>), images [img],
      table d'entites etendue, retour a la ligne auto dans le navigateur GUI
- [x] Alertes TLS lisibles (RFC 8446 §6) : handshake_failure, unknown_ca... (alert.rs)
- [x] Post-handshake : NewSessionTicket / KeyUpdate ignores proprement
- [x] ECDHE P-256 (secp256r1) + HelloRetryRequest (rejoue le ClientHello)
- [x] HTTP/2 : ALPN h2, frames, HPACK (table statique+dynamique+Huffman), GET
- [ ] Reprise de session (PSK / NewSessionTicket)
- [x] Brotli (`br`) COMPLET (RFC 7932) : codes simples/complexes, tables
      statique+dynamique, context maps, distances, dictionnaire 122 Ko + 121
      transformations ; valide hors-ligne contre python-brotli (q0..11)
- [x] Police de contenu agrandie (x2) dans le navigateur (lisibilite 720p)

## V0.22 - Cloture de la couche reseau : DHCP (+ socle TLS)
- [x] Client DHCP (DORA) : IP/passerelle/DNS automatiques (config dynamique)
- [x] Config reseau dynamique (our_ip/gateway/dns_server + set_config)
- [x] Socle TLS honnete (couche record) ; handshake NON implemente
- Couche reseau consideree COMPLETE pour le perimetre realiste :
  Ethernet/ARP/IPv4/ICMP/UDP/DNS/DHCP/TCP/HTTP + loopback
- [ ] HTTPS/TLS = chantier crypto dedie (X25519, AES-GCM, SHA-256, X.509...)
      hors perimetre from-scratch a court terme

## V0.21 - Pile reseau : UDP/DNS/TCP/HTTP
- [x] UDP (net/udp.rs) + client DNS (net/dns.rs) : commande dns <nom>
- [x] ping reel IPv4/ICMP via e1000 (V0.20)
- [x] client TCP minimal (net/tcp.rs) : SYN/SYN-ACK/ACK, envoi, ack, FIN
- [x] HTTP/1.0 GET (net/http.rs) : commandes wget/http <url>
- [x] Bouchaud Browser charge les pages http:// reelles
- [ ] HTTPS = TLS (handshake + crypto) : chantier lourd, plus tard
- [ ] DHCP (IP auto), retransmission TCP, gestion fine des fenetres

## V0.19 - Driver reseau e1000 (bring-up)
- [x] map_physical_memory (offset phys) + arene DMA (kernel::memory)
- [x] PCI: lecture BAR, bus mastering (config write)
- [x] drivers/e1000.rs : reset, lecture MAC, anneaux RX/TX, send/receive
- [x] commandes : ifup, ethinfo, arping <ip> (ARP reel via la carte)
- [x] init a la demande (le boot n'est pas affecte)
- [ ] IPv4/ICMP reels (ping externe), DHCP, DNS, UDP/TCP, HTTP, TLS
- Test QEMU : `-device e1000 -netdev user,id=n0`

## V0.18 - Decoupage GUI modulaire (window manager en sous-modules)

## V0.17 - Resolution superieure (640x480)
- [x] Bureau en mode VGA 12h : 640x480, 16 couleurs (planaire)
- [x] Double-buffer lineaire + conversion planaire (4 plans) au present()
- [x] Tas porte a 4 MiB (backbuffer ~300 Ko)
- [x] Sans risque boot : gate derriere `desktop`, shell texte intact
- [ ] Vraie HD truecolor (1280x720+/32 bits) = migration bootloader 0.11

## V0.16 - Fenetres avancees
- [x] Minimiser / maximiser / restaurer (boutons de titre)
- [x] Redimensionnement (poignee coin bas-droit)
- [x] Fond d'ecran deux tons
- [x] Barre des taches : restaure les fenetres minimisees
- [ ] Drag&drop entre apps, themes configurables

## V0.15 - Window manager + apps natives (Windows-like)
- [x] Gestionnaire de fenetres : multi-fenetres, focus/z-order, deplacement,
  fermeture, boucle d'evenements (clavier non bloquant)
- [x] Menu Demarrer + barre des taches (tuiles par fenetre)
- [x] Apps natives : Terminal, Fichiers, Moniteur, Bouchaud Browser
- [x] Bouchaud Browser : about:bouchaud, about:system, file:/<chemin>
- [x] Modele d'app : /apps/*.bapp (manifestes)
- [ ] Redimensionnement, drag&drop, themes
- [ ] Runtime .bapp generique

## V0.14 - Apps du bureau
- [x] Lanceur a 4 boutons (Terminal, Fichiers, Moniteur, Quitter)
- [x] App Fichiers : navigateur a la souris (dossiers, apercu fichier, droits)
- [x] App Moniteur : infos systeme en direct (heure, uptime, heap, CPU, PCI)
- [ ] Fenetres multiples simultanees + gestion du focus
- [ ] Editeur graphique

## V0.13 - Bureau graphique (phase 2)
- [x] Correctif retour mode texte : rechargement de la police VGA (plus de
  rayures), Echap instantane (drainage de la file clavier)
- [x] Terminal graphique interactif : REPL reutilisant tout le shell
  (commandes, pipes, redirections, $VAR) avec scrollback
- [x] Lanceur d'applications dans la barre des taches (Terminal, Quitter)
- [ ] Plusieurs fenetres/apps simultanees, focus
- [ ] Haute resolution (migration bootloader 0.11)
- [ ] Mini-navigateur texte HTTP (apres reseau e1000)

## V0.12 - Bureau graphique (phase 1)
- [x] Mode VGA 13h (320x200x256) : framebuffer + double-buffer + palette
- [x] Police bitmap 8x8, primitives (pixel, rect, fill, texte)
- [x] Souris PS/2 (IRQ12) + curseur
- [x] Bureau : fond, barre des taches, horloge RTC, fenetre deplacable
- [x] Commande `desktop` (Echap pour revenir au shell texte)
- [ ] Fenetre terminal interactive (reutiliser le shell dans le GUI)
- [ ] Lanceur d'applications + apps natives
- [ ] Haute resolution (migration bootloader 0.11) [plus tard]

Note de cadrage : un vrai navigateur web (HTML/CSS/JS/HTTPS), l'execution de
.exe (Windows) ou .jar (JVM), et l'integration d'un compilateur type gcc/rustc
sont hors de portee d'un OS from-scratch. Cibles realistes : apps maison +
scripts .bsh, et un mini-navigateur texte HTTP une fois le reseau e1000 pret.

## V0.11 - Userland
- [x] Horloge RTC (commande date)
- [x] Coreutils : grep, wc, head, tail, find (lisent fichier ou stdin)
- [x] Pipes cmd1 | cmd2 (capture en pile)
- [x] Variables d'environnement (export/env/unset, $NOM, ${NOM})
- [x] Scripts .bsh (run/source)
- [x] Editeur plein ecran edit (fleches, sauvegarde/quitter)
- [ ] Horodatage des fichiers (mtime) avec la RTC

## V0.10 - Tas (alloc) + shell pro
- [x] Allocateur de tas (linked_list_allocator, 1 MiB) -> Vec/String/BTreeMap
- [x] Chainage de commandes : ; && ||
- [x] Redirections : > et >>
- [x] Historique navigable (fleches haut/bas) + tab-completion (commandes/chemins)
- [x] Code de retour $? + builtins true/false
- [ ] Pipes | (necessite plomberie stdin/stdout)
- [ ] Variables d'environnement / export

## V0.9 - Comptes utilisateurs dynamiques
- [x] Base d'utilisateurs en table fixe (root + guest par defaut)
- [x] Ecran de connexion au boot (login + mot de passe masque)
- [x] useradd / userdel / passwd / users / su
- [x] chmod symbolique (+x, u+w, go-r, a=rx) en plus de l'octal
- [x] chown base sur la base d'utilisateurs
- [ ] /etc/passwd persistant (apres FS disque)
- [ ] groupes multiples par utilisateur

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

## Mode utilisateur (ring 3) et ABI Linux

Objectif : executer des binaires Linux x86-64 non modifies, jusqu'a une pile
graphique complete (navigateur webview / Python / Qt). Detail et chaine de
construction cote utilisateur : `tools/userland/README.md`.

- [x] **Memoire virtuelle** : allocateur de frames physiques + une PML4 par
      processus, creneau utilisateur dedie de 512 Gio (`src/kernel/vmm.rs`)
- [x] **Ring 3** : GDT complete (segments ring 0/3), TSS avec RSP0 par tache,
      `syscall`/`sysretq` via STAR/LSTAR/FMASK, `iretq`, base FS pour le TLS,
      SSE actif (`src/arch/x86_64/{gdt,usermode}.rs`)
- [x] **Chargeur ELF64** : `PT_LOAD` avec droits reels, `.bss`, `PT_TLS`,
      `PT_INTERP`, vecteur auxiliaire complet (`src/kernel/elf.rs`)
- [x] **Appels systeme POSIX** : ~75 appels aux numeros et structures Linux
      (`src/kernel/abi/`)
- [x] **Processus et threads** : `clone(CLONE_THREAD)`, futex, piles noyau par
      tache, preemption sur IRQ0 depuis le ring 3 (`src/kernel/task.rs`)
- [x] **libc musl statique** : valide avec un binaire `musl-gcc -static-pie`
      non modifie (printf, malloc, stdio, pthread, TLS)
- [x] **Editeur de liens dynamique** : `ld-musl-x86_64.so.1` charge par le
      noyau et resolvant les relocations en ring 3
- [x] **Runtime C++** : STL, exceptions (deroulement), `std::thread`,
      destructeurs — valide avec libstdc++ statique
- [x] **Serveur graphique** : `/dev/fb0` mmapable sur la VRAM, ioctls fbdev
      (`FBIOGET_*SCREENINFO`), `/dev/tty0` avec `KDSETMODE`/`VT_*`, et
      `/dev/input/event*` en evdev reel (codes de touches Linux, souris
      relative, bitmaps `EVIOCGBIT`)
- [x] **Boucle d'evenements** : `eventfd` (compteur), `timerfd`, `poll`/`ppoll`,
      `epoll`, futex a echeance absolue — valide par `tools/userland/qpa-probe.c`
      qui rejoue le demarrage du plugin `linuxfb` de Qt
- [x] **Tick a 1000 Hz** : les 18,2 Hz du PIT par defaut donnaient 55 ms de
      granularite, inutilisable pour animer une interface
- [x] **Arborescence systeme** : polices DejaVu dans `/usr/share/fonts`, `/proc`
      et `/sys` reduits aux fichiers lus au demarrage, `/etc` minimal
- [x] **Processus complets** : `fork`, `vfork`, `execve`, `wait4`, filiation,
      zombies et recolte du code de sortie (`src/kernel/abi/proc.rs`)
- [x] **Signaux reels** : trame ecrite sur la pile utilisateur, gestionnaire
      ring 3, `rt_sigreturn`, masques, `SIGCHLD`, `SIGALRM` (`src/kernel/signal.rs`)
- [x] **Sockets POSIX** : TCP et UDP sur la pile de `src/net/` — `getaddrinfo`
      et un client HTTP fonctionnent depuis le ring 3 (`src/kernel/abi/net.rs`)
- [x] **Cache de pages partage** : `MAP_SHARED` sur fichier partage les memes
      frames entre processus, `msync` repercute vers le RAMFS
- [ ] Sockets serveur : `listen`/`accept` (demande une reception en tache de
      fond et un demultiplexage par port)
- [ ] IPv6

- [x] **Installation sans recompilation** : pilote ATA PIO + archive `tar`
      depliee au demarrage depuis un second disque (`src/drivers/ata.rs`,
      `src/fs/tar.rs`). Fabriquer l'image avec `tools/userland/mkdisk.sh` ;
      `run.ps1` l'attache automatiquement. Limites RAMFS relevees a 4096
      inodes et 64 Mio par fichier.
- [x] **Mode non interactif** : un `/autorun` sur le disque de donnees est joue
      au demarrage a la place de la connexion, la sortie est recopiee sur COM1
      et la machine s'eteint en rendant un verdict a l'hote
      (`src/kernel/autorun.rs`, `src/kernel/power.rs`). C'est ce qui rend les
      sondes rejouables : `./tools/test.sh` construit, boote, verifie et renvoie
      un code de retour.
- [x] **Invariant de non-reentrance verrouille** : la regle (« aucun emprunt du
      `Process` ne survit a un point de commutation ») est enoncee en tete de
      `src/kernel/abi/mod.rs` et verifiee par un `debug_assert` dans
      `task::schedule` — la panique designe le coupable, pas sa victime.
- [x] **CPython 3.12 tourne sur la machine** : interprete statique-PIE contre
      musl, bibliotheque standard en archive zip lue par `zipimport`
      (`tools/userland/build-python.sh`). Deux fichiers sur le disque de
      donnees, aucune recompilation du noyau.
- [x] **Qt 5.15 dessine sur `/dev/fb0`** : qtbase statique, plateforme linuxfb,
      entrees evdev, lie en dur dans le binaire
      (`tools/userland/build-qt.sh`, `tools/userland/qt-demo.cpp`). QPainter,
      anticrenelage, degrades et melange alpha, en ring 3.
- [ ] **`sendto` non bloquant et resolution d'adresse materielle** (defaut
      ouvert). Une prise UDP passee en non bloquant fait echouer sa premiere
      emission vers un hote dont l'adresse materielle n'est pas encore connue :
      le noyau rend `ENETUNREACH` au lieu de laisser l'ARP aboutir. Contourne
      cote navigateur en emettant avant de poser le delai d'attente.
- [x] **Le drapeau d'interruption traversait les commutations de tache**. C'est
      la cause des deux gels qu'on avait notes comme distincts : le blocage
      intermittent apres le `pthread_create` de `qpa-probe`, et celui de
      `load_url` appele depuis le fil de `webview.start()`.

      `switch_context` sauvegardait les registres callee-saved mais pas RFLAGS.
      Or `IF` est un etat du processeur, pas de la pile : il suivait donc la
      **nouvelle** tache. Les deux appelants n'ont pas le meme etat —
      `schedule` commute depuis un appel systeme, interruptions actives, tandis
      que `preempt_from_irq` commute depuis le gestionnaire du timer, ou le CPU
      les a coupees en franchissant la porte d'interruption. Preempter une tache
      livrait donc `IF=0` a celle qui reprenait la main, au beau milieu de son
      appel systeme. Le plus souvent sans consequence visible — elle rendait la
      main en ring 3, et `sysretq` y remet un RFLAGS correct. Mais si elle
      attendait dans un `poll`, un `futex` ou un sommeil, son `hlt` arretait le
      processeur alors que plus aucune interruption ne pouvait le reveiller :
      machine gelee, sans faute ni message, `timer::ticks()` fige pour de bon.
      D'ou le caractere intermittent — il fallait que la preemption tombe
      exactement sur une tache en attente — et l'aggravation sous charge de la
      machine hote.

      Releve sous moniteur QEMU : `RIP` juste apres le `hlt` de `sys_poll`,
      `RFL=0x00000006` (donc `IF=0`), `HLT=1`, `CPL=0`.

      Corrige a deux niveaux. `switch_context` encadre desormais sa sauvegarde
      d'un `pushfq`/`popfq` : chaque tache retrouve l'etat d'interruption qui
      etait le sien, et l'amorce de pile de `Task::new` porte le RFLAGS de
      depart correspondant. Et les attentes bloquantes du noyau appellent
      `cpu::wait_for_interrupt` (`sti; hlt`, paire indivisible) au lieu d'un
      `hlt` nu, de sorte qu'aucune ne puisse plus s'endormir sans reveil
      possible. Un `debug_assert` dans `schedule` verrouille l'invariant : on ne
      commute jamais interruptions coupees, et la panique arrive dans le
      coupable plutot que dans la victime.
- [x] **Navigateur natif en ring 3** : Nautile supprime du noyau, remplace par
      `tools/userland/navigateur/` — un binaire unique (Qt + CPython + le
      moteur) qui analyse HTML et CSS, met en page, et peint par QPainter sur
      `/dev/fb0`. Reseau HTTP/HTTPS avec resolveur DNS ecrit pour l'occasion.
      Pas de JavaScript, pas d'images.
- [x] **pywebview tourne sur la machine** : la bibliotheque est embarquee telle
      quelle avec un moteur d'affichage ecrit pour l'OS
      (`tools/userland/navigateur/webview_bouchaud.py`, greffe par
      `greffe-pywebview.sh`). Le code d'un tutoriel pywebview s'execute sans
      modification. Sans JavaScript (`evaluate_js` leve), et sans les
      applications a fichiers locaux, qui passent par le serveur HTTP interne.
- [x] **`load_url` depuis le fil de `webview.start()`** : le tutoriel pywebview
      se deroule maintenant en entier, sans modification. Le fil de travail
      appelle `load_url`, le fil principal depile la demande a son battement, le
      moteur resout le nom, ouvre la connexion, recoit la page et la peint ; le
      fil de travail retrouve `events.loaded`, puis `get_current_url()` et la
      taille de la fenetre. Le gel etait celui du drapeau d'interruption
      ci-dessus, pas un defaut du backend.
- [ ] **Sockets serveur : `listen` / `accept`**. C'est ce qui manque au serveur
      HTTP interne de pywebview, donc a toute application webview servant des
      fichiers locaux. Une mise en œuvre limitee au bouclage suffirait : le
      serveur ecoute sur 127.0.0.1 et le moteur s'y connecte, tout reste dans le
      noyau sans passer par la carte reseau.
- [ ] Ecriture persistante : BFS sur peripherique bloc

Commandes associees : `exec`, `elfinfo`, `usermode`, `tasks`, `vmstat`,
`syscalls`, `strace`, `df`, `poweroff`.

## Au-dela
- [ ] Permissions completes, audit log
- [ ] Signature du noyau, secure boot
- [ ] Multi-CPU (APIC, SMP)
