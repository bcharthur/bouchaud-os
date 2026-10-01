# RTL8168 : l'arret au premier bouclage de l'anneau RX

BOUCHAUD_RTL8168_PREMIER_BOUCLAGE_V1 -- enquete ouverte le 1er octobre 2026,
releve physique TRIGKEY (RTL8168h, `xid=0x541`) sur `04fef512`.

Toutes les preuves de ce document sont **physiques** (telemetrie UDP du
TRIGKEY). QEMU n'emule pas de RTL8168 : aucun banc CI n'exerce ce pilote.

## 1. Le fait

Deuxieme demarrage du releve, extraits (champs choisis, valeurs converties en hexadecimal) :

```
[ 5.439] remote BRDP_ECOUTE port=2222                 rtl_rx_packets=8
[ 8.859] rtl8168 RING_WRAP_BEFORE rx_cur=63 rx_paquets=63 desc63_opts1=0xc0000800 desc0_opts1=0x80000800
[ 8.859] rtl8168 RX_REGISTRES chip_cmd=0x0c intr_status=0x0001 rx_config=0x0002cf0e cplus_cmd=0x2021
[ 8.859] rtl8168 RING_WRAP_AFTER rx_cur=0
[10.495] heartbeat rtl_rx_packets=64 rtl_isr_rx_ok=34 rtl_rx_sterile=1 rtl_rep_req=0
[12.499] audit AUDIT_SECOND_TOUR_ABSENT rendus_tour1=64 rendus_tour2=0 attente_ns=3048178991
[12.499] rtl8168 RX_OWN_MAP map_own=0xffffffffffffffff materiel=64 processeur=0 rx_cur=0
[12.499] rtl8168 RX_DESC index=0 opts1=0x80000800 opts2=0 addr=0x36e105000      (identique a 8.859)
[12.499] rtl8168 RX_DESC_ADDR_RELU relu=0x36e299000 attendu=0x36e299000 concorde=oui
[12.499] audit AUDIT_RX_DMA_STALL rx_paquets=64 isr_delta=20 desc_cpu=0
[15.563] heartbeat rtl_rx_packets=64 rtl_isr_rx_ok=65 rtl_rx_sterile=6 rtl_rep_req=2 rtl_rep_exec=2 rtl_rep_sans_effet=1 rtl_rep_degre=0
[20.618] heartbeat rtl_rx_packets=67 rtl_rx_cur=3 rtl_isr_rx_ok=78 rtl_rx_sterile=9 rtl_rep_req=3 rtl_rep_exec=3 rtl_rep_sans_effet=2 rtl_rep_degre=3 rtl_reinit=0
[25.678] heartbeat rtl_rx_packets=73 rtl_rx_cur=9 rtl_rep_sans_effet=0
[45.906] heartbeat rtl_rx_packets=74 rtl_isr_rx_ok=85 rtl_rx_sterile=9
[66.056] heartbeat rtl_rx_packets=74 rtl_isr_rx_ok=85 rtl_rx_sterile=9
[69.621] rtl8168 RING_WRAP_BEFORE rx_cur=63 rx_paquets=127 ...  RX_PROGRES rendus_tour2=64 tours_cpu=2
[71.071] heartbeat rtl_rx_packets=2157 rtl_rx_tours=33
```

## 2. La chronologie corrigee : la reprise a lieu vers 18,5 s, pas a 69 s

| instant | fait | preuve |
|---|---|---|
| 8,859 s | bouclage 63 -> 0 ; les 64 descripteurs sont au materiel | `RING_WRAP_*`, `RX_OWN_MAP` |
| 8,9 -> ~18,5 s | **arret** : aucun descripteur rendu, `RxOK` monte (34 -> 78) | `rtl_rx_sterile` 1 -> 9 |
| ~12,5 s | reprise n°1, **degre 0** (Draine) | `rep_exec=1` puis `sans_effet=1` |
| ~15,5 s | verdict negatif, reprise n°2, **degre 0** | `rep_req=2 rep_exec=2 rep_degre=0` a 15,563 |
| ~18,5 s (deduit de la fenetre de verdict de 3 s ; observe entre 15,6 et 20,6 s) | verdict negatif, reprise n°3, **degre 3** (ReconstruitAnneau) | `rep_req=3 rep_degre=3` a 20,618 |
| <= 20,6 s | **le second tour demarre** : `rx_cur` 0 -> 3, `rx_packets` 64 -> 67 | heartbeat 20,618 |
| ~21,5 s | verdict **effectif** | `rep_sans_effet=0` a 25,678 |
| 25 -> 66 s | lien calme : la CARTE elle-meme ne signale qu'une trame | `isr_rx_ok` 84 -> 85, `rx_sterile` reste 9 |
| 69,6 s | fin du **second tour** (capture du bouclage n°2), reprise du trafic | `rendus_tour2=64`, 2157 trames a 71 s |

La lecture « le second tour repart a 69 s » confond la capture du DEUXIEME
bouclage (`tour <= 2` declenche une capture complete) avec la reprise.
`rendus_tour2=64` a 69,6 s dit que le second tour est **termine**, donc
commence bien avant ; `rtl_rx_cur=3` a 20,618 s dit quand. Entre 25 et 66 s,
`isr_rx_ok` ne bouge que d'une unite : ce n'est pas un arret (la carte ne
recoit rien), et `rtl_rx_sterile` ne monte plus.

**Duree reelle de l'arret : ~9,7 s**, dont ~6 s perdues par deux reprises de
degre 0 qui, dans cet etat, ne peuvent rien (voir 3).

Premier demarrage du releve : la telemetrie ne commence qu'a t=400 s, mais ses
compteurs sont identiques (`rx_sterile=9 rep_req=3 rep_exec=3 rep_degre=3
sans_effet=0 reinit=0`). La meme sequence s'y est tres probablement produite --
c'est une deduction, pas une observation. Bilan : **2 demarrages sur 2** sur
`04fef512`, et le releve du 18 septembre (avant l'escalade) montrait le meme
arret a 64, permanent.

## 3. L'echelle de reprise, degre par degre

Entrees (toutes ne font que POSER un drapeau, sauf la derniere) :

* `maintenance_anneau_vide` : `ChipCmd` dit `RxEnb=0` ou `RxBufEmpty` (raison 1) ;
* `demande_reparation_si_arretee` (veilleur) : lien haut, emission apres la
  derniere reception, silence >= 3 s, emission de moins de 30 s, pas de reprise
  depuis 2 s, ET une preuve `RxOK sans progres` de moins de 5 s (raison 2) ;
* escalade apres un verdict negatif, tant que `sans_effet <= 3` (raison 3).

Execution : `repare_si_demande`, depuis le seul drainage verrouille, quand
aucun verdict ne court (fenetre 3 s) et 2 s apres la reprise precedente.
Le degre vient de `anneau_rx::degre` :

| degre | condition d'entree | registres ecrits | anneau | effet attendu | sur la TRIGKEY |
|---|---|---|---|---|---|
| 0 Draine | invariant intact, `sans_effet < 2`, `ChipCmd` sain, `processeur == 0` | `IntrStatus` (acquittement) | rien | lever un statut verrouille | **2 fois, sans effet** (`sans_effet` 0 -> 1 -> 2) |
| 1 Rearme | `processeur != 0` | `IntrStatus` | rend au NIC les descripteurs retenus (trames perdues comptees) | debloquer un anneau plein cote CPU | non atteint (`processeur=0`) |
| 2 RelanceRx | `RxEnb=0` ou `RxBufEmpty` | `ChipCmd = TE|RE` | rien | relancer un moteur tombe | non atteint (`chip_cmd=0x0c`) |
| 3 ReconstruitAnneau | invariant casse, ou `sans_effet >= 2` | `ChipCmd &= ~RE`, `RxDescAddrLow/High`, `ChipCmd = TE|RE` | 64 descripteurs reecrits (valeurs identiques), `RX_CUR = 0` | repartir du descripteur 0 des deux cotes | **efficace** : `rx_cur` 0 -> 3 en < 2,1 s |
| 4 ReinitialiseCarte | `sans_effet >= 3` | tout `programme_le_materiel` + autonegociation | tout | dernier recours, coupe le lien | non atteint (`reinit=0`) |

Ce que cela etablit :

* le degre 0 n'ecrit QUE `IntrStatus`. Sur un anneau dont les 64
  descripteurs sont deja au materiel avec un moteur arme, il ne peut rien
  changer -- deux essais, deux verdicts negatifs, ~6 s ;
* le degre 3 reecrit des descripteurs **identiques** a ceux que les captures
  montrent en memoire (memes `opts1`, memes adresses, invariant intact). Ce
  qui change, c'est donc l'etat INTERNE de la carte : `RxEnb` coupe puis
  remis, et la base RX reecrite ;
* apres ce degre 3, l'anneau boucle des dizaines de fois sans une faute
  (`rx_tours=56`, `rx_trous=0`, `rep_req` reste 3). L'arret est propre au
  **premier** bouclage apres `programme_le_materiel`.

`rtl_reinit=0` est conserve : rien dans ce lot ne s'en approche.

## 4. Ce que le releve ne dit pas, et les sondes qui le diront

Pendant l'arret, `RxOK` est leve ~40 fois alors qu'aucun descripteur ne
revient. Trois histoires, trois pannes differentes :

1. la carte **ecrit** les trames, mais pas dans notre anneau ;
2. la carte les **jette** : elle croit ne disposer d'aucun descripteur ;
3. `RxOK` ne correspond a aucune trame.

`RxMissed` (0x4C) ne tranche pas : sur 8168 ce registre ne compte plus (Linux
ne le lit que jusqu'a `VER_06`). Le « `rx_missed=0` » des releves precedents
ne prouvait donc rien.

Sondes ajoutees (aucune ne touche un descripteur, `ChipCmd`, `RxConfig` ni les
adresses d'anneau ; `tools/verifie-rtl8168-premier-bouclage.py` le garde) :

| evenement LAB | quand | ce qu'il porte |
|---|---|---|
| `RX_COMPTEURS_MAT rx_ok_mat manquees_mat rdu_isr raison` | bouclages 1-2, chaque reprise (avant d'agir), premier retour du second tour | compteurs DTCC de la carte (sequence `rtl8169_do_counters`), et le cumul `RDU` vu dans `IntrStatus`. `raison >= 100` : releve non acheve |
| `RX_HORS_ANNEAU ombre ombre_modifies ombre_trames queue_modifies` | a chaque capture | ecritures depuis le demarrage dans l'« ombre » (base RX privee de sa moitie haute, lue seulement si la carte memoire de boot la declare RAM) et dans les 3 Kio de la page de l'anneau apres `EOR` (que personne n'ecrit) |
| `RX_ADRESSES_INIT rx_desc_avant tx_desc_avant rx_anneau tx_anneau` | a chaque capture | `RxDescAddr`/`TxDescAddr` relus apres la remise a zero et AVANT que le pilote les ecrive |
| `RX_RECOVERY_BEGIN degre sans_effet rx_paquets own_rendus` | chaque reprise, avant d'agir | le degre reel (il ne s'ecrivait que sur COM1, absent sur la TRIGKEY) |
| `RX_RECOVERY_END effective sans_effet age_ns rx_paquets` | chaque verdict | effectif ou non |
| `RX_DESC_ADDR_RELU ... raison` | a chaque capture | la raison de capture, qui etait portee mais pas affichee |

Raisons de capture : 1 bouclage-avant, 2 bouclage-apres, 3 second tour absent,
4 arret DMA, 7 premier retour du second tour, 8 reprise.

### Table de decision (ecarts entre la capture du bouclage 1 et la reprise n°1)

| d `rx_ok_mat` | d `manquees_mat` / d `rdu_isr` | hors anneau | conclusion | correctif qui en decoule |
|---|---|---|---|---|
| > 0 | ~0 | `ombre_trames > 0` ou `queue_modifies > 0` | la carte ecrit HORS de l'anneau : base de bouclage fausse (moitie haute perdue, ou `EOR` ignore). **Corruption memoire possible.** | programmation des registres d'adresse (ordre haut/bas de `rtl_set_rx_tx_desc_registers`), a prouver par un demarrage |
| > 0 | ~0 | rien | ecrit ailleurs, hors des deux zones sondees | elargir la sonde avant tout correctif |
| ~0 | > 0 | rien | la carte croit l'anneau plein alors que les 64 `OWN` sont poses : descripteurs ou pointeur internes perimes | resynchroniser l'etat interne apres la montee du lien (ce que fait le degre 3), a prouver |
| ~0 | ~0 | rien | `RxOK` sans trame derriere | aucun correctif de reception ; revoir la regle de detection |

`rx_desc_avant` dit en plus si la moitie haute etait nulle quand la moitie
basse a ete ecrite (anneau a `0x3_6e29_9000`) -- condition necessaire de la
premiere ligne. Rien n'est conclu sur cette seule hypothese : l'anneau TX est
programme dans le meme ordre et boucle sans faute.

## 5. Protocole physique

Image construite sur le commit qui porte ces sondes, rien d'autre change.

1. Sur le PC, lancer `python .\tools\remote\bouchaud-lab.py telemetry --watch`
   AVANT la mise sous tension, sortie redirigee dans un fichier par demarrage.
2. Trois demarrages **a froid** (alimentation coupee entre deux), cable
   branche comme le 1er octobre, sans lancer Ladybird pendant les 90
   premieres secondes.
3. Rendre, par demarrage : toutes les lignes `rtl8168` et `audit` des 120
   premieres secondes, et les heartbeats de ~20 s et ~60 s.

Systematique ou intermittent : 3/3 -> systematique ; 0/3 -> intermittent, et
on recommence avec le meme protocole avant de toucher au pilote.

## 6. Critere de validation du futur correctif

Sur plusieurs demarrages physiques a froid :

* premier bouclage, puis `RX_COMPTEURS_MAT raison=7` (retour) dans la
  seconde qui suit ;
* aucun `RING_SECOND_LAP_TIMEOUT`, aucun `AUDIT_RX_DMA_STALL` ;
* `rtl_rep_req=0`, `rtl_reinit=0` ;
* trafic durable (`rtl_rx_tours` qui monte, `rtl_rx_trous=0`, `lab_q_lost=0`).

## 7. Ce qui n'est pas modifie

DHCP, DNS, TCP, TLS, la configuration reseau (B12, branche separee), le
scheduler, l'echelle de reprise et ses seuils, `programme_le_materiel` (hors
deux lectures de registres avant ecriture), le format des descripteurs.
