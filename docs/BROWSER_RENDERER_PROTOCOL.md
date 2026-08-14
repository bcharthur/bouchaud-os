# Le protocole navigateur ↔ renderer

*Ce document etait une esquisse. Il decrit maintenant un mecanisme qui tourne :
`moteur/protocole.py` porte le cadrage, `moteur/superviseur.py` le cote
navigateur, `moteur/renderer.py` le cote rendu, `moteur/surface.py` la memoire
partagee. Ce qui suit distingue partout ce qui **est** de ce qui **reste**.*

## Ce qui est implemente

| | Etat |
|---|---|
| Cadrage `version / genre / longueur / charge`, gros-boutiste, huit octets | fait |
| Verification de la version, du genre, du **sens**, de la longueur, de la completude | fait |
| `CREATE_DOCUMENT`, `NAVIGATE`, `RESIZE`, `INPUT_EVENT`, `TICK`, `SURFACE`, `CLOSE` | fait |
| `READY`, `TITLE_CHANGED`, `URL_CHANGED`, `CURSOR_CHANGED`, `FRAME_READY`, `CONSOLE_MESSAGE`, `ERROR`, `REQUEST_NAVIGATION` | fait |
| `FOCUS_CHANGED`, `FETCH_REQUEST` / `FETCH_RESPONSE` / `FETCH_DATA`, `AUDIT` / `AUDIT_RESULT` | fait |
| Utilise par le **vrai** navigateur, par defaut | fait — voir `BROWSER_ISOLATION.md` |
| Surface `memfd` + `MAP_SHARED` + `SCM_RIGHTS`, deux tampons, generation | fait |
| `CRASH` synthetise par le navigateur depuis `wait4` | fait |
| `RLIMIT_AS` dans le renderer, annonce en retour dans `READY` | fait |
| Un renderer par origine | **pas fait** — le superviseur ne tient qu'un enfant |
| Encodage binaire de la liste d'affichage | **sans objet** — voir plus bas |

Le protocole porte deja un identifiant de contexte dans chaque message : passer
a plusieurs renderers ne demande pas de le changer.

## Pourquoi l'ecrire avant

Un protocole IPC se decide une fois. Les deux cotes s'y adossent, les messages
se multiplient, et le refaire coute alors bien plus cher que de l'ecrire. Les
questions que la premiere version tranche — qui parle en premier, ce qui passe
par le canal et ce qui passe par la memoire partagee, ce qui arrive quand un
cote meurt — ne sont pas des details d'implementation : ce sont les hypotheses
sur lesquelles tout le reste va s'appuyer.

## Les deux canaux, et pourquoi deux

    controle   socketpair (AF_UNIX, SOCK_STREAM)  — messages courts, ordonnes
    surfaces   memfd + SCM_RIGHTS + mmap(MAP_SHARED) — pixels, sans copie

La separation n'est pas une commodite. Un `socketpair` a un tampon borne
(64 KiB ici) : y faire passer une surface de 4 MiB reviendrait a la decouper en
soixante-quatre morceaux, a les recopier deux fois — une a l'ecriture, une a la
lecture — et a bloquer le canal de controle pendant tout ce temps. La memoire
partagee, elle, ne recopie rien : les deux processus ecrivent dans les memes
frames physiques.

Ce qui decide de quel canal : **la taille et la duree de vie**. Un message de
controle est petit et ephemere ; une surface est grosse et reutilisee d'une
trame a l'autre.

## Navigateur → Renderer

| Message | Charge | Notes |
|---|---|---|
| `CREATE_DOCUMENT` | id de contexte, origine, viewport | Cree un `BrowsingContext` cote renderer. L'id est attribue par le navigateur : c'est lui qui tient l'arbre. |
| `NAVIGATE` | id, url, remplace ? | Charge un document. Le renderer ne decide jamais de naviguer seul. |
| `INPUT_EVENT` | id, genre, coordonnees ou touche | Souris, clavier, defilement. |
| `RESIZE` | id, largeur, hauteur | Le viewport du contexte, pas la fenetre. |
| `TICK` | horodatage | Fait avancer minuteries, animations, reponses reseau. Le renderer ne bat pas tout seul : c'est le navigateur qui cadence, et c'est ce qui permet de le geler sans le tuer. |
| `SURFACE` | id, descripteur memfd, largeur, hauteur, pas | Le tampon ou peindre. Passe par `SCM_RIGHTS`. |
| `CLOSE` | id | Detruit le contexte et tout ce qu'il tient. |
| `FETCH_RESPONSE` | id de requete, metadonnees, `fin` | La reponse a un `FETCH_REQUEST`. Le corps suit en `FETCH_DATA` : une image de trois mebioctets ne tient pas dans une trame de controle, dont `CHARGE_MAX` est petite a dessein. |
| `FETCH_DATA` | id de requete, morceau base64, `fin` | Un morceau de corps. La taille est choisie pour que le morceau **encode** reste sous `CHARGE_MAX` — base64 gonfle d'un tiers. |
| `AUDIT` | quoi | Demande au renderer ce qu'il possede. Un renderer de production ne sait pas repondre : la capacite est accordee au `fork`. |

## Renderer → Navigateur

| Message | Charge | Notes |
|---|---|---|
| `TITLE_CHANGED` | id, titre | |
| `URL_CHANGED` | id, url | Apres une redirection ou un `pushState`. |
| `CURSOR_CHANGED` | id, forme, lien | Le lien voyage avec la forme : le chrome affiche l'adresse survolee en barre d'etat, et la lui faire chercher dans le DOM d'en face serait exactement la lecture directe que cette architecture supprime. |
| `FOCUS_CHANGED` | id, foyer | Un champ de la page tient-il le clavier ? Le seul bit du DOM dont le chrome ait besoin, et le seul qu'il ne puisse pas aller chercher. |
| `FETCH_REQUEST` | id, url, methode, corps, en-tetes, document, destination | Le renderer demande une ressource. Le navigateur applique la politique, lit les temoins, ouvre la prise. Voir `moteur/transport.py`. |
| `AUDIT_RESULT` | quoi, descripteurs ou tentatives | Reponse a `AUDIT`. Voir `RENDERER_PRIVILEGE_AUDIT.md`. |
| `FRAME_READY` | id, generation, tampon, dimensions, hauteur du document, defilement | Les pixels sont dans la surface partagee ; le message ne porte que de quoi savoir quoi recomposer. La hauteur du document et la position du defilement voyagent avec : c'est ce dont le chrome a besoin pour son ascenseur, et c'est l'autre chose qu'il lisait dans le DOM. |
| `CONSOLE_MESSAGE` | id, niveau, texte | |
| `REQUEST_NAVIGATION` | id, url **absolue** ou pas d'historique, provenance | Un clic sur un lien, ou `location.href = …`, ou `history.go(n)`. Le renderer **demande**, il ne navigue pas. L'adresse est resolue contre celle du document avant l'envoi : la politique compare un schema, et un schema ne se lit pas dans `../ailleurs`. |
| `CRASH` | id, raison | Emis par le navigateur lui-meme quand `wait4` recolte une mort anormale — un renderer qui plante n'a plus rien pour parler. |

## Les regles qui comptent plus que la liste

**Le renderer ne decide de rien qui engage la securite.** Il demande, le
navigateur applique. Naviguer, ouvrir une connexion, lire un temoin, ecrire dans
le stockage : toutes ces decisions passent par le processus qui tient la
politique. Un renderer compromis ne doit gagner que ce que sa page pouvait deja
faire.

**Un contexte par renderer, ou un renderer par origine.** Le second est ce que
fait Chromium et ce vers quoi il faut aller : deux pages de la meme origine
peuvent se lire l'une l'autre de toute facon, donc les mettre ensemble ne perd
rien ; deux origines differentes ne le doivent pas, et un processus par origine
transforme la Same-Origin Policy en frontiere materielle plutot qu'en
verification logicielle.

**La surface appartient au navigateur.** Il l'alloue, la transmet, et la reprend
quand le contexte meurt. Un renderer qui allouerait sa propre surface pourrait
en allouer mille — et `RLIMIT_AS` le tuerait, ce qui est correct mais brutal.

**Le renderer ne bat pas tout seul.** `TICK` vient du navigateur. C'est ce qui
permet de geler un onglet en arriere-plan sans le tuer, et c'est aussi ce qui
rend le comportement reproductible dans une epreuve.

**Une mort n'est jamais silencieuse.** Le navigateur recolte par `wait4` depuis
un fil dedie et fabrique lui-meme le `CRASH` : attendre un message d'un
processus mort serait attendre pour toujours.

## Ce que l'OS fournit deja pour tout cela

Rien ne manque. `socketpair` duplex, `SCM_RIGHTS` avec `MSG_CTRUNC`,
`memfd_create` + `MAP_SHARED` sur frames reellement partagees, `futex` cle par
adresse physique — donc utilisable dans la surface —, `fork`/`execve`/`wait4`,
`kill`, `RLIMIT_AS`, et deux classes d'ordonnancement. Voir
`BROWSER_OS_MODERNIZATION.md` pour l'etat detaille.

## Ce que l'implementation a tranche, et comment

**Le format de la liste d'affichage : la question ne se pose plus.** Elle ne
traverse pas. Le renderer joue sa liste d'affichage chez lui et ne publie que
des pixels. C'est l'inverse de Chromium, qui envoie une liste que le processus
GPU rasterise — meilleur quand il y a un GPU et un compositeur, ce qui n'est pas
le cas ici. En echange, on evite d'inventer un encodage binaire stable pour des
tuples Python, qui etait la seule piece de ce protocole a demander vraiment du
travail.

**La charge des messages de controle est du JSON.** Ils sont petits et rares :
une navigation, un redimensionnement, une frappe. Un encodage binaire aurait
coute un analyseur de plus a ecrire et a verifier, sans rien gagner de mesurable.
Le jour ou un message de controle deviendra chaud, c'est ce paragraphe qu'il
faudra contredire — pas la structure.

**Le partage de la pile reseau : le renderer emet lui-meme.** C'est le choix le
moins sur des deux, et il est assume pour cette version : le renderer est encore
un `fork` du navigateur, donc il a de toute facon acces a tout ce que le
navigateur avait. La question redeviendra reelle le jour ou il sera lance par
`execve` avec ses propres droits — et c'est a ce moment-la qu'il faudra faire
passer les requetes par le navigateur.

**Un descripteur ne se lit pas avec `recv`.** Ce n'est pas un choix, c'est un
piege, et il a coute une soiree : un descripteur envoye par `SCM_RIGHTS` voyage
dans les donnees auxiliaires du meme envoi que les octets, et un `recv`
ordinaire lit les octets en **jetant le descripteur en silence** — le noyau le
ferme, personne n'est averti. Les deux cotes lisent donc par `recvmsg`
**partout**, y compris pour les messages qui ne portent rien : on ne sait pas a
l'avance lequel en portera.

**`RLIMIT_AS` est un budget, pas un plafond.** `fork` transmet l'espace
d'adressage du parent : une limite absolue se mesure contre ce que le navigateur
occupait deja. Un navigateur qui a beaucoup travaille avant de creer son
renderer lui donne un enfant qui nait au ras de son plafond, et dont le premier
`mmap` echoue sans que son code y soit pour rien. La limite s'exprime donc en
« tant de plus que ce dont il herite », et la vraie regle reste celle de
Chromium : **un zygote se forke tot**.

## Ce qui n'est toujours pas tranche

* **~~La reprise apres crash.~~** Tranche : la derniere image reste a l'ecran,
  l'etat dit « page crashee », les ressources sont rendues, et F5 fabrique un
  renderer neuf. L'image survit parce qu'elle a deja ete copiee chez Qt — la
  surface partagee peut donc etre liberee sans que l'ecran se vide.
* **Le courtage synchrone.** Le navigateur sert un `FETCH_REQUEST` dans son
  pompage d'evenements, donc il gele son chrome le temps de l'aller-retour. Ce
  n'est pas une regression — le chargement en-processus faisait pire — mais
  c'est le goulot suivant.
* **Le media.** `reseau.tranche` et `reseau.taille_distante` restent directs :
  ils lisent par tranches de plusieurs mebioctets, en boucle, et les passer par
  des trames de controle sans les avoir d'abord rendus asynchrones
  transformerait une lecture video en gel de l'interface.
* **Le nombre de renderers.** Un par onglet, ou un par origine ? Le second est ce
  vers quoi il faut aller — il transforme la Same-Origin Policy en frontiere
  materielle — et rien dans le protocole ne s'y oppose.
