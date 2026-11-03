# État courant — relevé du 2026-11-03

```text
MAIN_HEAD=c1e55da99dcc411f45ee68ebd285d91205f82067   (runtime identique à 5c3cd43a ; diff = docs + CI)
CANDIDAT_CAP_A=c1e55da (main, figé pendant les campagnes)
BRANCHE_TRAVAIL=claude/cap-stable-platform-20261103
MAIN_PROTECTED=false   (ruleset prêt : tools/github/ruleset-main.json, non appliqué : pas d'API d'administration dans la session)
BASELINE=NOT_ESTABLISHED
P13=PARTIAL — BLOQUÉ : aucun runner self-hosted bouchaud-kvm4 / bouchaud-kvm8
PHYSICAL=PHYSICAL_PENDING (protocole : TRIGKEY_CAMPAIGN.md)
```

Source machine : [status.json](status.json) — **instantané, pas mis à jour automatiquement.**

## Candidat `main@c1e55da` — preuves same-SHA

| Gate | Run | Résultat | Remarque |
|---|---:|---|---|
| CI Fast | 41539949788 | PASS | push main |
| Integration | 41539949905 | PASS | push main |
| Reliability V3 | 41539949936 | PASS | push main |
| Native IPC | 41592664152 | PASS | dispatch same-SHA |
| Security Runtime | 41592667159 | PASS | dispatch same-SHA |
| Nightly Full | 41592645380 | PASS | 9/9 jobs exécutés, smoke Ladybird produit + provenance artefact (run 41530447823, empreinte d'entrées) |
| Ladybird qualification (C22 6×6, primitives P13 strictes, convergence) | 41592642065 | EN COURS au relevé | |
| Endurance (tranches 90 min) | 41592677703 | EN COURS au relevé | |
| C5 KVM (témoin rouge / fenêtre / image ×24) | 41596509318 | EN COURS au relevé | revue statique : [C5_REVIEW_2026-11-03.md](C5_REVIEW_2026-11-03.md) |
| P13 final (KVM4 ×3 campagnes + KVM8 30 min) | — | **NON EXÉCUTABLE** | 4 runs antérieurs annulés après 24 h en file : runners absents |
| PHYSICAL Trigkey | — | PENDING | intervention humaine |

**Run Ladybird 41530447823 (push `5c3cd43a`) : FAIL non produit.** Seul job rouge : « piège du Compositor (diagnostic) », terminé à 6 min avec l'étape encore `in_progress`, étape `always()` des journaux jamais exécutée, journaux 404 → perte du runner (infrastructure). Même arbre vert en 20 min (run PR 41518023294). Classé **NON_QUALIFIANT**, ni PASS ni défaut produit ; rejoué par la qualification 41592642065.

## Ce qui a changé sur la branche de travail (2026-11-03)

- **Défaut sécurité corrigé** : un programme ring 3 non privilégié faisait paniquer le noyau en 2 appels (waitset qui se surveille → `SpinLock recursive acquisition`). Reproduit en TCG, corrigé, test permanent.
- **Défaut cycle de vie corrigé** : les handles natifs d'un processus mort n'étaient jamais libérés à sa mort (`PEER_CLOSED` dépendant de l'activité du reste de la machine).
- **ABI native 1.1** ([NATIVE_ABI.md](NATIVE_ABI.md)) : spawn filié et confiné, objet Process, attente bloquante, temps.
- **Premier service ring 3 supervisé** (`healthd` + `serviced`) : crash réel, hang, budget, arrêt, aucune fuite ; SMP1/2/4/8 TCG + CI.
- **SDK natif** `userland/libs/bouchaud-sdk` (en-tête unique vérifié contre le noyau).
- **Mise à jour A/B** prouvée sur hôte ([UPDATE_RECOVERY.md](UPDATE_RECOVERY.md)), non branchée au chargeur.
- **Image Trigkey traçable** + signature CPUID ; protocole physique exact.
- **Matrice Ladybird générée** ([../ladybird/PATCH_MATRIX.md](../ladybird/PATCH_MATRIX.md)).

Ces changements **ne font pas partie du candidat** `c1e55da` : ils seront qualifiés sur leur propre SHA après fusion.
