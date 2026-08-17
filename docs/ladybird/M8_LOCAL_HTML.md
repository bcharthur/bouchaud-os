# Ladybird M8 — HTML local dans une fenetre Bouchaud

M8 ferme la boucle entre le vrai `WebContent` Ladybird et le gestionnaire de
fenetres Bouchaud. Le critere reste celui du plan directeur : **un document HTML
local est rendu par LibWeb puis visible dans une vraie fenetre Bouchaud, avec une
capture comparee**.

## Chemin execute

```text
Bouchaud WM
  -> cree Surface partagee XRGB8888 + canal GUI
  -> lance /bo-navigateur
       -> webcontent-bootstrap
            -> fork/exec WebContent natif Ladybird
                 -> PageHost / PageClient
                 -> parse le HTML local dans le document initial avec HTMLParser
                 -> layout + paint CPU LibWeb/LibGfx
                 -> screenshot BGRA8888
                 -> invariant de contenu independant
                 -> normalisation XRGB8888
                 -> copie dans BO_SURFACE_FD
                 -> Hello + SetTitle + FrameReady sur BO_GUI_FD
  -> compose la Surface dans la fenetre
```

Le bootstrap de M7 est conserve. M8 n'est active que lorsque
`BOUCHAUD_M8=1` est present dans l'environnement. Le bureau normal et le test M7
ne changent donc pas de comportement.

## Pourquoi M8 parse directement le document

`Page::load_html()` n'est pas un simple parseur : Ladybird l'implemente comme une
vraie navigation `about:srcdoc`. Cette navigation demande au processus Browser de
coordonner une operation d'historique de session. Or M8 ne lance volontairement
pas encore le Browser Ladybird complet ; son bootstrap ne fournit que le canal
minimal necessaire a `WebContent`.

Pour garder la frontiere du jalon nette, M8 utilise donc le vrai `HTMLParser` de
LibWeb avec le scripting desactive, sur le document initial deja cree par
`PageHost`. La capture est ensuite traitee directement dans `WebContent`. La
navigation, l'historique de session et leur coordination Browser/WebContent
restent un chantier ulterieur au lieu d'etre simules dans le bootstrap M8.

## Pourquoi la capture ne depend pas encore du Compositor Ladybird

Le jalon M8 porte sur **LibWeb -> pixels -> fenetre Bouchaud**. Le chemin de
screenshot de Ladybird sait deja construire un `Gfx::Bitmap` CPU a partir du
document et de son arbre de peinture. On reutilise ce chemin au lieu d'introduire
le processus `Compositor` avant d'en avoir besoin.

Le processus `Compositor` Ladybird, RequestServer et ImageDecoder restent des
jalons ulterieurs. M8 n'implemente donc ni HTTP, ni HTTPS, ni images distantes,
ni GPU.

## Capture comparee

La validation se fait en deux etapes independantes.

D'abord, la capture Ladybird doit verifier un invariant de contenu. La zone utile
doit faire au moins 320x200 pixels, contenir au moins 256 pixels differents du
pixel de reference du coin superieur gauche et presenter une plage de luminance
d'au moins 48. Ces seuils sont volontairement larges : ils tolerent des variations
de police et d'antialiasing, mais empechent une capture blanche, noire ou uniforme
de faire passer M8 simplement parce qu'elle a ete copiee sans erreur.

Ensuite, la capture Ladybird produite en BGRA8888 est normalisee vers le
XRGB8888 attendu par Bouchaud. Pour chaque pixel, M8 conserve les composantes
B/G/R et force l'octet haut a zero. Un FNV-1a 64 bits est calcule sur les pixels
normalises avant et apres la copie dans la surface partagee. Cette empreinte ne
valide que l'integrite du transfert ; le rendu lui-meme est deja controle par
l'invariant precedent.

Le test QEMU exige ensuite que le gestionnaire de fenetres ait effectivement
recu le protocole et compose une premiere trame. Les marqueurs principaux sont :

```text
[ladybird-bouchaud] WEBCONTENT_READY
[ladybird-bouchaud] M8_FONT_READY
[ladybird-bouchaud] M8_STAGE initialize ok
[ladybird-bouchaud] M8_BOOTSTRAP
[ladybird-bouchaud] M8_STAGE parse begin
[ladybird-bouchaud] M8_STAGE parse ok
[ladybird-bouchaud] M8_LOCAL_HTML_RENDERED
[ladybird-bouchaud] M8_CONTENT_INVARIANT
[ladybird-bouchaud] M8_CAPTURE_MATCH
... parle le protocole v1
... premiere trame du client ...
[ladybird-bouchaud] RESULTAT : M8 HTML local dans fenetre Bouchaud OK
```

## Autostart de CI

Le scenario exporte `BO_AUTOSTART_BROWSER=1` avant de lancer `desktop`. La toute
premiere fenetre, historiquement un terminal, devient alors `/bo-navigateur`.
Cette exception est limitee a la premiere fenetre et a la presence explicite de
la variable ; le bureau interactif normal reste inchange.

## Hors perimetre

M8 utilise volontairement un HTML sans feuille CSS applicative ni JavaScript.
M9 valide le CSS local, M10 le JavaScript dans la page, M11 branche
RequestServer/HTTP, puis M12 HTTPS et Internet.
