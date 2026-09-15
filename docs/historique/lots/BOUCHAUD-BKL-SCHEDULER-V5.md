# Bouchaud OS — BKL Scheduler Fragmentation V5

Ce patch **ne cherche pas encore à modifier l'algorithme de context switch**.
Il fragmente précisément le domaine que les logs Google rendent suspect et
ajoute les métriques nécessaires pour distinguer deux causes très différentes.

## Nouvelle arborescence

```text
src/kernel/sync/bkl/
├── ordonnanceur.rs             # façade
└── ordonnanceur/
    ├── etat.rs                 # RESUME_WAITERS + compteurs scheduler
    ├── priorite.rs             # publication / anti-barging
    ├── trace.rs                # contrat + logs dédiés
    ├── suspend.rs              # suspend_for_schedule()
    └── resume.rs               # resume_after_schedule()
```

`attente.rs` ne contient plus la politique de priorité scheduler.
`enregistreur.rs` ne contient plus `note_switch()`.

Tous ces fichiers sont toujours `include!` dans le **même module Rust `bkl`** :
aucune migration d'API publique et aucune nouvelle frontière de visibilité.

## Nouveaux logs

### `[BKL-SCHED]`
Résumé global :
- suspensions profondeur > 0 / profondeur 0 ;
- `switch_context` avant/après ;
- `resume` commencées/réussies ;
- reprises encore en vol ;
- temps total/max d'attente de reprise ;
- nombre d'itérations avant acquisition.

### `[BKL-SCHED-RESUME]`
Émis pour chaque continuation réellement en attente :
- CPU ;
- âge ;
- profondeur à restaurer ;
- tentatives ;
- CPU garé ou non ;
- OWNER courant.

### `[BKL-SCHED-OWNER]`
Le plus important pour le P0 actuel.

Il n'est émis que si OWNER a été acquis par `resume_after_schedule`.

Exemple :

```text
[BKL-SCHED-OWNER]
cpu=0
post_resume_hold_ns=4100000000
last_resume_wait_ns=3000000
last_depth=1
last_attempts=2
...
```

Cela prouverait :

- `resume_after_schedule` a attendu seulement 3 ms ;
- il a bien réussi ;
- **le code repris garde ensuite le BKL 4,1 secondes**.

À l'inverse, si `BKL-SCHED-RESUME age_ns` monte à plusieurs secondes avant
`RESUME_OK`, le bug est réellement dans la réacquisition/handoff.

Cette distinction manquait dans les anciens logs : `origine=resume_after_schedule`
signifie seulement que la *prise* du BKL vient d'une reprise, pas forcément que
la boucle de reprise a duré tout le stall.

## Test

Après extraction à la racine :

```powershell
git status --short
git diff --check
.\run.ps1 -Ladybird -LadybirdUrl "https://www.google.com/" |
    Tee-Object -FilePath bkl-scheduler-v5-google.log
```

Attendre Google puis provoquer le freeze souris/clavier.

Les lignes prioritaires à récupérer sont :

```text
[BKL-SCHED]
[BKL-SCHED-RESUME]
[BKL-SCHED-OWNER]
[BKL-HEALTH]
[SMP-STALL]
[SMP-PROV]
```

## Important

Ce V5 est volontairement un **refactor + observabilité**. Il ne supprime pas
automatiquement un OWNER vieux et ne force pas une libération du BKL : le faire
avant de savoir si le temps est passé *dans* `resume_after_schedule` ou *après*
réintroduirait des races SMP.

## V5.1 — correction E0753

Les fragments `ordonnanceur/*.rs` sont injectés avec `include!` dans un module
`bkl` qui contient déjà des items. Les commentaires `//!` y sont donc invalides
car ce sont des commentaires rustdoc internes au module.

V5.1 remplace uniquement ces `//!` par `//`. Aucun code ni comportement du BKL,
du scheduler, de l'anti-barging ou des métriques n'est modifié.
