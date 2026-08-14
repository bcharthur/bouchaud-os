# RENDERER_PRIVILEGE_AUDIT

*Ce que le processus de rendu peut faire, et ce qu'il ne peut pas. Mesure, pas
intention. Chaque ligne vient d'une tentative que le renderer joue pour de vrai
dans `verifie_renderer_privileges`, et le resultat brut est depose dans
`tools/userland/navigateur/tests/audit-privileges.json`.*

## Pourquoi ce document existe

Le renderer etait une frontiere de **crash** : quand il meurt, la fenetre vit.
Cela a ete etabli tot et cela tient. Il n'etait pas une frontiere de
**privileges** : il naissait avec la table de descripteurs entiere du
navigateur, et il chargeait ses ressources lui-meme parce que le code Python
savait appeler les modules correspondants.

Un tableau PASS/FAIL vaut mieux qu'une intention pour deux raisons. La premiere
est qu'il rend l'ecart visible. La seconde est qu'il empeche de se raconter que
la frontiere est etanche : plusieurs lignes ci-dessous sont a FAIL, et elles y
restent tant qu'un mecanisme du noyau ne les ferme pas.

## Le tableau

| Capacite | Verdict | Ce que cela veut dire |
|---|---|---|
| Prises reseau heritees du navigateur | **PASS** | La reserve de connexions du navigateur — prises TCP ouvertes, certaines deja authentifiees TLS — est fermee dans l'enfant. Un renderer temoin cree **sans** balayage sert de mesure du probleme : il en herite, celui de production n'en a aucune. |
| Fichiers et repertoires herites | **PASS** | Aucun descripteur de fichier du navigateur ne survit au `fork`. |
| Descripteur herite exploitable | **PASS** | Le renderer cherche un descripteur utilisable en dehors de ceux qu'on lui a accordes, et n'en trouve aucun. |
| Prise de controle | **accordee** | Une, et une seule. C'est le canal du protocole. |
| Surface partagee | **accordee** | Le `memfd` de la fenetre, recu par `SCM_RIGHTS`. Le renderer ne peut pas en allouer d'autres : c'est le navigateur qui alloue. L'inventaire en compte deux et ce n'est pas une fuite — `mmap.mmap` duplique le descripteur qu'on lui donne, et les deux designent la meme surface. |
| Console serie (fd 0, 1, 2) | **accordee, assumee** | Un renderer muet est un renderer qu'on ne sait pas deboguer sous emulation. Ce qu'elle donne a un attaquant — ecrire dans un journal — est tres au-dessous d'une prise TCP heritee. |
| Reseau : requetes courtees | **PASS** | Sous courtage, chaque `reseau.charge` du renderer devient un `FETCH_REQUEST`. Le navigateur applique `securite.verifie`, lit les temoins, ouvre la prise, ecrit le cache. |
| Navigation decidee par le renderer | **PASS** | Le renderer **demande** (`REQUEST_NAVIGATION`) et resout l'adresse contre celle de son document avant d'envoyer. Le navigateur verifie le schema et applique — ou refuse. Une demande vers `file:///etc/shadow` remonte en `NAVIGATION_REFUSEE` et n'est jamais suivie. |
| Historique | **PASS** | `history.go(n)` remonte au chrome, qui seul sait ou cela mene. Le renderer ne peut pas se deplacer dans une pile qu'il n'a pas. |
| Ouverture d'un fichier **par le nom** | **FAIL** | `open("/etc/hostname")` reussit. Fermer les descripteurs herites ne ferme pas le systeme de fichiers. |
| Ouverture d'une prise **par le nom** | **FAIL** | `socket()` puis `connect()` reussit. Le courtage retire au renderer le *besoin* du reseau, il ne lui en retire pas la *possibilite*. |
| Lecture directe du stockage | **FAIL** | Le renderer peut importer `moteur.stockage` et ouvrir le magasin de temoins par son chemin. |
| Espace d'adressage | **PASS** | `RLIMIT_AS` pose dans l'enfant, et **verifie** : le renderer annonce la limite qui s'applique reellement de son cote, ce qui distingue une limite posee d'une limite appliquee. |
| Classe d'ordonnancement | **PASS** | Le renderer reste `Normale` quand le navigateur passe `Interactive`, et il y reste — la classe n'est pas heritee par accident. |
| Reponse a une demande d'audit | **PASS** | Un renderer de production ne sait pas repondre a « que possedes-tu ? ». La capacite est accordee au `fork`, par le navigateur, et seulement aux renderers d'audit. |

## La ligne qui separe les PASS des FAIL

Elle est nette, et elle merite d'etre nommee : **les PASS sont ce qu'on peut
retirer depuis l'espace utilisateur, les FAIL sont ce qui demande le noyau.**

Fermer un descripteur, ne pas accorder une capacite, refuser une navigation,
poser une limite d'adressage : tout cela se fait avec ce que le systeme offre
deja. Interdire `open()` et `socket()` demande un mecanisme qui n'existe pas
encore sous Bouchaud OS — l'equivalent d'un `seccomp` ou d'un `landlock`, ou
bien un `execve` vers un binaire de renderer demarre dans un espace de noms
reduit.

Tant que ce mecanisme n'existe pas, le modele de menace du renderer est celui-ci
et pas un autre :

> Une page qui obtient l'execution de code dans le renderer perd l'acces aux
> connexions ouvertes du navigateur, a ses fichiers, a son historique et a sa
> politique de navigation. Elle conserve la possibilite d'ouvrir un fichier ou
> une prise par le nom.

C'est un progres reel et une frontiere incomplete. Les deux sont vrais.

## Ce que `FD_CLOEXEC` n'aurait pas fait

Rien. Le renderer nait par `fork()` **sans `execve()`** — le modele « zygote »,
choisi parce qu'il coute quelques millisecondes au lieu du demarrage complet
d'un Python, et parce que sous Bouchaud OS le navigateur est un ELF statique
unique sans second binaire a lancer. `FD_CLOEXEC` ferme a `exec` ; sans `exec`,
il ne ferme jamais.

C'est le piege exact de ce modele, et il est d'autant plus dangereux que le
drapeau habituel donne l'impression d'avoir traite la question. La seule
methode qui marche est d'enumerer `/proc/self/fd` et de fermer par numero, dans
l'enfant, avant qu'il ne serve.

Par numero, et non en appelant `.close()` sur les objets Python qui les
detiennent : un destructeur qui tourne dans un enfant de `fork` vidange des
tampons et valide des transactions dans des fichiers que le parent croit tenir
seul.

## Le defaut que l'audit a trouve en lui-meme

L'inventaire des descripteurs classe les prises par famille, ce qui demande de
les regarder. La premiere version faisait `socket.socket(fileno=os.dup(n))`,
lisait la famille, puis `detach()` suivi de `os.close(prise.fileno())`.

`detach()` rend le descripteur a personne : l'objet le lache, le noyau le garde,
et `fileno()` rend ensuite `-1`. Le `os.close` qui suivait fermait donc `-1`,
c'est-a-dire rien. L'inventaire fuyait un descripteur par prise inspectee.

Comme c'est l'inventaire qui alimente le balayage, l'enfant se retrouvait apres
nettoyage avec un **duplicata de chacune des prises qu'on venait de lui
fermer** : onze prises reseau, la ou l'audit devait en trouver zero. La mesure
disait exactement le contraire de la verite, et elle le disait a cause de
l'instrument.

Un audit qui cree ce qu'il mesure ne mesure rien. C'est pour cela que la
batterie adversariale existe : elle ne lit pas un inventaire, elle **essaie**.

## La soupape

Le balayage emporte au passage la connexion a l'affichage, dont un `QPainter`
sur une `QImage` n'a pas besoin — mais dont on ne peut pas jurer qu'aucune
plateforme Qt n'aura besoin. `BO_RENDERER_GARDE_FD=1` le desactive. Si un jour
une plateforme le prouve, cela se constate en une relance au lieu d'une
bissection.

## Comment rejouer l'audit

```bash
cd tools/userland
BO_AUDIT_PRIVILEGES=$PWD/navigateur/tests/audit-privileges.json ./test-moteur.sh
```

Le fichier JSON est versionne, comme `jalons.json` : `git log` dessus raconte
quand une capacite a ete retiree, et l'integration continue echoue s'il bouge
sans qu'on l'ait commite. Il n'est jamais ecrit a la main — un tableau
PASS/FAIL recopie devient faux au premier changement que personne ne pense a y
reporter.

Il ne porte que des **verdicts**. Le premier jet y deposait aussi l'inventaire
brut des deux cotes du `fork` : c'etait interessant a lire et impossible a
comparer, parce que le nombre de prises que le navigateur tient au moment du
`fork` depend de ce qu'il vient de faire, et le genre des descripteurs 0, 1 et 2
depend de la facon dont on a lance le processus. Un fichier de reference dont le
contenu change selon la machine ne peut pas servir de reference.

## Ce qui vient ensuite

Dans cet ordre, parce que chaque etape rend la suivante mesurable :

1. **Un `execve` optionnel vers un binaire de renderer.** Il rendrait
   `FD_CLOEXEC` operant, permettrait de demarrer dans un espace de noms reduit,
   et transformerait les deux FAIL « par le nom » en PASS. Le chemin
   `SCM_RIGHTS` est deja eprouve precisement pour cela : la surface voyage par
   descripteur alors qu'un `fork` la transmettrait gratuitement.
2. **Retirer au renderer l'acces au module de stockage** en supprimant l'import,
   pas seulement l'usage. C'est peu de chose et cela ferme une ligne.
3. **Un processus par origine.** Volontairement reporte : un navigateur, un
   renderer, robuste en navigation reelle d'abord.
