// La latence de l'interface pendant qu'un autre processus calcule.
//
// C'est la seule mesure qui compte pour decider si un processus de rendu
// separe vaudra la peine. L'audit OS avait conclu : sur un cœur unique et un
// tourniquet strict, sortir le rendu d'un processus n'empeche pas une page
// lourde de rendre l'interface lente, parce que rien ne favorise l'interface.
// Deux classes d'ordonnancement viennent d'etre ajoutees ; cette sonde dit si
// elles changent quelque chose.
//
// Le dispositif :
//
//   * un processus **interface** se reveille toutes les 16 ms — la cadence
//     d'un rafraichissement — et note de combien il est en retard. Un reveil
//     demande a 16 ms qui arrive a 40 ms accuse 24 ms de retard, et c'est ce
//     retard-la que l'utilisateur ressent comme une saccade ;
//   * un processus **calcul** tourne sans jamais se bloquer, comme le ferait
//     une page qui met en page dix mille elements.
//
// Trois mesures, dans cet ordre, parce que chacune sert de temoin a la
// suivante :
//
//   1. l'interface seule — la latence de base, celle du timer ;
//   2. l'interface pendant le calcul, **sans** priorite ;
//   3. l'interface pendant le calcul, **avec** priorite.
//
// La troisieme doit s'approcher de la premiere. Et le processus de calcul doit
// avoir progresse dans les trois : une priorite qui l'affamerait ne serait pas
// une priorite mais une exclusion — « l'interface reste fluide » deviendrait
// « rien d'autre ne tourne ».
//
//   musl-gcc -O2 -static-pie ordonnanceur-probe.c -o ordonnanceur-probe
//   (ou via ./build.sh musl)

#define _GNU_SOURCE
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static int echecs = 0;
static int reussites = 0;

static void verifie(const char *nom, int condition, const char *detail)
{
    if (condition) {
        reussites++;
    } else {
        echecs++;
        printf("  - %s%s%s\n", nom, detail ? " : " : "", detail ? detail : "");
    }
}

static long long maintenant_us(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (long long)t.tv_sec * 1000000 + t.tv_nsec / 1000;
}

#define PERIODE_US 16000
#define REVEILS 60

// Le processus d'interface : se reveiller a l'heure, et mesurer de combien on
// ne l'est pas. On rend le retard **median** et le pire.
static void mesure_interface(long long *median, long long *pire)
{
    long long retards[REVEILS];
    long long attendu = maintenant_us();

    for (int i = 0; i < REVEILS; i++) {
        attendu += PERIODE_US;
        long long avant = maintenant_us();
        long long reste = attendu - avant;
        if (reste > 0) {
            struct timespec duree = {
                .tv_sec = reste / 1000000,
                .tv_nsec = (reste % 1000000) * 1000,
            };
            nanosleep(&duree, NULL);
        }
        long long reel = maintenant_us();
        retards[i] = reel > attendu ? reel - attendu : 0;
    }

    // Tri par insertion : soixante valeurs, la simplicite vaut mieux ici.
    for (int i = 1; i < REVEILS; i++) {
        long long v = retards[i];
        int j = i - 1;
        while (j >= 0 && retards[j] > v) { retards[j + 1] = retards[j]; j--; }
        retards[j + 1] = v;
    }
    *median = retards[REVEILS / 2];
    *pire = retards[REVEILS - 1];
}

// Le processus de calcul : une boucle qui ne se bloque jamais. Il ecrit son
// compteur de tours dans le tube avant de mourir, ce qui prouve qu'il a
// progresse.
static void calcule_jusqua(int tube, int secondes)
{
    long long fin = maintenant_us() + (long long)secondes * 1000000;
    volatile unsigned long long accumulateur = 0;
    unsigned long long tours = 0;
    while (maintenant_us() < fin) {
        for (int i = 0; i < 20000; i++)
            accumulateur += i * 2654435761u;
        tours++;
    }
    if (write(tube, &tours, sizeof tours) < 0)
        _exit(1);
    _exit(0);
}

// Lance le calcul en fond, mesure l'interface pendant ce temps, et rend les
// tours effectues par le calcul.
static unsigned long long sous_charge(int interactif, long long *median,
                                      long long *pire)
{
    int tube[2];
    if (pipe(tube) != 0)
        return 0;

    pid_t enfant = fork();
    if (enfant < 0) {
        close(tube[0]);
        close(tube[1]);
        return 0;
    }
    if (enfant == 0) {
        close(tube[0]);
        // Le calcul reste normal dans les deux cas : ce qu'on change est la
        // priorite de **l'interface**, pas la sienne. Le degrader serait une
        // autre experience, et une moins honnete.
        calcule_jusqua(tube[1], 2);
    }
    close(tube[1]);

    if (interactif) {
        // `nice` negatif : le seul moyen portable de se declarer interactif.
        // Un programme ecrit pour Linux n'a rien de special a faire.
        if (setpriority(PRIO_PROCESS, 0, -5) != 0)
            printf("  (setpriority a echoue : %s)\n", strerror(errno));
    }
    mesure_interface(median, pire);
    if (interactif)
        setpriority(PRIO_PROCESS, 0, 0);

    unsigned long long tours = 0;
    if (read(tube[0], &tours, sizeof tours) != (ssize_t)sizeof tours)
        tours = 0;
    close(tube[0]);
    int statut = 0;
    waitpid(enfant, &statut, 0);
    return tours;
}

int main(void)
{
    printf("ordonnanceur-probe: latence de l'interface sous charge\n");

    // 1. Le temoin : l'interface seule.
    long long repos_median = 0, repos_pire = 0;
    mesure_interface(&repos_median, &repos_pire);
    printf("  au repos              median %5lld us   pire %6lld us\n",
           repos_median, repos_pire);

    // 2. Sous charge, sans priorite.
    long long normale_median = 0, normale_pire = 0;
    unsigned long long tours_normale =
        sous_charge(0, &normale_median, &normale_pire);
    printf("  sous charge (normal)  median %5lld us   pire %6lld us"
           "   calcul : %llu tours\n",
           normale_median, normale_pire, tours_normale);

    // 3. Sous charge, avec priorite.
    long long prio_median = 0, prio_pire = 0;
    unsigned long long tours_prio = sous_charge(1, &prio_median, &prio_pire);
    printf("  sous charge (interactif) median %5lld us   pire %6lld us"
           "   calcul : %llu tours\n",
           prio_median, prio_pire, tours_prio);

    char detail[192];

    // Le test n'a de sens que si la charge degrade reellement la latence.
    // Sans cela, comparer « avec » et « sans » priorite ne mesurerait que du
    // bruit — et conclurait a une amelioration inexistante.
    int charge_visible = normale_median > repos_median + 1000;
    if (!charge_visible) {
        // Sur une machine a plusieurs cœurs, un seul processus de calcul ne
        // dispute rien a l'interface : il n'y a pas de degradation a corriger,
        // et comparer « avec » et « sans » priorite ne mesurerait que du bruit.
        //
        // On le dit plutot que de conclure. Un test qui annoncerait une
        // amelioration la ou il n'y a rien a ameliorer serait pire qu'absent :
        // il validerait n'importe quelle implementation, y compris une qui ne
        // fait rien.
        printf("  (la charge ne degrade pas la latence sur cette machine —\n"
               "   plusieurs cœurs, sans doute. L'effet de la priorite n'y est\n"
               "   pas mesurable ; c'est sous Bouchaud OS, sur un cœur unique,\n"
               "   que cette sonde a un sens.)\n");
    } else {
        snprintf(detail, sizeof detail, "sans %lld us, avec %lld us",
                 normale_median, prio_median);
        verifie("la priorite interactive reduit la latence",
                prio_median < normale_median, detail);
    }

    // Et surtout : le calcul doit avoir progresse dans les deux cas. Une
    // priorite qui l'affamerait ne serait pas une priorite.
    snprintf(detail, sizeof detail, "normal %llu tours, interactif %llu tours",
             tours_normale, tours_prio);
    verifie("le processus de calcul n'est pas affame", tours_prio > 0, detail);
    if (tours_normale > 0) {
        // On tolere une forte reduction — c'est le but — mais pas
        // l'annulation : au moins un dixieme du travail doit passer.
        verifie("et il conserve une part significative du temps",
                tours_prio * 10 >= tours_normale, detail);
    }

    printf("RESULTAT : %d verification(s) en echec (%d passees)\n",
           echecs, reussites);
    return echecs == 0 ? 0 : 1;
}
