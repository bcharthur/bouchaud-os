// Banc d'essai hote de l'echelle de zoom.
//
// Ce que ces cas gardent n'est pas « le zoom zoome » -- cela se voit -- mais
// les deux choses qui ne se voient pas : qu'un indice ne sorte jamais du
// tableau, et que Ctrl+0 revienne EXACTEMENT a cent pour cent quel que soit le
// chemin parcouru. Un zoom qui ne revient pas a son cran neutre est une gene
// permanente que rien ne signale.

#include "BouchaudZoom.h"

#include <cstdio>

using namespace BouchaudZoom;

static int echecs = 0;

static void verifie(bool condition, char const* cas, char const* detail)
{
    if (condition) {
        std::printf("  ok     %s\n", cas);
        return;
    }
    std::printf("  ECHEC  %s\n         %s\n", cas, detail);
    echecs += 1;
}

int main()
{
    std::printf("== echelle de zoom ==\n");

    verifie(pourcent(cran_neutre) == 100, "le cran neutre vaut 100 %",
        "Ctrl+0 ne rendrait pas la taille d'origine");

    // -----------------------------------------------------------------
    // Les bornes. Maintenir Ctrl+- ne doit pas lire hors du tableau.
    // -----------------------------------------------------------------
    {
        int cran = cran_neutre;
        for (int coup = 0; coup < 100; ++coup)
            cran = reduit(cran);
        verifie(cran == 0, "cent reductions saturent au minimum", "l'indice a deborde");
        verifie(pourcent(cran) == crans[0], "le minimum est le premier cran", "valeur inattendue");

        for (int coup = 0; coup < 100; ++coup)
            cran = agrandit(cran);
        verifie(cran == nombre_de_crans - 1, "cent agrandissements saturent au maximum",
            "l'indice a deborde");
        verifie(pourcent(cran) == crans[nombre_de_crans - 1], "le maximum est le dernier cran",
            "valeur inattendue");
    }

    // -----------------------------------------------------------------
    // Un indice deja hors bornes est ramene, jamais lu tel quel.
    // -----------------------------------------------------------------
    {
        verifie(pourcent(-1000) == crans[0], "un indice negatif est ramene au minimum", "hors tableau");
        verifie(pourcent(1000) == crans[nombre_de_crans - 1],
            "un indice trop grand est ramene au maximum", "hors tableau");
        verifie(agrandit(-1000) == 1, "agrandir depuis un indice negatif part du minimum", "faux");
        verifie(reduit(1000) == nombre_de_crans - 2,
            "reduire depuis un indice trop grand part du maximum", "faux");
    }

    // -----------------------------------------------------------------
    // Aller-retour : l'echelle est strictement croissante, donc reversible
    // tant qu'on ne touche pas les bornes.
    // -----------------------------------------------------------------
    {
        bool reversible = true;
        for (int cran = 1; cran < nombre_de_crans - 1; ++cran) {
            if (reduit(agrandit(cran)) != cran || agrandit(reduit(cran)) != cran)
                reversible = false;
        }
        verifie(reversible, "un aller-retour rend le cran de depart",
            "une paire agrandir/reduire ne se compense pas");
    }

    // -----------------------------------------------------------------
    // L'echelle est strictement croissante. Sans cela, « agrandir »
    // pourrait rendre une page plus petite.
    // -----------------------------------------------------------------
    {
        bool croissante = true;
        for (int cran = 1; cran < nombre_de_crans; ++cran) {
            if (crans[cran] <= crans[cran - 1])
                croissante = false;
        }
        verifie(croissante, "l'echelle est strictement croissante",
            "un cran n'est pas plus grand que le precedent");
    }

    // -----------------------------------------------------------------
    // La reciproque : `borne` ne doit pas tout ramener au meme cran, sans
    // quoi tous les tests ci-dessus passeraient avec un zoom fige.
    // -----------------------------------------------------------------
    {
        verifie(pourcent(cran_neutre) != pourcent(agrandit(cran_neutre)),
            "agrandir change reellement le pourcentage", "le zoom serait fige");
        verifie(pourcent(cran_neutre) != pourcent(reduit(cran_neutre)),
            "reduire change reellement le pourcentage", "le zoom serait fige");
    }

    std::printf("\n");
    if (echecs > 0) {
        std::printf("echelle de zoom : %d cas en echec\n", echecs);
        return 1;
    }
    std::printf("echelle de zoom : tous les cas passent\n");
    return 0;
}
