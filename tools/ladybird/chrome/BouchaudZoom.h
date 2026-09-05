#pragma once

// BOUCHAUD_CHROME_V18_ZOOM
//
// # Les crans de zoom du navigateur
//
// Le chrome n'avait aucun zoom. Sur une fenetre de 1278 pixels de large qui
// affiche des sites concus pour 1920, c'est la premiere chose qui manque : les
// pages arrivent trop grandes, et rien ne permettait de les reduire.
//
// Le moteur, lui, savait deja faire -- `PageClient::set_zoom_level()` existe
// dans l'arbre epingle et refait la mise en page. Il n'y avait personne pour
// l'appeler.
//
// # Pourquoi des ENTIERS, et une echelle
//
// Un facteur flottant multiplie a chaque frappe accumule ses erreurs :
// « x1,1 onze fois puis /1,1 onze fois » ne revient jamais exactement a 1, et
// Ctrl+0 devrait alors deviner ce qu'« approximativement cent pour cent »
// signifie. Une echelle d'entiers rend la question sans objet -- le cran neutre
// EST un cran -- et c'est de toute facon en pourcents que l'utilisateur lit un
// zoom.
//
// Comme BouchaudDegat.h, ce fichier ne depend de rien : `test_zoom.cpp`
// l'execute sur l'hote a chaque CI, la ou le reste du chrome ne tourne que dans
// QEMU au bout de vingt minutes de construction.

namespace BouchaudZoom {

/// Les crans, en pourcents. Ceux de Chrome, sans les extremes : au-dela de
/// 300 % une fenetre de 1278 pixels ne montre plus assez de page pour qu'on s'y
/// repere, et en-dessous de 50 % le texte cesse d'etre lisible a l'ecran.
inline constexpr int crans[] = { 50, 67, 75, 90, 100, 110, 125, 150, 175, 200, 250, 300 };
inline constexpr int nombre_de_crans = static_cast<int>(sizeof(crans) / sizeof(crans[0]));

/// L'indice du cran a 100 %.
inline constexpr int cran_neutre = 4;
static_assert(crans[cran_neutre] == 100, "le cran neutre doit valoir 100 %");
static_assert(nombre_de_crans > cran_neutre, "le cran neutre doit exister");

/// Ramene un indice dans l'echelle.
///
/// Toutes les autres fonctions passent par ici. Un indice hors bornes lirait
/// hors du tableau : ce n'est pas un zoom faux, c'est une lecture memoire
/// invalide, et c'est le seul defaut que ce fichier peut commettre.
constexpr int borne(int cran)
{
    if (cran < 0)
        return 0;
    if (cran >= nombre_de_crans)
        return nombre_de_crans - 1;
    return cran;
}

/// Le cran suivant, sature au maximum.
constexpr int agrandit(int cran) { return borne(borne(cran) + 1); }

/// Le cran precedent, sature au minimum.
constexpr int reduit(int cran) { return borne(borne(cran) - 1); }

constexpr int pourcent(int cran) { return crans[borne(cran)]; }

}
