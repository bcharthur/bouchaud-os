# BKL_LATENCY_AUDIT

*Ou le gros verrou est pris, combien de temps, et pourquoi. Audit de `249a005`,
declenche par un runtime Ladybird ou le BKL etait detenu 99,96 % d'une fenetre
de 5,2 s pour 77 acquisitions seulement.*

## Le verdict, d'abord

Le BKL n'est pas la maladie. C'est l'amplificateur.

Trois operations de la memoire virtuelle avaient un cout **quadratique**, et
elles s'executaient sous le verrou. Le profil mesure — tenues tres longues,
tres peu d'acquisitions, trois CPU en attente — est exactement la signature
d'un travail lineaire en un etat global, pas d'un verrou trop souvent pris.

| # | Ou | Cout | Ce qu'il coutait vraiment |
|---|---|---|---|
| 1 | `vmm::free_frame` | `O(frames libres)` **lectures froides** par `free` | Le double `free` etait detecte en parcourant la liste libre, chainee *dans* les frames liberees. 4096 pages liberees avec 100 000 frames libres = 4 x 10^8 lectures memoire non cachees. |
| 2 | `AddressSpace::pages` | `O(R x P)` | Un `Vec<u64>` balaye par `prepare_unmap` (par page), `finish_unmap` (par frame) et `owns_frame` (par page, dans `fork`). WebContent a 200 Mio resident donne P = 51 200. |
| 3 | `clean_page_cache::release` | `O(K x C)` prises de verrou | Recherche par balayage, puis `reclaim_excess` recomptait tout le cache **en prenant le verrou de chaque entree**, a chaque page rendue. |

Les trois se rejoignent dans `sys_madvise(MADV_DONTNEED)` — celui que le
ramasse-miettes de Ladybird emet en rafale au defilement. D'ou `max_hold_ns =
15,09 s` attribue a `madvise(28)`, origine `resume_after_schedule` : c'est
exactement le point ou reprend la deuxieme moitie du travail, apres le
shootdown TLB.

Cela explique aussi ce qu'une simple contention n'expliquerait pas : **la
degradation progressive**. Google se rend correctement au depart puis devient
inutilisable, parce que la liste libre et l'ensemble resident ne cessent de
grandir.

## `poll` etait une victime — et le suivant

*Cette section decrit le profil de `249a005`. Elle reste vraie de ce
profil-la ; voir « Le suivant, nomme par la mesure » pour ce qui a suivi.*

`poll(7)` montait a 11,4 s puis 12,4 s sur CPU0. L'audit du chemin dit que ce
ne peut pas etre de la DETENTION :

* `sys_poll` prend le verrou pour tout l'appel (`poll` n'est pas dans
  `SANS_BKL`), mais son travail propre est `O(count)` — sept descripteurs ;
* son attente passe par `wait_readiness` -> `WaitQueue::wait` ->
  `park_current_on` -> `schedule()` -> `suspend_for_schedule()` ;
* et `suspend_for_schedule` appelle `probe_note_release`, **qui ferme
  l'intervalle de detention**. Un `poll` endormi ne peut donc pas accumuler de
  `hold`.

Il n'y a aucune boucle non bornee entre la prise et la liberation. Le chiffre
observe est donc de l'ATTENTE — celle du `madvise` qui tenait le verrou. C'est
coherent avec `bkl_wait_delta_ns = 15,6 s` sur une fenetre de 5,2 s : environ
2,98 CPU en attente, dont CPU0.

C'est pourquoi la refonte de `poll` (collecte courte, liberation, attente sur
objet) n'a **pas** ete faite : l'audit ne la justifie pas. La ligne
`[BKL-SYSCALL]` separe desormais `hold_delta_ns` et `wait_total_ns` par appel,
ce qui rend la question tranchable au prochain run au lieu d'etre deduite.

## Les chemins, question par question

### `madvise(MADV_DONTNEED)` — `src/compat/linux/mem.rs`

| Question | Reponse |
|---|---|
| Ou le BKL est-il acquis ? | `usermode.rs:306`, pour tout l'appel. `madvise` n'est pas dans `SANS_BKL`. |
| Profondeur | 1 a l'entree ; 2 pendant `process.mm.lock()` (verrou fin, pas le BKL). |
| Ou est-il suspendu ? | `execute_process_invalidation` -> `suspend_for_schedule()` autour de `invalidation.execute()`. |
| Ou est-il repris ? | `resume_after_schedule(depth)` juste apres l'ACK TLB. |
| Operations longues APRES la reprise | `finish_unmap` (defaut 2) puis la boucle `clean_page_cache::release` (defaut 3). **C'est la que les 15 s se passaient.** |
| Boucles sous le verrou | `retire_clean_pages`, `prepare_unmap`, `finish_unmap`, la boucle de `release`. Toutes lineaires en la plage — acceptable — sauf que chaque iteration etait elle-meme lineaire en un etat global. |
| Ce qui peut bloquer | L'attente d'ACK TLB, et elle seule. Elle est deja hors verrou. |
| Ce qui doit vraiment etre protege | La coherence VMA <-> PTE <-> frames possedees pendant la fenetre ou elles sont desaccordees. |
| Ce qui peut etre fige avant de relacher | `retirement` est deja exactement cela : les PTE sont retirees, les frames collectees, et rien d'autre ne peut y toucher. |

**Ce qui a change** : le cout par iteration. `free_frame` passe de `O(L)`
lectures froides a un bit ; `prepare_unmap`/`finish_unmap` de `O(P)` a
`O(log P)` ; `release` de `O(C)` prises de verrou a `O(log C)`. La structure du
chemin — instantane court, travail hors verrou pour le shootdown, reprise —
n'avait pas besoin d'etre refaite : elle etait deja juste.

### `poll` — `src/compat/linux/file.rs`

| Question | Reponse |
|---|---|
| Ou le BKL est-il acquis ? | `usermode.rs:306`, pour tout l'appel. |
| Profondeur | 1 ; 2 transitoirement dans `WaitQueue::wait` (`enter_bkl()`). |
| Ou est-il suspendu ? | `park_current_on` -> `schedule()` -> `suspend_for_schedule()`, ou la voie inactive (HLT) qui suspend aussi. |
| Ou est-il repris ? | `resume_after_schedule` au retour de `switch_context`, ou apres le HLT. |
| Operations longues apres la reprise | Aucune : un nouveau balayage de `count` descripteurs. |
| Boucles sous le verrou | Le balayage, `O(count)` — lineaire en la DEMANDE. |
| Ce qui peut bloquer | `wait_readiness`, et `user_read`/`user_write` par faute de page. |
| Ce qui doit vraiment etre protege | La generation de readiness entre le scan final et le parking — le billet, deja pris avant le scan. |
| Ce qui peut etre fige | Deja fait : le billet ferme la course producteur. |

**Ce qui a change** : rien. L'audit ne trouve pas de defaut ici.

Une remarque reste, hors du sujet de cette latence : `READINESS` est une queue
**globale**. Tout changement de readiness reveille tous les guetteurs, qui
rebalayent puis se rendorment. C'est un troupeau, et il coute des acquisitions
— pas des tenues. Il n'a donc pas pu produire les chiffres observes, et le
corriger avant d'avoir mesure l'effet des trois corrections serait deux
changements melanges.

## La regle qui manquait

Les trois defauts ont la meme forme, et aucune mesure ne l'aurait nommee.
`kernel::sync::discipline` l'ecrit :

> Sous le gros verrou, le cout d'une phase peut dependre de la taille de la
> **demande**. Il ne doit jamais dependre de la taille d'un **etat global**.

Une plage de mille pages qui coute mille fois une page, c'est le travail
demande. Une plage d'une page qui coute cent mille comparaisons parce que la
machine tourne depuis longtemps, non.

Deux regles l'accompagnent : personne ne dort en tenant le verrou, et un chemin
qui le reprend apres un changement de contexte le rend avant de bloquer de
nouveau. `tools/smp/test_discipline_bkl.rs` decrit les chemins reels de
`madvise` et `poll`, rejoue les trois defauts comme traces fautives, et exige
qu'elles soient refusees.

## Le resultat mesure

Runtime reel sur `5b196fa`, chargement de `https://www.google.com/`, 4 vCPU TCG.

| Mesure | Avant (`249a005`) | Apres (`5b196fa`) |
|---|---|---|
| `bkl_hold_delta_ns` / fenetre | 5,23 s / 5,24 s — **99,96 %** | 1,0 a 2,0 s / 5,0 s — **20 a 40 %** |
| `bkl_wait_delta_ns` | 15,59 s | 2,1 a 3,5 s |
| `max_hold_ns` | **15,09 s**, `madvise(28)`, origine `resume_after_schedule` | 1,52 s, `syscall=none`, origine `enter`, **a l'amorcage** (t=3 s, avant le navigateur) |
| `madvise` dans `[BKL-SYSCALL]` | le coupable | **absent du classement** |

Les trois quadratiques ont disparu du profil. `madvise` ne figure plus une seule
fois dans les cinq relevés `[BKL-SYSCALL]` de la session, et la plus longue
tenue restante appartient a l'amorcage — donc a du travail qui n'a pas de
concurrent.

`bkl_acq_delta` est reste entre 16 000 et 31 000 par fenetre. C'etait attendu et
c'est dit plus bas : les corrections changent le cout DANS des sections
critiques, pas leur nombre.

## Le suivant, nomme par la mesure

Avec `madvise` corrige, `[BKL-SYSCALL]` a designe le suivant sans ambiguite :

```
poll=[hold_delta_ns=1949720609 hold_pct=38 acq_total=20567  wait_total_ns=3093632469]
poll=[hold_delta_ns=1554120148 hold_pct=31 acq_total=31339  ...]
poll=[hold_delta_ns=1527715238 hold_pct=30 acq_total=43625  ...]
poll=[hold_delta_ns=1197268826 hold_pct=23 acq_total=57546  ...]
poll=[hold_delta_ns= 580945784 hold_pct=11 acq_total=100898 wait_total_ns=10163942444]
```

`acq_total` passe de 20 000 a 100 000 en quarante secondes : environ 2 500
appels `poll` par seconde, chacun tenant le gros verrou pour toute sa duree.

**L'audit de `poll` disait qu'il etait une victime. Il l'etait — et il etait
aussi le suivant.** La phrase de la version precedente de ce document,
« `poll` est une victime, pas une cause », etait vraie du profil de l'epoque et
fausse comme prediction : une fois `madvise` corrige, `poll` est devenu la
premiere cause. La partie de l'audit qui tenait, c'est que son propre travail
est `O(count)` et qu'il ne dort pas le verrou tenu ; ce qui manquait, c'est que
tenir le verrou longtemps sans rien faire d'interdit reste desastreux pour trois
autres coeurs.

Ce qui l'obligeait au gros verrou n'etait pas son domaine — la table des
descripteurs a son verrou, chaque objet a le sien — mais la ROUTE vers le
processus : `task::current_process()` commence par `smp_lock::enter()`, et les
quatre sondes de readiness l'appelaient chacune, par descripteur et par
balayage. L'en-tete de `bkl.rs` avait deja nomme ce piege.

`current_process_local()` rend le meme `Arc` depuis le bloc par-CPU sans toucher
`TASKS`. `POLL` et `PPOLL` sont donc passes dans `SANS_BKL`, avec trois branches
— clavier, souris, socket inet — qui prennent le verrou elles-memes parce
qu'elles touchent un etat global sans verrou propre (l'anneau e1000 est
entierement en `static mut`).

## Comment lire l'avant/apres

Les corrections changent le cout **a l'interieur** de sections critiques qui
existaient deja. Elles ne suppriment aucune prise de verrou et n'en ajoutent
aucune.

**Le nombre d'acquisitions n'est donc pas un critere.** `bkl_acq_delta` peut
rester autour de 77 tout en ayant parfaitement mordu — c'est meme le resultat
attendu si la charge est identique. Le prendre pour un signe d'echec ferait
chercher au mauvais endroit.

Les criteres sont, dans cet ordre :

| Mesure | Ou | Ce qu'elle dit |
|---|---|---|
| `bkl_hold_delta_ns` | `[SMP-SAMPLE]` | Part de la fenetre pendant laquelle le verrou est confisque. |
| `bkl_wait_delta_ns` | `[SMP-SAMPLE]` | Ce que les autres CPU perdent a l'attendre. |
| `max_hold_ns` | `[BKL-MAX-HOLD]` | La plus longue tenue continue — la seule qui distingue un noyau qui travaille d'un noyau qui gele. |
| `madvise=[...]` | `[BKL-SYSCALL]` | La detention attribuee a l'appel incrimine, et sa separation d'avec l'attente. |
| `vm_phase` | `[SMP-STALL]` | Laquelle des cinq phases tenait le verrou. |

Les valeurs visees — moins de 5 % de detention, moins de 50 ms de tenue
maximale — sont des **objectifs**, pas des invariants. Elles disent ou l'on
voudrait arriver ; elles ne definissent pas la reussite d'un run.

## Ce que cet audit ne prouve pas

Que les traces de `test_discipline_bkl.rs` decrivent fidelement le code : c'est
une lecture, pas une mesure. Ce qu'elles garantissent, c'est que quiconque
ajoutera une phase globale sous le verrou devra l'ecrire dans la table — et que
le test la refusera.

Et que les corrections suffisent : cela se mesure au runtime, sur
`[BKL-SYSCALL]`.
