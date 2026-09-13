#include "chrome/BouchaudSortie.h"
#include <cassert>
#include <string>
#include <cstdio>

int main()
{
    BouchaudTransport::Sortie<16, 2> file;
    assert(file.ajoute("abc", 3));
    assert(file.ajoute("DEF", 3));
    assert(!file.ajoute("perdu", 5));
    int appels = 0;
    assert(file.vide([&](auto*, size_t) -> long { ++appels; errno = EAGAIN; return -1; }));
    assert(appels == 1 && file.nombre() == 2);
    std::string recu;
    auto partiel = [&](auto* p, size_t) -> long { recu += static_cast<char>(*p); return 1; };
    assert(file.vide(partiel));
    assert(recu == "ab"); // budget de deux appels, pas une boucle sans borne
    assert(file.vide(partiel));
    assert(recu == "abcD");
    assert(file.ajoute("ghi", 3)); // retour en debut d'anneau
    while (file.nombre())
        assert(file.vide(partiel));
    assert(recu == "abcDEFghi");
    assert(file.ajoute("x", 1));
    assert(file.vide([](auto*, size_t) -> long { errno = EINTR; return -1; }));
    assert(file.nombre() == 1);
    assert(!file.vide([](auto*, size_t) -> long { errno = EPIPE; return -1; }));
    assert(!file.ajoute("y", 1));
    BouchaudTransport::Sortie<2, 1> petite;
    assert(!petite.ajoute("abc", 3));
    assert(petite.ajoute("ab", 2));
    assert(!petite.vide([](auto*, size_t) -> long { return 3; }));
    std::puts("sortie GUI : EAGAIN, EINTR, ecritures partielles, ordre, bornes, fermeture : ok");
}
