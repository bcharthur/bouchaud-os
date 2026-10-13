# Handoff actif — Claude (session du 13 octobre 2026)

> Source vivante de reprise. Les handoffs datés (`HANDOFF_CLAUDE_P19_2026-10-12.md`,
> `HANDOFF_CODEX_2026-10-08.md`) restent l'historique ; ce fichier dit où en
> est le travail **maintenant**. Chaque verdict cite le run qui l'a produit ;
> rien ici n'annonce un résultat futur.

## 1. Git

| | |
|---|---|
| Dépôt / branche | `bcharthur/bouchaud-os` / `claude/ladybird-observability-performance` (jamais `main`) |
| Base de session | `0a83f182` (remote au démarrage) ; `main` = `788c5446` |
| HEAD de code campagné | `fa387b28` (noyau = `cd65aa45` + journal `SESSION_EMPORTE`) ; le commit qui ajoute ce fichier ne contient que de la documentation |
| Interdits respectés | aucun reset/clean/force-push/rebase ; aucun artefact binaire versionné ; aucun secret journalisé |

## 2. Corrigé pendant la session, avec la preuve

| Bug | Cause racine | Avant → après | Niveau atteint |
|---|---|---|---|
| Garde préemption 166/167 | réouverture d'IF P19 non bornée au ring 3 | → 169/169, 11 tests négatifs | STATIC |
| Crash Guard multi-fils (P0) | `exit_group` avec IF=0 → `schedule()` | KERNEL PANIC → `FAUTE_MULTIFIL_OK` | **KVM** (os-primitives `7ffedd2b` 38448072600 ; TCG SMP1/4/8 + KVM SMP4) |
| Fil frère en `pause()` immortel | `sys_sigsuspend` ne relisait pas la condamnation | 8/8 immortels → 0 | KVM (même run) |
| CPU0 ne préempte pas l'utilisateur (P0) | report diagnostic V8 jamais retiré | gel SMP1, immortel SMP8 → 24/24 | KVM ; soak 20 min vert ×5 |
| Panique RAMFS « used-node accounting overflow » (P0) | miroir de comptage jamais décrémenté | panique → `NOEUDS_CHURN_OK cycles=8000` | TCG en invité (CI) |
| `rmdir` → ENOSYS | appel non aiguillé, `AT_REMOVEDIR` ignoré | → OK | TCG (CI) |
| Signal de mort des fautes ring 3 | `exit_group(139)` fixe | SIGILL/SIGFPE/SIGSEGV exacts | KVM (CI) |
| **Panique d'extinction** « parking demandé hors de toute tâche » (P0) | l'autorun s'éteint sans tâche courante ; la persistance y attend un verrou dormant tenu par un processus vivant | SMP1 3/3 panique → 5/5 propres ; sites réels rouges → verts | **TCG CI** (`EXTINCTION_CONTENTION_BANC_OK` SMP1+SMP4, `cd65aa45`) |
| Nightly `wm_vers_navigateur` | banc sans socat/nc | défaut de banc | HOST |
| Build Ladybird `7ffedd2b` | `bytes()` sur String temporaire (code de session) | → build vert | CI |
| Endurance KVM « ?/57 » | ligne FIN tronquée par l'entrelacement série | ligne courte + repli (rouge si absente) | banc, simulé |

Diagnostics ajoutés (sans changement de comportement) : lockdep `held_at=`,
panique lisible dans le soak, `[LB:NAV-TRACE]`, `[LB:JSVALEUR]` (LibJS
`Value.cpp:484`, assertion conservée), `SORTIE_ABSENTE`, `SESSION_EMPORTE`.

## 3. Campagne de fin de session — `fa387b28`

Verts : CI Fast, Reliability V3 avec **soak 20 min**, Native IPC, SCM_RIGHTS,
Security ; Ladybird 13/16 : build, smoke, cache/SQL, WPT, sites réels,
**robustesse 9/9**, mémoire (P2/P9), ordre worker, piège Compositor,
performance, endurance 10 min TCG, endurance KVM A/B, endurance 20 min KVM.

Rouges (détail : `AUDIT_STABILITY_2026-10-13.md` §3 quater) :
1. **Integration / `persist-course-probe`** : 3 fils fautent au même `rip`
   (`printf_core+0`, un `push`) avec `cr2=0x45`. Incohérent pour un `push` →
   soupçon d'attribution de faute ou d'état de fil. Vert sur 3 SHA
   précédents, 0/12 en local. **Candidat P0 ouvert.**
2. **Audio OSS H3a** : `pire_exces_odelay=7` trames une fois. Ouvert.
3. **Crash des services** : `pid ?` (ligne série tronquée) — défaut de banc.

## 4. Rouges et inconnues restants (hors campagne)

1. **Endurance hebdo / `ordonnanceur-probe`** (38448087567) : un échantillon
   interactif isolé > pire normal + bruit ; 13 ms en SMP1 > 3 quanta. Critère
   laissé strict ; hypothèse « IPI de réveil » mesurée sans effet, retirée.
2. **Panique lockdep `courant.rs:276`** : jamais revue (5 soaks CI 20 min,
   17 cycles locaux). `held_at=` publié si elle revient.
3. **LibJS `Value.cpp:484`** : instrumenté et compilé ; pas encore observé.
4. **ONE-LOADER** : STATIC seulement (une preuve discriminante exige une
   capture PENDANT le chargement).
5. **P8 HDA, P10 scanout 2D/3D** non écrits ; **P14** non repéré ;
   **P17** non retrouvé dans le code versionné.

## 5. Prochaine commande exacte

```bash
# 1. Le candidat P0 : répéter persist-course-probe sous la CI (Integration)
gh api -X POST repos/bcharthur/bouchaud-os/actions/workflows/integration.yml/dispatches \
  -f ref=claude/ladybird-observability-performance
# 2. Puis instrumenter PROCESS_FAULT (fs_base, octets à rip) si le rouge revient.
```

Règle CI apprise : ne jamais relancer `os-primitives.yml` à la main pendant un
run `ladybird-native-browser` (même groupe de concurrence : le job
`primitives / probes` du run Ladybird est annulé, vu sur 38447976372).

Reproductions locales (TCG) :

```bash
cargo bootimage
BO_SMP=1 tools/ci/run_faute_multifil.sh target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin 2
BO_SMP=1 tools/ci/run_extinction_contention.sh target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin
```

## 6. Physique (TRIGKEY)

Aucun test physique, aucun flash, aucune modification du démarrage. L'image
de référence se prend dans l'artefact `bouchaud-ladybird-native-browser`
(avec `BOUCHAUD_ARTIFACT_MANIFEST.json`) d'un run Ladybird **entièrement
vert**, puis `docs/current/RELEASE_TRIGKEY_CHECKLIST.md`. Aucun run de cette
session n'est entièrement vert : **pas d'image TRIGKEY recommandée**.
Flash et démarrage : Arthur seulement. `PHYSICAL_STATUS=BLOCKED`.
