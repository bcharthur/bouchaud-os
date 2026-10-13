# Handoff actif — Claude (session du 13 octobre 2026)

> Source vivante de reprise. Les handoffs datés (`HANDOFF_CLAUDE_P19_2026-10-12.md`,
> `HANDOFF_CODEX_2026-10-08.md`) restent l'historique ; ce fichier dit où en
> est le travail **maintenant**. Rien ici n'annonce un résultat futur : chaque
> verdict cite le run qui l'a produit.

## 1. Git

| | |
|---|---|
| Dépôt / branche | `bcharthur/bouchaud-os` / `claude/ladybird-observability-performance` (jamais `main`) |
| Base de session | `0a83f182` (remote au démarrage) ; `main` = `788c5446` |
| HEAD poussé | voir la dernière ligne `LAST_COMMIT=` du compte rendu de session ; code : `d466c9ff` |
| Commits de code de la session | `56f12ef7` sigsuspend · `9728a4d9` Crash Guard borné + garde · `bf4a40b5` banc Nightly · `c9572100` préemption CPU0 · `ab9c4dda`/`50e0537a` rapport d'endurance · `ba8c68ff` étape multifil CI · `a28a9f9e`/`4401bcec` panique et lockdep lisibles · `871dc78a` garde ONE-LOADER · `7518bec0` signaux de faute · `f561e8f3`/`d466c9ff` trace de navigation · `de64dbf9` marge de bruit sonde ordo · `aa50aeba` comptage RAMFS · `c9963ae1` rmdir · `018da229` banc workers · `7ffedd2b` diagnostic LibJS |
| Worktrees | principal seulement en usage ; `scratchpad/wt-neg`, `scratchpad/wt-origin` jetables (hors dépôt) |
| Interdits respectés | aucun reset/clean/force-push/rebase ; aucun artefact binaire versionné |

## 2. Corrigé pendant la session, avec la preuve

| Bug | Cause racine | Avant → après | Niveau atteint |
|---|---|---|---|
| Garde préemption 166/167 | réouverture d'IF P19 non bornée au ring 3 | 166/167 → 169/169, 11 tests négatifs | STATIC |
| Crash Guard multi-fils (P0) | `exit_group` avec IF=0 → `schedule()` | KERNEL PANIC → `FAUTE_MULTIFIL_OK` | **KVM** (os-primitives `7ffedd2b`, run 38448072600 : SMP1/4/8 TCG, KVM SMP4 24 essais) |
| Fil frère en `pause()` immortel | `sys_sigsuspend` ne relisait pas la condamnation | 8/8 non terminés → 0 | KVM (même run) |
| CPU0 ne préempte pas l'utilisateur (P0) | report diagnostic V8 jamais retiré | SMP1 gel, SMP8 immortel → 24/24 | KVM (même run) ; soak 20 min vert ×2 |
| Panique RAMFS « used-node accounting overflow » (P0) | miroir de comptage jamais décrémenté (7 sites) | panique → `NOEUDS_CHURN_OK cycles=8000` | TCG en invité (même run) |
| `rmdir` → ENOSYS | appel 84 non aiguillé, `AT_REMOVEDIR` ignoré | ENOSYS → OK | TCG (même run) |
| Signal de mort des fautes ring 3 | `exit_group(139)` fixe | SIGILL/SIGFPE/SIGSEGV exacts | KVM (même run) |
| Nightly `wm_vers_navigateur` | banc sans socat/nc | défaut de banc corrigé | HOST |
| Build Ladybird `7ffedd2b` rouge | `bytes()` sur String temporaire (code de session) | erreur → `-fsyntax-only` rc=0 + contrôle négatif | STATIC (build CI `d466c9ff` : voir §3) |

## 3. Campagne same-HEAD

- `7ffedd2b` (noyau identique à `d466c9ff`) : CI Fast, Integration,
  Reliability V3 **avec soak 20 min**, os-primitives (multifil TCG+KVM, RAMFS,
  rmdir, matrice de rôles), Native IPC, SCM_RIGHTS, Security Runtime :
  **verts**. Ladybird : build rouge (corrigé par `d466c9ff`).
- `d466c9ff` : campagne Ladybird complète + Reliability/IPC/SCM/Security
  relancés — résultat consigné dans `AUDIT_STABILITY_2026-10-13.md` §3 ter.

## 4. Rouges et inconnues restants

1. **Endurance hebdo / `ordonnanceur-probe`** (38448087567) : un échantillon
   interactif isolé dépasse le pire normal + bruit (SMP1 1/14 cycles … SMP8
   1/1). 13 ms en SMP1 > 3 quanta (4 ms) : vrai retard, critère laissé strict.
   Hypothèse « IPI de réveil interactif » mesurée en A/B : sans effet, retirée.
2. **Panique lockdep `courant.rs:276`** : 0 reproduction depuis (17 cycles
   locaux, 2 soaks CI de 20 min). Le message publie désormais `held_at=`.
3. **Robustesse 8/9** (`4401bcec`) : un processus Ladybird pas sorti dans les
   5 s ; le banc nomme désormais l'absent (`SORTIE_ABSENTE`).
4. **LibJS `Value.cpp:484`** : instrumenté (`[LB:JSVALEUR]`), compilé en CI ;
   cause non établie tant que la ligne n'a pas été observée.
5. **ONE-LOADER** : STATIC seulement. Une preuve discriminante exige une
   capture PENDANT le chargement (l'ancienne jauge disparaissait à la fin).
6. **P8 HDA, P10 scanout 2D/3D** : non écrits. **P14** : non repéré.

## 5. Prochaine commande exacte

```bash
# lire la campagne Ladybird de d466c9ff, puis :
gh api repos/bcharthur/bouchaud-os/actions/workflows/endurance.yml/dispatches \
  -X POST -f ref=claude/ladybird-observability-performance -f 'inputs[minutes_par_tranche]=30'
```

Ne JAMAIS relancer `os-primitives.yml` à la main pendant un run
`ladybird-native-browser` : même groupe de concurrence, le job
`primitives / probes` du run Ladybird est annulé (vu sur 38447976372).

Reproduction locale du Crash Guard (TCG, ≈ 30 s par topologie) :

```bash
cargo bootimage
BO_SMP=1 tools/ci/run_faute_multifil.sh target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin 2
BO_SMP=8 tools/ci/run_faute_multifil.sh target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin 2
```

## 6. Physique (TRIGKEY)

Aucun test physique, aucun flash. L'image de référence se prend dans
l'artefact `bouchaud-ladybird-native-browser` (avec
`BOUCHAUD_ARTIFACT_MANIFEST.json`) d'un run Ladybird **entièrement vert**,
puis `docs/current/RELEASE_TRIGKEY_CHECKLIST.md`. Flash et démarrage : Arthur
seulement. `PHYSICAL_STATUS=BLOCKED`.
