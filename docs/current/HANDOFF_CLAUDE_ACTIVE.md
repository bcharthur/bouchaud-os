# Handoff actif — Claude (session du 14 octobre 2026)

> Source vivante de reprise. Les handoffs datés (`HANDOFF_CLAUDE_P19_2026-10-12.md`,
> `HANDOFF_CODEX_2026-10-08.md`) et la version du 13 octobre (dans l'historique
> Git de ce fichier) restent l'historique ; ce fichier dit où en est le travail
> **maintenant**. Chaque verdict cite le run qui l'a produit ; rien ici
> n'annonce un résultat futur.

## 1. Git

| | |
|---|---|
| Dépôt / branche | `bcharthur/bouchaud-os` / `claude/ladybird-observability-performance` (jamais `main`) |
| Base de session | `77cba441` (remote au démarrage) |
| Ordre de mission | message du 14 octobre (le « fichier joint » annoncé n'était pas joint ; exécuté d'après le texte) |
| Interdits respectés | aucun reset/clean/force-push/rebase ; aucun artefact binaire versionné (captures `.ppm`, journaux et images ignorés) ; aucun secret journalisé |

Commits de la session, dans l'ordre :

| SHA | Objet |
|---|---|
| `9766869d` | faute de page fatale : `cr2` lu à l'entrée, `code`, `issue`, `cr2_relu`, VMA voisines (diagnostic S1) |
| `05b542bf` | **P10** : scanout virtio-gpu 2D (ressources, mémoire, transfert, présentation, 7 refus, repli BGA) |
| `dc4005f9` | **P10** : banc du scanout vérifié aux pixels, en CI (TCG puis KVM) |
| `ed3a1bc3` | sonde + banc des lignes série entières, bilan série — **reproduction AVANT correctif** |
| `3188476a` | garde de profondeur de pile déterministe |
| `4f253ab6` | lignes série entières (course perdue → file ; userland et débordements sous jeton ; `_print` hors ligne) |
| `e2ddacb1` | ODELAY AC'97 sur un relevé cohérent (H3a) |
| `585fc0a0` | docs P10 (P10 reste BLOCKED) |
| `3a4a31e5` | P10 : file virtio cassée après une commande sans réponse ; DMA rendue seulement si sûre |

## 2. Corrigé pendant la session, avec la preuve

| Défaut | Avant → après | Niveau |
|---|---|---|
| Lignes série mêlées (crash des services `pid ?`, marqueurs absents, lignes collées) | KVM `ed3a1bc3` : **397** émissions sans jeton en un démarrage → KVM `585fc0a0` : 720/720 lignes de 1 800 o, **0** sans jeton | **KVM** |
| ODELAY AC'97 +7 trames (H3a) | test d'hôte : +7 exact avec l'ancien calcul → 0 ; banc OSS 3/3 local + CI `585fc0a0` | HOST + TCG |
| Garde de pile dépendant de `PYTHONHASHSEED` | 2/8 graines vertes → déterministe ; mutation rouge pour toutes | HOST |
| +20 Kio de pile (introduit par la session, retiré avant push) | 63 320 → 43 992 octets (budget 61 440) | STATIC |
| `cr2` relu tard dans `PROCESS_FAULT` | champs `code= issue= cr2_relu=` vus en CI KVM | KVM (format) |

## 2 bis. Campagne same-HEAD `585fc0a0` (lue)

| Workflow | Run | Résultat |
|---|---|---|
| CI Fast | 38530194633 | vert |
| Reliability V3 (soak) | 38530194629 | vert |
| Integration | 38530194636 | vert |
| ladybird-native-browser | 38530194699 | **16/16 verts**, dont verdict de convergence, crash des services (rouge 2/2 avant), endurance 20 min KVM (rouge avant), os-primitives TCG+KVM (rouge avant) |
| Native IPC, SCM_RIGHTS, Security | relancés à la main sur ce SHA | voir la mise à jour suivante |

Deuxième passage (`rerun`, attempt 2) lancé sur le MÊME SHA pour la
reproductibilité. Tant qu'il n'est pas lu, la baseline est « verte une fois ».

## 3. P10 — scanout virtio-gpu 2D

Prouvé sous **QEMU-TCG et QEMU-KVM** (os-primitives `585fc0a0`, KVM :
mire 3721/3721, bureau 921 130/921 600, 28 présentations sans échec,
182 µs chacune) : autotest 2D (positif + 7 refus
avec leur code), mire 64×64 exacte au pixel (3721/3721), bureau présenté par
virtio-gpu sans BGA identique au bureau BGA (921 404/921 600 pixels), horloge
vivante à l'écran, repli sans écran, mesure `[GUI-VIRTIO-GPU]` (449–531 µs par
présentation). Mutation `presente()` sans transfert ni flush → banc rouge.
**Pas une accélération, pas de 3D. P10 = BLOCKED.** Détail :
`docs/ladybird/P10_GPU_AUDIT.md` §4 bis.

## 4. Ouverts

1. **S1 `persist-course-probe`** (`cr2=0x45` sur un `push`, `fa387b28`) :
   vert sur `ed3a1bc3` (Integration, os-primitives KVM). Les nouveaux champs
   nommeront la prochaine occurrence.
2. **Endurance hebdo / `ordonnanceur-probe`** : critère strict, ouvert.
3. **Panique lockdep `courant.rs:276`** : jamais revue.
4. **LibJS `Value.cpp:484`** : instrumenté, pas observé.
5. **P8 HDA, P10 3D/AMD** non écrits ; **P14** non repéré ; **P17** non retrouvé.

## 5. Prochaine commande exacte

```bash
# Lire la campagne du HEAD poussé (os-primitives : pas « Console serie » KVM,
# pas « P10 -- scanout virtio-gpu 2D » TCG/KVM ; Ladybird : crash des services)
gh api "repos/bcharthur/bouchaud-os/actions/runs?branch=claude/ladybird-observability-performance&per_page=10"
```

Règle CI apprise : ne jamais relancer `os-primitives.yml` à la main pendant un
run `ladybird-native-browser` (même groupe de concurrence). Un push qui touche
`src/**` annule les runs Reliability et Ladybird en cours de la branche.

Reproductions locales (TCG) :

```bash
cargo bootimage
B=target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin
tools/ci/run_virtio_gpu_scanout.sh $B
tools/ci/run_serie_entrelacement.sh $B            # 3 x 200 lignes
tools/ci/run_serie_entrelacement.sh $B 6 120 120 1800
tools/ci/run_oss_horloge.sh $B
```

## 6. Physique (TRIGKEY)

Aucun test physique, aucun flash, aucune modification du démarrage.
`virtio-gpu (QEMU) ≠ GPU de la Trigkey`. La campagne `585fc0a0` est verte
une fois ; tant que le second passage n'est pas lu et que l'image n'a pas
été contrôlée selon `RELEASE_TRIGKEY_CHECKLIST.md`, **pas d'image TRIGKEY
recommandée**. Flash et
démarrage : Arthur seulement. `PHYSICAL_STATUS=BLOCKED`.
