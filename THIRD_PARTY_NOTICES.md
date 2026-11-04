# Composants tiers

Le code de Bouchaud OS est publie sous licence MIT OR Apache-2.0 (`LICENSE`,
`LICENSE-MIT`, `LICENSE-APACHE`). Il incorpore les composants ci-dessous, sous
leurs licences respectives, que la licence du depot ne remplace pas.

Les textes integraux sont dans `THIRD_PARTY_LICENSES/`.

## Crates Rust du noyau (`Cargo.toml`)

Licences relevees dans le `Cargo.toml` de chaque crate, aux versions resolues
par `Cargo.lock`.

| Crate | Licence | Usage |
|---|---|---|
| fontdue 0.9 (et ttf-parser) | MIT OR Apache-2.0 OR Zlib | rasterisation des polices |
| smoltcp 0.12 | 0BSD | pile TCP/IP |
| wasmi 0.31 | MIT / Apache-2.0 | interpreteur WebAssembly |
| hashbrown 0.15 | MIT OR Apache-2.0 | tables de hachage `no_std` |
| linked_list_allocator 0.10 | Apache-2.0 / MIT | tas d'amorcage |
| x86_64 0.14 | MIT / Apache-2.0 | structures du processeur |
| pic8259 0.10 | Apache-2.0 / MIT | controleur d'interruptions historique |
| bootloader 0.9, bootloader_api 0.11 | MIT / Apache-2.0 | chargeurs BIOS et UEFI |

## Incorpore aujourd'hui

| Composant | Licence | Usage |
|---|---|---|
| **Ladybird** (SHA epingle dans `third_party/UPSTREAM.md`) | **BSD 2-Clause** | moteur de Bouchaud Navigateur (userland) ; texte dans `THIRD_PARTY_LICENSES/ladybird-BSD-2-Clause.txt` |
| DejaVu Fonts | Bitstream Vera / licence DejaVu | polices systeme |
| musl | MIT | libc du userland |
| Qt 5.15.13 | LGPL-3.0 | ancien navigateur (userland historique, `run.ps1 -Legacy`) |
| CPython 3.12.3 | PSF-2.0 | ancien navigateur et environnement Python (userland historique) |
| QuickJS | MIT | JavaScript de l'ancien navigateur (userland historique) |
| FFmpeg 6.1.1 | LGPL-2.1+ | media (userland) |

## Assets du bureau Atelier (sur `main` depuis les PR #242 à #244)

| Composant | Licence | Usage |
|---|---|---|
| Lucide (icones) | ISC, texte dans [`assets/atelier/sources/lucide/LICENSE`](assets/atelier/sources/lucide/LICENSE) | glyphes des icones du bureau |
| « summer 1am », Risto Saukonpaa (KDE plasma-workspace-wallpapers) | LGPL-3.0, texte dans [`assets/atelier/sources/fond/LICENSE-LGPL-3.0.txt`](assets/atelier/sources/fond/LICENSE-LGPL-3.0.txt) (GPL-3.0 jointe) | fond d'ecran, recadre par la fabrique d'assets |

Provenance et manifeste : [`assets/atelier/ATTRIBUTIONS.md`](assets/atelier/ATTRIBUTIONS.md). Le logo Bouchaud OS est provisoire.

## Dans une PR ouverte, pas encore sur `main`

| Composant | Licence | Usage |
|---|---|---|
| Sous-ensemble de DejaVu Sans | Bitstream Vera / licence DejaVu | police du bureau prechargee (PR #247) |

## Dependances de Ladybird

| Composant | Licence | Statut |
|---|---|---|
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
2. **`LICENSE-APACHE`** est le texte officiel, copie tel quel depuis
   <https://www.apache.org/licenses/LICENSE-2.0.txt> (SHA-256
   `cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30`).
