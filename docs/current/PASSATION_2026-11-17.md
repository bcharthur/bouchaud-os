# Passation du 17/11/2026 — gel #283 : deux causes trouvées et corrigées (local)

Base : `main@65f1e65`. **Écriture GitHub perdue dans la session** (push et
fusion refusés, 403 « Claude doesn't have GitHub access ») : tout ce qui suit
est committé sur des branches locales, prêt à pousser. Lecture GitHub
(CI, journaux) intacte.

## 1. Le gel #283 : deux défauts noyau, prouvés par reproduction

### 1.1 Cycle registre des tâches / IRQ / `Mm` (le gel total)

Reproduit **en local**, tempête fork+exec (`tools/ci/run_tempete_exec.sh`,
mode `complet`), TCG SMP4, sur `main@65f1e65` **et** sur #330 : sortie série
arrêtée au milieu d'une ligne, QEMU à 170 %, **aucune** ligne
`[DIAG-EN-SOUFFRANCE]` (même forme que l'échec CI du job tempête de #330,
`etat=echeance`, et que les gels « silence total » de #283).

Relevé QEMU des vCPU (moniteur, deux échantillons à 1 s, RIP immobiles),
symbolisé :

| cœur | IF | où | rôle |
|---|---|---|---|
| 1 | 0 | `registre_ajoute_sous_ecriture` → `RegistreEcriture::acquire` | écrivain du registre, attend que les lecteurs partent |
| 2 | 1 | `services-metrics` → `mesure_processus` → `memory_usage` → `Mm::lock` | **tient une lecture** du registre, tourne sur un `Mm` |
| 0 | 0 | IRQ minuteur → `sonde_gel_tic` → `tasks()` → `RegistreLecture::acquire` | lecteur refusé : un écrivain attend |
| 3 | 0 | IRQ reprogrammation → `choose_runq_cpu` → `tasks()` | idem |

Le détenteur du `Mm` est un contexte interrompu par l'une de ces IRQ : il ne
rend le verrou qu'au retour de l'IRQ ; l'IRQ attend l'écrivain (priorité
écrivain) ; l'écrivain attend le lecteur ; le lecteur attend le `Mm`. C'est
exactement la signature historique de #283 (« services-metrics bloqué sur le
`Mm` d'un WebWorker qui vient d'être exécuté »).

Correctif (`BOUCHAUD_REGISTRE_ECRIVAIN_DEUX_TEMPS_V1`) :

* l'écrivain **attend** (`ECRIVAIN`) puis **écrit** (`ACTIF`) ; un contexte qui
  **tient un verrou** (IRQ, verrou tournant, verrou rangé) lit tant que
  l'écrivain n'est pas actif. Exclusion lecteur / écrivain actif par échange
  de Dekker en `SeqCst` (lecteur : compte puis lit `ACTIF` ; écrivain : pose
  `ACTIF` puis relit le compte) ;
* `mesure_processus` ne prend plus aucun verrou de processus sous la lecture
  du registre (`Mm` et métadonnées lus après, sur des `Arc`) ;
* compteur `lectures_porteuses` publié par `[PROC-STAT]` : **10 000 à 13 000
  par tempête**, chacune un contexte porteur de verrou qui aurait attendu un
  écrivain en attente.

### 1.2 Fenêtre de migration des marques par cœur (la panique)

`preempt::disable()` et `SpinLock::lock/try_lock` lisaient l'indice du cœur
**puis** incrémentaient son compteur (`preempt_count`, `verrous_simples`).
Entre les deux, le fil noyau est préemptible (préemption ciblée) : commuté
là, il marque le cœur qu'il quitte. Effets : `preempt_count underflow`
(panique du job `ladybird / perf avant-simple` sur l'image **main**,
run 43708358057, cœur 3) ; un cœur laissé non préemptible ; un verrou tenu
sur un cœur non marqué, donc préemptible verrou tenu.

Correctif (`BOUCHAUD_FENETRE_MIGRATION_V1`) : lecture et incrément
interruptions coupées. `note_fenetre` vérifie ensuite que le cœur courant
est le cœur marqué (`[PROC-STAT] fenetre_migree preempt= verrou=`).

Témoin `banc-fenetre-migration` (jamais livré), `tools/ci/run_fenetre_migration.sh`,
ajouté au gate d'intégration (SMP4 + SMP2) :

| image | résultat |
|---|---|
| ancien code + témoin | **`panicked at src/arch/x86_64/cpu_local.rs:281:9` (preempt_count underflow)**, cœur 2 |
| corrigé + témoin | 4/4 fils, 69 274 fenêtres élargies, 811 préemptions noyau, `fenetre_preempt=0 fenetre_verrou=0` |

Pendant les gels du §1.1, `fenetre_migree` restait à 0 : les deux défauts
sont distincts.

### 1.3 Qualification locale (TCG, hôte 4 cœurs chargé, image main + #330 + correctifs)

| config | runs | gels | cycles `complet` par run | processus en mémoire en fin |
|---|---|---|---|---|
| SMP4 | 16 | 0 | 3 289 – 5 268 | 18 – 22 |
| SMP2 | 4 | 0 | 4 445 – 5 656 | 19 – 20 |
| SMP1 | 3 | 0 | 3 824 – 4 320 | 19 – 20 |

Témoin négatif, mêmes conditions : main / #330 non corrigés, **4 gels totaux
sur 12 runs** (dont 2 relevés vCPU complets).

Niveau de preuve : **QEMU TCG local**. Ni KVM, ni WHPX, ni matériel. #283
ne doit pas être fermée avant la CI au SHA et une campagne Ladybird.

## 2. État des PR et branches (à pousser)

| branche | SHA local | contenu | état |
|---|---|---|---|
| `claude/nightly-provenance` (#329) | `dbbe3bc` (= distant) | Nightly : source Ladybird retrouvée, production déclenchée | **119 contrôles verts**, 49/49 tests ; fusion refusée (403) |
| `claude/fuite-processus` (#330) | `c148fec` (+1) | fuite de processus + tempête : silence déclaré, vCPU relevés, journal dans la sortie du job | push en attente |
| `claude/gel-283-nmi` (#328) | `19a7bc8` (+2) | relevé QEMU des vCPU (ex-`gel-283-qemu-cpus`) ; NMI sur pile IST dédiée ; pile lue seulement en RAM noyau ; verdict `indetermine` sans preuve (retours Codex P1/P2) | push en attente ; témoin local `SOUFFRANCE_VERROU_OK` |
| `claude/fenetre-migration-283` (nouvelle) | voir `git log` | §1.1 + §1.2, témoin, job CI, gardes | push + PR |
| `claude/endurance-smp8-sonde` (nouvelle) | `812f988` | sonde d'ordonnancement : 2 calculs par cœur | push + PR |
| `claude/sigchld-boucle` (#319) | `6a9a183` | fusion de main | push après les correctifs |

Branches `essai/*` : locales, de test uniquement, **à ne pas pousser**.

Ordre de fusion conseillé : #329 → #330 → nouvelle PR #283 (conflit attendu
dans `integration.yml` : garder les DEUX jobs `tempete-exec` et
`fenetre-migration` et les deux entrées du gate) → #328 → #319.

Échecs CI de #328 (95e43cf) : `perf avant-simple` = la panique du §1.2 sur
l'image **main** (pas le code de #328) ; `perf (bilan)` et `verdict
integration developpement` en découlent (pas d'artefact `perf-surface-*`,
source `failure`).

## 3. Endurance hebdomadaire du 15/11 (run 43706702060)

* **SMP8** : 82/82 cycles `NON_QUALIFIANT`, `ENDURANCE_ORDONNANCEUR_OK`
  absent, statut 2, aucune panique. Cause : 8 calculs sur 8 cœurs ne
  surchargent rien, la porte « la charge dégrade la latence » n'est jamais
  franchie. Corrigé dans `claude/endurance-smp8-sonde` (≥ 2 calculs par cœur,
  SMP1–4 inchangés). Non revalidé localement (à faire en CI).
* **SMP4** : 2/146 cycles non qualifiants (sonde), pas de gel.
* **SMP1** : verdict soak OK (172/172), mais `ECHEC budgets` sur tous les
  cycles (8/10 grandeurs mesurées ; écart isolé p99 33,6 ms sous 250 ms) :
  à analyser avec l'artefact (inaccessible depuis cette session).

## 4. #319 (SIGCHLD)

Requalification locale sur main + #330 + correctifs #283 : voir le compte
rendu de session (bancs `run_sigchld_repetition.sh` SMP4/2/1 et tempête).

## 5. Rien n'a été fait pour

Ladybird (profilage/optimisations), E2E-06 à E2E-10, ring 3 (windowd /
composited), P10, stockage, Trigkey, Insights : non commencés dans cette
session. Piste de performance mesurée et non implémentée : `fork` du
navigateur copie 2 000–2 500 pages (11–18 ms) que l'`execve` qui suit libère
(7–10 ms) — un `vfork`/`CLONE_VM` réel ou un COW supprimerait les deux.
