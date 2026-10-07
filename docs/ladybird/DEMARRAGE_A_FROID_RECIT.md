> **HISTORIQUE — récit déplacé du README le 2026-10-07.** Ce texte décrit
> la campagne de mesure du démarrage à froid telle qu'elle était menée à
> l'époque. Les chiffres sont datés et ne sont pas réévalués ici ; l'état
> courant est dans le [README](../../README.md) (tableau de bord) et
> [INTEGRATION_STATUS.md](INTEGRATION_STATUS.md).

# Observabilité du démarrage à froid

Le navigateur fonctionne ; il démarre lentement. Le chantier en cours sépare
ces deux questions et refuse de les confondre.

**Deux statuts CI indépendants.** `ladybird / browser-host smoke` bloque sur la
capacité, `ladybird / performance` bloque sur les budgets, et un échec de l'un
n'est jamais présenté comme un échec de l'autre. Le verdict de performance
était auparavant écrit mais lu par personne : il dépendait d'une variable
qu'aucun workflow ne définissait.

**Capacités vertes**, vérifiées à chaque run : canvas, images 11/11 codecs,
iframes, JS 17/17, WebWorker HTTP et blob, et la mire réellement retrouvée
dans une trame composée — capture prise pendant que QEMU vit, corrélée à un
numéro de trame, et non plus après sa mort.

**Ce que la mesure a établi, et ce qu'elle a réfuté.** Le premier WebWorker
coûte environ 130 s là où les suivants coûtent 1,5 à 9 s. Les bornes posées
sur le chemin noyau ont successivement innocenté :

| poste | mesure | verdict |
|---|---|---|
| `fork` | 28 ms | hors de cause |
| `fork` → `execve` | 8 ms | hors de cause |
| `execve` | 22 ms | hors de cause |
| `exec` → `main` | ~94 s | **le poste réel** |
| fautes fichier dans ce segment | ~10 s | 11 %, pas la cause |

Le segment de 43,4 s longtemps attribué à l'ordonnanceur n'existait pas : il
était mal borné.

**Cause confirmée, et corrigée : le balayage de secours du cache de pages.**
À chaque défaut de cache, dès que la table atteint son plafond de 16 384
pages, le noyau parcourait **toute** la table en prenant le verrou d'état de
chaque entrée, sous le verrou global — pour n'y rien trouver. Mesuré sous
Ladybird : 22 773 balayages, 632 millions d'entrées parcourues, **277 s**,
soit 57 % du temps noyau du run. Le modèle « un balayage par défaut de cache
une fois la table pleine » se vérifie à 0,0 % près sur banc local et 1,8 % sur
Ladybird. La correction sort quand le compteur d'entrées récupérables vaut
zéro — le balayage est alors garanti de ne rien trouver. Banc à 80 Mio, trois
exécutions par bras : durée **−58 %**, temps noyau **−62 %**, avec des témoins
identiques (`entrees=20480`, `recuperees=0`) qui prouvent qu'aucune
récupération n'a été perdue.

**Remesuré sous Ladybird (run #358), et le gain dépasse le banc.**
`HOST_WORKER_BLOB_PERF_FIRST` passe de **126 670 ms à 8 996 ms** — un facteur
14, sous un budget de 30 000 ms qui n'a pas bougé. Le temps noyau du run tombe
de 490 240 ms à 103 500 ms ; le premier WebWorker, de ~113 s à 8,985 s de
`sys_ms`. Deux témoins restent **identiques au run précédent** —
`candidats_suffisants=12722` et `miss=52534` — donc le chemin rapide
d'éviction a fait exactement le même travail et le cache a manqué exactement
les mêmes pages : seul le balayage a disparu. Ce qui domine maintenant, ce sont
132 s de lectures disque réelles, et c'est la prochaine question.

**Le même commit a figé la machine, et c'était la sonde.** `Integration #256`
s'arrête net après `SESSION_PERE_SORT fils=4`, machine vivante, scénario
bloqué. Le chef de session publie ses sondes sans encombre ; ce sont les
**quatre tâches arrêtées avec lui** qui ne publient jamais leur `PROCESS_EXIT`.
`exit_current` tient `process.lifecycle` pendant la publication des sondes, et
`balayage_temoins()` y prenait `CACHE.lock()` : une arête d'ordre de verrous
sur un chemin de sortie. Ce correctif était juste — la règle est écrite dans le
fichier même — mais il **n'a pas suffi** : `Integration #257` a échoué au même
endroit, et une passe locale verte ne prouve rien contre une course.

**Un réveil perdu dans `wait4`, antérieur — et ma correction l'a aggravé.**
`[SCHED-RESUME] pretes=2 ... au_repos=4 en_file=0` — deux tâches prêtes, quatre
cœurs au repos, file vide : le shell n'a jamais été remis en file. `sys_wait4`
cherchait les fils zombies, n'en trouvait aucun, **puis** se déclarait en
attente. Un fils mourant dans cet intervalle trouvait `waiting_for_child`
encore à faux ; son réveil tombait dans le vide et le parent s'endormait pour
toujours. Les sondes de sortie de processus n'ont fait que déplacer le timing
dans cette fenêtre. Corrigé en posant `Blocked` **avant** le drapeau — sinon le
réveilleur consomme le drapeau puis échoue sur l'état — et en **revérifiant**
après s'être déclaré. **Cette correction a été retirée : elle empirait le
défaut**, déplaçant le blocage du 8ᵉ marqueur au 3ᵉ. La relecture du protocole
établi (`blocage.rs`) est un compteur ; la mienne parcourait tous les processus
en prenant un verrou par processus, tâche déjà marquée bloquée. La course reste
**ouverte et documentée, non corrigée**.

**Ce qui est en périmètre, c'est le déclencheur.** Les trois sondes globales
étaient publiées *dans* le scope de `process.lifecycle` — trois écritures série
sous un verrou de processus. Elles sont désormais publiées après sa fermeture ;
mêmes chiffres, section critique rendue à sa longueur. Deux règles tirées de la
même erreur : une sonde ne prend pas de verrou, et une sonde ne rallonge pas la
section critique qu'elle observe. Reproducteur : sous contention CPU le défaut
sort **3 fois sur 3**, là où une passe non contrainte passait et m'avait fait
conclure trop vite. La règle était déjà
écrite dans le fichier même (« Reporting must stay lock-free »), et le
commentaire au-dessus du site d'appel mettait en garde contre ce geste exact.
Corrigé par un compteur atomique ; vérifié par
`tools/ci/verifie-sondes-sans-verrou.py`, qui refuse toute sonde d'`exit_current`
dont le corps contient `.lock()`.

**La mesure corrigée a inversé la conclusion.** Avec la frontière posée, le
premier WebWorker mesure `user_ms=724` et `sys_ms=113594` : il passe 0,7 s en
espace utilisateur et 113 s dans le noyau. `_dl_relocate_static_pie` s'exécute
en espace utilisateur — l'hypothèse des 405 396 relocations de démarrage est
donc **réfutée par borne supérieure**, et le banc A/B d'édition de liens a été
retiré plutôt que laissé rouge. Ce qui reste à expliquer est net : 113 594 ms
de noyau dont le livre des fautes n'explique que 13 285. Deux candidats
(balayage de secours du cache, chaîne de reprise des fautes) ont été posés puis
réfutés localement en quelques minutes — `appels=0` et `reprises=0`.

**Une conclusion a été retirée, parce que l'instrument était faux.** Ce
tableau portait « dont ~93 s de CPU » et la phrase « le premier worker
n'attend pas, il calcule ». Les deux venaient d'un relevé `user_ms=92250
sys_ms=671`. Or les frontières de comptabilité n'existaient qu'autour des
appels système : le gestionnaire de faute de page n'en avait aucune, et tout
ce qu'il fait — y compris **déclencher et attendre une lecture ATA** —
tombait dans `user_ns`. Mesuré sur banc local, trois exécutions par variante,
la frontière posée déplace 94 % du « temps utilisateur » vers le noyau
(1320 ms → 61 ms côté utilisateur, 14 ms → 1204 ms côté noyau, total
conservé). Rien n'est devenu plus rapide : l'étiquette était fausse. Le
partage réel du segment `exec` → `main` demande un nouveau run, et la piste
des relocations de démarrage perd l'argument qui la soutenait.

Dans la foulée, la variante ET_EXEC qui devait falsifier cette piste s'est
révélée **impossible à lier** : la glibc statique référence des symboles
faibles indéfinis résolus à l'adresse zéro, et un `R_X86_64_PLT32` ne peut pas
porter le déplacement depuis `0x400000000000`. Le micro-binaire qui semblait
la valider était lié en `-nostdlib`. Elle est remplacée par une variante RELR
(`-z pack-relative-relocs`), vérifiée localement : 26 280 octets de table
deviennent 288, l'ASLR est conservée.

**L'outillage de mesure est lui-même sous test.** Décomposition des fautes de
page attribuée par PID et non globalement, avec test de chevauchement de deux
processus ; `acquire` rend son coût à la faute qui l'a payé ; les sous-champs
« dont » ne sont jamais additionnés à leur contenant ; les compteurs globaux
portent `scope=global` pour ne pas se faire passer pour une attribution. Chaque
garde-fou a été mis en échec volontairement avant d'être retenu.

**Ce qui reste ouvert** est documenté dans
[docs/MESURE_DEMARRAGE_A_FROID.md](../MESURE_DEMARRAGE_A_FROID.md) :
la décomposition des ~94 s avant `main` avec un partage utilisateur/noyau
désormais honnête, et le coût propre de l'instrumentation à forte charge de
fautes.

