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

## 6. La prochaine optimisation, UNE SEULE

Faire lire au prechauffage les binaires et bibliotheques de Ladybird avant que
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
