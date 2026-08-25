# Chemin molette

## Les quatre conventions de signe

Le defilement traverse quatre conventions de signe, et rien dans le typage ne
les distingue — ce sont tous des `i32`.

| Etage | Positif signifie | Source |
|---|---|---|
| Paquet PS/2 IntelliMouse (4e octet) | vers le **bas** | QEMU `hw/input/ps2.c` (`WHEEL_UP` fait `mouse_dz--`) ; Linux `psmouse-base.c` publie `REL_WHEEL = -(signed char) packet[3]` |
| evdev `REL_WHEEL` | vers le **haut** | convention Linux ; `kernel::input::read_mouse` fait la negation |
| Protocole GUI (`Wheel.delta`) | vers le **haut** | convention Qt, `gui::protocole::Molette` |
| `WheelEvent.deltaY` du DOM | vers le **bas** | le pont M11 fait `-delta` |

Le bureau mettait sur le fil l'octet PS/2 brut, sans conversion. Tout
defilement du systeme partait donc a l'envers — navigateur, explorateur de
fichiers et Rustpad, les trois consommateurs lisant `positif = haut`
(`scroll - delta` pour les deux applications noyau, `-delta` pour M11).

Sur une page en haut de course — l'etat de toute page qui vient de charger —
le geste « vers le bas » demandait de remonter. Le delta arrivait bien jusqu'a
LibWeb, `scroll_viewport_by_delta` etait bien appele, et l'offset etait ramene
a zero par bornage : rien ne bougeait, et rien ne le disait.

La conversion vit maintenant en un seul endroit, `gui::mouse::take_wheel`, a
la frontiere du bureau. `gui::protocole::molette_depuis_ps2` la porte et un
test d'unite fixe le sens dans les deux directions.
`kernel::input` garde le brut du pilote : sa convention evdev est encore une
troisieme, et il fait deja la sienne.

## Coordonnees

`Wheel` transporte delta, x et y client. Le defaut precedent etait que seul le
delta voyageait : BrowserHost reutilisait `State.last_x/last_y`, mis a jour
uniquement par un ancien message `Pointer`, et le hit-test toolbar/viewport
pouvait donc travailler sur des coordonnees perimees.

## Sondes, etage par etage

```text
[INPUT-WHEEL] raw=…              drivers::mouse, un paquet a 4 octets non nul
[GUI-WHEEL-TX] … transmis=1      WM : fenetre trouvee, cran remis au canal
[GUI-WHEEL-TX] … transmis=0      WM : canal plein ou sans lecteur, cran perdu
[GUI-WHEEL-DROP] reason=no-window       aucune fenetre sous le pointeur
[GUI-WHEEL-DROP] reason=outside-client  pointeur sur le cadre ou la barre de titre
[GUI-WHEEL-APP] fenetre=…        consomme par une application du noyau
M11_WHEEL_RX                     le chrome a lu le message
WEB_WHEEL_DROP reason=toolbar    au-dessus de la barre d'outils
WEB_WHEEL_DISPATCH               converti en viewport, signe DOM applique
WEB_WHEEL_CALLBACK queued=1      mis dans la file d'entree de WebContent
WEB_WHEEL_HANDLED result=N       LibWeb a fini de traiter l'evenement
WEB_SCREENSHOT_REQUEST after_wheel=1
WEB_SCREENSHOT_READY after_wheel=1 valid=N
M11_FRAME_AFTER_SCROLL           la premiere trame publiee apres le cran
```

`[GUI-WHEEL-TX]` part **apres** l'envoi et porte son resultat : le canal GUI
est borne et abandonne ce qui ne tient pas. Annoncer la transmission avant de
la tenter faisait dire au bureau une chose qu'il ne savait pas encore, et
« le bureau a envoye » ne se distinguait plus de « le client a recu ».

Les quatre dernieres lignes etaient inatteignables entre `4b868fd` et
`443209d` : la boucle d'evenements de LibWeb sautait
`report_finished_handling_input_event` pour toute entree injectee, or M11 est
le seul producteur d'entree sous BrowserHost. Voir
`tools/ladybird/prepare-m11-input-ownership.py` §2.

## Ce qui reste a prouver a l'execution

Le signe et le transport sont prouves par lecture de source et par test
d'unite. Ne le sont pas :

- que le paquet a quatre octets arrive reellement sous QEMU en SMP4 (aucun
  `[INPUT-WHEEL]` n'a encore ete releve dans un journal) ;
- quel element du DOM consomme le `WheelEvent` sur une page reelle — document,
  modale, ou conteneur `overflow` interne ;
- que la capture reprogrammee apres le cran change bien l'image presentee.

La fixture `file:///usr/share/bouchaud/scroll-test.html` contient un document
de 4000 px et un element `overflow-y:scroll` independant : elle separe les deux
premiers cas.
