// Banc d'essai hote de BouchaudTampons (BOUCHAUD_GUI_SURFACE_DEUX_TAMPONS_V1).
//
//     g++ -std=c++20 -Wall -Wextra -Werror -o tampons test_tampons.cpp && ./tampons
//
// Les proprietes dont dependent les pixels :
//
//   * le client n'ecrit jamais la face, ni un tampon non rendu ;
//   * apres le rejeu, le tampon arriere porte EXACTEMENT la derniere trame
//     annoncee -- verifie sur une surface simulee, pixel par pixel, contre
//     une surface unique de reference, sur des trames aleatoires ;
//   * la regle de contrat est celle du noyau.

#include "BouchaudTampons.h"

#include <cstdint>
#include <cstdio>
#include <vector>

using namespace BouchaudTampons;

static int echecs = 0;

static void verifie(bool condition, char const* cas)
{
    if (condition) {
        std::printf("  ok     %s\n", cas);
        return;
    }
    std::printf("  ECHEC  %s\n", cas);
    echecs += 1;
}

static constexpr int L = 40;
static constexpr int H = 24;

struct Surface {
    std::vector<uint32_t> px = std::vector<uint32_t>(L * H, 0);
    void peint(Rect r, uint32_t c)
    {
        for (int y = r.y; y < r.y + r.h; ++y)
            for (int x = r.x; x < r.x + r.w; ++x)
                px[y * L + x] = c;
    }
    void copie_depuis(Surface const& s, Rect r)
    {
        for (int y = r.y; y < r.y + r.h; ++y)
            for (int x = r.x; x < r.x + r.w; ++x)
                px[y * L + x] = s.px[y * L + x];
    }
};

int main()
{
    std::printf("== deux tampons du chrome ==\n");

    verifie(Client::negocie(1, true).nombre == 1, "sans offre : un tampon");
    verifie(Client::negocie(2, false).nombre == 1, "sans volonte : un tampon");
    verifie(Client::negocie(2, true).nombre == 2, "offre et volonte : deux tampons");

    {
        Client v1 = Client::negocie(1, true);
        verifie(v1.choisit() == 0 && v1.rejeu(0, L, H).vide(), "V1 : toujours le tampon 0, aucun rejeu");
    }

    {
        Client c = Client::negocie(2, true);
        int a = c.choisit();
        verifie(a == 0 && c.rejeu(a, L, H).vide(), "premiere trame : tampon 0, rien a rejouer");
        c.annonce(a, Rect { 0, 0, L, H });
        int b = c.choisit();
        verifie(b == 1, "deuxieme trame : l'autre tampon");
        verifie(c.rejeu(b, L, H) == (Rect { 0, 0, L, H }), "un tampon jamais annonce recoit toute la surface");
        c.annonce(b, Rect { 1, 1, 2, 2 });
        verifie(c.choisit() == -1, "sans Liberation, aucun tampon libre");
        verifie(!c.libere(1), "la face ne se libere pas");
        verifie(c.libere(0), "l'ancienne face se libere");
        verifie(!c.libere(0), "une liberation ne compte qu'une fois");
        verifie(!c.libere(7), "indice hors bornes refuse");
        verifie(c.choisit() == 0 && c.rejeu(0, L, H) == (Rect { 1, 1, 2, 2 }),
            "le tampon rendu rejoue ce que la face a publie");
    }

    {
        // Une annonce ratee (canal plein) : le meme tampon reste l'arriere.
        Client c = Client::negocie(2, true);
        c.choisit();
        c.annonce(0, Rect { 0, 0, L, H });
        int b = c.choisit();
        verifie(c.choisit() == b, "un tampon ecrit mais pas annonce reste l'arriere");
    }

    {
        // Une annonce RATEE (canal plein), puis une nouvelle composition dans
        // le meme tampon arriere : ce qui a deja ete peint ne doit pas etre
        // ecrase par un second rejeu depuis la face -- le suivi du chrome le
        // croit present et ne le repeindra pas.
        Client c = Client::negocie(2, true);
        Surface tampons[2], reference;
        int t = c.choisit();
        tampons[t].peint(Rect { 0, 0, L, H }, 0x111111);
        reference.peint(Rect { 0, 0, L, H }, 0x111111);
        c.annonce(t, Rect { 0, 0, L, H });
        // Trame 2 : peinte, annonce ratee.
        t = c.choisit();
        tampons[t].copie_depuis(tampons[c.face], c.rejeu(t, L, H));
        c.rejeu_fait(t);
        tampons[t].peint(Rect { 2, 2, 5, 5 }, 0x222222);
        reference.peint(Rect { 2, 2, 5, 5 }, 0x222222);
        // Trame 3 : meme tampon (pas annonce), nouveau dessin, annonce reussie.
        int t3 = c.choisit();
        tampons[t3].copie_depuis(tampons[c.face], c.rejeu(t3, L, H));
        c.rejeu_fait(t3);
        tampons[t3].peint(Rect { 20, 10, 3, 3 }, 0x333333);
        reference.peint(Rect { 20, 10, 3, 3 }, 0x333333);
        c.annonce(t3, Rect { 0, 0, L, H });
        verifie(t3 == t && tampons[c.face].px == reference.px,
            "annonce ratee puis nouvelle trame : rien de ce qui etait peint n'est ecrase");
    }

    {
        // L'equivalence pixel a pixel : deux tampons + rejeu == une surface
        // unique, trame apres trame, la face etant ce que le gestionnaire lit.
        uint32_t graine = 0x9E3779B9u;
        auto alea = [&]() {
            graine ^= graine << 13;
            graine ^= graine >> 17;
            graine ^= graine << 5;
            return graine;
        };
        bool ok = true;
        bool ecrit_face = false;
        for (int essai = 0; essai < 200 && ok; ++essai) {
            Client c = Client::negocie(2, true);
            Surface tampons[2], reference;
            Rect publie_en_attente {};
            for (int trame = 1; trame < 60 && ok; ++trame) {
                int t = c.choisit();
                if (t < 0) {
                    // Le gestionnaire a lu la face : il rend l'autre.
                    ok = c.libere(1 - c.face);
                    t = c.choisit();
                }
                if (t == c.face)
                    ecrit_face = true;
                tampons[t].copie_depuis(tampons[c.face < 0 ? t : c.face], c.rejeu(t, L, H));
                c.rejeu_fait(t);
                int x = static_cast<int>(alea() % L), y = static_cast<int>(alea() % H);
                Rect r { x, y, 1 + static_cast<int>(alea() % (L - x)), 1 + static_cast<int>(alea() % (H - y)) };
                uint32_t couleur = alea() | 1;
                tampons[t].peint(r, couleur);
                reference.peint(r, couleur);
                publie_en_attente = publie_en_attente.englobe(r);
                // Une annonce sur quatre echoue (canal plein) : la suivante
                // republie tout ce qui a ete peint depuis.
                if (alea() % 4 == 0)
                    continue;
                c.annonce(t, publie_en_attente);
                publie_en_attente = {};
                ok = tampons[c.face].px == reference.px;
            }
        }
        verifie(ok, "200 x 60 trames aleatoires (une annonce sur quatre ratee) : la face egale la surface unique, pixel a pixel");
        verifie(!ecrit_face, "le client n'a jamais ecrit la face");
    }

    std::printf("\n");
    if (echecs > 0) {
        std::printf("deux tampons : %d cas en echec\n", echecs);
        return 1;
    }
    std::printf("deux tampons : tous les cas passent\n");
    return 0;
}
