# Performance P1 — première trame du bureau : −1,9 s

Branche `claude/perf-premiere-trame-20261104`, issue de la tête de la PR #245 (stockage S1). P13, Atomic A/B et l'ordonnanceur ne sont pas touchés : aucun ordre d'amorçage n'est modifié.

## Mesurer d'abord (`BOUCHAUD_PERF_PREMIERE_TRAME`, nouveau)

La première trame publie maintenant son coût, calque par calque, une fois par démarrage. Sous QEMU TCG, BIOS 1280×720 :

| Poste | Avant |
|---|---|
| Chargement de la police : `fontdue::Font::from_bytes` analyse d'emblée les **6253** glyphes de DejaVu Sans | **2,1 à 2,8 s** |
| Décodage du fond (calque `fond` de la première trame) | 0,4 à 0,6 s |
| Tout le reste de la première trame (filigrane, icônes, barre, fenêtres, dock, curseur, présentation) | < 70 ms |

## Ce qui change

1. **Police du bureau réduite** (`BOUCHAUD_PERF_POLICE_SOUS_ENSEMBLE_V1`).
   - La fabrique d'assets (fontTools, déterministe) produit `src/assets/atelier/polices/DejaVuSans-Bureau.ttf` : latin avec diacritiques combinants, grec, cyrillique, ponctuation, symboles monétaires, flèches, mathématiques, filets et formes géométriques. Cela fait **1938 glyphes, 167 Kio**, sans hinting (fontdue ne le lit pas : mêmes contours, même rendu).
   - Le fichier est tracé dans le manifeste, avec sa licence (Bitstream Vera / DejaVu : la modification est permise et le nom ne contient ni « Bitstream » ni « Vera »).
   - **Le fichier complet reste installé pour Ladybird** : les pages web gardent toute la couverture.
   - **Repli** : un caractère absent du sous-ensemble charge la police complète une fois, à la demande, et la couverture du bureau ne régresse pas. Vérifié : un texte hébreu au démarrage du bureau produit `repli : police complete chargee pour U+05E9`.
2. **Chargement en parallèle** (`BOUCHAUD_PERF_POLICE_PARALLELE_V1`).
   - Un fil noyau `precharge-police` est lancé dès `process::init`.
   - Un état atomique à trois valeurs (rien, en cours, prête, publiée avec Release) garantit un seul chargement.
   - Le bureau trouve la police prête, ou attend le reliquat (`BOUCHAUD_PERF_POLICE_ATTENTE`).
   - Le gain est limité par l'amorçage : les processeurs secondaires ne prennent des tâches qu'à `enable_scheduler()`, que je n'ai pas avancé (c'est l'ordonnanceur).

## Résultat (même hôte, 5 paires alternées)

| Image | Première trame (ms) | Moyenne |
|---|---|---|
| Base (tête de #245) | 5925, 5954, 5959, 5728, 5612 | **5836** |
| Cette branche | 3807, 3975, 4039, 3705, 4093 | **3924** |

Soit **−1,9 s (−33 %)**. Le chargement de la police passe de 2138–2798 ms à 445–616 ms.

## Ce qui a été essayé et retiré

**Précharger aussi le fond d'écran** sur le même fil. Sur 3 essais, la première trame se situait entre 3,9 et 4,5 s, contre 3,7 à 4,0 s sans ce préchargement. Sous TCG, le second décodage dispute les vCPU au bureau, et quand le bureau arrive avant la fin, le fond est décodé deux fois. Ce changement est retiré, et la raison est écrite dans le code.

## Coût

- Bootimage : 29 246 976 octets, soit +167 Kio pour la police réduite. Le fichier complet est toujours embarqué pour Ladybird.
- Mémoire : fontdue garde les contours de 1938 glyphes au lieu de 6253. La police complète n'est chargée qu'en cas de repli.

## Prochaines pistes, mesurées par la même ligne

- Le fond (0,4–0,6 s) : un format moins coûteux à décoder qu'un PNG compressé (rangées brutes ou RLE), à mettre en balance avec la taille de l'image.
- La police de repli (2,2 s sous TCG au premier caractère hors sous-ensemble) : un second sous-ensemble intermédiaire, si l'usage le justifie.
