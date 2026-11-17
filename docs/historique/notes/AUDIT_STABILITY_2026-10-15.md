# Audit de stabilité — 15 octobre 2026

> Campagne lue : **`c7018e56`** (HEAD de référence de l'ordre de mission
> SUPER P+). Tout ce qui suit est tiré des journaux des jobs, pas de leur seule
> conclusion. Les correctifs de cette session sont sur la branche
> `claude/ladybird-observability-performance`, au-dessus de `c7018e56` ; leur
> propre campagne CI est à lire (voir « Prochaine campagne »).
>
> Niveaux : HOST · TCG · KVM · PHYSICAL. Aucun lot n'est déclaré terminé ici.

## 1. Campagne `c7018e56` — état réel

| Run | Workflow | Conclusion | Ce qui a été lu |
|---|---|---|---|
| [38645396551](https://github.com/bcharthur/bouchaud-os/actions/runs/38645396551) | CI Fast | succès | gardes, tests hôte |
| [38645396809](https://github.com/bcharthur/bouchaud-os/actions/runs/38645396809) | Integration | succès | — |
| [38645396934](https://github.com/bcharthur/bouchaud-os/actions/runs/38645396934) | Reliability V3 | succès | — |
| [38645396562](https://github.com/bcharthur/bouchaud-os/actions/runs/38645396562) | SCM_RIGHTS | succès | — |
| [38645396730](https://github.com/bcharthur/bouchaud-os/actions/runs/38645396730) | ladybird-native-browser | **échec** (1 pas) | primitives : tous les pas sauf l'A/B IPI ; jobs Ladybird : robustesse, sites réels, WPT, smoke, smoke virtio-gpu, cache/SQL, mémoire, ordre worker, politique de crash, piège Compositor, performance, endurance verts |
| [38645418495](https://github.com/bcharthur/bouchaud-os/actions/runs/38645418495) | Endurance hebdomadaire (30 min/tranche, TCG) | **échec** (SMP8) | SMP1 succès, SMP2 succès, SMP4 succès (**40/40 cycles**, sonde 0 échec à chaque cycle), SMP8 NON_QUALIFIANT au cycle 1 |

### 1.1 os-primitives (job 116130860804, 56 min)

| Pas | TCG | KVM | Chiffres |
|---|---|---|---|
| Console série, 4 démarrages | vert | vert | 600/600, 720/720 (lignes de 1 800 o), 600/600 détenteur suspendu 3 s (6 différées, 6 vidées), 600/600 suspendu **12 s** (30 différées, 30 vidées, 0 perdue) ; 0 émission sans jeton, 0 réentrance |
| P10 transport virtio-PCI | vert | vert | `GET_DISPLAY_INFO` 720 µs (TCG), **71 µs (KVM)** |
| P10 scanout 2D aux pixels | vert | vert | mire 3 721/3 721 ; bureau virtio = bureau BGA 921 412/921 600 (TCG), **921 375/921 600 (KVM)** ; 0 présentation sautée |
| P10 BGA contre virtio-gpu | vert | vert | voir §3 (première mesure KVM de BARRIERE_V1) |
| P10 `ATTACH_BACKING` sans réponse | vert | vert | mémoire conservée `etat=Inconnue` |
| P10 réponse étrangère | vert | vert | `barriere-differente` reçue 19 / attendue 20, file cassée, abandon au 1er échec, compositeur vivant |
| P13 méthode de preuve KVM, SMP1/2/4/8 | — | vert (instrument) | **`ORDONNANCEUR_TEMOIN_INSTRUMENT` aux 4 topologies** : preuve de l'hôte publiée à chaque phase, verdicts explicites, 0 QUALIFIE_ECHEC — **P13 non qualifié** (§2) |
| A/B variante IPI, KVM SMP4 / SMP8 | — | **rouge** | SMP4 : 12/12 passes NON_QUALIFIANT + « 2 IPI de variante sous défaut (tour 4) » ; SMP8 : NON_CONCLUANT (0 tour apparié, vol de l'hôte ≈ 485 ‰) |

## 2. P13 — pourquoi la sonde ne qualifie rien sous KVM

**Constat (KVM, toutes topologies)** : chaque tentative rend NON_QUALIFIANT
« charge sans effet mesurable » : le p95 du retard de réveil **au repos**
(1,9 ms) dépasse celui **sous charge** (1,0–1,2 ms) ; médianes 1,3 ms contre
0,6–0,7 ms. Vol de l'hôte ≈ 0 ‰ (SMP1/2/4), aucun réveil prouvé retardé par
l'hôte.

**Cause dans le noyau (vérifiée au code)** : `nanosleep` lisait la durée par
`timespec_ms`, arrondie au **millième supérieur** ; l'échéance devenait
`maintenant + ms`, puis n'était détectée qu'au tick de 1 ms. Un sommeil de
15,2 ms durait au moins 16 ms, puis jusqu'au tick suivant : ≈ 0,5 ms
d'arrondi + ≈ 0,5 ms de détection en moyenne — la médiane de 1,3 ms au
repos.

**Correctif** `856db7b5` (BOUCHAUD_SOMMEIL_NS_V1) : `nanosleep` et
`clock_nanosleep` à la nanoseconde exacte, POSIX respecté (au moins la durée
demandée ; durée nulle = céder). Mesure locale, même banc, TCG SMP1, 2 passes :
médiane au repos **1 338 / 1 442 µs → 904 / 913 µs**, p95 2 205 / 2 305 →
1 807 / 1 657 µs.

**Ce qui reste** : la détection au tick (≤ 1 ms). Tant que l'échéance n'est
pas servie par un minuteur à l'échéance exacte, le repos garde un retard de
l'ordre du tick et la porte « la charge dégrade la latence » reste fragile
sous KVM. Prochaine étape P13+ : réveil précis (LAPIC TSC-deadline au repos,
multiplexé avec le quantum). **Non écrit à ce jour.**

**SMP8 sur runner à 4 cœurs** : sous TCG, 126/300 réveils retardés sans
explication et aucune preuve de l'hôte → NON_QUALIFIANT (attendu, ni succès
ni échec du noyau) ; sous KVM, vol de l'hôte ≈ 485 ‰. SMP8 n'est
qualifiable que sur un hôte offrant 8 cœurs libres (protocole :
`BO_QEMU_KVM=1 BANC_TEMOIN_INSTRUMENT=1 tools/ci/run_ordonnanceur_attribution.sh IMAGE 2 8`
sur un runner ≥ 8 cœurs, preuve de l'hôte exigée, vol ≤ quelques ‰).

## 3. P10 — première mesure KVM de BARRIERE_V1

Coût d'une présentation (copie + `TRANSFER_TO_HOST_2D` + `RESOURCE_FLUSH`,
chacun avec barrière et réponse jugée), µs :

| Catégorie | BGA moy | virtio moy | BGA p95 | virtio p95 |
|---|---|---|---|---|
| KVM plein 1280×720 | 328 | 399 | 198 | 472 |
| KVM bande 1280×30 | 3 | 102 | 3 | 121 |
| KVM tuile 64×64 | 2 | 43 | 2 | 62 |
| TCG plein | 1 766 | 1 866 | 1 852 | 1 873 |
| TCG bande | 68 | 189 | 85 | 215 |
| TCG tuile | 23 | 79 | 36 | 100 |

640/640 présentations acceptées sur KVM et TCG. Les deux allers-retours
clôturés coûtent ≈ 40–100 µs fixes par présentation sous KVM.

## 4. Défauts trouvés et corrigés pendant la session (au-dessus de `c7018e56`)

| Commit | Défaut | Démonstration | Test |
|---|---|---|---|
| `18dc70b8` | Endurance : les marqueurs de famille (`… && echo X_OK`) étaient trouvés dans l'**écho de la commande** (`+ …`) même quand la famille échouait (faux témoin ; seul `statut=0` rattrapait l'échec) ; un NON_QUALIFIANT était rapporté « noyau : cycle en échec » et arrêtait la campagne à 71 s | `shell::run_batch` imprime `+ ligne` avant d'exécuter ; journal SMP8 38645418495 | 15 tests (mutations : 3 et 1 rouges) ; garde `verifie-tests-fiabilite.py` (7 négatifs) |
| `2238d8b9` | P10 : un second `enter()` recréait la ressource 1 ; QEMU refusait (`0x1203`) et le chemin d'échec faisait UNREF de l'**ancienne** ressource à l'écran (fuite DMA, écran éteint, un démarrage sur deux) | noyau `c7018e56` : banc double ÉCHEC n=2 ; ce commit : 2 × 640 acceptées | `run_banc_presentation.sh` présente deux fois par démarrage |
| `daa5df34` | P10+.1 (fonctionnalité) : attente des réponses par interruption MSI-X | voir `docs/current/P10_PLUS_ROADMAP.md` | 12 tests hôte, 14 tests d'analyse, banc de pannes, garde (10 négatifs) |
| `81570c88` | A/B IPI : le départ du compteur d'un bras était la fin du bras précédent, relevée **sous la variante** avant la bascule | l'ancienne analyse reproduit exactement « 2 IPI sous défaut (tour 4) » sur un journal où les IPI précèdent la bascule ; journal brut du run non lisible ici → **à confirmer** par le prochain run | 3 tests (11) |
| `856db7b5` | P13 : sommeils arrondis au millième supérieur | §2 | garde `verifie-horloge-utilisateur` (règle ns, rougit sous mutation), `verifie-cycle-vie` |

## 5. Ouverts (non résolus, chiffrés)

| Sujet | Mesure | Prochaine action |
|---|---|---|
| Émission série interruptions masquées | Bilan KVM : `emission_masquee_pire_us=60241` (lignes de 1 800 o) et **169 900** (détenteur suspendu 12 s) ; `emissions_masquees_plus_de_2ms` 3 738–4 162 | ces bancs écrivent volontairement depuis des fautes (IF=0) ; décider si une ligne en contexte masqué doit toujours être différée (hors panique) pour borner la section masquée |
| P13 KVM non qualifié | §2 | réveil précis à l'échéance (P13+) puis rejouer SMP1/2/4 KVM |
| SMP8 | §2 | runner ≥ 8 cœurs |
| A/B IPI | V3 à lire | rejouer ; politique par défaut inchangée |
| `persist-course-probe`, `courant.rs:276`, LibJS `Value.cpp:484`, RTL8168 | inchangés | voir registre |

## 6. Prochaine campagne

Pousser les 5 commits, puis lire : CI Fast, Integration, Reliability, SCM,
ladybird-native-browser (primitives : A/B P10+.1 TCG/KVM, pannes P10+.1
TCG/KVM, BGA/virtio double, P13 KVM avec sommeils ns, A/B IPI V3), et une
endurance hebdomadaire (SMP8 doit sortir NON_QUALIFIANT et continuer ses
cycles, jamais vert).

## 7. Campagne `f23a1247` — lue (15 octobre, matin)

| Run | Workflow | Résultat |
|---|---|---|
| [38697040246](https://github.com/bcharthur/bouchaud-os/actions/runs/38697040246) | CI Fast | vert |
| [38697040138](https://github.com/bcharthur/bouchaud-os/actions/runs/38697040138) | Integration | vert |
| [38697040370](https://github.com/bcharthur/bouchaud-os/actions/runs/38697040370) | Reliability V3 | vert |
| [38697040371](https://github.com/bcharthur/bouchaud-os/actions/runs/38697040371) | ladybird-native-browser | **vert** : tous les jobs Ladybird (endurances 10 min, KVM, 20 min KVM, robustesse, sites réels, WPT, smokes BGA et virtio, cache/SQL, mémoire, ordre worker, politique de crash, piège Compositor, performance) et primitives (job 116295952133, tous les pas) |
| [38697057637](https://github.com/bcharthur/bouchaud-os/actions/runs/38697057637) | Endurance 30 min / tranche (TCG) | SMP1 **vert** ; SMP2 **NON_QUALIFIANT** (13 cycles sur 29) ; SMP4 **ÉCHEC** (cycle 11) ; SMP8 **NON_QUALIFIANT** (24/24 cycles, campagne poursuivie comme exigé) |

### 7.1 Primitives (job 116295952133, 59 min)

- **P10+.1, A/B scrutation/interruption** (6 tours AB/BA par accélérateur) :
  - TCG : NON_CONVAINCANT (latences dans les seuils, aucun gain processeur : sous TCG le cœur ne dort jamais pendant l'attente) ;
  - **KVM : CONVAINCANT**, 6/6 tours. p50 plein 358 → 346 µs, bande 125,5 → 115, tuile 72 → 72 ; p95 dans les seuils ; cœur **inactif 201,5 ‰** en plein écran (0 en scrutation), 353,5 ‰ en bande ; **0 achèvement vu sans interruption sur 3 840 présentations**.
  → défaut basculé sur l'interruption (`IRQ_VOULUE` vrai, commentaire et garde citent ce run). La CI rejoue l'A/B à chaque campagne : un KVM NON_CONVAINCANT rend le défaut à la scrutation.
- **Pannes P10+.1** TCG et KVM vertes (MSI-X masqué rattrapé par la relecture, parasites comptés sans progrès, attente aveugle échue une fois, abandon, bureau vivant). `ATTACH_BACKING` sans réponse et réponse étrangère : verts TCG et KVM.
- **BGA/virtio, deux bancs par démarrage** : 640/640 deux fois, `SCANOUT_REPRIS` TCG et KVM (le correctif `2238d8b9` tient en CI). KVM virtio : plein 448 µs (p95 602), bande 128 (164), tuile 78 (113).
- **P13 KVM SMP1/2/4/8 (sommeils à la nanoseconde)** : `ORDONNANCEUR_TEMOIN_INSTRUMENT` aux quatre topologies, 0 QUALIFIE_ECHEC, preuve de l'hôte 18/18 phases. SMP1 : repos médiane 410–500 µs, p95 ≈ 970 µs ; **sous charge normale médiane ≈ 215 µs, p95 ≈ 270 µs**. La charge *raccourcit* le retard : sous charge, chaque point d'ordonnancement balaie les échéances, au repos seul le tic (1 ms) le fait. C'est exactement la cible de P13+ (réveil à l'échéance) ; P13 **non qualifié**.
- **A/B IPI V3** : NON_CONCLUANT à SMP4 et SMP8 (0 passe qualifiée, même porte « charge sans effet ») ; **aucun défaut de sûreté** (le relevé dans le bras, V3, tient).
- Série : quatre démarrages verts (KVM 12 s : différées toutes vidées, aucune perdue).

### 7.2 Endurance : le rouge SMP4 et sa cause

Cycle 11 de SMP4 : `QUALIFIE_ECHEC` de la sonde, « la priorité interactive
raccourcit la queue » (p99 3 095 contre 2 580 µs, pire 5 954 contre 3 078 µs).
Le noyau n'explique que 2 422 µs du pire réveil (échéance → retour en ring 3) :
**3,5 ms tombent après le retour**. Cause trouvée dans le code : `pick_next`
tire la tête de la file locale sans regarder la tâche **courante** ; au quantum
(`preempt_from_irq`) comme à un point sûr (`safe_point`), une interactive tout
juste élue cédait son cœur au premier calcul **normal** en attente et
attendait le quantum suivant. Correctif `20e87dae` (cession par classe, défaut) ;
A/B local TCG SMP4 : 903–912 inversions par passe sous la politique historique,
0 sous la nouvelle. Même vérification rouge sur `1413289b` et `7ffedd2b` :
c'était une intermittence, pas une régression de `f23a1247`.

SMP2 NON_QUALIFIANT : la sonde juge la charge « sans effet mesurable » (p95 sous
charge 743–964 µs ≤ p95 au repos 776–1 048 µs) — la même signature que P13 KVM
(détection au tic au repos). Reproduit localement sur le HEAD corrigé (TCG
SMP2, deux passes) : **QUALIFIE_OK** les deux fois (repos p95 1 232 / 1 393 µs,
normale 4 008 / 4 055, interactive 908 / 956) — l'écart CI/local n'est pas
expliqué ; à relire sur la prochaine endurance.

### 7.3 Ce qui part avec la campagne suivante

| Commit | Objet | Preuve locale |
|---|---|---|
| `3929746a` | P13+ réveil à l'échéance exacte (expérimental, OFF) | A/B TCG SMP1 4 tours : ÉCHEC selon ses critères (p99) — non qualifié |
| `20e87dae` | fix P13 : cession par classe au quantum et aux points sûrs | 7 tests hôte, A/B TCG SMP4, OS_PRIMITIVES_OK local |
| `bb22f8d5` | ci : A/B KVM SMP1 du réveil précis | — |
| `0cc556a5`, `7c19ff46` | P10+.2 : composition directe dans la mémoire du scanout (OFF) | pixels identiques au bureau BGA, 0 octet copié, A/B TCG CONVAINCANT (plein 3 328 → 1 049 µs) |
| `43c05361` | P10+.3a : plan de soumission par trame (pur) | 7 tests hôte |
| `e275b543` | diag : la trace `+ echo AB_REVEIL_FIN` ne vaut plus fin (A/B IPI) | test négatif |
| *(ce lot)* | P10+.1 : interruption par défaut (A/B KVM CONVAINCANT ci-dessus) | garde et A/B mis à jour |

Nouveaux jobs du workflow des primitives : `ordonnanceur-ab` (A/B IPI déplacé,
A/B cession KVM SMP4, A/B réveil précis KVM SMP1) et `gpu-tampon` (pixels en
composition directe et A/B copie/direct, TCG puis KVM), en parallèle des sondes
(le job des sondes a duré 59 min sur 95).

## 8. Campagne `77e0e5de` — lue

| Run | Workflow | Résultat |
|---|---|---|
| [38704459510](https://github.com/bcharthur/bouchaud-os/actions/runs/38704459510) | CI Fast | vert |
| [38704459411](https://github.com/bcharthur/bouchaud-os/actions/runs/38704459411) | Integration | vert |
| [38704459481](https://github.com/bcharthur/bouchaud-os/actions/runs/38704459481) | Reliability V3 | vert |
| [38704459647](https://github.com/bcharthur/bouchaud-os/actions/runs/38704459647) | ladybird-native-browser | **rouge** : tous les jobs Ladybird verts ; `primitives / gpu-tampon` vert ; `primitives / probes` rouge (un pas, §8.2) ; `primitives / ordonnanceur-ab` rouge (un pas, §8.2) ; verdict de convergence rouge par conséquence (`causes=primitives=failure`) |
| [38704473612](https://github.com/bcharthur/bouchaud-os/actions/runs/38704473612) | Endurance 30 min / tranche (TCG) | **SMP1, SMP2, SMP4 verts** ; SMP8 NON_QUALIFIANT (code 2), cycles poursuivis |

### 8.1 Ce qui est démontré

- **Endurance SMP4 verte** après `20e87dae` (cession par classe) — le rouge de f23a1247 ne se reproduit pas ; SMP2 vert aussi.
- **P10+.1, A/B KVM : CONVAINCANT pour la deuxième campagne de suite** (6/6 : p50 plein 182 → 174 µs, cœur inactif 136 ‰ contre 0, 0 achèvement vu sans interruption sur 3 840). Le défaut « interruption » (`43f55f9c`) est reproduit.
- **P10+.2, job `gpu-tampon`** : pixels en composition directe TCG 918 494 / 921 600 et KVM 921 336 / 921 600, 0 octet copié, 0 accès pendant un transfert ; **A/B copie/direct KVM CONVAINCANT** 6/6 (p50 plein écran 428 → 232,5 µs, bande 116 → 110, tuile 72 → 70) et TCG CONVAINCANT (1 922 → 282,5). → défaut basculé sur la composition directe (`2a7ce3ed`), les trois bancs de pannes verts en local avec ce défaut.
- **Cession par classe sous KVM SMP4** : 916–919 inversions par passe sous la politique historique, 0 sous la nouvelle (≈ 302 cessions gardées par passe) — le mécanisme est confirmé sous KVM. L'A/B rend NON_CONCLUANT (0 tour apparié qualifié : la porte « charge sans effet » de la sonde, comme pour l'A/B IPI).
- P13 KVM SMP1/2/4/8 : `ORDONNANCEUR_TEMOIN_INSTRUMENT` aux quatre topologies (inchangé, non qualifié). Pixels P10 (copie) TCG+KVM, BGA/virtio ×2, `ATTACH_BACKING` sans réponse, réponse étrangère : verts.

### 8.2 Les deux rouges et leur cause

| Pas | Cause | Correction |
|---|---|---|
| probes : pannes P10+.1 (TCG) | `vus_par_relecture=0` sous TCG (4 sous KVM) : QEMU traite souvent la commande pendant l'écriture de la sonnette, la réponse était déjà là, l'injection « MSI-X masqué » n'était pas exercée — critère dépendant du hasard | `9519547b` : injection déterministe (réponse invisible jusqu'au premier sommeil de chaque attente masquée) ; critère **durci** à `>= 10` ; local TCG 10/10 |
| ordonnanceur-ab : A/B réveil précis KVM SMP1 | « mesure incomplète » pour 6 bras sur 8 : l'analyseur exigeait la ligne brute de la sonde terminée après « pire N us », or le noyau écrit ses relevés sur la même ligne sous KVM. Sûreté pourtant tenue (7 636–9 117 armements sous on, 0 sous off) | `35b6be1e` : les trois analyseurs A/B lisent la ligne de la sonde n'importe où dans la ligne, valeurs PROPRES ; 3 tests rouges sur les anciens analyseurs |

Aucun critère n'est affaibli ; le résultat de l'A/B du réveil précis sous KVM
reste **à mesurer** (campagne suivante).

## 9. Campagne `b6c99032` — lue (15 octobre, après-midi)

| Run | Workflow | Résultat |
|---|---|---|
| [38732049394](https://github.com/bcharthur/bouchaud-os/actions/runs/38732049394) | CI Fast | vert |
| [38732049305](https://github.com/bcharthur/bouchaud-os/actions/runs/38732049305) | Integration | vert |
| [38732049327](https://github.com/bcharthur/bouchaud-os/actions/runs/38732049327) | Reliability V3 | vert |
| [38732049308](https://github.com/bcharthur/bouchaud-os/actions/runs/38732049308) | ladybird-native-browser | **rouge** : les 15 jobs Ladybird verts (construction, smoke, robustesse, sites réels, WPT, endurances 10 min, KVM et 20 min KVM, cache/SQL, mémoire, crash des services, ordre worker, piège du Compositor, performance, smoke virtio-gpu) ; `primitives / gpu-tampon` et `primitives / ordonnanceur-ab` verts ; **`primitives / probes` rouge** (un pas, §9.2) ; verdict de convergence rouge par conséquence |
| [38732054379](https://github.com/bcharthur/bouchaud-os/actions/runs/38732054379) | Endurance 30 min / tranche (TCG) | **SMP1, SMP2, SMP4 verts** ; SMP8 **NON_QUALIFIANT** (`fatals=0` ; 22 cycles, porte de la sonde non qualifiée faute de preuve de l'hôte sous TCG — rien d'imputé au noyau, jamais une réussite) |

### 9.1 Ce qui est démontré

- **P13+ réveil précis, A/B KVM SMP1 : CONVAINCANT** 4/4 (job ordonnanceur-ab) : retard au repos p50 461 → 39 µs, p95 943,5 → 852, p99 1 007,5 → 1 016,5 (borne 1,25×), interactif sous charge p95 29,5 → 28,5 ; 8 239–8 672 armements par passe `on`, 0 sous `off`. → défaut **on**, borné au domaine prouvé (TSC-deadline sous hyperviseur ; mode périodique TCG et matériel nu restent OFF) : `b0bfd187`, `ada19b78`. Toutes les passes restent NON_QUALIFIANT (porte « charge sans effet ») : la queue au repos (p95 ≈ 850 µs) n'est pas résorbée — attribution publiée par `65fb83ab`.
- **P10+.2, A/B copie/direct KVM : CONVAINCANT pour la deuxième campagne** (plein 176 → 113,5 µs, bande 55 → 52, tuile 56,5 → 50) ; TCG CONVAINCANT ; pixels direct TCG 921 255 / KVM 921 314 sur 921 600, 0 octet copié. Défaut « direct » reproduit.
- **P10+.3b, A/B une trame / une soumission KVM : CONVAINCANT** 6/6 (16 → 5 commandes par trame, p50 474 → 177,5 µs, p95 550 → 237,5), TCG CONVAINCANT ; pixels TCG 921 272 / KVM 921 362. → défaut **on** (`ada19b78`), l'ancien chemin reste mesuré aux pixels (`SCANOUT_TRAME=off`, copie et direct ; local TCG 921 320 / 921 600).
- **Endurance SMP4 verte pour la deuxième campagne** depuis la cession par classe (`20e87dae`).
- A/B IPI (KVM SMP4, SMP8) et cession (KVM SMP4) : NON_CONCLUANT, 0 tour apparié qualifié (porte de la sonde). La cession par classe reste confirmée mécaniquement : 918 inversions/passe en historique, 0 sous classe.

### 9.2 Le rouge : `primitives / probes`, pas « P10+.1 — pannes de l'attente par interruption », bras KVM

- **Image testée** : `b6c99032a8caec12d7117405c16a9eb5de5da182`, option `banc-virtio-gpu-irq-pannes`, job [116411938282](https://github.com/bcharthur/bouchaud-os/actions/runs/38732049308/job/116411938282) (12:05–12:54 UTC). Artefacts (`os-primitives-debug`, id 11974813169) non récupérables depuis la session (hôte blob refusé) ; le journal du pas suffit.
- **Critère violé (seul)** : « parasites : au moins six envoyées, toutes comptées sans progrès par le gestionnaire » — `envoyees=9`, `irq_sans_progres=8`, `irq=38`. Bras TCG du même pas : vert (`envoyees=6`, `irq_sans_progres=6`, `irq=36`). Tous les autres critères tenus aux deux bras (relectures 10/10, une seule attente échue, un seul échec de présentation, abandon propre, aucune panique).
- **Cause** : les parasites étaient envoyées pendant qu'une commande était en vol. Un vecteur n'a qu'une place dans l'IRR : une parasite en attente et la vraie MSI-X de la réponse **fusionnent** en un seul passage du gestionnaire, qui voit du progrès. Les compteurs le montrent : les MSI-X réelles valent 30 (TCG : 36 − 6) ; KVM : 38 = 30 + 9 − **1** fusion, et 9 − 1 = 8 passages sans progrès. L'hypothèse inverse (parasite voyant le progrès, MSI-X ensuite sans progrès) donnerait 39 et 9 : rejetée. Le critère dépendait de l'instant ; le pilote n'est pas en cause.
- **Correction du banc** (`ef83b830`, option de banc seulement) : une parasite **hors vol** après chaque achèvement de la fenêtre (MSI-X de la réponse servie, une seule commande en vol), qui doit produire **exactement** un passage et un seul sans progrès ; critère : ≥ 6, toutes exactes. Les parasites en vol restent envoyées et publiées comme preuve (elles exercent le réveil sans progrès).
- **Avant / après (TCG local)** : avant, critère dépendant de l'instant (vert TCG, rouge KVM) ; après, `hors_vol=6 hors_vol_sans_progres=6`, `irq=42` (30 + 6 en vol + 6 hors vol), `VIRTIO_GPU_IRQ_PANNES_BANC_OK`. **KVM : campagne `0c1e9e5c`.**

### 9.3 P10+.3c / P10+.3d — gelés

Surface client à deux tampons (`617b5383`, `d2b80a9a`) : **OFF par défaut**, aucune extension pendant la stabilisation. Revue du chemin « composition qui échoue ou se reporte » du chrome : une annonce `FrameReady` ratée faisait rejouer deux fois le tampon arrière et écraser des pixels peints mais pas annoncés — reproduit par test hôte (2 cas rouges), corrigé (`d378ce42`). Le report faute de tampon libre garde son dégât (aucune perte). Preuves acquises : tests hôte, analyse clang, banc de déchirure **local TCG** CONVAINCANT. **Non acquises** : banc KVM, smoke Ladybird réel sous `gui-surface double` (job de diagnostic, hors verdict) ; le banc de déchirure est non bloquant (`0c1e9e5c`) tant que le lot est gelé.
