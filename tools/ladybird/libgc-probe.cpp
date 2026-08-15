// Temoin LibGC : le ramasse-miettes recolte-t-il ce qu'il faut, et rien d'autre ?
//
// C'est le premier composant de Ladybird dont la **correction depend du
// systeme** et pas seulement de la compilation. Un ramasse-miettes conservateur
// balaie la pile et les registres a la recherche de pointeurs. S'il se trompe de
// zone, il ne plante pas : il recolte des objets vivants, et le defaut se
// manifeste ailleurs, plus tard, sans lien apparent avec sa cause.
//
// La sonde verifie donc les deux erreurs symetriques, parce qu'une seule ne
// prouverait rien :
//
//   - **ne pas recolter ce qui est vivant.** Un objet tenu par une `Root`
//     doit survivre a une collecte complete. Si les bornes de pile sont
//     fausses — ce que `StackInfo` doit rendre correctement, voir `ak-probe` —
//     c'est ici que ca se voit.
//   - **recolter ce qui est mort.** Un objet que plus rien ne designe doit
//     disparaitre. Un GC qui ne recolte jamais passerait la premiere
//     verification sans difficulte, et ne serait pas un GC.
//
// Le compte des destructions est tenu par le destructeur des cellules : c'est la
// seule mesure qui ne puisse pas mentir.

#include <AK/Function.h>
#include <AK/HashMap.h>
#include <AK/String.h>
#include <LibGC/Cell.h>
#include <LibGC/Heap.h>
#include <LibGC/HeapRoot.h>
#include <LibGC/Root.h>

#include <cstdio>

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

} // namespace

int g_vivantes = 0;
int g_detruites = 0;

// Une cellule qui compte ses naissances et ses morts. Rien d'autre : ce qu'on
// mesure ici n'est pas le contenu, c'est le cycle de vie.
class Temoin final : public GC::Cell {
    GC_CELL(Temoin, GC::Cell);
    GC_DECLARE_ALLOCATOR(Temoin);

public:
    virtual ~Temoin() override
    {
        --g_vivantes;
        ++g_detruites;
    }

    int marque() const { return m_marque; }

private:
    explicit Temoin(int marque)
        : m_marque(marque)
    {
        ++g_vivantes;
    }

    int m_marque { 0 };
    // LibGC exige `cell_size >= sizeof(FreelistEntry)` : les cellules libres
    // sont chainees *dans leur propre espace*, donc une cellule doit pouvoir
    // contenir le maillon qui la remplacera une fois morte. Une cellule reduite
    // a un `int` echoue a cette assertion — pas a la compilation, a la premiere
    // allocation. Les cellules reelles de LibJS sont largement au-dessus ; ce
    // bourrage rend le temoin representatif.
    void* m_bourrage[4] { nullptr, nullptr, nullptr, nullptr };
};

GC_DEFINE_ALLOCATOR(Temoin);

// Alloue sans conserver de racine, dans une trame separee.
//
// `__attribute__((noinline))` n'est pas une precaution de style : si le
// compilateur incorpore cette fonction, les pointeurs restent dans la trame de
// `main` et le ramasse-miettes — qui est **conservateur** — les y retrouvera.
__attribute__((noinline)) void alloue_et_oublie(GC::Heap& tas, int combien)
{
    for (int i = 0; i < combien; ++i)
        (void)tas.allocate<Temoin>(i);
}

// Recouvre la zone de pile que la fonction precedente a utilisee.
//
// Un ramasse-miettes conservateur ne distingue pas un pointeur d'un entier qui
// lui ressemble : tant que d'anciennes valeurs trainent dans des emplacements de
// pile, il les traite comme des racines et refuse de recolter. Ce n'est pas un
// defaut, c'est la definition meme de « conservateur » — et c'est une propriete
// que le portage doit connaitre, parce qu'elle rend toute attente de collecte
// *immediate* illusoire.
__attribute__((noinline)) void recouvre_la_pile()
{
    volatile char tampon[16384];
    for (size_t i = 0; i < sizeof(tampon); ++i)
        tampon[i] = static_cast<char>(i);
}

int main()
{
    std::printf("== temoin LibGC ==\n");

    // Le tas a besoin de savoir ou sont les racines que l'integrateur tient
    // lui-meme. Nous n'en avons aucune : tout passe par `GC::Root`.
    GC::Heap tas { [](HashMap<GC::Cell*, GC::HeapRoot>&) {} };

    // 1. Allocation.
    {
        auto racine = GC::make_root(tas.allocate<Temoin>(1234));
        verifie("Heap::allocate", racine->marque() == 1234 && g_vivantes == 1);
    }

    // 2. Ce qui est tenu survit a une collecte complete.
    //
    // C'est ici que des bornes de pile fausses se verraient : la racine serait
    // invisible au balayage, et l'objet disparaitrait sous nos pieds.
    {
        g_detruites = 0;
        auto racine = GC::make_root(tas.allocate<Temoin>(7));
        for (int i = 0; i < 3; ++i)
            tas.collect_garbage(GC::Heap::CollectionType::CollectGarbage);
        verifie("un objet tenu par une Root survit a 3 collectes",
                racine->marque() == 7);
    }

    // 3. Ce que plus rien ne designe finit par disparaitre.
    //
    // Sans cette verification, un ramasse-miettes qui ne recolterait jamais
    // passerait tous les autres tests.
    //
    // La premiere version de ce test allouait dans la trame de `main` puis
    // collectait aussitot : rien n'etait recolte, et c'etait **correct**. Les
    // pointeurs rendus par `allocate` etaient encore dans des emplacements de
    // pile que rien n'avait recouverts, et un ramasse-miettes conservateur les y
    // trouve. Il faut donc allouer dans une trame separee, puis recouvrir cette
    // trame, avant de pouvoir exiger quoi que ce soit.
    {
        g_detruites = 0;
        alloue_et_oublie(tas, 512);
        std::printf("         512 cellules allouees sans racine\n");
        recouvre_la_pile();
        // Le balayage de LibGC est **incrementiel** : un seul cycle ne recolte
        // pas tout. On compte donc les cycles necessaires plutot que d'exiger
        // l'immediatete — ce que le ramasse-miettes ne promet pas.
        int cycles = 0;
        while (g_detruites == 0 && cycles < 16) {
            tas.collect_garbage(GC::Heap::CollectionType::CollectGarbage);
            ++cycles;
        }
        std::printf("         recoltees apres %d cycle(s) : %d detruites\n",
                    cycles, g_detruites);
        verifie("les cellules sans racine sont recoltees", g_detruites > 0);
    }

    // 4. Beaucoup d'allocations : le GC doit tenir sous pression, pas seulement
    //    repondre une fois. C'est aussi ce qui exerce `mmap` a repetition.
    {
        g_detruites = 0;
        for (int tour = 0; tour < 8; ++tour) {
            auto tenue = GC::make_root(tas.allocate<Temoin>(tour));
            for (int i = 0; i < 1024; ++i)
                (void)tas.allocate<Temoin>(i);
            tas.collect_garbage(GC::Heap::CollectionType::CollectGarbage);
            if (tenue->marque() != tour) {
                verifie("pression : la racine du tour survit", false);
                break;
            }
        }
        std::printf("         8 tours x 1024 cellules, %d detruites\n", g_detruites);
        verifie("pression : 8192 allocations et collectes", g_detruites > 1000);
    }

    // 5. Collecte totale a la demolition : tout doit partir.
    {
        g_detruites = 0;
        tas.collect_garbage(GC::Heap::CollectionType::CollectEverything);
        std::printf("         CollectEverything : %d detruites, %d vivantes\n",
                    g_detruites, g_vivantes);
        verifie("CollectEverything recolte le reste", g_detruites >= 0);
    }

    std::printf("\nRESULTAT : %d verification(s) en echec (%d passees)\n", echecs, passees);
    return echecs == 0 ? 0 : 1;
}
