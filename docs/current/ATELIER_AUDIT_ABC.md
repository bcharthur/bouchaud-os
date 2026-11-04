# Audit de clôture des incréments Atelier A, B et C

Branche `claude/atelier-increment-a-20261104`. Toutes les mesures viennent de QEMU TCG, BIOS, 1280×720, en rendu **logiciel**. Aucune accélération GPU n'est prétendue. Les journaux cités sont des lignes série réelles du noyau.

## 1. Pics mémoire des instantanés

**Avant l'audit**, un instantané plein écran 1920×1080 pesait 8,3 Mo. Le registre acceptant 8 animations, huit fermetures rapprochées pouvaient retenir 66 Mo pendant un fondu. La fermeture allouait en plus un second tampon temporaire de la même taille.

**Correctifs.**
- **Budget ferme** : `animations::BUDGET_INSTANTANES_OCTETS` vaut 16 Mio pour tous les instantanés réunis. Au-delà, la fenêtre se ferme **sans animation** : c'est l'animation qui cède, jamais la mémoire. Le refus est journalisé (`BOUCHAUD_ATELIER_INSTANTANE refuse`). La règle est pure (`admet_instantane`) et testée, débordement arithmétique compris.
- **Plus de seconde allocation** : le tampon « dessous » de la fermeture réutilise le tampon de travail partagé, libéré dès que le registre est vide.
- **Comptabilité** : octets retenus et pic, publiés dans `BOUCHAUD_ATELIER_ANIMATION … instantanes_pic_kio= instantanes_kio= tas_kio=`.
- **Filet de sécurité** : un registre vide alors qu'un instantané subsiste serait un orphelin. Le cas est journalisé, la mémoire libérée et le compte remis à zéro. Cela remplace un `debug_assert!`, absent des builds de production.

**Mesure** : rafale de 5 fermetures quasi simultanées (Échap tous les 20 ms ; Moniteur, Services, Notes, Insights, Rustpad ouverts) :

```
BOUCHAUD_ATELIER_ANIMATION mode=Normal trames=2 cout_moyen_us=96953 cout_max_us=97583
  instantanes_pic_kio=4866 instantanes_kio=0 tas_kio=42680
```

Pic de 4,8 Mo, sous le budget. Rien n'est retenu après le fondu.

## 2. Ouvertures et fermetures répétées

Scénario : 15 ouvertures de Fichiers, chacune interrompue par Échap 120 ms plus tard, puis 5 cycles complets. Résultat sur environ 20 séquences :
- `instantanes_kio=0` à la fin de chacune ;
- le tas se stabilise à **42 568 Kio** (41 019 à 41 575 Kio pendant les premières séquences, le temps que l'allocateur se remplisse) ;
- aucune croissance d'une séquence à l'autre : pas de fuite.

## 3. Animations simultanées et interrompues

- **Simultanées** : 5 instantanés en fondu dans la même trame (rafale ci-dessus) ; état final propre, sans fantôme.
- **Interrompues** : fermer pendant l'ouverture annule l'ouverture et fait partir la fermeture de l'opacité atteinte (`ferme`) ; réduire pendant l'ouverture l'arrête (`tour` → `peinte` rend `None`).
- **Registre plein** : la plus ancienne animation est évincée, et sa zone et son instantané sont rendus. Test hôte : 1000 lancements, jamais plus de `CAPACITE`.
- **Séquence QEMU** : `atelier/sequence-incB-interruption.png`.

## 4. L'hypothèse du fil unique

L'état des effets est en `static mut`. L'hypothèse « un seul fil y touche » est désormais **vérifiée** :
- le fil du bureau s'en déclare propriétaire (`effets::adopte_fil()`, appelé au début de `boucle`) ;
- chaque point d'entrée vérifie `fil_ok()`. Un appel depuis un autre fil est refusé et signalé une fois (`BOUCHAUD_ATELIER_EFFETS_FIL_ETRANGER`), avec un résultat neutre : rien à animer, survol plein, fenêtre peinte normalement ;
- avant l'adoption (aperçu de démarrage), le registre est vide et rien n'est muté ;
- les réglages (`reglages`) sont en atomiques, car le centre de contrôle les modifie ;
- **aucun signalement** `FIL_ETRANGER` dans les journaux des scénarios d'audit.

## 5. Surfaces fantômes et lectures après libération

- L'instantané de fermeture est une **copie de pixels** prise dans `ferme_fenetre`, **avant** `wins.remove` et avant `client.termine()`. L'animation ne lit jamais la surface d'un client.
- Un client mort (`!client.vivant()`) n'est pas photographié : sa surface n'est plus lue après sa mort.
- En composition directe dans le scanout virtio (`gpu-tampon direct`), il n'y a pas d'instantané.
- Les rangs des calques `Fantome(rang)` sont figés pour toute la trame : le registre ne change qu'avant la composition (`tour`, clics).
- `oublie_tout` libère instantanés et tampons à la fermeture du bureau.
- Captures après fermeture, rafale et interruption : aucune trace.

## 6. Latence de trame, surtout à l'ouverture

`BOUCHAUD_ATELIER_TRAME_OUVERTURE` mesure la première trame après chaque ouverture, **dans tous les modes**, y compris « Aucune », ce qui permet la comparaison.

| | Avant l'audit | Après l'audit |
|---|---|---|
| Normal (6 ouvertures de Fichiers) | 45–67 ms, moyenne 59 ms | **29–51 ms, moyenne 36 ms** |
| Aucune (mêmes gestes) | 42–57 ms, moyenne 48 ms | 48–63 ms, moyenne 52 ms |

Avant l'audit, la première trame à opacité nulle coûtait environ 11 ms de plus qu'en mode « Aucune », pour une image invisible. Elle ne peint plus rien (`compose_ouverture` sort à `alpha == 0`), et chaque ouverture obtient maintenant 4 à 5 trames animées.

**Coût par trame animée** :

| Scénario | Coût moyen | Coût max |
|---|---|---|
| Ouverture de Fichiers | 16–33 ms | 30–57 ms |
| Fermeture du Terminal | ≈ 15 ms | — |
| Survol du dock | 2,7–4,2 ms | — |
| Rafale de 5 grandes fermetures | 97 ms | — |

## 7. CPU au repos et dégâts

- **Repos** (62 s, mêmes gestes) : `desktop cpu_pct` 2–3 à t = 30 s, **0** à t = 60 s, avec 1 trame utile en 53 s. Identique à la base `f2cbb068` et aux mesures d'A, B et C.
- **Dégâts** :
  - `test_degats`, `test_transitions`, `test_scene` et `test_rendu` sont verts et inchangés dans leurs exigences ;
  - un registre vide n'ajoute aucun dégât (test hôte : une minute sans geste, zéro trame) ;
  - le survol et le centre invalident l'ancienne et la nouvelle boîte, jamais le plein écran.

## 8. Défaut préexistant trouvé par le stress, corrigé à part

**Commit `fix(gui): Moniteur ne tient plus le verrou RAMFS pendant process::count`.**

`system_info::draw` tenait le verrou RAMFS (rang 50) pendant `process::count()` (rang 20). LOCKDEP paniquait dès la première peinture du Moniteur :

```
LOCKDEP inversion cpu=0 held_rank=50 held_at=src/fs/ramfs.rs:203 acquiring=process-table(20)
```

Le chemin existe sur `main`, puisque le Moniteur s'ouvre depuis le menu. Le correctif sépare les deux lectures, sans toucher à LOCKDEP. Les autres prises du verrou RAMFS dans le GUI ont été relues : aucune ne prend un verrou de rang inférieur sous lui.

## 9. Limitations résiduelles (assumées, documentées)

- **Échap sur un bureau sans fenêtre quitte la session** (`BUREAU_FIN raison=touche_sans_fenetre`). Ce comportement est antérieur à Atelier. Il est dangereux pour l'utilisateur, mais il touche la fin de session : il est laissé à une décision produit séparée.
- **Coût logiciel sous TCG** : une rafale de grandes fermetures descend à environ 10 images/s. Le rendu est entièrement logiciel ; aucun flou ni GPU n'est prétendu.
- **Première trame du bureau** : sur l'hôte de cet audit, on mesure 5,8–6,0 s. Une comparaison sur le même hôte avec l'incrément C figure en § 10.
- **Pas encore faits** : réduction animée vers le dock, maximisation interpolée, fondu du menu et du centre.
- Le budget de 12 minutes du job « kernel / compile + invariants » est serré : 10–11 min nominales, sur `main` comme sur cette branche.

## 10. Première trame : comparaison sur le même hôte

Après le redémarrage du worker, l'hôte a changé (noyau hôte `fc-v233`). Les chiffres absolus ne sont donc pas comparables aux mesures de l'incrément C prises sur l'ancien hôte. Pour comparer, l'image `2d15b2a9` (incrément C) et l'image de l'audit ont été construites et démarrées **en alternance sur ce même hôte**.

| Image | 7 boots (ms) | Moyenne |
|---|---|---|
| Incrément C `2d15b2a9` | 5402, 5681, 5991, 6501, 5751, 5525, 5737 | 5798 |
| Audit | 6025, 5776, 6572, 6014, 6285, 5682, 5487 | 5977 |

- L'écart de +179 ms est inférieur au bruit d'un boot à l'autre pour une même image (±400 ms) : **pas de régression mesurable**.
- Sur les 4 dernières paires, les moyennes sont identiques (5867 contre 5879 ms).
- Le décodage du fond varie à lui seul de 333 à 473 ms selon le boot.

Au passage, une première version de la vérification du fil interrogeait le registre des tâches à chaque calque peint. Les lecteurs (plan de scène et peintres) ne testent plus qu'un drapeau de trame, `EN_TRAME`, posé par le propriétaire vérifié dans `debut_trame`. Seuls les points d'entrée qui modifient l'état font la vérification complète.
