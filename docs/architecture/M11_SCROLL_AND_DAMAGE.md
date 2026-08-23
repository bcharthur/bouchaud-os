# M11 : wheel, readback Compositor et dommages partiels

## Chemin wheel réel

Le bridge M11 construit un `Web::MouseEvent::Type::MouseWheel`, puis son callback
appelle `ConnectionFromClient::mouse_event(page_id, event)`. C'est le point
d'entrée WebContent upstream : l'événement continue dans `Page`/l'event handler,
ce qui conserve le hit-test DOM, les éléments `overflow`, iframes, listeners
`wheel`, `preventDefault()` et le scrolling asynchrone. Le chrome ne modifie
jamais `scrollY` et n'appelle pas `scrollBy()`.

Le défaut de présentation était après ce traitement. M11 n'affiche pas
 directement la surface native du Compositor : il présente un readback obtenu
par la file de screenshots. L'enfilage automatique de cette capture n'avait
lieu que pendant une étape de rendu WebContent. Un scroll asynchrone peut ne
modifier que l'état du Compositor et ne crée donc aucune invalidation WebContent,
aucun screenshot M11 et aucun `FrameReady`.

Lorsque `PageClient::report_finished_handling_input_event()` confirme désormais
la fin du vrai pipeline input, le port programme un readback single-flight du
Compositor. Il ne force aucun offset et ne recopie pas l'ancienne page. Les
marqueurs bornés sont :

```text
WEB_WHEEL_DISPATCH
WEB_WHEEL_CALLBACK queued=1
WEB_WHEEL_HANDLED result=N capture=scheduled
WEB_SCREENSHOT_REQUEST after_wheel=1
WEB_SCREENSHOT_READY after_wheel=1 valid=N
M11_FRAME_AFTER_SCROLL
```

La fixture `file:///usr/share/bouchaud/scroll-test.html` permet de distinguer le
document et l'élément interne `overflow-y:scroll`. La preuve dynamique de leur
comportement reste un test QEMU; la preuve statique est qu'ils passent tous deux
par le même input WebContent et non par une manipulation du document depuis le
chrome.

## Chrome partiel

Une capture page utilise `compose_full()`: elle remplace `last_page`, dessine la
barre, copie la page puis publie le dommage de toute la surface. Une modification
de barre d'adresse utilise `compose_toolbar_only()`: elle mappe la surface déjà
cohérente, ne touche qu'aux lignes `0..toolbar_height`, puis publie
`(0, 0, surface_width, toolbar_height)`.

Pour une surface 1100x604 et une toolbar de 36 px, une frappe écrit désormais
39 600 pixels de chrome. Elle évite 624 800 écritures de fond de viewport et,
avec une capture pleine valide, jusqu'à 624 800 écritures de copie page. Le
dommage publié passe de 664 400 à 39 600 pixels. Le coalescing existant
`chrome_frames_pending = 1` est conservé.

`M11_RENDER_STATS full=N toolbar=N page=N pixels=N` est émis toutes les seize
trames publiées; aucun log n'est ajouté dans le chemin pixel chaud.
