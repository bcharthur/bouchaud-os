# Continuation synchrone : `run_noyau` repris par une tache etrangere

BOUCHAUD_CONTINUATION_SYNCHRONE_V1, BOUCHAUD_SORTIE_NON_PREEMPTEE_V1 --
1er octobre 2026, base `b6d0f21b`.

## 1. L'incident physique (TRIGKEY)

Google charge, puis l'interface se fige. Journal serie a T+60,519 s :

```
RUN_NOYAU_RETOUR t=60519 nom=desktop pid=13 code=0 fil_mort=0 processus_vivants=14
PROCESS_KILL ... raison=run_noyau_retour parent=desktop      x14
[STAGE2] window manager exited
```

`fil_mort=0` : le fil du bureau etait VIVANT quand `run_noyau` a repris la
main. La telemetrie UDP du meme demarrage montre la suite : heartbeats, BRDP et
`AUDIT_SAIN` continuent jusqu'a T+965 s alors que `net-*`, `bouchaud-auditd`,
`bouchaud-telemetrie` et `bouchaud-brdp` ont ete « tues ».

Explication complete, verifiee dans le code :

* `process::kill(pid)` ne fait que retirer la ligne de la table d'affichage, et
  `PROCESSES.lock().clear()` vide la table des processus : aucune tache n'est
  arretee ;
* `stage2::run` imprime `window manager exited` puis fait `cli; hlt` en boucle
  sur le coeur zero. Le bureau, epingle au coeur zero, ne tournera plus jamais :
  c'est l'ecran fige ;
* les travailleurs migrables continuent sur les AP : c'est la telemetrie vivante.

Les erreurs Ladybird `RequestServer is currently unavailable` suivent cette
destruction ; elles en sont la consequence.

## 2. Audit (lecture seule, code de `b6d0f21b`)

1. **Chemins vers `switch_to_kernel()`** : `exit_current` (via `exit`,
   `exit_group`) -- branche AP (`cpu != 0`), branche BSP `racine == 0`, voie
   directe de la racine de `run`, sortie de la boucle d'attente du BSP -- et
   `retire_exec_zombie_current` (fil tue par `execve`, perdant d'un
   `exit_group`).
2. **Qui ecrit `KERNEL_CTX[cpu].rsp`** : `switch_context` depuis
   `secondary_cpu_loop` (AP), depuis `run` et depuis `run_noyau` (BSP).
3. **Qui le lit** : `switch_to_kernel` (cible du saut) et `kernel_ctx_rsp`
   (journaux).
4. **Proprietaire** : aucun. Un contexte PAR COEUR, ecrase par le dernier qui
   s'y gare.
5. **Plusieurs taches vers le meme contexte** : oui. Toute tache qui meurt sur
   le coeur zero -- epinglee ou volee -- vise `KERNEL_CTX[0]`.
6. **Une tache meurt sur le coeur zero pendant que `run_noyau` attend le
   bureau** : `RACINE_PREMIER_PLAN` vaut 0 (seul `run` la pose), donc branche
   `racine == 0` : `commute_sortie_definitive_si_possible`, puis, si rien
   d'autre n'est pret, `switch_to_kernel()` -- qui saute dans la continuation
   de `run_noyau`.
7. **Ce qui garantissait que seul le bureau y revienne** : rien. Et l'inverse
   non plus : si la racine mourait pendant qu'une autre tache etait prete, elle
   partait vers elle, et la continuation restait garee jusqu'a la prochaine mort
   sur un coeur zero vide.
8. **`fil_mort=0`** : `run_noyau` lit `lifecycle.threads == 0` juste apres la
   reprise. Le bureau vivait (`threads=1`) : la reprise venait d'une autre tache.
   La ligne `PROCESS_EXIT` qui precede `RUN_NOYAU_RETOUR` dans le journal serie
   physique nomme la tache etrangere ; elle n'est pas dans l'extrait recu.

## 3. Temoin negatif, dans le vrai noyau

Banc `continuation-banc 20` (autorun, QEMU TCG, protocole d'origine) : une
racine `run_noyau` epinglee au coeur zero reste vivante et dort pendant que
vingt taches courtes epinglees au coeur zero meurent ; puis elle sort pendant
qu'une tache du coeur zero est prete en permanence.

| SMP | protocole d'origine |
|---|---|
| 1 | racine sortie `code=7`, `run_noyau` **jamais repris** (continuation perdue) |
| 2 | idem |
| 4 | `SWITCH_TO_KERNEL cpu=0 pid=14 kernel_ctx_owner=13 etranger=1` puis `RUN_NOYAU_RETOUR fil_mort=0` des le 1er fil court |
| 8 | idem |

Les deux faces du meme defaut. La seconde est aussi le constat SMP1 du lot B12.

## 4. Correction

Modele de propriete :

* `KERNEL_CTX[cpu]` est l'**idle du coeur**, et rien d'autre. Les AP l'avaient
  (`secondary_cpu_loop`). Le BSP n'avait que la pile d'amorcage : il recoit une
  boucle idle dediee (pile de 64 Kio, amorcee une fois par
  `assure_idle_coeur_zero`), commune avec celle des AP (`boucle_idle`).
* `CONTINUATION` est la pile d'amorcage garee par `run` / `run_noyau`, avec son
  proprietaire : pid racine et coeur. « Racine terminee » veut dire : plus
  aucun fil d'execution non zombie du PROCESSUS racine -- pas sa descendance,
  que le teardown de session a deja arretee (`run` ne l'a jamais attendue :
  BOUCHAUD_C71). Etats LIBRE ->
  GAREE (contexte d'amorcage) -> LIBRE (consommee une fois, par CAS).
* Elle n'est reprise que si `continuation::reprenable` : racine terminee, sur
  le coeur proprietaire. Deux endroits seulement : la mort qui la rend due
  (`reprend_continuation`, avant toute commutation), ou la boucle idle de son
  coeur (racine morte sur un autre coeur, qui le reveille par IPI).
* Toute autre mort sans successeur pret va a l'idle de son coeur.
* `consomme_continuation` panique si elle n'est pas due : une reprise racine
  vivante est une corruption, pas un cas a absorber.
* `run_noyau` ne fait son menage (reap, `PROCESS_KILL`, `clear`) qu'apres
  avoir verifie `fil_mort` ; sinon, panique nommee. `RUN_NOYAU_RETOUR` reste.
* La boucle d'attente sur pile morte de `exit_current`, son garde-fou de 30 s
  et sa commutation sans retour disparaissent : ils n'existaient que parce que
  l'idle du BSP et la continuation etaient confondus.

Aucun verrou global : un etat atomique et un CAS ; la porte de transition par
coeur couvre chaque commutation comme avant.

Second defaut, decouvert par le banc corrige (SMP1, 1 boot sur 3) :
**BOUCHAUD_SORTIE_NON_PREEMPTEE_V1**. Une preemption noyau ciblee a coupe la
racine AU MILIEU d'`exit_current`, apres `marque_zombie`. `complete_switch_handoff`
ne republie que les taches `Ready` : la sortie de la racine n'a jamais ete
terminee, et la continuation n'a ete reprise que par la mort suivante (2 s).
`preemption_noyau_sure` refuse desormais la preemption d'une tache deja marquee
zombie (`RETRAITE_DEMANDEE[cpu]`). La preemption d'un code utilisateur n'est pas
touchee.

## 5. Preuves

QEMU TCG, `continuation-banc 20`, 3 demarrages par configuration :

| protocole | SMP1 | SMP2 | SMP4 | SMP8 |
|---|---|---|---|---|
| origine (`b6d0f21b` + sondes) | continuation perdue | continuation perdue | `fil_mort=0` au 1er fil court | `fil_mort=0` au 1er fil court |
| continuation seule (critere 500 ms) | 2/3 (1 racine coupee en sortie, reprise 2051 ms) | 1/3 (1071 et 1228 ms) | 3/3, 20/20 detours idle | 3/3, 19-20/20 |
| + sortie non preemptee (critere causal) | 3/3, 4-6 ms | 3/3, 5-7 ms, 13-18 detours | 3/3, 4-7 ms, 20/20 | 3/3, 6-11 ms, 20/20 |

Dans aucun des 36 demarrages corriges : `RUN_NOYAU_RETOUR ... fil_mort=0`,
reprise etrangere, panique. `detours_idle` compte les morts qui, avant ce lot,
reprenaient la continuation : elles vont desormais a l'idle du coeur zero.

Autres preuves :

* hote `tools/smp/test_continuation.rs` (7 tests) : incident, sorties
  multiples, racine vivante, racine qui meurt coeur occupe, dernier fil sur un
  AP, 2 000 suites aleatoires -- ancienne regle : 1 546 reprises racine vivante
  et 5 104 pertes ; nouvelle : 0 et 0 ;
* QEMU `continuation-banc`, SMP1/2/4/8 ;
* gardes `tools/verifie-continuation-synchrone.py` (7 negatifs),
  `tools/ci/verifie-retour-shell.py` (3 negatifs),
  `tools/verifie-ordonnanceur-sans-bkl.py` (deux sorties definitives, compte
  exact).

## 6. Identite de construction (`BOUCHAUD_BUILD commit=4de41fde69ba`)

`build.rs` pose `BOUCHAUD_BUILD_COMMIT` depuis `git rev-parse --short=12 HEAD`
**sauf si la variable existe deja dans l'environnement** ; il se relance sur
`.git/HEAD`, `.git/refs/heads` et les variables. Verifie :

* sans la variable, apres un commit, la reconstruction grave le nouveau HEAD ;
* avec `BOUCHAUD_BUILD_COMMIT=4de41fde69ba`, le noyau grave `4de41fde69ba`
  sans un mot -- exactement la ligne du releve physique (4de41fde date du
  27 septembre).

La variable etant retiree de l'environnement, un build neuf publie le HEAD.
Le pipeline n'est pas modifie : le probleme ne subsiste pas apres le nettoyage.
Le risque reste latent : une variable persistante l'emporte en silence.
`IMAGE-TRIGKEY.ps1` lit le commit par git pour son manifeste ; comparer ce
commit a la ligne `BOUCHAUD_BUILD` du noyau suffirait a le rendre bruyant.

## 7. Ce qui reste ouvert

* **Zombies ressuscites (pre-existant, hors de ce lot).** `session-probe 4`
  sous SMP4 : le teardown de session marque les quatre fils zombie (`4
  tache(s) de sa session arretees`), puis trois d'entre eux sont relus
  `Blocked` -- leur `nanosleep` ecrit l'etat par `range` (ecriture simple) et
  efface le `Zombie`. Une premiere version de ce lot attendait la descendance
  de la racine de `run` et a fige le shell sur ce cas ; elle attend desormais
  le seul processus racine, comme avant.

* Apres une VRAIE sortie du bureau, `stage2` fait toujours `cli; hlt` et le
  menage ne fait que vider des tables : les taches des AP continuent. Hors de
  ce lot (comportement de fin de session, pas de l'ordonnanceur).
* La racine d'un `run` dont le dernier fil meurt sur un AP depend de la boucle
  idle du coeur zero : si le coeur zero ne s'endort jamais (tache prete en
  permanence et aucune mort), la reprise attend. Le cas n'existe pas pour
  `run_noyau` (racine mono-fil epinglee au coeur zero).
* Une tache tuee par une autre (zombie non volontaire) en code noyau n'est plus
  preemptee par preemption noyau ciblee jusqu'a sa retraite ; c'etait deja le
  cas de toute tache noyau tenant un verrou.
* La tache etrangere exacte de T+60,519 s n'est pas identifiee (ligne
  `PROCESS_EXIT` precedente absente de l'extrait).
