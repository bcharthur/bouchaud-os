# GUI_DAMAGE_AUDIT

*Chaque fonction de dessin du bureau, et l'etat dont ses pixels dependent.
Quatre questions par ligne, et une seule reponse acceptable a la troisieme et a
la quatrieme : quelqu'un, nommement.*

## La regle

> Aucun pixel ne doit pouvoir changer sans degat correspondant.

Le compositeur ne presente que ce qu'on lui designe. Il ne se trompe donc jamais
tout seul : quand l'ecran ment, c'est qu'une transition d'etat a omis de dire ce
qu'elle changeait. Les compteurs ne peuvent pas le reveler — ils montent
fidelement pendant que la zone presentee est la mauvaise.

Chaque fonction de dessin est examinee ici avec les memes quatre questions :

1. **Quel etat** change ses pixels ?
2. **Quel rectangle** change ?
3. **Qui invalide l'ancien etat** ?
4. **Qui invalide le nouvel etat** ?

La question 3 est celle qui a produit tous les defauts trouves. Le nouvel etat
est presque toujours invalide, parce qu'on pense au changement qu'on vient de
faire. L'ancien, lui, n'appartient plus a personne.

## Le tableau

| Fonction | Etat dont les pixels dependent | Rectangle | Ancien etat | Nouvel etat |
|---|---|---|---|---|
| `draw_fond` | aucun — fonction de la taille de l'ecran | plein ecran | sans objet | sans objet |
| `draw_filigrane` | aucun | `filigrane_rect()` | sans objet | sans objet |
| `draw_icone(i)` | `ICON_POSITIONS[i]` (glisser-deposer) ; le libelle **deborde du carre** | `widgets::empreinte_icone(i)` — carre, ombre de 3 px, libelle et son ombre | glisser-deposer : empreinte AVANT le deplacement | glisser-deposer : empreinte APRES |
| `draw_barre_haute` → `draw_topbar` | l'heure RTC, la charge par coeur, la memoire, le disque — **la seule animation permanente du bureau** | `disposition::barre_haute()`, en HAUT | `transition::tic_horloge` (les deux etats occupent le meme rectangle) | idem |
| `draw_fenetre(w, focused)` — cadre | `w.x, w.y, w.w, w.h` | `empreinte_fenetre` = cadre + ombre de 4 px | `transition::fenetre_bougee`, cadre quitte | `transition::fenetre_bougee`, cadre atteint |
| `draw_fenetre(w, focused)` — barre de titre, ligne de separation, 4 bordures | `focused`, c'est-a-dire `widgets::indice_focus(wins)` | empreinte des **deux** fenetres concernees | `transition::focus_transfere`, cadre de celle qui PERD le focus | `transition::focus_transfere`, cadre de celle qui le gagne |
| `draw_fenetre` — contenu d'une application noyau | l'etat de l'application (texte saisi, calcul) | `empreinte_fenetre` | la frappe invalide la fenetre entiere | idem |
| `draw_fenetre` — surface d'un client ring 3 | la surface partagee | degat annonce par le client, ramene a l'ecran | le client annonce l'union de ce qu'il repeint | idem |
| `draw_menu(mx, my)` — corps, ombre | `menu_open` | `empreinte_menu` = menu + ombre de 4 px | `transition::menu_bascule` | idem |
| `draw_menu(mx, my)` — ligne survolee | `window::ligne_menu_survolee(mx, my)` — **une bande de 178 x 22**, fond, bordure de selection, couleur et graisse du texte | `window::rect_ligne_menu(i)` | `transition::survol_menu_change`, ancienne ligne | `transition::survol_menu_change`, nouvelle ligne |
| `draw_taskbar(wins, menu_open)` — bouton Demarrer | `menu_open` | `barre_taches_rect()` | `transition::menu_bascule` | idem |
| `draw_taskbar` — boutons de fenetres | liste et titres des fenetres | `barre_taches_rect()` | ouverture, fermeture, minimisation : degat plein ecran (voir plus bas) | idem |
| `draw_cursor(mx, my)` — forme | position de la souris | `disposition::curseur` = 14 x 22 | `transition::curseur_deplace`, position quittee | `transition::curseur_deplace`, position atteinte |
| `draw_cursor(mx, my)` — **couleur** | le pixel COMPOSE sous le point chaud : noir cercle de blanc sur fond clair, l'inverse sur fond sombre | l'empreinte entiere | `transition::recoloration_curseur`, applique une fois par trame sur les degats accumules | idem |

## Ce que ce tableau a revele

Cinq entrees etaient fausses, toutes a la colonne « ancien etat ».

| Defaut | Ce qu'on voyait a l'ecran |
|---|---|
| Le tic d'horloge invalidait `barre_taches_rect()` | `HH:MM:SS` fige, pendant que `frames_clock_only` montait chaque seconde |
| Le survol du menu n'invalidait que la nouvelle ligne | deux lignes en surbrillance, puis trois, puis toute la colonne parcourue |
| Le focus n'invalidait pas la fenetre qui le perdait | deux fenetres a la barre de titre bleue en meme temps |
| Le libelle d'une icone debordait des bornes de son calque | libelle tronque ; moities de texte laissees derriere une icone deplacee |
| Le curseur n'etait pas recolore quand son fond changeait | fleche bicolore, moitie noire moitie blanche |
| La fermeture du menu n'invalidait pas la barre des taches | le bouton Demarrer restait allume apres la fermeture |
| Le clic dans le menu lisait la ligne avec sa propre formule | l'entete de 8 px et la bande d'accent, qui ne surlignent rien, lancaient la premiere entree |

Les deux barres etant disjointes, la premiere erreur ne pouvait pas se
rattraper toute seule ; le libelle debordant violait le contrat `bornes_dessin`
du calque, ce qui casse aussi le culling.

## Les transitions qui repondent « tout l'ecran »

Quatre transitions annoncent volontairement un degat plein ecran :

* une fenetre se **ferme** ;
* une fenetre est **minimisee** ;
* une fenetre est **restauree** depuis la barre des taches ;
* une fenetre **apparait**.

Elles ont toutes la meme justification : ce qui etait sous la fenetre n'a jamais
ete dessine, et le bureau est seul a le savoir. C'est correct mais grossier —
l'union de l'empreinte et de la barre des taches suffirait. Ce n'est pas
optimise ici parce que reduire un degat est le sens dangereux de l'erreur, et
que ces quatre transitions sont rares. L'oracle les couvre telles quelles : il
verifie ce que la production annonce, pas ce qu'elle pourrait annoncer.

## Comment cela se verifie

`tools/gui/test_transitions.rs` : oracle de transition d'etat.

    tampon    = rendu_complet(etat_A)
    mutation    A -> B
    degats    = ce que la mutation annonce (gui::transition)
    rendu_partiel(tampon, etat_B, degats)
    reference = rendu_complet(etat_B)
    ASSERT      tampon == reference, bit pour bit, sur TOUT l'ecran

Le rasteriseur represente chaque dependance d'etat du tableau ci-dessus, et
quatre tests le verifient lui-meme : aucun calque ne peint hors de
`bornes_dessin`, tout calque opaque remplit `opaque_sur`, un rendu partiel plein
ecran egale un rendu complet, et chaque paire d'etats utilisee change reellement
des pixels.

`tools/gui/test_disposition.rs` verifie la geometrie qui rendait le premier
defaut fatal : les deux barres sont disjointes, le rectangle du survol contient
tout ce qui le declenche, les lignes ne se recouvrent pas.

## Ce qu'aucun de ces tests ne prouve

Qu'un appelant appelle la bonne fonction au bon moment. `gui::transition`
garantit que la fonction appelee rend un degat **suffisant** ; que la boucle
l'appelle a chaque transition reste verifie par lecture. C'est la moitie du
probleme — mais c'est l'autre moitie qui a produit les cinq defauts ci-dessus.
