# Assets Atelier — provenance et licences

Fabriqués par `tools/assets/fabrique-atelier.py`. Empreintes dans `src/assets/atelier/MANIFEST.json`, vérifiées par `tools/verifie-assets-atelier.py`.

| Asset | Auteur / source | Licence | Fichiers |
|---|---|---|---|
| Glyphes **Lucide** (sous-ensemble de 20 icônes : tuiles, barre haute, centre de contrôle ; liste dans `sources/lucide/`) | Lucide Icons et contributeurs, `lucide-static` 1.67.0 (npm) | **ISC** — texte dans `sources/lucide/LICENSE` ; chaque SVG garde son en-tête `@license` | `sources/lucide/*.svg`, glyphes des tuiles, `symboles/*` |
| Fond **« summer 1am »** | **Risto Saukonpää**, dépôt KDE `plasma-workspace-wallpapers` (`summer_1am`, image 2560×1600) | **LGPL-3.0** (déclarée dans `sources/fond/metadata.json`) — texte dans `sources/fond/LICENSE-LGPL-3.0.txt` (+ GPL-3.0 référencée) | `sources/fond/summer_1am-2560x1600.jpg` (source), `fond/summer_1am-*.png` (recadrages « cover ») |
| Tuiles applicatives | Projet Bouchaud OS (dégradé, forme) + glyphe Lucide (ISC) | projet + ISC | `sources/apps/*.svg`, `apps/*.png` |
| Marque **Bouchaud OS** | Projet Bouchaud OS, d'après la maquette Atelier | projet — **PROVISOIRE** | `sources/logo/bouchaud-os-provisoire.svg`, `logo/*.png` |

Le logo est **PROVISOIRE** : le dépôt ne contenait aucun logo officiel de Bouchaud OS, et l'ancien logo nautile du navigateur, dessiné en code, avait été retiré. Il ne doit pas être présenté comme l'identité définitive avant validation.

Écarté :
- **Papirus / Tela** (GPL-3.0) : la licence pèse sur des icônes embarquées, et leur style ne se marie pas aux glyphes Lucide. Les tuiles maison gardent un seul langage.
- La photo par défaut d'elementary OS : sa licence individuelle n'est pas documentée dans le dépôt (projet GPL-3.0+, photo probablement Unsplash).
- Les fonds polygonaux (KDE Altai).
