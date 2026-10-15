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
