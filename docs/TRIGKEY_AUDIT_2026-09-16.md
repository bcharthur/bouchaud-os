# Audit TRIGKEY — archive blackbox du 16 septembre 2026

Session `2026-09-16_17-00-37_520`, extraite a 17:22. Ce document dit ce que
l'archive contient, ce qu'elle ne contient pas, et ce que chaque constat a
produit comme correction.

## Ce que l'archive couvre

| Mesure | Valeur |
|---|---|
| Fenetre enregistree | **1,07 s → 7,30 s** |
| Duree reelle de la session | ~20 minutes |
| Enregistrements valides | 53 sur 8192 emplacements |
| `fatal.log` | vide |
| Marque de FIN | absente |
| Derniers compteurs | `bb_writes=47 bb_failures=0 bb_busy_skips=0 bb_filets=0` |

**Ces zeros ne disent pas que tout allait bien.** Ils disent qu'a 7,047 s tout
allait *encore* bien. L'enregistrement s'arrete a l'instant precis ou les
services Ladybird demarrent.

## Les constats, et ce qu'ils ont produit

### 1. Le clavier et l'enregistreur etaient affames par la meme cause

Trois consommateurs se partagent `RUNTIME_BUSY`, l'unique verrou du pilote
xHCI : la scrutation HID, l'enregistreur de vol, le systeme de fichiers. Les
deux premiers renoncent si le verrou est pris ; le troisieme attend.

Le navigateur lit quatre cents mebioctets sur la cle d'amorcage — qui est
aussi la cible de l'enregistreur. Le systeme de fichiers tient donc le verrou
presque en continu.

D'ou, le meme jour : « le clavier est deconnecte des que je tape dans
Ladybird » **et** l'archive tronquee. Ni panne ni deconnexion : une famine.
Corrige par `drivers::equite_pilote` — cession bornee du systeme de fichiers.

### 2. Le navigateur partait sans resolveur

```
17:00:38  BOUCHAUD_NET_LIEN etat=UP          <- le lien monte
17:00:39  BOUCHAUD_NAVIGATEUR_RESEAU dns=10.0.2.3 resolveur=NON-CONFIGURE
```

Le lancement ne dependait que du temps (500 ms apres la premiere trame) ;
l'autonegociation cuivre prend ~3 s. Le navigateur partait donc avant tout
bail DHCP, avec `10.0.2.3` — le resolveur du NAT de QEMU, qui ne mene nulle
part sur la machine. Il lit son resolveur **une seule fois, a l'exec**.

Corrige par `net::resolveur` (bail → passerelle → valeur compilee) et
`gui::demarrage_navigateur` (attente bornee du resolveur).

### 3. Le « B » bleu venait du prechargeur, pas du noyau

L'ecran de progression du noyau fonctionnait depuis le 15 septembre. Il
s'ouvre a `stage2::run`, donc **apres** le prechargeur UEFI — qui peignait le
logo et le laissait fige pendant toute la phase de chargement. Chercher cote
noyau ne pouvait rien donner.

### 4. Un compteur qui fait passer quinze coeurs pour morts

L'echantillon par coeur affiche `enters=0 exits=0 rip_u=0x0` sur cpu1..cpu15.
`TIMER_ENTERS` n'est alimente que par le handler d'IRQ0 — le PIT, que **seul
le processeur d'amorcage recoit**. Le meme demarrage imprime pourtant
`BOUCHAUD_SMP_BATTEMENT cpu=0..15 bat=1`.

La ligne porte desormais `quantums=`, seul compteur de cette ligne qui
distingue un coeur en ligne d'un coeur qui participe.

## Ce que l'archive ne permet PAS de conclure

- **La repartition reelle de Ladybird sur les coeurs.** La fenetre
  enregistree couvre 0,5 s d'activite du navigateur ; c'est trop court, et le
  seul compteur par coeur qu'elle portait etait celui decrit au point 4.
- **La cause du ralentissement general.** Aucune mesure au-dela de 7,3 s.
- **Le comportement a l'extinction.** Aucun enregistrement.

Les trois demandent une nouvelle archive, avec l'enregistreur corrige.

## Le temps d'amorcage, en revanche, est mesure

```
kernel-main      t=0
sysroot          t=1312  (+1258)   <- le second poste
reseau           t=5170  (+3003)   <- le premier : autonegociation cuivre
bureau           t=5476
navigateur-lance t=6814
```

**6,8 s du noyau au navigateur.** Les deux postes dominants sont l'attente du
lien Ethernet et le montage du sysroot. Le temps percu par l'utilisateur
inclut en plus le firmware et `BOUCHAUD-LOADER.EFI`, qui ne sont pas dans
cette mesure.

## Un point d'outillage a regler

`extract-blackbox.py` — l'outil qui produit ces archives — **n'est pas dans le
depot**. Il vit dans un dossier d'archive date sur la machine de
l'utilisateur (`docs/historique/notes/blackbox-utilisation.txt`). Tant qu'il
en est ainsi, aucun genre d'enregistrement nouveau ne peut etre ajoute sans
risquer de perdre sa charge utile, et la chaine de diagnostic depend d'un
fichier non versionne.
