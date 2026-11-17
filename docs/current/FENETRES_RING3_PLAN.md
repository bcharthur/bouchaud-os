# Gestion des fenêtres hors du noyau — état et plan (10 novembre 2026)

> Document de travail daté. Il dit **ce qui est déjà hors du noyau**, **ce qui
> y est encore**, et l'ordre exact des tranches pour l'en sortir, chacune avec
> sa preuve. Rien ici n'est annoncé comme fait sans preuve.

## 1. Où en est-on (vérifié dans le code)

| Pièce | Où elle tourne | Preuve |
|---|---|---|
| Ladybird (BrowserHost, WebContent, RequestServer, ImageDecoder, Compositor) | **ring 3** | smoke produit, CI |
| DOOM, Calculatrice, toute application du protocole GUI | **ring 3** (`App::Application` générique depuis le 10/11) | `BO_AUTOSTART_APP`, banc GPU |
| Politique de fenêtres `windowd` (#261) | **ring 3**, testée par IPC | `WINDOWD_RING3_IPC_OK` ; **pas** reliée aux fenêtres réelles |
| Composition du contenu des fenêtres | **GPU de l'hôte** sous VIRGL (`BOUCHAUD_GPU_COMPOSITION_V2`) ; CPU noyau sinon | [P10_GPU_COMPOSITEUR.md](P10_GPU_COMPOSITEUR.md) |
| Boucle du bureau (entrées, focus, z-order, chrome, dock, menus, Fichiers/Notes/Terminal natifs) | **fil noyau** `window_manager::boucle` | — |

Conséquence directe de la session du 10/11 : le gestionnaire de fenêtres ne
traite plus aucune application comme « le navigateur ». Le titre, l'écran de
démarrage et la composition sont les mêmes pour toutes ; seule l'icône
distingue Ladybird et DOOM.

## 2. Pourquoi la composition GPU prépare la sortie du noyau

Le compositeur V2 a une interface étroite et sans pointeur noyau :
`CoucheGpu { surface (nœud + pages), rectangle abîmé, origine, morceaux visibles }`.
C'est exactement ce qu'un serveur d'affichage ring 3 enverrait au pilote. Le
pilote garde la possession du périphérique, de la file et de la mémoire DMA ; le
serveur ne manipulerait que des **identifiants de surface** et des
**rectangles**, jamais d'adresse physique ni de commande VIRGL brute.

## 3. Tranches, dans l'ordre, chacune avec sa preuve

1. **Appel système de présentation** — **fait le 10/11, preuve TCG locale
   (`BOUCHAUD_COMPOSITORD_PRESENTATION_V1`, ABI native 1.3)**.
   `bo_display_present(affichage, région, demande*)` : capacité `Display`
   (droit `DISPLAY_PRESENT`) que **seul le noyau** crée, pour le seul
   programme `/bin/composited` qu'il lance lui-même (ni DUP ni TRANSFER) ;
   région lue avec le droit READ de l'appelant ; bornes (surface dans la
   région, rectangle dans la surface, destination dans l'écran, calculs en
   u64) jugées par un contrat pur (`affichage_contrat.rs`, 8 tests hôte) avant
   toute lecture ; refus tant que le bureau noyau tient l'écran. Retour :
   empreinte des pixels **relus à l'écran**.
   Preuve QEMU (`tools/ci/run_native_ipc_probe.sh`, CI `Native IPC Runtime`) :
   `composited` ring 3 pose une mire 320×200 en (100,100) ; la capture de
   l'écran QEMU est décodée au pixel : **64 000 exacts, 0 faux** ; refus vérifiés
   sans capacité, hors surface, hors écran (débordement u32), région sans
   lecture, capacité non duplicable. Garde : `tools/verifie-capacite-affichage.py`
   (6 témoins négatifs). Reste : présenter via `presente_trame_gpu` (VIRGL) et
   plusieurs surfaces par appel.
2. **`compositord` ring 3** : reçoit les surfaces des clients (déjà partagées par
   nœud), possède le fond, le chrome, le dock et le curseur dans **sa** surface,
   et présente par l'appel 1. Le fil noyau du bureau ne fait plus que relayer.
   Preuve : A/B de captures contre le bureau noyau, DOOM + Ladybird.
   - **2A — une vraie fenêtre (`BOUCHAUD_BUREAU_RING3_2A`, ABI 1.4), preuve TCG
     locale.** Une image qui porte `/bureau-ring3` fait lancer par le noyau un
     client BOGU **inchangé** (`mire-gpu`, son propre processus), puis
     `/bin/composited bureau` avec la capacité `Fenetre` sur ce client. Le noyau
     garde le côté client (validation des messages, contrat de tampons, récolte) ;
     `composited` compose fond, cadre et contenu dans sa région et présente les
     seuls rectangles abîmés. Le bureau noyau refuse alors de démarrer
     (exclusion publiée/relue des deux côtés). Banc `tools/ci/run_bureau_ring3.sh`
     (CI Native IPC Runtime) : capture décodée au pixel (254 976 exacts, carré
     mobile 1 024, 0 autre), 12 refus de sécurité, client tué par faute de page →
     fenêtre effacée, compositeur vivant ; A/B même client, TCG SMP4 : ring 3
     59 trames/s, copie 81 µs + présentation 599 µs par trame ; bureau noyau
     58 trames/s, dessin 202 µs + présentation 16 µs. Coût mesuré à résorber :
     la relecture d'empreinte de chaque présentation. Pas encore : DOOM et
     Ladybird, plusieurs fenêtres, déplacement, entrées.
   - **2B — plusieurs fenêtres, déplacement, empilement, cycle de vie** : bras
     B2 à B4 du même banc (deux clients, déplacement scripté, remontée,
     minimisation, redimensionnement, fermeture polie, relance, mort d'un
     client sans perte du survivant), captures au pixel.
3. **Entrées** — **fait le 17/11, preuve TCG locale + CI Native IPC
   (`BOUCHAUD_ENTREES_RING3_V1`, ABI native 1.5)**. Le noyau garde le pilote
   (IRQ, décodage, disposition AZERTY) et ne donne au compositeur qu'une
   lecture, sur la capacité `Display` qu'il est seul à détenir :
   `bo_display_entrees(affichage, tampon*, touches_max)` (position, boutons,
   crans de molette, transitions de touches déjà traduites). La capacité est
   LISIBLE tant qu'une entrée attend : `composited` la met dans son waitset et
   dort entre deux gestes (l'IRQ publie une activité différée, sans
   ordonnanceur ; la moitié basse du minuteur réveille). `composited` décide
   seul : zone sous le pointeur, remontée, foyer (`Configure` au perdant et au
   gagnant), saisie de la barre et glisser, case de fermeture, routage des
   frappes à la fenêtre active ; il route UN événement vers UNE fenêtre par
   `bo_fenetre_entree(fenetre, genre, …)` et le noyau écrit le message du
   protocole GUI (`Pointer`, `Wheel`, `Key`) au client, qui ne voit jamais le
   compositeur. Propriétaire unique : à la première lecture, le clavier est
   confié au compositeur, la file est purgée de ce qui précédait la passation
   (frappes du shell, accusés 0xFA du 8042), et le shell noyau ne vole plus de
   frappes.
   Preuve (`tools/ci/banc_bureau_ring3_entrees.py`, vrais gestes par le
   moniteur QEMU, trois clients `mire-gpu` dont un qui plante) : clic en (30,30)
   du contenu → foyer → l'application reçoit appui et relâchement aux mêmes
   coordonnées ; barre saisie et glissée de (150,90) au pixel près ; `a`,`b`
   frappés → l'application active reçoit `q` (AZERTY) puis `b`, une seule fois ;
   une application meurt d'une faute de page et la frappe suivante arrive
   encore ; case de fermeture → l'application sort d'elle-même ; aucune
   panique ; 9 refus de `bo_fenetre_entree` (genre inconnu, boutons > 7, x hors
   i32, Unicode 0x110000, appui 2, molette nulle, capacités croisées, tampon
   noyau). Non-régression : bras B0–B4 au pixel inchangés (curseur et cases de
   fermeture n'apparaissent qu'après la première entrée réelle).
   **Coût d'un pas de glisser (`BOUCHAUD_SHM_RECT_V1`, ABI 1.6), même banc,
   même machine, TCG, 6 pas de (25,15)** :

   | | moyen | pire | appels région / pas | présentation / pas |
   |---|---|---|---|---|
   | avant | 90,6 ms | 111,8 ms | 3 326 | ≈ 64 ms |
   | `bo_shm_fill_rect` seul | 76,8 ms | 108,8 ms | 60 | 64,2 ms |
   | + `present` sans allocation, relecture par ligne | 29,7 ms | 35,8 ms | 60 | 17,7 ms |
   | + enveloppe ancien∪nouveau cadre | **13,6 ms** | **22,0 ms** | 32 | 8,1 ms |

   Les trois leviers : fond, bordures et barres en rectangles pleins remplis
   par le noyau (au lieu d'un `bo_shm_write` par segment et par ligne) ; la
   pose de `present` sous un seul verrou, sans `Vec` par ligne, et la
   relecture d'empreinte par ligne (même valeur, test hôte) ; une seule
   recomposition de l'enveloppe des deux cadres. Les captures B0–B4 restent
   identiques au pixel. Effet de bord mesuré au bras A/B (animation) :
   présentation par trame 908 → 497 µs.
   Pas encore : routage selon `windowd`, curseur matériel, USB HID sous ce
   chemin, DOOM et Ladybird sous `composited`.
4. **Applications natives** (Fichiers, Notes, Terminal, Calculatrice) en
   processus ring 3 sur le même protocole GUI.
5. **Retrait** du fil noyau `window_manager::boucle` ; le noyau ne garde que le
   pilote d'affichage, le partage de surfaces et la capacité de présentation.

Chaque tranche se qualifie comme les autres (CI TCG + KVM, bancs au pixel,
non-régression DOOM et Ladybird) avant la suivante.
