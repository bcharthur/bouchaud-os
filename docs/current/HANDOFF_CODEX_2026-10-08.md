# Reprise Codex — 8 octobre 2026

> **Mise a jour de reprise (19 h 15 Paris) :** lire d'abord
> [Reception SCM_RIGHTS atomique](../handoffs/2026-10-08-scm-atomic-receive.md).
> La campagne instrumentee `37807102697` est terminee et analysee.
> Le correctif noyau minimal `c78be28` est publie ; la sonde renforcee
> `1a47387` passe 10000/10000 sous Bouchaud KVM (run `37814193790`).
> Le replay Ladybird `37813732807` reste en cours : ne pas le declarer valide.
> P13 et P2/P9 restent ouverts. Aucun changement P10, aucun test PHYSICAL.
> La suite de ce fichier decrit l'etat historique a `3658af0`.

**P13 reste ouvert. P2/P9 n'est pas DONE. P10 n'a pas été modifié. Aucun essai PHYSICAL.**

Branche unique : `claude/ladybird-observability-performance`.
HEAD technique de cette reprise : `3658af04f05e04edf732cc086ddc890faaf88c52`.
Le commit qui ajoute ce handoff est documentaire ; il ne constitue pas une nouvelle preuve runtime.
Ladybird épinglé : `cdfe5f858eb5fc64a8d9d3fcc247d71b03fbd1f6`.

## Audit initial et travaux préservés

Premier acte : `git fetch origin`. HEAD distant confirmé : `6fb5792c7cc246715328c2f9069c6833b2bc6076`.
Lecture de BOUCHAUD_AI_CONTEXT.md, tout docs/current/, README, code courant et Actions de
`9e086175c8e77c3a48d11eee8d2aa86442c78d67`. Les deux commits suivants étaient documentaires.

L'ancien checkout `/workspace/scratch/eac78e743345/repo` était à `4de41fde69ba0674b33f942994cd25f22194e037`,
avec neuf fichiers modifiés. Aucun n'a été effacé, écrasé ou poussé :
`src/drivers/network/anneau_rx.rs`, `src/drivers/network/rtl8168.rs`,
`src/gui/client.rs`, `src/gui/services.rs`, `src/gui/window_manager.rs`,
`src/net/diag_distant/telemetrie.rs`, `src/net/mod.rs`, `src/platform/pc/stage2.rs`,
`tools/net/test_anneau_rx.rs`.

Le checkout de reprise est `/workspace/scratch/b84b2d21fe05/bouchaud-os-verified`,
sur la bonne branche. Il était propre au HEAD technique avant les diagnostics non publiés ci-dessous.
Le premier checkout isolé `bouchaud-os-current` conserve un commit local `7a5af9b`
dont l'arbre est identique au commit publié `fd06001`, puis une copie indexée des changements
déjà publiés jusqu'à `3658af0`. Ne pas prendre cette copie de travail pour le HEAD distant.
Un essai de clone incomplet `bouchaud-os` a également été laissé intact.

Le push Git natif n'avait pas d'identifiants. Les commits publiés ont été créés par l'API GitHub,
puis la référence avancée avec SHA attendu et `force=false`. Arbres Git API/local vérifiés égaux.
Aucun travail sur main, force-push, reset, clean ou réécriture.

## Commits publiés, dans l'ordre

| Commit | Objet |
| --- | --- |
| `fd060010e7d1a93ce7dff2faa6aa2718b87ba137` | Preuve cumulative du cycle de vie du Compositor |
| `6f311362316707dc942f004acbb3ec613853b048` | Preuve exacte des swaps, fermetures et collecte des enfants |
| `20addbfb4d6406aa0c681d0f85e1639e9f553490` | Relevés mémoire frais, ordre du journal, refus des données tronquées, deux répétitions |
| `3658af04f05e04edf732cc086ddc890faaf88c52` | Verdict P13 exigeant tous les jobs de la campagne et os-primitives au même HEAD |

Aucun seuil de sécurité, timeout, sandbox, no_new_privs, SMP ou assertion n'a été relâché.
Le vieux banc acceptait jusqu'à deux swaps manquants ; cette tolérance est supprimée.
La tolérance RSS proportionnelle à la croissance précédente est supprimée :
la seconde phase ne dispose plus que du bruit fixe préexistant de 1 MiB.

## 1. Rouge historique Compositor : cause établie

Run historique [37776506426](https://github.com/bcharthur/bouchaud-os/actions/runs/37776506426),
job `113310262125`, artefact `11550463625`.

Le bras A contient bien un Compositor PID 18, avec une ligne intacte
`[LB] PROCESS_CREATE type=Compositor pid=18 total=4` (ligne 235).
La ligne `PERF_EXECVE` utilisée pour compter est entrelacée octet par octet
avec `[SMP-TASK]` et un autre message (lignes 227–231). D'où zéro compté.

Hypothèses réfutées par le code et les fichiers :
collecte démarrée trop tard, perte du début par tee/grep, mélange des bras, absence réelle de création.
Le fichier est tronqué avant QEMU, QEMU écrit directement avec `-serial file:`,
le parser lit tout après l'arrêt et les deux bras sont séquentiels et sauvegardés séparément.

Défaut d'observabilité encore présent : `console_write` contourne le jeton série ;
le chemin UART borné peut aussi émettre sans jeton. Aucun correctif UART n'a été improvisé.
Les compteurs cumulatifs du ProcessManager et leurs séquences périodiques prouvent désormais
créations, retraits, PID vivant, avant workload et après FIN. Une perte de ligne ne remet pas les compteurs à zéro.

Premier replay au HEAD `fd06001`, run [37796070396](https://github.com/bcharthur/bouchaud-os/actions/runs/37796070396) :
- TCG 10 min : 98 cycles, 609 relevés, `created=1 removed=0 live=1 pid=18`, stabilité/performance OK.
- KVM A/B : preuve Compositor OK sur les deux bras (300 / 298 relevés).
- KVM 20 min : 227 cycles, 1 170 relevés, même invariant, stabilité/performance OK.
- A/B néanmoins rouge sur 16/19 lignes de swap du bras A.
  Les trois enfants manquants existent dans les logs : PID 20, 36, 64, cycles 3, 15, 36.
  Cela a motivé le journal cumulatif des swaps, sans baisser l'exigence.

CI Fast `37796070161`, Reliability `37796070373`, Integration `37796070093` : success.
Dans le run Ladybird, tous les autres jobs étaient success ; A/B et convergence failure.
L'ancien job mémoire était affiché success mais constituait un faux vert :
18/20 marqueurs acceptés, +11 MiB WebContent acceptés, et une valeur tronquée « 16 KiB » interprétée comme une baisse.
Cette ancienne couleur verte ne prouve pas P2/P9.

## 2. Nouveau KVM A/B : vrai remplacement, priorité de reprise

Au HEAD `3658af0`, run [37800200518](https://github.com/bcharthur/bouchaud-os/actions/runs/37800200518),
job [113391576610](https://github.com/bcharthur/bouchaud-os/actions/runs/37800200518/job/113391576610) :
- bras A : 58 cycles, 58 frames, 58 workers, **19/19 swaps**, tous fermés et collectés, performance OK ;
- le Compositor PID 18 sort réellement avec `statut=0`, remplacé par le PID 28 ;
- les compteurs cumulatifs refusent correctement cette seconde création ;
- bras B : 19/19 swaps, unique Compositor, stabilité/performance OK.

Trace A autour de 40 s invité, 17:28:09 :
`CONTEXT_DESTROY ctx=9`, `CONTEXT_CREATE ctx=10`,
`CONNECTION_CREATE conn=9 total=3`, `CONTEXT_CREATE ctx=11 page=11 conn=1`,
puis nouveau Compositor PID 28 et `PROCESS_EXIT type=Compositor pid=18 statut=0`.
Aucune assertion ni faute CPU lisible observée.

**La cause de cette sortie normale n'est pas établie.**
Upstream `Services/Compositor/ConnectionFromClient.cpp::die()` notifie les WebContent puis
appelle `Core::Process::terminate_immediately(0)`. La fermeture du canal de contrôle est
donc une hypothèse précise, pas une conclusion. Examiner la fenêtre brute précédant cette sortie,
`LibIPC/Connection.cpp`, `TransportSocket.cpp`, les EOF/erreurs de réception/envoi et les fermetures de descripteurs.
Ne pas transformer cette nouvelle anomalie produit en faux rouge de parser.

## 3. P2/P9 : swaps prouvés, mémoire encore rouge

L'historique 17/20 venait de trois lignes perdues (onglets 5, 7, 14).
Les enfants cross-site correspondants sont présents : PID 23, 25, 32
(`mem1_4`, `mem1_6`, `mem2_3`), ouvreur PID 19.

L'ancien analyseur prenait le dernier événement de la même seconde, y compris APRES le repère.
Exemple historique : M1 à 54.075 s, création du contexte suivant à 54.091 s.
Il utilisait aussi des événements périmés comme instantanés et acceptait des nombres tronqués.

Le nouveau job [113391576473](https://github.com/bcharthur/bouchaud-os/actions/runs/37800200518/job/113391576473)
conserve deux démarrages froids dans l'artefact `ladybird-memoire` (`11561146491`).
**Les deux prouvent 20/20 swaps distincts, 20 fermetures, 20 nouveaux WebContent collectés.**

| Mesure | Répétition 1 | Répétition 2 |
| --- | --- | --- |
| Contextes Compositor M1 → M2 | 2 → 2 | 2 → 2 |
| Surfaces vivantes M1 → M2 | 2 → 2 | 2 → 2 |
| Surfaces KiB M1 → M2 | 4623 → 4623 | 4623 → 4623 |
| Créés / détruits M1 | 22 / 20 | 22 / 20 |
| Créés / détruits M2 | 42 / 40 | 42 / 40 |
| RSS Compositor KiB M0 → M1 → M2 | 41596 → 48104 → 48192 | 49096 → 48000 → 48068 |
| RSS WebContent PID 19 KiB M0 → M1 → M2 | 69528 → 90820 → 102232 | 69536 → 90876 → 102332 |
| Croissance WebContent phase 2 | +11412 KiB | +11456 KiB |

M0 reste **inconclusif** : le redimensionnement initial remplace 10 013 952 par 4 734 400 octets
environ trois secondes après le début ; la pause initiale de cinq secondes ne laisse que deux
échantillons identiques avant M0, alors que le banc en exige trois. M1/M2 sont frais et stables.
Ne pas déclarer DONE sur ces seules données.

La hausse WebContent est reproductible ; les objets Compositor stables ne l'expliquent pas.
Le run KVM 20 min précédent montrait aussi une pente WebContent positive
(170572 → 228152 KiB dans sa seconde moitié, environ 5723 KiB/min),
mais son workload est différent : ce n'est pas une comparaison contrôlée avec le banc mémoire.

Hypothèse à tester : racines PageHost retirées mais pages non encore collectées, ou autre rétention.
Le GC upstream a un timer de 4 s et un watchdog de 15 ticks, soit jusqu'à 60 s selon l'activité.
Ce constat n'autorise ni une attente arbitrairement allongée ni un GC forcé pour obtenir du vert.

## 4. os-primitives : autre preuve série perdue

Job [113390040080](https://github.com/bcharthur/bouchaud-os/actions/runs/37800200518/job/113390040080),
artefact `os-primitives-debug` (`11560312802`), premier démarrage KVM :
`PILE_DE_FAUTE_ECHEC piege_niveau1 absent de la pile symbolisee`.

Le piège volontaire PID 34 a une ligne PROCESS_FAULT complète.
Sa ligne PROCESS_FAULT_PILE est entrelacée avec SCHED-NG-ATTENTE-VIVE.
L'image et la liste d'adresses subsistent sur la ligne suivante, mais le préfixe et le PID sont corrompus.
La première adresse est `0x400000401515`, base `0x400000400000`.
Ne pas reconstituer arbitrairement la ligne pour rendre le test vert.
Les autres sondes de ce démarrage, et le TCG de ce job, ont réussi ; les étapes ultérieures sont skipped.
Le noyau n'a pas changé dans cette reprise. Aucun retry n'a été utilisé pour effacer cet échec.

## Campagne finale du HEAD technique

CI Fast [37800199148](https://github.com/bcharthur/bouchaud-os/actions/runs/37800199148) : success.
Reliability V3 [37800199675](https://github.com/bcharthur/bouchaud-os/actions/runs/37800199675) : success.
Integration [37800199197](https://github.com/bcharthur/bouchaud-os/actions/runs/37800199197) : success.

| Job | Résultat | Preuve |
| --- | --- | --- |
| ladybird / build once | success | [113390038887](https://github.com/bcharthur/bouchaud-os/actions/runs/37800200518/job/113390038887) |
| primitives / probes | failure | [113390040080](https://github.com/bcharthur/bouchaud-os/actions/runs/37800200518/job/113390040080) |
| ladybird / wpt | success | [113391576236](https://github.com/bcharthur/bouchaud-os/actions/runs/37800200518/job/113391576236) |
| ladybird / endurance 10 min | success | [113391576237](https://github.com/bcharthur/bouchaud-os/actions/runs/37800200518/job/113391576237) |
| ladybird / politique de crash des services (diagnostic) | success | [113391576277](https://github.com/bcharthur/bouchaud-os/actions/runs/37800200518/job/113391576277) |
| ladybird / cache et SQL apres redemarrage | success | [113391576278](https://github.com/bcharthur/bouchaud-os/actions/runs/37800200518/job/113391576278) |
| ladybird / robustesse (crash d'un rendu, cadres isoles, cycle des workers) | success | [113391576346](https://github.com/bcharthur/bouchaud-os/actions/runs/37800200518/job/113391576346) |
| ladybird / sites reels | success | [113391576398](https://github.com/bcharthur/bouchaud-os/actions/runs/37800200518/job/113391576398) |
| ladybird / endurance 20 min sous KVM (diagnostic) | success | [113391576422](https://github.com/bcharthur/bouchaud-os/actions/runs/37800200518/job/113391576422) |
| ladybird / memoire apres onglets fermes (diagnostic) | failure | [113391576473](https://github.com/bcharthur/bouchaud-os/actions/runs/37800200518/job/113391576473) |
| ladybird / browser-host smoke | success | [113391576507](https://github.com/bcharthur/bouchaud-os/actions/runs/37800200518/job/113391576507) |
| ladybird / piege du Compositor (diagnostic) | success | [113391576519](https://github.com/bcharthur/bouchaud-os/actions/runs/37800200518/job/113391576519) |
| ladybird / ordre worker (diagnostic) | success | [113391576548](https://github.com/bcharthur/bouchaud-os/actions/runs/37800200518/job/113391576548) |
| ladybird / endurance sous KVM (stabilite + performance) | failure | [113391576610](https://github.com/bcharthur/bouchaud-os/actions/runs/37800200518/job/113391576610) |
| ladybird / performance | success | [113393174533](https://github.com/bcharthur/bouchaud-os/actions/runs/37800200518/job/113393174533) |
| ladybird / verdict de convergence | failure | [113404313841](https://github.com/bcharthur/bouchaud-os/actions/runs/37800200518/job/113404313841) |

TCG 10 min : 108 cycles, 36/36 swaps, 614 relevés d'un unique Compositor.
KVM 20 min : 230 cycles en 1203 s, 76/76 swaps, 1187 relevés d'un unique Compositor.
Stabilité/performance OK dans ces deux runs.
Verdict final : `BOUCHAUD_LADYBIRD_CONVERGENCE_FAIL causes=primitives=failure,memoire=failure,endurance_kvm=failure`.

Le verdict P13 exige désormais les 15 jobs, y compris les anciens diagnostics.
Un échec, un skip ou une annulation le rend rouge. Les artefacts Ladybird restent liés au même HEAD par leur manifeste.

## Vérifications réalisées

HOST : tests négatifs Compositor (10), swaps (12 mémoire + 7 endurance),
mémoire (5 croissances + 8 preuves incomplètes), 60 cas de refus du verdict de convergence,
ancres des préparateurs sur les sources upstream épinglées et idempotence,
garde-fous lifecycle/UI/convergence, tests de réutilisation des sources, compilation Python,
syntaxe bash et diff --check.
Ces contrôles ne sont pas présentés comme des tests QEMU ou PHYSICAL.
Les builds C++ et les preuves QEMU décrits plus haut proviennent des Actions.

## Modifications préparées, non publiées sur la branche

Dans `bouchaud-os-verified`, trois fichiers sont indexés mais non committés localement :
- `tools/ladybird/prepare-echange-processus.py` : compteurs périodiques PageHost created/detached/roots et PageClient finalized ;
- `tools/ci/run_ladybird_memoire.sh` : active le diagnostic upstream `LIBGC_LOG_LEVEL=1` ;
- `tools/health/browser_host_fixture.py` : M0 utilise la même pause de stabilisation que M1/M2, sans changer le plafond ni le verdict.

Objets GitHub préparés mais **non rattachés à la branche** :
`718fd8f0cdb5c5017cb9ce52cac985ec33d7d4d5`, puis `1932861398db1a376ae2a7cf814fb1a27321c75c`.
Arbre final vérifié : `c613df21e2849edf45473c8538eeb06469beb8dc`.
Les ancres et l'idempotence ont été testées ; ces ajouts n'ont PAS été compilés ni joués en CI.

Le patch complet est conservé dans [le patch de reprise](../handoffs/2026-10-08-pending-memory.patch).
Il a été vérifié, hunk par hunk, contre les fichiers GitHub des deux commits.
Ne l'appliquer qu'une fois, après inspection du status et après la priorité KVM :
`git apply --check docs/handoffs/2026-10-08-pending-memory.patch`,
puis `git apply docs/handoffs/2026-10-08-pending-memory.patch`.
S'il est déjà présent dans le checkout restauré, ne pas le réappliquer.

## Blocage et prochain chantier exact

Vers 15:40 UTC, l'exécuteur local s'est déconnecté. Les tentatives de reprise ont fini par :
`409 Conflict, environment_offline: Environment is not connected`.
GitHub restait accessible ; aucune nouvelle modification produit non vérifiable n'a été poussée.

1. Refaire fetch/status/HEAD sur la branche unique et lire ce handoff.
2. Télécharger l'artefact `ladybird-endurance-kvm` du run 37800200518 ;
   inspecter `serie-endurance-ab-on.log` autour de 40 s invité / 17:28:09, sortie normale du PID 18.
   Instrumenter minimalement le motif d'arrêt du canal de contrôle et son transport ; reproduire sans changer les seuils.
3. Rétablir une preuve de pile os-primitives qui survive au défaut de journalisation, sans retirer l'assertion.
4. Après fermeture du KVM A/B, utiliser les diagnostics mémoire conservés pour distinguer page détachée,
   PageClient finalisée, GC et RSS résiduel. Exiger encore 20/20 swaps et une baseline/plateau reproductible.
5. Rejouer la campagne complète au même HEAD, puis seulement envisager P10.

Commandes des bancs (après construction et vérification de l'artefact Ladybird du même HEAD) :

```sh
BO_QEMU_KVM=1 BO_AB_EXIGE_PERF=1 tools/ci/run_ab_profil.sh target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin native-browser 300
BO_QEMU_KVM=1 tools/ci/run_ladybird_memoire.sh target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin native-browser 10
```

QEMU-KVM uniquement. Aucun test TRIGKEY, aucune conclusion GPU accéléré, aucun changement réseau sans régression.
