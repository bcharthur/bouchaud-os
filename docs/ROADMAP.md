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
- [~] Brotli (`br`) : flux + meta-blocs non compresses ; compresse =
      necessite le dictionnaire statique 122 Ko (RFC 7932 Annexe A)

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

## Au-dela
- [ ] Processus et ordonnanceur
- [ ] Syscalls + split user/kernel
- [ ] Permissions completes, audit log
- [ ] Signature du noyau, secure boot
- [ ] Interface graphique
