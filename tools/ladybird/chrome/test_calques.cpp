// Banc d'essai hote de BouchaudCalques.
//
//     g++ -std=c++20 -Wall -Wextra -Werror -o calques test_calques.cpp && ./calques
//
// Ce qui est verifie, ce sont les deux fautes qui laissent des trainees a
// l'ecran et que rien d'autre n'attrape :
//
//   * un calque qui disparait ou se deplace doit faire reecrire la ou il
//     ETAIT, sinon ses pixels restent ;
//   * un calque immobile doit etre redessine des que la page se repeint
//     dessous, sinon un trou rectangulaire s'y ouvre.
//
// Elles ne font echouer aucun test d'integration : la capture du smoke passe,
// et c'est l'oeil de l'utilisateur qui trouve le defaut.

#include "BouchaudCalques.h"

#include <cstdio>

using namespace BouchaudCalques;

// Le suivi ne connait que des rectangles numerotes : ce sont ces noms-la, du
// cote du chrome, qui leur donnent un sens. Les redeclarer ici plutot que de
// les importer garde le banc d'essai independant de l'interface -- il exerce
// l'arithmetique, pas la liste des elements qui existent aujourd'hui.
enum : int {
    Survol = 0,
    Recherche = 1,
    Menu = 2,
};

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
    std::printf("  ECHEC  %s\n         obtenu {%d,%d,%d,%d} attendu {%d,%d,%d,%d}\n",
        cas, obtenu.x, obtenu.y, obtenu.w, obtenu.h,
        attendu.x, attendu.y, attendu.w, attendu.h);
    echecs += 1;
}

int main()
{
    std::printf("BouchaudCalques\n");

    // -----------------------------------------------------------------
    // 1. Boite englobante : c'est elle qui rend le degat d'un calque sur.
    // -----------------------------------------------------------------
    {
        verifie_rect(Rect {}.englobe(Rect {}), Rect {},
            "englobe : vide + vide = vide");
        verifie_rect(Rect {}.englobe(Rect { 4, 5, 6, 7 }), Rect { 4, 5, 6, 7 },
            "englobe : vide est neutre a gauche");
        verifie_rect(Rect { 4, 5, 6, 7 }.englobe(Rect {}), Rect { 4, 5, 6, 7 },
            "englobe : vide est neutre a droite");
        verifie_rect(Rect { 0, 0, 10, 10 }.englobe(Rect { 20, 20, 5, 5 }),
            Rect { 0, 0, 25, 25 }, "englobe : deux rectangles disjoints");
        verifie_rect(Rect { 10, 10, 10, 10 }.englobe(Rect { 12, 12, 2, 2 }),
            Rect { 10, 10, 10, 10 }, "englobe : le contenu ne change rien");
        // Une largeur negative est vide, et un vide ne doit jamais elargir.
        verifie_rect(Rect { 100, 100, -5, 4 }.englobe(Rect { 0, 0, 2, 2 }),
            Rect { 0, 0, 2, 2 }, "englobe : une largeur negative reste vide");
    }

    // -----------------------------------------------------------------
    // 2. Rien ne bouge : aucun degat. C'est le cas de la trame ordinaire,
    //    et le seul qui doit rester gratuit.
    // -----------------------------------------------------------------
    {
        Suivi suivi;
        suivi.place(Survol, Rect { 0, 600, 200, 20 });
        suivi.acte();
        suivi.place(Survol, Rect { 0, 600, 200, 20 });
        verifie(suivi.degat().vide(), "calque immobile : aucun degat",
            "un calque qui n'a pas bouge a demande a reecrire des pixels");
    }

    // -----------------------------------------------------------------
    // 3. Apparition, deplacement, disparition.
    // -----------------------------------------------------------------
    {
        Suivi suivi;
        suivi.place(Survol, Rect { 0, 600, 200, 20 });
        verifie_rect(suivi.degat(), Rect { 0, 600, 200, 20 },
            "apparition : la nouvelle boite");
        suivi.acte();

        suivi.place(Survol, Rect { 0, 600, 320, 20 });
        verifie_rect(suivi.degat(), Rect { 0, 600, 320, 20 },
            "elargissement : l'union des deux boites");
        suivi.acte();

        // Le cas qui laisse une trainee si on ne prend que la nouvelle boite.
        suivi.place(Survol, Rect { 500, 100, 40, 20 });
        verifie_rect(suivi.degat(), Rect { 0, 100, 540, 520 },
            "deplacement : l'ancienne boite est comprise");
        suivi.acte();

        suivi.place(Survol, Rect {});
        verifie_rect(suivi.degat(), Rect { 500, 100, 40, 20 },
            "disparition : la ou le calque etait");
        suivi.acte();

        verifie(suivi.degat().vide(), "apres disparition : plus rien",
            "un calque absent continue de demander des pixels");
    }

    // -----------------------------------------------------------------
    // 4. Contenu change, boite identique : le compteur de recherche.
    // -----------------------------------------------------------------
    {
        Suivi suivi;
        suivi.place(Recherche, Rect { 800, 40, 300, 28 });
        suivi.acte();
        suivi.place(Recherche, Rect { 800, 40, 300, 28 });
        verifie(suivi.degat().vide(), "sans salir : rien a faire",
            "une boite identique a produit du degat");
        suivi.salit(Recherche);
        verifie_rect(suivi.degat(), Rect { 800, 40, 300, 28 },
            "salir : la boite courante");
        suivi.acte();
        verifie(suivi.degat().vide(), "acter efface le salissement",
            "le salissement a survecu a la trame");
    }

    // -----------------------------------------------------------------
    // 5. Plusieurs calques : le degat est leur union.
    // -----------------------------------------------------------------
    {
        Suivi suivi;
        suivi.place(Survol, Rect { 0, 600, 200, 20 });
        suivi.place(Menu, Rect { 400, 100, 160, 120 });
        verifie_rect(suivi.degat(), Rect { 0, 100, 560, 520 },
            "deux calques : l'union");
        suivi.acte();

        suivi.place(Survol, Rect { 0, 600, 200, 20 });
        suivi.place(Menu, Rect {});
        verifie_rect(suivi.degat(), Rect { 400, 100, 160, 120 },
            "un seul des deux disparait");
    }

    // -----------------------------------------------------------------
    // 6. La regle de dessin : tout calque que le rectangle publie touche.
    // -----------------------------------------------------------------
    {
        Rect bulle { 0, 600, 200, 20 };
        verifie(doit_redessiner(bulle, Rect { 0, 590, 1278, 40 }),
            "page repeinte sous la bulle : redessiner",
            "un calque immobile n'a pas ete redessine sous une capture");
        verifie(doit_redessiner(bulle, Rect { 199, 619, 1, 1 }),
            "un pixel commun suffit",
            "un chevauchement d'un pixel n'a pas ete vu");
        verifie(!doit_redessiner(bulle, Rect { 0, 36, 1278, 100 }),
            "page repeinte ailleurs : ne rien faire",
            "un calque hors du rectangle publie a ete redessine");
        verifie(!doit_redessiner(Rect {}, Rect { 0, 0, 1278, 640 }),
            "calque absent : jamais dessine",
            "un calque vide a ete dessine");
        verifie(!doit_redessiner(bulle, Rect {}),
            "rien de publie : rien a dessiner",
            "un calque a ete dessine sans rectangle publie");
    }

    // -----------------------------------------------------------------
    // 7. Un calque visible prend les clics qui le touchent.
    // -----------------------------------------------------------------
    {
        Rect barre { 950, 44, 320, 32 };
        verifie(contient(barre, 950, 44), "coin haut-gauche inclus",
            "le premier pixel du calque n'a pas pris le clic");
        verifie(contient(barre, 1269, 75), "coin bas-droit inclus",
            "le dernier pixel du calque n'a pas pris le clic");
        verifie(!contient(barre, 1270, 60), "un pixel a droite : dehors",
            "le calque a pris un clic hors de sa boite");
        verifie(!contient(barre, 960, 43), "un pixel au-dessus : dehors",
            "le calque a pris un clic hors de sa boite");
        verifie(!contient(Rect {}, 0, 0), "un calque absent ne prend rien",
            "un calque vide a pris un clic, qui n'irait donc pas a la page");
    }

    // -----------------------------------------------------------------
    // 8. Un indice hors bornes ne corrompt rien.
    //
    //    Les indices viennent de l'enumeration et non d'une entree, mais un
    //    tableau ecrit hors bornes ne se voit pas la ou la faute est commise.
    // -----------------------------------------------------------------
    {
        Suivi suivi;
        suivi.place(maximum, Rect { 0, 0, 10, 10 });
        suivi.place(-1, Rect { 0, 0, 10, 10 });
        suivi.salit(maximum);
        verifie(suivi.degat().vide(), "indice hors bornes : ignore",
            "un indice invalide a produit du degat");
        verifie(suivi.boite(maximum).vide() && !suivi.visible(-1),
            "indice hors bornes : boite vide",
            "un indice invalide a rendu une boite");
    }

    std::printf("\n");
    if (echecs > 0) {
        std::printf("calques : %d cas en echec\n", echecs);
        return 1;
    }
    std::printf("calques : tous les cas passent\n");
    return 0;
}
