# Bouchaud Navigateur

Produit navigateur de Bouchaud OS.

Moteur dérivé de Ladybird :
`cdfe5f858eb5fc64a8d9d3fcc247d71b03fbd1f6`

## Processus
BrowserHost, WebContent, WebWorker, RequestServer, ImageDecoder, Compositor.

## Principes
- chrome hors renderer ;
- réseau séparé ;
- crash renderer isolé ;
- sandbox par rôle ;
- surfaces et damage natifs ;
- persistance ;
- observabilité ;
- process swaps et isolation de site.

## Reconnexion Compositor
Une course de ressources de fonts a provoqué un trap dans `DrawGlyphRun`.
Le protocole retient désormais les messages d'état jusqu'à reconnexion complète.

## Lifecycle
Une ancienne page cross-site pouvait survivre avec ses contextes Compositor.
Le discard direct a fortement réduit la rétention ; validation finale encore
nécessaire.
