# Protocole GUI userland - jalon 2

Le navigateur n'est plus un programme qui prend l'ecran : c'est une fenetre du bureau.

    ecran physique
         ^
         | (seul ecrivain)
    Bouchaud WM (fil noyau, compositeur)
         |
         +-- Terminal, Fichiers, ...  (fenetres natives)
         |
         +-- Bouchaud Browser         (client ring 3)
                  |
             surface partagee (memfd, MAP_SHARED)
                  |
             Qt / Python / renderer

## 1. La regle

**Le gestionnaire de fenetres est seul proprietaire du framebuffer physique.**

Elle n'est pas tenue par la bonne volonte du client mais par le noyau :

| Ce que le client demande | Ce qu'il obtient |
|---|---|
| `open("/dev/fb0")` + `mmap` | la surface de sa fenetre (`Process::ecran`) |
| `FBIOGET_VSCREENINFO` | la geometrie de sa fenetre, pas celle de la dalle |
| `smem_start` de `FBIOGET_FSCREENINFO` | `0` - l'adresse physique ne lui est pas donnee |
| `open("/dev/input/event0")` | `EACCES` |

Un client qui ignore completement le protocole ne peut donc ni ecrire un pixel a
l'ecran, ni voler une touche. C'est ce qui rend le compositeur sûr sans avoir a
faire confiance a Qt.

## 2. Ce qui a change dans le noyau

- `task::Task` sait porter un **fil noyau** (`Task::new_kernel`, `task::run_noyau`).
  Le bureau en est un : il est ordonnance comme les autres taches, donc il
  continue a vivre pendant qu'un client ring 3 tourne. Auparavant `exec` etait
  synchrone et le bureau disparaissait pour toute la duree du navigateur.
- `exec::lance_detache` charge un ELF et rend son pid **sans attendre sa fin**.
- `Process::ecran` (`EcranVirtuel`) redirige `/dev/fb0` vers un nœud partage.
- `gui::surface::Surface` alloue les pages d'un `memfd` et en garde les frames,
  ce qui donne au compositeur une lecture directe sans copie intermediaire.

Un fil noyau **n'est jamais preempte** : l'IRQ0 ne commute que depuis le ring 3.
Une composition ne peut donc pas etre coupee en son milieu, et c'est ce qui rend
un tampon simple suffisant pour ce jalon. Le champ `tampon` du protocole existe
pour le jour ou ce ne sera plus vrai (preemption noyau, plusieurs cœurs).

## 3. Transport

Une paire de canaux, exactement ce qu'un `socketpair` donne a deux processus. Le
gestionnaire de fenetres etant dans le noyau, il tient ses deux extremites
directement ; le client recoit un descripteur ordinaire, dont le numero est dans
`BO_GUI_FD`.

Variables d'environnement posees par le gestionnaire de fenetres :

| Variable | Sens |
|---|---|
| `BO_GUI_FD` | descripteur du canal de protocole |
| `BO_SURFACE_FD` | descripteur de la surface (`mmap(MAP_SHARED)`) |
| `BO_SURFACE_WIDTH` / `HEIGHT` / `STRIDE` | geometrie de la surface |
| `QT_QPA_PLATFORM_PLUGIN_ARGS` | taille d'ecran = taille de la fenetre |
| `QT_QPA_FB_DISABLE_INPUT` | `1` - ceinture, le noyau refuse deja evdev |

## 4. Format de fil

Tout est en petit-boutiste explicite. En-tete de 16 octets devant chaque charge :

| Decalage | Taille | Champ |
|---|---|---|
| 0 | 4 | `magic` = `0x55474F42` ("BOGU") |
| 4 | 2 | `version` = 1 |
| 6 | 2 | `genre` |
| 8 | 4 | `taille_charge` (<= 4096) |
| 12 | 4 | `serie` |

### Client -> gestionnaire de fenetres

| Genre | Valeur | Charge |
|---|---|---|
| `Hello` | 1 | `version:u32`, `pid:u32` |
| `CreateWindow` | 2 | `fenetre:u32`, `largeur:u32`, `hauteur:u32`, `drapeaux:u32` |
| `SetTitle` | 3 | UTF-8 (96 caracteres au plus retenus) |
| `Damage` | 4 | `fenetre:u32`, `Rect` |
| `Close` | 5 | `fenetre:u32` |
| `FrameReady` | 6 | `fenetre:u32`, `tampon:u32`, `Rect` |
| `PressePapiersEcrit` | 7 | les octets du nouveau contenu |

### Gestionnaire de fenetres -> client

| Genre | Valeur | Charge |
|---|---|---|
| `Surface` | 0x100 | `fenetre`, `tampon`, `largeur`, `hauteur`, `pas`, `format` |
| `Configure` | 0x101 | `fenetre`, `largeur`, `hauteur`, `focus` |
| `Focus` | 0x102 | reserve |
| `Key` | 0x103 | `fenetre`, `code`, `modificateurs`, `unicode`, `appui` |

`appui` vaut 1 pour un enfoncement, 0 pour un relachement. Les deux sont
envoyes : un client qui ne veut que les frappes filtre lui-meme.

`modificateurs` est un masque : `1` Shift, `2` Ctrl, `4` Alt, `8` AltGr. Les
valeurs sont nommees dans les trois implementations
(`window_manager::modificateur`, `enum Modificateur` cote hote Qt et cote chrome
Ladybird) et `tools/verifie-protocole-gui.py` refuse un desaccord.
| `Pointer` | 0x104 | `fenetre`, `x:i32`, `y:i32`, `boutons` |
| `Wheel` | 0x105 | `fenetre`, `delta:i32`, `x:i32`, `y:i32` (coordonnées client) |
| `CloseRequest` | 0x106 | `fenetre:u32` |
| `PressePapiers` | 0x107 | les octets du contenu courant |

`Rect` fait 16 octets : `x:i32`, `y:i32`, `largeur:u32`, `hauteur:u32`, exprime
dans le repere de la **surface** (origine en haut a gauche de la zone utile).

### Le presse-papiers

Deux messages, et l'asymetrie entre les deux est le fond de la conception.

`PressePapiersEcrit` va du client au bureau. Il n'est accepte **que du client
qui a le foyer** : un programme en arriere-plan qui pourrait ecrire
remplacerait silencieusement ce que l'utilisateur vient de copier -- l'adresse
d'un virement, par exemple, par une autre --, et rien a l'ecran ne le
montrerait. Un refus est journalise, pas repondu : le client n'a rien a
apprendre d'un droit qu'il n'a pas.

`PressePapiers` va du bureau au client. Il est **pousse**, jamais demande : il
n'existe aucun message de lecture dans ce protocole. C'est la faiblesse
historique de X11 que cette absence ferme -- la ou n'importe quel client peut y
interroger la selection a tout moment, donc recolter en arriere-plan tout ce
que l'utilisateur copie (un mot de passe sorti d'un gestionnaire, une phrase de
recuperation, un jeton), ici un client sans foyer ne recoit rien et n'a aucun
moyen d'en obtenir. Il n'y a pas de chemin de lecture a garder, parce qu'il n'y
en a pas.

Le bureau ne pousse que ce qui a CHANGE pour ce client-la : le contenu porte un
numero de generation, et chaque client se souvient de celui qu'il possede.
Comparer deux entiers a chaque tour de composition et par client est gratuit ;
recopier quatre kibioctets ne l'est pas.

Le contenu est borne a `CHARGE_MAX` (4096 octets) : il voyage dans **un**
message, et ce qui ne tient pas dans un message ne pourrait pas etre remis.
`gui::presse_papiers` tronque au-dela plutot que de refuser -- c'est la defense
en profondeur, celle qui tient encore le jour ou les deux bornes divergent.
`tools/gui/test_presse_papiers.rs` exerce cette borne sur l'hote.

### Codes de touche

Ce ne sont pas des codes evdev. Le pilote clavier du bureau ne produit pas de
scancode brut mais une touche deja interpretee selon la disposition ; envoyer un
faux code Linux serait pire qu'un code a nous, parce que le client le croirait.

| Code | Sens |
|---|---|
| 0 | caractere - le point de code est dans `unicode` |
| 1 | Entree |
| 2 | Retour arriere |
| 3 | Tabulation |
| 4-7 | Haut, Bas, Gauche, Droite |
| 8 | Echap |
| 9-10 | Origine, Fin |
| 11-12 | Page precedente, Page suivante |
| 13-14 | Suppr, Inser |
| 15 | Touche de fonction - le NUMERO (1 a 12) est dans `unicode` |

Les codes 9 a 15 sont arrives avec le navigateur. Ils manquaient parce que le
decodeur clavier ne reconnaissait, parmi les sequences etendues, que les quatre
fleches : le pave de navigation etait perdu entre le controleur PS/2 et le
client. Sans consequence visible sur le bureau -- un octet inconnu ne produit
rien, et rien est ce qu'on attend d'un octet inconnu -- mais une page ne se
faisait alors defiler qu'a la molette. Suppr etait pire que perdue : elle
arrivait comme Retour arriere, et effacait donc le caractere de gauche.

Une touche de fonction par code aurait demande douze lignes a chaque
implementation, et douze occasions de se tromper : le numero voyage donc dans
`unicode`, comme le point de code d'un caractere.

Echap va au client quand celui-ci a le focus : un navigateur en a besoin, et le
lui confisquer pour fermer sa fenetre detruirait le travail en cours. Sans client
au premier plan, Echap garde son role d'avant (fermer le menu, puis la fenetre du
dessus, puis le bureau).

## 5. Contre-pression

Le canal a une capacite de 64 KiB, comme un `socketpair` du noyau.

- Cote gestionnaire de fenetres : ce qui ne tient pas est **abandonne**, jamais
  attendu. Les mouvements de souris sont en plus fusionnes - si le dernier
  message du canal est deja un `Pointer` non lu, sa charge est remplacee sur
  place au lieu d'en empiler un second. Un deplacement rapide ne remplit donc pas
  64 KiB de positions perimees.
- Cote client : un message est ecrit d'un bloc, en-tete et charge ensemble. Deux
  ecritures pourraient etre separees par un `EAGAIN` et laisser le lecteur devant
  un en-tete sans corps - c'est exactement le defaut de cadrage corrige sur le
  canal du renderer.
- Des deux cotes, les receptions partielles sont conservees telles quelles : on
  n'analyse jamais un message avant de l'avoir en entier.

## 6. Degats

`FrameReady` porte un rectangle, et il est rogne a la surface avant tout usage :
un `Damage { x: -1 }` ou un `largeur: u32::MAX` ferait autrement lire le
compositeur hors du tampon, depuis le noyau.

Ce que le jalon 2 en fait est volontairement modeste : le rectangle decide **s'il
faut recomposer**, pas quelle portion recopier. Le bureau redessine encore tout a
chaque trame, la zone utile est donc recopiee en entier. Recomposer partiellement
demande de savoir quels pixels du bureau sont encore valides - c'est le chantier
des regions sales du compositeur lui-meme, et il n'a pas a retarder celui-ci.

Le format, lui, est deja le bon : le jour ou le moteur saura dire quelles regions
il a refaites, seuls `hote.cpp` et le compositeur changent.

## 7. Cadence

Le bureau ne redessine que sur evenement, et au plus une fois par 16 ms.

Il redessinait autrefois a chaque tour de boucle - c'est-a-dire aussi vite que le
processeur le permettait, le PIT battant a 1 kHz. Tant que rien d'autre ne
tournait, cela ne se voyait pas. Face a un client qui a besoin du meme processeur,
c'est une famine.

Sont considerees comme des evenements : les entrees, les changements de fenetre,
les trames des clients, et l'ecoulement d'une seconde (l'horloge et les
indicateurs systeme changent seuls). Sans rien de tout cela, la boucle dort et
s'arrete sur un `hlt` comme avant.

## 8. Suivre ce qui se passe

Chaque ligne du journal serie porte l'heure et la charge de la machine :

    [18:51:48][ 12%:  5%:  6%] gui: client /bo-navigateur pid=5 ...
     \______/  \_/  \_/  \_/
      heure    cpu  ram  RAMFS

La memoire est celle des **frames physiques**, pas du tas : un navigateur projette
des centaines de mebioctets de Qt et de Python par `mmap`, qui ne passent jamais
par le tas. `journal off` coupe les couleurs ANSI.

Toutes les cinq secondes, deux lignes de plus disent qui consomme :

    [ps] bo-navigateur pid=5 cpu 18% rss 74 Mio (6 fils) | desktop pid=4 cpu 3% rss 0 Mio
    [gui] client pid=5 actif 61 trames (12/s, silence 40 ms) recu 1464 o, envoye 210 ev (0 perdus)

Le pourcentage processeur vient d'un profileur par echantillonnage : a chaque
IRQ0 — mille fois par seconde — la tache courante gagne un tick. Le denominateur
est le nombre de ticks reellement distribues sur la periode, pas la duree
ecoulee ; une machine qui dort ne fabrique donc pas des parts qui depassent cent.

Cote client, l'hote Qt tient les memes comptes et les journalise toutes les cinq
secondes :

    [bo] battement : 312 tics, 61 trames, 210 evenements recus

Les deux relevés se lisent ensemble : si le noyau compte des trames que l'hote
n'a pas envoyees, ou l'inverse, on sait de quel cote du fil regarder.

## 9. Ce qui arrive quand le client ne parle pas le protocole

L'image userland est construite par la CI et peut preceder le noyau qu'on
demarre. Un binaire d'avant le protocole ouvre `/dev/fb0` — c'est-a-dire la
surface — et y peint sans rien annoncer.

Au bout de six secondes sans `Hello`, le compositeur cesse de l'attendre : il
compose la surface au rythme fixe du bureau et le journalise. C'est moins
efficace, et c'est la difference entre un navigateur qui s'affiche et une fenetre
qui reste sur son ecran de demarrage.

## 10. Verifications

    tools/gui/test-protocole.sh

- tests d'unite de `src/gui/protocole.rs` compiles pour l'hote (`rustc --test`) :
  rognage des degats, union, coordonnees ecran -> fenetre, cadrage du flux,
  rejet d'un flux etranger ou d'une charge demesuree ;
- `tools/verifie-protocole-gui.py` : les valeurs numeriques du protocole sont
  les memes dans `src/gui/protocole.rs` et dans `hote.cpp`.

Les deux sont bloquantes en CI (`kernel-build`) et ne demandent ni QEMU, ni Qt,
ni le reseau.

## 11. Ce que ce jalon ne fait pas

- **Pas de redimensionnement cote hote Qt.** La surface est allouee une fois, a
  la plus grande zone utile possible, et Qt dimensionne son ecran dessus au
  demarrage : il ne suit pas `Configure`. Le chrome Ladybird, lui, le suit --
  il adopte la nouvelle taille, recompose et transmet le viewport au moteur --,
  donc le bouton maximiser agit sur la fenetre du navigateur et pas sur celle
  de l'hote Qt.
- **Pas de double tampon.** Voir la section 2 : inutile tant que le compositeur
  ne peut pas etre preempte.
- **Pas de repetition annoncee.** Le pilote distingue une touche maintenue
  d'une nouvelle frappe (`KeyEvent::repeat`), mais le message `Key` ne porte pas
  encore ce bit : le client recoit la repetition comme un appui de plus.
- **Pas de recomposition partielle de l'ecran.** Voir la section 6.
- **Une seule instance.** Deux navigateurs, ce sont deux Qt qui demarrent en
  meme temps sur un cœur unique.
