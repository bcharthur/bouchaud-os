# Architecture cible du moteur Web Nautile

Cette découpe s'inspire des responsabilités visibles dans WebKit : une couche API
WebView/navigation, puis un moteur interne séparant chargement réseau, parsing,
DOM, style, layout, display list, painting, JavaScript et bindings DOM. La page
Apple WebKit consultée demande JavaScript côté documentation, mais elle confirme
le périmètre WebKit public : navigation, vues Web, configuration, scripts et
interaction page/app.

## Modules nécessaires

- **loader / navigation** : résolution URL, historique, redirections, documents
  internes (`about:*`), état de chargement.
- **network / resource loader** : HTTP(S), cache, sous-ressources CSS/JS/images,
  priorités et annulation.
- **html tokenizer + tree builder** : tokenisation tolérante, correction de
  balises mal fermées, modes d'insertion HTML.
- **DOM** : nœuds, attributs, recherche, mutations, sérialisation, événements.
- **CSS tokenizer/parser** : règles, at-rules, media/supports, déclarations,
  valeurs (`calc`, couleurs, gradients, longueurs).
- **style resolver** : cascade, spécificité, héritage, variables CSS, index de
  sélecteurs et matching descendant.
- **layout** : block/inline/flex/grid, fragmentation de lignes, box model,
  positionnement, overflow/scroll containers.
- **display list / stacking** : items peignables, z-index, stacking contexts,
  clipping, transforms.
- **paint/compositor** : clipping viewport, images, texte, fonds, bordures,
  ombres, gradients, futurs layers composités.
- **JavaScript engine** : lexer/parser/interpréteur, objets natifs, timers,
  microtasks, erreurs récupérables.
- **DOM bindings JS** : `document`, `window`, `Element`, événements,
  mutations DOM, reflow/repaint après mutation.
- **media/images/fonts** : PNG/JPEG/WebP/SVG, polices système et `@font-face`.
- **diagnostic/devtools** : `about:log`, timings, compteurs DOM/CSS/layout/paint.

## Découpe engagée dans le code

- `web.rs` reste l'orchestrateur historique : pipeline HTML → CSS → layout → paint.
- `style.rs` contient maintenant les structures de règles CSS et l'index de
  matching (`Sel`, `Rule`, `CssIndex`).
- `css_parser.rs` contient le parsing des feuilles CSS : declarations, selecteurs,
  at-rules simples et production de `Rule`.
- `css_values.rs` contient le parsing des valeurs CSS isolables du layout
  (`transform` aujourd'hui, `calc`/couleurs/gradients demain).
- `display_list.rs` contient maintenant les items peignables, les liens, les
  layers et les transformations simples de fragments.
- `paint.rs` contient la traversée de display list vers framebuffer : couches,
  clipping viewport, texte/images/rectangles/coins arrondis.

La suite naturelle est d'extraire progressivement `layout_block.rs`,
`layout_inline.rs`, `html_parser.rs` et `dom_bindings.rs`, en gardant une
compilation verte à chaque étape.
