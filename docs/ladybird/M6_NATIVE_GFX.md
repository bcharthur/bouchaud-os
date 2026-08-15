# M6 — LibGfx / Skia CPU natif dans Bouchaud OS

Ce jalon fait volontairement une chose mesurable : construire **le vrai
`Libraries/LibGfx` du SHA Ladybird epingle** et executer son `PainterSkia` dans
un ELF `static-pie` sous Bouchaud OS.

## Ce qui est upstream

La source de verite reste `third_party/UPSTREAM.md`. `fetch.sh` recupere l'arbre
Ladybird correspondant ; aucune copie de LibGfx n'est versionnee dans Bouchaud.

Le graphe utilise est celui de `Libraries/LibGfx/CMakeLists.txt` :

- LibCompress
- LibCore / LibCrypto / LibFileSystem / LibTextCodec / LibIPC / LibSync /
  LibUnicode
- WOFF2
- JPEG
- PNG
- HarfBuzz
- Skia
- `libgfx_rust`

## Pourquoi un vcpkg graphique isole

M1–M5 ont volontairement remplace vcpkg par quelques constructions directes.
Skia change l'echelle du probleme : son port vcpkg encode une fermeture de
dependances et des choix de compilation qu'il serait dangereux de recopier a la
main.

`build-vcpkg-gfx.sh` epingle donc le **meme builtin-baseline** que Ladybird, mais
ne lui demande qu'un backend CPU :

- Skia + FreeType + Fontconfig
- aucun Vulkan
- aucun OpenGL
- aucun Direct3D / Metal

HarfBuzz est installe sans ICU. C'est intentionnel : le port Bouchaud possede
deja son ICU pour LibUnicode et ne doit pas lier deux ICU differents dans le meme
ELF.

## Reproduire localement sous Linux / WSL

```bash
tools/ladybird/native-m6.sh --cible
```

Sorties :

```text
third_party/build-libgfx-bouchaud/libGfx.a
third_party/build-libgfx-bouchaud/libgfx-probe
```

Le binaire cible ne doit pas etre execute sous Linux pour valider M6. Le workflow
`ladybird-native-m6` le place dans une image et l'execute dans QEMU/Bouchaud.

## Critere de succes

```text
== temoin LibGfx : Skia CPU ==
  ok     Bitmap BGRA8888 320x200 cree par LibGfx
  ok     PainterSkia CPU cree
  ok     clear_rect a produit des pixels blancs
  ok     fill_rect Skia a produit rouge, bleu et fond intact

RESULTAT : 0 verification(s) en echec (4 passees)
```

Cette preuve ferme M6 : les pixels viennent de `Gfx::Painter` -> `PainterSkia`,
pas du moteur Qt/Python historique.

## Ce que M6 ne pretend pas encore faire

M6 n'est pas WebContent. Le prochain jalon ajoute le processus
`Services/WebContent`, sa poignee de main LibIPC et une surface partagee
Bouchaud. L'integration du navigateur utilisateur ne doit basculer sur
`BO_WEB_ENGINE=ladybird` qu'une fois ce processus fonctionnel.
