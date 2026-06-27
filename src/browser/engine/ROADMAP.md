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
| `html/` tokenizer  | WHATWG HTML tokenizer             | 🟡 ad-hoc + fermetures implicites + UTF-8 lossy |
| `html/` tree       | tree construction                 | 🟡 auto-close li/p/td/tr/option/dt/dd |
| `dom/`             | Node/Element/Document             | ⚠️ `web.rs::Dom` minimal |
| `css/` parser      | CSSParser                         | 🟡 dans `web.rs` (commentaires, @media) |
| `css/` values      | CSSPrimitiveValue (px/%/calc…)    | 🟢 `Len`, calc(), rem/vw/vh, gradients |
| `css/` cascade     | StyleResolver (sélecteurs, spéc.) | 🟢 sélecteurs descendants + spécificité |
| `style/`           | ComputedStyle / héritage          | 🟢 `Style`/`BoxProps` |
| `layout/` block    | block formatting context          | 🟢 fonctionnel |
| `layout/` inline   | inline/line boxes                 | 🟢 line-height, white-space, vertical-align |
| `layout/` flex     | flexbox                           | 🟢 row/col/justify/align/gap |
| `layout/` grid     | CSS grid                          | 🟢 colonnes/wrap/span |
| `layout/` position | absolute/relative/fixed/z-index   | 🟢 + sticky, stacking, overflow clip |
| `layout/` table    | table formatting                  | 🟡 `tr` via flex |
| `paint/`           | display list + ordre de peinture  | 🟢 `Item`/layers/clip/gradient |
| `js/`              | JS engine + DOM bindings          | 🟡 `js.rs` (interprète) |
| `image/`           | décodeurs PNG/JPEG/…              | 🟡 PNG, JPEG baseline, lazy/srcset |
| `text/`            | shaping + fontes                  | 🟡 `font_ttf` TrueType |

Légende : 🟢 solide · 🟡 partiel · ⚠️ ad-hoc à refondre · 🔴 absent

## Phases du chantier

- **P0 — Fidélité CSS incrémentale** ✅ : box model par côté, flexbox, grid,
  sélecteurs descendants, images lazy/srcset, rem/vw/vh, calc(),
  text-transform, marqueurs de listes.
- **P1 — Positionnement** ✅ : `position: relative/absolute/fixed/sticky`,
  `top/left/...`, `z-index`, stacking contexts, overflow clip, box-shadow,
  bloc conteneur, peinture ordonnée en couches.
- **P2 — Inline formatting context** ✅ (en partie) : `line-height`,
  `vertical-align`, `white-space` (nowrap/pre-wrap). Reste : `text-overflow`,
  césure, `text-decoration`.
- **P3 — Robustesse HTML** 🟡 : fermetures implicites (li/p/td/tr/option/dt/dd),
  décodage UTF-8 lossy. Reste : tokenizer WHATWG complet (états, insertion modes).
- **P4 — Style avancé** 🟡 : box-shadow ✅, gradients linéaires 2-stops ✅.
  Reste : `border-radius` (rendu arrondi), `transform`, `opacity` (alpha),
  gradients multi-stops/radial.
- **P5 — JS/DOM** : montée en charge de `js.rs` (plus d'APIs DOM, `fetch`,
  `requestAnimationFrame`), reflow déclenché par mutations.
- **P6 — Réseau/format** : WebP/AVIF/GIF, HTTP/2 streams, compression.

## Principe de travail

1. Tout changement **compile pour la cible bare-metal** `x86_64-bouchaud_os`.
2. La logique pure est **validée par tests host** (pas de QEMU en CI).
3. Pas de régression sur les pages internes (`about:*`) qui rendent déjà.
4. Le moteur vit dans `src/browser/engine/` → suivi par la version Nautile au
   boot et miroité vers le repo `nautile-navigateur`.
