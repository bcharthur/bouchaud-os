# Bouchaud OS

Bouchaud OS est un système d'exploitation expérimental **bare-metal écrit from
scratch en Rust** (`no_std`, x86_64). Il possède son propre noyau, son
ordonnanceur SMP, sa mémoire virtuelle, ses processus, ses primitives IPC, son
réseau, ses pilotes et son environnement graphique.

Il ne repose **ni sur Linux ni sur Windows à l'exécution**. Une couche de
compatibilité Linux/POSIX existe pour porter des logiciels complexes : c'est
une interface de compatibilité, pas un noyau Linux caché sous Bouchaud OS.

> **EXPÉRIMENTAL.** Une fonctionnalité n'est déclarée acquise qu'au niveau de
> preuve réellement obtenu : `STATIC`, `HOST`, `QEMU-TCG`, `QEMU-KVM` ou
> `PHYSICAL`.

## État de référence

| | |
|---|---|
| Branche de travail | `claude/ladybird-observability-performance` |
| HEAD code ayant porté la campagne P1→P13 | `9e086175c8e77c3a48d11eee8d2aa86442c78d67` |
| HEAD documentation | `7317f06aed8842e721c7e78ecad6e293986c670d` |
| Ladybird upstream épinglé | `cdfe5f858eb5fc64a8d9d3fcc247d71b03fbd1f6` |
| Référence physique | **TRIGKEY Speed S5** |
| Statut | convergence Bouchaud Navigateur en cours ; P2/P9 proches de fermeture, P13 encore PARTIAL |

Pour reprendre le projet, lire en premier
[`BOUCHAUD_AI_CONTEXT.md`](BOUCHAUD_AI_CONTEXT.md), puis
[`docs/current/`](docs/current/README.md).

## Résultat de la dernière campagne

```text
CI Fast                     SUCCESS
Reliability V3              SUCCESS
Integration                 SUCCESS
os-primitives               SUCCESS

Bouchaud Navigateur :
  build                     SUCCESS
  BrowserHost smoke         SUCCESS
  cache + SQL               SUCCESS
  WPT                       SUCCESS
  sites réels               SUCCESS
  robustesse                SUCCESS
  crash services            SUCCESS
  ordre Worker              SUCCESS
  piège Compositor          SUCCESS
  performance               SUCCESS
  endurance TCG 10 min      SUCCESS
  endurance KVM 20 min      SUCCESS

  mémoire onglets           FAILURE diagnostic
  endurance KVM A/B         FAILURE
  convergence P13           FAILURE
```

Le workflow navigateur est donc **presque entièrement vert**, mais le marqueur
final de convergence n'est pas atteint.

## Lots P1 → P13

| Lot | Sujet | État | Niveau | Reste principal |
|---|---|---|---|---|
| P1 | UI Bouchaud / BrowserHost | **DONE** | QEMU | validation physique |
| P2 | Compositor → WM / lifecycle | **PARTIAL, proche fermeture** | QEMU | banc mémoire 20/20 + plateau |
| P3 | damage tracking | **DONE** | QEMU | physique |
| P4 | scroll asynchrone | **DONE** | QEMU | latence physique |
| P5 | WebWorker | **DONE** | QEMU | physique |
| P6 | sandbox / rôles / `no_new_privs` | **DONE** | HOST + QEMU | physique |
| P7 | profil / SQL / cache persistant | **DONE** | QEMU | physique |
| P8 | audio | **PARTIAL** | QEMU AC'97 | HDA physique |
| P9 | multi-WebContent / isolation / OOPIF | **PARTIAL, proche fermeture** | QEMU | 20/20 swaps + plateau mémoire |
| P10 | GPU | **BLOCKED, fondation commencée** | STATIC + QEMU | virtio-gpu complet puis vraie accélération |
| P11 | WPT | **DONE sur le corpus courant** | QEMU | élargir le corpus |
| P12 | documentation courante | **DONE** | STATIC | maintenance continue |
| P13 | convergence | **PARTIAL** | QEMU | expliquer le premier bras KVM A/B |
| P18 | `/proc` / comptabilité historique | **DONE ciblé** | QEMU | surveillance des régressions |

Détail vivant : [`docs/current/P1_P13_CURRENT.md`](docs/current/P1_P13_CURRENT.md).

## P2/P9 — le défaut mémoire a changé de nature

Avant le discard direct des anciennes pages après un process swap :

```text
contexts_live       3 → 13 → 22
backing_store KiB   4623 → 50857 → 97092
```

Sur le HEAD de campagne :

```text
Compositor RSS      41652 → 48276 → 48280 KiB
contexts_live       2 → 3 → 3
backing_stores      4 → 2 → 4
backing_store KiB   16822 → 4623 → 16822
```

La croissance linéaire précédente n'est plus présente. Le banc reste rouge
parce que **17/20** onglets seulement ont effectué le process swap attendu et
parce que l'analyseur conservateur classe encore `2 → 3 → 3` comme croissance.

P2/P9 ne sont donc pas déclarés DONE, mais le défaut majeur initial paraît
corrigé.

## Endurance KVM

Les deux bras A/B produisent environ **58 cycles / 58 frames en 304 s**, sans
frame au-delà de 30 s.

Le premier bras échoue au `STABILITY_GATE` uniquement parce que le banc compte
zéro Compositor alors que le même run montre :

- 58/58 frames ;
- aucune mort du Compositor ;
- aucune faute processus ;
- aucune assertion ;
- aucun abandon de lien ;
- `PERFORMANCE_GATE` vert.

Le second bras compte correctement un Compositor et passe. La priorité est donc
d'attribuer cette divergence de comptage avant de modifier un budget.

---

# Bouchaud Navigateur

**Bouchaud Navigateur** est le navigateur de Bouchaud OS.

Son moteur web dérive de **Ladybird**. L'intégration Bouchaud apporte notamment
le BrowserHost, l'intégration au Window Manager, les adaptations système, la
sandbox, le stockage, l'observabilité, le recovery et les bancs de convergence.

```text
┌──────────────────────── ring 3 ──────────────────────────────┐
│ BouchaudBrowserHost                                         │
│    ├── WebContent × N                                       │
│    ├── WebWorker × N                                        │
│    ├── RequestServer                                        │
│    ├── ImageDecoder                                         │
│    └── Compositor ── surfaces / damage ──► Window Manager   │
├────────────────────── noyau Bouchaud ───────────────────────┤
│ SMP · VM · VFS · IPC · sécurité · réseau · signaux · temps  │
│ stockage · audio · pilotes · observabilité                  │
└──────────────────────────────────────────────────────────────┘
```

La provenance Ladybird reste explicite dans les notices et licences. Le produit
Bouchaud n'est pas présenté comme un moteur web développé entièrement from
scratch.

### Crash/reconnexion Compositor

Une course de reconnexion faisait parvenir une display list référençant une font
absente dans le nouveau Compositor, jusqu'à
`DisplayListPlayerSkia::play_command(DrawGlyphRun)`.

Le protocole retient désormais les mises à jour concernées jusqu'à la
reconnexion complète puis rejoue l'état. Le smoke, le banc ciblé, la robustesse
et l'endurance longue n'ont pas reproduit le trap sur la campagne courante.

---

# Architecture

```text
                        Applications ring 3
                               │
                    API / compatibilité / IPC
                               │
┌──────────────────────── Bouchaud OS ──────────────────────────┐
│                                                               │
│  Processus / threads      Scheduler SMP       Signaux / futex │
│  VM / VMA / shared mem   VFS / persist       Temps / timers  │
│  Réseau / sockets        Sécurité / rôles    GUI / surfaces  │
│                                                               │
│          Driver APIs / bus / découverte matériel              │
│                                                               │
└──────────────────────────────┬────────────────────────────────┘
                               │
                   QEMU x86_64 ou TRIGKEY
```

La compatibilité Linux/POSIX est une **personnalité de compatibilité**. La
direction est de faire émerger progressivement des API Bouchaud natives sans
casser les ports complexes qui servent aujourd'hui de bancs système.

## Pourquoi le navigateur est central

Un navigateur moderne force l'OS à être correct simultanément sur :

- processus et threads ;
- mémoire virtuelle et mappings fichier ;
- IPC et mémoire partagée ;
- scheduler et temps ;
- signaux et supervision ;
- sockets, DNS, TCP et TLS ;
- stockage, SQL et cache ;
- rendu, surfaces et input ;
- isolation et recovery.

Les bugs les plus structurants corrigés récemment ont justement été révélés par
ce workload.

---

# Machine physique de référence : TRIGKEY Speed S5

La trajectoire matérielle actuelle est centrée sur la **TRIGKEY Speed S5**, pas
sur une plateforme ARM expérimentale.

Configuration de référence déclarée : **AMD Ryzen 7 5700U**, 16 CPU logiques,
NVMe interne, Ethernet RTL8111/8168 et boot UEFI.

> Certains anciens relevés archivés du dépôt identifient le CPU comme
> `AMD Ryzen 7 5800H with Radeon Graphics`. Cette contradiction doit être
> revalidée au prochain boot physique avant d'utiliser le modèle CPU comme
> preuve matérielle. Les autres verdicts doivent eux aussi rester reliés au
> manifeste et au relevé physique qui les a produits.

Le détail courant est dans
[`docs/current/TRIGKEY_REFERENCE.md`](docs/current/TRIGKEY_REFERENCE.md).

### Chemins matériels

- **GOP UEFI** : framebuffer physique actuel ; pas de pilote AMD accéléré.
- **NVMe** : pilote physique séparé du chemin ATA utilisé par certains tests QEMU.
- **xHCI / HID** : clavier/souris et diagnostic physique.
- **RTL8111/8168** : Ethernet physique de référence.
- **Wi-Fi** : aucune compatibilité complète revendiquée.
- **Audio** : AC'97 QEMU ne prouve pas le HDA physique.
- **GPU** : virtio-gpu QEMU ne prouve pas l'iGPU AMD.

### Image physique

Chaîne de référence :

```powershell
powershell -ExecutionPolicy Bypass -File .\tools\reference\IMAGE-TRIGKEY.ps1
.\tools\reference\run-trigkey-single-usb-qemu.ps1 -Accel tcg -SkipBuild
```

Une image physique doit conserver ensemble :

- image ;
- SHA256 ;
- manifeste ;
- HEAD ;
- SHA Ladybird ;
- noyau ELF non strippé.

QEMU exact-layout précède le flash. Une validation QEMU n'est jamais présentée
comme une validation PHYSICAL.

---

# P10 — GPU

P10 reste **BLOCKED pour l'accélération GPU**.

Le premier composant réel existe :

```text
PCI
 → transport virtio moderne
 → control virtqueue
 → négociation VERSION_1
 → GET_DISPLAY_INFO
 → réponse structurée
```

Cela ne prouve ni scanout 2D complet, ni virgl/Venus, ni Vulkan, ni Skia GPU,
ni pilote AMD physique.

L'ordre raisonnable reste :

1. ressources virtio-gpu 2D ;
2. backing / scanout / transfer / flush ;
3. fences et interruptions ;
4. étude 3D ;
5. API utilisateur ;
6. pile Vulkan/Mesa si pertinente ;
7. Skia accéléré ;
8. mesure CPU/GPU.

---

# Stockage et persistance

## ATA sous QEMU

Une source majeure de latence a été identifiée : un `FLUSH CACHE` était émis
après chaque écriture.

Le modèle courant est :

```text
write
  ↓
écriture
  ↓
barrière explicite
  ↓
FLUSH CACHE + vérification
```

Un banc d'intégrité vérifie motifs pseudo-aléatoires, lecture après écriture,
restauration et repli PIO après erreur injectée.

## NVMe physique

La TRIGKEY utilise un chemin NVMe distinct. Un résultat ATA QEMU n'est pas une
preuve du NVMe physique.

## `/persist`

Le profil navigateur, SQL/WAL et le cache persistant exercent le stockage et les
barrières au-delà d'un simple test de lecture/écriture.

---

# Sécurité

Le navigateur sépare les rôles :

```text
Browser UI / Broker
Content / Renderer
Worker
Network
ImageDecoder
Compositor
Media
```

Les politiques utilisent notamment `no_new_privs`, des profils par rôle et des
tests négatifs.

Un test de sécurité n'est vert que lorsqu'une opération autorisée fonctionne
**et** qu'une opération interdite échoue comme attendu.

---

# Observabilité et fiabilité

Bouchaud OS intègre comme outils de première classe :

- journal série sous QEMU ;
- blackbox persistante ;
- télémétrie réseau ;
- BRDP ;
- profils RIP ;
- statistiques processus/syscalls ;
- RSS ;
- métriques de contextes/surfaces Compositor ;
- compteurs stockage ;
- métriques scheduler/SMP.

Principe :

```text
"lent"    → profil et latences
"mémoire" → ownership + objets + RSS
"crash"   → type de faute + pile + lifecycle
"réseau"  → couche exacte
"freeze"  → scheduler / verrou / I/O / raster
```

---

# Construire et tester

## QEMU interactif Windows

```powershell
git clone https://github.com/bcharthur/bouchaud-os.git
cd bouchaud-os

.\run.ps1
```

Les paramètres exacts d'un test doivent toujours être relus dans le script du
HEAD courant.

## Vérification noyau / CI locale

```bash
cargo bootimage
tools/test.sh
tools/ci/run_os_primitives.sh target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin
tools/ci/run_architecture_guards.sh
python3 tools/verifie-readme-commandes.py
```

## Build local du navigateur sous WSL

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass `
  -File .\tools\reference\build-ladybird-local.ps1 `
  -RepoRoot "<repo>" `
  -Jobs 8
```

Lorsqu'un artefact local vient d'être installé volontairement, ne pas utiliser
`-RefreshLadybird` pour le même essai.

---

# Modèle de preuve

| Niveau | Signification |
|---|---|
| STATIC | inspection, garde-fou ou contrat |
| HOST | test exécuté sur l'OS de développement |
| QEMU-TCG | exécution réelle mais CPU émulé |
| QEMU-KVM | exécution virtualisée avec accélération matérielle |
| PHYSICAL | observation sur la TRIGKEY |

La hiérarchie n'est pas automatique : `QEMU-KVM` ne devient jamais
`PHYSICAL` sans nouveau test.

### Politique endurance

- TCG : stabilité bloquante, performance mesurée.
- KVM : stabilité **et** performance bloquantes.
- campagne KVM longue : diagnostic supplémentaire.

Cette séparation évite d'optimiser Bouchaud OS pour le coût du raster SIMD
émulé par TCG.

---

# Vision

Bouchaud OS n'a pas vocation à être un simple clone d'un système existant.

Directions de recherche :

- modes d'usage (Work, Gaming, Focus, Créatif, etc.) capables d'adapter UI,
  notifications et politiques système ;
- restauration de contexte complet ;
- Bouchaud Insights pour expliquer l'état de la machine ;
- IA locale comme **service système isolé** ;
- automatisations explicites sous contrôle utilisateur ;
- IPC/objets partagés limitant les copies ;
- API Bouchaud natives à côté de la compatibilité ;
- ordonnancement progressivement plus conscient des besoins de service.

Ces éléments sont des **directions** tant qu'ils n'ont pas leur preuve runtime.

---

# Différence avec Windows, Linux et macOS

Bouchaud OS ne prétend pas aujourd'hui avoir leur maturité, leur couverture
matérielle ou leur écosystème.

Sa différence expérimentale vient de la possibilité de faire évoluer ensemble :

- noyau ;
- scheduler ;
- mémoire ;
- sécurité ;
- navigateur ;
- observabilité ;
- expérience utilisateur ;
- futurs services IA ;

et de relier ces changements à des bancs de preuve du même HEAD.

La valeur du projet ne réside donc pas dans l'affirmation « différent » mais
dans la capacité à **mesurer exactement ce qu'une décision système apporte**.

---

# Documentation

Point d'entrée IA/humain :

- [`BOUCHAUD_AI_CONTEXT.md`](BOUCHAUD_AI_CONTEXT.md)

Synthèse courante :

- [`docs/current/README.md`](docs/current/README.md)
- [`docs/current/STATUS_CURRENT.md`](docs/current/STATUS_CURRENT.md)
- [`docs/current/P1_P13_CURRENT.md`](docs/current/P1_P13_CURRENT.md)
- [`docs/current/ARCHITECTURE_CURRENT.md`](docs/current/ARCHITECTURE_CURRENT.md)
- [`docs/current/TRIGKEY_REFERENCE.md`](docs/current/TRIGKEY_REFERENCE.md)
- [`docs/current/BOUCHAUD_NAVIGATEUR.md`](docs/current/BOUCHAUD_NAVIGATEUR.md)
- [`docs/current/CI_AND_PROOFS.md`](docs/current/CI_AND_PROOFS.md)
- [`docs/current/HISTORY_AND_DECISIONS.md`](docs/current/HISTORY_AND_DECISIONS.md)
- [`docs/current/ROADMAP_CURRENT.md`](docs/current/ROADMAP_CURRENT.md)
- [`docs/current/KNOWN_LIMITATIONS.md`](docs/current/KNOWN_LIMITATIONS.md)

La documentation historique reste dans le dépôt pour expliquer l'évolution du
projet, mais le code, les logs et les documents `docs/current/` ont priorité
pour décrire l'état actuel.

---

# Licence

Voir `LICENSE`, `LICENSE-MIT`, `LICENSE-APACHE` et les notices tierces du
dépôt. Les composants de Bouchaud Navigateur issus de Ladybird et des autres
projets open source conservent leurs notices respectives.
