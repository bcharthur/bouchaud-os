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

- ~~Les 43,4 s entre le retour du `fork` et le debut de l'`execve`~~ :
  REFUTE. Ce segment etait mal borne et n'existe pas. Mesure : fork 28 ms,
  fork -> exec 8 ms, execve 22 ms.
- `first_user_instruction_ms` et `spawn_request_ms` ne sont pas instrumentes.
- `unexplained_ms` vaut 78 % sur la decomposition du premier worker.
- `HOST_IMAGES_OK codecs=11/11` n'est jamais atteint sous Ladybird alors qu'il
  l'est sous Chromium headless. Un codec echoue, ou la fenetre de mesure se
  ferme avant.

## 8. Le partage utilisateur/noyau etait faux (BOUCHAUD_C66)

**CAUSE CONFIRMEE, et c'est un defaut d'INSTRUMENT, pas de performance.**

`account_kernel_enter` / `account_kernel_exit` n'etaient appelees que depuis
`usermode.rs`, autour du corps d'un appel systeme. `exceptions.rs` n'en
contenait aucune. Tout ce que fait le gestionnaire de faute de page --
attendre le cache de pages, declencher et attendre une lecture ATA, recopier
la page, poser la traduction -- se deroulait avec `COMPTA_EN_NOYAU` reste a
`false`, et tombait donc dans `user_ns`.

### Ce que cela a coute

Le releve `user_ms=92250 sys_ms=671` du segment `exec_fin -> main` a ete lu
comme « limite par le CPU en espace utilisateur, donc ce n'est pas de
l'attente disque ». Cette lecture a oriente toute une campagne vers les
relocations de demarrage de la glibc. Elle ne tenait pas : la seule chose que
`sys_ms` mesurait, c'etait le corps des appels systeme.

### La mesure

Banc `run_faute_fichier.sh`, trois executions par variante, meme arbre, seule
la frontiere de comptabilite change. Compteurs REPLIES (`CUMUL_*`, sans
tranche en vol) ; `vue_*` et `replie_*` coincidaient sur les six releves, donc
aucune attribution en vol ne contamine le resultat.

| variante | `user_ms` min/med/max | `noyau_ms` min/med/max | total med |
|----------|----------------------|------------------------|-----------|
| avant    | 1319 / **1320** / 1322 | 13 / **14** / 14     | 1334 |
| apres    | 56 / **61** / 74     | 1174 / **1204** / 1278 | 1260 |

**94 % de ce qui etait appele « temps CPU utilisateur » etait du traitement de
faute de page.** Le total est conserve : le temps a change d'etiquette, il n'a
pas change de valeur. **Rien n'est devenu plus rapide.**

### Ce que cela ne dit pas

Cela ne dit PAS ou va le temps du segment `exec_fin -> main` de Ladybird. Cela
dit que la mesure qui servait a le trancher ne le tranchait pas. Le partage
honnete demande un nouveau run.

Garde-fou : `tools/ci/verifie-compta-faute.py`, avec trois tests negatifs
(frontiere retiree, sortie retiree, sortie qui ecrase au lieu de restaurer).

## 9. La variante ET_EXEC est REFUTEE (BOUCHAUD_C67)

L'experience devait supprimer entierement `_dl_relocate_static_pie` en liant
WebWorker en statique non-PIE. Elle ne peut pas exister.

Le commentaire qui la justifiait affirmait : « verifie sur un micro-binaire :
lie a `0x400000000000` il s'execute ». C'etait vrai et cela ne prouvait rien :
le micro-binaire etait lie en `-nostdlib`. Il montrait que le CHARGEUR de
Bouchaud accepte cette adresse, pas que la glibc peut y etre liee.

Mesure locale, gcc 13 / binutils Ubuntu :

```
$ gcc -static -no-pie -Wl,-Ttext-segment=0x400000000000 mini.c
crt1.o: in function `_start':
failed to convert GOTPCREL relocation against 'main'; relink with --no-relax

$ ... -Wl,--no-relax
libc.a(printf_buffer_flush.o): relocation truncated to fit:
R_X86_64_PLT32 against undefined symbol `__printf_buffer_flush_obstack'
libgcc_eh.a(unwind-dw2-fde-dip.o): relocation truncated to fit:
R_X86_64_PLT32 against undefined symbol `pthread_cond_wait'
```

Le mecanisme est structurel : la glibc statique reference des symboles faibles
indefinis qui se resolvent a l'adresse zero. Un deplacement relatif de
`0x400000000000` vers `0` ne tient pas dans les trente-deux bits d'un
`R_X86_64_PLT32`. Un `-static-pie` y echappe parce qu'il est LIE a la base
zero : c'est le chargement, pas l'edition de liens, qui le deplace. Aucun
choix d'adresse haute ne contourne cela.

### Ce qui la remplace : RELR

`-Wl,-z,pack-relative-relocs`. Temoin local de vingt mille pointeurs :

| lien | `RELASZ` | `RELACOUNT` | `RELRSZ` |
|------|---------:|------------:|---------:|
| defaut | 26 280 | 1 095 | -- |
| RELR   | 0      | --    | 288 |

Quatre-vingt-onze fois moins, et les deux binaires s'executent. RELR conserve
la relocalisation, donc l'ASLR -- contrairement a ET_EXEC.

**Ce que RELR ne fait pas, et il faut le dire** : il reduit ce qu'il faut
LIRE, pas ce qu'il faut ECRIRE. Les 405 396 ecritures restent. L'experience
discrimine donc entre deux couts qu'on confondait : parcourir 9,7 Mio de table
(donc des fautes de page) et appliquer les relocations.

**Elle n'est pas lancee pour l'instant.** Elle n'a de sens qu'apres le partage
utilisateur/noyau honnete de la section 8 : tant qu'il n'est pas remesure,
rien ne dit que le cout est en espace utilisateur.

Garde-fou : `tools/ci/verifie-bascule-relr.py` verifie la bascule sur un arbre
Ladybird factice portant les motifs d'ancrage amont -- la boucle courte qui
manquait au run 35920701144, ou une experience entiere a ete perdue sur un
chemin de source suppose au lieu d'etre verifie.

## 10. Le run #355 tranche : le cout est dans le NOYAU (BOUCHAUD_C68)

`run_id=35944547625`, `head_sha=f06aebb0e81b02cdf008f91988a521b111de3cbf`,
verifie identique a `HEAD_TESTE`. Premier run avec la comptabilite corrigee de
la section 8.

### Le partage honnete, par processus

`FAULT_FILE_SNAPSHOT` porte `user_ms`/`sys_ms` issus de
`proc_processus_cumul(pid)` -- verifie per-process (`task.process.pid != pid`
saute l'entree), et `temps_vivant` exige `on_cpu >= 0`, donc une tache bloquee
ne contribue rien : ce sont des durees de CPU repliees, pas de l'attente.

| pid | image | `user_ms` | `sys_ms` | fautes (ms) |
|---|---|---:|---:|---:|
| 16 | **WebWorker #1 (froid)** | **724** | **113 594** | 13 285 |
| 15 | WebContent | 6 544 | 160 508 | 22 924 |
| 14 | Compositor | 2 085 | 54 903 | 11 813 |
| 13 | ImageDecoder | 383 | 17 999 | 3 157 |
| 12 | RequestServer | 995 | 17 687 | 3 603 |
| 17 | WebWorker #2 | 267 | 7 956 | 1 929 |
| 18 | WebWorker #3 | 205 | 799 | 125 |
| 19 | WebWorker #4 | 133 | 340 | 81 |

### `HYPOTHESE RELOCATIONS` → **REFUTEE**

`_dl_relocate_static_pie` s'execute en espace UTILISATEUR. Toute la vie
utilisateur du premier WebWorker tient dans **724 ms**. Les 405 396
relocations ne peuvent pas couter davantage, quel que soit leur nombre.

L'ancien relevé `user_ms=92250 sys_ms=671` avait les deux valeurs
essentiellement INVERSEES. C'est cette inversion qui avait envoye la campagne
vers l'editeur de liens.

Consequence : le job CI `variante-elf` est retire. Vingt-cinq minutes de CI
pour un effet borne par 724 ms sur un probleme de 113 s serait du gaspillage,
et un job rouge pour une experience qu'on a decide de ne pas mener polluerait
le signal. La bascule RELR et son garde-fou restent dans l'arbre.

### Ce qui reste a expliquer

Pour le premier WebWorker : **113 594 ms de noyau, dont 13 285 ms de fautes**.
Cent secondes de temps noyau ne sont attribuees a rien. Le rapport est le meme
sur tous les services (`sys_ms` vaut cinq a huit fois leur temps de faute),
donc le mecanisme est systematique, pas accidentel.

Deux candidats poses puis **REFUTES** sur banc local, en quelques minutes :

| candidat | sonde | mesure | verdict |
|---|---|---|---|
| balayage de secours du cache (`retire_un_candidat`, O(n) sous verrou global, 52 537 entrees) | `CACHE_BALAYAGE` | `appels=0` | jamais atteint |
| chaine de reprise des fautes (`FaultOutcome::Retry`, non comptee au livre) | `FAULT_REPRISE` | `reprises=0 chaines=0` | aucune reprise |

Candidat restant : le **corps des appels systeme**. `SYSCALL_TEMPS` le decoupe
desormais par numero. Le banc local ne peut PAS trancher -- `gros-elf` est lie
en `-nostdlib` et n'emet que quatre appels systeme en tout. Seul un run
Ladybird le dira.

Limite a garder en tete : sur le banc local, `CPU_CUMUL` est cumulatif depuis
l'amorcage et les fautes des processus qui ne meurent pas (shell, init) ne
sont dans aucune somme. L'ecart local n'est donc pas comparable a l'ecart
Ladybird, qui est per-process des deux cotes.

