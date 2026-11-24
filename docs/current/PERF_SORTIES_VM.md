# Performances : les sorties de VM (24/11/2026)

Ce document ne contient que des mesures. Niveau de preuve : **TCG** (QEMU 8.2.2,
4 vCPU, 8 Gio, conteneur sans KVM ni GPU). Rien ici n'a été mesuré sous KVM,
WHPX ou sur le Trigkey : la CI KVM de la PR et le banc Windows le diront.

## La méthode : compter ce qui coûte une sortie de VM

Sous KVM et surtout sous WHPX, chaque accès à un port d'E/S ou à un registre
MMIO émulé interrompt la VM et passe par l'hyperviseur, puis souvent par QEMU.
TCG ne paie pas ce prix : il cache le problème. Mais il permet de le
**compter** exactement, avec la trace QEMU `memory_region_ops_read/write` (une
ligne par accès, nommée par périphérique) :

```
python3 tools/perf/campagne_ab.py --bras main=boot-main.bin --bras cand=boot-cand.bin \
    --ladybird DOSSIER_ARTEFACT --paires 3 --accel tcg --sortie out/ [--io]
```

`--io` compte les accès avant le noyau (BIOS + chargeur) et après (frontière :
premier octet écrit par le noyau sur COM1). Les passages `--io` ralentissent
l'invité : ils ne servent qu'au comptage, jamais aux temps. Sans `--io`, la
campagne alterne les bras (A B, B A, ...) et exporte JSON, CSV et un résumé.
L'historique est dans [`docs/perf/campagnes/`](../perf/campagnes/).

`tools/perf/echantillonneur_qemu.py` donne en plus un profil RIP de tous les
vCPU vu de l'extérieur (le profil `[PERF-RIP]` du noyau ne voit pas le code à
IF=0). Limite : il prend le verrou global de QEMU et sur-représente donc les
instructions d'E/S ; il sert à localiser, pas à chiffrer.

## Ce qui a été trouvé (smoke Ladybird réel, `main@e18a954`)

10,5 millions d'accès d'E/S émulés en une session de ~60 s :

| Origine | Accès | Cause |
|---|---:|---|
| Chargeur BIOS (avant le noyau) | 7 260 942 | `bootloader` 0.9 lit 13 Mio **un secteur par `int 0x13`** : 25 337 commandes ATA PIO, leurs attentes d'état, 220 000 lectures du PM timer par SeaBIOS (et 6,5 M mots de données PIO) |
| ATA, attente de fin de DMA | 944 384 | boucle sur le statut bus-master, ~170 lectures par `READ DMA` |
| ATA, `FLUSH CACHE` | ~500 000–710 000 | 10 756 lectures du statut par vidange |
| Journal série | 965 093 | un octet = une écriture COM1 ; 48 % des octets étaient le même préfixe de métriques |
| e1000, registre `RDH` | 315 174 | instantané de diagnostic pris à chaque drainage, jeté hors fenêtre de reprise |
| Config PCI | 99 072 | 256 bus × 32 emplacements rebalayés par chaque `find_*` |

## Les correctifs

| PR | Changement | Accès avant → après |
|---|---|---:|
| perf (noyau) | ATA : `READ/WRITE DMA` et `FLUSH CACHE` attendent l'IRQ14 (rotation espacée 200 µs, puis sommeil borné à 1 ms, repli si aucune IRQ) | bus-master 944 384 → 80 854 ; état ATA 496 368 → 113 212 |
| perf (noyau) | Journal série : le bloc de métriques n'est répété que s'il change (`[=]` sinon, format des analyseurs conservé) | 965 093 → 636 912 octets |
| perf (noyau) | Réseau : instantané `RDH` pris seulement quand un verdict de reprise est dû | 315 174 → 4 072 |
| démarrage | PCI : inventaire gardé après le premier balayage | 99 072 → 16 482 |
| démarrage | Chargeur : noyau lu par lots de 64 secteurs (bootloader 0.9.34 vendu, une modification) | 25 337 → ~400 commandes ; PM timer 220 003 → 70 886 |

Côté noyau, total : **3 238 457 → 1 280 713 accès (−60 %)** sur la même session.

## Les temps (TCG, 3 paires alternées, même artefact Ladybird)

`main` = `e18a954` ; `c2b` = correctifs noyau ; `c3` = noyau + démarrage.
Médiane [min–max] sur 3 démarrages à froid par bras
([données](../perf/campagnes/2026-11-24-tcg-main-c2b-c3.json)).

| Mesure | main | c2b | c3 |
|---|---:|---:|---:|
| QEMU → 1re ligne du noyau (hôte) | 3 685 [3 643–3 868] | 3 727 [3 623–3 790] | **2 059** [1 996–2 117] |
| QEMU → bureau (hôte) | 9 578 [9 497–9 976] | 8 285 [8 097–8 298] | **6 530** [6 406–6 855] |
| Noyau → bureau (horloge invitée) | 5 930 [5 802–6 092] | 4 473 [4 463–4 569] | 4 465 [4 408–4 731] |
| QEMU → 1re trame Ladybird (hôte) | 12 235 [12 224–12 726] | 12 253 [12 252–12 556] | **10 832** [10 772–11 523] |
| QEMU → page chargée (hôte) | 13 727 [13 474–14 198] | 13 745 [13 623–13 952] | **12 283** [12 243–13 017] |
| Dessin CPU du bureau par trame (µs) | 34 347 [31 967–34 356] | 17 248 [17 021–17 577] | 15 457 |
| Trames du bureau / 30 s | 110 [104–115] | 167 [164–175] | 179 |
| Présentation Ladybird, pire cas (µs) | 30 480 | 30 940 | 26 388 |
| Lancement Ladybird → 1re image (invité, ms) | 4 136 [3 991–4 182] | 4 213 [4 117–4 216] | 4 317 [4 197–4 718] |
| Journal série (octets) | 893 385 | 572 711 | 594 753 |

Lecture :

- **Démarrage** : −3,0 s du lancement de QEMU au bureau (−32 %) sous TCG. Le
  gain du chargeur (−1,6 s avant la première ligne du noyau) est le plus
  grand ; sous WHPX, où chaque accès PIO sort de la VM, il devrait être
  plus grand encore — **non mesuré**.
- **Bureau** : le dessin CPU par trame est divisé par deux et le bureau
  compose 50 % de trames en plus pendant le démarrage de Ladybird. Les cœurs
  ne tournent plus sur des registres disque.
- **Lancement de Ladybird** : **pas de gain**, +2 % (c2b) à +4 % (c3) en
  médiane, plages qui se recouvrent. Une première version (rotation de
  10 µs) coûtait +6 % : le réveil par IRQ ajoutait ~117 µs à chacune des
  ~5 300 lectures disque des fautes de pages. La rotation espacée a réduit
  l'écart sans l'annuler. **Non résolu** : la latence de réveil
  (PIC → BSP → `wake_wait_queue`, qui parcourt tout le registre des tâches →
  IPI) reste le prochain chantier.

## Stabilité

- Smoke BrowserHost fonctionnel (passages des campagnes) : `main` 12/12, `c2`/`c2b` 7/7, `c3` 8/9.
- L'échec `c3` (campagne 2, passage 1) : `LADYBIRD_FUNCTIONAL_SMOKE fail
  raison=audio maillon=aucune_ecriture` — le clic injecté après
  `HOST_AUDIO_PRET` n'a pas lancé la lecture ; l'événement molette suivant
  est transmis par le WM sans être reçu par le BrowserHost. Non reproduit
  sur les 7 passages `c3` suivants (dont 4/4 dans une campagne dédiée,
  [données](../perf/campagnes/2026-11-24-tcg-stabilite-main-c3.json)).
  **Cause non établie** ; à surveiller dans la CI de la PR.
- Garde-fous d'architecture : 216/216 sur les deux branches.

## Ce qui n'est pas mesuré

| Sujet | État |
|---|---|
| KVM, WHPX, Trigkey | non mesurés ici (pas de `/dev/kvm`, pas de Windows) ; la CI KVM de la PR qualifie le fonctionnement, `campagne_ab.py --accel kvm` mesure |
| Latence entrée → affichage, FPS de défilement | non instrumentés au-delà du smoke |
| GPU | aucun GPU ni `/dev/dri` dans ce conteneur : rien de ce document ne concerne l'accélération graphique |
