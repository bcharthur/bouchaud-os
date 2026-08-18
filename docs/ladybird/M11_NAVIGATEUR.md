# M11 — le navigateur devient utilisable

## Ce que M11 change, et pourquoi c'etait le jalon bloquant

M9 affiche une page distante dans une fenetre Bouchaud. M12 lui donne HTTPS.
Les deux sont des preuves, et aucune des deux ne fait un navigateur : l'URL est
une variable d'environnement, la page est capturee une fois, et rien de ce que
fait l'utilisateur n'atteint le document. On peut regarder une page ; on ne peut
pas **naviguer**.

M11 ajoute exactement les quatre choses qui manquent :

| Manquait | M11 |
|---|---|
| Choisir une page | barre d'adresse : cliquer, taper, Entree |
| Revenir en arriere | boutons reculer / avancer / recharger |
| Suivre un lien | le clic atteint le document, la navigation reste locale |
| Voir la suite | molette, fleches, entree clavier vers la page |

Le critere du plan directeur pour M11 — « barre d'adresse, historique, liens,
defilement » — est donc tenu par ce jalon, et par lui seul.

## Ce que M11 n'introduit pas

**Pas de processus Browser.** Upstream met le chrome dans un processus separe
qui parle a WebContent par LibIPC. C'est la bonne architecture, et c'est M13 :
elle n'a de sens qu'avec plusieurs onglets, donc plusieurs WebContent. L'ajouter
ici pour un seul onglet couterait un processus, un protocole et une classe de
pannes, pour rien de visible.

**Pas de nouvelle dependance de construction.** Le chrome est un en-tete unique,
`tools/ladybird/chrome/BouchaudChrome.h`, copie dans l'arbre jetable par
`tools/ladybird/prepare-m11-chrome.py`. `Services/WebContent/CMakeLists.txt`
n'est pas touche.

**Pas de police a charger.** La barre d'outils dessine avec la police bitmap 8x8
du noyau (`src/drivers/gfx/font.rs`, « font8x8 basic », domaine public), recopiee
dans l'en-tete. Le chrome ne depend donc d'aucune API de dessin d'upstream, qui
sont precisement celles qui bougent le plus.

## Le chemin execute

```text
Bouchaud WM (fil noyau, compositeur)
  |  Key / Pointer / Wheel / Configure / CloseRequest        protocole GUI v1
  v
BouchaudChrome::drain()          lit le canal, cadre les messages
  |
  +-- clic dans la barre  --> historique, rechargement, foyer de la saisie
  +-- touche dans la barre --> edition, Entree -> load_url()
  +-- clic dans la page   --> Web::MouseEvent  --> ConnectionFromClient::mouse_event
  +-- touche dans la page --> Web::KeyEvent    --> ConnectionFromClient::key_event
  +-- molette             --> MouseWheel
  |
  v
file d'entree normale de WebContent -> LibWeb -> mise en page -> peinture
  |
  v
PageClient::page_did_take_screenshot -> BouchaudChrome::present()
  |
  +-- barre d'outils peinte dans la surface partagee
  +-- capture de page copiee sous la barre
  +-- FrameReady sur BO_GUI_FD
```

Les entrees ne prennent aucun raccourci : elles entrent par
`ConnectionFromClient::mouse_event` / `key_event`, c'est-a-dire par la meme porte
que le processus Browser d'upstream. La coalescence des mouvements, la file, la
boucle d'evenements de LibWeb — tout cela est celui d'upstream, non reecrit.

## Geometrie

La barre fait 36 pixels. Le viewport de la page vaut donc
`hauteur_surface - 36`, et un clic a l'ordonnee `y` de la surface arrive a
`y - 36` dans la page. La conversion vit a **un seul endroit**,
`BouchaudChrome::page_origin_y()`, utilisee par la composition comme par le
routage des entrees : c'est ce qui garantit qu'un clic tombe la ou le pixel a
ete peint. Deux constantes auraient fini par diverger d'un pixel, et un lien sur
deux aurait manque.

## Le rythme des trames, et pourquoi il n'est pas immediat

Une entree ne peut pas etre peinte dans la foulee. `mouse_event` **met en file**
l'evenement dans WebContent ; une capture demandee au meme instant
photographierait l'etat d'avant. Le chrome demande donc quelques trames sur les
tics suivants d'un minuteur a 16 ms — la cadence du bureau
(`docs/GUI_USERLAND_PROTOCOL.md` §7). Le compteur retombe a zero des que
l'utilisateur s'arrete : **au repos, rien n'est repeint**, et le moteur garde le
processeur.

Pendant un chargement, une trame est demandee a chaque tic : c'est la difference
entre une page qui apparait progressivement et une page qui surgit d'un coup au
bout de trois secondes.

## Le canal est lu deux fois, exprès

Le canal GUI est lu par un `Core::Notifier` **et** par le minuteur. C'est
redondant et c'est voulu : la sonde M9 (`M9_BODY_DRAIN_BEGIN`) a etabli qu'un
notificateur de lecture pouvait rester endormi sous Bouchaud alors que la donnee
etait deja arrivee. Un navigateur dont la souris s'arrete par intermittence est
inutilisable. Le notificateur est le chemin rapide, le minuteur est la garantie.

## Trois implementations d'un seul protocole

Le format de fil existe maintenant trois fois : `src/gui/protocole.rs` (noyau),
`tools/userland/navigateur/hote.cpp` (client Qt) et
`tools/ladybird/chrome/BouchaudChrome.h` (ce jalon). Rien dans la construction
ne les relie.

`tools/verifie-protocole-gui.py` compare desormais les **trois**, chaque client
contre le noyau — jamais les clients entre eux, sinon deux clients pourraient
s'accorder sur une valeur fausse. Le controle est bloquant en CI et ne demande
ni QEMU, ni Qt, ni le reseau.

## Ce que la barre d'adresse accepte

| Saisie | Resultat |
|---|---|
| `https://…`, `http://…`, `about:…`, `data:…`, `file://…` | tel quel |
| une chaine sans espace contenant un point | prefixee `https://` |
| tout le reste | envoye au moteur de recherche |

Le moteur de recherche se change par `BOUCHAUD_SEARCH_URL` ; par defaut
`https://duckduckgo.com/?q=`. Completer « example.com » en URL n'est pas de la
complaisance : personne ne tape le schema, et refuser l'entree serait exact et
inutile.

## Clavier

Le gestionnaire de fenetres n'envoie que des **appuis** : il n'a pas de source
de relachements a transmettre. Le chrome emet donc le `keyup` juste apres le
`keydown`, sans quoi une page qui compte les deux resterait persuadee qu'une
touche est enfoncee.

| Touche | Barre d'adresse au foyer | Page au foyer |
|---|---|---|
| caractere | insere | envoye au document |
| Entree | navigue | envoye au document |
| Retour arriere | efface | envoye au document |
| Gauche / Droite | deplace le curseur | envoye au document |
| Haut / Bas | ignore | defilement / document |
| Echap | rend le foyer, restaure l'URL | arrete le chargement |

Echap arrive au client parce que le gestionnaire de fenetres le lui laisse
quand il a le focus (`docs/GUI_USERLAND_PROTOCOL.md` §4) : la croix de la barre
de titre reste le moyen de fermer la fenetre.

## Lancer

```powershell
.\run.ps1 -Ladybird                                  # https://example.com/
.\run.ps1 -Ladybird -LadybirdUrl "https://fr.wikipedia.org/"
.\run.ps1 -Ladybird -LadybirdSansChrome              # revenir au comportement M9
```

`-LadybirdSansChrome` retire la barre et revient a la capture unique de M9. Il
existe pour trancher une regression sans discussion : est-ce la page, ou est-ce
la barre ?

Le magasin d'autorites necessaire a HTTPS est fabrique automatiquement s'il
manque — voir `tools/ladybird/certs/README.md`.

## Marqueurs du journal

```text
[ladybird-bouchaud] M11_CHROME gui_fd=… surface_fd=… surface=1100x604 toolbar=36
[ladybird-bouchaud] M11_READY toolbar=36 viewport_height=568
[ladybird-bouchaud] M11_GUI_HANDSHAKE_OK
[ladybird-bouchaud] M11_FIRST_FRAME pixels=… viewport=1100x568
[ladybird-bouchaud] M11_DOCUMENT_LOADED page=1 url=https://example.com/
[ladybird-bouchaud] M11_NAVIGATE url=…            (barre d'adresse)
[ladybird-bouchaud] M11_HISTORY delta=-1          (bouton reculer)
```

`M11_DOCUMENT_SKIPPED url=about:blank` est normal : le document vide initial se
termine pendant que la vraie navigation est encore en vol, et l'afficher ferait
clignoter une URL que personne n'a demandee.

## Ce que M11 ne fait toujours pas

- **Pas de redimensionnement.** La surface est allouee une fois
  (`docs/GUI_USERLAND_PROTOCOL.md` §11) ; `Configure` est journalise, pas suivi.
- **Pas d'onglets** — M13.
- **Pas d'ecran d'avertissement de certificat.** Une chaine invalide fait
  echouer la requete, et le statut affiche l'echec ; il n'y a pas encore
  d'interface pour porter une decision de l'utilisateur.
- **Pas de modificateurs clavier.** Le pilote du bureau n'expose pas encore
  Ctrl/Alt separement, donc pas de Ctrl+L ni de Ctrl+R : la barre se prend a la
  souris.
- **Pas de menu contextuel, pas de telechargements, pas de favoris.**
