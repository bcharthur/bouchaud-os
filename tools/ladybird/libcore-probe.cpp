// Temoin LibSync + LibCore.
//
// LibGC depend publiquement de LibSync et privativement de LibCore.
//
// Cette sonde a d'abord verifie trois choses — c'etait l'epoque ou LibCore
// n'etait construit qu'en sous-ensemble, dix-sept sources sur vingt-huit. Elle
// en verifie desormais davantage, parce que LibCore est construit **en entier**
// et qu'une bibliotheque qu'on ne fait que compiler n'est pas une bibliotheque
// qu'on a portee. Les verifications ajoutees visent precisement la surface que
// le sous-ensemble n'avait pas.
//
// Les trois d'origine :
//
//   - les verrous de LibSync fonctionnent **entre fils** — pas seulement dans un
//     programme a un seul fil, ou un mutex casse ne se voit jamais ;
//   - `ElapsedTimer` mesure une duree qui avance, donc `clock_gettime` de
//     Bouchaud rend des valeurs monotones utilisables ;
//   - `Environment` lit l'environnement que le noyau a construit sur la pile
//     initiale — c'est par la que passeront `BO_WEB_ENGINE` et les variables du
//     protocole GUI.
//
// La verification des verrous est volontairement une course : quatre fils qui
// incrementent un compteur partage cent mille fois. Sans exclusion mutuelle
// reelle, le total final est faux — et il l'est de facon *intermittente*, ce qui
// est exactement le genre de defaut qu'on ne veut pas decouvrir dans LibGC.

#include <AK/Format.h>
#include <AK/String.h>
#include <LibCore/AnonymousBuffer.h>
#include <LibCore/ArgsParser.h>
#include <LibCore/Directory.h>
#include <LibCore/ElapsedTimer.h>
#include <LibCore/Environment.h>
#include <LibCore/File.h>
#include <LibCore/MimeData.h>
#include <LibCore/Process.h>
#include <LibCore/StandardPaths.h>
#include <LibCore/System.h>
#include <LibCore/Version.h>
#include <LibSync/ConditionVariable.h>
#include <LibSync/Mutex.h>

#include <cstdio>
#include <pthread.h>
#include <unistd.h>

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

// --- Course sur un compteur partage -----------------------------------------

constexpr int NB_FILS = 4;
constexpr int PAR_FIL = 100000;

Sync::Mutex g_verrou;
long g_compteur = 0;

void* travail(void*)
{
    for (int i = 0; i < PAR_FIL; ++i) {
        g_verrou.lock();
        ++g_compteur;
        g_verrou.unlock();
    }
    return nullptr;
}

// --- Rendez-vous par variable de condition ----------------------------------

Sync::Mutex g_verrou_signal;
Sync::ConditionVariable g_condition { g_verrou_signal };
bool g_pret = false;

void* signale(void*)
{
    usleep(20000);
    g_verrou_signal.lock();
    g_pret = true;
    g_condition.signal();
    g_verrou_signal.unlock();
    return nullptr;
}

} // namespace

int main()
{
    std::printf("== temoin LibSync + LibCore ==\n");

    // 1. Exclusion mutuelle sous contention reelle.
    {
        pthread_t fils[NB_FILS];
        bool lances = true;
        for (int i = 0; i < NB_FILS; ++i) {
            if (pthread_create(&fils[i], nullptr, travail, nullptr) != 0)
                lances = false;
        }
        for (int i = 0; i < NB_FILS; ++i)
            pthread_join(fils[i], nullptr);
        std::printf("         compteur = %ld (attendu %d)\n",
                    g_compteur, NB_FILS * PAR_FIL);
        verifie("Sync::Mutex : exclusion sous contention (4 fils)",
                lances && g_compteur == static_cast<long>(NB_FILS) * PAR_FIL);
    }

    // 2. Variable de condition : un fil attend, un autre reveille.
    //
    // Si `wait` ne bloquait pas reellement, le test passerait quand meme — d'ou
    // la mesure de duree : le reveil doit arriver *apres* le sommeil de 20 ms du
    // fil signaleur.
    {
        auto chrono = Core::ElapsedTimer::start_new();
        pthread_t fil;
        bool lance = pthread_create(&fil, nullptr, signale, nullptr) == 0;

        g_verrou_signal.lock();
        while (!g_pret)
            g_condition.wait();
        g_verrou_signal.unlock();
        pthread_join(fil, nullptr);

        auto ecoule = chrono.elapsed_milliseconds();
        std::printf("         reveil apres %lld ms\n", static_cast<long long>(ecoule));
        verifie("Sync::ConditionVariable : attente puis reveil",
                lance && g_pret && ecoule >= 15);
    }

    // 3. ElapsedTimer : une duree qui avance.
    {
        auto chrono = Core::ElapsedTimer::start_new();
        usleep(30000);
        auto ecoule = chrono.elapsed_milliseconds();
        std::printf("         30 ms mesures a %lld ms\n", static_cast<long long>(ecoule));
        verifie("Core::ElapsedTimer : horloge monotone", ecoule >= 25 && ecoule < 5000);
    }

    // 4. Environment : ce que le noyau a pose sur la pile initiale.
    {
        auto chemin = Core::Environment::get("PATH"sv);
        auto absente = Core::Environment::get("BO_VARIABLE_QUI_N_EXISTE_PAS"sv);
        verifie("Core::Environment : lecture", chemin.has_value());
        verifie("Core::Environment : variable absente", !absente.has_value());

        // L'ecriture sert au lanceur : c'est ainsi que `BO_WEB_ENGINE` et les
        // descripteurs du protocole GUI seront transmis aux sous-processus.
        auto pose = Core::Environment::set("BO_TEMOIN"sv, "ok"sv,
                                           Core::Environment::Overwrite::Yes);
        auto relue = Core::Environment::get("BO_TEMOIN"sv);
        verifie("Core::Environment : ecriture puis relecture",
                !pose.is_error() && relue.has_value() && relue.value() == "ok"sv);
    }

    // --- La surface que le sous-ensemble n'avait pas ------------------------
    //
    // Ce qui suit n'existait pas dans `libCoreMin.a`. Chaque verification vise
    // une brique que LibIPC, RequestServer ou le navigateur reclameront, et le
    // fait *maintenant*, tant qu'un echec est encore attribuable a une cause
    // unique.

    // 5. AnonymousBuffer : de la memoire partagee designee par un descripteur.
    //
    // C'est le fondement de LibIPC, et c'est aussi exactement le mecanisme que
    // Bouchaud emploie deja pour ses ecrans virtuels (`memfd_create` +
    // `MAP_SHARED`). La verification ecrit puis relit : un tampon qui
    // s'alloue mais ne retient rien passerait un simple test d'existence.
    {
        auto tampon = Core::AnonymousBuffer::create_with_size(8192);
        bool ok = !tampon.is_error() && tampon.value().size() >= 8192;
        if (ok) {
            auto* octets = static_cast<u8*>(tampon.value().data<void>());
            for (size_t i = 0; i < 8192; ++i)
                octets[i] = static_cast<u8>(i * 7);
            for (size_t i = 0; i < 8192; ++i) {
                if (octets[i] != static_cast<u8>(i * 7)) { ok = false; break; }
            }
        }
        verifie("Core::AnonymousBuffer : memoire partagee ecrite puis relue", ok);
    }

    // 6. MimeData : le reniflage de type, tel que le navigateur en aura besoin
    //    pour decider quoi faire d'une reponse.
    {
        auto par_nom = Core::guess_mime_type_based_on_filename("page.html"sv);
        u8 png[] = { 0x89, 'P', 'N', 'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0 };
        auto par_octets = Core::guess_mime_type_based_on_sniffed_bytes(ReadonlyBytes { png, sizeof(png) });
        std::printf("         \"page.html\" -> %s ; octets PNG -> %s\n",
                    ByteString(par_nom).characters(),
                    par_octets.has_value() ? ByteString(*par_octets).characters() : "(rien)");
        verifie("Core::MimeData : type devine par le nom", par_nom == "text/html"sv);
        verifie("Core::MimeData : type devine par les octets",
                par_octets.has_value() && *par_octets == "image/png"sv);
    }

    // 7. StandardPaths et Directory : le systeme de fichiers vu par LibCore.
    {
        auto tmp = Core::StandardPaths::tempfile_directory();
        auto existe = !Core::Directory::create(tmp, Core::Directory::CreateDirectories::Yes).is_error();
        std::printf("         repertoire temporaire : %s\n", tmp.characters());
        verifie("Core::StandardPaths + Core::Directory", !tmp.is_empty() && existe);
    }

    // 8. File : ouvrir, ecrire, relire. `FilePosix.cpp` porte les virtuelles de
    //    `Core::File` — c'est le fichier dont l'absence donnait « undefined
    //    reference to vtable for Core::File », un message qui ne nomme rien.
    {
        auto chemin = ByteString::formatted("{}/bo-temoin-libcore", Core::StandardPaths::tempfile_directory());
        bool ok = false;
        if (auto f = Core::File::open(chemin, Core::File::OpenMode::Write); !f.is_error()) {
            ok = !f.value()->write_until_depleted("bouchaud"sv.bytes()).is_error();
        }
        if (ok) {
            auto lu = Core::File::open(chemin, Core::File::OpenMode::Read);
            ok = !lu.is_error();
            if (ok) {
                auto contenu = lu.value()->read_until_eof();
                ok = !contenu.is_error() && contenu.value().size() == 8;
            }
        }
        (void)Core::System::unlink(chemin);
        verifie("Core::File : ecriture puis relecture (FilePosix)", ok);
    }

    // 9. Process : lancer un programme et attendre sa fin.
    //
    // Sous Bouchaud cela passe par `clone` — la glibc n'emet plus `fork`. C'est
    // le chemin dont depend la separation navigateur/renderer.
    {
        Vector<ByteString> arguments;
        auto lance = Core::Process::spawn("/bin/true"sv, arguments.span());
        bool ok = false;
        if (!lance.is_error()) {
            auto code = lance.value().wait_for_termination();
            ok = !code.is_error();
        }
        verifie("Core::Process : lancer et attendre un sous-processus", ok);
    }

    // 10. ArgsParser et Version : de la logique pure, mais deux sources de plus
    //     qui n'entraient pas dans le sous-ensemble.
    {
        auto version = Core::Version::read_long_version_string();
        std::printf("         version Ladybird : %s\n", version.to_byte_string().characters());
        verifie("Core::Version", !version.is_empty());
    }

    std::printf("\nRESULTAT : %d verification(s) en echec (%d passees)\n", echecs, passees);
    return echecs == 0 ? 0 : 1;
}
