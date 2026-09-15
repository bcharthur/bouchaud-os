# Audit materiel Trigkey -- releve du 15 septembre 2026

Source : `blackbox-extract-20260915-190540.zip`, session `20260915185504640`,
529 enregistrements, **zero fatal**, 60 secondes couvertes.

Cet audit repond a une question posee telle quelle : « le CPU semble coherent
et fonctionnel, j'aimerais que la memoire et le disque le soient aussi. Que les
inputs physiques soient fonctionnels aussi. J'ai le sentiment qu'ils ne le sont
pas, alors audite. »

La reponse courte : **en partie tort, en partie raison**. Les bases marchent ;
trois defauts reels sont nommes plus bas, avec le chiffre qui les prouve.

---

## Ce qui est prouve par ce releve

### La double faute d'amorcage est reparee

```
SMP_HANDOFF_BEFORE_STI pic_maitre=0xfe pic_esclave=0xff if=0
    scheduler_enabled=0 bootstrap=0 lapic_svr=0x1ff cpus_en_ligne=16
SMP_HANDOFF_AFTER_FIRST_IRQ vector=0x20 rip=0x80013d86e3 rsp=0x10000014d50
    cpu=0 scheduler_enabled=0 bootstrap=0
```

La premiere interruption apres la frontiere est IRQ0, livree avec
`scheduler_enabled=0` : exactement la fenetre que `fix(timer)` ferme. Le
`rsp=0x10000014d50` est **le meme** que celui de la session qui plantait -- la
preuve definitive qu'il s'agissait d'une pile d'amorcage normale et non d'une
corruption.

### L'hypothese des portes IDT absentes est ECARTEE pour cette machine

```
[IRQ-IMPREVUES] pic_irq7_spurious=0 pic_irq15_spurious=0 lapic_spurious=0
                idt_absentes=0
```

Aucun parasite, aucune porte manquante. Le durcissement reste correct -- une
porte absente ne doit jamais se lire « DOUBLE FAULT » -- mais il n'etait pas la
cause.

### CPU, memoire virtuelle, reseau de base

| Sous-systeme | Chiffre | Verdict |
|---|---|---|
| SMP | `SMP4_AP_STARTED count=15 expected=15`, 16/16 en ligne | fonctionnel |
| Fautes de page | `fault_resolved=8483 fault_invalid=0 fault_io_error=0` | fonctionnel |
| Lien Ethernet | `vitesse_mbps=1000 duplex=complet ip=192.168.1.97` | fonctionnel |
| DHCP | bail obtenu, `nom=lan`, resolveur `192.168.1.254` | fonctionnel |
| NVMe | `lectures=33 erreurs=0 delais=0 hors_service=0` | fonctionnel en lecture |
| Souris USB | `BOUCHAUD_HID_MOUSE_INPUT_GREEN`, 1462 rapports | fonctionnelle |

---

## Les trois defauts reels

### 1. L'arene DMA ne recycle rien

```
[MEM-NG-DMA] total=33542144 utilise=1118208 rendu=32423936
             allocations=65 liberations=16 reutilisations=0 fusions=0
```

Seize liberations, **zero reutilisation, zero fusion**. Chaque region rendue
est perdue pour de bon : l'arene de 32 Mio ne se remplit que dans un sens.

Consequence pratique : chaque branchement a chaud d'un peripherique USB
consomme de la DMA qui ne reviendra jamais. Sur une session courte cela ne se
voit pas ; sur une session longue avec des branchements repetes, l'arene
s'epuise et l'enumeration echoue sans que rien ne l'annonce -- `echecs=0`
aujourd'hui parce qu'on n'a pas encore atteint le fond.

**Non corrige dans ce lot.** C'est une reecriture d'allocateur qui demande ses
propres tests.

### 2. Le disque ne recoit jamais rien

```
[NVME]     lectures=33 ecritures=0 vidanges=0
[BLOC-NG]  soumissions=33 achevements=33 lus=33 ecrits=0 vidanges=0
[BACKING-CACHE] reads=0 bytes=0 clean_hit=8801 clean_miss=23177
```

Le NVMe repond, sans une seule erreur -- mais **rien n'y est jamais ecrit**, et
le cache de pages ne lit **jamais** la couche bloc : les 32 000 acces viennent
tous du disque memoire. Le systeme tourne entierement en RAM.

Ce n'est pas une panne du pilote : c'est que la persistance est differee
(`BOUCHAUD_NVME_PERSISTENCE_DEFERRED`) et n'aboutit pas. Rien de ce qui est
fait pendant une session ne survit a l'extinction.

**Non corrige dans ce lot.**

### 3. Sept refus d'allocation de pages

```
[MEM-NG-TAS-PAGES] servies=29569 rendues=18394 en_service=11175
                   refus=7 libres=1201528 plus_grand_contigu=4194304
[MEM-NG-BACKING]   grandes_pages=17360 grandes_repli=7 oom=0
```

Sept refus avec 1,2 million de pages libres et 4 Mio contigus disponibles. Les
sept `grandes_repli` correspondent : une demande de grande page a echoue sept
fois et est retombee sur le chemin ordinaire. Sans gravite immediate, mais un
refus avec autant de memoire libre demande une explication.

---

## Le reseau est configure et INUTILISE

```
[NET-ROUTAGE] trames=38 arp=4 dhcp=2 arp_resolus=0 arp_echoues=0
```

Trente-huit trames en soixante secondes, et `arp_resolus=0` **avec**
`arp_echoues=0` : aucune resolution n'a meme ete TENTEE. Aucun paquet unicast
n'est jamais parti.

Ce n'est donc pas un defaut reseau : c'est que le navigateur n'est jamais alle
jusqu'a une requete. Conclure « le reseau ne marche pas » a partir de ce
releve serait une erreur de lecture.

---

## L'entree : la souris marche, le clavier non

```
events=1463 reports=1463 mouse=1463        <- TOUT vient de la souris
[GUI-COMPOSITOR-SOURCES] clavier=0 souris=1462
[GUI-INPUT] touches_client=0 touches_bureau=0
```

Deux claviers enumeres, zero touche. Sur le MEME recepteur Logitech, le point
souris (dci 5) fonctionne et le point clavier (dci 3) non.

Le diagnostic d'alors affichait pourtant `BOUCHAUD_HID_CONTROL_FALLBACK_GREEN
kind=keyboard`. Ce vert etait faux, et sa cause est corrigee dans
`fix(usb): stop reporting a mute keyboard as green` : « accepte » voulait dire
« analyse sans erreur », et un clavier au repos rend un rapport valide et vide.

La cause du silence n'est PAS encore etablie. Le releve suivant la nommera :
`[USB-HID-POINT]` sort desormais un etat par point de terminaison.

---

## Performance : ou part le temps

```
WebContent   cpu_pct=76  ctx_delta=4     <- 76 % d'un coeur, 4 commutations
Compositor   cpu_pct=35  ctx_delta=83
usb-repli    cpu_pct=12  ctx_delta=12
desktop      cpu_pct=5
```

`WebContent` a 76 % avec **quatre** commutations de contexte est la signature
d'une attente active : le processus ne travaille pas, il tourne en rond sur
quelque chose qui n'arrive jamais. C'est la premiere piste a suivre pour la
lenteur de Ladybird, avant toute optimisation de rendu.

---

## Ce que le prochain releve apportera

Les lots de ce jour ajoutent :

- `BOUCHAUD_BOOT_POINT <nom> t_ms= delta_ms=` -- ou part le temps d'amorcage ;
- `[USB-HID-POINT]` -- un etat par point de terminaison HID ;
- `[USB-HID-TEMOINS]` -- les temoins du clavier, seul signe visible que le
  chemin de controle atteint l'interface ;
- `[IRQ-IMPREVUES]` -- parasites PIC/LAPIC et portes absentes.
