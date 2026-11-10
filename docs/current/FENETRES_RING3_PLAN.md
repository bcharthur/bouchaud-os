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

1. **Appel système de présentation** `bo_display_present(surface_ids, rects)`
   réservé à un processus détenant la capacité « affichage » (accordée au
   démarrage, comme `windowd`). Le noyau valide les surfaces (appartenance,
   bornes) et appelle `presente_trame_gpu`. Preuve : mire au pixel identique au
   banc actuel, refus d'une surface d'un autre principal.
2. **`compositord` ring 3** : reçoit les surfaces des clients (déjà partagées par
   nœud), possède le fond, le chrome, le dock et le curseur dans **sa** surface,
   et présente par l'appel 1. Le fil noyau du bureau ne fait plus que relayer.
   Preuve : A/B de captures contre le bureau noyau, DOOM + Ladybird.
3. **Entrées** : un service d'entrées ring 3 reçoit les événements HID et les
   route selon `windowd` (focus, z-order). Preuve : un client qui plante ne gèle
   ni les entrées ni le bureau.
4. **Applications natives** (Fichiers, Notes, Terminal, Calculatrice) en
   processus ring 3 sur le même protocole GUI.
5. **Retrait** du fil noyau `window_manager::boucle` ; le noyau ne garde que le
   pilote d'affichage, le partage de surfaces et la capacité de présentation.

Chaque tranche se qualifie comme les autres (CI TCG + KVM, bancs au pixel,
non-régression DOOM et Ladybird) avant la suivante.
