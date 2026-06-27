# Moteur Nautile — chantier « WebKit-like en Rust »

Objectif : un moteur de rendu web **from-scratch en Rust `no_std`**, exécuté
nativement dans Bouchaud OS, capable d'afficher le web moderne — sans WebKit,
sans Chromium, sans `std`. Démarche identique à **Servo** (réécriture d'un
moteur en Rust), pas un binding d'un moteur C++ (impossible en `no_std`).

On part de l'existant fonctionnel (`web.rs`, `js.rs`, `image.rs`, `font_ttf.rs`)
et on le fait monter en fidélité, subsystème par subsystème.

## Pipeline cible (modèle WebKit / Blink / Gecko)

```
octets ──▶ décodage (encoding) ──▶ tokenizer HTML ──▶ tree builder ──▶ DOM
DOM ──▶ parse CSS ──▶ cascade + héritage ──▶ style calculé (ComputedStyle)
style + DOM ──▶ arbre de layout ──▶ formatage (block/inline/flex/grid) ──▶ boxes
boxes ──▶ paint (display list) ──▶ compositing ──▶ framebuffer
DOM ◀──▶ JS (event loop, DOM bindings, microtâches) ──▶ relayout/repaint
```

## Décomposition en modules (cible `src/browser/engine/`)

| Module             | Rôle WebKit équivalent            | Statut |
|--------------------|-----------------------------------|--------|
| `html/` tokenizer  | WHATWG HTML tokenizer             | ⚠️ ad-hoc dans `web.rs::parse` |
| `html/` tree       | tree construction                 | ⚠️ ad-hoc |
| `dom/`             | Node/Element/Document             | ⚠️ `web.rs::Dom` minimal |
| `css/` parser      | CSSParser                         | ⚠️ dans `web.rs` |
| `css/` values      | CSSPrimitiveValue (px/%/calc…)    | 🟡 `Len`, `calc()`, rem/vw/vh |
| `css/` cascade     | StyleResolver (sélecteurs, spéc.) | 🟡 sélecteurs descendants OK |
| `style/`           | ComputedStyle / héritage          | 🟡 `Style`/`BoxProps` |
| `layout/` block    | block formatting context          | 🟢 fonctionnel |
| `layout/` inline   | inline/line boxes                 | 🟡 basique (pas de vertical-align) |
| `layout/` flex     | flexbox                           | 🟢 row/col/justify/align/gap |
| `layout/` grid     | CSS grid                          | 🟢 colonnes/wrap/span |
| `layout/` position | absolute/relative/fixed/z-index   | 🔴 absent |
| `layout/` table    | table formatting                  | 🟡 `tr` via flex |
| `paint/`           | display list + ordre de peinture  | 🟢 `Item`/`paint` |
| `js/`              | JS engine + DOM bindings          | 🟡 `js.rs` (interprète) |
| `image/`           | décodeurs PNG/JPEG/…              | 🟡 PNG, JPEG baseline |
| `text/`            | shaping + fontes                  | 🟡 `font_ttf` TrueType |

Légende : 🟢 solide · 🟡 partiel · ⚠️ ad-hoc à refondre · 🔴 absent

## Phases du chantier

- **P0 — Fidélité CSS incrémentale** (en cours) : box model par côté ✅,
  flexbox ✅, grid ✅, sélecteurs descendants ✅, images lazy/srcset ✅,
  unités rem/vw/vh ✅, **calc()**, **text-transform**, **marqueurs de listes**.
- **P1 — Positionnement** : `position: relative/absolute/fixed`, `top/left/...`,
  `z-index`, bloc conteneur, 2ᵉ passe de peinture ordonnée. (overlays, menus,
  badges, en-têtes collants → indispensables au web moderne)
- **P2 — Inline formatting context propre** : `line-height`, `vertical-align`,
  `white-space` (nowrap/pre-wrap), `text-overflow`, césure.
- **P3 — Tokenizer HTML WHATWG** : remplacer le parseur ad-hoc par un
  tokenizer conforme (états, entités, insertion modes) → robustesse sur le
  vrai web. Réutilise les crates `html_tokenizer`/`html_tree_builder`.
- **P4 — Style avancé** : `box-shadow`, `border-radius` (rendu), `transform`,
  `opacity` réelle (alpha compositing), gradients multi-stops, `background-image`.
- **P5 — JS/DOM** : montée en charge de `js.rs` (plus d'APIs DOM, `fetch`,
  `requestAnimationFrame`), reflow déclenché par mutations.
- **P6 — Réseau/format** : WebP/AVIF/GIF, HTTP/2 streams, compression.

## Principe de travail

1. Tout changement **compile pour la cible bare-metal** `x86_64-bouchaud_os`.
2. La logique pure est **validée par tests host** (pas de QEMU en CI).
3. Pas de régression sur les pages internes (`about:*`) qui rendent déjà.
4. Le moteur vit dans `src/browser/engine/` → suivi par la version Nautile au
   boot et miroité vers le repo `nautile-navigateur`.
