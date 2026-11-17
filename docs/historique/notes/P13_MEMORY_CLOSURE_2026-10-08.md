# P13 — fermeture mémoire Ladybird — 2026-10-08

## Base exacte

Ce lot est figé pour le commit Bouchaud OS :

`36a55cdf6c38e697aa5b0697110decb9f06e9283`

Ladybird amont reste épinglé par `third_party/UPSTREAM.md` à :

`cdfe5f858eb5fc64a8d9d3fcc247d71b03fbd1f6`

Aucun changement de SHA Ladybird n'est inclus.

## Ce qui est déjà prouvé avant ce lot

La campagne `37830173252` du commit `36a55cdf` a rendu verts les primitives,
le build Ladybird, cache/SQL, endurance TCG 10 min, endurance KVM A/B,
endurance KVM 20 min, WPT, sites réels, robustesse, BrowserHost smoke,
politique de crash des services, ordre worker, piège Compositor et performance.
Le job mémoire reste rouge, donc le verdict de convergence P13 reste rouge.

Deux répétitions mémoire ont montré le même phénomène dans le WebContent
ouvreur : environ +11.45 Mio de RSS en phase 2. En parallèle, le Compositor
revient à sa baseline (`contexts=2`, `backing_stores=2`, `backing_store=4623
KiB`).

La preuve `BOUCHAUD_PAGES_MEMOIRE_V1` montre en outre que, dans le WebContent
ouvreur, les pages sont retirées du `PageHost` (`roots=1`, `detached=20`) mais
les anciens `PageClient` ne sont pas détruits pendant le banc (`finalized=0`).
LibGC collecte et balaie pourtant le tas pendant le scénario. Cela exclut une
simple fuite de `PageHost::m_pages`, mais ne permet pas encore d'affirmer si la
rétention est une racine forte réelle ou une racine conservative.

## Ce que ce lot ajoute

`prepare-gc-retention-proof.py` appelle `GC::Heap::dump_graph()` à deux jalons
uniquement : 10 puis 20 pages détachées. Il ne force pas de GC et ne change
aucune politique mémoire. Le dump est réduit dans le processus aux plus courts
chemins depuis les racines GC vers les cellules dont le nom contient
`PageClient`.

Les marqueurs sont :

- `[LB:GC_PATH] ... root=... frame=... depth=... path=... END`
- `[LB:GC_RETENTION] ... active=... pageclients=... retained=... paths=... END`

`analyse_gc_retention.py` vérifie que les deux jalons existent, que les comptes
sont cohérents et que chaque `PageClient` a un chemin exploitable. Il classe la
racine comme `conservative`, `strong` ou `unknown`. Une preuve inconnue ou
tronquée reste rouge.

## Ce que ce lot ne prétend pas

Ce lot **ne prétend pas corriger la fuite avant d'avoir identifié sa racine**.
Il ne modifie ni les seuils du banc, ni `analyse_memoire.py`, ni le GC, ni le
lifecycle produit. Il ne force pas `collect_garbage()` et ne transforme pas une
croissance réelle en succès.

Le prochain correctif produit devra être décidé à partir des chemins obtenus :

- `StackPointer`, `RegisterPointer`, `ConservativeVector`,
  `ConservativeHashMap`, `ConservativeHashTable` ou
  `HeapFunctionCapturedPointer` : rétention conservative à traiter au niveau de
  la durée de vie des références/piles ou de la stratégie GC, sans casser le
  graphe produit ;
- `Root ...`, `VM`, `RootVector`, `RootHashMap`, `RootHashTable` ou
  `CrossHeapMember` : remonter la chaîne affichée et corriger l'owner fort qui
  conserve la page ;
- `retained=0` : les anciens `PageClient` ne sont plus vivants ; si le RSS
  reste haut, l'analyse se déplace vers l'allocateur / le retour de pages au
  système, sans accuser à tort le graphe Web.

P13 ne doit passer vert que quand `analyse_memoire.py` rend `MEMOIRE_BORNEE` et
que la preuve GC est complète.

## Upstream

Le Ladybird actuel a profondément déplacé la propriété des traversables et de
l'historique vers le processus UI depuis le SHA épinglé. Ces changements ne
sont pas rétroportés en bloc ici : ils couvrent de nombreuses évolutions de
site isolation et d'histoire de navigation et seraient une source de
régressions si on les greffait pour résoudre une fuite encore non attribuée.

Le lot reste donc borné au diagnostic de la version réellement construite par
Bouchaud OS.


## Isolation du benchmark mémoire

`GC::Heap::dump_graph()` alloue des structures temporaires importantes. L'appeler
pendant les deux répétitions qui mesurent M0/M1/M2 fausserait potentiellement le
RSS et pourrait créer un faux rouge après correction de la fuite. Le mode V2
sépare donc strictement les responsabilités :

- les deux répétitions mémoire de référence n'activent pas le dump de graphe ;
- si leur verdict reste rouge, une troisième répétition diagnostic reçoit
  `BOUCHAUD_GC_RETENTION_PROOF=1` ;
- seule cette troisième répétition émet `[LB:GC_PATH]` et
  `[LB:GC_RETENTION]` ;
- la troisième répétition ne peut jamais rendre vert le verdict mémoire des
  deux premières.

Pour une racine `StackPointer`, le marqueur contient aussi `frame_label=...`,
issu du tableau `stack_frames` déjà produit par LibGC. Un simple numéro de frame
sans son label n'est pas considéré comme une attribution suffisante.


## V3 - lifecycle closure

- Base: `de27a322d07052731f46d494fae852e021105ddf`.
- Les deux mesures restent non instrumentees par `dump_graph()`.
- La preuve GC n'est requise que dans le troisieme run diagnostic.
- `bouchaud_discard_page` attend maintenant la destruction asynchrone du Document avant retrait du BrowsingContext/PageHost.
- `Document::destroy()` annule explicitement un `HTMLParserEndState` restant afin de relacher son Timer activity-root.
- Les resumes `LB:GC_SUMMARY` sont courts, emis trois fois et valides seulement si au moins deux copies completes sont identiques.
- Aucun seuil RSS n'est releve, aucun GC n'est force.
- P13 reste ouvert jusqu'a une campagne same-HEAD entierement verte.
