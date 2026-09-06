// Banc d'essai hote de BouchaudDegat.
//
// Le chrome complet ne s'execute que dans QEMU, apres une construction de
// Ladybird de vingt minutes, et seulement dans le scenario browser-host. Cette
// arithmetique-la, elle, ne depend de rien : elle se compile et se verifie ici
// en une seconde.
//
//     g++ -std=c++20 -Wall -Wextra -Werror -o degat test_degat.cpp && ./degat
//
// Ce qui est verifie n'est pas « le code fait ce que le code fait » mais les
// proprietes dont dependent les pixels a l'ecran :
//
//   * la premiere trame d'une geometrie est COMPLETE, meme si le moteur
//     n'annonce qu'un degat minuscule -- sinon la surface garde ce que la
//     fenetre precedente y avait laisse ;
//   * tout changement de geometrie redevient complet -- une fenetre agrandie,
//     une capture retrecie ;
//   * rien n'est jamais publie hors de la surface ;
//   * `publie` est decale de la hauteur de la barre d'outils, `copie` ne l'est
//     pas ;
//   * un degat entierement hors page ne publie rien du tout, plutot qu'un
//     rectangle vide que le compositeur devrait ignorer.

#include "BouchaudDegat.h"

#include <cstdio>

using namespace BouchaudDegat;

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

static void verifie_rect(Rect obtenu, Rect attendu, char const* cas)
{
    if (obtenu == attendu) {
        std::printf("  ok     %s\n", cas);
        return;
    }
    std::printf("  ECHEC  %s\n         attendu {%d,%d %dx%d}, obtenu {%d,%d %dx%d}\n",
        cas, attendu.x, attendu.y, attendu.w, attendu.h,
        obtenu.x, obtenu.y, obtenu.w, obtenu.h);
    echecs += 1;
}

// La geometrie mesuree sur la machine de l'utilisateur : surface 1278x626,
// barre d'outils de 36 pixels, capture de page 1278x590.
static constexpr Geometrie kEcran { 1278, 626, 36, 1278, 590 };

int main()
{
    std::printf("== degat partiel du chrome ==\n");

    // -----------------------------------------------------------------
    // 1. La premiere trame est complete, quel que soit le degat annonce.
    // -----------------------------------------------------------------
    {
        Suivi suivi;
        auto plan = suivi.planifie(kEcran, Rect { 10, 10, 2, 16 });
        verifie(plan.complet, "premiere trame : complete", "un degat minuscule l'a emportee");
        verifie_rect(plan.copie, Rect { 0, 0, 1278, 590 }, "premiere trame : copie toute la page");
        verifie_rect(plan.publie, Rect { 0, 36, 1278, 590 }, "premiere trame : publie sous la barre");
        verifie(!plan.efface_necessaire,
            "premiere trame : la capture couvre la page, pas d'effacement",
            "un effacement inutile a ete demande");
    }

    // -----------------------------------------------------------------
    // 2. Le curseur qui clignote : deux pixels, pas 1 554 048.
    // -----------------------------------------------------------------
    {
        Suivi suivi;
        suivi.planifie(kEcran, {});
        auto plan = suivi.planifie(kEcran, Rect { 400, 120, 2, 17 });
        verifie(!plan.complet, "curseur : trame partielle", "la trame est restee complete");
        verifie_rect(plan.copie, Rect { 400, 120, 2, 17 }, "curseur : copie le curseur seul");
        verifie_rect(plan.publie, Rect { 400, 156, 2, 17 }, "curseur : publie decale de 36");
        verifie(!plan.efface_necessaire, "curseur : aucun effacement",
            "un effacement inutile a ete demande");
    }

    // -----------------------------------------------------------------
    // 3. Un degat plus grand que la page est borne, pas refuse.
    // -----------------------------------------------------------------
    {
        Suivi suivi;
        suivi.planifie(kEcran, {});
        auto plan = suivi.planifie(kEcran, Rect { -50, -50, 5000, 5000 });
        verifie_rect(plan.copie, Rect { 0, 0, 1278, 590 }, "degat deborde : copie bornee");
        verifie_rect(plan.publie, Rect { 0, 36, 1278, 590 }, "degat deborde : publie borne");
        verifie(plan.complet, "degat deborde : reconnu comme complet",
            "un degat couvrant toute la page n'a pas ete vu comme complet");
    }

    // -----------------------------------------------------------------
    // 4. Un degat entierement hors page ne publie rien.
    //
    // C'est le cas qui rend la boucle silencieuse : quand le moteur signale un
    // changement qui ne touche pas la fenetre, le compositeur ne doit pas etre
    // reveille.
    // -----------------------------------------------------------------
    {
        Suivi suivi;
        suivi.planifie(kEcran, {});
        auto plan = suivi.planifie(kEcran, Rect { 2000, 2000, 40, 40 });
        verifie(plan.rien_a_faire(), "degat hors page : rien publie",
            "un rectangle a ete publie hors de la page");
        verifie(plan.copie.vide(), "degat hors page : rien copie", "une copie a ete demandee");
    }

    // -----------------------------------------------------------------
    // 5. Une capture plus courte que la page laisse un bas a effacer.
    // -----------------------------------------------------------------
    {
        Geometrie courte { 1278, 626, 36, 1278, 200 };
        Suivi suivi;
        auto plan = suivi.planifie(courte, {});
        verifie(plan.efface_necessaire, "capture courte : effacement demande",
            "le bas de la page serait reste sur l'ancienne image");
        verifie_rect(plan.efface, Rect { 0, 0, 1278, 590 }, "capture courte : efface toute la page");
        verifie_rect(plan.copie, Rect { 0, 0, 1278, 200 }, "capture courte : copie ce qui existe");
    }

    // -----------------------------------------------------------------
    // 6. Agrandir la fenetre redonne une trame complete.
    // -----------------------------------------------------------------
    {
        Suivi suivi;
        suivi.planifie(kEcran, {});
        auto partielle = suivi.planifie(kEcran, Rect { 4, 4, 8, 8 });
        verifie(!partielle.complet, "avant agrandissement : partielle", "deja complete");

        Geometrie agrandie { 1278, 700, 36, 1278, 664 };
        auto plan = suivi.planifie(agrandie, Rect { 4, 4, 8, 8 });
        verifie(plan.complet, "agrandissement : trame complete",
            "la zone nouvellement decouverte serait restee vide");
        verifie_rect(plan.publie, Rect { 0, 36, 1278, 664 }, "agrandissement : publie la nouvelle page");
    }

    // -----------------------------------------------------------------
    // 7. Une capture qui RETRECIT redevient complete, elle aussi.
    //
    // Sans cela le bas de l'ancienne page resterait affiche sous la nouvelle,
    // et aucun degat futur ne viendrait jamais l'effacer.
    // -----------------------------------------------------------------
    {
        Suivi suivi;
        suivi.planifie(kEcran, {});
        Geometrie capture_courte { 1278, 626, 36, 1278, 300 };
        auto plan = suivi.planifie(capture_courte, Rect { 4, 4, 8, 8 });
        verifie(plan.complet, "capture retrecie : trame complete",
            "l'ancienne page serait restee visible sous la nouvelle");
        verifie(plan.efface_necessaire, "capture retrecie : effacement demande",
            "aucun effacement, donc une trainee");
    }

    // -----------------------------------------------------------------
    // 8. La reciproque : une geometrie INCHANGEE ne force pas le complet.
    //
    // Sans ce test, une implementation qui rendrait toujours `complet` passerait
    // tous les cas ci-dessus, et ne corrigerait rien.
    // -----------------------------------------------------------------
    {
        Suivi suivi;
        suivi.planifie(kEcran, {});
        for (int tour = 0; tour < 5; ++tour) {
            auto plan = suivi.planifie(kEcran, Rect { 400, 120, 2, 17 });
            if (plan.complet) {
                verifie(false, "geometrie stable : reste partielle",
                    "une trame est redevenue complete sans raison");
                break;
            }
            if (tour == 4)
                verifie(true, "geometrie stable : reste partielle", "");
        }
    }

    // -----------------------------------------------------------------
    // 9. `invalide()` force la trame suivante, et une seule.
    // -----------------------------------------------------------------
    {
        Suivi suivi;
        suivi.planifie(kEcran, {});
        suivi.invalide();
        auto forcee = suivi.planifie(kEcran, Rect { 1, 1, 1, 1 });
        verifie(forcee.complet, "invalide() : trame suivante complete", "l'invalidation n'a rien fait");
        auto suivante = suivi.planifie(kEcran, Rect { 1, 1, 1, 1 });
        verifie(!suivante.complet, "invalide() : la trame d'apres redevient partielle",
            "l'invalidation est restee collee");
    }

    // -----------------------------------------------------------------
    // 10. Rien ne sort jamais de la surface.
    // -----------------------------------------------------------------
    {
        Suivi suivi;
        suivi.planifie(kEcran, {});
        Rect degats[] = {
            { -1000, -1000, 1, 1 },
            { -10, -10, 20, 20 },
            { 1270, 580, 100, 100 },
            { 0, 589, 1278, 2 },
            { 1277, 0, 2, 590 },
        };
        bool dedans = true;
        for (auto degat : degats) {
            auto plan = suivi.planifie(kEcran, degat);
            if (plan.rien_a_faire())
                continue;
            dedans = dedans
                && plan.publie.x >= 0 && plan.publie.y >= kEcran.page_haut
                && plan.publie.x + plan.publie.w <= kEcran.surface_largeur
                && plan.publie.y + plan.publie.h <= kEcran.surface_hauteur
                && plan.copie.x >= 0 && plan.copie.y >= 0
                && plan.copie.x + plan.copie.w <= kEcran.capture_largeur
                && plan.copie.y + plan.copie.h <= kEcran.capture_hauteur;
        }
        verifie(dedans, "aucun rectangle ne sort de la surface ni de la capture",
            "un rectangle deborde");
    }

    // -----------------------------------------------------------------
    // 11. Une surface degeneree ne produit aucun travail.
    // -----------------------------------------------------------------
    {
        Suivi suivi;
        Geometrie plate { 1278, 36, 36, 1278, 0 };
        auto plan = suivi.planifie(plate, Rect { 0, 0, 100, 100 });
        verifie(plan.rien_a_faire(), "surface sans page : rien publie",
            "une page de hauteur nulle a produit du travail");
    }

    std::printf("\n");
    if (echecs > 0) {
        std::printf("degat partiel : %d cas en echec\n", echecs);
        return 1;
    }
    std::printf("degat partiel : tous les cas passent\n");
    return 0;
}
