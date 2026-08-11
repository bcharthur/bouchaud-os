# Le protocole navigateur ↔ renderer

*Esquisse, pas implementation. Aucun processus de rendu n'existe : ce document
fixe le contrat pendant qu'il est encore gratuit de le changer.*

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

## Renderer → Navigateur

| Message | Charge | Notes |
|---|---|---|
| `TITLE_CHANGED` | id, titre | |
| `URL_CHANGED` | id, url | Apres une redirection ou un `pushState`. |
| `CURSOR_CHANGED` | id, forme | |
| `DISPLAY_LIST_READY` | id, numero de trame, rectangle sale | Les pixels sont dans la surface partagee ; le message ne porte que de quoi savoir quoi recomposer. |
| `CONSOLE_MESSAGE` | id, niveau, texte | |
| `REQUEST_NAVIGATION` | id, url, provenance | Un clic sur un lien. Le renderer **demande**, il ne navigue pas : c'est le navigateur qui applique la politique. |
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

## Ce qui n'est pas tranche

* **Le format de la liste d'affichage sur le fil.** Aujourd'hui ce sont des
  tuples Python. Entre deux processus il faudra un encodage binaire stable, et
  c'est la seule piece qui demande vraiment du travail.
* **La reprise apres crash.** Recharger la page, ou afficher un cadre mort ?
  Chromium fait le second et laisse l'utilisateur decider.
* **Le partage de la pile reseau.** Le renderer emet-il ses requetes lui-meme,
  ou les demande-t-il ? Le second est plus sur — les temoins et le cache restent
  hors de sa portee — et plus lent d'un aller-retour.
