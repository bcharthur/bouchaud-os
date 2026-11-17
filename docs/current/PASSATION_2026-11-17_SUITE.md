# Passation du 17/11/2026 (suite) — #329, #330, #331 fusionnées ; #328 et #319 en qualification

Suite de `PASSATION_2026-11-17.md`. Écriture GitHub rétablie dans cette
session (fusion, création de PR, push).

## 1. Fusions et CI de main

| PR | SHA fusionné | CI au SHA de la PR | CI de main après fusion |
|---|---|---|---|
| #329 Nightly provenance Ladybird | `6d94bcf` | `dbbe3bc` : 167 contrôles verts/sautés | tout vert (Ladybird annulé par le push suivant, voulu) ; **Nightly Full planifié vert sur `427fa15`** |
| #330 processus jamais détruits | `1f97b4b` | `13a83b3` : 147 contrôles verts/sautés (SMP8 relancé une fois : action `download-artifact` injoignable, aucun code exécuté) | tout vert (Ladybird annulé par le push suivant) |
| #331 gel #283 (registre + fenêtre de migration) | `427fa15` | `619da3c` : 17 runs push + PR verts au premier essai | 9/10 verts, Ladybird en file au moment d'écrire |

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

## 4. #328 et #319

* Les deux branches ont reçu main (`427fa15`) sans conflit ; gardes 183/183,
  compilation OK.
* **#328** : défaut réel trouvé et corrigé (`4dda074`). `releve_fils_qemu.sh`
  doit rendre une ligne de verdict sur stdout ; le relevé QEMU des vCPU
  ajouté devant y imprimait `[QEMU-CPU]`, et `run_releve_fils_e2e.sh`
  concluait « Alt+SysRq n'a publié aucun relevé ». Lignes passées sur
  stderr ; local avant ÉCHEC, après `RELEVE_FILS_E2E_OK fils=11`. Retours
  Codex P1 (IST NMI) et P2 (`indetermine`) vérifiés dans le code et résolus.
* **#319** : CI en cours à `fe2e645`, aucun échec à ce SHA au moment d'écrire.
* Les runs de #328/#319 aux anciens SHA ont été annulés pour libérer les
  runners (ils testaient une base périmée).

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
