# Demarrage a froid : ce que la mesure dit, et ce qu'elle ne dit pas

BOUCHAUD_C47_LE_PLUS_GROS_POSTE_EST_MESURE

Document de mesure. Il ne propose rien qu'un chiffre ne soutienne, et nomme
explicitement ce qui reste inconnu.

## 1. Les deux runs compares

| | #347 | #348 |
|---|---|---|
| `head_sha` | `6b9c792` | `7500034` |
| BROWSER_HOST_START | T+22 s | T+13 s |
| BROWSER_HOST_INITIALIZED | T+38 s | T+29 s |
| M11_GUI_HANDSHAKE_OK | T+152 s | T+100 s |
| FRAME_PRESENTED | T+180 s | **T+116 s** |
| DOCUMENT_LOADED | T+203 s | T+130 s |

**Ceci est n=1 contre n=1.** La regle de ce chantier demande trois mesures de
chaque cote avant de parler de gain. Le seul changement de performance entre
les deux est la frame non mise a zero au `fork` (environ 25 % sur le `fork`,
soit ~40 ms par service) : il ne peut pas expliquer 64 s a lui seul. La
difference est donc pour l'essentiel NON ATTRIBUEE, et probablement de la
variance de coureur.

Niveau : OBSERVE. Pas CAUSE CONFIRMEE.

## 2. Le plus gros poste mesure : les fautes de page file-backed

Releve `[PERF-PROC]` de #348, dernier echantillon de chaque processus :

| service | pid | rss | fautes | total | dont fichier |
|---|---|---|---|---|---|
| WebContent | 15 | 87 Mio | 17748 | 17,34 s | **3277 / 17,13 s** |
| Compositor | 14 | 45 Mio | 13690 | 11,73 s | **2347 / 11,48 s** |
| WebWorker #1 | 16 | 41 Mio | 3061 | 8,02 s | **2440 / 8,01 s** |
| ImageDecoder | 13 | 9 Mio | 667 | 3,48 s | **309 / 3,44 s** |
| WebWorker #2 | 17 | 42 Mio | 3129 | 1,18 s | 2480 / 1,15 s |
| WebWorker #3 | 18 | 41 Mio | 3058 | 0,062 s | 2440 / 0,054 s |
| WebWorker #4 | 19 | 42 Mio | 3134 | 0,066 s | 2480 / 0,057 s |

**Total des fautes fichier : environ 41 s.**

## 2 bis. CORRECTION : ce que la section 3 affirmait de trop

Ce document a d'abord ecrit « le premier paie le disque » et « une faute a
155 ms, c'est une lecture de disque ». Les deux etaient trop affirmatives.

`faute_memoire.rs` le dit lui-meme : « une faute qui finit par charger
appartient a sa categorie, et **son attente est deja comprise dans sa duree** ».
`FichierPrive total_us` mesure donc la latence VECUE par le processus -- ce
qui est la bonne grandeur, mais pas une mesure d'entrees-sorties.

    OBSERVE       une faute classee FichierPrive a coute 155 ms au processus.
    NON ETABLI    combien de ces 155 ms sont du backing, de l'attente, du
                  verrouillage, du cache ou du mapping.

De meme, « le premier paie le disque » devient : « le premier paie un cout
associe au chemin FichierPrive FROID ; sa composition se mesure, et pour
Ladybird elle ne l'est pas encore. »

## 2 ter. Une inference invalide, et pourquoi

J'ai sonde une image de scenario, trouve tous les fichiers `disk_backed=0`, et
failli en conclure que les ELF de Ladybird ne sont pas adosses au disque.
C'etait faux, pour une raison de seuil :

    tar.rs   INLINE_BOOT_FILE_SIZE = 4 Mio
             <= 4 Mio  -> contenu copie dans le noeud, AUCUNE etendue
             >  4 Mio  -> register_disk(Drive::Slave)   sous QEMU
             >  4 Mio  -> register_memory(adresse)      sur la Trigkey UEFI

Les fichiers de mon scenario faisaient treize kilooctets : inline PAR
CONSTRUCTION. Les ELF de Ladybird depassent tous quatre mebioctets. Verifie
avec un temoin de six mebioctets :

    BACKING_PROBE path=/gros-temoin size=6291456 source=ata-disk generation=1
    BACKING_PROBE path=/coutfork    size=13864   source=inline   generation=none

`disk_backed` est desormais remplace par `BackingKind::{Inline,Disk,Memory}` :
une lecture ATA et un memcpy depuis le ramdisk UEFI ne se soignent pas pareil,
et une optimisation du chemin ATA qui gagnerait trente secondes en CI pourrait
ne rien changer sur la machine physique.

## 3. La demonstration tient en trois lignes

    WebWorker #1  pid=16  fichier=2440 fautes / 8 008 342 us
    WebWorker #3  pid=18  fichier=2440 fautes /    54 398 us
    WebWorker #4  pid=19  fichier=2480 fautes /    57 153 us

Le MEME nombre de fautes, sur le MEME binaire, coute **147 fois moins** au
troisieme worker qu'au premier.

La difference n'est ni le code, ni le nombre de pages, ni l'ordonnanceur :
c'est que les pages sont deja dans le cache. Le premier paie le disque, les
suivants ne paient rien.

Le pire cas le confirme : `pire_us=155246` pour WebContent. Une seule faute a
coute 155 ms -- c'est une lecture de disque, pas un defaut de conception du
gestionnaire de fautes.

## 4. Ce que cela invalide

Le chantier a longtemps cherche le temps perdu du cote du `fork`, de
l'`execve` et du gros verrou. Les mesures disent :

    fork    169 ms par service
    execve    5 ms par service  (dont 99,6 % de liberation)
    fautes   41 s cumulees

Le `fork` et l'`execve` REUNIS font moins d'une seconde sur les six services.
Optimiser le `fork` COW -- qui est le reflexe naturel -- s'attaquerait a un
poste cent fois plus petit que celui qui domine.

## 5. Le prechauffage existe deja, et il ne fait rien

    [PRECHAUFFAGE] termine=1 fichiers=0 pages=0 duree_ms=0

Zero fichier, zero page. Le sous-systeme est present, il se declare termine,
et il n'a rien charge.

## 5 bis. MESURE CAUSALE LOCALE : le contraste reproduit sans Ladybird

Un ELF de 5,01 Mio -- au-dessus du seuil, donc `ata-disk`, donc le meme chemin
que les binaires de Ladybird -- lance quatre fois, toutes pages touchees :

      #  pid  fautes  total_ms  attente  acquire  cc_miss  cc_lect  cc_att   map  residu  ata_n   ata_ms
      1   10      77      73.2      0.0     61.5   1159.1   1114.9     0.0   1.9     9.5     81   1108.3
      2   11      77      15.1      0.0      0.6      0.0      0.0     0.0  12.6     1.8      0      0.0
      3   12      77       2.7      0.0      0.4      0.0      0.0     0.0   0.6     1.6      0      0.0
      4   13      77       2.6      0.0      0.3      0.0      0.0     0.0   0.7     1.6      0      0.0

Le contraste est reproduit : **73,2 ms a froid contre 2,6 ms a chaud**, pour un
nombre de fautes IDENTIQUE (77). Meme forme que 8 s contre 54 ms chez Ladybird.

Et il est decompose :

    cc_miss   1159,1 ms   le cout des defauts de clean_page_cache
    cc_lect   1114,9 ms   dont 96 % est la LECTURE DU SUPPORT
    ata_ms    1108,3 ms   et cette lecture est bien de l'ATA
    attente       0,0 ms
    cc_att        0,0 ms   AUCUNE contention

Un defaut de ma propre instrumentation a ete corrige en route : `acquire` ne
consulte pas un cache, il LIT LE SUPPORT lui-meme sur un defaut
(`page_cache.rs`). La premiere version publiait `cache_us=62 ms backing_us=0`,
ce qui se lisait « le cache est lent et le disque ne fait rien ». Le disque
travaillait DANS le cache.

### Ce que cela etablit, et sur quoi

CAUSE CONFIRMEE, pour CETTE charge, sous QEMU, source `ata-disk` :
le surcout du premier lancement est de la lecture ATA faite dans
`clean_page_cache::acquire`, et la contention n'y est pour rien.

NON ETABLI pour Ladybird. Deux raisons de ne pas extrapoler :

1. Ici le processus ne paie que 73 ms sur 1,16 s de chargement systeme :
   l'essentiel est paye par la lecture anticipee et l'image d'`execve`, hors
   des fautes comptees. Chez Ladybird, WebWorker #1 paie 8 s DANS ses propres
   fautes. Ce n'est pas la meme repartition.
2. `ata_n=81` pour 5,25 Mio, soit environ 64 Kio par lecture : la lecture
   anticipee groupe bien. Si les fautes de Ladybird tombent hors de sa
   fenetre, elles feraient des lectures de quatre kilooctets -- seize fois
   plus d'operations. C'est une hypothese testable, pas une conclusion.

### Sur la Trigkey, ce resultat ne vaut PAS

Les gros ELF y passent par `register_memory` : un memcpy depuis le ramdisk
UEFI, pas une lecture ATA. `BACKING_MEMORY` les compte a part precisement pour
que les deux ne soient jamais melanges dans une meme conclusion.

## 6. La prochaine optimisation, UNE SEULE

SUSPENDUE. Avant de prechauffer quoi que ce soit, il faut la meme
decomposition sur un vrai run Ladybird -- l'instrumentation existe desormais,
il manque le run. Prechauffer maintenant reviendrait a corriger une cause
etablie sur une charge synthetique et supposee sur la vraie.

L'idee, quand elle reviendra :
faire lire au prechauffage les binaires et bibliotheques de Ladybird avant que
le premier service ne les touche.

- **Hypothese** : les 41 s de fautes fichier sont des lectures de disque
  synchrones, payees une fois par page, en serie avec le demarrage.
- **Metrique visee** : `fichier=N/Xus` de `[PERF-PROC]` pour WebContent,
  Compositor et le premier WebWorker.
- **Prediction falsifiable** : le premier worker doit approcher les 54 ms du
  troisieme. S'il reste a 8 s, l'hypothese est fausse et le cache n'est pas
  en cause.
- **Ce qui l'invaliderait** : si `[PRECHAUFFAGE]` charge les pages et que les
  durees ne bougent pas, alors le cout n'est pas la lecture mais le chemin de
  faute lui-meme, et il faut chercher ailleurs.

## 7. Ce qui reste OUVERT

- Les 43,4 s entre le retour du `fork` et le debut de l'`execve` du premier
  worker (baseline #347) ne sont toujours pas decomposees. Aucune sonde ne
  borne ce que fait l'enfant entre les deux.
- `first_user_instruction_ms` et `spawn_request_ms` ne sont pas instrumentes.
- `unexplained_ms` vaut 78 % sur la decomposition du premier worker.
- `HOST_IMAGES_OK codecs=11/11` n'est jamais atteint sous Ladybird alors qu'il
  l'est sous Chromium headless. Un codec echoue, ou la fenetre de mesure se
  ferme avant.
