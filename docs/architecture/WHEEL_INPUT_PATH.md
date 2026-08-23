# Chemin molette

Le paquet PS/2 IntelliMouse à quatre octets est décodé dans `drivers::mouse`.
Un delta non nul produit `[INPUT-WHEEL]`. Le WM consomme l'accumulateur, choisit
la fenêtre supérieure sous le pointeur, convertit écran→client et produit
`[GUI-WHEEL-TX]` ou un rejet explicite.

Le défaut localisé était dans le protocole GUI: `Wheel` ne transportait que le
delta. BrowserHost réutilisait `State.last_x/last_y`, mis à jour uniquement par
un ancien message Pointer. Le hit-test toolbar/viewport pouvait donc utiliser
des coordonnées périmées, particulièrement quand la fenêtre ou le pointeur
avait changé sans nouveau Pointer visible du client.

`Wheel` transporte désormais delta, x et y client. Le pont M11 journalise la
réception, le rejet toolbar ou le dispatch viewport, conserve la conversion de
signe PS/2(+haut)→Web(+bas), puis marque la première FrameReady suivant le
dispatch. Ces marqueurs prouvent le transport/repaint; seul un test runtime peut
confirmer quel élément DOM (document, modal ou overflow interne) consomme le
WheelEvent.

La fixture `file:///usr/share/bouchaud/scroll-test.html` contient un document de
4000 px et un élément `overflow-y:scroll` indépendant.
