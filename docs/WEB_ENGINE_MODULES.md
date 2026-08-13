# Les modules du moteur Web

*Carte de `tools/userland/navigateur/moteur/`. La version precedente de ce
document decrivait une decoupe visee pour un moteur ecrit en Rust **dans** le
noyau (`web.rs`, `style.rs`, `paint.rs`) : ces fichiers n'existent plus, le
moteur est du Python au-dessus de Qt et de QuickJS. L'ancienne carte est dans
`docs/history/WEB_ENGINE_MODULES.md`.*

Chaque module porte une explication en tete de fichier qui dit ce qu'il fait et
**pourquoi il est fait ainsi**. Ce tableau ne les remplace pas : il sert a
trouver le bon fichier.

## Le pipeline

```text
  URL ──► reseau ──► html ──► css ──► mise_en_page ──► peinture ──► pixels
            │          │        │          │              │
       securite    invalidation │      flex/grille    police/images
       transport                │      tableau
       stockage             intrinseque
```

| Module | Role |
|---|---|
| `reseau.py` | HTTP, HTTPS, `file://`, pages internes, cache, redirections |
| `transport.py` | par ou une ressource arrive : `Direct` ou `Courtier` |
| `securite.py` | qui a le droit de demander quoi |
| `prechargement.py` | les sous-ressources partent ensemble |
| `html.py` | du texte a un arbre de nœuds |
| `css.py` | analyse, specificite, cascade, heritage |
| `invalidation.py` | ce qu'il faut refaire quand quelque chose change, et rien de plus |
| `intrinseque.py` | ce qu'une boite demande avant qu'on lui donne quoi que ce soit |
| `mise_en_page.py` | de l'arbre style aux boites positionnees |
| `flex.py`, `grille.py`, `tableau.py` | les dispositions qui ont leur propre algorithme |
| `peinture.py` | de l'arbre de boites a une liste d'affichage |
| `police.py`, `images.py` | `@font-face` et `src` vers quelque chose que l'hote sait dessiner |
| `animation.py` | ce qui fait qu'une page bouge |

## Le monde JavaScript

| Module | Role |
|---|---|
| `js.py` | le JavaScript d'une page, branche sur l'arbre du moteur |
| `worker.py` | un second monde JavaScript, sur son propre fil |
| `prelude.js` | la surface globale d'une fenetre |
| `prelude_worker.js` | celle d'un Worker, ecrite positivement et non par retrait |
| `prelude_partage.js` | ce que les deux mondes ont en commun |
| `prelude_clone.js` | le clonage structure, ecrit une fois pour les quatre chemins |

## Les frontieres

| Module | Role |
|---|---|
| `origine.py` | l'unite de confiance du Web |
| `cors.py` | ce qu'un script a le droit de **lire** |
| `contexte.py` | plusieurs mondes Web a la fois |
| `cadres.py` | les `<iframe>` |

## Le multiprocessus

| Module | Role |
|---|---|
| `vue.py` | ce que le chrome voit d'une page, ou qu'elle vive |
| `superviseur.py` | le cote navigateur : forke, applique la politique, courte les ressources |
| `renderer.py` | le cote rendu : un moteur Web au bout d'une prise |
| `protocole.py` | les trames echangees entre les deux |
| `surface.py` | `memfd` + `MAP_SHARED`, deux tampons, une generation |
| `privileges.py` | ce dont l'enfant n'herite pas, et ce qu'il peut encore |

## La persistance et le reste

| Module | Role |
|---|---|
| `stockage.py` | temoins, `localStorage`, cache HTTP |
| `indexeddb.py` | la memoire d'une application Web |
| `websocket.py` | la premiere connexion que la page garde ouverte |
| `edition.py` | l'etat d'un champ editable |
| `media.py` | audio et video, de la trame decodee a l'ecran |
| `youtube.py`, `signature.py`, `lecteur_youtube.py` | extraire le flux plutot qu'executer l'application |
| `distant.py` | afficher ce qu'un vrai Chromium a rendu ailleurs |
| `telemetrie.py` | ce que la page a demande et que le moteur n'a pas su faire |
