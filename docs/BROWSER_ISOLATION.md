# L'execution isolee : du Worker au renderer

*Ce que trois vagues de travail ont etabli, ce qu'elles ont coute, et ce qui
reste ouvert. Rien ici n'est projete : chaque affirmation renvoie a une epreuve
qui passe ou a une mesure qui existe.*

## La question de depart

Un navigateur moderne fait tourner plusieurs mondes JavaScript en meme temps.
Le moteur n'en avait qu'un : un document, un contexte, un fil. Deux clients de
ce manque, et dans cet ordre :

* un **Worker** — deux mondes dans le meme processus, sans DOM a partager ;
* un **renderer separe** — deux mondes dans deux processus, avec une surface a
  partager et une frontiere de securite a tenir.

Le premier est le second en plus petit et sans frontiere de processus. Le faire
d'abord n'etait pas de la prudence : c'etait le seul moyen de decouvrir les
problemes d'isolation la ou ils se diagnostiquent encore — dans le meme
debogueur.

## Ce que le Worker a coute, et ce qu'il a appris

| Piece | Ecrite ou | Ce qu'elle a demande |
|---|---|---|
| Runtime QuickJS par contexte | `bojs.cpp` | **Rien** : `bojs_cree` en fabriquait deja un par contexte. Il fallait le verifier, pas l'ecrire. |
| GIL relache pendant l'execution | `bojs.cpp` | `Py_BEGIN_ALLOW_THREADS` autour de `JS_Eval`, `JS_Call` et de la pompe a promesses ; `PyGILState_Ensure` dans le pont. Sans cela un Worker aurait tenu le verrou de Python pendant tout son calcul. |
| Arret d'un script qui boucle | `bojs.cpp` | `bojs.interromps` : un `bool` relu par le gardien d'interruption. Le seul point de rendez-vous sur entre deux fils qui partagent un runtime. |
| Surface globale dediee | `prelude_worker.js` | Ecrite positivement, pas obtenue en retirant des proprietes au prelude de la fenetre. 480 lignes. |
| Surfaces communes | `prelude_partage.js` | `WebSocket` et `indexedDB` sortis du prelude de la fenetre, parametres par les primitives d'evenement de chaque monde. |
| Hote du Worker | `moteur/worker.py` | Un fil, deux files, et **aucun service en propre** : reseau et stockage passent par ceux du document. |

### La reponse a la question qui comptait

**Chaque mecanisme du Worker se reutilise-t-il pour le renderer ?**

| Mecanisme du Worker | Reutilise pour le renderer ? | Pourquoi |
|---|---|---|
| Runtime QuickJS separe | **Oui, gratuitement** | Un processus separe a forcement son runtime. Le travail fait pour le Worker rendait la chose deja vraie. |
| GIL relache | **Oui, et c'etait indispensable** | Sans lui, le renderer forke aurait herite d'un pont qui suppose le GIL tenu. Le pont marche desormais depuis n'importe quel fil, donc depuis n'importe quel processus. |
| `bojs.interromps` | **Non, remplace par mieux** | Entre processus, `SIGKILL` fait le meme travail sans coordination. Le drapeau reste ce qui sert **dans** le renderer, pour ses propres workers. |
| File de messages entre mondes | **Oui, dans sa forme** | Vidangee au battement, jamais au milieu d'un script. Le renderer applique la meme regle a `INPUT_EVENT` et `TICK`. |
| Clonage structure | **Oui, tel quel** | `Worker.postMessage`, `Window.postMessage`, `MessagePort` et IndexedDB s'en servent deja. Le protocole du renderer, lui, encode en JSON — ses messages sont petits et rares, et la surface porte le volume. |
| Cycle de vie et `terminate()` | **Oui, dans son ordre** | Contexte detruit, minuteries oubliees, requetes abandonnees, connexions fermees, transactions rendues. `superviseur.ferme()` suit la meme liste, `SIGKILL` en dernier recours. |
| Emprunt des services de l'hote | **Non — c'est la difference** | Un Worker emprunte le reseau et le stockage de son document parce qu'il partage son origine et son processus. Un renderer ne doit **rien** emprunter : il demande. C'est la seule ligne ou le renderer est plus strict que le Worker, et c'est celle qui compte pour la securite. |

La derniere ligne est la lecon du chantier. Tout le reste s'est transpose ;
c'est la politique qui a demande un dessin different, et c'est aussi la seule
chose qu'un Worker ne pouvait pas apprendre — il n'y a pas de frontiere de
confiance entre une page et son worker.

## Ce que le renderer separe etablit

### La forme

    navigateur (chrome Qt, fenetre, entrees, politique)
        |
        |  socketpair AF_UNIX      messages courts, ordonnes, versionnes
        |  memfd + MAP_SHARED      pixels, sans copie, deux tampons
        |
    renderer (HTML, CSS, DOM, mise en page, QuickJS)

Cree par `fork` sans `execve` — le « zygote » de Chromium. L'enfant herite de
l'interprete deja demarre et des modules deja importes : la creation coute
quelques millisecondes au lieu du demarrage complet d'un Python. C'est aussi ce
qui rend le mecanisme portable sous Bouchaud OS, ou le navigateur est un unique
ELF statique et ou il n'y a pas de second binaire a lancer.

### Le protocole

Huit octets d'en-tete — version, genre, longueur — puis une charge JSON.
Quatorze genres, sept dans chaque sens. Ce qui est **verifie a chaque trame** :
la version exacte, le genre connu, le sens autorise, la longueur bornee a
1 Mio, et la lecture complete. Une trame illisible est fatale a la connexion :
continuer reviendrait a lire la suite du flux depuis une position dont on ne
sait plus rien.

Une chose a coute une soiree et merite d'etre dite ici : **un descripteur
envoye par `SCM_RIGHTS` voyage dans les donnees auxiliaires du meme envoi que
les octets, et un `recv` ordinaire le jette en silence.** La surface partait, le
message arrivait, et le renderer se retrouvait a parler d'une surface dont le
descripteur n'existait plus. La seule facon sure est de lire par `recvmsg`
**partout**, y compris pour les messages qui ne portent rien — on ne sait pas a
l'avance lequel en portera. C'est ce que fait `protocole.Canal`.

### Ce qui est mesure

| Jalon | Etat | Ce qui l'etablit |
|---|---|---|
| `RENDERER_BASIC` | PASS | La fixture de connexion, jouee de bout en bout dans un autre processus : cliquer, taper six lettres, envoyer. L'epreuve ne touche aucun DOM d'en face — elle n'a que des messages et une surface, exactement comme un vrai chrome. |
| `RENDERER_CRASH_ISOLATION` | PASS | Un `SIGKILL` sur le renderer. Le navigateur l'apprend par `wait4`, fabrique lui-meme l'evenement — un processus mort ne parle pas —, garde la derniere trame lisible, et fait repartir un remplacant. |
| `RENDERER_MEMORY_ISOLATION` | PASS | Le renderer annonce dans `READY` la limite d'adressage dans laquelle il s'est reveille. Puis on lui demande de mapper une surface de 128 Mio que le navigateur, lui, alloue et mappe sans peine : il refuse, le dit, et survit. Une limite posee mais non appliquee se lirait exactement comme une limite appliquee. |
| Isolation processeur | **PASS par mecanisme + mesure separee** | Voir ci-dessous. |

### L'isolation processeur, et pourquoi elle est dite en deux temps

Deux choses distinctes, et les confondre serait une facon polie de mentir :

* **le mecanisme** est verifie par l'epreuve du renderer : l'enfant est en
  classe `Normale`, et il y reste quand le navigateur se declare interactif. Il
  n'herite donc pas de la classe de son parent, ce qui annulerait tout ;
* **l'effet** est mesure par `ordonnanceur-probe`, sous Bouchaud OS, dans QEMU,
  avec huit processus de calcul concurrents. Pire retard de l'interface :
  **8 000 us sans priorite, 1 000 us avec**, sans que les calculs perdent quoi
  que ce soit (39 055 tours contre 39 492). Le detail est dans
  `BROWSER_OS_MODERNIZATION.md`.

Ce qui n'a **pas** ete fait : rejouer cette mesure avec le renderer lui-meme
comme charge, sous Bouchaud OS. Rien ne laisse penser que le resultat differerait
— un renderer qui met en page est exactement le processus de calcul de la sonde
—, mais ce serait une inference, pas une mesure, et le tableau ci-dessus le dit
comme tel.

## Ce que la separation a corrige en chemin

Un renderer n'a pas de chrome. Il recoit un `INPUT_EVENT` et doit produire le
comportement complet d'un clic : foyer, evenement, action par defaut, lien.
Cette sequence vivait dans `navigateur.py`, et `Document.clique` ne faisait que
la premiere moitie — deplacer le foyer. Tant qu'il n'y avait qu'un appelant,
c'etait defendable ; le renderer en a fait un second.

La sequence est donc descendue dans le moteur (`Document.clic_complet`), et les
deux appelants la partagent. **Le formulaire de connexion s'envoie desormais par
un vrai clic**, la ou l'epreuve devait auparavant tricher avec un
`dispatchEvent(new Event('submit'))` depuis JavaScript. Ce n'est pas un effet de
bord du chantier : c'est ce que le chantier a rendu visible.

## Ce qui n'est pas fait, et qui est nomme

* **L'isolation par site** — un processus par origine. C'est ce vers quoi il
  faut aller : deux pages de la meme origine peuvent se lire de toute facon,
  deux origines differentes ne le doivent pas, et un processus par origine
  transforme la Same-Origin Policy en frontiere materielle. Le protocole porte
  deja un identifiant de contexte par message, donc rien ne l'empeche.
* **Un processus par cadre.** Meme raison, un cran plus loin.
* **Le compositeur.** La liste d'affichage est rasterisee **dans** le renderer
  et seul le resultat en pixels est publie. Chromium fait l'inverse et laisse un
  processus GPU rasteriser. C'est mieux quand il y a un GPU ; il n'y en a pas
  ici, et rasteriser sur place evite d'inventer un encodage binaire pour des
  tuples Python.
* **Le `SharedArrayBuffer` et les `Atomics`.** Le noyau saurait les porter —
  `memfd` + `futex` cle par adresse physique — mais QuickJS non, et ce serait le
  chemin le plus court vers une corruption silencieuse.
* **Les `ServiceWorker`.** Ils demandent un intercepteur de requetes et un cache
  persistant, et n'apprennent rien de plus sur l'isolation.
* **Le transfert (`transfer`) d'un `postMessage`.** Tout est copie. Un
  `ArrayBuffer` transmis reste utilisable chez l'emetteur, la ou la norme le
  detacherait. C'est une difference observable, et elle est dite plutot que
  decouverte.

## L'ordre de travail suivant

1. **Brancher le chrome Qt sur le superviseur.** Le prototype est pilote par les
   epreuves ; il ne l'est pas encore par la fenetre. C'est le pas qui transforme
   un mecanisme en navigateur.
2. **Un renderer par origine.** Le protocole est pret, le superviseur ne tient
   qu'un enfant. C'est la seule piece qui manque pour que la SOP devienne une
   frontiere de processus.
3. **Rejouer `ordonnanceur-probe` avec un renderer comme charge**, sous
   Bouchaud OS, pour remplacer la derniere inference de ce document par une
   mesure.
