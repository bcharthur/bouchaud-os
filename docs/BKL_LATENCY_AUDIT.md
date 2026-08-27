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

## `poll` est une victime, pas une cause

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

## Ce que cet audit ne prouve pas

Que les traces de `test_discipline_bkl.rs` decrivent fidelement le code : c'est
une lecture, pas une mesure. Ce qu'elles garantissent, c'est que quiconque
ajoutera une phase globale sous le verrou devra l'ecrire dans la table — et que
le test la refusera.

Et que les corrections suffisent : cela se mesure au runtime, sur
`[BKL-SYSCALL]`.
