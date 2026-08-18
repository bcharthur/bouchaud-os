# Bouchaud OS — vision et plan de developpement

*Document de conception, ecrit le 18 aout 2026. Il dit ou l'on veut aller. Ce qui
est **fait** est dans `ETAT_DES_LIEUX.md`, et les deux ne doivent jamais etre
confondus.*

## 0. Le pari

Bouchaud OS n'a pas d'interet s'il est un Linux en moins bien. Il en a un s'il
prend au serieux trois choses que les systemes existants tranchent par heritage
plutot que par raisonnement :

1. **la memoire est un objet, pas un tableau d'octets** — le systeme sait ce
   qu'il transporte, donc il peut ne pas le copier ;
2. **le processeur se reserve, il ne se dispute pas** — l'ordonnancement sert un
   contrat, pas une file d'attente ;
3. **l'inference locale est un service systeme** — au meme titre que le reseau
   ou le systeme de fichiers, pas une application qui s'en arrange.

Le portage Ladybird n'est pas un but en soi : c'est le **banc d'essai** qui
rend ces trois affirmations verifiables. Un navigateur moderne est le programme
le plus exigeant qu'on puisse faire tourner — memoire, processus, reseau,
graphisme, temps reel souple. Ce qu'il revele est vrai pour tout le reste.

Ce document a d'ailleurs deja une dette envers lui : les trois defauts les plus
serieux corriges dans le noyau cette annee ont ete trouves **par** le portage,
pas par introspection.

---

## 1. Memoire — du Memory Fabric a un espace d'objets

### Ce qui existe

`Memory Fabric` et le moteur `VMA` savent deja mapper un fichier
paresseusement : au demarrage du disque Ladybird, 320 Mio de binaires sont
montes **sans etre copies**. C'est ce qui a rendu WebContent embarquable alors
que le plafond d'archive etait de 192 Mio.

### Ce qu'il faut en faire

**a. Un espace d'objets unifie.** Fichier, memoire anonyme, tampon IPC, surface
graphique, poids d'un modele : cinq noms pour la meme chose — une region
adressable, comptee, potentiellement partagee. `Resource Core` a commence cette
unification pour le CPU, la memoire, le DMA et le GPU ; il faut la mener au bout
cote adressage.

Critere : `mmap` d'un fichier, reception d'un tampon par IPC et allocation d'une
surface graphique doivent produire **le meme type d'objet** dans le noyau.

**b. L'IPC ne copie plus.** Aujourd'hui le corps d'une reponse HTTP traverse une
paire de sockets, octet par octet. Pour 672 octets c'est indolore ; pour une
image de 8 Mio, c'est trois copies inutiles. Passer une **poignee de VMA** au
lieu des octets rend le transfert independant de la taille.

Critere mesurable : le temps de transfert d'un tampon de 64 Mio entre deux
processus ne doit pas dependre de sa taille.

**c. Deduplication par contenu.** C'est la reponse structurelle a la dette ICU.
Aujourd'hui chaque binaire Ladybird embarque ~40 Mio de donnees ICU
**identiques**. Un cache de pages adresse par contenu (hachage de page) les
fusionne en une seule copie physique, automatiquement, sans que les programmes
en sachent rien.

Critere : trois binaires ICU-lourds resident en memoire pour le cout d'un seul
jeu de donnees.

**d. Instantanes de processus.** Geler l'image d'un processus et la reprendre.
Pour un navigateur : un onglet en arriere-plan libere sa memoire physique et
revient sans recharger. Pour un modele d'IA : l'etat post-chargement se restaure
en millisecondes au lieu de secondes.

### Ce que cela demande de retirer

La limite actuelle de **512 Mio de trames** annoncee par le VMM. Elle est un
plafond de developpement, pas une decision d'architecture, et elle bloquera tout
ce qui precede.

---

## 2. Processeur — du partage a la reservation

### Le probleme visible aujourd'hui

Dans les journaux QEMU du chantier M9 :

    [ps] desktop pid=4 cpu 77% ... RequestServer pid=6 cpu 0%

Le bureau brule 77 % du processeur pendant que le travail utile en consomme
zero. Ce n'est pas un defaut de reglage : c'est une boucle d'attente active la
ou il faudrait un reveil.

### La direction

**a. SMP reel.** Le noyau est mono-processeur. Tant qu'il l'est, « optimiser
l'ordonnancement » n'a pas beaucoup de sens. C'est le prealable de tout le
reste.

**b. Ordonnancement par contrat.** Un compositeur ne veut pas « une part
equitable » : il veut *une image avant la prochaine synchronisation verticale*.
Un decodeur audio veut *un tampon toutes les 10 ms*. Une compilation veut *tout
ce qui reste*. Trois besoins qui ne se comparent pas sur une seule echelle de
priorite.

Modele vise : chaque tache declare une echeance ou une part minimale, et
l'ordonnanceur la sert ou refuse le contrat — il ne promet jamais en silence.

**c. Appels systeme par lots.** Une boucle d'evenements de navigateur fait des
milliers d'appels courts. Une file d'ordres partagee entre l'espace utilisateur
et le noyau — le principe d'`io_uring` — supprime la transition pour le cas
courant.

Critere : une iteration de la boucle d'evenements de WebContent doit couter un
ordre de grandeur de moins qu'aujourd'hui, mesure a l'appel systeme pres.

**d. Attente sans bruit.** Generaliser ce que le correctif de `sys_poll` a
etabli en petit : aucune tache ne doit tourner pour attendre. Le `[ps]` du
bureau a 77 % est le premier candidat.

---

## 3. Graphisme — du tampon unique au compositeur

### Ce qui existe

Le gestionnaire de fenetres est **seul proprietaire** du tampon d'ecran, et les
applications peignent dans des surfaces partagees. C'est la bonne fondation, et
elle est deja tenue.

Skia tourne en CPU, sans GPU, et cela suffit a afficher une vraie page Web.

### La direction

**a. Composition par regions sales.** Aujourd'hui une image complete est
produite a chaque changement. Ne recomposer que ce qui a change divise le cout
d'un curseur qui clignote par plusieurs ordres de grandeur.

**b. Synchronisation verticale et double tampon.** Sans quoi le dechirement est
structurel, quelle que soit la vitesse du rendu.

**c. Un vrai chemin GPU, par virtio-gpu.** QEMU expose `virtio-gpu` ; c'est la
porte d'entree honnete vers l'acceleration, avant tout pilote materiel reel.
Ensuite seulement, Skia sur Vulkan.

Attention : le portage a **volontairement** retire Vulkan des dependances
graphiques. Le reintroduire est une decision d'architecture, pas un reglage de
construction, et elle se prend quand le compositeur en a besoin — pas avant.

**d. Echelle et couleur.** HiDPI et gestion colorimetrique se decident tot ou se
paient tres cher. Le moment de trancher est celui du compositeur, pas apres.

---

## 4. Intelligence artificielle — un service systeme, pas une application

C'est la partie la plus ambitieuse, et c'est aussi celle ou il faut etre le plus
franc sur les contraintes.

### Le principe

L'inference locale doit etre un **service du systeme**, expose comme le reseau
l'est par RequestServer : un processus separe, adresse par IPC, dont les clients
ne savent rien de l'implementation.

    application            navigateur              shell
         \                     |                    /
          \-------------- IPC -+-------------------/
                               |
                       Inference Server
                               |
                    runtime CPU quantifie
                               |
                   poids mappes paresseusement
                          (Memory Fabric)

### Pourquoi Bouchaud est bien place pour cela

Ce n'est pas un argument de facade : **le Memory Fabric est exactement ce dont
un moteur d'inference a besoin**. Les poids d'un modele sont un gros fichier en
lecture seule dont on ne touche qu'une fraction a chaque passe. Un mapping
paresseux avec eviction par pression memoire est la bonne structure — et elle
existe deja, prouvee sur 320 Mio de binaires Ladybird.

La deduplication par contenu (section 1c) s'y applique aussi : deux modeles
partageant un tokeniseur ou des couches quantifiees identiques ne les paient
qu'une fois.

### Ce qui manque, concretement

1. **SIMD.** Le noyau active SSE. L'inference quantifiee veut AVX2 au minimum,
   donc `XSAVE`/`XCR0` et la sauvegarde de contexte etendue. Sans cela, les
   performances seront dix fois en dessous du possible.
2. **La memoire.** Un modele utile en quantification 4 bits demande 0,5 a 2 Gio.
   Le plafond de 512 Mio de trames l'interdit purement et simplement.
3. **Le stockage.** Les poids doivent venir d'un vrai systeme de fichiers
   persistant, pas d'une archive depliee au demarrage.

Ces trois points sont des prealables durs. Les enoncer maintenant evite de
promettre une demonstration qui tiendrait par un modele-jouet.

### Ce que cela permet, une fois la fondation posee

- **Recherche semantique** dans le systeme de fichiers : indexation par plongements,
  requete en langue naturelle.
- **Palette de commandes** du shell : l'intention devient une commande, avec la
  commande montree avant d'etre executee — jamais executee a l'aveugle.
- **Navigateur** : resume de page, traduction, extraction — le tout **local**,
  ce qui est un argument de confidentialite reel et non decoratif.
- **Accessibilite** : description d'image, lecture d'ecran, dictee.

### La regle qui ne se negocie pas

Un modele local ne doit **jamais** obtenir plus de droits que le programme qui
l'appelle, et une sortie de modele n'est **jamais** une instruction. Un agent
qui propose une commande la montre ; c'est l'utilisateur qui la lance. Cette
regle se pose maintenant, dans l'architecture, parce qu'elle est impossible a
rajouter apres.

---

## 5. Ordre de travail

L'ordre n'est pas negociable : chaque etage repose sur le precedent.

**Etape A — finir le navigateur (M9 -> M12).** C'est le banc d'essai. Tant qu'il
n'est pas fini, les trois chantiers ci-dessus n'ont pas de juge.

**Etape B — retirer les plafonds.** La limite de 512 Mio de trames, puis le
stockage persistant. Sans eux, ni IA ni multi-onglets serieux.

**Etape C — SMP et ordonnancement par contrat.** Avant toute optimisation
graphique : recomposer plus finement sur un seul cœur deja sature ne se verra
pas.

**Etape D — compositeur (regions sales, vsync), puis virtio-gpu.**

**Etape E — espace d'objets unifie et IPC sans copie.** Le navigateur multi-onglets
en est le premier beneficiaire, et son premier verificateur.

**Etape F — SIMD etendu, puis Inference Server.**

**Etape G — deduplication par contenu et instantanes de processus.** Ce sont des
multiplicateurs : ils valent quand il y a beaucoup de processus lourds, donc
apres les onglets et l'IA, pas avant.

---

## 6. Ce que ce document promet — et ne promet pas

Il ne promet **aucune date**. Le chantier Ladybird a montre ce que valent les
estimations : trois causes annoncees pour un seul defaut M9, refutees l'une
apres l'autre par la mesure.

Il ne promet pas non plus que tout sera fait. Il fixe un ordre tel que
**s'arreter a n'importe quelle etape laisse un systeme coherent** — pas un
chantier a moitie demoli.

Ce qu'il engage, en revanche : rien n'entre dans `ETAT_DES_LIEUX.md` sans un
temoin qui s'execute en ring 3 et un verdict de CI qui exige sa sortie ligne par
ligne. C'est la seule regle qui ait vraiment tenu jusqu'ici, et c'est elle qui
rend le reste credible.
