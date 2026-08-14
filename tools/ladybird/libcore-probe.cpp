// Temoin LibSync + LibCore minimal.
//
// LibGC depend publiquement de LibSync et privativement de LibCore. Cette sonde
// verifie les trois choses dont la suite du portage a besoin, et rien de plus :
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
#include <LibCore/ElapsedTimer.h>
#include <LibCore/Environment.h>
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

    std::printf("\nRESULTAT : %d verification(s) en echec (%d passees)\n", echecs, passees);
    return echecs == 0 ? 0 : 1;
}
