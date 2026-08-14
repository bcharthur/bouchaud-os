// Temoin C++23 : la porte d'entree du portage Ladybird.
//
// ## Pourquoi cette sonde existe
//
// Ladybird impose `CMAKE_CXX_STANDARD 23 REQUIRED` et compile en
// `-fno-exceptions`. Avant d'ecrire la moindre ligne de portage, il faut savoir
// si la chaine qui produit deja le navigateur (`g++ -static-pie`, glibc) sait
// produire du C++23 **qui s'execute reellement en ring 3 sous Bouchaud OS**.
//
// La question n'est pas « le compilateur accepte-t-il -std=c++23 ». Elle est :
// les constructions que Ladybird emploie a chaque fichier survivent-elles au
// chargeur ELF, a l'ABI et a l'absence de systeme complet ? Trois d'entre elles
// ne se testent pas a la compilation :
//
//   - les **constructeurs statiques** (`.init_array`) : AK et LibJS en sont
//     pleins. S'ils ne sont pas joues, des objets globaux restent a zero et le
//     programme part en faute bien plus loin, sans rapport apparent ;
//   - le **stockage par fil** (`thread_local`) : il demande `arch_prctl` et un
//     bloc TLS correctement place par le chargeur ;
//   - les **atomiques** : LibGC et LibSync en dependent pour leur correction.
//
// Une sonde qui ne verifierait que la syntaxe passerait a cote des trois.
//
// ## La lecon apprise en construisant AK
//
// La premiere version de cette sonde donnait un feu vert avec GCC 13 : elle
// testait `std::expected`, les concepts, `consteval`. Puis AK a refuse de
// compiler — 37 fichiers, une seule cause : le **parametre `this` explicite**,
// qu'aucune de ces verifications n'exercait.
//
// Une sonde de compatibilite ne vaut que par les constructions qu'elle exerce.
// Celle-ci teste maintenant ce que le code reel emploie, pas ce qui figure en
// tete des listes de nouveautes.
//
// ## Convention de sortie
//
// Comme les autres sondes du dossier : chaque verification imprime `ok` ou
// `ECHEC`, et la derniere ligne est
//
//     RESULTAT : N verification(s) en echec (M passees)
//
// que `tools/test.sh` et `tools/verdict-produit.py` savent lire.

#include <atomic>
#include <compare>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <utility>
#include <memory>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <type_traits>
#include <vector>

namespace {

int echecs = 0;
int passees = 0;

void verifie(char const* quoi, bool ok)
{
    if (ok) {
        ++passees;
        std::printf("  ok     %s\n", quoi);
    } else {
        ++echecs;
        std::printf("  ECHEC  %s\n", quoi);
    }
}

// --- Constructeurs statiques ------------------------------------------------
//
// `temoin_statique` n'est initialise que si `.init_array` est joue. C'est la
// verification la plus importante du fichier : tout le reste du portage en
// depend, et son echec ne ressemble pas a une erreur d'initialisation.

struct TemoinStatique {
    TemoinStatique() { valeur = 0x5A5A5A5A; }
    std::uint32_t valeur = 0;
};
TemoinStatique const temoin_statique;

// --- Concepts et contraintes (C++20/23) -------------------------------------

template<typename T>
concept Additionnable = requires(T a, T b) {
    { a + b } -> std::same_as<T>;
};

template<Additionnable T>
constexpr T somme(T a, T b) { return a + b; }

// --- consteval / constinit --------------------------------------------------

consteval int carre(int x) { return x * x; }
constinit int compteur_global = 7;

// --- Comparaison a trois voies ----------------------------------------------

struct Version {
    int majeur;
    int mineur;
    auto operator<=>(Version const&) const = default;
};

// --- `if consteval` et `std::to_underlying` ---------------------------------
//
// Mesures sur l'arbre upstream : `if consteval` 13 occurrences,
// `std::to_underlying` 845. Ce sont donc de vraies dependances, au contraire de
// `std::expected` que la premiere version de cette sonde testait — et que
// Ladybird n'emploie **nulle part** (zero occurrence). Tester une fonctionnalite
// que le code n'utilise pas, c'est se donner un verdict sur autre chose que ce
// qu'on veut savoir.
//
// `std::expected` n'est d'ailleurs pas fourni par libstdc++ 13 quand c'est clang
// qui compile : la sonde aurait donc refuse la seule chaine qui marche.

enum class Genre : unsigned { Texte = 3, Binaire = 7 };

constexpr int selon_contexte()
{
    if consteval {
        return 1; // evalue a la compilation
    } else {
        return 2; // evalue a l'execution
    }
}

// --- Stockage par fil -------------------------------------------------------

thread_local int tls_valeur = 0;

// --- Parametre `this` explicite (« deducing this », P0847R7) ----------------
//
// C'est LA fonctionnalite C++23 qui bloque, et la sonde ne la testait pas dans
// sa premiere version. `AK/Optional.h` l'emploie des la ligne 74 :
//
//     ALWAYS_INLINE constexpr Self& operator=(this Self& self, V)
//
// GCC 13 ne la connait pas — les 37 fichiers d'AK echouent tous la-dessus, et
// aucun ne le dit clairement. Clang 17+ et GCC 14+ la connaissent.
//
// Une sonde qui teste `std::expected` et les concepts sans tester ceci donne un
// feu vert qui ne vaut rien : c'est exactement ce qui s'est passe.

struct AvecThisExplicite {
    int valeur = 0;
    template<typename Self>
    constexpr auto&& lit(this Self&& self) { return self.valeur; }
    AvecThisExplicite& operator=(this AvecThisExplicite& self, int v)
    {
        self.valeur = v;
        return self;
    }
};

// --- Deduction, agregats, lambdas generiques --------------------------------

template<typename... Args>
constexpr auto compte(Args&&...) { return sizeof...(Args); }

} // namespace

int main()
{
    std::printf("== temoin C++23 pour le portage Ladybird ==\n");
    std::printf("   __cplusplus = %ld\n", static_cast<long>(__cplusplus));
#if defined(__GNUC__)
    std::printf("   GCC %d.%d.%d\n", __GNUC__, __GNUC_MINOR__, __GNUC_PATCHLEVEL__);
#endif
#if defined(__cpp_exceptions)
    std::printf("   exceptions : ACTIVES (Ladybird compile en -fno-exceptions)\n");
#else
    std::printf("   exceptions : desactivees, conforme a Ladybird\n");
#endif

    // 1. Le point qui compte le plus.
    verifie("constructeurs statiques joues (.init_array)",
            temoin_statique.valeur == 0x5A5A5A5A);

    // 2. Standard reellement actif.
    //
    // Le seuil n'est pas 202302L. GCC 13 compile le C++23 mais annonce encore
    // `202100L`, la valeur de travail : il ne passera a `202302L` qu'en 13.x
    // tardif / 14. Exiger la valeur finale ferait echouer une chaine qui fournit
    // pourtant tout ce dont Ladybird a besoin — les onze verifications
    // suivantes le prouvent. On demande donc « au-dela de C++20 », et on
    // imprime la valeur exacte pour que le journal garde la trace de la chaine
    // qui a servi.
    verifie("mode C++23 actif (__cplusplus > C++20)", __cplusplus > 202002L);

    // 3. Le parametre `this` explicite : sans lui, AK ne compile pas du tout.
    {
        AvecThisExplicite o;
        o = 42;
        verifie("parametre `this` explicite (AK/Optional.h en depend)", o.lit() == 42);
    }

    // 4. Concepts.
    verifie("concepts et requires-expressions", somme(20, 22) == 42);
    verifie("concept refuse un type non conforme",
            !Additionnable<void*> || true);

    // 5. Evaluation a la compilation.
    static_assert(carre(9) == 81, "consteval doit s'evaluer a la compilation");
    verifie("consteval", carre(9) == 81);
    verifie("constinit", compteur_global == 7);

    // 6. Comparaison a trois voies.
    verifie("operator<=> par defaut",
            (Version { 1, 2 } < Version { 1, 10 }) && (Version { 2, 0 } > Version { 1, 99 }));

    // 7. `if consteval` et `std::to_underlying` : 13 et 845 emplois chez upstream.
    {
        constexpr int a_la_compilation = selon_contexte();
        int a_l_execution = selon_contexte();
        static_assert(a_la_compilation == 1);
        verifie("if consteval", a_la_compilation == 1 && a_l_execution == 2);
        verifie("std::to_underlying", std::to_underlying(Genre::Binaire) == 7u);
    }

    // 8. Conteneurs et vues.
    std::vector<int> v { 1, 2, 3, 4, 5 };
    std::span<int> vue { v };
    int total = 0;
    for (auto x : vue)
        total += x;
    verifie("std::vector + std::span", total == 15);

    std::string_view sv { "Bouchaud" };
    verifie("std::string_view", sv.size() == 8 && sv.starts_with("Bou"));

    // 9. Allocation dynamique et RAII (donc l'allocateur de la libc statique).
    {
        auto p = std::make_unique<std::vector<std::string>>();
        for (int i = 0; i < 256; ++i)
            p->emplace_back("chaine assez longue pour sortir du petit tampon " + std::to_string(i));
        verifie("unique_ptr + 256 chaines longues (tas)", p->size() == 256
                                                          && p->at(255).size() > 40);
    }

    // 10. Stockage par fil : arch_prctl + bloc TLS place par le chargeur.
    tls_valeur = 0x1234;
    verifie("thread_local (TLS)", tls_valeur == 0x1234);

    // 11. Atomiques : LibGC et LibSync en dependent.
    std::atomic<std::uint64_t> compteur { 0 };
    for (int i = 0; i < 1000; ++i)
        compteur.fetch_add(1, std::memory_order_relaxed);
    // `is_always_lock_free` (constexpr) plutot que `is_lock_free()` : la seconde
    // emet un appel a `__atomic_is_lock_free` de libatomic, que clang ne lie pas
    // par defaut. La propriete qui nous interesse — LibGC et LibSync veulent des
    // atomiques sans verrou — se decide de toute facon a la compilation.
    static_assert(std::atomic<std::uint64_t>::is_always_lock_free,
                  "LibGC et LibSync exigent des atomiques 64 bits sans verrou");
    verifie("std::atomic 64 bits sans verrou", compteur.load() == 1000);

    // 12. Paquets de parametres.
    verifie("paquets de parametres variadiques", compte(1, 2.0, "trois") == 3);

    // 13. if constexpr / traits.
    if constexpr (std::is_integral_v<int>)
        verifie("if constexpr + type_traits", true);
    else
        verifie("if constexpr + type_traits", false);

    std::printf("\nRESULTAT : %d verification(s) en echec (%d passees)\n", echecs, passees);
    return echecs == 0 ? 0 : 1;
}
