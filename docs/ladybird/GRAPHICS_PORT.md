# Portage graphique

## Etat actuel

Bouchaud a un chemin graphique complet et **qui marche** :

    moteur Python -> liste d'affichage -> Qt/QPainter -> QImage
        -> surface partagee (memfd) -> compositeur WM (fil noyau) -> BGA 1280x720

Le gestionnaire de fenetres est seul proprietaire du framebuffer physique ; le
client recoit un `/dev/fb0` redirige vers la surface de sa fenetre. Protocole GUI
v1 documente dans `../GUI_USERLAND_PROTOCOL.md`.

**Cette integration ne doit pas etre jetee** tant que Ladybird ne sait pas
presenter ses propres pixels.

## Ce que Ladybird demande

C'est la partie la plus lourde du portage, et de loin :

| Composant | Dependance | Difficulte |
|---|---|---|
| `LibGfx` | **Skia** (148), HarfBuzz, FreeType, fontconfig | tres elevee |
| Accelere | ANGLE, Vulkan, Metal, Direct3D | hors sujet — pas de GPU |
| `Services/Compositor` | LibGfx | elevee |
| Polices | HarfBuzz + FreeType | moyenne |

Skia sans GPU se construit en mode CPU. C'est le seul mode envisageable ici : la
carte QEMU est une BGA sans acceleration.

## Chemin

    LibGfx (Skia CPU)
        -> bitmap RGBA
        -> adaptateur Bouchaud
        -> surface partagee (memfd, deja en place)
        -> compositeur WM
        -> ecran

L'adaptateur est **la seule piece a ecrire**. Tout ce qui est en aval existe et
est eprouve.

## Etapes

1. LibGfx cree une surface hors ecran et ecrit un PNG (M6). Comparaison pixel a
   pixel avec une reference — c'est ainsi que `verifie-hote.sh` valide deja le
   pont Qt, avec 31 verifications.
2. Adaptateur `LibGfx::Bitmap` -> surface partagee.
3. Premiere page HTML locale affichee dans une fenetre Bouchaud (M8).
4. Entrees : le WM route deja clavier, souris et molette vers un client par le
   protocole GUI v1 ; il faut les traduire en evenements LibWeb.

## Decision differee

Faut-il porter Skia, ou brancher LibGfx sur un rasteriseur plus modeste ? La
reponse depend d'une mesure qui n'a pas encore ete faite : la taille et le temps
de construction de Skia CPU pour la cible. A trancher avant M6, pas avant.

Tant que la reponse n'est pas connue, ce document ne promet rien sur M6.

## Ce qui reste du chemin actuel

Le bogue connu — fenetre du navigateur visuellement blanche alors que les trames
sont produites — reste a corriger pour garder un repli utilisable. C'est un
travail de correction, pas d'investissement : aucune nouvelle API Web ne doit
etre ajoutee au moteur Python.
