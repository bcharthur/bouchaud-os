// Banc d'essai hote de BouchaudUrl.
//
//     g++ -std=c++20 -Wall -Wextra -Werror -o url test_url.cpp && ./url
//
// Ce qui est verifie, ce sont les schemas qui EXECUTENT et les octets qu'un
// fichier peut porter. Une liste de schemas est exactement le genre de code
// qu'on relit sans y voir le trou -- et le trou, ici, est une adresse qui
// evalue du script dans le document courant.

#include "BouchaudUrl.h"

#include <cstdio>
#include <cstring>
#include <initializer_list>

using namespace BouchaudUrl;

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

static bool navigable(char const* url)
{
    return schema_navigable(url, static_cast<int>(std::strlen(url)));
}

static bool stockable(char const* url)
{
    return acceptable_pour_le_magasin(url, static_cast<int>(std::strlen(url)));
}

int main()
{
    std::printf("BouchaudUrl\n");

    // -----------------------------------------------------------------
    // 1. Ce qui passe.
    // -----------------------------------------------------------------
    for (auto const* url : {
             "https://example.com/",
             "http://192.168.1.1:8080/etat",
             "about:blank",
             "file:///usr/share/doc/index.html",
             "https://example.com/a?b=c&d=e#f",
         }) {
        verifie(navigable(url) && stockable(url), url,
            "une adresse legitime a ete refusee");
    }

    // -----------------------------------------------------------------
    // 2. Les schemas qui EXECUTENT. Le coeur du fichier.
    // -----------------------------------------------------------------
    for (auto const* url : {
             "javascript:alert(document.cookie)",
             "JAVASCRIPT:alert(1)",
             "javascript:void(0)",
             "data:text/html,<script>fetch('//evil')</script>",
             "data:text/html;base64,PHNjcmlwdD4=",
             "vbscript:msgbox(1)",
         }) {
        verifie(!navigable(url) && !stockable(url), url,
            "un schema qui execute a ete accepte : c'est l'auto-XSS classique");
    }

    // -----------------------------------------------------------------
    // 3. Ce qui ressemble a un schema permis sans en etre un.
    // -----------------------------------------------------------------
    for (auto const* url : {
             "https:/example.com",   // une seule barre
             "http:example.com",
             "httpsx://example.com",
             "aboutx:blank",
             " https://example.com", // espace de tete : ce n'est pas le schema
             "",
         }) {
        verifie(!navigable(url), url,
            "un faux schema a ete pris pour un vrai");
    }

    // -----------------------------------------------------------------
    // 4. Les octets qu'un FICHIER peut porter et qu'un clavier ne produit pas.
    // -----------------------------------------------------------------
    {
        char const saut[] = "https://example.com/\nhttps://evil.example/";
        verifie(!stockable(saut), "un saut de ligne coupe l'enregistrement",
            "une adresse portant un saut de ligne a ete acceptee : elle "
            "fabriquerait une seconde ligne dans le magasin");

        char const tabulation[] = "https://example.com/\tautre";
        verifie(!stockable(tabulation), "une tabulation est le separateur",
            "une adresse portant le separateur de champs a ete acceptee");

        char const nul[] = { 'h', 't', 't', 'p', 's', ':', '/', '/', 'a', '\0' };
        verifie(acceptable_pour_le_magasin(nul, 9),
            "un octet nul en fin n'est pas lu",
            "la taille explicite n'a pas ete respectee");

        char const bidi[] = { 'h', 't', 't', 'p', 's', ':', '/', '/', 'a',
            static_cast<char>(0xe2), static_cast<char>(0x80), static_cast<char>(0xae),
            'b', '\0' };
        verifie(!acceptable_pour_le_magasin(bidi, 13),
            "un marqueur de direction est refuse",
            "une adresse qui se lit a l'envers a ete acceptee");
    }

    // -----------------------------------------------------------------
    // 5. La borne de longueur.
    // -----------------------------------------------------------------
    {
        char longue[longueur_max + 64];
        std::memset(longue, 'a', sizeof(longue));
        std::memcpy(longue, "https://", 8);
        verifie(!acceptable_pour_le_magasin(longue, static_cast<int>(sizeof(longue))),
            "une adresse demesuree est refusee",
            "la borne de longueur n'a pas ete appliquee");
        verifie(acceptable_pour_le_magasin(longue, longueur_max),
            "exactement a la borne, elle passe",
            "la borne a rogne un octet de trop");
    }

    std::printf("\n");
    if (echecs > 0) {
        std::printf("url : %d cas en echec\n", echecs);
        return 1;
    }
    std::printf("url : tous les cas passent\n");
    return 0;
}
