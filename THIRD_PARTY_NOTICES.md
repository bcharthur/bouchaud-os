# Composants tiers

Bouchaud OS est publie sous licence MIT OR Apache-2.0. Il incorpore ou prevoit
d'incorporer les composants ci-dessous, sous leurs licences respectives.

Les textes integraux sont dans `THIRD_PARTY_LICENSES/`.

## Incorpore aujourd'hui

| Composant | Licence | Usage |
|---|---|---|
| Qt 5.15.13 | LGPL-3.0 | rasterisation du navigateur actuel (userland) |
| CPython 3.12.3 | PSF-2.0 | moteur du navigateur actuel (userland) |
| QuickJS | MIT | JavaScript du navigateur actuel (userland) |
| FFmpeg 6.1.1 | LGPL-2.1+ | media (userland) |
| DejaVu Fonts | Bitstream Vera / licence DejaVu | polices systeme |
| musl | MIT | libc du userland |

## Prevu — portage Ladybird

| Composant | Licence | Statut |
|---|---|---|
| **Ladybird** | **BSD 2-Clause** | texte copie dans `THIRD_PARTY_LICENSES/ladybird-BSD-2-Clause.txt` |
| ICU 78.3 | Unicode-3.0 | requis par LibUnicode (donc par LibJS) |
| OpenSSL 3.6.3 | Apache-2.0 | requis par LibCrypto (liaison publique) |
| libtommath 1.3.0 | Unlicense / dual | BigInt |
| simdjson 4.6.4 | Apache-2.0 | `JSON.parse` |
| fast-float 8.2.10 | Apache-2.0 / MIT / BSL-1.0 | conversion numerique |
| fmt 12.2.0 | MIT | formatage |
| mimalloc 2.2.7 | MIT | allocateur (peut etre evite) |
| Skia 148 | BSD-3-Clause | LibGfx — a partir de M6 |
| HarfBuzz 10.2.0 | MIT (old) | mise en forme du texte |
| FreeType 2.13.3 | FTL ou GPL-2.0 | rasterisation des glyphes |
| curl 8.21.0 | curl (type MIT) | RequestServer, si voie 1 |

Cette liste suit `vcpkg.json` d'upstream au SHA `cdfe5f8`. Elle doit etre
reverifiee a chaque montee de SHA : c'est une etape du travail de
synchronisation, pas une formalite.

## Regles

1. **Les notices de copyright ne sont jamais retirees** d'un fichier repris de
   Ladybird ou d'ailleurs, meme modifie.
2. Un fichier repris puis modifie porte la notice d'origine **et** la mention de
   la modification.
3. Tout nouveau composant tiers arrive avec son texte de licence dans
   `THIRD_PARTY_LICENSES/` **dans la meme PR**. Une PR qui ajoute une dependance
   sans sa licence n'est pas acceptable.
4. LGPL (Qt, FFmpeg) impose de permettre le remplacement de la bibliotheque. La
   forme actuelle — bibliotheques statiques dans une image — demande un examen
   qui n'a pas encore ete fait. **Point ouvert, a traiter avant toute
   distribution binaire hors developpement.**

## Points ouverts

1. **LGPL et image statique.** La compatibilite LGPL de l'image userland
   statique (Qt, FFmpeg) n'est pas tranchee. Elle ne bloque pas le developpement
   mais doit l'etre avant toute distribution binaire.
2. **`LICENSE-APACHE` manquant.** `LICENSE` et `LICENSE-MIT` sont en place. Le
   texte integral d'Apache-2.0 n'a **pas** ete ajoute : un texte juridique
   reproduit de memoire peut differer de l'original sur un detail qui compte. Il
   doit etre copie tel quel depuis <https://www.apache.org/licenses/LICENSE-2.0.txt>.
