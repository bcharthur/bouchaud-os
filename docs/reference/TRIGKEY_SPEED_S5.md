# TRIGKEY Speed S5 — ce que la machine porte, et ce que le noyau en fait

Machine de reference de Bouchaud OS. Ryzen 7 5700U (Zen 2 « Lucienne »,
8 cœurs / 16 fils, FP6), 32 Gio DDR4, NVMe M.2 de 500 Go.

Ce document ne decrit pas ce qu'on voudrait. Il decrit, composant par
composant, **comment le noyau le trouve**, **ce qu'il en fait**, et **ce qu'on
voit quand cela ne marche pas** — parce que cette machine n'a pas de port
serie, et que tout ce qu'on ne montre pas a l'ecran est perdu.

---

## Le probleme de fond : aucune console serie

Un mini-PC n'expose pas de RS-232. Toutes les traces `serial_println!` du
noyau partent donc dans le vide sur cette machine. C'est la contrainte qui
gouverne tout le reste :

* le diagnostic materiel est **dessine sur le framebuffer GOP**
  (`platform/pc/physical_diag.rs`), pas seulement journalise ;
* un echec d'entree doit se lire a l'ecran, sinon l'utilisateur voit un bureau
  sur lequel il ne peut rien faire et rien ne lui dit pourquoi ;
* `sync` avant extinction n'est pas un confort : ce qui n'est pas sur le disque
  ne laisse aucune trace consultable apres coup.

Sous QEMU, la meme image ecrit sur le port serie et l'on a les deux.

---

## Affichage — Radeon Vega 8, par le micrologiciel

| | |
|---|---|
| **Comment on le trouve** | On ne le cherche pas. Le micrologiciel UEFI a deja programme un mode video et remet son adresse dans `BootInfo::framebuffer`. |
| **Ce que le noyau en fait** | `drivers::gfx::install_firmware_framebuffer` prend le tampon tel quel, a sa resolution NATIVE. |
| **Marqueurs** | `BOUCHAUD_TRIGKEY_STAGE2_EARLY_GOP_OK`, `BOUCHAUD_STAGE2_NATIVE_VIEWPORT_OK` |
| **Ce qui manque** | Aucun pilote Vega. Pas d'acceleration, pas de changement de mode, pas de gestion d'ecran multiple. |

Le Stage 2 **refuse de demarrer** si le viewport n'est pas natif
(`panic!("stage2: viewport GOP non natif")`). C'est deliberé : un bureau
etire sur un mode qui n'est pas celui de l'ecran est pire qu'un refus, parce
qu'on ne sait pas d'ou vient le flou.

---

## Stockage — NVMe M.2

| | |
|---|---|
| **Comment on le trouve** | Balayage PCI, classe `0x01` sous-classe `0x08`. `pci::find_nvme()`. Le controleur est derriere un pont racine PCIe : l'enumeration doit descendre au-dela du bus 0, ce qu'elle fait depuis le decodage PCIe. |
| **Ce que le noyau en fait** | `drivers::nvme::bring_up()` : remise a zero, file admin, une file d'entree-sortie, `Identify` du controleur puis du namespace, enregistrement comme volume `api::bloc`. |
| **Marqueurs** | `BOUCHAUD_NVME_GREEN blocs=... taille_bloc=... mio=... modele=...`, ou `BOUCHAUD_NVME_FAIL raison=...` |
| **Taille de bloc** | Lue sur le disque (`FLBAS` → `LBADS`). Jamais supposee : un M.2 formate en 4096 lu comme du 512 donne des adresses huit fois trop loin. |
| **Ce qui manque** | Pas d'interruption (MSI-X) : le pilote SCRUTE. Pas de file par CPU. Pas de gestion d'erreur au-dela du statut. |

---

## USB — deux controleurs xHCI AMD

C'est la partie qui decide si la machine est **utilisable**, et celle qui a le
plus de facons de rater en silence.

### Comment on les trouve

Balayage PCI, classe `0x0c` sous-classe `0x03` **interface `0x30`** — la
derniere compte : `0x00` serait de l'UHCI, `0x10` de l'OHCI, `0x20` de l'EHCI.
Sur FP6 il y en a typiquement **deux** (un du FCH, un du SoC), et les ports
physiques sont repartis entre eux. Le pilote les prend **tous** ; n'en prendre
qu'un laisse la moitie des prises muettes.

### Les quatre pieges du materiel reel

**1. Le micrologiciel possede le controleur.** C'est ainsi qu'un clavier USB
marche dans le menu du BIOS. La remise passe par un semaphore dans la capacite
« USB Legacy Support » : on pose « OS possede » et on attend que « BIOS
possede » tombe. Certains ne le baissent jamais — on force alors la reprise
(`BOUCHAUD_XHCI_HANDOFF_FORCE`).

**2. Les SMI du micrologiciel restent armes.** Prendre le controleur ne suffit
pas. Tant que les autorisations de `USBLEGCTLSTS` sont posees, le
micrologiciel continue d'etre APPELE sur chaque evenement USB, par une
interruption de gestion systeme que le noyau ne voit pas et ne peut pas
masquer. Le symptome est exactement celui qu'on cherche : **un clavier qui
marche dans le BIOS et pas dans le systeme**. Sur une machine AMD dont le BIOS
propose « USB legacy emulation », c'est le cas normal, pas le cas rare.
`BOUCHAUD_XHCI_SMI_DESARMES avant=... apres=...`

**3. Un port USB2 et un port USB3 ne se reinitialisent pas pareil.** Le premier
veut `PORTSC.PR`, le second `PORTSC.WPR`. Et une souris USB 1.1 branchee dans
une prise bleue apparait sur le NUMERO DE PORT USB2, pas sur celui qu'on
regarde. Le pilote lit la capacite « Supported Protocol » pour savoir lequel
est lequel, au lieu de deviner.

**4. Un concentrateur se traverse, et rien de ce qu'il faut pour cela ne se
voit.** Un clavier branche sur un hub — ou sur les prises USB d'un ecran, ou
d'un clavier qui en integre un — n'est pas sur un port du controleur. Pour
l'atteindre il faut quatre choses, dont aucune ne produit de message d'erreur
quand elle est fausse :

* la **chaine de route**, cinq etages de quatre bits, ou le numero de port du
  concentrateur va a l'etage de SA profondeur. Un etage de decalage designe un
  autre sous-arbre — et l'adressage y reussit, sur le mauvais peripherique ;
* le **bit `Hub`** du contexte de slot, pose par une commande *Configure
  Endpoint*. Sans lui, le controleur ne sait pas qu'il y a un « derriere » et
  refuse d'adresser quoi que ce soit ;
* le **transactionneur** : un bus haute vitesse ne transporte pas directement
  une transaction basse vitesse, et la quasi-totalite des claviers filaires
  sont basse vitesse. Sans le slot et le port du concentrateur qui traduit, le
  clavier est adresse et ne repond a rien. Il **s'herite** : un concentrateur
  pleine vitesse derriere un concentrateur haute vitesse garde celui de son
  ancetre ;
* les **requetes de classe** du concentrateur, adressees au PORT et non au
  peripherique — un `SET_FEATURE(RESET)` adresse au peripherique
  reinitialiserait le concentrateur entier.

Le pilote fait les quatre. La descente est **en largeur** : chaque
peripherique trouve est mis en file et enumere a plat, jamais par recursion —
cinq etages de tampons de descripteurs sur une pile de noyau deborderaient
sans rien dire. `BOUCHAUD_USB_CONCENTRATEUR_TRAVERSE ... occupes=N` dit ce qui
a ete trouve, `BOUCHAUD_USB_ADDRESS_OK ... profondeur=1` dit qu'un
peripherique a ete atteint derriere.

Ce qui reste hors de portee, et se dit : au-dela de **cinq** concentrateurs en
cascade la chaine de route est pleine
(`BOUCHAUD_USB_CONCENTRATEUR_TROP_PROFOND`) — c'est une limite du champ xHCI,
pas du code. Un concentrateur SuperSpeed n'est traverse que par sa moitie USB
2.0, ce qui suffit : un clavier ou une souris y est toujours basse ou pleine
vitesse. Et une traversee qui echoue est comptee et nommee
(`BOUCHAUD_USB_CONCENTRATEUR_ECHEC`), parce qu'un clavier absent et un
concentrateur en echec se ressemblent trop.

La commande **`lsusb`** rejoue l'arbre depuis l'etat, a n'importe quel moment.
L'indentation dit la profondeur. C'est ce qui repond a la seule question qui
compte en premier quand un clavier ne marche pas : a-t-il ete VU ? Un clavier
absent de la liste est un probleme d'enumeration, un clavier present mais muet
un probleme de transport — deux enquetes differentes.

### Clavier et souris

| | |
|---|---|
| **Ce qu'on configure** | `SET_PROTOCOL(boot)` puis `SET_IDLE`, une extremite d'interruption entrante par interface HID. Les peripheriques COMPOSITES sont traites : un recepteur sans fil expose souvent une interface clavier ET une interface souris, et les deux sont armees. |
| **Ce qu'on lit** | Un rapport de clavier dit les six touches ENFONCEES, pas celles qu'on vient d'appuyer. Les appuis et relachements sont la difference entre deux rapports. Les relachements sortent avant les appuis, sinon deux touches paraissent enfoncees ensemble quand on tape vite. |
| **Comment on scrute** | Pas d'interruption : le bureau appelle `xhci_active::poll()` a chaque tour et **borne son sommeil a 2 ms** tant qu'un HID est scrute. Sans cette borne, une frappe ne reveillerait rien et le clavier paraitrait mort. |
| **Marqueurs** | `BOUCHAUD_HID_KEYBOARD_GREEN`, `BOUCHAUD_HID_MOUSE_GREEN`, `BOUCHAUD_XHCI_V34_SUMMARY ... polling=1` |

**La table des touches est francaise.** L'usage HID `0x64` — la touche
`<`/`>` a gauche du W, absente d'un clavier americain — et la touche menu
`0x65` sont traduites. AltGr porte son prefixe etendu, sans quoi il serait
confondu avec Alt gauche et `@`, `#`, `[`, `]`, `{`, `}` deviendraient
intapables.

---

## Le repli quand l'USB ne donne rien

C'est le filet, et il est **granulaire** : clavier et souris sont decides
separement.

* Aucun clavier USB trouve → on initialise le clavier PS/2.
* Aucune souris USB trouvee → on initialise la souris PS/2.

Un seul drapeau pour les deux forcerait a choisir entre « tout » et « rien » :
une machine dont le clavier USB repond et la souris non se retrouverait sans
pointeur. Et initialiser le PS/2 alors qu'un peripherique USB du meme genre
repond deja ferait arriver chaque frappe DEUX FOIS si le micrologiciel emulait
encore un 8042 par-dessus l'USB.

**Ce que ce repli ne rattrape pas, et il faut le dire :** on vient de desarmer
les SMI. Si le 8042 de cette machine etait une EMULATION de nos peripheriques
USB, il ne repondra plus. Le repli ne sauve que les machines dont le 8042 est
reel. C'est peu, et c'est plus que rien — aujourd'hui, un echec USB donnait un
bureau sans aucune entree.

`BOUCHAUD_STAGE2_ENTREE_DECIDEE claviers_usb=... souris_usb=... ps2_clavier=... ps2_souris=...`

---

## Reseau — Realtek gigabit

| | |
|---|---|
| **Comment on le trouve** | Balayage PCI, classe reseau. `10EC:8168` → pilote `rtl8168` ; `8086:100E` → `e1000` (QEMU). |
| **Ce que le noyau en fait** | Anneaux RX/TX en DMA, 64 descripteurs en reception, DHCP, TCP avec RTO/Karn/retransmission rapide, DNS, TLS. |
| **Marqueur** | `[STAGE2] runtime ... nic=rtl8168` |
| **Ce qui manque** | Pas d'IPv6. Pas de zero-copie. Pas de Wi-Fi du tout — ni Intel AX200/AX201, ni Realtek RTL8821CE. **Sur cette machine, le reseau passe par le cable.** |

---

## Ce que la machine porte et que le noyau ignore

| Composant | Etat |
|---|---|
| **Wi-Fi** (Intel AX2xx ou Realtek) | Aucun pilote. |
| **Audio** (AMD HD Audio + codec Realtek) | Seul l'AC97 existe, qui n'est pas ce que porte cette machine. Pas de son. |
| **Bluetooth** | Aucun pilote. |
| **Batterie / temperatures / ventilateur** | Aucune lecture ACPI au-dela de HPET, MCFG, MADT et FADT. |
| **Suspend / resume** | Absent. La machine s'eteint, elle ne se met pas en veille. |
| **GPU Vega** | Aucun pilote. Le framebuffer du micrologiciel, et rien d'autre. |

---

## Quand cela ne marche pas : par ou commencer

L'ecran de diagnostic s'affiche avant le bureau (`physical_diag::show_usb`).
Il montre l'inventaire en direct pendant quelques secondes, puis les
marqueurs retenus.

| Ce qu'on voit | Ce que cela veut dire |
|---|---|
| `xhci-absent` | Aucun controleur de classe `0c:03:30`. Verifier que le BIOS n'est pas en mode « USB legacy only ». |
| `controllers=0/2` | Les deux controleurs ont refuse de demarrer. Regarder `BOUCHAUD_XHCI_CONTROLLER_FAIL error=...`. |
| `ports=0` | Le controleur demarre, aucun port ne voit de connexion. Le peripherique est peut-etre derriere un concentrateur. |
| `usb=N keyboards=0` | Des peripheriques repondent, aucun n'est un clavier d'amorcage. Un clavier « gaming » en mode rapport seul tombe ici. |
| `BOUCHAUD_USB_CONCENTRATEUR` | Un concentrateur est branche. Lire la ligne `_TRAVERSE` qui suit : `occupes=0` veut dire qu'il est vide, son absence veut dire que la traversee a echoue. |
| `BOUCHAUD_USB_CONCENTRATEUR_ECHEC` | La descente a echoue. Ce qui est derriere n'existe pas pour le noyau. Brancher le clavier sur une prise de la machine. |
| `BOUCHAUD_USB_CONCENTRATEUR_TROP_PROFOND` | Plus de cinq concentrateurs en cascade. La chaine de route xHCI ne va pas plus loin ; il faut rapprocher le peripherique. |
| `BOUCHAUD_USB_ARBRE_TRONQUE` | Plus de 32 peripheriques en attente d'enumeration. Les suivants sont ignores. |
| `BOUCHAUD_TRIGKEY_REPLI_PS2` | Le controleur existe, aucun clavier n'en est sorti, on a essaye le 8042. |

---

## Installer sur cette machine

1. Construire l'image et l'ecrire sur une cle (`tools/reference/`).
2. Demarrer dessus. Le systeme tourne alors en LIVE, en RAM.
3. Ouvrir un terminal et taper `installer` — il **regarde** et n'ecrit rien.
4. `installer --go` pose la table GPT, l'ESP FAT32 et la partition systeme sur
   le NVMe interne. `--ecrase` est necessaire si le disque porte deja autre
   chose ; sans lui, l'installation est refusee avec une phrase.
5. Retirer la cle et redemarrer.

L'archive que l'installateur copie est **celle qui tourne**, octet pour octet.
Le systeme installe est donc exactement celui qu'on vient d'essayer.
