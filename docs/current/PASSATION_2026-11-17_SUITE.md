# Passation du 17/11/2026 (suite) — #329, #330, #331, #328, #319, #332 fusionnées

Suite de `PASSATION_2026-11-17.md`. Écriture GitHub rétablie dans cette
session (fusion, création de PR, push).

## 1. Fusions et CI de main

| PR | SHA fusionné | CI au SHA de la PR | CI de main après fusion |
|---|---|---|---|
| #329 Nightly provenance Ladybird | `6d94bcf` | `dbbe3bc` : 167 contrôles verts/sautés | tout vert (Ladybird annulé par le push suivant, voulu) ; **Nightly Full planifié vert sur `427fa15`** |
| #330 processus jamais détruits | `1f97b4b` | `13a83b3` : 147 contrôles verts/sautés (SMP8 relancé une fois : action `download-artifact` injoignable, aucun code exécuté) | tout vert (Ladybird annulé par le push suivant) |
| #331 gel #283 (registre + fenêtre de migration) | `427fa15` | `619da3c` : 17 runs push + PR verts au premier essai | tout vert, Ladybird compris ; Nightly Full et ladybird-sync planifiés verts |
| #328 relevé NMI + rapport de gel | `504fdf1` | `4dda074` : 12 runs verts au premier essai | tout vert |
| #319 signaux, fil principal | `0243643` | `e9e0ce5` (main `504fdf1` fusionné) : 15 runs verts, dont une relance (voir §4) | 10/10 verts |
| #332 oracle QEMU du banc souffrance | `5471368` | `48972d5` : 10 runs verts, Native IPC après une relance (bureau B3, voir §3) | à vérifier |

`integration.yml` garde les DEUX jobs `tempete-exec` et `fenetre-migration`
et les deux entrées du gate.

## 2. Ce qui a changé dans #331 pendant la qualification

* **Témoin `fenetre-migration`** : l'oracle exigeait une migration
  observée. Elle relève du hasard de l'ordonnanceur (CI : SMP2 0, SMP4 1 ;
  local : SMP2 toujours 0, même fils répartis et sommeils ; SMP4 0 à 5).
  Nouvel oracle **déterministe** : `fenetres_ouvertes=0` — chaque fenêtre
  lecture-du-cœur/marque traversée interruptions masquées (seule une IRQ
  commute un fil noyau). Migrations publiées, plus exigées.

  | | SMP2 | SMP4 |
  |---|---|---|
  | corrigé | OK ×2 | OK ×3 |
  | négatif (fenêtre rouverte, même banc) | ÉCHEC 28 786 / 88 440 fenêtres ouvertes | panique `preempt_count underflow` ×3 |

* Le script classe une panique même quand QEMU (`-no-reboot`) sort avant
  la relecture du journal.
* **Constat ordonnanceur (non traité)** : en SMP2, des fils noyau migrables
  (`aff=0x3`), nés sur des cœurs différents et qui dorment périodiquement,
  ne changent JAMAIS de cœur, même au réveil, en 30 s. À instruire : vol de
  travail ou placement au réveil des fils noyau en SMP2.

## 3. Échecs CI analysés, non liés aux diffs (relancés une fois)

* `reliability / host` : délai de 18 min atteint dans
  `tools/update/test_signature.rs` (ed25519 compilé sans optimisation :
  **150 s** en local) sur runner lent. Marge mince : compiler les tests hôte
  en `-O` ou relever le délai — **à faire à part**.
* `ab-rollback (recovery-rollback, nvme)` : `attendu="R B=" obtenu="A= A="`,
  la touche `r` doit être lue par le chargeur UEFI avant le noyau : course
  du banc.
* `primitives / probes`, étape « réveil derrière un cœur muet », rouge une
  fois sur `7553fba` : rejouée en local (TCG) production SMP2/SMP4 OK,
  témoin ÉCHEC attendu, observateur OK ; verte au SHA suivant. Journal
  complet (7 000 lignes) et artefact (hôte Azure refusé par le proxy de la
  session) non lus : cause exacte non établie.

* `native-ipc-runtime`, bureau ring 3, **bras B3 seul** (`BOUCHAUD_BUREAU_NOYAU_REFUSE
  raison=ecran-au-compositeur`, `COMPOSITED_BUREAU_FAIL raison=ecran`,
  scénario incomplet ; B4, B0, A verts), une fois sur #332 à `48972d5`,
  vert à la relance et sur main au même code noyau. **Non analysé** : à
  instruire s'il revient (course entre le bureau noyau et `composited` pour
  l'écran ?).

## 4. #328 et #319

* **#328** : défaut réel trouvé et corrigé avant fusion (`4dda074`).
  `releve_fils_qemu.sh` doit rendre une ligne de verdict sur stdout ; le
  relevé QEMU des vCPU imprimait `[QEMU-CPU]` devant, et
  `run_releve_fils_e2e.sh` concluait « Alt+SysRq n'a publié aucun relevé ».
  Lignes passées sur stderr (local : ÉCHEC avant, OK après). Retours Codex
  P1 (IST NMI) et P2 (`indetermine`) vérifiés dans le code et résolus.
* **#319** : qualifiée sur l'arbre exact fusionné (main avec #328). Un
  échec, relancé une fois et vert : `qemu / noyau fige, proprietaire du
  verrou`, oracle QEMU de #328 tiré au sort (voir #332).
* **#332 (fusionnée)** : l'oracle QEMU du banc souffrance prenait deux
  échantillons et exigeait le détenteur au même RIP dans `detenteur` ; le
  détenteur sert ses IRQ (IF=1). Six échantillons, trois exigés ; le banc
  efface aussi le journal et le relevé d'un run précédent (en local, la
  boucle d'attente lisait l'ancien journal : témoin « non armé »). Local :
  4/4 OK enchaînés.

## 5. #283 : NE PAS FERMER

Correctifs fusionnés, preuve TCG locale + CI. Manque : une campagne Ladybird
sur main (endurance KVM, batterie de workers, perf avant/après) au SHA
`427fa15` ou suivant, puis WHPX / matériel.

## 6. Ladybird / performances (priorité suivante)

Piste mesurée : chaque processus du navigateur coûte 11–18 ms de copie
d'espace (`fork`) + 7–10 ms de libération à l'`execve` qui suit.

* `posix_spawn` émet `clone(CLONE_VM|CLONE_VFORK)` ; `proc_clone`
  (`src/compat/linux/mod.rs`) renvoie tout appel sans `CLONE_THREAD` vers
  `sys_fork` : **copie immédiate complète** (`AddressSpace::duplicate`).
* Un vrai `CLONE_VFORK` (fils sur le `Mm` du père, père suspendu jusqu'à
  `execve`/sortie du fils — obligatoire : la glibc libère la pile du fils au
  retour de `clone`) supprimerait les deux coûts.
* **Prérequis bloquant** : `sys_execve` remplace l'espace **en place** dans
  le `Mm` (`core::mem::replace(&mut mm.space, space)`, après avoir arrêté
  « tous les fils sur l'ancien CR3 »). Sur un `Mm` partagé, l'`execve` du
  fils détruirait l'espace du navigateur. Il faut d'abord rendre le `Mm`
  d'un processus remplaçable (`Process.mm` : 55 usages), puis faire de
  l'`execve` d'un fils vfork l'installation d'un `Mm` neuf. PR dédiée, avec
  témoin négatif (père qui écrit pendant que le fils tourne).
