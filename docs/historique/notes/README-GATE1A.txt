BOUCHAUD OS — GATE 1A / DAMAGE REGION SPARSE
================================================

Base exacte
-----------
branche : perf/native-gui-ng
base    : 2ad9d394e790b5929e9903b61a787026aa182fc0

But
---
Remplacer la boite englobante unique des degats par 16 rectangles fixes,
sans heap, puis composer/presenter chaque rectangle independamment.

Le code Gate 0 faisait :
    let present = proto_rect_ecran(degats.region());

Donc deux petits degats eloignes pouvaient provoquer une enorme copie.
La source actuelle le documente elle-meme comme "une seule boite englobante".

Gate 1A fait :
    for region in degats.regions() {
        set_clip(region)
        draw
        present_rect(region)
    }

Fusion
------
- chevauchement ou fusion <= 25 % d'overdraw : fusion ;
- sinon regions distinctes ;
- 16 slots fixes ;
- au 17e, fusion avec le slot dont la boite finale est la plus petite ;
- aucun pixel sale n'est perdu ;
- `tout()` reste une unique region plein ecran.

Metriques
---------
[GUI-DAMAGE] ajoute :
- rects
- requested_pixels
- gate0_bbox_pixels
- saved_pixels
- merges
- overflows

`presented_pixels` est maintenant la somme des regions sparse.
`gate0_bbox_pixels` est ce qu'aurait presente l'ancien moteur sur les memes
trames. `saved_pixels` est donc le gain directement mesurable.

Application
-----------
Extraire a la racine du depot, puis :

    .\APPLY-GATE1A.ps1

Le script refuse tout fichier suivi deja modifie, exige la bonne branche/base,
execute toute la validation + bootimage, puis commit uniquement :
- src/gui/degats.rs
- src/gui/window_manager.rs
- tools/gui/test_degats.rs

Ensuite :

    .\MEASURE-GATE1A.ps1

Le runner boote SMP4 + Ladybird, attend M11_DOCUMENT_LOADED, mesure 30 s et
affiche les dernieres lignes [GUI-DAMAGE].

Aucun scheduler/BKL n'est modifie par Gate 1A.
Aucun git clean/reset n'est utilise.
