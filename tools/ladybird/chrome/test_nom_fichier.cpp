// Banc d'essai hote de BouchaudNomFichier.
//
//     g++ -std=c++20 -Wall -Wextra -Werror -o nom test_nom_fichier.cpp && ./nom
//
// L'entree est HOSTILE : elle vient d'un en-tete `Content-Disposition`,
// c'est-a-dire de la machine d'en face. Ce qui est verifie n'est donc pas « la
// fonction fait ce qu'elle fait » mais les quatre garanties dont depend le
// fait qu'un serveur ne choisisse pas ou l'on ecrit : jamais de separateur,
// jamais de point en tete, jamais vide, jamais plus long que la borne.

#include "BouchaudNomFichier.h"

#include <cstdio>
#include <cstring>

using namespace BouchaudNomFichier;

static int echecs = 0;

static Nom assainit(char const* propose)
{
    return BouchaudNomFichier::assainit(propose, static_cast<int>(std::strlen(propose)));
}

static void verifie(bool condition, char const* cas, char const* detail)
{
    if (condition) {
        std::printf("  ok     %s\n", cas);
        return;
    }
    std::printf("  ECHEC  %s\n         %s\n", cas, detail);
    echecs += 1;
}

static void verifie_nom(char const* propose, char const* attendu, char const* cas)
{
    auto const obtenu = assainit(propose);
    if (std::strcmp(obtenu.c_str(), attendu) == 0) {
        std::printf("  ok     %s\n", cas);
        return;
    }
    std::printf("  ECHEC  %s\n         « %s » -> « %s », attendu « %s »\n",
        cas, propose, obtenu.c_str(), attendu);
    echecs += 1;
}

int main()
{
    std::printf("BouchaudNomFichier\n");

    // -----------------------------------------------------------------
    // 1. Un nom ordinaire traverse sans changer.
    // -----------------------------------------------------------------
    verifie_nom("rapport-2026.pdf", "rapport-2026.pdf", "un nom ordinaire");
    verifie_nom("Mon Document (1).txt", "Mon Document (1).txt", "espaces et parentheses");

    // -----------------------------------------------------------------
    // 2. La traversee de repertoire. La faute qu'on ne veut pas commettre.
    // -----------------------------------------------------------------
    // Les deux points de tete disparaissent ensuite avec la regle du fichier
    // cache : `..` remplace par rien laisserait `_ladybird_...`, ce qui est
    // exactement ce qu'on veut -- un nom qui montre qu'on a retire quelque
    // chose, plutot qu'un nom qui a l'air choisi par le serveur.
    verifie_nom("../ladybird/profile/cookies.sqlite",
        "_ladybird_profile_cookies.sqlite", "traversee vers le profil");
    verifie_nom("/etc/passwd", "_etc_passwd", "chemin absolu");
    verifie_nom("..", "telechargement", "le parent seul ne reste rien");
    verifie_nom(".", "telechargement", "le point seul ne reste rien");
    verifie_nom("....", "telechargement", "une suite de points non plus");
    verifie_nom("a\\b", "a_b", "la barre inverse aussi");

    {
        auto const nom = assainit("../../../persist/ladybird/data/cookies.sqlite");
        verifie(std::strchr(nom.c_str(), '/') == nullptr,
            "aucun separateur ne survit",
            "un separateur a survecu : le serveur choisirait ou l'on ecrit");
        verifie(nom.octets[0] != '.',
            "aucun point en tete",
            "un point en tete : le fichier serait cache");
    }

    // -----------------------------------------------------------------
    // 3. Le fichier cache.
    // -----------------------------------------------------------------
    verifie_nom(".bashrc", "bashrc", "un point de tete saute");
    verifie_nom("  .profile", "profile", "espaces puis point");

    // -----------------------------------------------------------------
    // 4. Les octets qui ne sont pas du texte.
    // -----------------------------------------------------------------
    {
        char const brut[] = { 'a', '\n', 'b', '\t', 'c', '\0' };
        verifie_nom(brut, "a_b_c", "les caracteres de controle deviennent des soulignes");
    }
    {
        // Un octet de direction droite-a-gauche (U+202E) renverse l'affichage
        // d'un nom : « photo », ce marqueur, puis « gnp.exe » se lit
        // « photoexe.png ». C'est un tour connu -- et le compilateur lui-meme
        // refuse ce caractere dans un source, ce pour quoi il est ecrit ici en
        // octets. La liste blanche le neutralise sans avoir eu a y penser.
        // la liste blanche le neutralise sans avoir eu a y penser.
        char const trompeur[] = { 'p', 'h', 'o', 't', 'o',
            static_cast<char>(0xe2), static_cast<char>(0x80), static_cast<char>(0xae),
            'g', 'n', 'p', '.', 'e', 'x', 'e', '\0' };
        auto const nom = assainit(trompeur);
        verifie(std::strcmp(nom.c_str(), "photo___gnp.exe") == 0,
            "un marqueur de direction est neutralise",
            "un octet de controle bidirectionnel a survecu");
    }

    // -----------------------------------------------------------------
    // 5. La borne de longueur.
    // -----------------------------------------------------------------
    {
        char long_nom[400];
        std::memset(long_nom, 'a', sizeof(long_nom) - 1);
        long_nom[sizeof(long_nom) - 1] = '\0';
        auto const nom = assainit(long_nom);
        verifie(nom.taille == longueur_max,
            "un nom demesure est borne",
            "la borne de longueur n'a pas ete appliquee");
        verifie(std::strlen(nom.c_str()) == static_cast<size_t>(longueur_max),
            "et il reste termine par un nul",
            "le tampon n'est plus une chaine C valide");
    }

    // -----------------------------------------------------------------
    // 6. Le vide, sous toutes ses formes.
    // -----------------------------------------------------------------
    verifie_nom("", "telechargement", "un nom vide");
    verifie_nom("   ", "telechargement", "des espaces seuls");
    {
        auto const nom = BouchaudNomFichier::assainit(nullptr, 12);
        verifie(std::strcmp(nom.c_str(), "telechargement") == 0,
            "un pointeur nul ne fait pas tomber",
            "un pointeur nul n'a pas donne le nom par defaut");
    }

    // -----------------------------------------------------------------
    // 7. Ce que la fonction ne rend JAMAIS, quelle que soit l'entree.
    // -----------------------------------------------------------------
    {
        char const* entrees[] = {
            "", ".", "..", "../..", "/", "//", "./.", "   . ",
            "a/b/c", "\\\\serveur\\part", "con.txt", "-", "_",
        };
        for (auto const* entree : entrees) {
            auto const nom = assainit(entree);
            verifie(nom.taille > 0 && nom.taille <= longueur_max
                    && nom.octets[0] != '.' && nom.octets[0] != ' '
                    && std::strchr(nom.c_str(), '/') == nullptr
                    && std::strcmp(nom.c_str(), ".") != 0
                    && std::strcmp(nom.c_str(), "..") != 0,
                entree[0] == '\0' ? "invariants sur la chaine vide" : entree,
                "une entree hostile a viole un invariant");
        }
    }

    std::printf("\n");
    if (echecs > 0) {
        std::printf("nom de fichier : %d cas en echec\n", echecs);
        return 1;
    }
    std::printf("nom de fichier : tous les cas passent\n");
    return 0;
}
