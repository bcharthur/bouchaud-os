# GUI userland bridge - jalon 1

Ce jalon avance deux chantiers sans essayer de livrer tout le compositor en une fois.

## 1. Transition bureau -> navigateur

Avant ce patch, `lance_navigateur()` faisait:

    fb::leave() -> exec(/bo-navigateur) -> fb::enter()

`leave()` repassait la carte en VGA texte 80x25. Le flash etait donc structurel.

Apres ce patch:

    dessine carte de lancement
    -> handoff_to_userland()
    -> exec(/bo-navigateur)
    -> resume_from_userland()

BGA reste actif. Le backbuffer du bureau reste en RAM et `present()` devient un no-op
pendant que le navigateur possede logiquement la sortie.

## 2. Qt -> pixels

L'hote Qt garde linuxfb pour la creation de QApplication et les entrees, mais il ouvre
aussi `/dev/fb0` lui-meme. Le paintEvent produit sa frame dans une QImage qui enveloppe
un mmap anonyme, puis copie les pixels vers la sortie.

Au prochain boot, le journal doit contenir une ligne de ce type:

    [bo] raster Qt : QImage=ok paintEngine=ok QPainter=actif
    [bo] sortie graphique : /dev/fb0 1280x720 stride=5120

Si QPainter est inactif, on saura que le probleme est le moteur raster Qt et non le WM.

## 3. Surface partagee deja preparee cote hote

L'hote reconnait maintenant:

- `BO_SURFACE_FD`
- `BO_SURFACE_WIDTH`
- `BO_SURFACE_HEIGHT`
- `BO_SURFACE_STRIDE`

Si ces variables existent, il mappe ce fd au lieu de `/dev/fb0`. Le noyau ne les fournit
pas encore: ce sera le prochain jalon, avec `memfd + SCM_RIGHTS + socketpair` et un vrai
client de fenetre asynchrone.

## 4. Protocole v1

`src/gui/client.rs` fige deja les noms de messages:

- client -> WM: Hello, CreateWindow, SetTitle, Damage, Close
- WM -> client: Configure, Focus, Key, Pointer, Wheel, CloseRequest

Le protocole n'est pas encore branche dans la boucle du WM. Il est volontairement ajoute
maintenant pour que la prochaine etape ne melange pas conception du format et transport.

## Prochain test

1. push du commit afin que le workflow userland reconstruise `/bo-navigateur` pour ce SHA;
2. `run.ps1` sous Windows;
3. ouvrir Bouchaud Browser depuis le bureau;
4. relever les lignes `[gfx] framebuffer ...`, `[bo] raster Qt ...`, `[bo] sortie graphique ...`;
5. verifier visuellement qu'il n'y a plus de retour VGA texte entre le bureau et Qt.