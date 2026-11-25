# Suivi de l'audit architectural du 25/11/2026 — chantiers noyau

Branche `claude/lucid-galileo-0g4429`. Périmètre demandé : **l'OS**, pas
Ladybird, pas de nouvelle campagne CI longue. Chaque ligne « fait » porte son
niveau de preuve : **HOST** (test hôte), **TCG** (QEMU TCG local, banc court),
jamais KVM, WHPX ni PHYSICAL — rien ici ne vaut au-delà.

## 1. Ce qui a été fait

| Marqueur | Chantier de l'audit | Défaut corrigé | Preuve (témoin = image d'avant le correctif) |
|---|---|---|---|
| `BOUCHAUD_MM_REMPLACABLE_V1` | C « Mm indépendant par espace d'adressage » | `Process.mm` figé ; `execve` remplaçait l'espace **en place** ; les références du cache rendues par `Drop for Process` | prérequis des trois lignes suivantes |
| `BOUCHAUD_VFORK_REEL_V1` | D « vfork conforme, suspension du père, posix_spawn » | `clone(CLONE_VM\|CLONE_VFORK)` (donc `posix_spawn`) et `vfork` faisaient un `fork` à **copie complète** | `tools/ci/run_vfork.sh`, TCG SMP1/2/4 : témoin ÉCHEC `vfork_memoire_non_partagee`, nouvelle image OK. SMP2 : `posix_spawn` 9,1 ms contre 30,3 ms pour `fork+execve` (père de 8 Mio) ; `execve` après vfork : 0 page rendue au lieu de 2 159 |
| `BOUCHAUD_CLONE_CHILD_SETTID_V1` | B « interactions glibc » | `CLONE_CHILD_SETTID` ignoré (le fils gardait le tid du père dans son bloc de fil) ; `CLONE_CHILD_CLEARTID`/`SETTLS`/pile ignorés hors fils | sonde `vfork-probe` (`pthread_kill` du fil principal depuis un second fil) — la glibc 2.39 se compense elle-même, l'épreuve ne discrimine donc pas ce point seul |
| `BOUCHAUD_GETPPID_V1` | B « parenté des processus » | `getppid()` rendait **1 pour tous** | `run_vfork.sh … tid` : témoin ÉCHEC (code 6), nouvelle image OK |
| `BOUCHAUD_ESPACE_RENDU_A_LA_MORT_V1` | C « ressources résiduelles », transverse « gouvernance des ressources » | l'espace d'adressage d'un mort restait attaché au zombie jusqu'à sa récolte — et, mesuré, jusqu'au recyclage de l'emplacement de sa tâche | `tools/ci/run_memoire_mort.sh … zombie` : témoin 143 → 143 Mio libres (même après `wait4`) ; nouvelle image 143 → 191 Mio, **14 ms** après la mort, avant récolte |
| `BOUCHAUD_OOM_CONTROLE_V1` | C « gestion OOM contrôlée » | à court de frames, le processus **fautif** mourait (`issue=erreur_es`), quel qu'il soit | `run_memoire_mort.sh … oom` : témoin, le père fautif meurt ; nouvelle image `OOM_VICTIME pid=19 kio=172552 fautif=18`, gouffre tué par SIGKILL, père survivant. Politique pure : `tools/memoire/test_oom.rs` (HOST, 7 tests) |
| `BOUCHAUD_ORPHELINS_RECOLTES_V1` | B/D « reparentage et récolte des orphelins » | sans `init`, un fils dont le père était mort restait zombie **pour toujours** | `tools/ci/run_orphelins.sh` : témoin ÉCHEC `g1_zombie_pour_toujours` ; nouvelle image OK (règles « fils déjà zombie » et « père disparu ») |
| ligne `[PROC-CYCLE]` | O « comptage des pertes, ressources non libérées » | ces mécanismes n'avaient aucun compteur | publiée avec `[PROC-STAT]` : vfork, espaces rendus, orphelins, victimes OOM |

Non-régression jouée sur les images finales (TCG) : `run_tempete_exec.sh`
SMP4 (≈ 7 600 processus créés, 21 en mémoire à la fin, 0 échec),
`run_os_primitives.sh` (`OS_PRIMITIVES_VERDICT_OK`), gardes d'architecture
(214/216, les deux mêmes échecs qu'avant la branche :
`verifie-audit-chantiers.py`, `verifie-pile-noyau-profondeur.py`), suites hôte.

### Contrats introduits

- **Un `Mm` rend ce qu'il tient à SA mort.** Il peut survivre au processus
  (fils vfork) ou le précéder (`execve`, mort). `MmCourant::lock()` rend un
  garde qui garde le `Mm` vivant : un `execve` concurrent ne détruit jamais ce
  qu'un lecteur tient.
- **Un `execve` sur un `Mm` partagé installe un `Mm` neuf** et ne retire pas
  l'identité de l'autre processus.
- **Tout chemin qui finit un emprunt vfork réveille le père** : `execve`
  réussi, sortie du dernier fil, mort forcée (`tue_processus`), destruction du
  `Process` (filet). Le père attend en `TASK_KILLABLE` : un signal ordinaire ne
  le reprend pas, une condamnation si.
- **À la mort du dernier fil, l'espace est rendu** — sauf si c'est le `Mm`
  du père (fils vfork mort sans `execve`) ou s'il est encore emprunté. Le
  zombie pointe sur un `Mm` des morts partagé, vide, jamais exécuté.
- **L'OOM ne tue jamais sous verrou**, attend sa victime au plus
  `DELAI_MORT_NS` (3 s) par victime, et abandonne (comportement d'avant) si le
  fautif est lui-même le plus gros ou si personne n'est désignable.

## 2. Ce qui n'a pas été fait, et pourquoi

| Chantier | Raison | Ce qu'il faut pour avancer |
|---|---|---|
| A — politique de placement au réveil (constat SMP2 de la passation du 17/11) | La règle `portee_replacement` est verrouillée par un protocole témoin CI (image `banc-reveil-historique`, KVM, `verifie-reveil-coeur-muet.py`). La changer sans rejouer ce protocole contredirait l'audit (« établir les défauts résiduels avant de toucher l'ordonnanceur »). | Mesurer d'abord : `[SCHED-NG-CENTILES]` par classe existe ; ajouter le même histogramme par CPU, puis un banc « normale réveillée derrière un calcul utilisateur, cœur voisin au repos ». |
| C/D — fork Copy-on-Write | Changement profond : compteur de références par frame, faute d'écriture sur PTE présente (aujourd'hui `protection_fault` ⇒ `Invalid`), **toute écriture noyau en mémoire utilisateur** (`AddressSpace::write`, `clear_child_tid`, `user_write`) doit casser le partage, shootdown TLB du père à la mise en lecture seule, `mprotect`/`munmap`/`madvise` à adapter. Non qualifiable sans campagne. | Le vfork couvre déjà `posix_spawn` (le chemin du navigateur). Pour le reste : PR dédiée, avec témoin « père qui écrit après fork » et « noyau qui écrit dans une page partagée ». |
| C — page zéro partagée, TLB shootdown borné | Même prérequis que le CoW (une PTE en lecture seule partagée). | Après le CoW. |
| O.4 — console série | Déjà en place : anneau, contre-pression, fil de vidange, repli différé (`BOUCHAUD_CONSOLE_CONTRE_PRESSION_V1`). Rien à ajouter sans mesure nouvelle. | — |
| E, F, G, H, J, K, L, M, N | Hors de portée d'une session sans KVM/WHPX ni matériel : chacun exige un banc graphique, réseau ou physique pour être qualifié. | Voir l'audit ; F et E demandent d'abord l'instrumentation de bout en bout (identifiant de corrélation HID → présentation). |
| I — Ladybird | Exclu par la demande. Le vfork réel profite à son `posix_spawn` sans y toucher. | Mesurer `PERF_VFORK` sur une campagne Ladybird. |

## 3. Rejouer les preuves

```bash
cargo bootimage
B=target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin
tools/ci/run_vfork.sh $B 10 2            # vfork, posix_spawn, CHILD_SETTID, getppid
tools/ci/run_memoire_mort.sh $B "zombie oom" 2 1024
tools/ci/run_orphelins.sh $B 2
tools/ci/run_tempete_exec.sh $B 30       # non-regression fork/exec (musl-gcc requis)
```

Chaque banc dure de 1 à 5 minutes en TCG. Ce ne sont pas des campagnes : ils
démontrent une propriété, ils ne qualifient pas une session.
