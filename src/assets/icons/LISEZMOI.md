# Les icones du bureau

Cinq images, embarquees dans le noyau par `src/gui/icones.rs` et decodees par
`src/gui/png.rs`.

## Quatre sont fabriquees ici

`calculatrice.png`, `terminal.png`, `fichiers.png` et `rustpad.png` sont
produites par :

```
python3 tools/assets/fabrique-icones.py
```

Le dessin est du code — lisible, revu comme le reste — et le generateur refait
les fichiers **a l'octet pres**. `fabrique-icones.py --verifie` compare les
fichiers commis a ce que le code produit, et `validate-fast` l'appelle : le
depot ne peut pas se retrouver avec des images que son propre code n'explique
plus.

Aucune dependance : `zlib` et `struct`, tous deux dans la bibliotheque standard
de Python. Le dessin se fait a quatre fois la taille finale puis se reduit par
moyenne de blocs, ce qui donne l'antialiassage sans code d'antialiassage.

## Une vient d'ailleurs, et c'est voulu

`ladybird.png` est le logo de **Ladybird**, repris de son depot
(`UI/Icons/ladybird.png`, <https://github.com/LadybirdBrowser/ladybird>), reduit
de 256 a 128 pixels par moyenne de blocs sur l'alpha premultiplie.

Bouchaud OS execute reellement Ladybird : le bureau doit donc afficher sa
marque, et non une coccinelle approchee dessinee a la main — ce qu'il faisait
jusqu'ici. Le projet Ladybird est distribue sous licence BSD 2-Clause ; le
fichier est repris tel quel, sans modification autre que la reduction, pour
designer le logiciel qu'il designe.

## Le format attendu

PNG, 128 x 128, huit bits par composante, RGBA, sans entrelacement. Le decodeur
du noyau refuse tout le reste plutot que de le decoder a moitie — voir
`tools/gui/test_png.rs`, qui verifie que ces cinq fichiers se decodent, qu'ils
portent bien de la transparence, et que le decodeur lit les cinq filtres de
ligne et les cinq types de couleur du format.
