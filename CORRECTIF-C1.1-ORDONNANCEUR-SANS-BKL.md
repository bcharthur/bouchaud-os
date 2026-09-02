# C1.1 — Ordonnanceur hors BKL

Base de reference : `6d67ef8d1d350965e169b319eae2de8c682df5b0`.

## Resultat

Le domaine `Ordonnanceur` ne contient plus aucun site d'acquisition du Big
Kernel Lock. Les interruptions timer et reschedule, l'election, le vol de
travail, la preemption ring 3, la commutation et les boucles scheduler des AP
s'appuient maintenant sur des invariants locaux ou par objet.

Le nombre de sites BKL d'architecture passe de 27 a 17. Le budget ne contient
plus `Ordonnanceur` et son contrat devient `Migre` : toute reprise du verrou
dans ce domaine est desormais une regression mesurable et bloquante.

## Invariants introduits

1. Une porte `TRANSITION_ORDONNANCEUR[cpu]` interdit la reentree du scheduler
   sur un meme CPU. Elle reste publiee pendant le changement physique de pile
   et n'est rendue que par la continuation entrante.
2. Une tache elue est revendiquee par CAS `on_cpu: -1 -> cpu` pendant que son
   garde generationnel est encore vivant. Deux CPU ne peuvent donc jamais
   executer la meme incarnation et le recyclage ABA reste ferme.
3. La tache sortante conserve `on_cpu == cpu` et `switching_out == true`
   jusqu'a l'abandon effectif de sa pile. Elle n'est republiee qu'apres la
   passation.
4. Le coeur de `schedule` exige une profondeur BKL nulle. Les rares appelants
   legacy suspendent leur profondeur a la frontiere et la recuperent au retour;
   ce repli est compte par `detach_bkl_legacy`.
5. Les alarmes POSIX vivent sous `SchedulerAlarms(5)`, apres la transition
   locale `SchedulerTransition(1)`. Lockdep et le garde-fou source imposent cet
   ordre.
6. Le balayage des echeances est revendique par CAS. Un seul CPU balaie et une
   echeance armee pendant le scan ne peut plus etre ecrasee par le recalage.
7. Tous les reveils concurrents concernes arbitrent `Blocked -> Ready` par CAS;
   une tache n'est publiee qu'une fois dans sa runqueue.
8. Le bottom-half d'entree IRQ ne reprend plus le BKL : le registre
   generationnel et les transitions atomiques de `WaitQueue` suffisent.
9. Une tache terminee abandonne explicitement la profondeur BKL du syscall
   legacy avant son dernier choix d'ordonnancement. Sa pile ne revenant jamais,
   aucun `Drop` ulterieur ne pourrait rendre ce verrou a sa place.
10. Le choix final d'une tache zombie ouvre lui aussi la porte locale avant
    l'election. Si une remplacante est trouvee, la continuation entrante rend
    la porte apres le changement de pile ; sinon le chemin la rend lui-meme.

## Preuves ajoutees

- `tools/smp/test_scheduler_sans_bkl.rs` falsifie la double revendication, la
  reentree IRQ, la publication avant abandon de pile, le reveil pendant une
  passation, la sortie definitive d'un syscall legacy et l'ouverture de sa
  transition locale avant la derniere commutation.
- `tools/smp/test_echeances.rs` couvre maintenant le balayage concurrent et la
  conservation d'un armement publie pendant le recalage.
- `tools/verifie-ordonnanceur-sans-bkl.py` exige zero acquisition, le CAS de
  revendication, la porte per-CPU, le verrou des alarmes, le balayage revendique
  et les reveils conditionnels.
- `tools/verifie-rangs-verrous.py` reconnait les verrous scheduler manuels et
  classes.
- `tools/dev/validate-fast.ps1` compile et execute le nouveau modele hote.

## Validation de cloture

Depuis PowerShell, a la racine du depot :

```powershell
& ".\VALIDER-C1.1-ORDONNANCEUR-SANS-BKL.ps1"
```

Le script execute les garde-fous, les budgets source, toutes les suites de
`validate-fast`, la bootimage, puis `mm-ng6` sous Ubuntu/WSL et QEMU TCG SMP4.
Le marqueur final attendu est :

```text
C1_1_ORDONNANCEUR_SANS_BKL_OK
```

`-SansQemu` permet une validation compilation/hote, mais ne remplace pas la
preuve runtime SMP4.

## Suite du chantier

C1.1 ne pretend pas supprimer le BKL du noyau entier. Les 17 acquisitions
restantes appartiennent encore aux domaines `Fd`, `Panique`, `Processus`,
`Readiness`, `Securite` et `Vm`. Le lot logique suivant est C1.2 : retirer les
tables processus/FD du verrou global, puis rendre les points surs de preemption
noyau effectivement utilisables autour de ces verrous par objet.
