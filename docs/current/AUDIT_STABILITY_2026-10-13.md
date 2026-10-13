# Audit de stabilité — 13 octobre 2026

> Relevé fait au début de la session « toutes les P, zéro faux vert », puis
> mis à jour à chaque correctif. Les preuves de l'**ancien** HEAD sont
> séparées de celles du **HEAD courant**. Niveaux : STATIC / HOST / TCG / KVM
> / PHYSICAL. Aucun pourcentage de « stabilité » n'est donné : seulement des
> verdicts mesurés et leurs limites.

## 1. Identité

| Élément | Valeur |
|---|---|
| Dépôt | `bcharthur/bouchaud-os` |
| Branche | `claude/ladybird-observability-performance` (jamais `main`) |
| HEAD de départ (remote) | `0a83f1822d3d4f1701dbfbefc943ca264e5efacb` (docs P19) |
| Checkout local au départ | `9e086175`, 41 commits en retard, arbre propre → avance rapide (`--ff-only`), aucune modification perdue |
| `origin/main` | `788c5446b85a7fe131538eb37c37ba839ac42748` (ancêtre de la branche) |
| Ladybird épinglé | `cdfe5f858eb5fc64a8d9d3fcc247d71b03fbd1f6` (`third_party/UPSTREAM.md`) |
| Worktrees | principal ; `scratchpad/wt-origin` (détaché, ancien) ; `scratchpad/wt-neg` (contrôle négatif, jetable) |

## 2. CI au point de départ — rouges examinés

| Run | SHA | Verdict | Cause établie | Niveau | Correctif |
|---|---|---|---|---|---|
| CI Fast 38301361569 | `6e881ba` | FAIL 166/167 | `verifie-preemption-noyau.py` : IF rouvert dans `kill_faulting_task` (Crash Guard P19) | STATIC (reproduit local) | `9728a4d9` (code gardé par `from_user`, garde durci : TOUTES les réouvertures, IST) |
| Reliability V3 38301361804 | `6e881ba` | FAIL | même garde (« Architecture + host properties ») | STATIC | `9728a4d9` |
| Nightly 38378134772 / 38231350694 / 38127619243 | `788c544` (main) | FAIL `defilement maillon=wm_vers_navigateur` | **défaut de banc** : le job Nightly n'installait ni `socat` ni `nc` ; aucune molette envoyée (`souris=0`, aucune ligne GUI-WHEEL), verdict imputé au produit | HOST (journal + workflow) | `bf4a40b5` (banc refuse sans socat/nc, Nightly les installe) |
| Endurance hebdo 38130602802 | `788c544` (main) | FAIL SMP1/2/4/8 | **pas un blocage** : QEMU sort en 29 s `rc=0`, `autorun_statut=1` ; `ordonnanceur-probe` rouge (1/4 vérifications). Laquelle : non lisible (artefact inaccessible, journal = comptes seulement) | HOST | `ab9c4dda` (le rapport nomme la vérification) ; non reproduit en local (SMP1, 2 essais) — **à rejouer en CI** |
| Reliability V3 38217842305 | `788c544` (main) | FAIL hosted soak, cycle 6/6 | `KERNEL PANIC cpu=1 courant.rs:276:5` = `lockdep::before_acquire(SchedulerTransition)` : une transition d'ordonnanceur ouverte avec un verrou rangé tenu sur ce cœur (message lockdep complet non lisible : artefact inaccessible) | HOST | **OUVERT** — non reproduit : 11/11 cycles locaux sur `0a83f182` (TCG SMP4). Reliability V3 du 13 oct (même SHA main) : SUCCESS |
| CI Fast 38302132898 | `0a83f18` (docs) | SUCCESS, jobs code `skipped` | ne revalide rien | — | — |

Ladybird P19 `6e881bae` (run 38301361986) : **16/16 jobs verts, verdict de
convergence vert**, mémoire `MEMOIRE_BORNEE` (WebContent phase 2 −856 Kio,
20/20 swaps, Compositor 2/2/4623 Kio). Mais CI Fast et Reliability étaient
rouges sur ce SHA : la campagne **n'est pas** une campagne same-HEAD complète.

## 3. Bugs trouvés pendant la session (au-delà des rouges CI)

| BUG | Sévérité | Symptôme | Cause | Preuve avant → après | Commit |
|---|---|---|---|---|---|
| FMF-1 (Crash Guard) | P0 | #UD/#GP/#DE dans un processus **multi-fils** → KERNEL PANIC `ordonnancement.rs:277` | `exit_group` depuis le gestionnaire avec IF=0 → `attend_extinction_freres` → `schedule()` | TCG SMP4 : panique à la 1re faute → 16/16 OK | P19 `8e1bdf97` (correctif) + `9728a4d9` (borne ring 3, garde, sonde) |
| FMF-2 (sigsuspend) | P1 | faute d'un **fil frère** : processus jamais mort (30 s → SIGKILL du banc) | `pause()`/`rt_sigsuspend` boucle sans relire la condamnation ; la tâche reste prête et n'atteint jamais la frontière | TCG SMP4 : 8/8 fautes de frère non terminées → 16/16 | `56f12ef7` |
| BSP-V8 | P0 | SMP1 : un fil de calcul pur gèle tout l'espace utilisateur ; SMP8 : fil condamné immortel (> 4 min), père affamé, 6 cœurs au repos | `BSP_DEFER_DIRECT_IRQ_PREEMPT_V8 = true` : mode **diagnostic** jamais retiré ; CPU0 ne préempte jamais une tâche utilisateur depuis IRQ | SMP1 : échéance sans faute → 24/24 ; SMP8 : bloqué → 24/24 | `c9572100` |

Ces trois défauts étaient invisibles des bancs existants : `faute-noncanonique-probe`
ne fait fauter que des processus mono-fil. La nouvelle sonde
`faute-multifil-probe` les attrape tous les trois (os-primitives, et étape
SMP1/SMP8/KVM dédiée).

## 4. Inventaire de vérité (affirmation → preuve → état au HEAD courant)

| Affirmation historique | Preuve vérifiée + SHA | État au HEAD courant |
|---|---|---|
| « Il faut supprimer le BKL » | `docs/AUDIT_12_CHANTIERS.md` : 0 acquisition `smp_lock` ; garde `verifie-bkl-supprime` (B8–B11, `d768baf8`) | **Aucun BKL** sur les chemins actuels. Ne pas relancer ce chantier ; mesurer les verrous rangés (`lockdep`, 5 classes) |
| « P13 convergé » | run 38301361986 (`6e881bae`) : verdict vert ; 38061564487 (`788c5446`) vert | HISTORIQUE. Rouges CI Fast/Reliability sur le même SHA → **pas** qualifié. À rejouer sur le HEAD de cette session |
| « P2/P9 rouge (+11,4 Mio) » (STATUS_CURRENT, P1_P13_CURRENT) | rouge à `3658af04` ; `MEMOIRE_BORNEE` à `6e881bae` | **Périmé** : docs `current/` décrivent `3658af04`. À rejouer au HEAD |
| « Compositor remplacé PID 18 → 28 (KVM A) » | `3658af04`, run 37800200518 | A/B KVM vert à `6e881bae`. Cause non établie dans les docs ; à surveiller |
| « Crash Guard : le noyau ne meurt plus d'une faute ring 3 » | P19 : un WebContent récupéré | **Vrai à TCG** pour #UD/#DE/#GP/#PF, fil principal ET frère, SMP1/4/8, après `56f12ef7`+`c9572100` (sans eux : processus immortels) |
| « ONE-LOADER corrige la double barre » | commit `8e1bdf97`, vérification de source | STATIC seulement. **Aucune preuve de rendu QEMU** |
| « BSP report V8 : mitigation temporaire » | lot V8 (docs/historique) ; gel global résolu ailleurs (`9f7c5cf8`) | Retiré (`c9572100`) ; à qualifier en CI (soak, endurance, Ladybird) |

## 5. Triage par sévérité (P0 sûreté / P1 fonctionnement / P2 qualité)

**P0** : panique lockdep `courant.rs:276` (non reproduite) ; régressions
éventuelles du retrait V8 (à mesurer en CI).
**P1** : endurance hebdo `ordonnanceur-probe` 1/4 (CI seulement) ; P19 ONE-LOADER
non prouvé au rendu ; LibJS `Value.cpp:484` (WebContent) ouvert ; YouTube en
squelette sous TCG.
**P2** : statut de sortie des fautes ring 3 toujours `SIGSEGV` (Linux :
SIGILL / SIGFPE / SIGSEGV) ; performances TCG (P19 8 pages : expérience).

## 6. Graphe de dépendances (mesuré, à ajuster)

```text
Crash Guard + sigsuspend + préemption BSP (lifecycle/scheduler)
   └─> CI Fast / Reliability / os-primitives verts au même SHA
        └─> campagne Ladybird same-HEAD (P2/P9 mémoire, Compositor, P13)
             └─> P19 (ONE-LOADER au rendu, chargements par onglet, Value.cpp:484)
                  └─> image QEMU exacte → TRIGKEY (physique, par Arthur)
                       └─> P8 HDA, P10 virtio-gpu 2D
```

## 7. Expériences en cours et suivantes

1. Campagne CI complète au HEAD de cette session (CI Fast, Reliability V3,
   Integration, ladybird-native-browser + os-primitives) : même SHA.
2. Endurance hebdomadaire (`workflow_dispatch`) sur la branche : le rapport
   nomme désormais la vérification rouge.
3. Soak 20 min répété (Reliability V3) sur le HEAD : la panique lockdep.
4. ONE-LOADER : capture QEMU du noyau recompilé (barre noyau absente sur
   page active).
